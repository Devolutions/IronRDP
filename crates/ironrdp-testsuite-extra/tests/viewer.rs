use ironrdp_input::Scancode;
use ironrdp_viewer::app::physical_key_to_scancode;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::platform::scancode::PhysicalKeyExtScancode as _;

#[test]
fn reported_keys_use_rdp_scancodes() {
    let mappings = [
        (KeyCode::ShiftLeft, Scancode::from_u8(false, 0x2A)),
        (KeyCode::AltLeft, Scancode::from_u8(false, 0x38)),
        (KeyCode::AltRight, Scancode::from_u8(true, 0x38)),
        (KeyCode::SuperLeft, Scancode::from_u8(true, 0x5B)),
        (KeyCode::SuperRight, Scancode::from_u8(true, 0x5C)),
        (KeyCode::Home, Scancode::from_u8(true, 0x47)),
        (KeyCode::ArrowUp, Scancode::from_u8(true, 0x48)),
        (KeyCode::PageUp, Scancode::from_u8(true, 0x49)),
        (KeyCode::ArrowLeft, Scancode::from_u8(true, 0x4B)),
        (KeyCode::ArrowRight, Scancode::from_u8(true, 0x4D)),
        (KeyCode::End, Scancode::from_u8(true, 0x4F)),
        (KeyCode::ArrowDown, Scancode::from_u8(true, 0x50)),
        (KeyCode::PageDown, Scancode::from_u8(true, 0x51)),
        (KeyCode::Insert, Scancode::from_u8(true, 0x52)),
        (KeyCode::Delete, Scancode::from_u8(true, 0x53)),
    ];

    for (key_code, expected) in mappings {
        assert_eq!(
            physical_key_to_scancode(PhysicalKey::Code(key_code)),
            Ok(Some(expected)),
            "{key_code:?}"
        );
    }
}

#[test]
fn unaffected_keys_use_platform_native_scancodes() {
    let physical_key = PhysicalKey::Code(KeyCode::KeyA);
    let expected = physical_key
        .to_scancode()
        .map(|scancode| u16::try_from(scancode).map(Scancode::from_u16).map_err(|_| scancode))
        .transpose();

    assert_eq!(physical_key_to_scancode(physical_key), expected);
}
