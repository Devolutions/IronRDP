using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct DirectDvcEvent
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DirectDvcEvent_get_kind", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DirectDvcEventKind GetKind(DirectDvcEvent* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DirectDvcEvent_get_channel_id", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern uint GetChannelId(DirectDvcEvent* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DirectDvcEvent_get_channel_name", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultVoidIronRdpError GetChannelName(DirectDvcEvent* handle, DiplomatWrite* writeable);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DirectDvcEvent_get_endpoint_id", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern ulong GetEndpointId(DirectDvcEvent* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DirectDvcEvent_take_data", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern VecU8* TakeData(DirectDvcEvent* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DirectDvcEvent_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(DirectDvcEvent* handle);
}