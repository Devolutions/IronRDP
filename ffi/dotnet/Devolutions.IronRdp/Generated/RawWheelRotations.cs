using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct WheelRotations
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "WheelRotations_new", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern WheelRotations* New([MarshalAs(UnmanagedType.U1)] bool isVertical, short rotationUnits);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "WheelRotations_as_operation", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern Operation* AsOperation(WheelRotations* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "WheelRotations_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(WheelRotations* handle);
}