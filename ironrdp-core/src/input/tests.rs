use lazy_static::lazy_static;

use super::fast_path::{FastPathInput, FastPathInputEvent};
use crate::input::mouse::{ButtonEvents, MovementEvents, WheelEvents};
use crate::input::MousePdu;
use crate::PduParsing;

const FASTPATH_INPUT_MESSAGE: [u8; 44] = [
    0x18, 0x2c, 0x20, 0x0, 0x90, 0x1a, 0x0, 0x26, 0x4, 0x20, 0x0, 0x8, 0x1b, 0x0, 0x26, 0x4, 0x20, 0x0, 0x10, 0x1b,
    0x0, 0x26, 0x4, 0x20, 0x0, 0x8, 0x1a, 0x0, 0x27, 0x4, 0x20, 0x0, 0x8, 0x19, 0x0, 0x27, 0x4, 0x20, 0x0, 0x8, 0x19,
    0x0, 0x28, 0x4,
];

lazy_static! {
    pub static ref FASTPATH_INPUT: FastPathInput = FastPathInput(vec![
        FastPathInputEvent::MouseEvent(MousePdu {
            wheel_events: WheelEvents::empty(),
            movement_events: MovementEvents::empty(),
            button_events: ButtonEvents::DOWN | ButtonEvents::LEFT_BUTTON,
            number_of_wheel_rotation_units: 0,
            x_position: 26,
            y_position: 1062
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            wheel_events: WheelEvents::empty(),
            movement_events: MovementEvents::MOVE,
            button_events: ButtonEvents::empty(),
            number_of_wheel_rotation_units: 0,
            x_position: 27,
            y_position: 1062
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            wheel_events: WheelEvents::empty(),
            movement_events: MovementEvents::empty(),
            button_events: ButtonEvents::LEFT_BUTTON,
            number_of_wheel_rotation_units: 0,
            x_position: 27,
            y_position: 1062
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            wheel_events: WheelEvents::empty(),
            movement_events: MovementEvents::MOVE,
            button_events: ButtonEvents::empty(),
            number_of_wheel_rotation_units: 0,
            x_position: 26,
            y_position: 1063
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            wheel_events: WheelEvents::empty(),
            movement_events: MovementEvents::MOVE,
            button_events: ButtonEvents::empty(),
            number_of_wheel_rotation_units: 0,
            x_position: 25,
            y_position: 1063
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            wheel_events: WheelEvents::empty(),
            movement_events: MovementEvents::MOVE,
            button_events: ButtonEvents::empty(),
            number_of_wheel_rotation_units: 0,
            x_position: 25,
            y_position: 1064
        })
    ]);
}

#[test]
fn from_buffer_correctly_parses_fastpath_input_message() {
    let mut buffer = FASTPATH_INPUT_MESSAGE.as_ref();

    assert_eq!(*FASTPATH_INPUT, FastPathInput::from_buffer(&mut buffer).unwrap());
    assert!(buffer.is_empty());
}

#[test]
fn to_buffer_correctly_serializes_fastpath_input_message() {
    let mut buffer = Vec::with_capacity(1024);
    FASTPATH_INPUT.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, FASTPATH_INPUT_MESSAGE.as_ref());
}
