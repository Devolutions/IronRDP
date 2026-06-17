using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ConnectionActivationState
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationState_get_type", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ConnectionActivationStateType GetType(ConnectionActivationState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationState_get_capabilities_exchange", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultConnectionActivationStateCapabilitiesExchangeIronRdpError GetCapabilitiesExchange(ConnectionActivationState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationState_get_connection_finalization", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultConnectionActivationStateConnectionFinalizationIronRdpError GetConnectionFinalization(ConnectionActivationState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationState_get_finalized", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultConnectionActivationStateFinalizedIronRdpError GetFinalized(ConnectionActivationState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationState_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ConnectionActivationState* handle);
}