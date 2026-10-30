use std::sync::{Arc, Mutex};

use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_dvc::pdu::{DrdynvcClientPdu, DrdynvcDataPdu};
use ironrdp_pdu::input::MousePdu;
use ironrdp_pdu::input::fast_path::{FastPathInputEvent, KeyboardFlags, SynchronizeFlags};
use ironrdp_pdu::input::mouse::PointerFlags;
use ironrdp_rdpeci::client::CoreInputClient;
use ironrdp_rdpeci::pdu::{CoreInputPdu, InitRequestPdu, InitResponsePdu, InputPdu, ProtocolVersion};
use ironrdp_rdpeci::server::{CoreInputHandler, CoreInputServer};

const CHANNEL_ID: u32 = 7;

// oracle: spec MS-RDPECI §4.1.1
const INIT_REQUEST: [u8; 16] = [
    0x03, // signature
    0x01, // pduType: PDUTYPE_CS_INIT_REQUEST
    0x00, // eventCount
    0x00, // padding
    0x00, 0x01, // protocolVersionMin: RDP_CORE_INPUT_PROTOCOL_VERSION_100
    0x00, 0x01, // protocolVersionMax: RDP_CORE_INPUT_PROTOCOL_VERSION_100
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // reserved
];

// oracle: spec MS-RDPECI §4.1.2
// The capture ends with two bytes past the 16-byte PDU that section 2.2.3.2 defines.
const INIT_RESPONSE_CAPTURE: [u8; 18] = [
    0x03, // signature
    0x02, // pduType: PDUTYPE_SC_INIT_RESPONSE
    0x00, // eventCount
    0x00, // padding
    0x00, 0x01, // selectedProtocolVersion: RDP_CORE_INPUT_PROTOCOL_VERSION_100
    0x00, 0x01, // protocolVersionMax: RDP_CORE_INPUT_PROTOCOL_VERSION_100
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // reserved
    0x00, 0x00, // trailing capture bytes
];

// oracle: spec MS-RDPECI §4.2
const INPUT: [u8; 16] = [
    0x03, // signature
    0x03, // pduType: PDUTYPE_CS_KEYBOARD_AND_MOUSE_INPUT
    0x02, // eventCount
    0x00, // padding
    0xC0, // typeAndFlags: INPUT_EVENT_QOE_TIMESTAMP
    0xC0, 0x57, 0x86, 0x04, // timestamp
    0x20, // typeAndFlags: INPUT_EVENT_MOUSE
    0x00, 0x04, // pointerFlags: PTRFLAGS_HWHEEL
    0x00, 0x00, // xPos
    0x00, 0x00, // yPos
];

fn spec_input_events() -> Vec<FastPathInputEvent> {
    vec![
        FastPathInputEvent::QoeEvent(0x0486_57C0),
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::HORIZONTAL_WHEEL,
            number_of_wheel_rotation_units: 0,
            x_position: 0,
            y_position: 0,
        }),
    ]
}

fn decode_svc_message(message: &ironrdp_svc::SvcMessage) -> CoreInputPdu {
    let encoded = message.encode_unframed_pdu().expect("encode unframed DRDYNVC PDU");
    let dvc: DrdynvcClientPdu = decode(&encoded).expect("decode DRDYNVC PDU");
    let DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(data)) = dvc else {
        panic!("expected unsplit DRDYNVC data");
    };
    decode(data.data()).expect("decode core input PDU")
}

fn ready_client() -> CoreInputClient {
    let mut client = CoreInputClient::new();
    client.start(CHANNEL_ID).expect("start core input channel");
    client
        .process(CHANNEL_ID, &INIT_RESPONSE_CAPTURE)
        .expect("process init response");
    client
}

#[derive(Clone, Default)]
struct RecordingHandler(Arc<Mutex<Vec<FastPathInputEvent>>>);

impl CoreInputHandler for RecordingHandler {
    fn input(&mut self, events: Vec<FastPathInputEvent>) {
        self.0.lock().unwrap().extend(events);
    }
}

#[test]
fn init_request_matches_spec_capture() {
    let pdu = CoreInputPdu::InitRequest(InitRequestPdu {
        protocol_version_min: ProtocolVersion::V1_0,
        protocol_version_max: ProtocolVersion::V1_0,
    });

    assert_eq!(encode_vec(&pdu).expect("encode init request"), INIT_REQUEST);
    assert_eq!(decode::<CoreInputPdu>(&INIT_REQUEST).expect("decode init request"), pdu);
}

#[test]
fn init_response_decodes_spec_capture() {
    let pdu = CoreInputPdu::InitResponse(InitResponsePdu {
        selected_protocol_version: ProtocolVersion::V1_0,
        protocol_version_max: ProtocolVersion::V1_0,
    });

    assert_eq!(
        decode::<CoreInputPdu>(&INIT_RESPONSE_CAPTURE).expect("decode init response"),
        pdu
    );
    // oracle: spec MS-RDPECI §2.2.3.2
    assert_eq!(
        encode_vec(&pdu).expect("encode init response"),
        INIT_RESPONSE_CAPTURE[..16]
    );
}

#[test]
fn input_pdu_matches_spec_capture() {
    let pdu = CoreInputPdu::Input(InputPdu::new(spec_input_events()).expect("two events fit"));

    assert_eq!(decode::<CoreInputPdu>(&INPUT).expect("decode input PDU"), pdu);
    assert_eq!(encode_vec(&pdu).expect("encode input PDU"), INPUT);
}

// oracle: spec MS-RDPECI §2.2.2.2
#[test]
fn keyboard_event_flags_pack_below_the_event_type() {
    let pdu = CoreInputPdu::Input(
        InputPdu::new(vec![
            FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE | KeyboardFlags::EXTENDED, 0x1D),
            FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::RELEASE, 0x00E9),
            FastPathInputEvent::SyncEvent(SynchronizeFlags::NUM_LOCK | SynchronizeFlags::CAPS_LOCK),
        ])
        .expect("three events fit"),
    );

    assert_eq!(
        encode_vec(&pdu).expect("encode input PDU"),
        [
            0x03, // signature
            0x03, // pduType: PDUTYPE_CS_KEYBOARD_AND_MOUSE_INPUT
            0x03, // eventCount
            0x00, // padding
            0x03, // typeAndFlags: INPUT_EVENT_SCANCODE, INPUT_KBDFLAGS_RELEASE | INPUT_KBDFLAGS_EXTENDED
            0x1D, // keyCode
            0x81, // typeAndFlags: INPUT_EVENT_UNICODE, INPUT_KBDFLAGS_RELEASE
            0xE9, 0x00, // unicodeCode
            0x66, // typeAndFlags: INPUT_EVENT_SYNC, INPUT_SYNC_NUM_LOCK | INPUT_SYNC_CAPS_LOCK
        ]
    );
}

// oracle: spec MS-RDPECI §2.2.2.1
#[test]
fn decoder_rejects_a_wrong_signature() {
    let mut bytes = INIT_REQUEST;
    bytes[0] = 0x04;

    assert!(decode::<CoreInputPdu>(&bytes).is_err());
}

// oracle: spec MS-RDPECI §2.2.3.3
#[test]
fn input_pdu_holds_at_most_255_events() {
    let events = vec![FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x1E); 256];

    assert!(InputPdu::new(events[..255].to_vec()).is_ok());
    assert!(InputPdu::new(events).is_err());
}

// oracle: spec MS-RDPECI §3.3.3
#[test]
fn client_sends_init_request_on_open_and_waits_for_response() {
    let mut client = CoreInputClient::new();

    let messages = client.start(CHANNEL_ID).expect("start core input channel");
    let encoded = messages
        .iter()
        .map(|message| encode_vec(message.as_ref()).expect("encode DVC message"))
        .collect::<Vec<_>>();
    assert_eq!(encoded, [INIT_REQUEST.to_vec()]);
    assert!(!client.ready());
    assert!(client.encode_input(CHANNEL_ID, &spec_input_events()).is_err());

    let responses = client
        .process(CHANNEL_ID, &INIT_RESPONSE_CAPTURE)
        .expect("process init response");
    assert!(responses.is_empty());
    assert!(client.ready());
}

// oracle: spec MS-RDPECI §2.2.3.3
#[test]
fn client_splits_input_into_pdus_of_at_most_255_events() {
    let client = ready_client();
    let events = (0..=u8::MAX)
        .map(|code| FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), code))
        .chain([FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0)])
        .collect::<Vec<_>>();

    let messages = client.encode_input(CHANNEL_ID, &events).expect("encode 257 events");
    let decoded = messages
        .iter()
        .map(|message| match decode_svc_message(message) {
            CoreInputPdu::Input(pdu) => pdu.into_events(),
            other => panic!("expected an input PDU, got {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(decoded.iter().map(Vec::len).collect::<Vec<_>>(), [255, 2]);
    assert_eq!(decoded.concat(), events);
}

// oracle: spec MS-RDPECI §3.3.5.1
// The client advertises only version 1.0, so a response selecting another version leaves input on fast-path.
#[test]
fn client_is_not_ready_when_server_selects_another_version() {
    let mut client = CoreInputClient::new();
    client.start(CHANNEL_ID).expect("start core input channel");
    let mut response = INIT_RESPONSE_CAPTURE;
    response[5] = 0x02; // selectedProtocolVersion: 0x0200

    client.process(CHANNEL_ID, &response).expect("process init response");

    assert!(!client.ready());
}

// oracle: spec MS-RDPECI §3.1.5.1
#[test]
fn client_ignores_unexpected_messages() {
    let mut client = CoreInputClient::new();
    client.start(CHANNEL_ID).expect("start core input channel");

    let unknown_type = [0x03, 0x07, 0x00, 0x00];
    assert!(
        client
            .process(CHANNEL_ID, &unknown_type)
            .expect("ignore unknown type")
            .is_empty()
    );
    assert!(
        client
            .process(CHANNEL_ID, &INPUT)
            .expect("ignore client PDU")
            .is_empty()
    );
    assert!(!client.ready());
}

// oracle: spec MS-RDPECI §3.3.3
#[test]
fn client_requires_a_new_exchange_after_reopening() {
    let mut client = ready_client();

    client.close(CHANNEL_ID);
    assert!(!client.ready());
    client.start(CHANNEL_ID).expect("restart core input channel");
    assert!(!client.ready());
}

// oracle: spec MS-RDPECI §3.2.3
// oracle: spec MS-RDPECI §3.2.5.3
#[test]
fn server_answers_init_request_then_delivers_input() {
    let handler = RecordingHandler::default();
    let mut server = CoreInputServer::new(Box::new(handler.clone()));
    assert!(server.start(CHANNEL_ID).expect("start core input channel").is_empty());

    // Input before the version exchange is outside the running phase of section 1.3.
    server.process(CHANNEL_ID, &INPUT).expect("ignore early input");
    assert!(handler.0.lock().unwrap().is_empty());

    let responses = server.process(CHANNEL_ID, &INIT_REQUEST).expect("process init request");
    let encoded = responses
        .iter()
        .map(|message| encode_vec(message.as_ref()).expect("encode DVC message"))
        .collect::<Vec<_>>();
    assert_eq!(encoded, [INIT_RESPONSE_CAPTURE[..16].to_vec()]);

    server.process(CHANNEL_ID, &INPUT).expect("process input");
    assert_eq!(*handler.0.lock().unwrap(), spec_input_events());
}

// oracle: spec MS-RDPECI §2.2.3.2
// Version 1.0 is the only version a server can select, so a client range without it gets no response.
#[test]
fn server_does_not_answer_a_request_without_version_1_0() {
    let mut server = CoreInputServer::new(Box::new(RecordingHandler::default()));
    server.start(CHANNEL_ID).expect("start core input channel");
    let mut request = INIT_REQUEST;
    request[5] = 0x02; // protocolVersionMin: 0x0200
    request[7] = 0x02; // protocolVersionMax: 0x0200

    assert!(
        server
            .process(CHANNEL_ID, &request)
            .expect("process init request")
            .is_empty()
    );
}
