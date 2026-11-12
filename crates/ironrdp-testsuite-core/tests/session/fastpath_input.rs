//! Tests for `encode_fastpath_input`, the stage-free encoder behind `ActiveStage::process_fastpath_input`.

use ironrdp_core::{Decode as _, ReadCursor};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_pdu::input::MousePdu;
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent, KeyboardFlags};
use ironrdp_pdu::input::mouse::PointerFlags;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStageBuilder, ActiveStageOutput, encode_fastpath_input};
use ironrdp_svc::StaticChannelSet;

fn mouse_move(x: u16, y: u16) -> FastPathInputEvent {
    FastPathInputEvent::MouseEvent(MousePdu {
        flags: PointerFlags::MOVE,
        number_of_wheel_rotation_units: 0,
        x_position: x,
        y_position: y,
    })
}

/// A mouse move becomes one TS_FP_INPUT_PDU holding one TS_FP_POINTER_EVENT
/// ([MS-RDPBCGR] 2.2.8.1.2 and 2.2.8.1.2.2.3).
#[test]
fn mouse_move_encodes_to_the_spec_bytes() {
    let frames = encode_fastpath_input(&[mouse_move(10, 20)]).expect("encode one mouse move");

    assert_eq!(
        frames,
        [vec![
            0x04, // fpInputHeader: action FASTPATH_INPUT_ACTION_FASTPATH, numEvents 1
            0x09, // length1: 9 bytes
            0x20, // eventHeader: FASTPATH_INPUT_EVENT_MOUSE
            0x00, 0x08, // pointerFlags: PTRFLAGS_MOVE
            0x0a, 0x00, // xPos: 10
            0x14, 0x00, // yPos: 20
        ]]
    );
}

#[test]
fn no_events_encode_to_no_frames() {
    assert!(encode_fastpath_input(&[]).expect("encode nothing").is_empty());
}

/// numEvents is one byte at most, so a batch over `FastPathInput::MAX_EVENTS` spans several PDUs in order.
#[test]
fn a_large_batch_splits_at_the_event_limit() {
    let events = (0..=u16::from(u8::MAX))
        .map(|code| FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::empty(), code))
        .collect::<Vec<_>>();

    let frames = encode_fastpath_input(&events).expect("encode 256 events");
    let decoded = frames
        .iter()
        .map(|frame| FastPathInput::decode(&mut ReadCursor::new(frame)).expect("decode one PDU"))
        .collect::<Vec<_>>();

    assert_eq!(
        decoded.iter().map(|pdu| pdu.input_events().len()).collect::<Vec<_>>(),
        [FastPathInput::MAX_EVENTS, 1]
    );
    assert_eq!(
        decoded.iter().flat_map(FastPathInput::input_events).collect::<Vec<_>>(),
        events.iter().collect::<Vec<_>>()
    );
}

/// `ActiveStage::process_fastpath_input` sends exactly these frames, so a client that moves
/// input off the stage changes nothing on the wire.
#[test]
fn active_stage_sends_the_same_frames() {
    let mut stage = ActiveStageBuilder {
        static_channels: StaticChannelSet::new(),
        user_channel_id: 1001,
        io_channel_id: 1003,
        message_channel_id: None,
        share_id: 1,
        compression_type: None,
        enable_server_pointer: true,
        pointer_software_rendering: false,
    }
    .build();
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 8, 8);
    let events = [mouse_move(1, 1), mouse_move(5, 5)];

    let stage_frames = stage
        .process_fastpath_input(&mut image, &events)
        .expect("process through the stage")
        .into_iter()
        .map(|output| match output {
            ActiveStageOutput::ResponseFrame(frame) => frame,
            other => panic!("unexpected output {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        stage_frames,
        encode_fastpath_input(&events).expect("encode without the stage")
    );
}
