use core::fmt::Display;

// FIXME: extended-key flag?
// https://learn.microsoft.com/en-us/windows/win32/inputdev/about-keyboard-input#extended-key-flag

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct WinKey(u8);

impl WinKey {
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    pub const fn from_u8(code: u8) -> Self {
        Self(code)
    }
}

impl Display for WinKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#04x}", self.0)
    }
}

// Based on
// https://learn.microsoft.com/en-us/windows/win32/inputdev/virtual-key-codes

impl WinKey {
    /// BACKSPACE key
    pub const VK_BACK: Self = Self(0x08);
    /// TAB key
    pub const VK_TAB: Self = Self(0x09);
    /// CLEAR key
    pub const VK_CLEAR: Self = Self(0x0C);
    /// ENTER key
    pub const VK_RETURN: Self = Self(0x0D);
    /// SHIFT key
    pub const VK_SHIFT: Self = Self(0x10);
    /// CTRL key
    pub const VK_CONTROL: Self = Self(0x11);
    /// ALT key
    pub const VK_MENU: Self = Self(0x12);
    /// PAUSE key
    pub const VK_PAUSE: Self = Self(0x13);
    /// CAPS LOCK key
    pub const VK_CAPITAL: Self = Self(0x14);
    /// IME Kana mode
    pub const VK_KANA: Self = Self(0x15);
    /// IME Hangul mode
    pub const VK_HANGUL: Self = Self(0x15);
    /// IME On
    pub const VK_IME_ON: Self = Self(0x16);
    /// IME Junja mode
    pub const VK_JUNJA: Self = Self(0x17);
    /// IME final mode
    pub const VK_FINAL: Self = Self(0x18);
    /// IME Hanja mode
    pub const VK_HANJA: Self = Self(0x19);
    /// IME Kanji mode
    pub const VK_KANJI: Self = Self(0x19);
    /// IME Off
    pub const VK_IME_OFF: Self = Self(0x1A);
    /// ESC key
    pub const VK_ESCAPE: Self = Self(0x1B);
    /// IME convert
    pub const VK_CONVERT: Self = Self(0x1C);
    /// IME nonconvert
    pub const VK_NONCONVERT: Self = Self(0x1D);
    /// IME accept
    pub const VK_ACCEPT: Self = Self(0x1E);
    /// IME mode change request
    pub const VK_MODECHANGE: Self = Self(0x1F);
    /// SPACEBAR
    pub const VK_SPACE: Self = Self(0x20);
    /// PAGE UP key
    pub const VK_PRIOR: Self = Self(0x21);
    /// PAGE DOWN key
    pub const VK_NEXT: Self = Self(0x22);
    /// END key
    pub const VK_END: Self = Self(0x23);
    /// HOME key
    pub const VK_HOME: Self = Self(0x24);
    /// LEFT ARROW key
    pub const VK_LEFT: Self = Self(0x25);
    /// UP ARROW key
    pub const VK_UP: Self = Self(0x26);
    /// RIGHT ARROW key
    pub const VK_RIGHT: Self = Self(0x27);
    /// DOWN ARROW key
    pub const VK_DOWN: Self = Self(0x28);
    /// SELECT key
    pub const VK_SELECT: Self = Self(0x29);
    /// PRINT key
    pub const VK_PRINT: Self = Self(0x2A);
    /// EXECUTE key
    pub const VK_EXECUTE: Self = Self(0x2B);
    /// PRINT SCREEN key
    pub const VK_SNAPSHOT: Self = Self(0x2C);
    /// INS key
    pub const VK_INSERT: Self = Self(0x2D);
    /// DEL key
    pub const VK_DELETE: Self = Self(0x2E);
    /// HELP key
    pub const VK_HELP: Self = Self(0x2F);

    pub const VK_0: Self = Self(0x30);
    pub const VK_1: Self = Self(0x31);
    pub const VK_2: Self = Self(0x32);
    pub const VK_3: Self = Self(0x33);
    pub const VK_4: Self = Self(0x34);
    pub const VK_5: Self = Self(0x35);
    pub const VK_6: Self = Self(0x36);
    pub const VK_7: Self = Self(0x37);
    pub const VK_8: Self = Self(0x38);
    pub const VK_9: Self = Self(0x39);
    pub const VK_A: Self = Self(0x41);
    pub const VK_B: Self = Self(0x42);
    pub const VK_C: Self = Self(0x43);
    pub const VK_D: Self = Self(0x44);
    pub const VK_E: Self = Self(0x45);
    pub const VK_F: Self = Self(0x46);
    pub const VK_G: Self = Self(0x47);
    pub const VK_H: Self = Self(0x48);
    pub const VK_I: Self = Self(0x49);
    pub const VK_J: Self = Self(0x4A);
    pub const VK_K: Self = Self(0x4B);
    pub const VK_L: Self = Self(0x4C);
    pub const VK_M: Self = Self(0x4D);
    pub const VK_N: Self = Self(0x4E);
    pub const VK_O: Self = Self(0x4F);
    pub const VK_P: Self = Self(0x50);
    pub const VK_Q: Self = Self(0x51);
    pub const VK_R: Self = Self(0x52);
    pub const VK_S: Self = Self(0x53);
    pub const VK_T: Self = Self(0x54);
    pub const VK_U: Self = Self(0x55);
    pub const VK_V: Self = Self(0x56);
    pub const VK_W: Self = Self(0x57);
    pub const VK_X: Self = Self(0x58);
    pub const VK_Y: Self = Self(0x59);
    pub const VK_Z: Self = Self(0x5A);

    /// Left Windows key
    pub const VK_LWIN: Self = Self(0x5B);
    /// Right Windows key
    pub const VK_RWIN: Self = Self(0x5C);
    /// Applications key
    pub const VK_APPS: Self = Self(0x5D);
    /// Computer Sleep key
    pub const VK_SLEEP: Self = Self(0x5F);
    /// Numeric keypad 0 key
    pub const VK_NUMPAD0: Self = Self(0x60);
    /// Numeric keypad 1 key
    pub const VK_NUMPAD1: Self = Self(0x61);
    /// Numeric keypad 2 key
    pub const VK_NUMPAD2: Self = Self(0x62);
    /// Numeric keypad 3 key
    pub const VK_NUMPAD3: Self = Self(0x63);
    /// Numeric keypad 4 key
    pub const VK_NUMPAD4: Self = Self(0x64);
    /// Numeric keypad 5 key
    pub const VK_NUMPAD5: Self = Self(0x65);
    /// Numeric keypad 6 key
    pub const VK_NUMPAD6: Self = Self(0x66);
    /// Numeric keypad 7 key
    pub const VK_NUMPAD7: Self = Self(0x67);
    /// Numeric keypad 8 key
    pub const VK_NUMPAD8: Self = Self(0x68);
    /// Numeric keypad 9 key
    pub const VK_NUMPAD9: Self = Self(0x69);
    /// Multiply key
    pub const VK_MULTIPLY: Self = Self(0x6A);
    /// Add key
    pub const VK_ADD: Self = Self(0x6B);
    /// Separator key
    pub const VK_SEPARATOR: Self = Self(0x6C);
    /// Subtract key
    pub const VK_SUBTRACT: Self = Self(0x6D);
    /// Decimal key
    pub const VK_DECIMAL: Self = Self(0x6E);
    /// Divide key
    pub const VK_DIVIDE: Self = Self(0x6F);
    /// F1 key
    pub const VK_F1: Self = Self(0x70);
    /// F2 key
    pub const VK_F2: Self = Self(0x71);
    /// F3 key
    pub const VK_F3: Self = Self(0x72);
    /// F4 key
    pub const VK_F4: Self = Self(0x73);
    /// F5 key
    pub const VK_F5: Self = Self(0x74);
    /// F6 key
    pub const VK_F6: Self = Self(0x75);
    /// F7 key
    pub const VK_F7: Self = Self(0x76);
    /// F8 key
    pub const VK_F8: Self = Self(0x77);
    /// F9 key
    pub const VK_F9: Self = Self(0x78);
    /// F10 key
    pub const VK_F10: Self = Self(0x79);
    /// F11 key
    pub const VK_F11: Self = Self(0x7A);
    /// F12 key
    pub const VK_F12: Self = Self(0x7B);
    /// F13 key
    pub const VK_F13: Self = Self(0x7C);
    /// F14 key
    pub const VK_F14: Self = Self(0x7D);
    /// F15 key
    pub const VK_F15: Self = Self(0x7E);
    /// F16 key
    pub const VK_F16: Self = Self(0x7F);
    /// F17 key
    pub const VK_F17: Self = Self(0x80);
    /// F18 key
    pub const VK_F18: Self = Self(0x81);
    /// F19 key
    pub const VK_F19: Self = Self(0x82);
    /// F20 key
    pub const VK_F20: Self = Self(0x83);
    /// F21 key
    pub const VK_F21: Self = Self(0x84);
    /// F22 key
    pub const VK_F22: Self = Self(0x85);
    /// F23 key
    pub const VK_F23: Self = Self(0x86);
    /// F24 key
    pub const VK_F24: Self = Self(0x87);
    /// NUM LOCK key
    pub const VK_NUMLOCK: Self = Self(0x90);
    /// SCROLL LOCK key
    pub const VK_SCROLL: Self = Self(0x91);
    /// Left SHIFT key
    pub const VK_LSHIFT: Self = Self(0xA0);
    /// Right SHIFT key
    pub const VK_RSHIFT: Self = Self(0xA1);
    /// Left CONTROL key
    pub const VK_LCONTROL: Self = Self(0xA2);
    /// Right CONTROL key
    pub const VK_RCONTROL: Self = Self(0xA3);
    /// Left ALT key
    pub const VK_LMENU: Self = Self(0xA4);
    /// Right ALT key
    pub const VK_RMENU: Self = Self(0xA5);
    /// Browser Back key
    pub const VK_BROWSER_BACK: Self = Self(0xA6);
    /// Browser Forward key
    pub const VK_BROWSER_FORWARD: Self = Self(0xA7);
    /// Browser Refresh key
    pub const VK_BROWSER_REFRESH: Self = Self(0xA8);
    /// Browser Stop key
    pub const VK_BROWSER_STOP: Self = Self(0xA9);
    /// Browser Search key
    pub const VK_BROWSER_SEARCH: Self = Self(0xAA);
    /// Browser Favorites key
    pub const VK_BROWSER_FAVORITES: Self = Self(0xAB);
    /// Browser Start and Home key
    pub const VK_BROWSER_HOME: Self = Self(0xAC);
    /// Volume Mute key
    pub const VK_VOLUME_MUTE: Self = Self(0xAD);
    /// Volume Down key
    pub const VK_VOLUME_DOWN: Self = Self(0xAE);
    /// Volume Up key
    pub const VK_VOLUME_UP: Self = Self(0xAF);
    /// Next Track key
    pub const VK_MEDIA_NEXT_TRACK: Self = Self(0xB0);
    /// Previous Track key
    pub const VK_MEDIA_PREV_TRACK: Self = Self(0xB1);
    /// Stop Media key
    pub const VK_MEDIA_STOP: Self = Self(0xB2);
    /// Play/Pause Media key
    pub const VK_MEDIA_PLAY_PAUSE: Self = Self(0xB3);
    /// Start Mail key
    pub const VK_LAUNCH_MAIL: Self = Self(0xB4);
    /// Select Media key
    pub const VK_LAUNCH_MEDIA_SELECT: Self = Self(0xB5);
    /// Start Application 1 key
    pub const VK_LAUNCH_APP1: Self = Self(0xB6);
    /// Start Application 2 key
    pub const VK_LAUNCH_APP2: Self = Self(0xB7);
    /// Used for miscellaneous characters; it can vary by keyboard. For the US standard keyboard, the ;: key
    pub const VK_OEM_1: Self = Self(0xBA);
    /// For any country/region, the + key
    pub const VK_OEM_PLUS: Self = Self(0xBB);
    /// For any country/region, the , key
    pub const VK_OEM_COMMA: Self = Self(0xBC);
    /// For any country/region, the - key
    pub const VK_OEM_MINUS: Self = Self(0xBD);
    /// For any country/region, the . key
    pub const VK_OEM_PERIOD: Self = Self(0xBE);
    /// Used for miscellaneous characters; it can vary by keyboard. For the US standard keyboard, the /? key
    pub const VK_OEM_2: Self = Self(0xBF);
    /// Used for miscellaneous characters; it can vary by keyboard. For the US standard keyboard, the `~ key
    pub const VK_OEM_3: Self = Self(0xC0);
    /// Used for miscellaneous characters; it can vary by keyboard. For the US standard keyboard, the [{ key
    pub const VK_OEM_4: Self = Self(0xDB);
    /// Used for miscellaneous characters; it can vary by keyboard. For the US standard keyboard, the \\| key
    pub const VK_OEM_5: Self = Self(0xDC);
    /// Used for miscellaneous characters; it can vary by keyboard. For the US standard keyboard, the ]} key
    pub const VK_OEM_6: Self = Self(0xDD);
    /// Used for miscellaneous characters; it can vary by keyboard. For the US standard keyboard, the '" key
    pub const VK_OEM_7: Self = Self(0xDE);
    /// Used for miscellaneous characters; it can vary by keyboard.
    pub const VK_OEM_8: Self = Self(0xDF);
    /// The <> keys on the US standard keyboard, or the \\| key on the non-US 102-key keyboard
    pub const VK_OEM_102: Self = Self(0xE2);
    /// IME PROCESS key
    pub const VK_PROCESSKEY: Self = Self(0xE5);
    /// Used to pass Unicode characters as if they were keystrokes. The VK_PACKET key is the low word of a 32-bit Virtual Key value used for non-keyboard input methods. For more information, see Remark in KEYBDINPUT, SendInput, WM_KEYDOWN, and WM_KEYUP
    pub const VK_PACKET: Self = Self(0xE7);
    /// Attn key
    pub const VK_ATTN: Self = Self(0xF6);
    /// CrSel key
    pub const VK_CRSEL: Self = Self(0xF7);
    /// ExSel key
    pub const VK_EXSEL: Self = Self(0xF8);
    /// Erase EOF key
    pub const VK_EREOF: Self = Self(0xF9);
    /// Play key
    pub const VK_PLAY: Self = Self(0xFA);
    /// Zoom key
    pub const VK_ZOOM: Self = Self(0xFB);
    /// Reserved
    pub const VK_NONAME: Self = Self(0xFC);
    /// PA1 key
    pub const VK_PA1: Self = Self(0xFD);
    /// Clear key
    pub const VK_OEM_CLEAR: Self = Self(0xFE);
}
