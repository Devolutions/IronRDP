using Avalonia.Input;
using System.Collections.Generic;

public static class KeyCodeMapper
{
    private static readonly Dictionary<PhysicalKey, ushort> KeyToScancodeMap = new Dictionary<PhysicalKey, ushort>
    {
        { PhysicalKey.None, 0x00 },
        { PhysicalKey.A, 0x04 },
        { PhysicalKey.B, 0x05 },
        { PhysicalKey.C, 0x06 },
        { PhysicalKey.D, 0x07 },
        { PhysicalKey.E, 0x08 },
        { PhysicalKey.F, 0x09 },
        { PhysicalKey.G, 0x0A },
        { PhysicalKey.H, 0x0B },
        { PhysicalKey.I, 0x0C },
        { PhysicalKey.J, 0x0D },
        { PhysicalKey.K, 0x0E },
        { PhysicalKey.L, 0x0F },
        { PhysicalKey.M, 0x10 },
        { PhysicalKey.N, 0x11 },
        { PhysicalKey.O, 0x12 },
        { PhysicalKey.P, 0x13 },
        { PhysicalKey.Q, 0x14 },
        { PhysicalKey.R, 0x15 },
        { PhysicalKey.S, 0x16 },
        { PhysicalKey.T, 0x17 },
        { PhysicalKey.U, 0x18 },
        { PhysicalKey.V, 0x19 },
        { PhysicalKey.W, 0x1A },
        { PhysicalKey.X, 0x1B },
        { PhysicalKey.Y, 0x1C },
        { PhysicalKey.Z, 0x1D },
        { PhysicalKey.Digit1, 0x1E },
        { PhysicalKey.Digit2, 0x1F },
        { PhysicalKey.Digit3, 0x20 },
        { PhysicalKey.Digit4, 0x21 },
        { PhysicalKey.Digit5, 0x22 },
        { PhysicalKey.Digit6, 0x23 },
        { PhysicalKey.Digit7, 0x24 },
        { PhysicalKey.Digit8, 0x25 },
        { PhysicalKey.Digit9, 0x26 },
        { PhysicalKey.Digit0, 0x27 },
        { PhysicalKey.Enter, 0x28 },
        { PhysicalKey.Escape, 0x29 },
        { PhysicalKey.Backspace, 0x2A },
        { PhysicalKey.Tab, 0x2B },
        { PhysicalKey.Space, 0x2C },
        { PhysicalKey.Minus, 0x2D },
        { PhysicalKey.Equal, 0x2E },
        { PhysicalKey.BracketLeft, 0x2F },
        { PhysicalKey.BracketRight, 0x30 },
        { PhysicalKey.Backslash, 0x31 },
        { PhysicalKey.Semicolon, 0x33 },
        { PhysicalKey.Quote, 0x34 },
        { PhysicalKey.Backquote, 0x35 },
        { PhysicalKey.Comma, 0x36 },
        { PhysicalKey.Period, 0x37 },
        { PhysicalKey.Slash, 0x38 },
        { PhysicalKey.CapsLock, 0x39 },
        { PhysicalKey.F1, 0x3A },
        { PhysicalKey.F2, 0x3B },
        { PhysicalKey.F3, 0x3C },
        { PhysicalKey.F4, 0x3D },
        { PhysicalKey.F5, 0x3E },
        { PhysicalKey.F6, 0x3F },
        { PhysicalKey.F7, 0x40 },
        { PhysicalKey.F8, 0x41 },
        { PhysicalKey.F9, 0x42 },
        { PhysicalKey.F10, 0x43 },
        { PhysicalKey.F11, 0x44 },
        { PhysicalKey.F12, 0x45 },
        { PhysicalKey.PrintScreen, 0x46 },
        { PhysicalKey.ScrollLock, 0x47 },
        { PhysicalKey.Pause, 0x48 },
        { PhysicalKey.Insert, 0x49 },
        { PhysicalKey.Home, 0x4A },
        { PhysicalKey.PageUp, 0x4B },
        { PhysicalKey.Delete, 0x4C },
        { PhysicalKey.End, 0x4D },
        { PhysicalKey.PageDown, 0x4E },
        { PhysicalKey.ArrowRight, 0x4F },
        { PhysicalKey.ArrowLeft, 0x50 },
        { PhysicalKey.ArrowDown, 0x51 },
        { PhysicalKey.ArrowUp, 0x52 },
        { PhysicalKey.NumLock, 0x53 },
        { PhysicalKey.NumPadDivide, 0x54 },
        { PhysicalKey.NumPadMultiply, 0x55 },
        { PhysicalKey.NumPadSubtract, 0x56 },
        { PhysicalKey.NumPadAdd, 0x57 },
        { PhysicalKey.NumPadEnter, 0x58 },
        { PhysicalKey.NumPad1, 0x59 },
        { PhysicalKey.NumPad2, 0x5A },
        { PhysicalKey.NumPad3, 0x5B },
        { PhysicalKey.NumPad4, 0x5C },
        { PhysicalKey.NumPad5, 0x5D },
        { PhysicalKey.NumPad6, 0x5E },
        { PhysicalKey.NumPad7, 0x5F },
        { PhysicalKey.NumPad8, 0x60 },
        { PhysicalKey.NumPad9, 0x61 },
        { PhysicalKey.NumPad0, 0x62 },
        { PhysicalKey.NumPadDecimal, 0x63 },
        { PhysicalKey.IntlBackslash, 0x64 },
        { PhysicalKey.ContextMenu, 0x65 },
        { PhysicalKey.Power, 0x66 },
        { PhysicalKey.NumPadEqual, 0x67 },
        { PhysicalKey.F13, 0x68 },
        { PhysicalKey.F14, 0x69 },
        { PhysicalKey.F15, 0x6A },
        { PhysicalKey.F16, 0x6B },
        { PhysicalKey.F17, 0x6C },
        { PhysicalKey.F18, 0x6D },
        { PhysicalKey.F19, 0x6E },
        { PhysicalKey.F20, 0x6F },
        { PhysicalKey.F21, 0x70 },
        { PhysicalKey.F22, 0x71 },
        { PhysicalKey.F23, 0x72 },
        { PhysicalKey.F24, 0x73 },
        { PhysicalKey.Open, 0x74 },
        { PhysicalKey.Help, 0x75 },
        { PhysicalKey.Props, 0x76 },
        { PhysicalKey.Again, 0x79 },
        { PhysicalKey.Undo, 0x7A },
        { PhysicalKey.Cut, 0x7B },
        { PhysicalKey.Copy, 0x7C },
        { PhysicalKey.Paste, 0x7D },
        { PhysicalKey.Find, 0x7E },
        { PhysicalKey.NumPadComma, 0x85 },
        { PhysicalKey.Lang1, 0x90 },
        { PhysicalKey.Lang2, 0x91 },
        { PhysicalKey.Lang3, 0x92 },
        { PhysicalKey.Lang4, 0x93 },
        { PhysicalKey.Lang5, 0x94 },
        // Additional keys can be mapped here
    };

    public static ushort GetScancode(PhysicalKey key)
    {
        if (KeyToScancodeMap.TryGetValue(key, out ushort scancode))
        {
            return scancode;
        }
        throw new KeyNotFoundException($"Key {key} not found in the map");
    }
}
