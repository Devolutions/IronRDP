use std::sync::LazyLock;

use ironrdp_core::{ReadCursor, decode_cursor, encode_vec};
use ironrdp_pdu::input::MousePdu;
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent};
use ironrdp_pdu::input::mouse::PointerFlags;

const FASTPATH_INPUT_MESSAGE: [u8; 44] = [
    0x18, 0x2c, 0x20, 0x0, 0x90, 0x1a, 0x0, 0x26, 0x4, 0x20, 0x0, 0x8, 0x1b, 0x0, 0x26, 0x4, 0x20, 0x0, 0x10, 0x1b,
    0x0, 0x26, 0x4, 0x20, 0x0, 0x8, 0x1a, 0x0, 0x27, 0x4, 0x20, 0x0, 0x8, 0x19, 0x0, 0x27, 0x4, 0x20, 0x0, 0x8, 0x19,
    0x0, 0x28, 0x4,
];

static FASTPATH_INPUT: LazyLock<FastPathInput> = LazyLock::new(|| {
    FastPathInput::new(vec![
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::DOWN | PointerFlags::LEFT_BUTTON,
            number_of_wheel_rotation_units: 0,
            x_position: 26,
            y_position: 1062,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::MOVE,
            number_of_wheel_rotation_units: 0,
            x_position: 27,
            y_position: 1062,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::LEFT_BUTTON,
            number_of_wheel_rotation_units: 0,
            x_position: 27,
            y_position: 1062,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::MOVE,
            number_of_wheel_rotation_units: 0,
            x_position: 26,
            y_position: 1063,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::MOVE,
            number_of_wheel_rotation_units: 0,
            x_position: 25,
            y_position: 1063,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::MOVE,
            number_of_wheel_rotation_units: 0,
            x_position: 25,
            y_position: 1064,
        }),
    ])
    .expect("can't panic")
});

#[test]
fn from_buffer_correctly_parses_fastpath_input_message() {
    let buffer = FASTPATH_INPUT_MESSAGE.as_ref();

    let mut cursor = ReadCursor::new(buffer);
    assert_eq!(*FASTPATH_INPUT, decode_cursor(&mut cursor).unwrap());
    assert!(cursor.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_fastpath_input_message() {
    let buffer = encode_vec(&*FASTPATH_INPUT).unwrap();

    assert_eq!(buffer, FASTPATH_INPUT_MESSAGE.as_ref());
}

#[test]
fn relative_mouse_events_carry_their_body_and_keep_batches_aligned() {
    use ironrdp_pdu::input::fast_path::SynchronizeFlags;
    use ironrdp_pdu::input::mouse_rel::{MouseRelPdu, PointerRelFlags};

    // oracle: spec MS-RDPBCGR 2.2.8.1.2.2.7
    // TS_FP_RELPOINTER_EVENT is the event header
    // (FASTPATH_INPUT_EVENT_MOUSEREL = 0x5 in bits 5-7) followed by pointerFlags, xDelta and yDelta,
    // each a little-endian 16-bit field.
    let relative = FastPathInputEvent::MouseEventRel(MouseRelPdu {
        flags: PointerRelFlags::MOVE,
        x_delta: 5,
        y_delta: -7,
    });
    assert_eq!(
        encode_vec(&relative).unwrap(),
        [0xA0, 0x00, 0x08, 0x05, 0x00, 0xF9, 0xFF]
    );

    // A following event must start right after the relative event's body.
    let batch = FastPathInput::new(vec![
        relative,
        FastPathInputEvent::SyncEvent(SynchronizeFlags::SCROLL_LOCK),
    ])
    .unwrap();
    let encoded = encode_vec(&batch).unwrap();
    let decoded: FastPathInput = decode_cursor(&mut ReadCursor::new(&encoded)).unwrap();
    assert_eq!(decoded, batch);
}
