// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp.Raw;

#nullable enable

[StructLayout(LayoutKind.Sequential)]
public partial struct PduHint
{
    private const string NativeLib = "DevolutionsIronRdp";

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "PduHint_is_some", ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.U1)]
    public static unsafe extern bool IsSome(PduHint* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "PduHint_find_size", ExactSpelling = true)]
    public static unsafe extern ConnectorFfiResultOptBoxOptionalUsizeBoxIronRdpError FindSize(PduHint* self, byte* bytes, nuint bytesSz);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "PduHint_destroy", ExactSpelling = true)]
    public static unsafe extern void Destroy(PduHint* self);
}
