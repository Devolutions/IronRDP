using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct DvcPipeProxyMessage
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DvcPipeProxyMessage_get_channel_id", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern uint GetChannelId(DvcPipeProxyMessage* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DvcPipeProxyMessage_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(DvcPipeProxyMessage* handle);
}