/**
 * Key Flags
 */

pub(crate) const KBDEXT: u16 = 0x0100;
/*
 * Virtual Key Codes (Windows):
 * http://msdn.microsoft.com/en-us/library/windows/desktop/dd375731/
 * http://msdn.microsoft.com/en-us/library/ms927178.aspx
 */

/* Mouse buttons */
pub(crate) const VK_LBUTTON: u16 = 0x01; /* Left mouse button */
pub(crate) const VK_RBUTTON: u16 = 0x02; /* Right mouse button */
pub(crate) const VK_CANCEL: u16 = 0x03; /* Control-break processing */
pub(crate) const VK_MBUTTON: u16 = 0x04; /* Middle mouse button (three-button mouse) */
pub(crate) const VK_XBUTTON1: u16 = 0x05; /* Windows 2000/XP: X1 mouse button */
pub(crate) const VK_XBUTTON2: u16 = 0x06; /* Windows 2000/XP: X2 mouse button */

/* 0x07 is undefined */

pub(crate) const VK_BACK: u16 = 0x08; /* BACKSPACE key */
pub(crate) const VK_TAB: u16 = 0x09; /* TAB key */

/* 0x0A to 0x0B are reserved */

pub(crate) const VK_CLEAR: u16 = 0x0C; /* CLEAR key */
pub(crate) const VK_RETURN: u16 = 0x0D; /* ENTER key */

/* 0x0E to 0x0F are undefined */

pub(crate) const VK_SHIFT: u16 = 0x10; /* SHIFT key */
pub(crate) const VK_CONTROL: u16 = 0x11; /* CTRL key */
pub(crate) const VK_MENU: u16 = 0x12; /* ALT key */
pub(crate) const VK_PAUSE: u16 = 0x13; /* PAUSE key */
pub(crate) const VK_CAPITAL: u16 = 0x14; /* CAPS LOCK key */
pub(crate) const VK_KANA: u16 = 0x15; /* Input Method Editor (IME) Kana mode */
pub(crate) const VK_HANGUEL: u16 = 0x15; /* IME Hanguel mode (maintained for compatibility; use #define VK_HANGUL) \
                                          */
pub(crate) const VK_HANGUL: u16 = 0x15; /* IME Hangul mode */

/* 0x16 is undefined */

pub(crate) const VK_JUNJA: u16 = 0x17; /* IME Junja mode */
pub(crate) const VK_FINAL: u16 = 0x18; /* IME final mode */
pub(crate) const VK_HANJA: u16 = 0x19; /* IME Hanja mode */
pub(crate) const VK_KANJI: u16 = 0x19; /* IME Kanji mode */

/* 0x1A is undefined, use it for missing Hiragana/Katakana Toggle */

pub(crate) const VK_HKTG: u16 = 0x1A; /* Hiragana/Katakana toggle */
pub(crate) const VK_ESCAPE: u16 = 0x1B; /* ESC key */
pub(crate) const VK_CONVERT: u16 = 0x1C; /* IME convert */
pub(crate) const VK_NONCONVERT: u16 = 0x1D; /* IME nonconvert */
pub(crate) const VK_ACCEPT: u16 = 0x1E; /* IME accept */
pub(crate) const VK_MODECHANGE: u16 = 0x1F; /* IME mode change request */

pub(crate) const VK_SPACE: u16 = 0x20; /* SPACEBAR */
pub(crate) const VK_PRIOR: u16 = 0x21; /* PAGE UP key */
pub(crate) const VK_NEXT: u16 = 0x22; /* PAGE DOWN key */
pub(crate) const VK_END: u16 = 0x23; /* END key */
pub(crate) const VK_HOME: u16 = 0x24; /* HOME key */
pub(crate) const VK_LEFT: u16 = 0x25; /* LEFT ARROW key */
pub(crate) const VK_UP: u16 = 0x26; /* UP ARROW key */
pub(crate) const VK_RIGHT: u16 = 0x27; /* RIGHT ARROW key */
pub(crate) const VK_DOWN: u16 = 0x28; /* DOWN ARROW key */
pub(crate) const VK_SELECT: u16 = 0x29; /* SELECT key */
pub(crate) const VK_PRINT: u16 = 0x2A; /* PRINT key */
pub(crate) const VK_EXECUTE: u16 = 0x2B; /* EXECUTE key */
pub(crate) const VK_SNAPSHOT: u16 = 0x2C; /* PRINT SCREEN key */
pub(crate) const VK_INSERT: u16 = 0x2D; /* INS key */
pub(crate) const VK_DELETE: u16 = 0x2E; /* DEL key */
pub(crate) const VK_HELP: u16 = 0x2F; /* HELP key */

/* Digits, the last 4 bits of the code represent the corresponding digit */

pub(crate) const VK_KEY_0: u16 = 0x30; /* '0' key */
pub(crate) const VK_KEY_1: u16 = 0x31; /* '1' key */
pub(crate) const VK_KEY_2: u16 = 0x32; /* '2' key */
pub(crate) const VK_KEY_3: u16 = 0x33; /* '3' key */
pub(crate) const VK_KEY_4: u16 = 0x34; /* '4' key */
pub(crate) const VK_KEY_5: u16 = 0x35; /* '5' key */
pub(crate) const VK_KEY_6: u16 = 0x36; /* '6' key */
pub(crate) const VK_KEY_7: u16 = 0x37; /* '7' key */
pub(crate) const VK_KEY_8: u16 = 0x38; /* '8' key */
pub(crate) const VK_KEY_9: u16 = 0x39; /* '9' key */

/* 0x3A to 0x40 are undefined */

/* The alphabet, the code corresponds to the capitalized letter in the ASCII code */

pub(crate) const VK_KEY_A: u16 = 0x41; /* 'A' key */
pub(crate) const VK_KEY_B: u16 = 0x42; /* 'B' key */
pub(crate) const VK_KEY_C: u16 = 0x43; /* 'C' key */
pub(crate) const VK_KEY_D: u16 = 0x44; /* 'D' key */
pub(crate) const VK_KEY_E: u16 = 0x45; /* 'E' key */
pub(crate) const VK_KEY_F: u16 = 0x46; /* 'F' key */
pub(crate) const VK_KEY_G: u16 = 0x47; /* 'G' key */
pub(crate) const VK_KEY_H: u16 = 0x48; /* 'H' key */
pub(crate) const VK_KEY_I: u16 = 0x49; /* 'I' key */
pub(crate) const VK_KEY_J: u16 = 0x4A; /* 'J' key */
pub(crate) const VK_KEY_K: u16 = 0x4B; /* 'K' key */
pub(crate) const VK_KEY_L: u16 = 0x4C; /* 'L' key */
pub(crate) const VK_KEY_M: u16 = 0x4D; /* 'M' key */
pub(crate) const VK_KEY_N: u16 = 0x4E; /* 'N' key */
pub(crate) const VK_KEY_O: u16 = 0x4F; /* 'O' key */
pub(crate) const VK_KEY_P: u16 = 0x50; /* 'P' key */
pub(crate) const VK_KEY_Q: u16 = 0x51; /* 'Q' key */
pub(crate) const VK_KEY_R: u16 = 0x52; /* 'R' key */
pub(crate) const VK_KEY_S: u16 = 0x53; /* 'S' key */
pub(crate) const VK_KEY_T: u16 = 0x54; /* 'T' key */
pub(crate) const VK_KEY_U: u16 = 0x55; /* 'U' key */
pub(crate) const VK_KEY_V: u16 = 0x56; /* 'V' key */
pub(crate) const VK_KEY_W: u16 = 0x57; /* 'W' key */
pub(crate) const VK_KEY_X: u16 = 0x58; /* 'X' key */
pub(crate) const VK_KEY_Y: u16 = 0x59; /* 'Y' key */
pub(crate) const VK_KEY_Z: u16 = 0x5A; /* 'Z' key */

pub(crate) const VK_LWIN: u16 = 0x5B; /* Left Windows key (Microsoft Natural keyboard) */
pub(crate) const VK_RWIN: u16 = 0x5C; /* Right Windows key (Natural keyboard) */
pub(crate) const VK_APPS: u16 = 0x5D; /* Applications key (Natural keyboard) */

/* 0x5E is reserved */

pub(crate) const VK_POWER: u16 = 0x5E; /* Power key */

pub(crate) const VK_SLEEP: u16 = 0x5F; /* Computer Sleep key */

/* Numeric keypad digits, the last four bits of the code represent the corresponding digit */

pub(crate) const VK_NUMPAD0: u16 = 0x60; /* Numeric keypad '0' key */
pub(crate) const VK_NUMPAD1: u16 = 0x61; /* Numeric keypad '1' key */
pub(crate) const VK_NUMPAD2: u16 = 0x62; /* Numeric keypad '2' key */
pub(crate) const VK_NUMPAD3: u16 = 0x63; /* Numeric keypad '3' key */
pub(crate) const VK_NUMPAD4: u16 = 0x64; /* Numeric keypad '4' key */
pub(crate) const VK_NUMPAD5: u16 = 0x65; /* Numeric keypad '5' key */
pub(crate) const VK_NUMPAD6: u16 = 0x66; /* Numeric keypad '6' key */
pub(crate) const VK_NUMPAD7: u16 = 0x67; /* Numeric keypad '7' key */
pub(crate) const VK_NUMPAD8: u16 = 0x68; /* Numeric keypad '8' key */
pub(crate) const VK_NUMPAD9: u16 = 0x69; /* Numeric keypad '9' key */

/* Numeric keypad operators and special keys */

pub(crate) const VK_MULTIPLY: u16 = 0x6A; /* Multiply key */
pub(crate) const VK_ADD: u16 = 0x6B; /* Add key */
pub(crate) const VK_SEPARATOR: u16 = 0x6C; /* Separator key */
pub(crate) const VK_SUBTRACT: u16 = 0x6D; /* Subtract key */
pub(crate) const VK_DECIMAL: u16 = 0x6E; /* Decimal key */
pub(crate) const VK_DIVIDE: u16 = 0x6F; /* Divide key */

/* Function keys, from F1 to F24 */

pub(crate) const VK_F1: u16 = 0x70; /* F1 key */
pub(crate) const VK_F2: u16 = 0x71; /* F2 key */
pub(crate) const VK_F3: u16 = 0x72; /* F3 key */
pub(crate) const VK_F4: u16 = 0x73; /* F4 key */
pub(crate) const VK_F5: u16 = 0x74; /* F5 key */
pub(crate) const VK_F6: u16 = 0x75; /* F6 key */
pub(crate) const VK_F7: u16 = 0x76; /* F7 key */
pub(crate) const VK_F8: u16 = 0x77; /* F8 key */
pub(crate) const VK_F9: u16 = 0x78; /* F9 key */
pub(crate) const VK_F10: u16 = 0x79; /* F10 key */
pub(crate) const VK_F11: u16 = 0x7A; /* F11 key */
pub(crate) const VK_F12: u16 = 0x7B; /* F12 key */
pub(crate) const VK_F13: u16 = 0x7C; /* F13 key */
pub(crate) const VK_F14: u16 = 0x7D; /* F14 key */
pub(crate) const VK_F15: u16 = 0x7E; /* F15 key */
pub(crate) const VK_F16: u16 = 0x7F; /* F16 key */
pub(crate) const VK_F17: u16 = 0x80; /* F17 key */
pub(crate) const VK_F18: u16 = 0x81; /* F18 key */
pub(crate) const VK_F19: u16 = 0x82; /* F19 key */
pub(crate) const VK_F20: u16 = 0x83; /* F20 key */
pub(crate) const VK_F21: u16 = 0x84; /* F21 key */
pub(crate) const VK_F22: u16 = 0x85; /* F22 key */
pub(crate) const VK_F23: u16 = 0x86; /* F23 key */
pub(crate) const VK_F24: u16 = 0x87; /* F24 key */

/* 0x88 to 0x8F are unassigned */

pub(crate) const VK_NUMLOCK: u16 = 0x90; /* NUM LOCK key */
pub(crate) const VK_SCROLL: u16 = 0x91; /* SCROLL LOCK key */

/* 0x92 to 0x96 are OEM specific */
/* 0x97 to 0x9F are unassigned */

/* Modifier keys */

pub(crate) const VK_LSHIFT: u16 = 0xA0; /* Left SHIFT key */
pub(crate) const VK_RSHIFT: u16 = 0xA1; /* Right SHIFT key */
pub(crate) const VK_LCONTROL: u16 = 0xA2; /* Left CONTROL key */
pub(crate) const VK_RCONTROL: u16 = 0xA3; /* Right CONTROL key */
pub(crate) const VK_LMENU: u16 = 0xA4; /* Left MENU key */
pub(crate) const VK_RMENU: u16 = 0xA5; /* Right MENU key */

/* Browser related keys */

pub(crate) const VK_BROWSER_BACK: u16 = 0xA6; /* Windows 2000/XP: Browser Back key */
pub(crate) const VK_BROWSER_FORWARD: u16 = 0xA7; /* Windows 2000/XP: Browser Forward key */
pub(crate) const VK_BROWSER_REFRESH: u16 = 0xA8; /* Windows 2000/XP: Browser Refresh key */
pub(crate) const VK_BROWSER_STOP: u16 = 0xA9; /* Windows 2000/XP: Browser Stop key */
pub(crate) const VK_BROWSER_SEARCH: u16 = 0xAA; /* Windows 2000/XP: Browser Search key */
pub(crate) const VK_BROWSER_FAVORITES: u16 = 0xAB; /* Windows 2000/XP: Browser Favorites key */
pub(crate) const VK_BROWSER_HOME: u16 = 0xAC; /* Windows 2000/XP: Browser Start and Home key */

/* Volume related keys */

pub(crate) const VK_VOLUME_MUTE: u16 = 0xAD; /* Windows 2000/XP: Volume Mute key */
pub(crate) const VK_VOLUME_DOWN: u16 = 0xAE; /* Windows 2000/XP: Volume Down key */
pub(crate) const VK_VOLUME_UP: u16 = 0xAF; /* Windows 2000/XP: Volume Up key */

/* Media player related keys */

pub(crate) const VK_MEDIA_NEXT_TRACK: u16 = 0xB0; /* Windows 2000/XP: Next Track key */
pub(crate) const VK_MEDIA_PREV_TRACK: u16 = 0xB1; /* Windows 2000/XP: Previous Track key */
pub(crate) const VK_MEDIA_STOP: u16 = 0xB2; /* Windows 2000/XP: Stop Media key */
pub(crate) const VK_MEDIA_PLAY_PAUSE: u16 = 0xB3; /* Windows 2000/XP: Play/Pause Media key */

/* Application launcher keys */

pub(crate) const VK_LAUNCH_MAIL: u16 = 0xB4; /* Windows 2000/XP: Start Mail key */
pub(crate) const VK_MEDIA_SELECT: u16 = 0xB5; /* Windows 2000/XP: Select Media key */
pub(crate) const VK_LAUNCH_MEDIA_SELECT: u16 = 0xB5; /* Windows 2000/XP: Select Media key */
pub(crate) const VK_LAUNCH_APP1: u16 = 0xB6; /* Windows 2000/XP: Start Application 1 key */
pub(crate) const VK_LAUNCH_APP2: u16 = 0xB7; /* Windows 2000/XP: Start Application 2 key */

/* 0xB8 and 0xB9 are reserved */

/* OEM keys */

pub(crate) const VK_OEM_1: u16 = 0xBA; /* Used for miscellaneous characters; it can vary by keyboard. */
/* Windows 2000/XP: For the US standard keyboard, the ';:' key */

pub(crate) const VK_OEM_PLUS: u16 = 0xBB; /* Windows 2000/XP: For any country/region, the '+' key */
pub(crate) const VK_OEM_COMMA: u16 = 0xBC; /* Windows 2000/XP: For any country/region, the ',' key */
pub(crate) const VK_OEM_MINUS: u16 = 0xBD; /* Windows 2000/XP: For any country/region, the '-' key */
pub(crate) const VK_OEM_PERIOD: u16 = 0xBE; /* Windows 2000/XP: For any country/region, the '.' key */

pub(crate) const VK_OEM_2: u16 = 0xBF; /* Used for miscellaneous characters; it can vary by keyboard. */
/* Windows 2000/XP: For the US standard keyboard, the '/?' key */

pub(crate) const VK_OEM_3: u16 = 0xC0; /* Used for miscellaneous characters; it can vary by keyboard. */
/* Windows 2000/XP: For the US standard keyboard, the '`~' key */

/* 0xC1 to 0xD7 are reserved */
pub(crate) const VK_ABNT_C1: u16 = 0xC1; /* Brazilian (ABNT) Keyboard */
pub(crate) const VK_ABNT_C2: u16 = 0xC2; /* Brazilian (ABNT) Keyboard */

/* 0xD8 to 0xDA are unassigned */

pub(crate) const VK_OEM_4: u16 = 0xDB; /* Used for miscellaneous characters; it can vary by keyboard. */
/* Windows 2000/XP: For the US standard keyboard, the '[{' key */

pub(crate) const VK_OEM_5: u16 = 0xDC; /* Used for miscellaneous characters; it can vary by keyboard. */
/* Windows 2000/XP: For the US standard keyboard, the '\|' key */

pub(crate) const VK_OEM_6: u16 = 0xDD; /* Used for miscellaneous characters; it can vary by keyboard. */
/* Windows 2000/XP: For the US standard keyboard, the ']}' key */

pub(crate) const VK_OEM_7: u16 = 0xDE; /* Used for miscellaneous characters; it can vary by keyboard. */
/* Windows 2000/XP: For the US standard keyboard, the 'single-quote/double-quote' key */

pub(crate) const VK_OEM_8: u16 = 0xDF; /* Used for miscellaneous characters; it can vary by keyboard. */

/* 0xE0 is reserved */

pub(crate) const VK_OEM_AX: u16 = 0xE1; /* AX key on Japanese AX keyboard */

pub(crate) const VK_OEM_102: u16 = 0xE2; /* Windows 2000/XP: Either the angle bracket key or */
/* the backslash key on the RT 102-key keyboard */

/* 0xE3 and 0xE4 are OEM specific */

pub(crate) const VK_PROCESSKEY: u16 = 0xE5; /* Windows 95/98/Me, Windows NT 4.0, Windows 2000/XP: IME PROCESS key \
                                             */

/* 0xE6 is OEM specific */

pub(crate) const VK_PACKET: u16 = 0xE7; /* Windows 2000/XP: Used to pass Unicode characters as if they were keystrokes. */
/* The #define VK_PACKET key is the low word of a 32-bit Virtual Key value used */
/* for non-keyboard input methods. For more information, */
/* see Remark in KEYBDINPUT, SendInput, WM_KEYDOWN, and WM_KEYUP */

/* 0xE8 is unassigned */
/* 0xE9 to 0xF5 are OEM specific */

pub(crate) const VK_OEM_RESET: u16 = 0xE9;
pub(crate) const VK_OEM_JUMP: u16 = 0xEA;
pub(crate) const VK_OEM_PA1: u16 = 0xEB;
pub(crate) const VK_OEM_PA2: u16 = 0xEC;
pub(crate) const VK_OEM_PA3: u16 = 0xED;
pub(crate) const VK_OEM_WSCTRL: u16 = 0xEE;
pub(crate) const VK_OEM_CUSEL: u16 = 0xEF;
pub(crate) const VK_OEM_ATTN: u16 = 0xF0;
pub(crate) const VK_OEM_FINISH: u16 = 0xF1;
pub(crate) const VK_OEM_COPY: u16 = 0xF2;
pub(crate) const VK_OEM_AUTO: u16 = 0xF3;
pub(crate) const VK_OEM_ENLW: u16 = 0xF4;
pub(crate) const VK_OEM_BACKTAB: u16 = 0xF5;

pub(crate) const VK_ATTN: u16 = 0xF6; /* Attn key */
pub(crate) const VK_CRSEL: u16 = 0xF7; /* CrSel key */
pub(crate) const VK_EXSEL: u16 = 0xF8; /* ExSel key */
pub(crate) const VK_EREOF: u16 = 0xF9; /* Erase EOF key */
pub(crate) const VK_PLAY: u16 = 0xFA; /* Play key */
pub(crate) const VK_ZOOM: u16 = 0xFB; /* Zoom key */
pub(crate) const VK_NONAME: u16 = 0xFC; /* Reserved */
pub(crate) const VK_PA1: u16 = 0xFD; /* PA1 key */
pub(crate) const VK_OEM_CLEAR: u16 = 0xFE; /* Clear key */

pub(crate) const VK_NONE: u16 = 0xFF; /* no key */

/**
 * For East Asian Input Method Editors (IMEs)
 * the following additional virtual keyboard definitions must be observed.
 */

pub(crate) const VK_DBE_ALPHANUMERIC: u16 = 0xF0; /* Changes the mode to alphanumeric. */
pub(crate) const VK_DBE_KATAKANA: u16 = 0xF1; /* Changes the mode to Katakana. */
pub(crate) const VK_DBE_HIRAGANA: u16 = 0xF2; /* Changes the mode to Hiragana. */
pub(crate) const VK_DBE_SBCSCHAR: u16 = 0xF3; /* Changes the mode to single-byte characters. */
pub(crate) const VK_DBE_DBCSCHAR: u16 = 0xF4; /* Changes the mode to double-byte characters. */
pub(crate) const VK_DBE_ROMAN: u16 = 0xF5; /* Changes the mode to Roman characters. */
pub(crate) const VK_DBE_NOROMAN: u16 = 0xF6; /* Changes the mode to non-Roman characters. */
pub(crate) const VK_DBE_ENTERWORDREGISTERMODE: u16 = 0xF7; /* Activates the word registration dialog box. */
pub(crate) const VK_DBE_ENTERIMECONFIGMODE: u16 = 0xF8; /* Activates a dialog box for setting up an IME environment. */
pub(crate) const VK_DBE_FLUSHSTRING: u16 = 0xF9; /* Deletes the undetermined string without determining it. */
pub(crate) const VK_DBE_CODEINPUT: u16 = 0xFA; /* Changes the mode to code input. */
pub(crate) const VK_DBE_NOCODEINPUT: u16 = 0xFB; /* Changes the mode to no-code input. */

/*
 * Virtual Scan Codes
 */

/**
 * Keyboard Type 4
 */

pub(crate) const KBD4_T00: u16 = VK_NONE;
pub(crate) const KBD4_T01: u16 = VK_ESCAPE;
pub(crate) const KBD4_T02: u16 = VK_KEY_1;
pub(crate) const KBD4_T03: u16 = VK_KEY_2;
pub(crate) const KBD4_T04: u16 = VK_KEY_3;
pub(crate) const KBD4_T05: u16 = VK_KEY_4;
pub(crate) const KBD4_T06: u16 = VK_KEY_5;
pub(crate) const KBD4_T07: u16 = VK_KEY_6;
pub(crate) const KBD4_T08: u16 = VK_KEY_7;
pub(crate) const KBD4_T09: u16 = VK_KEY_8;
pub(crate) const KBD4_T0A: u16 = VK_KEY_9;
pub(crate) const KBD4_T0B: u16 = VK_KEY_0;
pub(crate) const KBD4_T0C: u16 = VK_OEM_MINUS;
pub(crate) const KBD4_T0D: u16 = VK_OEM_PLUS; /* NE */
pub(crate) const KBD4_T0E: u16 = VK_BACK;
pub(crate) const KBD4_T0F: u16 = VK_TAB;
pub(crate) const KBD4_T10: u16 = VK_KEY_Q;
pub(crate) const KBD4_T11: u16 = VK_KEY_W;
pub(crate) const KBD4_T12: u16 = VK_KEY_E;
pub(crate) const KBD4_T13: u16 = VK_KEY_R;
pub(crate) const KBD4_T14: u16 = VK_KEY_T;
pub(crate) const KBD4_T15: u16 = VK_KEY_Y;
pub(crate) const KBD4_T16: u16 = VK_KEY_U;
pub(crate) const KBD4_T17: u16 = VK_KEY_I;
pub(crate) const KBD4_T18: u16 = VK_KEY_O;
pub(crate) const KBD4_T19: u16 = VK_KEY_P;
pub(crate) const KBD4_T1A: u16 = VK_OEM_4; /* NE */
pub(crate) const KBD4_T1B: u16 = VK_OEM_6; /* NE */
pub(crate) const KBD4_T1C: u16 = VK_RETURN;
pub(crate) const KBD4_T1D: u16 = VK_LCONTROL;
pub(crate) const KBD4_T1E: u16 = VK_KEY_A;
pub(crate) const KBD4_T1F: u16 = VK_KEY_S;
pub(crate) const KBD4_T20: u16 = VK_KEY_D;
pub(crate) const KBD4_T21: u16 = VK_KEY_F;
pub(crate) const KBD4_T22: u16 = VK_KEY_G;
pub(crate) const KBD4_T23: u16 = VK_KEY_H;
pub(crate) const KBD4_T24: u16 = VK_KEY_J;
pub(crate) const KBD4_T25: u16 = VK_KEY_K;
pub(crate) const KBD4_T26: u16 = VK_KEY_L;
pub(crate) const KBD4_T27: u16 = VK_OEM_1; /* NE */
pub(crate) const KBD4_T28: u16 = VK_OEM_7; /* NE */
pub(crate) const KBD4_T29: u16 = VK_OEM_3; /* NE */
pub(crate) const KBD4_T2A: u16 = VK_LSHIFT;
pub(crate) const KBD4_T2B: u16 = VK_OEM_5;
pub(crate) const KBD4_T2C: u16 = VK_KEY_Z;
pub(crate) const KBD4_T2D: u16 = VK_KEY_X;
pub(crate) const KBD4_T2E: u16 = VK_KEY_C;
pub(crate) const KBD4_T2F: u16 = VK_KEY_V;
pub(crate) const KBD4_T30: u16 = VK_KEY_B;
pub(crate) const KBD4_T31: u16 = VK_KEY_N;
pub(crate) const KBD4_T32: u16 = VK_KEY_M;
pub(crate) const KBD4_T33: u16 = VK_OEM_COMMA;
pub(crate) const KBD4_T34: u16 = VK_OEM_PERIOD;
pub(crate) const KBD4_T35: u16 = VK_OEM_2;
pub(crate) const KBD4_T36: u16 = VK_RSHIFT;
pub(crate) const KBD4_T37: u16 = VK_MULTIPLY;
pub(crate) const KBD4_T38: u16 = VK_LMENU;
pub(crate) const KBD4_T39: u16 = VK_SPACE;
pub(crate) const KBD4_T3A: u16 = VK_CAPITAL;
pub(crate) const KBD4_T3B: u16 = VK_F1;
pub(crate) const KBD4_T3C: u16 = VK_F2;
pub(crate) const KBD4_T3D: u16 = VK_F3;
pub(crate) const KBD4_T3E: u16 = VK_F4;
pub(crate) const KBD4_T3F: u16 = VK_F5;
pub(crate) const KBD4_T40: u16 = VK_F6;
pub(crate) const KBD4_T41: u16 = VK_F7;
pub(crate) const KBD4_T42: u16 = VK_F8;
pub(crate) const KBD4_T43: u16 = VK_F9;
pub(crate) const KBD4_T44: u16 = VK_F10;
pub(crate) const KBD4_T45: u16 = VK_NUMLOCK;
pub(crate) const KBD4_T46: u16 = VK_SCROLL;
pub(crate) const KBD4_T47: u16 = VK_NUMPAD7; /* VK_HOME */
pub(crate) const KBD4_T48: u16 = VK_NUMPAD8; /* VK_UP */
pub(crate) const KBD4_T49: u16 = VK_NUMPAD9; /* VK_PRIOR */
pub(crate) const KBD4_T4A: u16 = VK_SUBTRACT;
pub(crate) const KBD4_T4B: u16 = VK_NUMPAD4; /* VK_LEFT */
pub(crate) const KBD4_T4C: u16 = VK_NUMPAD5; /* VK_CLEAR */
pub(crate) const KBD4_T4D: u16 = VK_NUMPAD6; /* VK_RIGHT */
pub(crate) const KBD4_T4E: u16 = VK_ADD;
pub(crate) const KBD4_T4F: u16 = VK_NUMPAD1; /* VK_END */
pub(crate) const KBD4_T50: u16 = VK_NUMPAD2; /* VK_DOWN */
pub(crate) const KBD4_T51: u16 = VK_NUMPAD3; /* VK_NEXT */
pub(crate) const KBD4_T52: u16 = VK_NUMPAD0; /* VK_INSERT */
pub(crate) const KBD4_T53: u16 = VK_DECIMAL; /* VK_DELETE */
pub(crate) const KBD4_T54: u16 = VK_SNAPSHOT;
pub(crate) const KBD4_T55: u16 = VK_NONE;
pub(crate) const KBD4_T56: u16 = VK_OEM_102; /* NE */
pub(crate) const KBD4_T57: u16 = VK_F11; /* NE */
pub(crate) const KBD4_T58: u16 = VK_F12; /* NE */
pub(crate) const KBD4_T59: u16 = VK_CLEAR;
pub(crate) const KBD4_T5A: u16 = VK_OEM_WSCTRL;
pub(crate) const KBD4_T5B: u16 = VK_OEM_FINISH;
pub(crate) const KBD4_T5C: u16 = VK_OEM_JUMP;
pub(crate) const KBD4_T5D: u16 = VK_EREOF;
pub(crate) const KBD4_T5E: u16 = VK_OEM_BACKTAB;
pub(crate) const KBD4_T5F: u16 = VK_OEM_AUTO;
pub(crate) const KBD4_T60: u16 = VK_NONE;
pub(crate) const KBD4_T61: u16 = VK_NONE;
pub(crate) const KBD4_T62: u16 = VK_ZOOM;
pub(crate) const KBD4_T63: u16 = VK_HELP;
pub(crate) const KBD4_T64: u16 = VK_F13;
pub(crate) const KBD4_T65: u16 = VK_F14;
pub(crate) const KBD4_T66: u16 = VK_F15;
pub(crate) const KBD4_T67: u16 = VK_F16;
pub(crate) const KBD4_T68: u16 = VK_F17;
pub(crate) const KBD4_T69: u16 = VK_F18;
pub(crate) const KBD4_T6A: u16 = VK_F19;
pub(crate) const KBD4_T6B: u16 = VK_F20;
pub(crate) const KBD4_T6C: u16 = VK_F21;
pub(crate) const KBD4_T6D: u16 = VK_F22;
pub(crate) const KBD4_T6E: u16 = VK_F23;
pub(crate) const KBD4_T6F: u16 = VK_OEM_PA3;
pub(crate) const KBD4_T70: u16 = VK_NONE;
pub(crate) const KBD4_T71: u16 = VK_OEM_RESET;
pub(crate) const KBD4_T72: u16 = VK_NONE;
pub(crate) const KBD4_T73: u16 = VK_ABNT_C1;
pub(crate) const KBD4_T74: u16 = VK_NONE;
pub(crate) const KBD4_T75: u16 = VK_NONE;
pub(crate) const KBD4_T76: u16 = VK_F24;
pub(crate) const KBD4_T77: u16 = VK_NONE;
pub(crate) const KBD4_T78: u16 = VK_NONE;
pub(crate) const KBD4_T79: u16 = VK_NONE;
pub(crate) const KBD4_T7A: u16 = VK_NONE;
pub(crate) const KBD4_T7B: u16 = VK_OEM_PA1;
pub(crate) const KBD4_T7C: u16 = VK_TAB;
pub(crate) const KBD4_T7D: u16 = VK_NONE;
pub(crate) const KBD4_T7E: u16 = VK_ABNT_C2;
pub(crate) const KBD4_T7F: u16 = VK_OEM_PA2;

pub(crate) const KBD4_X10: u16 = VK_MEDIA_PREV_TRACK;
pub(crate) const KBD4_X19: u16 = VK_MEDIA_NEXT_TRACK;
pub(crate) const KBD4_X1C: u16 = VK_RETURN;
pub(crate) const KBD4_X1D: u16 = VK_RCONTROL;
pub(crate) const KBD4_X20: u16 = VK_VOLUME_MUTE;
pub(crate) const KBD4_X21: u16 = VK_LAUNCH_APP2;
pub(crate) const KBD4_X22: u16 = VK_MEDIA_PLAY_PAUSE;
pub(crate) const KBD4_X24: u16 = VK_MEDIA_STOP;
pub(crate) const KBD4_X2E: u16 = VK_VOLUME_DOWN;
pub(crate) const KBD4_X30: u16 = VK_VOLUME_UP;
pub(crate) const KBD4_X32: u16 = VK_BROWSER_HOME;
pub(crate) const KBD4_X35: u16 = VK_DIVIDE;
pub(crate) const KBD4_X37: u16 = VK_SNAPSHOT;
pub(crate) const KBD4_X38: u16 = VK_RMENU;
pub(crate) const KBD4_X46: u16 = VK_PAUSE; /* VK_CANCEL */
pub(crate) const KBD4_X47: u16 = VK_HOME;
pub(crate) const KBD4_X48: u16 = VK_UP;
pub(crate) const KBD4_X49: u16 = VK_PRIOR;
pub(crate) const KBD4_X4B: u16 = VK_LEFT;
pub(crate) const KBD4_X4D: u16 = VK_RIGHT;
pub(crate) const KBD4_X4F: u16 = VK_END;
pub(crate) const KBD4_X50: u16 = VK_DOWN;
pub(crate) const KBD4_X51: u16 = VK_NEXT; /* NE */
pub(crate) const KBD4_X52: u16 = VK_INSERT;
pub(crate) const KBD4_X53: u16 = VK_DELETE;
pub(crate) const KBD4_X5B: u16 = VK_LWIN;
pub(crate) const KBD4_X5C: u16 = VK_RWIN;
pub(crate) const KBD4_X5D: u16 = VK_APPS;
pub(crate) const KBD4_X5E: u16 = VK_POWER;
pub(crate) const KBD4_X5F: u16 = VK_SLEEP;
pub(crate) const KBD4_X65: u16 = VK_BROWSER_SEARCH;
pub(crate) const KBD4_X66: u16 = VK_BROWSER_FAVORITES;
pub(crate) const KBD4_X67: u16 = VK_BROWSER_REFRESH;
pub(crate) const KBD4_X68: u16 = VK_BROWSER_STOP;
pub(crate) const KBD4_X69: u16 = VK_BROWSER_FORWARD;
pub(crate) const KBD4_X6A: u16 = VK_BROWSER_BACK;
pub(crate) const KBD4_X6B: u16 = VK_LAUNCH_APP1;
pub(crate) const KBD4_X6C: u16 = VK_LAUNCH_MAIL;
pub(crate) const KBD4_X6D: u16 = VK_LAUNCH_MEDIA_SELECT;

pub(crate) const KBD4_Y1D: u16 = VK_PAUSE;

/**
 * Keyboard Type 7
 */

pub(crate) const KBD7_T00: u16 = VK_NONE;
pub(crate) const KBD7_T01: u16 = VK_ESCAPE;
pub(crate) const KBD7_T02: u16 = VK_KEY_1;
pub(crate) const KBD7_T03: u16 = VK_KEY_2;
pub(crate) const KBD7_T04: u16 = VK_KEY_3;
pub(crate) const KBD7_T05: u16 = VK_KEY_4;
pub(crate) const KBD7_T06: u16 = VK_KEY_5;
pub(crate) const KBD7_T07: u16 = VK_KEY_6;
pub(crate) const KBD7_T08: u16 = VK_KEY_7;
pub(crate) const KBD7_T09: u16 = VK_KEY_8;
pub(crate) const KBD7_T0A: u16 = VK_KEY_9;
pub(crate) const KBD7_T0B: u16 = VK_KEY_0;
pub(crate) const KBD7_T0C: u16 = VK_OEM_MINUS;
pub(crate) const KBD7_T0D: u16 = VK_OEM_PLUS;
pub(crate) const KBD7_T0E: u16 = VK_BACK;
pub(crate) const KBD7_T0F: u16 = VK_TAB;
pub(crate) const KBD7_T10: u16 = VK_KEY_Q;
pub(crate) const KBD7_T11: u16 = VK_KEY_W;
pub(crate) const KBD7_T12: u16 = VK_KEY_E;
pub(crate) const KBD7_T13: u16 = VK_KEY_R;
pub(crate) const KBD7_T14: u16 = VK_KEY_T;
pub(crate) const KBD7_T15: u16 = VK_KEY_Y;
pub(crate) const KBD7_T16: u16 = VK_KEY_U;
pub(crate) const KBD7_T17: u16 = VK_KEY_I;
pub(crate) const KBD7_T18: u16 = VK_KEY_O;
pub(crate) const KBD7_T19: u16 = VK_KEY_P;
pub(crate) const KBD7_T1A: u16 = VK_OEM_4; /* NE */
pub(crate) const KBD7_T1B: u16 = VK_OEM_6; /* NE */
pub(crate) const KBD7_T1C: u16 = VK_RETURN;
pub(crate) const KBD7_T1D: u16 = VK_LCONTROL;
pub(crate) const KBD7_T1E: u16 = VK_KEY_A;
pub(crate) const KBD7_T1F: u16 = VK_KEY_S;
pub(crate) const KBD7_T20: u16 = VK_KEY_D;
pub(crate) const KBD7_T21: u16 = VK_KEY_F;
pub(crate) const KBD7_T22: u16 = VK_KEY_G;
pub(crate) const KBD7_T23: u16 = VK_KEY_H;
pub(crate) const KBD7_T24: u16 = VK_KEY_J;
pub(crate) const KBD7_T25: u16 = VK_KEY_K;
pub(crate) const KBD7_T26: u16 = VK_KEY_L;
pub(crate) const KBD7_T27: u16 = VK_OEM_1;
pub(crate) const KBD7_T28: u16 = VK_OEM_7;
pub(crate) const KBD7_T29: u16 = VK_OEM_3; /* NE */
pub(crate) const KBD7_T2A: u16 = VK_LSHIFT;
pub(crate) const KBD7_T2B: u16 = VK_OEM_5; /* NE */
pub(crate) const KBD7_T2C: u16 = VK_KEY_Z;
pub(crate) const KBD7_T2D: u16 = VK_KEY_X;
pub(crate) const KBD7_T2E: u16 = VK_KEY_C;
pub(crate) const KBD7_T2F: u16 = VK_KEY_V;
pub(crate) const KBD7_T30: u16 = VK_KEY_B;
pub(crate) const KBD7_T31: u16 = VK_KEY_N;
pub(crate) const KBD7_T32: u16 = VK_KEY_M;
pub(crate) const KBD7_T33: u16 = VK_OEM_COMMA;
pub(crate) const KBD7_T34: u16 = VK_OEM_PERIOD;
pub(crate) const KBD7_T35: u16 = VK_OEM_2;
pub(crate) const KBD7_T36: u16 = VK_RSHIFT;
pub(crate) const KBD7_T37: u16 = VK_MULTIPLY;
pub(crate) const KBD7_T38: u16 = VK_LMENU;
pub(crate) const KBD7_T39: u16 = VK_SPACE;
pub(crate) const KBD7_T3A: u16 = VK_CAPITAL;
pub(crate) const KBD7_T3B: u16 = VK_F1;
pub(crate) const KBD7_T3C: u16 = VK_F2;
pub(crate) const KBD7_T3D: u16 = VK_F3;
pub(crate) const KBD7_T3E: u16 = VK_F4;
pub(crate) const KBD7_T3F: u16 = VK_F5;
pub(crate) const KBD7_T40: u16 = VK_F6;
pub(crate) const KBD7_T41: u16 = VK_F7;
pub(crate) const KBD7_T42: u16 = VK_F8;
pub(crate) const KBD7_T43: u16 = VK_F9;
pub(crate) const KBD7_T44: u16 = VK_F10;
pub(crate) const KBD7_T45: u16 = VK_NUMLOCK;
pub(crate) const KBD7_T46: u16 = VK_SCROLL;
pub(crate) const KBD7_T47: u16 = VK_NUMPAD7; /* VK_HOME */
pub(crate) const KBD7_T48: u16 = VK_NUMPAD8; /* VK_UP */
pub(crate) const KBD7_T49: u16 = VK_NUMPAD9; /* VK_PRIOR */
pub(crate) const KBD7_T4A: u16 = VK_SUBTRACT;
pub(crate) const KBD7_T4B: u16 = VK_NUMPAD4; /* VK_LEFT */
pub(crate) const KBD7_T4C: u16 = VK_NUMPAD5; /* VK_CLEAR */
pub(crate) const KBD7_T4D: u16 = VK_NUMPAD6; /* VK_RIGHT */
pub(crate) const KBD7_T4E: u16 = VK_ADD;
pub(crate) const KBD7_T4F: u16 = VK_NUMPAD1; /* VK_END */
pub(crate) const KBD7_T50: u16 = VK_NUMPAD2; /* VK_DOWN */
pub(crate) const KBD7_T51: u16 = VK_NUMPAD3; /* VK_NEXT */
pub(crate) const KBD7_T52: u16 = VK_NUMPAD0; /* VK_INSERT */
pub(crate) const KBD7_T53: u16 = VK_DECIMAL; /* VK_DELETE */
pub(crate) const KBD7_T54: u16 = VK_SNAPSHOT;
pub(crate) const KBD7_T55: u16 = VK_NONE;
pub(crate) const KBD7_T56: u16 = VK_OEM_102;
pub(crate) const KBD7_T57: u16 = VK_F11;
pub(crate) const KBD7_T58: u16 = VK_F12;
pub(crate) const KBD7_T59: u16 = VK_CLEAR;
pub(crate) const KBD7_T5A: u16 = VK_NONAME; /* NE */
pub(crate) const KBD7_T5B: u16 = VK_NONAME; /* NE */
pub(crate) const KBD7_T5C: u16 = VK_NONAME; /* NE */
pub(crate) const KBD7_T5D: u16 = VK_EREOF;
pub(crate) const KBD7_T5E: u16 = VK_NONE; /* NE */
pub(crate) const KBD7_T5F: u16 = VK_NONAME; /* NE */
pub(crate) const KBD7_T60: u16 = VK_NONE;
pub(crate) const KBD7_T61: u16 = VK_NONE; /* NE */
pub(crate) const KBD7_T62: u16 = VK_NONE; /* NE */
pub(crate) const KBD7_T63: u16 = VK_NONE;
pub(crate) const KBD7_T64: u16 = VK_F13;
pub(crate) const KBD7_T65: u16 = VK_F14;
pub(crate) const KBD7_T66: u16 = VK_F15;
pub(crate) const KBD7_T67: u16 = VK_F16;
pub(crate) const KBD7_T68: u16 = VK_F17;
pub(crate) const KBD7_T69: u16 = VK_F18;
pub(crate) const KBD7_T6A: u16 = VK_F19;
pub(crate) const KBD7_T6B: u16 = VK_F20;
pub(crate) const KBD7_T6C: u16 = VK_F21;
pub(crate) const KBD7_T6D: u16 = VK_F22;
pub(crate) const KBD7_T6E: u16 = VK_F23;
pub(crate) const KBD7_T6F: u16 = VK_NONE; /* NE */
pub(crate) const KBD7_T70: u16 = VK_HKTG; /* NE */
pub(crate) const KBD7_T71: u16 = VK_NONE; /* NE */
pub(crate) const KBD7_T72: u16 = VK_NONE;
pub(crate) const KBD7_T73: u16 = VK_ABNT_C1;
pub(crate) const KBD7_T74: u16 = VK_NONE;
pub(crate) const KBD7_T75: u16 = VK_NONE;
pub(crate) const KBD7_T76: u16 = VK_F24;
pub(crate) const KBD7_T77: u16 = VK_NONE;
pub(crate) const KBD7_T78: u16 = VK_NONE;
pub(crate) const KBD7_T79: u16 = VK_CONVERT; /* NE */
pub(crate) const KBD7_T7A: u16 = VK_NONE;
pub(crate) const KBD7_T7B: u16 = VK_NONCONVERT; /* NE */
pub(crate) const KBD7_T7C: u16 = VK_TAB;
pub(crate) const KBD7_T7D: u16 = VK_OEM_8;
pub(crate) const KBD7_T7E: u16 = VK_ABNT_C2;
pub(crate) const KBD7_T7F: u16 = VK_OEM_PA2;

pub(crate) const KBD7_X10: u16 = VK_MEDIA_PREV_TRACK;
pub(crate) const KBD7_X19: u16 = VK_MEDIA_NEXT_TRACK;
pub(crate) const KBD7_X1C: u16 = VK_RETURN;
pub(crate) const KBD7_X1D: u16 = VK_RCONTROL;
pub(crate) const KBD7_X20: u16 = VK_VOLUME_MUTE;
pub(crate) const KBD7_X21: u16 = VK_LAUNCH_APP2;
pub(crate) const KBD7_X22: u16 = VK_MEDIA_PLAY_PAUSE;
pub(crate) const KBD7_X24: u16 = VK_MEDIA_STOP;
pub(crate) const KBD7_X2E: u16 = VK_VOLUME_DOWN;
pub(crate) const KBD7_X30: u16 = VK_VOLUME_UP;
pub(crate) const KBD7_X32: u16 = VK_BROWSER_HOME;
pub(crate) const KBD7_X33: u16 = VK_NONE;
pub(crate) const KBD7_X35: u16 = VK_DIVIDE;
pub(crate) const KBD7_X37: u16 = VK_SNAPSHOT;
pub(crate) const KBD7_X38: u16 = VK_RMENU;
pub(crate) const KBD7_X42: u16 = VK_NONE;
pub(crate) const KBD7_X43: u16 = VK_NONE;
pub(crate) const KBD7_X44: u16 = VK_NONE;
pub(crate) const KBD7_X46: u16 = VK_CANCEL;
pub(crate) const KBD7_X47: u16 = VK_HOME;
pub(crate) const KBD7_X48: u16 = VK_UP;
pub(crate) const KBD7_X49: u16 = VK_PRIOR;
pub(crate) const KBD7_X4B: u16 = VK_LEFT;
pub(crate) const KBD7_X4D: u16 = VK_RIGHT;
pub(crate) const KBD7_X4F: u16 = VK_END;
pub(crate) const KBD7_X50: u16 = VK_DOWN;
pub(crate) const KBD7_X51: u16 = VK_NEXT;
pub(crate) const KBD7_X52: u16 = VK_INSERT;
pub(crate) const KBD7_X53: u16 = VK_DELETE;
pub(crate) const KBD7_X5B: u16 = VK_LWIN;
pub(crate) const KBD7_X5C: u16 = VK_RWIN;
pub(crate) const KBD7_X5D: u16 = VK_APPS;
pub(crate) const KBD7_X5E: u16 = VK_POWER;
pub(crate) const KBD7_X5F: u16 = VK_SLEEP;
pub(crate) const KBD7_X65: u16 = VK_BROWSER_SEARCH;
pub(crate) const KBD7_X66: u16 = VK_BROWSER_FAVORITES;
pub(crate) const KBD7_X67: u16 = VK_BROWSER_REFRESH;
pub(crate) const KBD7_X68: u16 = VK_BROWSER_STOP;
pub(crate) const KBD7_X69: u16 = VK_BROWSER_FORWARD;
pub(crate) const KBD7_X6A: u16 = VK_BROWSER_BACK;
pub(crate) const KBD7_X6B: u16 = VK_LAUNCH_APP1;
pub(crate) const KBD7_X6C: u16 = VK_LAUNCH_MAIL;
pub(crate) const KBD7_X6D: u16 = VK_LAUNCH_MEDIA_SELECT;
pub(crate) const KBD7_XF1: u16 = VK_NONE; /* NE */
pub(crate) const KBD7_XF2: u16 = VK_NONE; /* NE */

pub(crate) const KBD7_Y1D: u16 = VK_PAUSE;

pub(crate) const KEY_CODE_TO_VKCODE_APPLE: [u16; 256] = [
    VK_KEY_A,             /* APPLE_VK_ANSI_A (0x00) */
    VK_KEY_S,             /* APPLE_VK_ANSI_S (0x01) */
    VK_KEY_D,             /* APPLE_VK_ANSI_D (0x02) */
    VK_KEY_F,             /* APPLE_VK_ANSI_F (0x03) */
    VK_KEY_H,             /* APPLE_VK_ANSI_H (0x04) */
    VK_KEY_G,             /* APPLE_VK_ANSI_G (0x05) */
    VK_KEY_Z,             /* APPLE_VK_ANSI_Z (0x06) */
    VK_KEY_X,             /* APPLE_VK_ANSI_X (0x07) */
    VK_KEY_C,             /* APPLE_VK_ANSI_C (0x08) */
    VK_KEY_V,             /* APPLE_VK_ANSI_V (0x09) */
    VK_OEM_102,           /* APPLE_VK_ISO_Section (0x0A) */
    VK_KEY_B,             /* APPLE_VK_ANSI_B (0x0B) */
    VK_KEY_Q,             /* APPLE_VK_ANSI_Q (0x0C) */
    VK_KEY_W,             /* APPLE_VK_ANSI_W (0x0D) */
    VK_KEY_E,             /* APPLE_VK_ANSI_E (0x0E) */
    VK_KEY_R,             /* APPLE_VK_ANSI_R (0x0F) */
    VK_KEY_Y,             /* APPLE_VK_ANSI_Y (0x10) */
    VK_KEY_T,             /* APPLE_VK_ANSI_T (0x11) */
    VK_KEY_1,             /* APPLE_VK_ANSI_1 (0x12) */
    VK_KEY_2,             /* APPLE_VK_ANSI_2 (0x13) */
    VK_KEY_3,             /* APPLE_VK_ANSI_3 (0x14) */
    VK_KEY_4,             /* APPLE_VK_ANSI_4 (0x15) */
    VK_KEY_6,             /* APPLE_VK_ANSI_6 (0x16) */
    VK_KEY_5,             /* APPLE_VK_ANSI_5 (0x17) */
    VK_OEM_PLUS,          /* APPLE_VK_ANSI_Equal (0x18) */
    VK_KEY_9,             /* APPLE_VK_ANSI_9 (0x19) */
    VK_KEY_7,             /* APPLE_VK_ANSI_7 (0x1A) */
    VK_OEM_MINUS,         /* APPLE_VK_ANSI_Minus (0x1B) */
    VK_KEY_8,             /* APPLE_VK_ANSI_8 (0x1C) */
    VK_KEY_0,             /* APPLE_VK_ANSI_0 (0x1D) */
    VK_OEM_6,             /* APPLE_VK_ANSI_RightBracket (0x1E) */
    VK_KEY_O,             /* APPLE_VK_ANSI_O (0x1F) */
    VK_KEY_U,             /* APPLE_VK_ANSI_U (0x20) */
    VK_OEM_4,             /* APPLE_VK_ANSI_LeftBracket (0x21) */
    VK_KEY_I,             /* APPLE_VK_ANSI_I (0x22) */
    VK_KEY_P,             /* APPLE_VK_ANSI_P (0x23) */
    VK_RETURN,            /* APPLE_VK_Return (0x24) */
    VK_KEY_L,             /* APPLE_VK_ANSI_L (0x25) */
    VK_KEY_J,             /* APPLE_VK_ANSI_J (0x26) */
    VK_OEM_7,             /* APPLE_VK_ANSI_Quote (0x27) */
    VK_KEY_K,             /* APPLE_VK_ANSI_K (0x28) */
    VK_OEM_1,             /* APPLE_VK_ANSI_Semicolon (0x29) */
    VK_OEM_5,             /* APPLE_VK_ANSI_Backslash (0x2A) */
    VK_OEM_COMMA,         /* APPLE_VK_ANSI_Comma (0x2B) */
    VK_OEM_2,             /* APPLE_VK_ANSI_Slash (0x2C) */
    VK_KEY_N,             /* APPLE_VK_ANSI_N (0x2D) */
    VK_KEY_M,             /* APPLE_VK_ANSI_M (0x2E) */
    VK_OEM_PERIOD,        /* APPLE_VK_ANSI_Period (0x2F) */
    VK_TAB,               /* APPLE_VK_Tab (0x30) */
    VK_SPACE,             /* APPLE_VK_Space (0x31) */
    VK_OEM_3,             /* APPLE_VK_ANSI_Grave (0x32) */
    VK_BACK,              /* APPLE_VK_Delete (0x33) */
    0,                    /* APPLE_VK_0x34 (0x34) */
    VK_ESCAPE,            /* APPLE_VK_Escape (0x35) */
    VK_RWIN | KBDEXT,     /* APPLE_VK_RightCommand (0x36) */
    VK_LWIN | KBDEXT,     /* APPLE_VK_Command (0x37) */
    VK_LSHIFT,            /* APPLE_VK_Shift (0x38) */
    VK_CAPITAL,           /* APPLE_VK_CapsLock (0x39) */
    VK_LMENU,             /* APPLE_VK_Option (0x3A) */
    VK_LCONTROL,          /* APPLE_VK_Control (0x3B) */
    VK_RSHIFT,            /* APPLE_VK_RightShift (0x3C) */
    VK_RMENU | KBDEXT,    /* APPLE_VK_RightOption (0x3D) */
    VK_RWIN | KBDEXT,     /* APPLE_VK_RightControl (0x3E) */
    VK_RWIN | KBDEXT,     /* APPLE_VK_Function (0x3F) */
    VK_F17,               /* APPLE_VK_F17 (0x40) */
    VK_DECIMAL,           /* APPLE_VK_ANSI_KeypadDecimal (0x41) */
    0,                    /* APPLE_VK_0x42 (0x42) */
    VK_MULTIPLY,          /* APPLE_VK_ANSI_KeypadMultiply (0x43) */
    0,                    /* APPLE_VK_0x44 (0x44) */
    VK_ADD,               /* APPLE_VK_ANSI_KeypadPlus (0x45) */
    0,                    /* APPLE_VK_0x46 (0x46) */
    VK_NUMLOCK,           /* APPLE_VK_ANSI_KeypadClear (0x47) */
    VK_VOLUME_UP,         /* APPLE_VK_VolumeUp (0x48) */
    VK_VOLUME_DOWN,       /* APPLE_VK_VolumeDown (0x49) */
    VK_VOLUME_MUTE,       /* APPLE_VK_Mute (0x4A) */
    VK_DIVIDE | KBDEXT,   /* APPLE_VK_ANSI_KeypadDivide (0x4B) */
    VK_RETURN | KBDEXT,   /* APPLE_VK_ANSI_KeypadEnter (0x4C) */
    0,                    /* APPLE_VK_0x4D (0x4D) */
    VK_SUBTRACT,          /* APPLE_VK_ANSI_KeypadMinus (0x4E) */
    VK_F18,               /* APPLE_VK_F18 (0x4F) */
    VK_F19,               /* APPLE_VK_F19 (0x50) */
    VK_CLEAR | KBDEXT,    /* APPLE_VK_ANSI_KeypadEquals (0x51) */
    VK_NUMPAD0,           /* APPLE_VK_ANSI_Keypad0 (0x52) */
    VK_NUMPAD1,           /* APPLE_VK_ANSI_Keypad1 (0x53) */
    VK_NUMPAD2,           /* APPLE_VK_ANSI_Keypad2 (0x54) */
    VK_NUMPAD3,           /* APPLE_VK_ANSI_Keypad3 (0x55) */
    VK_NUMPAD4,           /* APPLE_VK_ANSI_Keypad4 (0x56) */
    VK_NUMPAD5,           /* APPLE_VK_ANSI_Keypad5 (0x57) */
    VK_NUMPAD6,           /* APPLE_VK_ANSI_Keypad6 (0x58) */
    VK_NUMPAD7,           /* APPLE_VK_ANSI_Keypad7 (0x59) */
    VK_F20,               /* APPLE_VK_F20 (0x5A) */
    VK_NUMPAD8,           /* APPLE_VK_ANSI_Keypad8 (0x5B) */
    VK_NUMPAD9,           /* APPLE_VK_ANSI_Keypad9 (0x5C) */
    0,                    /* APPLE_VK_JIS_Yen (0x5D) */
    0,                    /* APPLE_VK_JIS_Underscore (0x5E) */
    VK_DECIMAL,           /* APPLE_VK_JIS_KeypadComma (0x5F) */
    VK_F5,                /* APPLE_VK_F5 (0x60) */
    VK_F6,                /* APPLE_VK_F6 (0x61) */
    VK_F7,                /* APPLE_VK_F7 (0x62) */
    VK_F3,                /* APPLE_VK_F3 (0x63) */
    VK_F8,                /* APPLE_VK_F8 (0x64) */
    VK_F9,                /* APPLE_VK_F9 (0x65) */
    0,                    /* APPLE_VK_JIS_Eisu (0x66) */
    VK_F11,               /* APPLE_VK_F11 (0x67) */
    0,                    /* APPLE_VK_JIS_Kana (0x68) */
    VK_SNAPSHOT | KBDEXT, /* APPLE_VK_F13 (0x69) */
    VK_F16,               /* APPLE_VK_F16 (0x6A) */
    VK_F14,               /* APPLE_VK_F14 (0x6B) */
    0,                    /* APPLE_VK_0x6C (0x6C) */
    VK_F10,               /* APPLE_VK_F10 (0x6D) */
    0,                    /* APPLE_VK_0x6E (0x6E) */
    VK_F12,               /* APPLE_VK_F12 (0x6F) */
    0,                    /* APPLE_VK_0x70 (0x70) */
    VK_PAUSE | KBDEXT,    /* APPLE_VK_F15 (0x71) */
    VK_INSERT | KBDEXT,   /* APPLE_VK_Help (0x72) */
    VK_HOME | KBDEXT,     /* APPLE_VK_Home (0x73) */
    VK_PRIOR | KBDEXT,    /* APPLE_VK_PageUp (0x74) */
    VK_DELETE | KBDEXT,   /* APPLE_VK_ForwardDelete (0x75) */
    VK_F4,                /* APPLE_VK_F4 (0x76) */
    VK_END | KBDEXT,      /* APPLE_VK_End (0x77) */
    VK_F2,                /* APPLE_VK_F2 (0x78) */
    VK_NEXT | KBDEXT,     /* APPLE_VK_PageDown (0x79) */
    VK_F1,                /* APPLE_VK_F1 (0x7A) */
    VK_LEFT | KBDEXT,     /* APPLE_VK_LeftArrow (0x7B) */
    VK_RIGHT | KBDEXT,    /* APPLE_VK_RightArrow (0x7C) */
    VK_DOWN | KBDEXT,     /* APPLE_VK_DownArrow (0x7D) */
    VK_UP | KBDEXT,       /* APPLE_VK_UpArrow (0x7E) */
    0,                    /* 127 */
    0,                    /* 128 */
    0,                    /* 129 */
    0,                    /* 130 */
    0,                    /* 131 */
    0,                    /* 132 */
    0,                    /* 133 */
    0,                    /* 134 */
    0,                    /* 135 */
    0,                    /* 136 */
    0,                    /* 137 */
    0,                    /* 138 */
    0,                    /* 139 */
    0,                    /* 140 */
    0,                    /* 141 */
    0,                    /* 142 */
    0,                    /* 143 */
    0,                    /* 144 */
    0,                    /* 145 */
    0,                    /* 146 */
    0,                    /* 147 */
    0,                    /* 148 */
    0,                    /* 149 */
    0,                    /* 150 */
    0,                    /* 151 */
    0,                    /* 152 */
    0,                    /* 153 */
    0,                    /* 154 */
    0,                    /* 155 */
    0,                    /* 156 */
    0,                    /* 157 */
    0,                    /* 158 */
    0,                    /* 159 */
    0,                    /* 160 */
    0,                    /* 161 */
    0,                    /* 162 */
    0,                    /* 163 */
    0,                    /* 164 */
    0,                    /* 165 */
    0,                    /* 166 */
    0,                    /* 167 */
    0,                    /* 168 */
    0,                    /* 169 */
    0,                    /* 170 */
    0,                    /* 171 */
    0,                    /* 172 */
    0,                    /* 173 */
    0,                    /* 174 */
    0,                    /* 175 */
    0,                    /* 176 */
    0,                    /* 177 */
    0,                    /* 178 */
    0,                    /* 179 */
    0,                    /* 180 */
    0,                    /* 181 */
    0,                    /* 182 */
    0,                    /* 183 */
    0,                    /* 184 */
    0,                    /* 185 */
    0,                    /* 186 */
    0,                    /* 187 */
    0,                    /* 188 */
    0,                    /* 189 */
    0,                    /* 190 */
    0,                    /* 191 */
    0,                    /* 192 */
    0,                    /* 193 */
    0,                    /* 194 */
    0,                    /* 195 */
    0,                    /* 196 */
    0,                    /* 197 */
    0,                    /* 198 */
    0,                    /* 199 */
    0,                    /* 200 */
    0,                    /* 201 */
    0,                    /* 202 */
    0,                    /* 203 */
    0,                    /* 204 */
    0,                    /* 205 */
    0,                    /* 206 */
    0,                    /* 207 */
    0,                    /* 208 */
    0,                    /* 209 */
    0,                    /* 210 */
    0,                    /* 211 */
    0,                    /* 212 */
    0,                    /* 213 */
    0,                    /* 214 */
    0,                    /* 215 */
    0,                    /* 216 */
    0,                    /* 217 */
    0,                    /* 218 */
    0,                    /* 219 */
    0,                    /* 220 */
    0,                    /* 221 */
    0,                    /* 222 */
    0,                    /* 223 */
    0,                    /* 224 */
    0,                    /* 225 */
    0,                    /* 226 */
    0,                    /* 227 */
    0,                    /* 228 */
    0,                    /* 229 */
    0,                    /* 230 */
    0,                    /* 231 */
    0,                    /* 232 */
    0,                    /* 233 */
    0,                    /* 234 */
    0,                    /* 235 */
    0,                    /* 236 */
    0,                    /* 237 */
    0,                    /* 238 */
    0,                    /* 239 */
    0,                    /* 240 */
    0,                    /* 241 */
    0,                    /* 242 */
    0,                    /* 243 */
    0,                    /* 244 */
    0,                    /* 245 */
    0,                    /* 246 */
    0,                    /* 247 */
    0,                    /* 248 */
    0,                    /* 249 */
    0,                    /* 250 */
    0,                    /* 251 */
    0,                    /* 252 */
    0,                    /* 253 */
    0,                    /* 254 */
    0,                    /* 255 */
];

/**
 * evdev (Linux)
 *
 * Refer to linux/input-event-codes.h
 */

pub(crate) const KEYCODE_TO_VKCODE_EVDEV: [u16; 256] = [
    0,                       /* KEY_RESERVED (0) */
    VK_ESCAPE,               /* KEY_ESC (1) */
    VK_KEY_1,                /* KEY_1 (2) */
    VK_KEY_2,                /* KEY_2 (3) */
    VK_KEY_3,                /* KEY_3 (4) */
    VK_KEY_4,                /* KEY_4 (5) */
    VK_KEY_5,                /* KEY_5 (6) */
    VK_KEY_6,                /* KEY_6 (7) */
    VK_KEY_7,                /* KEY_7 (8) */
    VK_KEY_8,                /* KEY_8 (9) */
    VK_KEY_9,                /* KEY_9 (10) */
    VK_KEY_0,                /* KEY_0 (11) */
    VK_OEM_MINUS,            /* KEY_MINUS (12) */
    VK_OEM_PLUS,             /* KEY_EQUAL (13) */
    VK_BACK,                 /* KEY_BACKSPACE (14) */
    VK_TAB,                  /* KEY_TAB (15) */
    VK_KEY_Q,                /* KEY_Q (16) */
    VK_KEY_W,                /* KEY_W (17) */
    VK_KEY_E,                /* KEY_E (18) */
    VK_KEY_R,                /* KEY_R (19) */
    VK_KEY_T,                /* KEY_T (20) */
    VK_KEY_Y,                /* KEY_Y (21) */
    VK_KEY_U,                /* KEY_U (22) */
    VK_KEY_I,                /* KEY_I (23) */
    VK_KEY_O,                /* KEY_O (24) */
    VK_KEY_P,                /* KEY_P (25) */
    VK_OEM_4,                /* KEY_LEFTBRACE (26) */
    VK_OEM_6,                /* KEY_RIGHTBRACE (27) */
    VK_RETURN,               /* KEY_ENTER (28) */
    VK_LCONTROL,             /* KEY_LEFTCTRL (29) */
    VK_KEY_A,                /* KEY_A (30) */
    VK_KEY_S,                /* KEY_S (31) */
    VK_KEY_D,                /* KEY_D (32) */
    VK_KEY_F,                /* KEY_F (33) */
    VK_KEY_G,                /* KEY_G (34) */
    VK_KEY_H,                /* KEY_H (35) */
    VK_KEY_J,                /* KEY_J (36) */
    VK_KEY_K,                /* KEY_K (37) */
    VK_KEY_L,                /* KEY_L (38) */
    VK_OEM_1,                /* KEY_SEMICOLON (39) */
    VK_OEM_7,                /* KEY_APOSTROPHE (40) */
    VK_OEM_3,                /* KEY_GRAVE (41) */
    VK_LSHIFT,               /* KEY_LEFTSHIFT (42) */
    VK_OEM_5,                /* KEY_BACKSLASH (43) */
    VK_KEY_Z,                /* KEY_Z (44) */
    VK_KEY_X,                /* KEY_X (45) */
    VK_KEY_C,                /* KEY_C (46) */
    VK_KEY_V,                /* KEY_V (47) */
    VK_KEY_B,                /* KEY_B (48) */
    VK_KEY_N,                /* KEY_N (49) */
    VK_KEY_M,                /* KEY_M (50) */
    VK_OEM_COMMA,            /* KEY_COMMA (51) */
    VK_OEM_PERIOD,           /* KEY_DOT (52) */
    VK_OEM_2,                /* KEY_SLASH (53) */
    VK_RSHIFT,               /* KEY_RIGHTSHIFT (54) */
    VK_MULTIPLY,             /* KEY_KPASTERISK (55) */
    VK_LMENU,                /* KEY_LEFTALT (56) */
    VK_SPACE,                /* KEY_SPACE (57) */
    VK_CAPITAL,              /* KEY_CAPSLOCK (58) */
    VK_F1,                   /* KEY_F1 (59) */
    VK_F2,                   /* KEY_F2 (60) */
    VK_F3,                   /* KEY_F3 (61) */
    VK_F4,                   /* KEY_F4 (62) */
    VK_F5,                   /* KEY_F5 (63) */
    VK_F6,                   /* KEY_F6 (64) */
    VK_F7,                   /* KEY_F7 (65) */
    VK_F8,                   /* KEY_F8 (66) */
    VK_F9,                   /* KEY_F9 (67) */
    VK_F10,                  /* KEY_F10 (68) */
    VK_NUMLOCK,              /* KEY_NUMLOCK (69) */
    VK_SCROLL,               /* KEY_SCROLLLOCK (70) */
    VK_NUMPAD7,              /* KEY_KP7 (71) */
    VK_NUMPAD8,              /* KEY_KP8 (72) */
    VK_NUMPAD9,              /* KEY_KP9 (73) */
    VK_SUBTRACT,             /* KEY_KPMINUS (74) */
    VK_NUMPAD4,              /* KEY_KP4 (75) */
    VK_NUMPAD5,              /* KEY_KP5 (76) */
    VK_NUMPAD6,              /* KEY_KP6 (77) */
    VK_ADD,                  /* KEY_KPPLUS (78) */
    VK_NUMPAD1,              /* KEY_KP1 (79) */
    VK_NUMPAD2,              /* KEY_KP2 (80) */
    VK_NUMPAD3,              /* KEY_KP3 (81) */
    VK_NUMPAD0,              /* KEY_KP0 (82) */
    VK_DECIMAL,              /* KEY_KPDOT (83) */
    0,                       /* (84) */
    0,                       /* KEY_ZENKAKUHANKAKU (85) */
    VK_OEM_102,              /* KEY_102 (86) */
    VK_F11,                  /* KEY_F11 (87) */
    VK_F12,                  /* KEY_F12 (88) */
    VK_ABNT_C1,              /* KEY_RO (89) */
    VK_DBE_KATAKANA,         /* KEY_KATAKANA (90) */
    VK_DBE_HIRAGANA,         /* KEY_HIRAGANA (91) */
    VK_CONVERT,              /* KEY_HENKAN (92) */
    VK_HKTG,                 /* KEY_KATAKANAHIRAGANA (93) */
    VK_NONCONVERT,           /* KEY_MUHENKAN (94) */
    0,                       /* KEY_KPJPCOMMA (95) */
    VK_RETURN | KBDEXT,      /* KEY_KPENTER (96) */
    VK_RCONTROL | KBDEXT,    /* KEY_RIGHTCTRL (97) */
    VK_DIVIDE | KBDEXT,      /* KEY_KPSLASH (98) */
    VK_SNAPSHOT | KBDEXT,    /* KEY_SYSRQ (99) */
    VK_RMENU | KBDEXT,       /* KEY_RIGHTALT (100) */
    0,                       /* KEY_LINEFEED (101) */
    VK_HOME | KBDEXT,        /* KEY_HOME (102) */
    VK_UP | KBDEXT,          /* KEY_UP (103) */
    VK_PRIOR | KBDEXT,       /* KEY_PAGEUP (104) */
    VK_LEFT | KBDEXT,        /* KEY_LEFT (105) */
    VK_RIGHT | KBDEXT,       /* KEY_RIGHT (106) */
    VK_END | KBDEXT,         /* KEY_END (107) */
    VK_DOWN | KBDEXT,        /* KEY_DOWN (108) */
    VK_NEXT | KBDEXT,        /* KEY_PAGEDOWN (109) */
    VK_INSERT | KBDEXT,      /* KEY_INSERT (110) */
    VK_DELETE | KBDEXT,      /* KEY_DELETE (111) */
    0,                       /* KEY_MACRO (112) */
    VK_VOLUME_MUTE | KBDEXT, /* KEY_MUTE (113) */
    VK_VOLUME_DOWN | KBDEXT, /* KEY_VOLUMEDOWN (114) */
    VK_VOLUME_UP | KBDEXT,   /* KEY_VOLUMEUP (115) */
    0,                       /* KEY_POWER (SC System Power Down) (116) */
    0,                       /* KEY_KPEQUAL (117) */
    0,                       /* KEY_KPPLUSMINUS (118) */
    VK_PAUSE | KBDEXT,       /* KEY_PAUSE (119) */
    0,                       /* KEY_SCALE (AL Compiz Scale (Expose)) (120) */
    VK_ABNT_C2,              /* KEY_KPCOMMA (121) */
    VK_HANGUL,               /* KEY_HANGEUL, KEY_HANGUEL (122) */
    VK_HANJA,                /* KEY_HANJA (123) */
    VK_OEM_8,                /* KEY_YEN (124) */
    VK_LWIN | KBDEXT,        /* KEY_LEFTMETA (125) */
    VK_RWIN | KBDEXT,        /* KEY_RIGHTMETA (126) */
    0,                       /* KEY_COMPOSE (127) */
    0,                       /* KEY_STOP (AC Stop) (128) */
    0,                       /* KEY_AGAIN (AC Properties) (129) */
    0,                       /* KEY_PROPS (AC Undo) (130) */
    0,                       /* KEY_UNDO (131) */
    0,                       /* KEY_FRONT (132) */
    0,                       /* KEY_COPY (AC Copy) (133) */
    0,                       /* KEY_OPEN (AC Open) (134) */
    0,                       /* KEY_PASTE (AC Paste) (135) */
    0,                       /* KEY_FIND (AC Search) (136) */
    0,                       /* KEY_CUT (AC Cut) (137) */
    VK_HELP,                 /* KEY_HELP (AL Integrated Help Center) (138) */
    VK_APPS | KBDEXT,        /* KEY_MENU (Menu (show menu)) (139) */
    0,                       /* KEY_CALC (AL Calculator) (140) */
    0,                       /* KEY_SETUP (141) */
    VK_SLEEP,                /* KEY_SLEEP (SC System Sleep) (142) */
    0,                       /* KEY_WAKEUP (System Wake Up) (143) */
    0,                       /* KEY_FILE (AL Local Machine Browser) (144) */
    0,                       /* KEY_SENDFILE (145) */
    0,                       /* KEY_DELETEFILE (146) */
    VK_CONVERT,              /* KEY_XFER (147) */
    VK_LAUNCH_APP1,          /* KEY_PROG1 (148) */
    VK_LAUNCH_APP2,          /* KEY_PROG2 (149) */
    0,                       /* KEY_WWW (AL Internet Browser) (150) */
    0,                       /* KEY_MSDOS (151) */
    0,                       /* KEY_COFFEE, KEY_SCREENLOCK
                              * (AL Terminal Lock/Screensaver) (152) */
    0,                             /* KEY_ROTATE_DISPLAY, KEY_DIRECTION
                                    * (Display orientation for e.g. tablets) (153) */
    0,                             /* KEY_CYCLEWINDOWS (154) */
    VK_LAUNCH_MAIL | KBDEXT,       /* KEY_MAIL (155) */
    VK_BROWSER_FAVORITES | KBDEXT, /* KEY_BOOKMARKS (AC Bookmarks) (156) */
    0,                             /* KEY_COMPUTER (157) */
    VK_BROWSER_BACK | KBDEXT,      /* KEY_BACK (AC Back) (158) */
    VK_BROWSER_FORWARD | KBDEXT,   /* KEY_FORWARD (AC Forward) (159) */
    0,                             /* KEY_CLOSECD (160) */
    0,                             /* KEY_EJECTCD (161) */
    0,                             /* KEY_EJECTCLOSECD (162) */
    VK_MEDIA_NEXT_TRACK | KBDEXT,  /* KEY_NEXTSONG (163) */
    VK_MEDIA_PLAY_PAUSE | KBDEXT,  /* KEY_PLAYPAUSE (164) */
    VK_MEDIA_PREV_TRACK | KBDEXT,  /* KEY_PREVIOUSSONG (165) */
    VK_MEDIA_STOP | KBDEXT,        /* KEY_STOPCD (166) */
    0,                             /* KEY_RECORD (167) */
    0,                             /* KEY_REWIND (168) */
    0,                             /* KEY_PHONE (Media Select Telephone) (169) */
    0,                             /* KEY_ISO (170) */
    0,                             /* KEY_CONFIG (AL Consumer Control Configuration) (171) */
    VK_BROWSER_HOME | KBDEXT,      /* KEY_HOMEPAGE (AC Home) (172) */
    VK_BROWSER_REFRESH | KBDEXT,   /* KEY_REFRESH (AC Refresh) (173) */
    0,                             /* KEY_EXIT (AC Exit) (174) */
    0,                             /* KEY_MOVE (175) */
    0,                             /* KEY_EDIT (176) */
    0,                             /* KEY_SCROLLUP (177) */
    0,                             /* KEY_SCROLLDOWN (178) */
    0,                             /* KEY_KPLEFTPAREN (179) */
    0,                             /* KEY_KPRIGHTPAREN (180) */
    0,                             /* KEY_NEW (AC New) (181) */
    0,                             /* KEY_REDO (AC Redo/Repeat) (182) */
    VK_F13,                        /* KEY_F13 (183) */
    VK_F14,                        /* KEY_F14 (184) */
    VK_F15,                        /* KEY_F15 (185) */
    VK_F16,                        /* KEY_F16 (186) */
    VK_F17,                        /* KEY_F17 (187) */
    VK_F18,                        /* KEY_F18 (188) */
    VK_F19,                        /* KEY_F19 (189) */
    VK_F20,                        /* KEY_F20 (190) */
    VK_F21,                        /* KEY_F21 (191) */
    VK_F22,                        /* KEY_F22 (192) */
    VK_F23,                        /* KEY_F23 (193) */
    VK_F24,                        /* KEY_F24 (194) */
    0,                             /* (195) */
    0,                             /* (196) */
    0,                             /* (197) */
    0,                             /* (198) */
    0,                             /* (199) */
    VK_PLAY,                       /* KEY_PLAYCD (200) */
    0,                             /* KEY_PAUSECD (201) */
    0,                             /* KEY_PROG3 (202) */
    0,                             /* KEY_PROG4 (203) */
    0,                             /* KEY_ALL_APPLICATIONS, KEY_DASHBOARD
                                    * (AC Desktop Show All Applications) (204) */
    0,                          /* KEY_SUSPEND (205) */
    0,                          /* KEY_CLOSE (AC Close) (206) */
    VK_PLAY,                    /* KEY_PLAY (207) */
    0,                          /* KEY_FASTFORWARD (208) */
    0,                          /* KEY_BASSBOOST (209) */
    VK_PRINT | KBDEXT,          /* KEY_PRINT (AC Print) (210) */
    0,                          /* KEY_HP (211) */
    0,                          /* KEY_CAMERA (212) */
    0,                          /* KEY_SOUND (213) */
    0,                          /* KEY_QUESTION (214) */
    0,                          /* KEY_EMAIL (215) */
    0,                          /* KEY_CHAT (216) */
    VK_BROWSER_SEARCH | KBDEXT, /* KEY_SEARCH (217) */
    0,                          /* KEY_CONNECT (218) */
    0,                          /* KEY_FINANCE (AL Checkbook/Finance) (219) */
    0,                          /* KEY_SPORT (220) */
    0,                          /* KEY_SHOP (221) */
    0,                          /* KEY_ALTERASE (222) */
    0,                          /* KEY_CANCEL (AC Cancel) (223) */
    0,                          /* KEY_BRIGHTNESSDOWN (224) */
    0,                          /* KEY_BRIGHTNESSUP (225) */
    0,                          /* KEY_MEDIA (226) */
    0,                          /* KEY_SWITCHVIDEOMODE
                                 * (Cycle between available video outputs
                                 *  (Monitor/LCD/TV-out/etc)) (227) */
    0, /* KEY_KBDILLUMTOGGLE (228) */
    0, /* KEY_KBDILLUMDOWN (229) */
    0, /* KEY_KBDILLUMUP (230) */
    0, /* KEY_SEND (AC Send) (231) */
    0, /* KEY_REPLY (AC Reply) (232) */
    0, /* KEY_FORWARDMAIL (AC Forward Msg) (233) */
    0, /* KEY_SAVE (AC Save) (234) */
    0, /* KEY_DOCUMENTS (235) */
    0, /* KEY_BATTERY (236) */
    0, /* KEY_BLUETOOTH (237) */
    0, /* KEY_WLAN (238) */
    0, /* KEY_UWB (239) */
    0, /* KEY_UNKNOWN (240) */
    0, /* KEY_VIDEO_NEXT (drive next video source) (241) */
    0, /* KEY_VIDEO_PREV (drive previous video source) (242) */
    0, /* KEY_BRIGHTNESS_CYCLE
        * (brightness up, after max is min) (243) */
    0, /* KEY_BRIGHTNESS_AUTO, KEY_BRIGHTNESS_ZERO
        * (Set Auto Brightness: manual brightness control is off,
        *  rely on ambient) (244) */
    0, /* KEY_DISPLAY_OFF (display device to off state) (245) */
    0, /* KEY_WWAN, KEY_WIMAX
        * (Wireless WAN (LTE, UMTS, GSM, etc.)) (246) */
    0, /* KEY_RFKILL (Key that controls all radios) (247) */
    0, /* KEY_MICMUTE (Mute / unmute the microphone) (248) */
    0, /* (249) */
    0, /* (250) */
    0, /* (251) */
    0, /* (252) */
    0, /* (253) */
    0, /* (254) */
    0, /* (255) */
];

pub(crate) fn get_vkcode_from_apple_keycode(key_code: u16) -> u16 {
    if key_code < 0xFF {
        KEY_CODE_TO_VKCODE_APPLE[key_code as usize]
    } else {
        VK_NONE
    }
}
pub(crate) fn get_vkcode_from_linux_keycode(key_code: u16) -> u16 {
    if key_code < 0xFF {
        KEYCODE_TO_VKCODE_EVDEV[key_code as usize]
    } else {
        VK_NONE
    }
}
