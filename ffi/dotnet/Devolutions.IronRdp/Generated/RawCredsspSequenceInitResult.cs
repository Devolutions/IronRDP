using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct CredsspSequenceInitResult
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CredsspSequenceInitResult_get_credssp_sequence", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultCredsspSequenceIronRdpError GetCredsspSequence(CredsspSequenceInitResult* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CredsspSequenceInitResult_get_ts_request", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultTsRequestIronRdpError GetTsRequest(CredsspSequenceInitResult* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CredsspSequenceInitResult_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(CredsspSequenceInitResult* handle);
}