import UAParser from 'ua-parser-js';

/*
 * Scancode found on: https://developer.mozilla.org/en-US/docs/Web/API/UI_Events/Keyboard_event_code_values
 */

type EngineName = 'gecko' | 'blink' | 'webkit';

type CodeMap = Record<string, string>;

const parser = new UAParser();
const parsedUA = parser.getResult();
const engine = parsedUA.engine.name?.toLowerCase() as EngineName;
const engineMajorVersion = Number(parsedUA.engine.version?.split('.')[0]);

const scanCodeToKeyCode = {
    '0x0001': 'Escape',
    '0x0002': 'Digit1',
    '0x0003': 'Digit2',
    '0x0004': 'Digit3',
    '0x0005': 'Digit4',
    '0x0006': 'Digit5',
    '0x0007': 'Digit6',
    '0x0008': 'Digit7',
    '0x0009': 'Digit8',
    '0x000A': 'Digit9',
    '0x000B': 'Digit0',
    '0x000C': 'Minus',
    '0x000D': 'Equal',
    '0x000E': 'Backspace',
    '0x000F': 'Tab',
    '0x0010': 'KeyQ',
    '0x0011': 'KeyW',
    '0x0012': 'KeyE',
    '0x0013': 'KeyR',
    '0x0014': 'KeyT',
    '0x0015': 'KeyY',
    '0x0016': 'KeyU',
    '0x0017': 'KeyI',
    '0x0018': 'KeyO',
    '0x0019': 'KeyP',
    '0x001A': 'BracketLeft',
    '0x001B': 'BracketRight',
    '0x001C': 'Enter',
    '0x001D': 'ControlLeft',
    '0x001E': 'KeyA',
    '0x001F': 'KeyS',
    '0x0020': 'KeyD',
    '0x0021': 'KeyF',
    '0x0022': 'KeyG',
    '0x0023': 'KeyH',
    '0x0024': 'KeyJ',
    '0x0025': 'KeyK',
    '0x0026': 'KeyL',
    '0x0027': 'Semicolon',
    '0x0028': 'Quote',
    '0x0029': 'Backquote',
    '0x002A': 'ShiftLeft',
    '0x002B': 'Backslash',
    '0x002C': 'KeyZ',
    '0x002D': 'KeyX',
    '0x002E': 'KeyC',
    '0x002F': 'KeyV',
    '0x0030': 'KeyB',
    '0x0031': 'KeyN',
    '0x0032': 'KeyM',
    '0x0033': 'Comma',
    '0x0034': 'Period',
    '0x0035': 'Slash',
    '0x0036': 'ShiftRight',
    '0x0037': 'NumpadMultiply',
    '0x0038': 'AltLeft',
    '0x0039': 'Space',
    '0x003A': 'CapsLock',
    '0x003B': 'F1',
    '0x003C': 'F2',
    '0x003D': 'F3',
    '0x003E': 'F4',
    '0x003F': 'F5',
    '0x0040': 'F6',
    '0x0041': 'F7',
    '0x0042': 'F8',
    '0x0043': 'F9',
    '0x0044': 'F10',
    '0x0045': 'Pause',
    '0x0046': 'ScrollLock',
    '0x0047': 'Numpad7',
    '0x0048': 'Numpad8',
    '0x0049': 'Numpad9',
    '0x004A': 'NumpadSubtract',
    '0x004B': 'Numpad4',
    '0x004C': 'Numpad5',
    '0x004D': 'Numpad6',
    '0x004E': 'NumpadAdd',
    '0x004F': 'Numpad1',
    '0x0050': 'Numpad2',
    '0x0051': 'Numpad3',
    '0x0052': 'Numpad0',
    '0x0053': 'NumpadDecimal',
    '0x0056': 'IntlBackslash',
    '0x0057': 'F11',
    '0x0058': 'F12',
    '0x0059': 'NumpadEqual',
    '0x0064': 'F13',
    '0x0065': 'F14',
    '0x0066': 'F15',
    '0x0067': 'F16',
    '0x0068': 'F17',
    '0x0069': 'F18',
    '0x006A': 'F19',
    '0x006B': 'F20',
    '0x006C': 'F21',
    '0x006D': 'F22',
    '0x006E': 'F23',
    '0x0070': 'KanaMode',
    '0x0071': 'Lang2',
    '0x0072': 'Lang1',
    '0x0073': 'IntlRo',
    '0x0076': 'F24',
    '0x0079': 'Convert',
    '0x007B': 'NonConvert',
    '0x007D': 'IntlYen',
    '0x007E': 'NumpadComma',
    '0xE010': 'MediaTrackPrevious',
    '0xE019': 'MediaTrackNext',
    '0xE01C': 'NumpadEnter',
    '0xE01D': 'ControlRight',
    '0xE021': 'LaunchApp2',
    '0xE022': 'MediaPlayPause',
    '0xE024': 'MediaStop',
    '0xE032': 'BrowserHome',
    '0xE035': 'NumpadDivide',
    '0xE037': 'PrintScreen',
    '0xE038': 'AltRight',
    '0xE045': 'NumLock',
    '0xE046': 'Pause',
    '0xE047': 'Home',
    '0xE048': 'ArrowUp',
    '0xE049': 'PageUp',
    '0xE04B': 'ArrowLeft',
    '0xE04D': 'ArrowRight',
    '0xE04F': 'End',
    '0xE050': 'ArrowDown',
    '0xE051': 'PageDown',
    '0xE052': 'Insert',
    '0xE053': 'Delete',
    '0xE05D': 'ContextMenu',
    '0xE05E': 'Power',
    '0xE065': 'BrowserSearch',
    '0xE066': 'BrowserFavorites',
    '0xE067': 'BrowserRefresh',
    '0xE068': 'BrowserStop',
    '0xE069': 'BrowserForward',
    '0xE06A': 'BrowserBack',
    '0xE06B': 'LaunchApp1',
    '0xE06C': 'LaunchMail',
    '0xE06D': 'MediaSelect',
};

const codeToScanCodeBlinkOverride = {
    '0x0077': 'Lang4',
    '0x0078': 'Lang3',
    '0xE008': 'Undo',
    '0xE00A': 'Paste',
    '0xE017': 'Cut',
    '0xE018': 'Copy',
    '0xE020': 'AudioVolumeMute',
    '0xE02C': 'Eject',
    '0xE02E': 'AudioVolumeDown',
    '0xE030': 'AudioVolumeUp',
    '0xE03B': 'Help',
    '0xE05B': 'MetaLeft',
    '0xE05C': 'MetaRight',
    '0xE05F': 'Sleep',
    '0xE063': 'WakeUp',
};

const scanCodeToKeyCodeGeckoOverride = {
    '0x0054': 'PrintScreen',
    '0xE020': 'VolumeMute', // The documentation says it's 'AudioVolumeMute', but the actual test shows that it's 'VolumeMute'.
    '0xE02E': 'VolumeDown',
    '0xE030': 'VolumeUp',
    '0xE05B': engineMajorVersion > 117 ? 'MetaLeft' : 'OSLeft',
    '0xE05C': engineMajorVersion > 117 ? 'MetaRight' : 'OSRight',
};

const KeyCodeToScanCode = {
    blink: invertCodesMapping({ ...scanCodeToKeyCode, ...codeToScanCodeBlinkOverride }),
    gecko: invertCodesMapping({ ...scanCodeToKeyCode, ...scanCodeToKeyCodeGeckoOverride }),
    webkit: invertCodesMapping(scanCodeToKeyCode),
};

function invertCodesMapping(obj: CodeMap) {
    const result: CodeMap = {};

    for (const [scanCode, keyCode] of Object.entries(obj)) {
        result[keyCode] = scanCode;
    }

    return result;
}

/**
 * Retrieves the Windows scancode corresponding to a browser key code.
 *
 * @remarks
 * We use only the mapping to **Windows** scancodes because:
 *
 * - RDP requires Windows scancodes to transmit correct key events. Since the RDP server runs only on Windows,
 *   only Windows scancodes are relevant.
 * - VNC is cross-platform protocol, and it uses KeySym for key events. IronVNC module has mapping from
 *   Windows scancodes to the corresponding KeySyms for non-Unicode symbols to ensure compatability.
 */
export const scanCode = function (code: string): number {
    const map = KeyCodeToScanCode[engine];
    return parseInt(map[code], 16);
};
