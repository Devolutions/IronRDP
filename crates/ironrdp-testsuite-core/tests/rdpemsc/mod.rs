use std::sync::Arc;

use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::{DvcMessage, DvcProcessor as _};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_pdu::pointer::Point16;
use ironrdp_rdpemsc::client::MAX_PENDING_UPDATES;
use ironrdp_rdpemsc::{
    CapabilitySet, CapsAdvertisePdu, MouseCursorClient, MouseCursorPdu, MouseCursorServer, PointerShape, PointerUpdate,
};
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_svc::StaticChannelSet;
use proptest::prelude::*;
use rstest::rstest;

const CHANNEL_ID: u32 = 7;

/// MS-RDPEMSC 4.1.1.
const CAPS_ADVERTISE: [u8; 16] = [
    0x01, // pduType = PDUTYPE_CS_CAPS_ADVERTISE
    0x00, // updateType
    0x00, 0x00, // reserved
    0x43, 0x41, 0x50, 0x53, // signature = "CAPS"
    0x01, 0x00, 0x00, 0x00, // version = RDP_MOUSE_CURSOR_CAPVERSION_1
    0x0C, 0x00, 0x00, 0x00, // size
];

/// MS-RDPEMSC 4.1.2.
const CAPS_CONFIRM: [u8; 16] = [
    0x02, // pduType = PDUTYPE_SC_CAPS_CONFIRM
    0x00, // updateType
    0x00, 0x00, // reserved
    0x43, 0x41, 0x50, 0x53, // signature = "CAPS"
    0x01, 0x00, 0x00, 0x00, // version = RDP_MOUSE_CURSOR_CAPVERSION_1
    0x0C, 0x00, 0x00, 0x00, // size
];

/// MS-RDPEMSC 4.2.1.
const POSITION_UPDATE: [u8; 8] = [
    0x03, // pduType = PDUTYPE_SC_MOUSEPTR_UPDATE
    0x08, // updateType = TS_UPDATETYPE_MOUSEPTR_POSITION
    0x00, 0x00, // reserved
    0x78, 0x00, // xPos = 120
    0x64, 0x00, // yPos = 100
];

/// Fixed part of MS-RDPEMSC 4.2.2, before its 6912-byte XOR mask and 288-byte AND mask.
const POINTER_UPDATE_HEADER: [u8; 20] = [
    0x03, // pduType = PDUTYPE_SC_MOUSEPTR_UPDATE
    0x0B, // updateType = TS_UPDATETYPE_MOUSEPTR_POINTER
    0x00, 0x00, // reserved
    0x18, 0x00, // xorBpp = 24
    0x00, 0x00, // cacheIndex = 0
    0x0E, 0x00, 0x0F, 0x00, // hotSpot = (14, 15)
    0x30, 0x00, // width = 48
    0x30, 0x00, // height = 48
    0x20, 0x01, // lengthAndMask = 288
    0x00, 0x1B, // lengthXorMask = 6912
];

/// MS-RDPEMSC 4.2.2: a 48x48 24-bpp pointer with a zero XOR mask and an all-ones AND mask.
///
/// The example elides the mask bytes after the first row; they are filled as its annotations
/// describe (`00 00 00 00 …` and `ff ff ff ff …`).
fn pointer_update_bytes() -> Vec<u8> {
    let mut bytes = POINTER_UPDATE_HEADER.to_vec();
    bytes.extend([0x00; 6912]);
    bytes.extend([0xFF; 288]);
    bytes
}

fn pointer_update_pdu() -> PointerUpdate {
    PointerUpdate::Pointer(PointerShape {
        xor_bpp: 24,
        cache_index: 0,
        hot_spot: Point16 { x: 14, y: 15 },
        width: 48,
        height: 48,
        xor_mask: vec![0x00; 6912],
        and_mask: vec![0xFF; 288],
    })
}

fn encode_message(message: &DvcMessage) -> Vec<u8> {
    encode_vec(message.as_ref()).expect("encode DVC message")
}

fn started_client() -> MouseCursorClient {
    let mut client = MouseCursorClient::new();
    client.start(CHANNEL_ID).expect("start the channel");
    client
}

fn active_stage() -> ActiveStage {
    ActiveStageBuilder {
        static_channels: StaticChannelSet::new(),
        user_channel_id: 1002,
        io_channel_id: 1003,
        message_channel_id: None,
        share_id: 1,
        compression_type: None,
        enable_server_pointer: true,
        pointer_software_rendering: false,
    }
    .build()
}

// oracle: spec MS-RDPEMSC §4.1.1
// oracle: spec MS-RDPEMSC §3.3.3
#[test]
fn client_advertises_version_1_when_the_channel_opens() {
    let mut client = MouseCursorClient::new();

    let messages = client.start(CHANNEL_ID).expect("start the channel");

    assert_eq!(messages.len(), 1);
    assert_eq!(encode_message(&messages[0]), CAPS_ADVERTISE);
    assert!(!client.is_ready());
}

// oracle: spec MS-RDPEMSC §4.1.2
// oracle: spec MS-RDPEMSC §3.3.5.2
#[test]
fn client_is_ready_after_the_server_confirms() {
    let mut client = started_client();

    let replies = client
        .process(CHANNEL_ID, &CAPS_CONFIRM)
        .expect("process the confirmation");

    assert!(replies.is_empty());
    assert!(client.is_ready());
}

// oracle: spec MS-RDPEMSC §4.2.1
#[test]
fn client_queues_a_position_update() {
    let mut client = started_client();

    client
        .process(CHANNEL_ID, &POSITION_UPDATE)
        .expect("process the update");

    assert_eq!(
        client.take_updates(),
        [PointerUpdate::Position(Point16 { x: 120, y: 100 })]
    );
    assert!(client.take_updates().is_empty());
}

// oracle: spec MS-RDPEMSC §3.1.5.1
#[test]
fn client_ignores_an_unexpected_message() {
    let mut client = started_client();

    let replies = client
        .process(CHANNEL_ID, &CAPS_ADVERTISE)
        .expect("an unexpected message is not an error");

    assert!(replies.is_empty());
    assert!(client.take_updates().is_empty());
    assert!(!client.is_ready());
}

// oracle: spec MS-RDPEMSC §3.1.5.1
#[test]
fn client_ignores_an_unknown_update_type() {
    let mut client = started_client();
    // pduType = PDUTYPE_SC_MOUSEPTR_UPDATE, updateType = 0x07 (FASTPATH_UPDATETYPE_COLOR, not defined here).
    let update = [0x03, 0x07, 0x00, 0x00];

    client
        .process(CHANNEL_ID, &update)
        .expect("a malformed message is not an error");

    assert!(client.take_updates().is_empty());
}

// oracle: mutant keeping the confirmation or the pending updates across a channel close; a reopened channel restarts the Initializing Phase
#[test]
fn closing_the_channel_resets_the_client() {
    let mut client = started_client();
    client
        .process(CHANNEL_ID, &CAPS_CONFIRM)
        .expect("process the confirmation");
    client
        .process(CHANNEL_ID, &POSITION_UPDATE)
        .expect("process the update");

    client.close(CHANNEL_ID);

    assert!(!client.is_ready());
    assert!(client.take_updates().is_empty());
}

/// A 1x1 shape that `tag` tells apart from other shapes.
fn small_shape(cache_index: u16, tag: u8) -> PointerShape {
    PointerShape {
        xor_bpp: 24,
        cache_index,
        hot_spot: Point16 { x: 0, y: 0 },
        width: 1,
        height: 1,
        xor_mask: vec![tag, 0],
        and_mask: vec![0, 0],
    }
}

fn feed(client: &mut MouseCursorClient, updates: impl IntoIterator<Item = PointerUpdate>) {
    for update in updates {
        let pdu = MouseCursorPdu::PointerUpdate(update);
        client
            .process(CHANNEL_ID, &encode_vec(&pdu).expect("encode update"))
            .expect("process the update");
    }
}

/// Pointer state after applying `updates` in order, per MS-RDPEMSC §3.3.5.3.
#[derive(Debug, Default, PartialEq)]
struct PointerModel {
    position: Option<Point16>,
    /// `None` when hidden, default or set from an empty cache slot.
    visible: Option<PointerShape>,
    cache: std::collections::BTreeMap<u16, PointerShape>,
}

impl PointerModel {
    fn apply(updates: &[PointerUpdate]) -> Self {
        let mut model = Self::default();
        for update in updates {
            match update {
                PointerUpdate::Hidden | PointerUpdate::Default => model.visible = None,
                PointerUpdate::Position(position) => model.position = Some(*position),
                PointerUpdate::Cached { cache_index } => model.visible = model.cache.get(cache_index).cloned(),
                PointerUpdate::Pointer(shape) | PointerUpdate::LargePointer(shape) => {
                    model.cache.insert(shape.cache_index, shape.clone());
                    model.visible = Some(shape.clone());
                }
            }
        }
        model
    }
}

// oracle: mutant dropping the newest update, or growing past the bound, when updates are not collected
#[test]
fn client_keeps_the_last_position_past_the_bound() {
    let mut client = started_client();
    let count = u16::try_from(MAX_PENDING_UPDATES + 1).expect("fits");

    feed(
        &mut client,
        (0..count).map(|x| PointerUpdate::Position(Point16 { x, y: 0 })),
    );

    let updates = client.take_updates();
    assert!(updates.len() <= MAX_PENDING_UPDATES);
    assert_eq!(
        updates.last(),
        Some(&PointerUpdate::Position(Point16 { x: count - 1, y: 0 }))
    );
}

// oracle: spec MS-RDPEMSC §3.3.5.3
#[test]
fn client_keeps_a_shape_that_a_later_cached_update_reads_past_the_bound() {
    let mut client = started_client();
    let shape = small_shape(0, 1);
    let count = u16::try_from(MAX_PENDING_UPDATES).expect("fits");

    feed(
        &mut client,
        core::iter::once(PointerUpdate::Pointer(shape.clone()))
            .chain((0..count).map(|x| PointerUpdate::Position(Point16 { x, y: 0 })))
            .chain(core::iter::once(PointerUpdate::Cached { cache_index: 0 })),
    );

    let updates = client.take_updates();
    // oracle: mutant growing past the bound
    assert!(updates.len() <= MAX_PENDING_UPDATES);
    assert_eq!(PointerModel::apply(&updates).visible, Some(shape));
}

// oracle: mutant growing past the bound when every update fills a different cache slot
#[test]
fn client_drops_the_oldest_shape_when_the_slots_exceed_the_bound() {
    let mut client = started_client();
    let count = u16::try_from(MAX_PENDING_UPDATES + 1).expect("fits");

    feed(
        &mut client,
        (0..count).map(|cache_index| PointerUpdate::Pointer(small_shape(cache_index, 0))),
    );

    let updates = client.take_updates();
    assert_eq!(updates.len(), MAX_PENDING_UPDATES);
    assert_eq!(updates[0], PointerUpdate::Pointer(small_shape(1, 0)));
}

// oracle: spec MS-RDPEMSC §4.1.2
// oracle: spec MS-RDPEMSC §3.2.5.1
#[test]
fn server_confirms_version_1() {
    let mut server = MouseCursorServer::new();
    assert!(server.start(CHANNEL_ID).expect("start the channel").is_empty());

    let replies = server
        .process(CHANNEL_ID, &CAPS_ADVERTISE)
        .expect("process the advertisement");

    assert_eq!(replies.len(), 1);
    assert_eq!(encode_message(&replies[0]), CAPS_CONFIRM);
    assert!(server.is_ready());
}

// oracle: spec MS-RDPEMSC §3.2.5.2
#[test]
fn server_does_not_confirm_unsupported_capabilities() {
    let mut server = MouseCursorServer::new();
    server.start(CHANNEL_ID).expect("start the channel");
    let advertise = [
        0x01, 0x00, 0x00, 0x00, // header, pduType = PDUTYPE_CS_CAPS_ADVERTISE
        0x43, 0x41, 0x50, 0x53, // signature = "CAPS"
        0x02, 0x00, 0x00, 0x00, // version = 2, not defined
        0x0C, 0x00, 0x00, 0x00, // size
    ];

    let replies = server
        .process(CHANNEL_ID, &advertise)
        .expect("process the advertisement");

    assert!(replies.is_empty());
    assert!(!server.is_ready());
}

// oracle: spec MS-RDPEMSC §4.2.2
#[test]
fn pointer_update_matches_the_specification_example() {
    let bytes = pointer_update_bytes();

    assert_eq!(
        decode::<MouseCursorPdu>(&bytes).expect("decode the update"),
        MouseCursorPdu::PointerUpdate(pointer_update_pdu())
    );
    assert_eq!(
        encode_vec(&MouseCursorPdu::PointerUpdate(pointer_update_pdu())).expect("encode the update"),
        bytes
    );
}

// oracle: spec MS-RDPEMSC §2.2.2.5
#[test]
fn pointer_update_ignores_the_pad_byte() {
    // TS_POINTERATTRIBUTE ends with an optional 1-byte `pad` whose value MUST be ignored.
    let mut bytes = pointer_update_bytes();
    bytes.push(0x00);

    assert_eq!(
        decode::<MouseCursorPdu>(&bytes).expect("decode the update"),
        MouseCursorPdu::PointerUpdate(pointer_update_pdu())
    );
}

// oracle: spec MS-RDPEMSC §2.2.2.6
#[test]
fn large_pointer_update_has_32_bit_mask_lengths() {
    let bytes = [
        0x03, // pduType = PDUTYPE_SC_MOUSEPTR_UPDATE
        0x0C, // updateType = TS_UPDATETYPE_MOUSEPTR_LARGE_POINTER
        0x00, 0x00, // reserved
        0x20, 0x00, // xorBpp = 32
        0x01, 0x00, // cacheIndex = 1
        0x02, 0x00, 0x03, 0x00, // hotSpot = (2, 3)
        0x02, 0x00, // width = 2
        0x01, 0x00, // height = 1
        0x02, 0x00, 0x00, 0x00, // lengthAndMask = 2
        0x08, 0x00, 0x00, 0x00, // lengthXorMask = 8
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, // xorMaskData
        0x40, 0x00, // andMaskData
    ];
    let update = MouseCursorPdu::PointerUpdate(PointerUpdate::LargePointer(PointerShape {
        xor_bpp: 32,
        cache_index: 1,
        hot_spot: Point16 { x: 2, y: 3 },
        width: 2,
        height: 1,
        xor_mask: vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
        and_mask: vec![0x40, 0x00],
    }));

    assert_eq!(decode::<MouseCursorPdu>(&bytes).expect("decode the update"), update);
    assert_eq!(encode_vec(&update).expect("encode the update"), bytes);
}

// oracle: spec MS-RDPEMSC §2.2.3.3
#[rstest]
// updateType = TS_UPDATETYPE_MOUSEPTR_SYSTEM_NULL, no payload.
#[case::hidden(PointerUpdate::Hidden, &[0x03, 0x05, 0x00, 0x00])]
// updateType = TS_UPDATETYPE_MOUSEPTR_SYSTEM_DEFAULT, no payload.
#[case::default(PointerUpdate::Default, &[0x03, 0x06, 0x00, 0x00])]
// updateType = TS_UPDATETYPE_MOUSEPTR_CACHED, cachedPointerIndex = 5.
#[case::cached(PointerUpdate::Cached { cache_index: 5 }, &[0x03, 0x0A, 0x00, 0x00, 0x05, 0x00])]
fn update_without_a_shape_has_the_specified_layout(#[case] update: PointerUpdate, #[case] bytes: &[u8]) {
    let pdu = MouseCursorPdu::PointerUpdate(update);

    assert_eq!(decode::<MouseCursorPdu>(bytes).expect("decode the update"), pdu);
    assert_eq!(encode_vec(&pdu).expect("encode the update"), bytes);
}

// oracle: spec MS-RDPEMSC §2.2.2.3.1
#[test]
fn version_1_capability_set_must_be_12_bytes() {
    let confirm = [
        0x02, 0x00, 0x00, 0x00, // header, pduType = PDUTYPE_SC_CAPS_CONFIRM
        0x43, 0x41, 0x50, 0x53, // signature = "CAPS"
        0x01, 0x00, 0x00, 0x00, // version = RDP_MOUSE_CURSOR_CAPVERSION_1
        0x10, 0x00, 0x00, 0x00, // size = 16
        0x00, 0x00, 0x00, 0x00, // capsData
    ];

    decode::<MouseCursorPdu>(&confirm).expect_err("a version 1 capability set has no data");
}

// oracle: spec MS-RDPEMSC §2.2.2.3.1
#[test]
fn version_1_capability_set_with_data_is_not_encoded() {
    let confirm = MouseCursorPdu::CapsConfirm(ironrdp_rdpemsc::CapsConfirmPdu {
        caps_set: CapabilitySet {
            version: ironrdp_rdpemsc::pdu::CAPVERSION_1,
            data: vec![1],
        },
    });

    encode_vec(&confirm).expect_err("a version 1 capability set has no data");
}

// oracle: spec MS-RDPEMSC §2.2.2.2
#[test]
fn capability_set_signature_is_checked() {
    let mut confirm = CAPS_CONFIRM;
    confirm[4] = b'X';

    decode::<MouseCursorPdu>(&confirm).expect_err("the signature must be CAPS");
}

// oracle: spec MS-RDPEMSC §3.3.5.3
#[test]
fn active_stage_caches_and_reuses_a_channel_pointer() {
    let mut stage = active_stage();
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 64, 64);

    let shape_outputs = stage
        .process_pointer_update(&mut image, pointer_update_pdu().as_pointer_update_data())
        .expect("apply the shape");
    let [ActiveStageOutput::PointerBitmap(pointer)] = shape_outputs.as_slice() else {
        panic!("expected one pointer bitmap, got {shape_outputs:?}");
    };
    // The image is the one MS-RDPEMSC 4.2.2 describes.
    assert_eq!((pointer.width, pointer.height), (48, 48));
    assert_eq!((pointer.hotspot_x, pointer.hotspot_y), (14, 15));

    let cached_outputs = stage
        .process_pointer_update(
            &mut image,
            PointerUpdate::Cached { cache_index: 0 }.as_pointer_update_data(),
        )
        .expect("apply the cached pointer");
    assert!(
        cached_outputs
            .iter()
            .any(|output| matches!(output, ActiveStageOutput::PointerBitmap(cached) if Arc::ptr_eq(cached, pointer))),
        "the cached pointer must be the image stored in its slot, got {cached_outputs:?}"
    );
}

// oracle: spec MS-RDPEMSC §4.2.1
#[test]
fn active_stage_moves_the_pointer() {
    let mut stage = active_stage();
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 64, 64);
    let update = PointerUpdate::Position(Point16 { x: 120, y: 100 });

    let outputs = stage
        .process_pointer_update(&mut image, update.as_pointer_update_data())
        .expect("apply the position");

    assert!(
        matches!(
            outputs.as_slice(),
            [ActiveStageOutput::PointerPosition { x: 120, y: 100 }]
        ),
        "got {outputs:?}"
    );
}

fn even_stride_mask(height: u16) -> impl Strategy<Value = Vec<u8>> {
    (1usize..=4)
        .prop_flat_map(move |half_stride| proptest::collection::vec(any::<u8>(), half_stride * 2 * usize::from(height)))
}

fn pointer_shape() -> impl Strategy<Value = PointerShape> {
    (1u16..=8).prop_flat_map(|height| {
        (
            any::<u16>(),
            any::<u16>(),
            any::<(u16, u16)>(),
            any::<u16>(),
            even_stride_mask(height),
            even_stride_mask(height),
        )
            .prop_map(
                move |(xor_bpp, cache_index, (x, y), width, xor_mask, and_mask)| PointerShape {
                    xor_bpp,
                    cache_index,
                    hot_spot: Point16 { x, y },
                    width,
                    height,
                    xor_mask,
                    and_mask,
                },
            )
    })
}

fn pointer_update() -> impl Strategy<Value = PointerUpdate> {
    prop_oneof![
        Just(PointerUpdate::Hidden),
        Just(PointerUpdate::Default),
        any::<(u16, u16)>().prop_map(|(x, y)| PointerUpdate::Position(Point16 { x, y })),
        any::<u16>().prop_map(|cache_index| PointerUpdate::Cached { cache_index }),
        pointer_shape().prop_map(PointerUpdate::Pointer),
        pointer_shape().prop_map(PointerUpdate::LargePointer),
    ]
}

fn mouse_cursor_pdu() -> impl Strategy<Value = MouseCursorPdu> {
    let caps_set = (any::<u32>(), proptest::collection::vec(any::<u8>(), 0..8)).prop_map(|(version, data)| {
        if version == ironrdp_rdpemsc::pdu::CAPVERSION_1 {
            CapabilitySet::version1()
        } else {
            CapabilitySet { version, data }
        }
    });
    prop_oneof![
        proptest::collection::vec(caps_set.clone(), 0..4)
            .prop_map(|caps_sets| MouseCursorPdu::CapsAdvertise(CapsAdvertisePdu { caps_sets })),
        caps_set.prop_map(|caps_set| MouseCursorPdu::CapsConfirm(ironrdp_rdpemsc::CapsConfirmPdu { caps_set })),
        pointer_update().prop_map(MouseCursorPdu::PointerUpdate),
    ]
}

fn queued_update() -> impl Strategy<Value = PointerUpdate> {
    // Mostly positions, so that a shape often has no later shape for its slot and dropping it
    // would lose cache state; few slots, so that later shapes still override earlier ones.
    prop_oneof![
        1 => Just(PointerUpdate::Hidden),
        1 => Just(PointerUpdate::Default),
        200 => (0u16..4).prop_map(|x| PointerUpdate::Position(Point16 { x, y: 0 })),
        4 => (0u16..8).prop_map(|cache_index| PointerUpdate::Cached { cache_index }),
        1 => (0u16..8, any::<u8>()).prop_map(|(cache_index, tag)| PointerUpdate::Pointer(small_shape(cache_index, tag))),
        1 => (0u16..8, any::<u8>())
            .prop_map(|(cache_index, tag)| PointerUpdate::LargePointer(small_shape(cache_index, tag))),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn client_queue_bound_keeps_the_pointer_state(
        updates in proptest::collection::vec(queued_update(), MAX_PENDING_UPDATES..MAX_PENDING_UPDATES * 3),
    ) {
        let mut client = started_client();
        feed(&mut client, updates.iter().cloned());

        let queued = client.take_updates();
        // oracle: mutant growing past the bound
        prop_assert!(queued.len() <= MAX_PENDING_UPDATES);
        // oracle: spec MS-RDPEMSC section 3.3.5.3
        prop_assert_eq!(PointerModel::apply(&queued), PointerModel::apply(&updates));
    }
}

proptest! {
    // oracle: property round-trip
    #[test]
    fn pdu_round_trips(pdu in mouse_cursor_pdu()) {
        let encoded = encode_vec(&pdu).expect("encode the PDU");
        prop_assert_eq!(encoded.len(), ironrdp_core::Encode::size(&pdu));
        prop_assert_eq!(decode::<MouseCursorPdu>(&encoded).expect("decode the PDU"), pdu);
    }
}
