using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct RDCleanPathPdu
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathPdu_new_request", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultRDCleanPathPduIronRdpError NewRequest(DiplomatSliceU8 x224Pdu, DiplomatSliceU8 destination, DiplomatSliceU8 proxyAuth, DiplomatSliceU8 pcb);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathPdu_from_der", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultRDCleanPathPduIronRdpError FromDer(DiplomatSliceU8 bytes);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathPdu_to_der", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVecU8IronRdpError ToDer(RDCleanPathPdu* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathPdu_detect", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern RDCleanPathDetectionResult* Detect(DiplomatSliceU8 bytes);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathPdu_get_type", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultRDCleanPathResultTypeIronRdpError GetType(RDCleanPathPdu* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathPdu_get_x224_response", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVecU8IronRdpError GetX224Response(RDCleanPathPdu* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathPdu_get_server_cert_chain", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultCertificateChainIteratorIronRdpError GetServerCertChain(RDCleanPathPdu* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathPdu_get_server_addr", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void GetServerAddr(RDCleanPathPdu* handle, DiplomatWriteable* writeable);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathPdu_get_error_message", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void GetErrorMessage(RDCleanPathPdu* handle, DiplomatWriteable* writeable);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathPdu_get_error_code", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultUShortIronRdpError GetErrorCode(RDCleanPathPdu* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathPdu_get_http_status_code", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultUShortIronRdpError GetHttpStatusCode(RDCleanPathPdu* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "RDCleanPathPdu_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(RDCleanPathPdu* handle);
}