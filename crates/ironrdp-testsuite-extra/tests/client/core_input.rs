//! Input routing between the MS-RDPECI core input channel and fast-path input.

use std::borrow::Cow;

use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_client::__test_process_input;
use ironrdp_client::config::{ConfigBuilder, Destination};
use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::pdu::{ClosePdu, CreateRequestPdu, DataPdu, DrdynvcClientPdu, DrdynvcDataPdu, DrdynvcServerPdu};
use ironrdp_dvc::{DrdynvcClient, DvcMessageBatch};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_pdu::Action;
use ironrdp_pdu::fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_pdu::input::MousePdu;
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent, KeyboardFlags};
use ironrdp_pdu::input::mouse::PointerFlags;
use ironrdp_pdu::mcs::SendDataIndication;
use ironrdp_pdu::pointer::{ColorPointerAttribute, Point16};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};
use ironrdp_pdu::x224::X224;
use ironrdp_rdpeci::client::CoreInputClient;
use ironrdp_rdpeci::pdu::CoreInputPdu;
use ironrdp_svc::StaticChannelSet;

const USER_CHANNEL: u16 = 1002;
const DRDYNVC: u16 = 1004;
const CORE_INPUT: u32 = 3;

// oracle: spec MS-RDPECI §4.1.2
const INIT_RESPONSE: [u8; 16] = [
    0x03, // signature
    0x02, // pduType: PDUTYPE_SC_INIT_RESPONSE
    0x00, // eventCount
    0x00, // padding
    0x00, 0x01, // selectedProtocolVersion: RDP_CORE_INPUT_PROTOCOL_VERSION_100
    0x00, 0x01, // protocolVersionMax: RDP_CORE_INPUT_PROTOCOL_VERSION_100
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // reserved
];

fn mouse_move(x: u16, y: u16) -> FastPathInputEvent {
    FastPathInputEvent::MouseEvent(MousePdu {
        flags: PointerFlags::MOVE,
        number_of_wheel_rotation_units: 0,
        x_position: x,
        y_position: y,
    })
}

fn input_events() -> [FastPathInputEvent; 3] {
    [
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x1E),
        mouse_move(1, 1),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x1E),
    ]
}

/// A stage whose DRDYNVC client registers the core input channel when `core_input` is set,
/// as the client does when `ChannelConfig::core_input` is enabled.
fn stage(core_input: bool, software_pointer: bool) -> ActiveStage {
    let mut drdynvc = DrdynvcClient::new();
    if core_input {
        drdynvc = drdynvc.with_dynamic_channel(CoreInputClient::new());
    }
    let mut channels = StaticChannelSet::new();
    channels.insert(drdynvc);
    channels.attach_channel_id(core::any::TypeId::of::<DrdynvcClient>(), DRDYNVC);
    ActiveStageBuilder {
        static_channels: channels,
        user_channel_id: USER_CHANNEL,
        io_channel_id: 1003,
        message_channel_id: None,
        share_id: 1,
        compression_type: None,
        enable_server_pointer: software_pointer,
        pointer_software_rendering: software_pointer,
    }
    .build()
}

/// A DRDYNVC PDU as the server sends it: one static channel chunk.
fn drdynvc_frame(pdu: &DrdynvcServerPdu) -> Vec<u8> {
    let pdu = encode_vec(pdu).expect("encode DRDYNVC PDU");
    let mut data = encode_vec(&ChannelPduHeader {
        length: u32::try_from(pdu.len()).expect("fits"),
        flags: ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST,
    })
    .expect("encode channel header");
    data.extend_from_slice(&pdu);
    encode_vec(&X224(SendDataIndication {
        initiator_id: USER_CHANNEL,
        channel_id: DRDYNVC,
        user_data: Cow::Owned(data),
    }))
    .expect("encode X224")
}

fn server_sends(stage: &mut ActiveStage, image: &mut DecodedImage, pdu: DrdynvcServerPdu) {
    stage
        .process(image, Action::X224, &drdynvc_frame(&pdu), None)
        .expect("process DRDYNVC PDU");
}

fn open_core_input(stage: &mut ActiveStage, image: &mut DecodedImage) {
    server_sends(
        stage,
        image,
        DrdynvcServerPdu::Create(CreateRequestPdu::new(
            CORE_INPUT,
            ironrdp_rdpeci::CHANNEL_NAME.to_owned(),
        )),
    );
}

/// Opens the channel and completes the MS-RDPECI 3.3.3 version exchange.
fn ready_core_input(stage: &mut ActiveStage, image: &mut DecodedImage) {
    open_core_input(stage, image);
    server_sends(
        stage,
        image,
        DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(CORE_INPUT, INIT_RESPONSE.to_vec()))),
    );
}

/// The events carried by the fast-path input frames among `outputs`.
fn fast_path_events(outputs: &[ActiveStageOutput]) -> Vec<FastPathInputEvent> {
    outputs
        .iter()
        .filter_map(|output| match output {
            ActiveStageOutput::ResponseFrame(frame) => Some(frame),
            _ => None,
        })
        .flat_map(|frame| {
            decode::<FastPathInput>(frame)
                .expect("decode fast-path input")
                .input_events()
                .to_vec()
        })
        .collect()
}

/// The events carried by the core input PDUs in `batch`.
fn core_input_events(batch: &DvcMessageBatch) -> Vec<FastPathInputEvent> {
    batch
        .messages()
        .iter()
        .flat_map(|message| {
            let encoded = message.encode_unframed_pdu().expect("encode DVC message");
            let data = match decode::<DrdynvcClientPdu>(&encoded).expect("decode DVC message") {
                DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(data)) => Some(data),
                _ => None,
            }
            .expect("core input batch holds only DVC data PDUs");
            match decode::<CoreInputPdu>(data.data()).expect("decode core input PDU") {
                CoreInputPdu::Input(input) => Some(input.into_events()),
                _ => None,
            }
            .expect("core input batch holds only input PDUs")
        })
        .collect()
}

fn graphics_updates(outputs: &[ActiveStageOutput]) -> Vec<InclusiveRectangle> {
    outputs
        .iter()
        .filter_map(|output| match output {
            ActiveStageOutput::GraphicsUpdate(rect) => Some(rect.clone()),
            _ => None,
        })
        .collect()
}

/// Whether a connector built with `ChannelConfig::core_input` set to `enabled` registers the core input channel.
fn connector_registers_core_input(enabled: bool) -> bool {
    let config = ConfigBuilder::new()
        .with_destination(Destination::from_parts("core-input.invalid", 3389))
        .with_username("user")
        .with_password(String::new())
        .with_client_build(1)
        .with_client_dir(r"C:\")
        .with_platform(MajorPlatformType::WINDOWS)
        .with_client_name("client")
        .with_core_input(enabled)
        .build()
        .expect("core input test configuration");
    let mut connector = ironrdp_client::__test_build_connector(&config, None).expect("build connector");
    connector
        .get_static_channel_processor::<DrdynvcClient>()
        .expect("DRDYNVC processor")
        .has_registered_dvc::<CoreInputClient>()
}

// oracle: mutant catches a connector that never registers the core input channel
#[test]
fn connector_registers_core_input_when_enabled() {
    assert!(connector_registers_core_input(true));
}

// oracle: mutant catches a connector that registers the core input channel although the option is off
#[test]
fn connector_omits_core_input_when_disabled() {
    assert!(!connector_registers_core_input(false));
}

// oracle: spec MS-RDPECI §1.3
// oracle: spec MS-RDPBCGR §2.2.8.1.2
#[test]
fn input_uses_fast_path_without_the_core_input_channel() {
    let mut stage = stage(false, false);
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 8, 8);

    let (batch, outputs) = __test_process_input(&mut stage, &mut image, &input_events()).expect("route input");

    assert!(batch.is_none());
    assert_eq!(fast_path_events(&outputs), input_events());
}

/// Input goes on the channel only in the Running Phase, after the server answers the init request.
// oracle: spec MS-RDPECI §1.3
// oracle: spec MS-RDPBCGR §2.2.8.1.2
#[test]
fn input_uses_fast_path_until_the_server_answers_the_init_request() {
    let mut stage = stage(true, false);
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 8, 8);
    open_core_input(&mut stage, &mut image);

    let (batch, outputs) = __test_process_input(&mut stage, &mut image, &input_events()).expect("route input");

    assert!(batch.is_none());
    assert_eq!(fast_path_events(&outputs), input_events());
}

/// Once ready, the channel carries the input in place of the fast-path input PDU.
// oracle: spec MS-RDPECI §1.4
#[test]
fn input_uses_only_the_core_input_channel_once_it_is_ready() {
    let mut stage = stage(true, false);
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 8, 8);
    ready_core_input(&mut stage, &mut image);

    let (batch, outputs) = __test_process_input(&mut stage, &mut image, &input_events()).expect("route input");

    let batch = batch.expect("core input batch");
    assert_eq!(batch.channel_id(), CORE_INPUT);
    assert_eq!(core_input_events(&batch), input_events());
    assert!(fast_path_events(&outputs).is_empty());
}

// oracle: mutant catches input still routed to the core input channel after the server closes it
#[test]
fn input_returns_to_fast_path_after_the_core_input_channel_closes() {
    let mut stage = stage(true, false);
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 8, 8);
    ready_core_input(&mut stage, &mut image);
    server_sends(
        &mut stage,
        &mut image,
        DrdynvcServerPdu::Close(ClosePdu::new(CORE_INPUT)),
    );

    let (batch, outputs) = __test_process_input(&mut stage, &mut image, &input_events()).expect("route input");

    assert!(batch.is_none());
    assert_eq!(fast_path_events(&outputs), input_events());
}

/// A mouse move sent on the core input channel redraws the software pointer as fast-path input does.
// oracle: mutant catches the software pointer redraw dropped from the core input path
#[test]
fn core_input_keeps_software_pointer_updates() {
    // A 2x2 24 bpp color pointer: each XOR row is 6 bytes and each AND row is padded to 2 bytes.
    let xor_mask = [0xFF; 12];
    let and_mask = [0x00; 4];
    let pointer = encode_vec(&ColorPointerAttribute {
        cache_index: 0,
        hot_spot: Point16 { x: 0, y: 0 },
        width: 2,
        height: 2,
        xor_mask: &xor_mask,
        and_mask: &and_mask,
    })
    .expect("encode color pointer");
    let update = encode_vec(&FastPathUpdatePdu {
        fragmentation: Fragmentation::Single,
        update_code: UpdateCode::ColorPointer,
        compression_flags: None,
        compression_type: None,
        data: &pointer,
    })
    .expect("encode pointer update");
    let mut pointer_frame =
        encode_vec(&FastPathHeader::new(EncryptionFlags::empty(), update.len())).expect("encode fast-path header");
    pointer_frame.extend_from_slice(&update);

    let move_pointer = |stage: &mut ActiveStage, ready: bool| {
        let mut image = DecodedImage::new(PixelFormat::RgbA32, 8, 8);
        if ready {
            ready_core_input(stage, &mut image);
        }
        stage
            .process(&mut image, Action::FastPath, &pointer_frame, None)
            .expect("set the pointer");
        __test_process_input(stage, &mut image, &[mouse_move(4, 4)]).expect("route input")
    };
    let (fast_path_batch, fast_path_outputs) = move_pointer(&mut stage(false, true), false);
    let (core_input_batch, core_input_outputs) = move_pointer(&mut stage(true, true), true);

    assert!(fast_path_batch.is_none());
    assert_eq!(
        core_input_events(&core_input_batch.expect("core input batch")),
        [mouse_move(4, 4)]
    );
    assert!(fast_path_events(&core_input_outputs).is_empty());
    let pointer_updates = graphics_updates(&fast_path_outputs);
    assert_eq!(pointer_updates.len(), 1);
    assert_eq!(graphics_updates(&core_input_outputs), pointer_updates);
}
