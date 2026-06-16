using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct OptionalUsize
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "OptionalUsize_is_some", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool IsSome(OptionalUsize* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "OptionalUsize_get", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultNUIntIronRdpError Get(OptionalUsize* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "OptionalUsize_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(OptionalUsize* handle);
}