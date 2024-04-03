// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp.Raw;

#nullable enable

[StructLayout(LayoutKind.Sequential)]
public partial struct VecU8
{
    private const string NativeLib = "DevolutionsIronRdp";

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "VecU8_from_bytes", ExactSpelling = true)]
    public static unsafe extern VecU8* FromBytes(byte* bytes, nuint bytesSz);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "VecU8_get_size", ExactSpelling = true)]
    public static unsafe extern nuint GetSize(VecU8* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "VecU8_fill", ExactSpelling = true)]
    public static unsafe extern UtilsFfiResultVoidBoxIronRdpError Fill(VecU8* self, byte* buffer, nuint bufferSz);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "VecU8_new_empty", ExactSpelling = true)]
    public static unsafe extern VecU8* NewEmpty();

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "VecU8_destroy", ExactSpelling = true)]
    public static unsafe extern void Destroy(VecU8* self);
}
