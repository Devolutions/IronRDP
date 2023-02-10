use ironrdp_core::input::fast_path::{FastPathInputEvent, KeyboardFlags, SynchronizeFlags};
use ironrdp_core::input::mouse::{ButtonEvents, MovementEvents, WheelEvents};
use ironrdp_core::input::mouse_x::PointerFlags;
use ironrdp_core::input::{MousePdu, MouseXPdu};
use ironrdp_input::*;
use rstest::rstest;

enum MouseFlags {
    Button(ButtonEvents),
    Pointer(PointerFlags),
}

#[rstest]
#[case::left(MouseButton::LEFT, MouseFlags::Button(ButtonEvents::LEFT_BUTTON))]
#[case::middle(MouseButton::MIDDLE, MouseFlags::Button(ButtonEvents::MIDDLE_BUTTON_OR_WHEEL))]
#[case::right(MouseButton::RIGHT, MouseFlags::Button(ButtonEvents::RIGHT_BUTTON))]
#[case::x1(MouseButton::X1, MouseFlags::Pointer(PointerFlags::BUTTON1))]
#[case::x2(MouseButton::X2, MouseFlags::Pointer(PointerFlags::BUTTON2))]
fn mouse_buttons(#[case] button: MouseButton, #[case] expected_flag: MouseFlags) {
    let mut db = Database::default();

    {
        let packets = db.apply(std::iter::once(Operation::MouseButtonPressed(button)));
        let packet = packets.into_iter().next().expect("one input event");

        let expected_input_event = match expected_flag {
            MouseFlags::Button(flags) => FastPathInputEvent::MouseEvent(MousePdu {
                wheel_events: WheelEvents::empty(),
                movement_events: MovementEvents::empty(),
                button_events: flags | ButtonEvents::DOWN,
                number_of_wheel_rotations: 0,
                x_position: 0,
                y_position: 0,
            }),
            MouseFlags::Pointer(flags) => FastPathInputEvent::MouseEventEx(MouseXPdu {
                flags: flags | PointerFlags::DOWN,
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
                wheel_events: WheelEvents::empty(),
                movement_events: MovementEvents::empty(),
                button_events: flags,
                number_of_wheel_rotations: 0,
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
            Operation::KeyPressed(Scancode::from((0, false))),
            Operation::KeyPressed(Scancode::from((23, false))),
            Operation::KeyPressed(Scancode::from((39, false))),
            Operation::KeyPressed(Scancode::from((19, true))),
            Operation::KeyPressed(Scancode::from((20, true))),
            Operation::KeyPressed(Scancode::from((90, false))),
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
            Operation::KeyReleased(Scancode::from((0, false))),
            Operation::KeyReleased(Scancode::from((2, false))),
            Operation::KeyReleased(Scancode::from((3, false))),
            Operation::KeyReleased(Scancode::from((19, true))),
            Operation::KeyReleased(Scancode::from((20, true))),
            Operation::KeyReleased(Scancode::from((100, false))),
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
        Operation::KeyPressed(Scancode::from((0, false))),
        Operation::KeyPressed(Scancode::from((0, false))),
        Operation::KeyPressed(Scancode::from((0, false))),
        Operation::KeyPressed(Scancode::from((20, false))),
        Operation::KeyPressed(Scancode::from((90, false))),
        Operation::KeyPressed(Scancode::from((90, false))),
        Operation::KeyReleased(Scancode::from((90, false))),
        Operation::KeyReleased(Scancode::from((90, false))),
        Operation::KeyPressed(Scancode::from((20, false))),
        Operation::KeyReleased(Scancode::from((120, false))),
        Operation::KeyReleased(Scancode::from((90, false))),
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
        Operation::MouseButtonPressed(MouseButton::LEFT),
        Operation::MouseButtonPressed(MouseButton::LEFT),
        Operation::MouseButtonPressed(MouseButton::RIGHT),
        Operation::MouseButtonPressed(MouseButton::LEFT),
        Operation::MouseButtonPressed(MouseButton::LEFT),
        Operation::MouseButtonPressed(MouseButton::RIGHT),
        Operation::MouseButtonPressed(MouseButton::LEFT),
        Operation::MouseButtonReleased(MouseButton::RIGHT),
        Operation::MouseButtonPressed(MouseButton::RIGHT),
    ];

    let expected_inputs = [
        FastPathInputEvent::MouseEvent(MousePdu {
            wheel_events: WheelEvents::empty(),
            movement_events: MovementEvents::empty(),
            button_events: ButtonEvents::LEFT_BUTTON | ButtonEvents::DOWN,
            number_of_wheel_rotations: 0,
            x_position: 0,
            y_position: 0,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            wheel_events: WheelEvents::empty(),
            movement_events: MovementEvents::empty(),
            button_events: ButtonEvents::RIGHT_BUTTON | ButtonEvents::DOWN,
            number_of_wheel_rotations: 0,
            x_position: 0,
            y_position: 0,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            wheel_events: WheelEvents::empty(),
            movement_events: MovementEvents::empty(),
            button_events: ButtonEvents::RIGHT_BUTTON,
            number_of_wheel_rotations: 0,
            x_position: 0,
            y_position: 0,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            wheel_events: WheelEvents::empty(),
            movement_events: MovementEvents::empty(),
            button_events: ButtonEvents::RIGHT_BUTTON | ButtonEvents::DOWN,
            number_of_wheel_rotations: 0,
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
        Operation::KeyPressed(Scancode::from((0, false))),
        Operation::KeyPressed(Scancode::from((23, false))),
        Operation::KeyPressed(Scancode::from((39, false))),
        Operation::KeyPressed(Scancode::from((19, true))),
        Operation::KeyPressed(Scancode::from((20, true))),
        Operation::KeyPressed(Scancode::from((90, false))),
        Operation::MouseButtonPressed(MouseButton::LEFT),
        Operation::MouseButtonPressed(MouseButton::LEFT),
        Operation::MouseButtonPressed(MouseButton::RIGHT),
        Operation::MouseButtonPressed(MouseButton::LEFT),
        Operation::MouseButtonPressed(MouseButton::MIDDLE),
        Operation::MouseButtonPressed(MouseButton::RIGHT),
        Operation::MouseButtonPressed(MouseButton::LEFT),
        Operation::MouseButtonReleased(MouseButton::RIGHT),
    ];

    let _ = db.apply(ops);

    let expected_inputs = [
        FastPathInputEvent::MouseEvent(MousePdu {
            wheel_events: WheelEvents::empty(),
            movement_events: MovementEvents::empty(),
            button_events: ButtonEvents::LEFT_BUTTON,
            number_of_wheel_rotations: 0,
            x_position: 0,
            y_position: 0,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            wheel_events: WheelEvents::empty(),
            movement_events: MovementEvents::empty(),
            button_events: ButtonEvents::MIDDLE_BUTTON_OR_WHEEL,
            number_of_wheel_rotations: 0,
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
