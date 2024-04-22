using Avalonia.Input;
using System.Collections.Generic;

public static class KeyCodeMapper
{
    private static readonly Dictionary<PhysicalKey, ushort> KeyToScancodeMap = new Dictionary<PhysicalKey, ushort>
    {
        {PhysicalKey.Escape, 0x01},
        {PhysicalKey.Digit1, 0x02},
        {PhysicalKey.Digit2, 0x03},
        {PhysicalKey.Digit3, 0x04},
        {PhysicalKey.Digit4, 0x05},
        {PhysicalKey.Digit5, 0x06},
        {PhysicalKey.Digit6, 0x07},
        {PhysicalKey.Digit7, 0x08},
        {PhysicalKey.Digit8, 0x09},
        {PhysicalKey.Digit9, 0x0A},
        {PhysicalKey.Digit0, 0x0B},
        {PhysicalKey.Minus, 0x0C},
        {PhysicalKey.Equal, 0x0D},
        {PhysicalKey.Backspace, 0x0E},
        {PhysicalKey.Tab, 0x0F},
        {PhysicalKey.Q, 0x10},
        {PhysicalKey.W, 0x11},
        {PhysicalKey.E, 0x12},
        {PhysicalKey.R, 0x13},
        {PhysicalKey.T, 0x14},
        {PhysicalKey.Y, 0x15},
        {PhysicalKey.U, 0x16},
        {PhysicalKey.I, 0x17},
        {PhysicalKey.O, 0x18},
        {PhysicalKey.P, 0x19},
        {PhysicalKey.BracketLeft, 0x1A},
        {PhysicalKey.BracketRight, 0x1B},
        {PhysicalKey.Enter, 0x1C},
        {PhysicalKey.ControlLeft, 0x1D},
        {PhysicalKey.A, 0x1E},
        {PhysicalKey.S, 0x1F},
        {PhysicalKey.D, 0x20},
        {PhysicalKey.F, 0x21},
        {PhysicalKey.G, 0x22},
        {PhysicalKey.H, 0x23},
        {PhysicalKey.J, 0x24},
        {PhysicalKey.K, 0x25},
        {PhysicalKey.L, 0x26},
        {PhysicalKey.Semicolon, 0x27},
        {PhysicalKey.Quote, 0x28},
        {PhysicalKey.ShiftLeft, 0x2A},
        {PhysicalKey.Backslash, 0x2B},
        {PhysicalKey.Z, 0x2C},
        {PhysicalKey.X, 0x2D},
        {PhysicalKey.C, 0x2E},
        {PhysicalKey.V, 0x2F},
        {PhysicalKey.B, 0x30},
        {PhysicalKey.N, 0x31},
        {PhysicalKey.M, 0x32},
        {PhysicalKey.Comma, 0x33},
        {PhysicalKey.Period, 0x34},
        {PhysicalKey.Slash, 0x35},
        {PhysicalKey.ShiftRight, 0x36},
        {PhysicalKey.PrintScreen, 0x37},
        {PhysicalKey.AltLeft, 0x38},
        {PhysicalKey.Space, 0x39},
        {PhysicalKey.CapsLock, 0x3A},
        {PhysicalKey.F1, 0x3B},
        {PhysicalKey.F2, 0x3C},
        {PhysicalKey.F3, 0x3D},
        {PhysicalKey.F4, 0x3E},
        {PhysicalKey.F5, 0x3F},
        {PhysicalKey.F6, 0x40},
        {PhysicalKey.F7, 0x41},
        {PhysicalKey.F8, 0x42},
        {PhysicalKey.F9, 0x43},
        {PhysicalKey.F10, 0x44},
        {PhysicalKey.NumLock, 0x45},
        {PhysicalKey.ScrollLock, 0x46},
        {PhysicalKey.Home, 0x47},
        {PhysicalKey.ArrowUp, 0x48},
        {PhysicalKey.PageUp, 0x49},
        {PhysicalKey.NumPadSubtract, 0x4A},
        {PhysicalKey.ArrowLeft, 0x4B},
        {PhysicalKey.NumPad5, 0x4C},
        {PhysicalKey.ArrowRight, 0x4D},
        {PhysicalKey.NumPadAdd, 0x4E},
        {PhysicalKey.End, 0x4F},
        {PhysicalKey.ArrowDown, 0x50},
        {PhysicalKey.PageDown, 0x51},
        {PhysicalKey.Insert, 0x52},
        {PhysicalKey.Delete, 0x53},
        {PhysicalKey.F11, 0x57},
        {PhysicalKey.F12, 0x58}
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
