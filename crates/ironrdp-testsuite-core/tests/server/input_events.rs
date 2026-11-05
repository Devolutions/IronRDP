use ironrdp_ainput::{MouseEventFlags, MousePdu as AinputMousePdu};
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent};
use ironrdp_pdu::input::scan_code::{KeyboardFlags as ScanCodeFlags, ScanCodePdu};
use ironrdp_server::{KeyboardEvent, MouseButton, MouseEvent};
use rstest::rstest;

fn fast_path_keys(pdu: &[u8]) -> Vec<KeyboardEvent> {
    let input: FastPathInput = ironrdp_core::decode(pdu).expect("fast-path input PDU");
    input
        .input_events()
        .iter()
        .map(|event| match *event {
            FastPathInputEvent::KeyboardEvent(flags, key) => (key, flags).into(),
            other => panic!("unexpected event {other:?}"),
        })
        .collect()
}

// oracle: regression #2376
// oracle: spec MS-RDPBCGR §2.2.8.1.2.2.1 (eventFlags RELEASE = 0x01, EXTENDED1 = 0x04)
#[test]
fn fast_path_pause_keeps_the_e1_prefix() {
    // Header 0x10: fast-path action, four events; length 10.
    // Then E1 1D down, 45 down, E1 1D up, 45 up, each as eventHeader and keyCode.
    let events = fast_path_keys(&[0x10, 0x0a, 0x04, 0x1d, 0x00, 0x45, 0x05, 0x1d, 0x01, 0x45]);

    assert!(matches!(
        events.as_slice(),
        [
            KeyboardEvent::Pressed {
                code: 0x1d,
                extended: false,
                extended1: true
            },
            KeyboardEvent::Pressed {
                code: 0x45,
                extended: false,
                extended1: false
            },
            KeyboardEvent::Released {
                code: 0x1d,
                extended: false,
                extended1: true
            },
            KeyboardEvent::Released {
                code: 0x45,
                extended: false,
                extended1: false
            },
        ]
    ));
}

// oracle: spec MS-RDPBCGR §2.2.8.1.2.2.1 (eventFlags RELEASE = 0x01, EXTENDED1 = 0x04)
#[test]
fn fast_path_ctrl_and_num_lock_have_no_e1_prefix() {
    // The Pause sequence above without EXTENDED1: Ctrl down, Num Lock down, Ctrl up, Num Lock up.
    let events = fast_path_keys(&[0x10, 0x0a, 0x00, 0x1d, 0x00, 0x45, 0x01, 0x1d, 0x01, 0x45]);

    assert!(events.iter().all(|event| matches!(
        event,
        KeyboardEvent::Pressed { extended1: false, .. } | KeyboardEvent::Released { extended1: false, .. }
    )));
}

// oracle: regression #2376
// oracle: spec MS-RDPBCGR §2.2.8.1.1.3.1.1.1 (keyboardFlags KBDFLAGS_EXTENDED1 and KBDFLAGS_RELEASE)
#[test]
fn slow_path_scan_code_keeps_the_e1_prefix() {
    let down = ScanCodePdu {
        flags: ScanCodeFlags::EXTENDED_1,
        key_code: 0x1d,
    };
    let up = ScanCodePdu {
        flags: ScanCodeFlags::EXTENDED_1 | ScanCodeFlags::RELEASE,
        key_code: 0x1d,
    };

    let events: [KeyboardEvent; 2] = [(down.key_code, down.flags).into(), (up.key_code, up.flags).into()];

    assert!(matches!(
        events,
        [
            KeyboardEvent::Pressed {
                code: 0x1d,
                extended: false,
                extended1: true
            },
            KeyboardEvent::Released {
                code: 0x1d,
                extended: false,
                extended1: true
            },
        ]
    ));
}

// oracle: regression #2376
#[rstest]
#[case::x1_down(MouseEventFlags::XBUTTON1 | MouseEventFlags::DOWN, MouseButton::X1, true)]
#[case::x1_up(MouseEventFlags::XBUTTON1, MouseButton::X1, false)]
#[case::x2_down(MouseEventFlags::XBUTTON2 | MouseEventFlags::DOWN, MouseButton::X2, true)]
#[case::x2_up(MouseEventFlags::XBUTTON2, MouseButton::X2, false)]
fn ainput_side_buttons_are_x1_and_x2_at_the_pdu_position(
    #[case] flags: MouseEventFlags,
    #[case] expected_button: MouseButton,
    #[case] expected_pressed: bool,
) {
    let event: MouseEvent = AinputMousePdu {
        time: 0,
        flags,
        x: 100,
        y: 200,
    }
    .into();

    let MouseEvent::Button { x, y, button, pressed } = event else {
        panic!("{flags:?} decoded as {event:?}");
    };
    assert_eq!((x, y, button, pressed), (100, 200, expected_button, expected_pressed));
}
