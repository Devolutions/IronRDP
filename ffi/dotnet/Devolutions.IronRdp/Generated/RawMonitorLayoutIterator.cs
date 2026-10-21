using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct MonitorLayoutIterator
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MonitorLayoutIterator_len", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern nuint Len(MonitorLayoutIterator* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MonitorLayoutIterator_is_empty", CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.U1)]
    internal static unsafe extern bool IsEmpty(MonitorLayoutIterator* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MonitorLayoutIterator_next", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern MonitorLayoutEntry* Next(MonitorLayoutIterator* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MonitorLayoutIterator_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(MonitorLayoutIterator* handle);
}