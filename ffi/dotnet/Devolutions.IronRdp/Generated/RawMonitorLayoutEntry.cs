using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct MonitorLayoutEntry
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MonitorLayoutEntry_get_left", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern int GetLeft(MonitorLayoutEntry* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MonitorLayoutEntry_get_top", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern int GetTop(MonitorLayoutEntry* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MonitorLayoutEntry_get_right", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern int GetRight(MonitorLayoutEntry* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MonitorLayoutEntry_get_bottom", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern int GetBottom(MonitorLayoutEntry* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MonitorLayoutEntry_is_primary", CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.U1)]
    internal static unsafe extern bool IsPrimary(MonitorLayoutEntry* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MonitorLayoutEntry_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(MonitorLayoutEntry* handle);
}