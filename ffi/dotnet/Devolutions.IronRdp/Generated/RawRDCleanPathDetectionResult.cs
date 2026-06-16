using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct RDCleanPathDetectionResult
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathDetectionResult_is_detected", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool IsDetected(RDCleanPathDetectionResult* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathDetectionResult_is_not_enough_bytes", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool IsNotEnoughBytes(RDCleanPathDetectionResult* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathDetectionResult_is_failed", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool IsFailed(RDCleanPathDetectionResult* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathDetectionResult_get_total_length", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultNUIntIronRdpError GetTotalLength(RDCleanPathDetectionResult* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathDetectionResult_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(RDCleanPathDetectionResult* handle);
}