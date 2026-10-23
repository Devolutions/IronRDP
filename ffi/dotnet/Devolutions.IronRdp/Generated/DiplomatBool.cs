using System.Runtime.InteropServices;

namespace Devolutions.IronRdp.Diplomat;

// Blittable one-byte stand-in for `bool`. .NET Framework P/Invoke can't return a
// struct by value if any field is non-blittable, and `bool` is non-blittable, so
// Result/Option tags and struct bool fields use a byte (these bindings target
// netstandard2.0 and must load on net48).
// Rules: https://learn.microsoft.com/en-us/dotnet/standard/native-interop/blittable-and-non-blittable-types
// Matches Rust's 1-byte bool (0/1): https://doc.rust-lang.org/reference/types/boolean.html
[StructLayout(LayoutKind.Sequential)]
internal readonly struct DiplomatBool
{
    private readonly byte _value;

    private DiplomatBool(byte value) => _value = value;

    // Implicit both ways so the idiomatic layer stays bool-only.
    public static implicit operator DiplomatBool(bool value) => new DiplomatBool(value ? (byte)1 : (byte)0);
    public static implicit operator bool(DiplomatBool value) => value._value != 0;
}