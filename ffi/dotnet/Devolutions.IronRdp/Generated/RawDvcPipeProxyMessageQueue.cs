using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct DvcPipeProxyMessageQueue
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DvcPipeProxyMessageQueue_new", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DvcPipeProxyMessageQueue* New(uint queueSize);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DvcPipeProxyMessageQueue_next_message", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultDvcPipeProxyMessageIronRdpError NextMessage(DvcPipeProxyMessageQueue* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DvcPipeProxyMessageQueue_next_message_blocking", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultDvcPipeProxyMessageIronRdpError NextMessageBlocking(DvcPipeProxyMessageQueue* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DvcPipeProxyMessageQueue_get_sink", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DvcPipeProxyMessageSink* GetSink(DvcPipeProxyMessageQueue* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DvcPipeProxyMessageQueue_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(DvcPipeProxyMessageQueue* handle);
}