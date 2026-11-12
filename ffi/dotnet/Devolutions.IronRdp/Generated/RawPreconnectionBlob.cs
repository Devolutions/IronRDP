using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct PreconnectionBlob
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PreconnectionBlob_new_v1", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern PreconnectionBlob* NewV1(uint id);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PreconnectionBlob_new_v2", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern PreconnectionBlob* NewV2(uint id, DiplomatSliceU8 payload);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PreconnectionBlob_encode", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultVecU8IronRdpError Encode(PreconnectionBlob* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PreconnectionBlob_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(PreconnectionBlob* handle);
}