use core::fmt::Display;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct AppleKey(u8);

impl AppleKey {
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    pub const fn from_u8(code: u8) -> Self {
        Self(code)
    }
}

impl Display for AppleKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#04x}", self.0)
    }
}

// Based on HIToolbox/Events.h

#[allow(non_upper_case_globals)]
impl AppleKey {
    //  Summary:
    //    Virtual keycodes
    //
    //  Discussion:
    //    These constants are the virtual keycodes defined originally in
    //    Inside Mac Volume V, pg. V-191. They identify physical keys on a
    //    keyboard. Those constants with "ANSI" in the name are labeled
    //    according to the key position on an ANSI-standard US keyboard.
    //    For example, kVK_ANSI_A indicates the virtual keycode for the key
    //    with the letter 'A' in the US keyboard layout. Other keyboard
    //    layouts may have the 'A' key label on a different physical key;
    //    in this case, pressing 'A' will generate a different virtual
    //    keycode.

    pub const kVK_ANSI_A: Self = Self(0x00);
    pub const kVK_ANSI_S: Self = Self(0x01);
    pub const kVK_ANSI_D: Self = Self(0x02);
    pub const kVK_ANSI_F: Self = Self(0x03);
    pub const kVK_ANSI_H: Self = Self(0x04);
    pub const kVK_ANSI_G: Self = Self(0x05);
    pub const kVK_ANSI_Z: Self = Self(0x06);
    pub const kVK_ANSI_X: Self = Self(0x07);
    pub const kVK_ANSI_C: Self = Self(0x08);
    pub const kVK_ANSI_V: Self = Self(0x09);
    pub const kVK_ANSI_B: Self = Self(0x0B);
    pub const kVK_ANSI_Q: Self = Self(0x0C);
    pub const kVK_ANSI_W: Self = Self(0x0D);
    pub const kVK_ANSI_E: Self = Self(0x0E);
    pub const kVK_ANSI_R: Self = Self(0x0F);
    pub const kVK_ANSI_Y: Self = Self(0x10);
    pub const kVK_ANSI_T: Self = Self(0x11);
    pub const kVK_ANSI_1: Self = Self(0x12);
    pub const kVK_ANSI_2: Self = Self(0x13);
    pub const kVK_ANSI_3: Self = Self(0x14);
    pub const kVK_ANSI_4: Self = Self(0x15);
    pub const kVK_ANSI_6: Self = Self(0x16);
    pub const kVK_ANSI_5: Self = Self(0x17);
    pub const kVK_ANSI_Equal: Self = Self(0x18);
    pub const kVK_ANSI_9: Self = Self(0x19);
    pub const kVK_ANSI_7: Self = Self(0x1A);
    pub const kVK_ANSI_Minus: Self = Self(0x1B);
    pub const kVK_ANSI_8: Self = Self(0x1C);
    pub const kVK_ANSI_0: Self = Self(0x1D);
    pub const kVK_ANSI_RightBracket: Self = Self(0x1E);
    pub const kVK_ANSI_O: Self = Self(0x1F);
    pub const kVK_ANSI_U: Self = Self(0x20);
    pub const kVK_ANSI_LeftBracket: Self = Self(0x21);
    pub const kVK_ANSI_I: Self = Self(0x22);
    pub const kVK_ANSI_P: Self = Self(0x23);
    pub const kVK_ANSI_L: Self = Self(0x25);
    pub const kVK_ANSI_J: Self = Self(0x26);
    pub const kVK_ANSI_Quote: Self = Self(0x27);
    pub const kVK_ANSI_K: Self = Self(0x28);
    pub const kVK_ANSI_Semicolon: Self = Self(0x29);
    pub const kVK_ANSI_Backslash: Self = Self(0x2A);
    pub const kVK_ANSI_Comma: Self = Self(0x2B);
    pub const kVK_ANSI_Slash: Self = Self(0x2C);
    pub const kVK_ANSI_N: Self = Self(0x2D);
    pub const kVK_ANSI_M: Self = Self(0x2E);
    pub const kVK_ANSI_Period: Self = Self(0x2F);
    pub const kVK_ANSI_Grave: Self = Self(0x32);
    pub const kVK_ANSI_KeypadDecimal: Self = Self(0x41);
    pub const kVK_ANSI_KeypadMultiply: Self = Self(0x43);
    pub const kVK_ANSI_KeypadPlus: Self = Self(0x45);
    pub const kVK_ANSI_KeypadClear: Self = Self(0x47);
    pub const kVK_ANSI_KeypadDivide: Self = Self(0x4B);
    pub const kVK_ANSI_KeypadEnter: Self = Self(0x4C);
    pub const kVK_ANSI_KeypadMinus: Self = Self(0x4E);
    pub const kVK_ANSI_KeypadEquals: Self = Self(0x51);
    pub const kVK_ANSI_Keypad0: Self = Self(0x52);
    pub const kVK_ANSI_Keypad1: Self = Self(0x53);
    pub const kVK_ANSI_Keypad2: Self = Self(0x54);
    pub const kVK_ANSI_Keypad3: Self = Self(0x55);
    pub const kVK_ANSI_Keypad4: Self = Self(0x56);
    pub const kVK_ANSI_Keypad5: Self = Self(0x57);
    pub const kVK_ANSI_Keypad6: Self = Self(0x58);
    pub const kVK_ANSI_Keypad7: Self = Self(0x59);
    pub const kVK_ANSI_Keypad8: Self = Self(0x5B);
    pub const kVK_ANSI_Keypad9: Self = Self(0x5C);

    // keycodes for keys that are independent of keyboard layout

    pub const kVK_Return: Self = Self(0x24);
    pub const kVK_Tab: Self = Self(0x30);
    pub const kVK_Space: Self = Self(0x31);
    pub const kVK_Delete: Self = Self(0x33);
    pub const kVK_Escape: Self = Self(0x35);
    pub const kVK_Command: Self = Self(0x37);
    pub const kVK_Shift: Self = Self(0x38);
    pub const kVK_CapsLock: Self = Self(0x39);
    pub const kVK_Option: Self = Self(0x3A);
    pub const kVK_Control: Self = Self(0x3B);
    pub const kVK_RightShift: Self = Self(0x3C);
    pub const kVK_RightOption: Self = Self(0x3D);
    pub const kVK_RightControl: Self = Self(0x3E);
    pub const kVK_Function: Self = Self(0x3F);
    pub const kVK_F17: Self = Self(0x40);
    pub const kVK_VolumeUp: Self = Self(0x48);
    pub const kVK_VolumeDown: Self = Self(0x49);
    pub const kVK_Mute: Self = Self(0x4A);
    pub const kVK_F18: Self = Self(0x4F);
    pub const kVK_F19: Self = Self(0x50);
    pub const kVK_F20: Self = Self(0x5A);
    pub const kVK_F5: Self = Self(0x60);
    pub const kVK_F6: Self = Self(0x61);
    pub const kVK_F7: Self = Self(0x62);
    pub const kVK_F3: Self = Self(0x63);
    pub const kVK_F8: Self = Self(0x64);
    pub const kVK_F9: Self = Self(0x65);
    pub const kVK_F11: Self = Self(0x67);
    pub const kVK_F13: Self = Self(0x69);
    pub const kVK_F16: Self = Self(0x6A);
    pub const kVK_F14: Self = Self(0x6B);
    pub const kVK_F10: Self = Self(0x6D);
    pub const kVK_F12: Self = Self(0x6F);
    pub const kVK_F15: Self = Self(0x71);
    pub const kVK_Help: Self = Self(0x72);
    pub const kVK_Home: Self = Self(0x73);
    pub const kVK_PageUp: Self = Self(0x74);
    pub const kVK_ForwardDelete: Self = Self(0x75);
    pub const kVK_F4: Self = Self(0x76);
    pub const kVK_End: Self = Self(0x77);
    pub const kVK_F2: Self = Self(0x78);
    pub const kVK_PageDown: Self = Self(0x79);
    pub const kVK_F1: Self = Self(0x7A);
    pub const kVK_LeftArrow: Self = Self(0x7B);
    pub const kVK_RightArrow: Self = Self(0x7C);
    pub const kVK_DownArrow: Self = Self(0x7D);
    pub const kVK_UpArrow: Self = Self(0x7E);
}
