use ironrdp::input::Scancode;
use winit::keyboard::KeyCode;

pub(crate) fn is_modifier(key_code: KeyCode) -> bool {
    matches!(
        key_code,
        KeyCode::ShiftLeft
            | KeyCode::ShiftRight
            | KeyCode::ControlLeft
            | KeyCode::ControlRight
            | KeyCode::AltLeft
            | KeyCode::AltRight
            | KeyCode::SuperLeft
            | KeyCode::SuperRight
    )
}

/// RDP expects PC/AT set-1 scancodes, while native winit scancodes vary by platform.
///
/// `Pause` deliberately has no mapping because [MS-RDPBCGR] 2.2.8.1.2.2.1 requires four events, including `EXTENDED1`, while `Scancode` represents one ordinary event.
///
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/5073f4ed-1e93-45e1-b039-6e30c385867c
pub(crate) const fn map_key_code(key_code: KeyCode) -> Option<Scancode> {
    let scancode = match key_code {
        KeyCode::Escape => (false, 0x01),
        KeyCode::Digit1 => (false, 0x02),
        KeyCode::Digit2 => (false, 0x03),
        KeyCode::Digit3 => (false, 0x04),
        KeyCode::Digit4 => (false, 0x05),
        KeyCode::Digit5 => (false, 0x06),
        KeyCode::Digit6 => (false, 0x07),
        KeyCode::Digit7 => (false, 0x08),
        KeyCode::Digit8 => (false, 0x09),
        KeyCode::Digit9 => (false, 0x0A),
        KeyCode::Digit0 => (false, 0x0B),
        KeyCode::Minus => (false, 0x0C),
        KeyCode::Equal => (false, 0x0D),
        KeyCode::Backspace => (false, 0x0E),
        KeyCode::Tab => (false, 0x0F),
        KeyCode::KeyQ => (false, 0x10),
        KeyCode::KeyW => (false, 0x11),
        KeyCode::KeyE => (false, 0x12),
        KeyCode::KeyR => (false, 0x13),
        KeyCode::KeyT => (false, 0x14),
        KeyCode::KeyY => (false, 0x15),
        KeyCode::KeyU => (false, 0x16),
        KeyCode::KeyI => (false, 0x17),
        KeyCode::KeyO => (false, 0x18),
        KeyCode::KeyP => (false, 0x19),
        KeyCode::BracketLeft => (false, 0x1A),
        KeyCode::BracketRight => (false, 0x1B),
        KeyCode::Enter => (false, 0x1C),
        KeyCode::KeyA => (false, 0x1E),
        KeyCode::KeyS => (false, 0x1F),
        KeyCode::KeyD => (false, 0x20),
        KeyCode::KeyF => (false, 0x21),
        KeyCode::KeyG => (false, 0x22),
        KeyCode::KeyH => (false, 0x23),
        KeyCode::KeyJ => (false, 0x24),
        KeyCode::KeyK => (false, 0x25),
        KeyCode::KeyL => (false, 0x26),
        KeyCode::Semicolon => (false, 0x27),
        KeyCode::Quote => (false, 0x28),
        KeyCode::Backquote => (false, 0x29),
        KeyCode::Backslash => (false, 0x2B),
        KeyCode::KeyZ => (false, 0x2C),
        KeyCode::KeyX => (false, 0x2D),
        KeyCode::KeyC => (false, 0x2E),
        KeyCode::KeyV => (false, 0x2F),
        KeyCode::KeyB => (false, 0x30),
        KeyCode::KeyN => (false, 0x31),
        KeyCode::KeyM => (false, 0x32),
        KeyCode::Comma => (false, 0x33),
        KeyCode::Period => (false, 0x34),
        KeyCode::Slash => (false, 0x35),
        KeyCode::NumpadMultiply => (false, 0x37),
        KeyCode::Space => (false, 0x39),
        KeyCode::CapsLock => (false, 0x3A),
        KeyCode::F1 => (false, 0x3B),
        KeyCode::F2 => (false, 0x3C),
        KeyCode::F3 => (false, 0x3D),
        KeyCode::F4 => (false, 0x3E),
        KeyCode::F5 => (false, 0x3F),
        KeyCode::F6 => (false, 0x40),
        KeyCode::F7 => (false, 0x41),
        KeyCode::F8 => (false, 0x42),
        KeyCode::F9 => (false, 0x43),
        KeyCode::F10 => (false, 0x44),
        KeyCode::NumLock => (false, 0x45),
        KeyCode::ScrollLock => (false, 0x46),
        KeyCode::Numpad7 => (false, 0x47),
        KeyCode::Numpad8 => (false, 0x48),
        KeyCode::Numpad9 => (false, 0x49),
        KeyCode::NumpadSubtract => (false, 0x4A),
        KeyCode::Numpad4 => (false, 0x4B),
        KeyCode::Numpad5 => (false, 0x4C),
        KeyCode::Numpad6 => (false, 0x4D),
        KeyCode::NumpadAdd => (false, 0x4E),
        KeyCode::Numpad1 => (false, 0x4F),
        KeyCode::Numpad2 => (false, 0x50),
        KeyCode::Numpad3 => (false, 0x51),
        KeyCode::Numpad0 => (false, 0x52),
        KeyCode::NumpadDecimal => (false, 0x53),
        KeyCode::IntlBackslash => (false, 0x56),
        KeyCode::F11 => (false, 0x57),
        KeyCode::F12 => (false, 0x58),
        KeyCode::NumpadEqual => (false, 0x59),
        KeyCode::F13 => (false, 0x64),
        KeyCode::F14 => (false, 0x65),
        KeyCode::F15 => (false, 0x66),
        KeyCode::F16 => (false, 0x67),
        KeyCode::F17 => (false, 0x68),
        KeyCode::F18 => (false, 0x69),
        KeyCode::F19 => (false, 0x6A),
        KeyCode::F20 => (false, 0x6B),
        KeyCode::F21 => (false, 0x6C),
        KeyCode::F22 => (false, 0x6D),
        KeyCode::F23 => (false, 0x6E),
        KeyCode::KanaMode => (false, 0x70),
        KeyCode::Lang2 => (false, 0x71),
        KeyCode::Lang1 => (false, 0x72),
        KeyCode::IntlRo => (false, 0x73),
        KeyCode::F24 => (false, 0x76),
        KeyCode::Lang4 => (false, 0x77),
        KeyCode::Lang3 => (false, 0x78),
        KeyCode::Convert => (false, 0x79),
        KeyCode::NonConvert => (false, 0x7B),
        KeyCode::IntlYen => (false, 0x7D),
        KeyCode::NumpadComma => (false, 0x7E),
        KeyCode::Undo => (true, 0x08),
        KeyCode::Paste => (true, 0x0A),
        KeyCode::MediaTrackPrevious => (true, 0x10),
        KeyCode::MediaTrackNext => (true, 0x19),
        KeyCode::NumpadEnter => (true, 0x1C),
        KeyCode::ControlRight => (true, 0x1D),
        KeyCode::Cut => (true, 0x17),
        KeyCode::Copy => (true, 0x18),
        KeyCode::AudioVolumeMute => (true, 0x20),
        KeyCode::LaunchApp2 => (true, 0x21),
        KeyCode::MediaPlayPause => (true, 0x22),
        KeyCode::MediaStop => (true, 0x24),
        KeyCode::Eject => (true, 0x2C),
        KeyCode::AudioVolumeDown => (true, 0x2E),
        KeyCode::AudioVolumeUp => (true, 0x30),
        KeyCode::BrowserHome => (true, 0x32),
        KeyCode::NumpadDivide => (true, 0x35),
        KeyCode::AltRight => (true, 0x38),
        KeyCode::Help => (true, 0x3B),
        KeyCode::Home => (true, 0x47),
        KeyCode::ArrowUp => (true, 0x48),
        KeyCode::PageUp => (true, 0x49),
        KeyCode::ArrowLeft => (true, 0x4B),
        KeyCode::ArrowRight => (true, 0x4D),
        KeyCode::End => (true, 0x4F),
        KeyCode::ArrowDown => (true, 0x50),
        KeyCode::PageDown => (true, 0x51),
        KeyCode::Insert => (true, 0x52),
        KeyCode::Delete => (true, 0x53),
        KeyCode::ContextMenu => (true, 0x5D),
        KeyCode::Power => (true, 0x5E),
        KeyCode::Sleep => (true, 0x5F),
        KeyCode::WakeUp => (true, 0x63),
        KeyCode::BrowserSearch => (true, 0x65),
        KeyCode::BrowserFavorites => (true, 0x66),
        KeyCode::BrowserRefresh => (true, 0x67),
        KeyCode::BrowserStop => (true, 0x68),
        KeyCode::BrowserForward => (true, 0x69),
        KeyCode::BrowserBack => (true, 0x6A),
        KeyCode::LaunchApp1 => (true, 0x6B),
        KeyCode::LaunchMail => (true, 0x6C),
        KeyCode::MediaSelect => (true, 0x6D),
        KeyCode::SuperLeft => (true, 0x5B),
        KeyCode::SuperRight => (true, 0x5C),
        KeyCode::PrintScreen => (true, 0x37),
        _ => return None,
    };

    Some(Scancode::from_u8(scancode.0, scancode.1))
}

#[cfg(windows)]
pub(crate) const fn requires_native_layout_mapping(key_code: KeyCode) -> bool {
    matches!(key_code, KeyCode::Lang1 | KeyCode::Lang2)
}

#[cfg(windows)]
pub(crate) fn map_native_layout_scancode(scancode: u32) -> Option<Scancode> {
    let scancode = u16::try_from(scancode).ok()?;
    Some(Scancode::from_u16(scancode))
}
