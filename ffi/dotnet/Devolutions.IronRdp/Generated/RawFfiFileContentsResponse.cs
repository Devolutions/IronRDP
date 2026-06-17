using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct FfiFileContentsResponse
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "FfiFileContentsResponse_stream_id", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern uint StreamId(FfiFileContentsResponse* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "FfiFileContentsResponse_is_error", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool IsError(FfiFileContentsResponse* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "FfiFileContentsResponse_data", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern VecU8* Data(FfiFileContentsResponse* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "FfiFileContentsResponse_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(FfiFileContentsResponse* handle);
}