using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ClientState
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientState_is_reply_needed", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool IsReplyNeeded(ClientState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientState_is_final_message", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool IsFinalMessage(ClientState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientState_get_ts_request", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultTsRequestIronRdpError GetTsRequest(ClientState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientState_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ClientState* handle);
}