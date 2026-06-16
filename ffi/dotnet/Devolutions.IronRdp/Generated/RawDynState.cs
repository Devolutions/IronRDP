using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct DynState
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DynState_get_name", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVoidIronRdpError GetName(DynState* handle, DiplomatWriteable* writeable);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DynState_is_terminal", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool IsTerminal(DynState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DynState_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(DynState* handle);
}