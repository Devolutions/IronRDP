using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct DirectDvcConfig
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DirectDvcConfig_new", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DirectDvcConfig* New(DirectDvcEventSink* eventSink);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DirectDvcConfig_add_channel", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void AddChannel(DirectDvcConfig* handle, DiplomatSliceU8 channelName);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DirectDvcConfig_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(DirectDvcConfig* handle);
}