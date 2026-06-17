using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct PerformanceFlags
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PerformanceFlags_new_default", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern PerformanceFlags* NewDefault();

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PerformanceFlags_new_empty", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern PerformanceFlags* NewEmpty();

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PerformanceFlags_add_flag", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void AddFlag(PerformanceFlags* handle, PerformanceFlagsType flag);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PerformanceFlags_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(PerformanceFlags* handle);
}