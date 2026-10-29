using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct DisplayControlCapabilities
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DisplayControlCapabilities_get_max_num_monitors", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern uint GetMaxNumMonitors(DisplayControlCapabilities* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DisplayControlCapabilities_get_max_monitor_area", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern ulong GetMaxMonitorArea(DisplayControlCapabilities* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DisplayControlCapabilities_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(DisplayControlCapabilities* handle);
}