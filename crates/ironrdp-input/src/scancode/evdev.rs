use core::fmt::Display;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct EvdevKey(u16);

impl EvdevKey {
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    pub const fn from_u16(code: u16) -> Self {
        Self(code)
    }
}

impl Display for EvdevKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#06x}", self.0)
    }
}

// From evdev-scancodes.h
// https://github.com/xkbcommon/libxkbcommon/blob/c0065c95a479c7111417a6547d26594a5e31378b/test/evdev-scancodes.h

impl EvdevKey {
    pub const KEY_RESERVED: Self = Self(0);
    pub const KEY_ESC: Self = Self(1);
    pub const KEY_1: Self = Self(2);
    pub const KEY_2: Self = Self(3);
    pub const KEY_3: Self = Self(4);
    pub const KEY_4: Self = Self(5);
    pub const KEY_5: Self = Self(6);
    pub const KEY_6: Self = Self(7);
    pub const KEY_7: Self = Self(8);
    pub const KEY_8: Self = Self(9);
    pub const KEY_9: Self = Self(10);
    pub const KEY_0: Self = Self(11);
    pub const KEY_MINUS: Self = Self(12);
    pub const KEY_EQUAL: Self = Self(13);
    pub const KEY_BACKSPACE: Self = Self(14);
    pub const KEY_TAB: Self = Self(15);
    pub const KEY_Q: Self = Self(16);
    pub const KEY_W: Self = Self(17);
    pub const KEY_E: Self = Self(18);
    pub const KEY_R: Self = Self(19);
    pub const KEY_T: Self = Self(20);
    pub const KEY_Y: Self = Self(21);
    pub const KEY_U: Self = Self(22);
    pub const KEY_I: Self = Self(23);
    pub const KEY_O: Self = Self(24);
    pub const KEY_P: Self = Self(25);
    pub const KEY_LEFTBRACE: Self = Self(26);
    pub const KEY_RIGHTBRACE: Self = Self(27);
    pub const KEY_ENTER: Self = Self(28);
    pub const KEY_LEFTCTRL: Self = Self(29);
    pub const KEY_A: Self = Self(30);
    pub const KEY_S: Self = Self(31);
    pub const KEY_D: Self = Self(32);
    pub const KEY_F: Self = Self(33);
    pub const KEY_G: Self = Self(34);
    pub const KEY_H: Self = Self(35);
    pub const KEY_J: Self = Self(36);
    pub const KEY_K: Self = Self(37);
    pub const KEY_L: Self = Self(38);
    pub const KEY_SEMICOLON: Self = Self(39);
    pub const KEY_APOSTROPHE: Self = Self(40);
    pub const KEY_GRAVE: Self = Self(41);
    pub const KEY_LEFTSHIFT: Self = Self(42);
    pub const KEY_BACKSLASH: Self = Self(43);
    pub const KEY_Z: Self = Self(44);
    pub const KEY_X: Self = Self(45);
    pub const KEY_C: Self = Self(46);
    pub const KEY_V: Self = Self(47);
    pub const KEY_B: Self = Self(48);
    pub const KEY_N: Self = Self(49);
    pub const KEY_M: Self = Self(50);
    pub const KEY_COMMA: Self = Self(51);
    pub const KEY_DOT: Self = Self(52);
    pub const KEY_SLASH: Self = Self(53);
    pub const KEY_RIGHTSHIFT: Self = Self(54);
    pub const KEY_KPASTERISK: Self = Self(55);
    pub const KEY_LEFTALT: Self = Self(56);
    pub const KEY_SPACE: Self = Self(57);
    pub const KEY_CAPSLOCK: Self = Self(58);
    pub const KEY_F1: Self = Self(59);
    pub const KEY_F2: Self = Self(60);
    pub const KEY_F3: Self = Self(61);
    pub const KEY_F4: Self = Self(62);
    pub const KEY_F5: Self = Self(63);
    pub const KEY_F6: Self = Self(64);
    pub const KEY_F7: Self = Self(65);
    pub const KEY_F8: Self = Self(66);
    pub const KEY_F9: Self = Self(67);
    pub const KEY_F10: Self = Self(68);
    pub const KEY_NUMLOCK: Self = Self(69);
    pub const KEY_SCROLLLOCK: Self = Self(70);
    pub const KEY_KP7: Self = Self(71);
    pub const KEY_KP8: Self = Self(72);
    pub const KEY_KP9: Self = Self(73);
    pub const KEY_KPMINUS: Self = Self(74);
    pub const KEY_KP4: Self = Self(75);
    pub const KEY_KP5: Self = Self(76);
    pub const KEY_KP6: Self = Self(77);
    pub const KEY_KPPLUS: Self = Self(78);
    pub const KEY_KP1: Self = Self(79);
    pub const KEY_KP2: Self = Self(80);
    pub const KEY_KP3: Self = Self(81);
    pub const KEY_KP0: Self = Self(82);
    pub const KEY_KPDOT: Self = Self(83);
    pub const KEY_ZENKAKUHANKAKU: Self = Self(85);
    pub const KEY_102ND: Self = Self(86);
    pub const KEY_F11: Self = Self(87);
    pub const KEY_F12: Self = Self(88);
    pub const KEY_RO: Self = Self(89);
    pub const KEY_KATAKANA: Self = Self(90);
    pub const KEY_HIRAGANA: Self = Self(91);
    pub const KEY_HENKAN: Self = Self(92);
    pub const KEY_KATAKANAHIRAGANA: Self = Self(93);
    pub const KEY_MUHENKAN: Self = Self(94);
    pub const KEY_KPJPCOMMA: Self = Self(95);
    pub const KEY_KPENTER: Self = Self(96);
    pub const KEY_RIGHTCTRL: Self = Self(97);
    pub const KEY_KPSLASH: Self = Self(98);
    pub const KEY_SYSRQ: Self = Self(99);
    pub const KEY_RIGHTALT: Self = Self(100);
    pub const KEY_LINEFEED: Self = Self(101);
    pub const KEY_HOME: Self = Self(102);
    pub const KEY_UP: Self = Self(103);
    pub const KEY_PAGEUP: Self = Self(104);
    pub const KEY_LEFT: Self = Self(105);
    pub const KEY_RIGHT: Self = Self(106);
    pub const KEY_END: Self = Self(107);
    pub const KEY_DOWN: Self = Self(108);
    pub const KEY_PAGEDOWN: Self = Self(109);
    pub const KEY_INSERT: Self = Self(110);
    pub const KEY_DELETE: Self = Self(111);
    pub const KEY_MACRO: Self = Self(112);
    pub const KEY_MUTE: Self = Self(113);
    pub const KEY_VOLUMEDOWN: Self = Self(114);
    pub const KEY_VOLUMEUP: Self = Self(115);
    /// SC System Power Down
    pub const KEY_POWER: Self = Self(116);
    pub const KEY_KPEQUAL: Self = Self(117);
    pub const KEY_KPPLUSMINUS: Self = Self(118);
    pub const KEY_PAUSE: Self = Self(119);
    /// AL Compiz Scale (Expose)
    pub const KEY_SCALE: Self = Self(120);
    pub const KEY_KPCOMMA: Self = Self(121);
    pub const KEY_HANGEUL: Self = Self(122);
    pub const KEY_HANJA: Self = Self(123);
    pub const KEY_YEN: Self = Self(124);
    pub const KEY_LEFTMETA: Self = Self(125);
    pub const KEY_RIGHTMETA: Self = Self(126);
    pub const KEY_COMPOSE: Self = Self(127);
    /// AC Stop
    pub const KEY_STOP: Self = Self(128);
    pub const KEY_AGAIN: Self = Self(129);
    /// AC Properties
    pub const KEY_PROPS: Self = Self(130);
    /// AC Undo
    pub const KEY_UNDO: Self = Self(131);
    pub const KEY_FRONT: Self = Self(132);
    /// AC Copy
    pub const KEY_COPY: Self = Self(133);
    /// AC Open
    pub const KEY_OPEN: Self = Self(134);
    /// AC Paste
    pub const KEY_PASTE: Self = Self(135);
    /// AC Search
    pub const KEY_FIND: Self = Self(136);
    /// AC Cut
    pub const KEY_CUT: Self = Self(137);
    /// AL Integrated Help Center
    pub const KEY_HELP: Self = Self(138);
    /// Menu (show menu)
    pub const KEY_MENU: Self = Self(139);
    /// AL Calculator
    pub const KEY_CALC: Self = Self(140);
    pub const KEY_SETUP: Self = Self(141);
    /// SC System Sleep
    pub const KEY_SLEEP: Self = Self(142);
    /// System Wake Up
    pub const KEY_WAKEUP: Self = Self(143);
    /// AL Local Machine Browser
    pub const KEY_FILE: Self = Self(144);
    pub const KEY_SENDFILE: Self = Self(145);
    pub const KEY_DELETEFILE: Self = Self(146);
    pub const KEY_XFER: Self = Self(147);
    pub const KEY_PROG1: Self = Self(148);
    pub const KEY_PROG2: Self = Self(149);
    /// AL Internet Browser
    pub const KEY_WWW: Self = Self(150);
    pub const KEY_MSDOS: Self = Self(151);
    /// AL Terminal Lock/Screensaver
    pub const KEY_COFFEE: Self = Self(152);
    pub const KEY_DIRECTION: Self = Self(153);
    pub const KEY_ROTATE_DISPLAY: Self = Self(153);
    pub const KEY_CYCLEWINDOWS: Self = Self(154);
    pub const KEY_MAIL: Self = Self(155);
    /// AC Bookmarks
    pub const KEY_BOOKMARKS: Self = Self(156);
    pub const KEY_COMPUTER: Self = Self(157);
    /// AC Back
    pub const KEY_BACK: Self = Self(158);
    /// AC Forward
    pub const KEY_FORWARD: Self = Self(159);
    pub const KEY_CLOSECD: Self = Self(160);
    pub const KEY_EJECTCD: Self = Self(161);
    pub const KEY_EJECTCLOSECD: Self = Self(162);
    pub const KEY_NEXTSONG: Self = Self(163);
    pub const KEY_PLAYPAUSE: Self = Self(164);
    pub const KEY_PREVIOUSSONG: Self = Self(165);
    pub const KEY_STOPCD: Self = Self(166);
    pub const KEY_RECORD: Self = Self(167);
    pub const KEY_REWIND: Self = Self(168);
    /// Media Select Telephone
    pub const KEY_PHONE: Self = Self(169);
    pub const KEY_ISO: Self = Self(170);
    /// AL Consumer Control Configuration
    pub const KEY_CONFIG: Self = Self(171);
    /// AC Home
    pub const KEY_HOMEPAGE: Self = Self(172);
    /// AC Refresh
    pub const KEY_REFRESH: Self = Self(173);
    /// AC Exit
    pub const KEY_EXIT: Self = Self(174);
    pub const KEY_MOVE: Self = Self(175);
    pub const KEY_EDIT: Self = Self(176);
    pub const KEY_SCROLLUP: Self = Self(177);
    pub const KEY_SCROLLDOWN: Self = Self(178);
    pub const KEY_KPLEFTPAREN: Self = Self(179);
    pub const KEY_KPRIGHTPAREN: Self = Self(180);
    /// AC New
    pub const KEY_NEW: Self = Self(181);
    /// AC Redo/Repeat
    pub const KEY_REDO: Self = Self(182);
    pub const KEY_F13: Self = Self(183);
    pub const KEY_F14: Self = Self(184);
    pub const KEY_F15: Self = Self(185);
    pub const KEY_F16: Self = Self(186);
    pub const KEY_F17: Self = Self(187);
    pub const KEY_F18: Self = Self(188);
    pub const KEY_F19: Self = Self(189);
    pub const KEY_F20: Self = Self(190);
    pub const KEY_F21: Self = Self(191);
    pub const KEY_F22: Self = Self(192);
    pub const KEY_F23: Self = Self(193);
    pub const KEY_F24: Self = Self(194);
    pub const KEY_PLAYCD: Self = Self(200);
    pub const KEY_PAUSECD: Self = Self(201);
    pub const KEY_PROG3: Self = Self(202);
    pub const KEY_PROG4: Self = Self(203);
    /// AL Dashboard
    pub const KEY_DASHBOARD: Self = Self(204);
    pub const KEY_SUSPEND: Self = Self(205);
    /// AC Close
    pub const KEY_CLOSE: Self = Self(206);
    pub const KEY_PLAY: Self = Self(207);
    pub const KEY_FASTFORWARD: Self = Self(208);
    pub const KEY_BASSBOOST: Self = Self(209);
    /// AC Print
    pub const KEY_PRINT: Self = Self(210);
    pub const KEY_HP: Self = Self(211);
    pub const KEY_CAMERA: Self = Self(212);
    pub const KEY_SOUND: Self = Self(213);
    pub const KEY_QUESTION: Self = Self(214);
    pub const KEY_EMAIL: Self = Self(215);
    pub const KEY_CHAT: Self = Self(216);
    pub const KEY_SEARCH: Self = Self(217);
    pub const KEY_CONNECT: Self = Self(218);
    pub const KEY_FINANCE: Self = Self(219);
    pub const KEY_SPORT: Self = Self(220);
    pub const KEY_SHOP: Self = Self(221);
    pub const KEY_ALTERASE: Self = Self(222);
    pub const KEY_CANCEL: Self = Self(223);
    pub const KEY_BRIGHTNESSDOWN: Self = Self(224);
    pub const KEY_BRIGHTNESSUP: Self = Self(225);
    pub const KEY_MEDIA: Self = Self(226);
    pub const KEY_SWITCHVIDEOMODE: Self = Self(227);
    pub const KEY_KBDILLUMTOGGLE: Self = Self(228);
    pub const KEY_KBDILLUMDOWN: Self = Self(229);
    pub const KEY_KBDILLUMUP: Self = Self(230);
    pub const KEY_SEND: Self = Self(231);
    pub const KEY_REPLY: Self = Self(232);
    pub const KEY_FORWARDMAIL: Self = Self(233);
    pub const KEY_SAVE: Self = Self(234);
    pub const KEY_DOCUMENTS: Self = Self(235);
    pub const KEY_BATTERY: Self = Self(236);
    pub const KEY_BLUETOOTH: Self = Self(237);
    pub const KEY_WLAN: Self = Self(238);
    pub const KEY_UWB: Self = Self(239);
    pub const KEY_UNKNOWN: Self = Self(240);
    pub const KEY_VIDEO_NEXT: Self = Self(241);
    pub const KEY_VIDEO_PREV: Self = Self(242);
    pub const KEY_BRIGHTNESS_CYCLE: Self = Self(243);
    pub const KEY_BRIGHTNESS_AUTO: Self = Self(244);
    pub const KEY_DISPLAY_OFF: Self = Self(245);
    pub const KEY_WWAN: Self = Self(246);
    pub const KEY_RFKILL: Self = Self(247);
    pub const KEY_MICMUTE: Self = Self(248);
    pub const BTN_0: Self = Self(0x100);
    pub const BTN_1: Self = Self(0x101);
    pub const BTN_2: Self = Self(0x102);
    pub const BTN_3: Self = Self(0x103);
    pub const BTN_4: Self = Self(0x104);
    pub const BTN_5: Self = Self(0x105);
    pub const BTN_6: Self = Self(0x106);
    pub const BTN_7: Self = Self(0x107);
    pub const BTN_8: Self = Self(0x108);
    pub const BTN_9: Self = Self(0x109);
    pub const BTN_LEFT: Self = Self(0x110);
    pub const BTN_RIGHT: Self = Self(0x111);
    pub const BTN_MIDDLE: Self = Self(0x112);
    pub const BTN_SIDE: Self = Self(0x113);
    pub const BTN_EXTRA: Self = Self(0x114);
    pub const BTN_FORWARD: Self = Self(0x115);
    pub const BTN_BACK: Self = Self(0x116);
    pub const BTN_TASK: Self = Self(0x117);
    pub const BTN_TRIGGER: Self = Self(0x120);
    pub const BTN_THUMB: Self = Self(0x121);
    pub const BTN_THUMB2: Self = Self(0x122);
    pub const BTN_TOP: Self = Self(0x123);
    pub const BTN_TOP2: Self = Self(0x124);
    pub const BTN_PINKIE: Self = Self(0x125);
    pub const BTN_BASE: Self = Self(0x126);
    pub const BTN_BASE2: Self = Self(0x127);
    pub const BTN_BASE3: Self = Self(0x128);
    pub const BTN_BASE4: Self = Self(0x129);
    pub const BTN_BASE5: Self = Self(0x12a);
    pub const BTN_BASE6: Self = Self(0x12b);
    pub const BTN_DEAD: Self = Self(0x12f);
    pub const BTN_SOUTH: Self = Self(0x130);
    pub const BTN_EAST: Self = Self(0x131);
    pub const BTN_C: Self = Self(0x132);
    pub const BTN_NORTH: Self = Self(0x133);
    pub const BTN_WEST: Self = Self(0x134);
    pub const BTN_Z: Self = Self(0x135);
    pub const BTN_TL: Self = Self(0x136);
    pub const BTN_TR: Self = Self(0x137);
    pub const BTN_TL2: Self = Self(0x138);
    pub const BTN_TR2: Self = Self(0x139);
    pub const BTN_SELECT: Self = Self(0x13a);
    pub const BTN_START: Self = Self(0x13b);
    pub const BTN_MODE: Self = Self(0x13c);
    pub const BTN_THUMBL: Self = Self(0x13d);
    pub const BTN_THUMBR: Self = Self(0x13e);
    pub const BTN_TOOL_PEN: Self = Self(0x140);
    pub const BTN_TOOL_RUBBER: Self = Self(0x141);
    pub const BTN_TOOL_BRUSH: Self = Self(0x142);
    pub const BTN_TOOL_PENCIL: Self = Self(0x143);
    pub const BTN_TOOL_AIRBRUSH: Self = Self(0x144);
    pub const BTN_TOOL_FINGER: Self = Self(0x145);
    pub const BTN_TOOL_MOUSE: Self = Self(0x146);
    pub const BTN_TOOL_LENS: Self = Self(0x147);
    /// Five fingers on trackpad
    pub const BTN_TOOL_QUINTTAP: Self = Self(0x148);
    pub const BTN_TOUCH: Self = Self(0x14a);
    pub const BTN_STYLUS: Self = Self(0x14b);
    pub const BTN_STYLUS2: Self = Self(0x14c);
    pub const BTN_TOOL_DOUBLETAP: Self = Self(0x14d);
    pub const BTN_TOOL_TRIPLETAP: Self = Self(0x14e);
    /// Four fingers on trackpad
    pub const BTN_TOOL_QUADTAP: Self = Self(0x14f);
    pub const BTN_GEAR_DOWN: Self = Self(0x150);
    pub const BTN_GEAR_UP: Self = Self(0x151);
    pub const KEY_OK: Self = Self(0x160);
    pub const KEY_SELECT: Self = Self(0x161);
    pub const KEY_GOTO: Self = Self(0x162);
    pub const KEY_CLEAR: Self = Self(0x163);
    pub const KEY_POWER2: Self = Self(0x164);
    pub const KEY_OPTION: Self = Self(0x165);
    /// AL OEM Features/Tips/Tutorial
    pub const KEY_INFO: Self = Self(0x166);
    pub const KEY_TIME: Self = Self(0x167);
    pub const KEY_VENDOR: Self = Self(0x168);
    pub const KEY_ARCHIVE: Self = Self(0x169);
    /// Media Select Program Guide
    pub const KEY_PROGRAM: Self = Self(0x16a);
    pub const KEY_CHANNEL: Self = Self(0x16b);
    pub const KEY_FAVORITES: Self = Self(0x16c);
    pub const KEY_EPG: Self = Self(0x16d);
    /// Media Select Home
    pub const KEY_PVR: Self = Self(0x16e);
    pub const KEY_MHP: Self = Self(0x16f);
    pub const KEY_LANGUAGE: Self = Self(0x170);
    pub const KEY_TITLE: Self = Self(0x171);
    pub const KEY_SUBTITLE: Self = Self(0x172);
    pub const KEY_ANGLE: Self = Self(0x173);
    pub const KEY_ZOOM: Self = Self(0x174);
    pub const KEY_FULL_SCREEN: Self = Self(0x174);
    pub const KEY_MODE: Self = Self(0x175);
    pub const KEY_KEYBOARD: Self = Self(0x176);
    pub const KEY_SCREEN: Self = Self(0x177);
    /// Media Select Computer
    pub const KEY_PC: Self = Self(0x178);
    /// Media Select TV
    pub const KEY_TV: Self = Self(0x179);
    /// Media Select Cable
    pub const KEY_TV2: Self = Self(0x17a);
    /// Media Select VCR
    pub const KEY_VCR: Self = Self(0x17b);
    /// VCR Plus
    pub const KEY_VCR2: Self = Self(0x17c);
    /// Media Select Satellite
    pub const KEY_SAT: Self = Self(0x17d);
    pub const KEY_SAT2: Self = Self(0x17e);
    /// Media Select CD
    pub const KEY_CD: Self = Self(0x17f);
    /// Media Select Tape
    pub const KEY_TAPE: Self = Self(0x180);
    pub const KEY_RADIO: Self = Self(0x181);
    /// Media Select Tuner
    pub const KEY_TUNER: Self = Self(0x182);
    pub const KEY_PLAYER: Self = Self(0x183);
    pub const KEY_TEXT: Self = Self(0x184);
    /// Media Select DVD
    pub const KEY_DVD: Self = Self(0x185);
    pub const KEY_AUX: Self = Self(0x186);
    pub const KEY_MP3: Self = Self(0x187);
    /// AL Audio Browser
    pub const KEY_AUDIO: Self = Self(0x188);
    /// AL Movie Browser
    pub const KEY_VIDEO: Self = Self(0x189);
    pub const KEY_DIRECTORY: Self = Self(0x18a);
    pub const KEY_LIST: Self = Self(0x18b);
    /// Media Select Messages
    pub const KEY_MEMO: Self = Self(0x18c);
    pub const KEY_CALENDAR: Self = Self(0x18d);
    pub const KEY_RED: Self = Self(0x18e);
    pub const KEY_GREEN: Self = Self(0x18f);
    pub const KEY_YELLOW: Self = Self(0x190);
    pub const KEY_BLUE: Self = Self(0x191);
    /// Channel Increment
    pub const KEY_CHANNELUP: Self = Self(0x192);
    /// Channel Decrement
    pub const KEY_CHANNELDOWN: Self = Self(0x193);
    pub const KEY_FIRST: Self = Self(0x194);
    /// Recall Last
    pub const KEY_LAST: Self = Self(0x195);
    pub const KEY_AB: Self = Self(0x196);
    pub const KEY_NEXT: Self = Self(0x197);
    pub const KEY_RESTART: Self = Self(0x198);
    pub const KEY_SLOW: Self = Self(0x199);
    pub const KEY_SHUFFLE: Self = Self(0x19a);
    pub const KEY_BREAK: Self = Self(0x19b);
    pub const KEY_PREVIOUS: Self = Self(0x19c);
    pub const KEY_DIGITS: Self = Self(0x19d);
    pub const KEY_TEEN: Self = Self(0x19e);
    pub const KEY_TWEN: Self = Self(0x19f);
    /// Media Select Video Phone
    pub const KEY_VIDEOPHONE: Self = Self(0x1a0);
    /// Media Select Games
    pub const KEY_GAMES: Self = Self(0x1a1);
    /// AC Zoom In
    pub const KEY_ZOOMIN: Self = Self(0x1a2);
    /// AC Zoom Out
    pub const KEY_ZOOMOUT: Self = Self(0x1a3);
    /// AC Zoom
    pub const KEY_ZOOMRESET: Self = Self(0x1a4);
    /// AL Word Processor
    pub const KEY_WORDPROCESSOR: Self = Self(0x1a5);
    /// AL Text Editor
    pub const KEY_EDITOR: Self = Self(0x1a6);
    /// AL Spreadsheet
    pub const KEY_SPREADSHEET: Self = Self(0x1a7);
    /// AL Graphics Editor
    pub const KEY_GRAPHICSEDITOR: Self = Self(0x1a8);
    /// AL Presentation App
    pub const KEY_PRESENTATION: Self = Self(0x1a9);
    /// AL Database App
    pub const KEY_DATABASE: Self = Self(0x1aa);
    /// AL Newsreader
    pub const KEY_NEWS: Self = Self(0x1ab);
    /// AL Voicemail
    pub const KEY_VOICEMAIL: Self = Self(0x1ac);
    /// AL Contacts/Address Book
    pub const KEY_ADDRESSBOOK: Self = Self(0x1ad);
    /// AL Instant Messaging
    pub const KEY_MESSENGER: Self = Self(0x1ae);
    /// Turn display (LCD) on and off
    pub const KEY_DISPLAYTOGGLE: Self = Self(0x1af);
    /// AL Spell Check
    pub const KEY_SPELLCHECK: Self = Self(0x1b0);
    /// AL Logoff
    pub const KEY_LOGOFF: Self = Self(0x1b1);
    pub const KEY_DOLLAR: Self = Self(0x1b2);
    pub const KEY_EURO: Self = Self(0x1b3);
    /// Consumer - transport controls
    pub const KEY_FRAMEBACK: Self = Self(0x1b4);
    pub const KEY_FRAMEFORWARD: Self = Self(0x1b5);
    /// GenDesc - system context menu
    pub const KEY_CONTEXT_MENU: Self = Self(0x1b6);
    /// Consumer - transport control
    pub const KEY_MEDIA_REPEAT: Self = Self(0x1b7);
    /// 10 channels up (10+)
    pub const KEY_10CHANNELSUP: Self = Self(0x1b8);
    /// 10 channels down (10-)
    pub const KEY_10CHANNELSDOWN: Self = Self(0x1b9);
    /// AL Image Browser
    pub const KEY_IMAGES: Self = Self(0x1ba);
    pub const KEY_DEL_EOL: Self = Self(0x1c0);
    pub const KEY_DEL_EOS: Self = Self(0x1c1);
    pub const KEY_INS_LINE: Self = Self(0x1c2);
    pub const KEY_DEL_LINE: Self = Self(0x1c3);
    pub const KEY_FN: Self = Self(0x1d0);
    pub const KEY_FN_ESC: Self = Self(0x1d1);
    pub const KEY_FN_F1: Self = Self(0x1d2);
    pub const KEY_FN_F2: Self = Self(0x1d3);
    pub const KEY_FN_F3: Self = Self(0x1d4);
    pub const KEY_FN_F4: Self = Self(0x1d5);
    pub const KEY_FN_F5: Self = Self(0x1d6);
    pub const KEY_FN_F6: Self = Self(0x1d7);
    pub const KEY_FN_F7: Self = Self(0x1d8);
    pub const KEY_FN_F8: Self = Self(0x1d9);
    pub const KEY_FN_F9: Self = Self(0x1da);
    pub const KEY_FN_F10: Self = Self(0x1db);
    pub const KEY_FN_F11: Self = Self(0x1dc);
    pub const KEY_FN_F12: Self = Self(0x1dd);
    pub const KEY_FN_1: Self = Self(0x1de);
    pub const KEY_FN_2: Self = Self(0x1df);
    pub const KEY_FN_D: Self = Self(0x1e0);
    pub const KEY_FN_E: Self = Self(0x1e1);
    pub const KEY_FN_F: Self = Self(0x1e2);
    pub const KEY_FN_S: Self = Self(0x1e3);
    pub const KEY_FN_B: Self = Self(0x1e4);
    pub const KEY_BRL_DOT1: Self = Self(0x1f1);
    pub const KEY_BRL_DOT2: Self = Self(0x1f2);
    pub const KEY_BRL_DOT3: Self = Self(0x1f3);
    pub const KEY_BRL_DOT4: Self = Self(0x1f4);
    pub const KEY_BRL_DOT5: Self = Self(0x1f5);
    pub const KEY_BRL_DOT6: Self = Self(0x1f6);
    pub const KEY_BRL_DOT7: Self = Self(0x1f7);
    pub const KEY_BRL_DOT8: Self = Self(0x1f8);
    pub const KEY_BRL_DOT9: Self = Self(0x1f9);
    pub const KEY_BRL_DOT10: Self = Self(0x1fa);
    /// used by phones, remote controls,
    pub const KEY_NUMERIC_0: Self = Self(0x200);
    /// and other keypads
    pub const KEY_NUMERIC_1: Self = Self(0x201);
    pub const KEY_NUMERIC_2: Self = Self(0x202);
    pub const KEY_NUMERIC_3: Self = Self(0x203);
    pub const KEY_NUMERIC_4: Self = Self(0x204);
    pub const KEY_NUMERIC_5: Self = Self(0x205);
    pub const KEY_NUMERIC_6: Self = Self(0x206);
    pub const KEY_NUMERIC_7: Self = Self(0x207);
    pub const KEY_NUMERIC_8: Self = Self(0x208);
    pub const KEY_NUMERIC_9: Self = Self(0x209);
    pub const KEY_NUMERIC_STAR: Self = Self(0x20a);
    pub const KEY_NUMERIC_POUND: Self = Self(0x20b);
    /// Phone key A - HUT Telephony 0xb9
    pub const KEY_NUMERIC_A: Self = Self(0x20c);
    pub const KEY_NUMERIC_B: Self = Self(0x20d);
    pub const KEY_NUMERIC_C: Self = Self(0x20e);
    pub const KEY_NUMERIC_D: Self = Self(0x20f);
    pub const KEY_CAMERA_FOCUS: Self = Self(0x210);
    /// WiFi Protected Setup key
    pub const KEY_WPS_BUTTON: Self = Self(0x211);
    /// Request switch touchpad on or off
    pub const KEY_TOUCHPAD_TOGGLE: Self = Self(0x212);
    pub const KEY_TOUCHPAD_ON: Self = Self(0x213);
    pub const KEY_TOUCHPAD_OFF: Self = Self(0x214);
    pub const KEY_CAMERA_ZOOMIN: Self = Self(0x215);
    pub const KEY_CAMERA_ZOOMOUT: Self = Self(0x216);
    pub const KEY_CAMERA_UP: Self = Self(0x217);
    pub const KEY_CAMERA_DOWN: Self = Self(0x218);
    pub const KEY_CAMERA_LEFT: Self = Self(0x219);
    pub const KEY_CAMERA_RIGHT: Self = Self(0x21a);
    pub const KEY_ATTENDANT_ON: Self = Self(0x21b);
    pub const KEY_ATTENDANT_OFF: Self = Self(0x21c);
    /// Attendant call on or off
    pub const KEY_ATTENDANT_TOGGLE: Self = Self(0x21d);
    /// Reading light on or off
    pub const KEY_LIGHTS_TOGGLE: Self = Self(0x21e);
    pub const BTN_DPAD_UP: Self = Self(0x220);
    pub const BTN_DPAD_DOWN: Self = Self(0x221);
    pub const BTN_DPAD_LEFT: Self = Self(0x222);
    pub const BTN_DPAD_RIGHT: Self = Self(0x223);
    /// Ambient light sensor
    pub const KEY_ALS_TOGGLE: Self = Self(0x230);
    /// AL Button Configuration
    pub const KEY_BUTTONCONFIG: Self = Self(0x240);
    /// AL Task/Project Manager
    pub const KEY_TASKMANAGER: Self = Self(0x241);
    /// AL Log/Journal/Timecard
    pub const KEY_JOURNAL: Self = Self(0x242);
    /// AL Control Panel
    pub const KEY_CONTROLPANEL: Self = Self(0x243);
    /// AL Select Task/Application
    pub const KEY_APPSELECT: Self = Self(0x244);
    /// AL Screen Saver
    pub const KEY_SCREENSAVER: Self = Self(0x245);
    /// Listening Voice Command
    pub const KEY_VOICECOMMAND: Self = Self(0x246);
    pub const KEY_ASSISTANT: Self = Self(0x247);
    pub const KEY_KBD_LAYOUT_NEXT: Self = Self(0x248);
    /// Set Brightness to Minimum
    pub const KEY_BRIGHTNESS_MIN: Self = Self(0x250);
    /// Set Brightness to Maximum
    pub const KEY_BRIGHTNESS_MAX: Self = Self(0x251);
    pub const KEY_KBDINPUTASSIST_PREV: Self = Self(0x260);
    pub const KEY_KBDINPUTASSIST_NEXT: Self = Self(0x261);
    pub const KEY_KBDINPUTASSIST_PREVGROUP: Self = Self(0x262);
    pub const KEY_KBDINPUTASSIST_NEXTGROUP: Self = Self(0x263);
    pub const KEY_KBDINPUTASSIST_ACCEPT: Self = Self(0x264);
    pub const KEY_KBDINPUTASSIST_CANCEL: Self = Self(0x265);
    pub const KEY_RIGHT_UP: Self = Self(0x266);
    pub const KEY_RIGHT_DOWN: Self = Self(0x267);
    pub const KEY_LEFT_UP: Self = Self(0x268);
    pub const KEY_LEFT_DOWN: Self = Self(0x269);
    pub const KEY_ROOT_MENU: Self = Self(0x26a);
    pub const KEY_MEDIA_TOP_MENU: Self = Self(0x26b);
    pub const KEY_NUMERIC_11: Self = Self(0x26c);
    pub const KEY_NUMERIC_12: Self = Self(0x26d);
    pub const KEY_AUDIO_DESC: Self = Self(0x26e);
    pub const KEY_3D_MODE: Self = Self(0x26f);
    pub const KEY_NEXT_FAVORITE: Self = Self(0x270);
    pub const KEY_STOP_RECORD: Self = Self(0x271);
    pub const KEY_PAUSE_RECORD: Self = Self(0x272);
    /// Video on Demand
    pub const KEY_VOD: Self = Self(0x273);
    pub const KEY_UNMUTE: Self = Self(0x274);
    pub const KEY_FASTREVERSE: Self = Self(0x275);
    pub const KEY_SLOWREVERSE: Self = Self(0x276);
    pub const KEY_DATA: Self = Self(0x277);
    pub const KEY_ONSCREEN_KEYBOARD: Self = Self(0x278);
    pub const KEY_PRIVACY_SCREEN_TOGGLE: Self = Self(0x279);
    pub const KEY_SELECTIVE_SCREENSHOT: Self = Self(0x27a);
    pub const BTN_TRIGGER_HAPPY1: Self = Self(0x2c0);
    pub const BTN_TRIGGER_HAPPY2: Self = Self(0x2c1);
    pub const BTN_TRIGGER_HAPPY3: Self = Self(0x2c2);
    pub const BTN_TRIGGER_HAPPY4: Self = Self(0x2c3);
    pub const BTN_TRIGGER_HAPPY5: Self = Self(0x2c4);
    pub const BTN_TRIGGER_HAPPY6: Self = Self(0x2c5);
    pub const BTN_TRIGGER_HAPPY7: Self = Self(0x2c6);
    pub const BTN_TRIGGER_HAPPY8: Self = Self(0x2c7);
    pub const BTN_TRIGGER_HAPPY9: Self = Self(0x2c8);
    pub const BTN_TRIGGER_HAPPY10: Self = Self(0x2c9);
    pub const BTN_TRIGGER_HAPPY11: Self = Self(0x2ca);
    pub const BTN_TRIGGER_HAPPY12: Self = Self(0x2cb);
    pub const BTN_TRIGGER_HAPPY13: Self = Self(0x2cc);
    pub const BTN_TRIGGER_HAPPY14: Self = Self(0x2cd);
    pub const BTN_TRIGGER_HAPPY15: Self = Self(0x2ce);
    pub const BTN_TRIGGER_HAPPY16: Self = Self(0x2cf);
    pub const BTN_TRIGGER_HAPPY17: Self = Self(0x2d0);
    pub const BTN_TRIGGER_HAPPY18: Self = Self(0x2d1);
    pub const BTN_TRIGGER_HAPPY19: Self = Self(0x2d2);
    pub const BTN_TRIGGER_HAPPY20: Self = Self(0x2d3);
    pub const BTN_TRIGGER_HAPPY21: Self = Self(0x2d4);
    pub const BTN_TRIGGER_HAPPY22: Self = Self(0x2d5);
    pub const BTN_TRIGGER_HAPPY23: Self = Self(0x2d6);
    pub const BTN_TRIGGER_HAPPY24: Self = Self(0x2d7);
    pub const BTN_TRIGGER_HAPPY25: Self = Self(0x2d8);
    pub const BTN_TRIGGER_HAPPY26: Self = Self(0x2d9);
    pub const BTN_TRIGGER_HAPPY27: Self = Self(0x2da);
    pub const BTN_TRIGGER_HAPPY28: Self = Self(0x2db);
    pub const BTN_TRIGGER_HAPPY29: Self = Self(0x2dc);
    pub const BTN_TRIGGER_HAPPY30: Self = Self(0x2dd);
    pub const BTN_TRIGGER_HAPPY31: Self = Self(0x2de);
    pub const BTN_TRIGGER_HAPPY32: Self = Self(0x2df);
    pub const BTN_TRIGGER_HAPPY33: Self = Self(0x2e0);
    pub const BTN_TRIGGER_HAPPY34: Self = Self(0x2e1);
    pub const BTN_TRIGGER_HAPPY35: Self = Self(0x2e2);
    pub const BTN_TRIGGER_HAPPY36: Self = Self(0x2e3);
    pub const BTN_TRIGGER_HAPPY37: Self = Self(0x2e4);
    pub const BTN_TRIGGER_HAPPY38: Self = Self(0x2e5);
    pub const BTN_TRIGGER_HAPPY39: Self = Self(0x2e6);
    pub const BTN_TRIGGER_HAPPY40: Self = Self(0x2e7);
}
