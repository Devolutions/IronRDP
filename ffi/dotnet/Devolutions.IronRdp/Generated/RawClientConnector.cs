using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ClientConnector
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnector_new", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultClientConnectorIronRdpError New(Config* config, DiplomatSliceU8 clientAddr);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnector_with_static_channel_rdp_snd", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVoidIronRdpError WithStaticChannelRdpSnd(ClientConnector* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnector_with_static_channel_rdpdr", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVoidIronRdpError WithStaticChannelRdpdr(ClientConnector* handle, DiplomatSliceU8 computerName, uint smartCardDeviceId);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnector_with_dynamic_channel_display_control", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVoidIronRdpError WithDynamicChannelDisplayControl(ClientConnector* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnector_with_dynamic_channel_pipe_proxy", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVoidIronRdpError WithDynamicChannelPipeProxy(ClientConnector* handle, DvcPipeProxyConfig* config);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnector_should_perform_security_upgrade", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultBoolIronRdpError ShouldPerformSecurityUpgrade(ClientConnector* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnector_mark_security_upgrade_as_done", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVoidIronRdpError MarkSecurityUpgradeAsDone(ClientConnector* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnector_should_perform_credssp", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultBoolIronRdpError ShouldPerformCredssp(ClientConnector* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnector_mark_credssp_as_done", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVoidIronRdpError MarkCredsspAsDone(ClientConnector* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnector_step", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultWrittenIronRdpError Step(ClientConnector* handle, DiplomatSliceU8 input, WriteBuf* writeBuf);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnector_step_no_input", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultWrittenIronRdpError StepNoInput(ClientConnector* handle, WriteBuf* writeBuf);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnector_attach_static_cliprdr", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVoidIronRdpError AttachStaticCliprdr(ClientConnector* handle, Cliprdr* cliprdr);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnector_next_pdu_hint", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultPduHintIronRdpError NextPduHint(ClientConnector* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnector_get_dyn_state", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultDynStateIronRdpError GetDynState(ClientConnector* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnector_consume_and_cast_to_client_connector_state", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultClientConnectorStateIronRdpError ConsumeAndCastToClientConnectorState(ClientConnector* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClientConnector_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ClientConnector* handle);
}