// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp.Raw;

#nullable enable

[StructLayout(LayoutKind.Sequential)]
public partial struct DecodedImage
{
    private const string NativeLib = "DevolutionsIronRdp";

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "DecodedImage_get_data", ExactSpelling = true)]
    public static unsafe extern BytesSlice* GetData(DecodedImage* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "DecodedImage_get_width", ExactSpelling = true)]
    public static unsafe extern ushort GetWidth(DecodedImage* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "DecodedImage_get_height", ExactSpelling = true)]
    public static unsafe extern ushort GetHeight(DecodedImage* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "DecodedImage_destroy", ExactSpelling = true)]
    public static unsafe extern void Destroy(DecodedImage* self);
}
