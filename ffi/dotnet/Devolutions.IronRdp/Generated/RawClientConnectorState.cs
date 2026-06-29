using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ClientConnectorState
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnectorState_get_enum_type", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultClientConnectorStateTypeIronRdpError GetEnumType(ClientConnectorState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnectorState_get_connection_initiation_wait_confirm_requested_protocol", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultSecurityProtocolIronRdpError GetConnectionInitiationWaitConfirmRequestedProtocol(ClientConnectorState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnectorState_get_enhanced_security_upgrade_selected_protocol", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultSecurityProtocolIronRdpError GetEnhancedSecurityUpgradeSelectedProtocol(ClientConnectorState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnectorState_get_credssp_selected_protocol", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultSecurityProtocolIronRdpError GetCredsspSelectedProtocol(ClientConnectorState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnectorState_get_basic_settings_exchange_send_initial_selected_protocol", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultSecurityProtocolIronRdpError GetBasicSettingsExchangeSendInitialSelectedProtocol(ClientConnectorState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnectorState_get_basic_settings_exchange_wait_response_connect_initial", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultConnectInitialIronRdpError GetBasicSettingsExchangeWaitResponseConnectInitial(ClientConnectorState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnectorState_get_connected_result", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultConnectionResultIronRdpError GetConnectedResult(ClientConnectorState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnectorState_get_connection_finalization_result", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultConnectionActivationSequenceIronRdpError GetConnectionFinalizationResult(ClientConnectorState* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnectorState_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ClientConnectorState* handle);
}