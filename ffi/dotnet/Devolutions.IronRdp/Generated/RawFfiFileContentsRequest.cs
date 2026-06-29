using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct FfiFileContentsRequest
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "FfiFileContentsRequest_stream_id", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern uint StreamId(FfiFileContentsRequest* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "FfiFileContentsRequest_index", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern int Index(FfiFileContentsRequest* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "FfiFileContentsRequest_is_size_request", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool IsSizeRequest(FfiFileContentsRequest* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "FfiFileContentsRequest_is_range_request", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool IsRangeRequest(FfiFileContentsRequest* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "FfiFileContentsRequest_position", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ulong Position(FfiFileContentsRequest* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "FfiFileContentsRequest_requested_size", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern uint RequestedSize(FfiFileContentsRequest* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "FfiFileContentsRequest_has_data_id", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool HasDataId(FfiFileContentsRequest* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "FfiFileContentsRequest_data_id", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultUIntIronRdpError DataId(FfiFileContentsRequest* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "FfiFileContentsRequest_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(FfiFileContentsRequest* handle);
}