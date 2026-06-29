using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct NetworkRequest
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "NetworkRequest_get_data", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern VecU8* GetData(NetworkRequest* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "NetworkRequest_get_protocol", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern NetworkRequestProtocol GetProtocol(NetworkRequest* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "NetworkRequest_get_url", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVoidIronRdpError GetUrl(NetworkRequest* handle, DiplomatWriteable* writeable);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "NetworkRequest_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(NetworkRequest* handle);
}