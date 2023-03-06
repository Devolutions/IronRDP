use ironrdp_core::input::fast_path::{FastPathInputEvent, KeyboardFlags, SynchronizeFlags};
use ironrdp_core::input::mouse::PointerFlags;
use ironrdp_core::input::mouse_x::PointerXFlags;
use ironrdp_core::input::{MousePdu, MouseXPdu};
use ironrdp_input::*;
use rstest::rstest;

enum MouseFlags {
    Button(PointerFlags),
    Pointer(PointerXFlags),
}

#[rstest]
#[case::left(MouseButton::Left, MouseFlags::Button(PointerFlags::LEFT_BUTTON))]
#[case::middle(MouseButton::Middle, MouseFlags::Button(PointerFlags::MIDDLE_BUTTON_OR_WHEEL))]
#[case::right(MouseButton::Right, MouseFlags::Button(PointerFlags::RIGHT_BUTTON))]
#[case::x1(MouseButton::X1, MouseFlags::Pointer(PointerXFlags::BUTTON1))]
#[case::x2(MouseButton::X2, MouseFlags::Pointer(PointerXFlags::BUTTON2))]
fn mouse_buttons(#[case] button: MouseButton, #[case] expected_flag: MouseFlags) {
    let mut db = Database::default();

    {
        let packets = db.apply(std::iter::once(Operation::MouseButtonPressed(button)));
        let packet = packets.into_iter().next().expect("one input event");

        let expected_input_event = match expected_flag {
            MouseFlags::Button(flags) => FastPathInputEvent::MouseEvent(MousePdu {
                flags: flags | PointerFlags::DOWN,
                number_of_wheel_rotation_units: 0,
                x_position: 0,
                y_position: 0,
            }),
            MouseFlags::Pointer(flags) => FastPathInputEvent::MouseEventEx(MouseXPdu {
                flags: flags | PointerXFlags::DOWN,
                x_position: 0,
                y_position: 0,
            }),
        };

        assert_eq!(packet, expected_input_event);
    }

    {
        let packets = db.apply(std::iter::once(Operation::MouseButtonReleased(button)));
        let packet = packets.into_iter().next().expect("one input event");

        let expected_input_event = match expected_flag {
            MouseFlags::Button(flags) => FastPathInputEvent::MouseEvent(MousePdu {
                flags,
                number_of_wheel_rotation_units: 0,
                x_position: 0,
                y_position: 0,
            }),
            MouseFlags::Pointer(flags) => FastPathInputEvent::MouseEventEx(MouseXPdu {
                flags,
                x_position: 0,
                y_position: 0,
            }),
        };

        assert_eq!(packet, expected_input_event);
    }
}

#[test]
fn keyboard() {
    let mut db = Database::default();

    {
        let to_press = [
            Operation::KeyPressed(Scancode::from_u8(false, 0)),
            Operation::KeyPressed(Scancode::from_u8(false, 23)),
            Operation::KeyPressed(Scancode::from_u8(false, 39)),
            Operation::KeyPressed(Scancode::from_u8(true, 19)),
            Operation::KeyPressed(Scancode::from_u8(true, 20)),
            Operation::KeyPressed(Scancode::from_u8(false, 90)),
        ];

        let expected_inputs = [
            FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0),
            FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 23),
            FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 39),
            FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED, 19),
            FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED, 20),
            FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 90),
        ];

        let mut expected_keyboard_state = KeyboardState::ZERO;
        expected_keyboard_state.set(0, true);
        expected_keyboard_state.set(23, true);
        expected_keyboard_state.set(39, true);
        expected_keyboard_state.set(256 + 19, true);
        expected_keyboard_state.set(256 + 20, true);
        expected_keyboard_state.set(90, true);

        let actual_inputs = db.apply(to_press);
        let actual_keyboard_state = db.keyboard_state();

        assert_eq!(actual_inputs.as_slice(), expected_inputs.as_slice());
        assert_eq!(*actual_keyboard_state, expected_keyboard_state);
    }

    {
        let to_press = [
            Operation::KeyReleased(Scancode::from_u8(false, 0)),
            Operation::KeyReleased(Scancode::from_u8(false, 2)),
            Operation::KeyReleased(Scancode::from_u8(false, 3)),
            Operation::KeyReleased(Scancode::from_u8(true, 19)),
            Operation::KeyReleased(Scancode::from_u8(true, 20)),
            Operation::KeyReleased(Scancode::from_u8(false, 100)),
        ];

        let expected_inputs = [
            FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0),
            FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE | KeyboardFlags::EXTENDED, 19),
            FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE | KeyboardFlags::EXTENDED, 20),
        ];

        let mut expected_keyboard_state = KeyboardState::ZERO;
        expected_keyboard_state.set(23, true);
        expected_keyboard_state.set(39, true);
        expected_keyboard_state.set(90, true);

        let actual_inputs = db.apply(to_press);
        let actual_keyboard_state = db.keyboard_state();

        assert_eq!(actual_inputs.as_slice(), expected_inputs.as_slice());
        assert_eq!(*actual_keyboard_state, expected_keyboard_state);
    }
}

#[test]
fn keyboard_repeat() {
    let mut db = Database::default();

    let to_press = [
        Operation::KeyPressed(Scancode::from_u8(false, 0)),
        Operation::KeyPressed(Scancode::from_u8(false, 0)),
        Operation::KeyPressed(Scancode::from_u8(false, 0)),
        Operation::KeyPressed(Scancode::from_u8(false, 20)),
        Operation::KeyPressed(Scancode::from_u8(false, 90)),
        Operation::KeyPressed(Scancode::from_u8(false, 90)),
        Operation::KeyReleased(Scancode::from_u8(false, 90)),
        Operation::KeyReleased(Scancode::from_u8(false, 90)),
        Operation::KeyPressed(Scancode::from_u8(false, 20)),
        Operation::KeyReleased(Scancode::from_u8(false, 120)),
        Operation::KeyReleased(Scancode::from_u8(false, 90)),
    ];

    let expected_inputs = [
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 20),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 90),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 90),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 90),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 90),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 20),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 20),
    ];

    let actual_inputs = db.apply(to_press);

    assert_eq!(actual_inputs.as_slice(), expected_inputs.as_slice());
}

#[test]
fn mouse_button_no_duplicate() {
    let mut db = Database::default();

    let to_press = [
        Operation::MouseButtonPressed(MouseButton::Left),
        Operation::MouseButtonPressed(MouseButton::Left),
        Operation::MouseButtonPressed(MouseButton::Right),
        Operation::MouseButtonPressed(MouseButton::Left),
        Operation::MouseButtonPressed(MouseButton::Left),
        Operation::MouseButtonPressed(MouseButton::Right),
        Operation::MouseButtonPressed(MouseButton::Left),
        Operation::MouseButtonReleased(MouseButton::Right),
        Operation::MouseButtonPressed(MouseButton::Right),
    ];

    let expected_inputs = [
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::LEFT_BUTTON | PointerFlags::DOWN,
            number_of_wheel_rotation_units: 0,
            x_position: 0,
            y_position: 0,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::RIGHT_BUTTON | PointerFlags::DOWN,
            number_of_wheel_rotation_units: 0,
            x_position: 0,
            y_position: 0,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::RIGHT_BUTTON,
            number_of_wheel_rotation_units: 0,
            x_position: 0,
            y_position: 0,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::RIGHT_BUTTON | PointerFlags::DOWN,
            number_of_wheel_rotation_units: 0,
            x_position: 0,
            y_position: 0,
        }),
    ];

    let actual_inputs = db.apply(to_press);

    assert_eq!(actual_inputs.as_slice(), expected_inputs.as_slice());
}

#[test]
fn release_all() {
    let mut db = Database::default();

    let ops = [
        Operation::KeyPressed(Scancode::from_u8(false, 0)),
        Operation::KeyPressed(Scancode::from_u8(false, 23)),
        Operation::KeyPressed(Scancode::from_u8(false, 39)),
        Operation::KeyPressed(Scancode::from_u8(true, 19)),
        Operation::KeyPressed(Scancode::from_u8(true, 20)),
        Operation::KeyPressed(Scancode::from_u8(false, 90)),
        Operation::MouseButtonPressed(MouseButton::Left),
        Operation::MouseButtonPressed(MouseButton::Left),
        Operation::MouseButtonPressed(MouseButton::Right),
        Operation::MouseButtonPressed(MouseButton::Left),
        Operation::MouseButtonPressed(MouseButton::Middle),
        Operation::MouseButtonPressed(MouseButton::Right),
        Operation::MouseButtonPressed(MouseButton::Left),
        Operation::MouseButtonReleased(MouseButton::Right),
    ];

    let _ = db.apply(ops);

    let expected_inputs = [
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::LEFT_BUTTON,
            number_of_wheel_rotation_units: 0,
            x_position: 0,
            y_position: 0,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::MIDDLE_BUTTON_OR_WHEEL,
            number_of_wheel_rotation_units: 0,
            x_position: 0,
            y_position: 0,
        }),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 23),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 39),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 90),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE | KeyboardFlags::EXTENDED, 19),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE | KeyboardFlags::EXTENDED, 20),
    ];

    let actual_inputs = db.release_all();

    assert_eq!(actual_inputs.as_slice(), expected_inputs.as_slice());
}

#[rstest]
#[case(true, false, true, false, SynchronizeFlags::SCROLL_LOCK | SynchronizeFlags::CAPS_LOCK)]
#[case(true, true, true, false, SynchronizeFlags::SCROLL_LOCK | SynchronizeFlags::NUM_LOCK | SynchronizeFlags::CAPS_LOCK)]
#[case(false, false, false, true, SynchronizeFlags::KANA_LOCK)]
fn sync_lock_keys(
    #[case] scroll_lock: bool,
    #[case] num_lock: bool,
    #[case] caps_lock: bool,
    #[case] kana_lock: bool,
    #[case] expected_flags: SynchronizeFlags,
) {
    let event = synchronize_event(scroll_lock, num_lock, caps_lock, kana_lock);

    let FastPathInputEvent::SyncEvent(actual_flags) = event else {
        panic!("Unexpected fast path input event");
    };

    assert_eq!(actual_flags, expected_flags);
}

#[test]
fn wheel_rotations() {
    let mut db = Database::default();

    let ops = [
        Operation::WheelRotations(WheelRotations {
            is_vertical: false,
            rotation_units: 2,
        }),
        Operation::WheelRotations(WheelRotations {
            is_vertical: true,
            rotation_units: -1,
        }),
        Operation::WheelRotations(WheelRotations {
            is_vertical: false,
            rotation_units: -1,
        }),
    ];

    let actual_inputs = db.apply(ops);

    let expected_inputs = [
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::HORIZONTAL_WHEEL,
            number_of_wheel_rotation_units: 2,
            x_position: 0,
            y_position: 0,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::VERTICAL_WHEEL,
            number_of_wheel_rotation_units: -1,
            x_position: 0,
            y_position: 0,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::HORIZONTAL_WHEEL,
            number_of_wheel_rotation_units: -1,
            x_position: 0,
            y_position: 0,
        }),
    ];

    assert_eq!(actual_inputs.as_slice(), expected_inputs.as_slice());
}
