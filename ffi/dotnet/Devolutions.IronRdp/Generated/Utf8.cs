namespace Devolutions.IronRdp.Diplomat;

/// <summary>
/// Explicit conversions between a C# <c>string</c> (always UTF-16 internally)
/// and raw UTF-8 bytes. Handing a <c>string</c> to a Rust <c>&amp;str</c>
/// parameter always requires transcoding — there's no way around that
/// allocation. This class exists so the copy is a visible, separately-named
/// step in generated code (<c>Utf8.Clone(...)</c>) instead of being buried
/// inside marshalling.
/// </summary>
internal static class Utf8
{
    public static byte[] Clone(string value) => System.Text.Encoding.UTF8.GetBytes(value);
}