using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct CredsspSequence
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CredsspSequence_next_pdu_hint", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern PduHint* NextPduHint(CredsspSequence* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CredsspSequence_init", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultCredsspSequenceInitResultIronRdpError Init(ClientConnector* connector, DiplomatSliceU8 serverName, DiplomatSliceU8 serverPublicKey, KerberosConfig* kerberoConfigs);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CredsspSequence_decode_server_message", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultTsRequestIronRdpError DecodeServerMessage(CredsspSequence* handle, DiplomatSliceU8 pdu);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CredsspSequence_process_ts_request", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultCredsspProcessGeneratorIronRdpError ProcessTsRequest(CredsspSequence* handle, TsRequest* tsRequest);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CredsspSequence_handle_process_result", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultWrittenIronRdpError HandleProcessResult(CredsspSequence* handle, ClientState* clientState, WriteBuf* buf);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CredsspSequence_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(CredsspSequence* handle);
}