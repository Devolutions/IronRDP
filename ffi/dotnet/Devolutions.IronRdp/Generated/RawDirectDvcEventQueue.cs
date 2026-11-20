using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct DirectDvcEventQueue
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DirectDvcEventQueue_new", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultDirectDvcEventQueueIronRdpError New(uint queueSize);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DirectDvcEventQueue_next_event", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultDirectDvcEventIronRdpError NextEvent(DirectDvcEventQueue* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DirectDvcEventQueue_get_sink", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DirectDvcEventSink* GetSink(DirectDvcEventQueue* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DirectDvcEventQueue_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(DirectDvcEventQueue* handle);
}