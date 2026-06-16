using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct GeneratorState
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "GeneratorState_is_suspended", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool IsSuspended(GeneratorState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "GeneratorState_is_completed", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool IsCompleted(GeneratorState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "GeneratorState_get_network_request_if_suspended", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern NetworkRequest* GetNetworkRequestIfSuspended(GeneratorState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "GeneratorState_get_client_state_if_completed", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultClientStateIronRdpError GetClientStateIfCompleted(GeneratorState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "GeneratorState_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(GeneratorState* handle);
}