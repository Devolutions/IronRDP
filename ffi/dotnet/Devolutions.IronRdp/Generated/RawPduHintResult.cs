// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp.Raw;

#nullable enable

[StructLayout(LayoutKind.Sequential)]
public partial struct PduHintResult
{
    private const string NativeLib = "DevolutionsIronRdp";

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "PduHintResult_is_some", ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.U1)]
    public static unsafe extern bool IsSome(PduHintResult* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "PduHintResult_find_size", ExactSpelling = true)]
    public static unsafe extern ConnectorFfiResultOptBoxOptionalUsizeBoxIronRdpError FindSize(PduHintResult* self, VecU8* buffer);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "PduHintResult_destroy", ExactSpelling = true)]
    public static unsafe extern void Destroy(PduHintResult* self);
}
