// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp.Raw;

#nullable enable

[StructLayout(LayoutKind.Sequential)]
public partial struct ClientConnectorState
{
    private const string NativeLib = "DevolutionsIronRdp";

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ClientConnectorState_get_enum_type", ExactSpelling = true)]
    public static unsafe extern ConnectorStateFfiResultClientConnectorStateTypeBoxIronRdpError GetEnumType(ClientConnectorState* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ClientConnectorState_get_connection_initiation_wait_confirm_requested_protocol", ExactSpelling = true)]
    public static unsafe extern ConnectorStateFfiResultBoxSecurityProtocolBoxIronRdpError GetConnectionInitiationWaitConfirmRequestedProtocol(ClientConnectorState* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ClientConnectorState_get_enhanced_security_upgrade_selected_protocol", ExactSpelling = true)]
    public static unsafe extern ConnectorStateFfiResultBoxSecurityProtocolBoxIronRdpError GetEnhancedSecurityUpgradeSelectedProtocol(ClientConnectorState* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ClientConnectorState_get_credssp_selected_protocol", ExactSpelling = true)]
    public static unsafe extern ConnectorStateFfiResultBoxSecurityProtocolBoxIronRdpError GetCredsspSelectedProtocol(ClientConnectorState* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ClientConnectorState_get_basic_settings_exchange_send_initial_selected_protocol", ExactSpelling = true)]
    public static unsafe extern ConnectorStateFfiResultBoxSecurityProtocolBoxIronRdpError GetBasicSettingsExchangeSendInitialSelectedProtocol(ClientConnectorState* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ClientConnectorState_get_basic_settings_exchange_wait_response_connect_initial", ExactSpelling = true)]
    public static unsafe extern ConnectorStateFfiResultBoxConnectInitialBoxIronRdpError GetBasicSettingsExchangeWaitResponseConnectInitial(ClientConnectorState* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ClientConnectorState_get_connected_result", ExactSpelling = true)]
    public static unsafe extern ConnectorStateFfiResultBoxConnectionResultBoxIronRdpError GetConnectedResult(ClientConnectorState* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ClientConnectorState_get_connection_finalization_result", ExactSpelling = true)]
    public static unsafe extern ConnectorStateFfiResultBoxConnectionActivationSequenceBoxIronRdpError GetConnectionFinalizationResult(ClientConnectorState* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ClientConnectorState_destroy", ExactSpelling = true)]
    public static unsafe extern void Destroy(ClientConnectorState* self);
}
