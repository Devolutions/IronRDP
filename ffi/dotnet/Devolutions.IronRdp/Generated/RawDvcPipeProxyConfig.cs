using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct DvcPipeProxyConfig
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DvcPipeProxyConfig_new", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DvcPipeProxyConfig* New(DvcPipeProxyMessageSink* messageSink);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DvcPipeProxyConfig_add_pipe_proxy", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void AddPipeProxy(DvcPipeProxyConfig* handle, DvcPipeProxyDescriptor* descriptor);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DvcPipeProxyConfig_get_message_sink", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DvcPipeProxyMessageSink* GetMessageSink(DvcPipeProxyConfig* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DvcPipeProxyConfig_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(DvcPipeProxyConfig* handle);
}