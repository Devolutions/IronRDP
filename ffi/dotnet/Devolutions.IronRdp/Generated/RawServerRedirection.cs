using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ServerRedirection
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ServerRedirection_get_session_id", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern uint GetSessionId(ServerRedirection* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ServerRedirection_get_flags", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern uint GetFlags(ServerRedirection* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ServerRedirection_get_target_fqdn", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultVoidIronRdpError GetTargetFqdn(ServerRedirection* handle, DiplomatWrite* writeable);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ServerRedirection_get_target_netbios_name", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultVoidIronRdpError GetTargetNetbiosName(ServerRedirection* handle, DiplomatWrite* writeable);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ServerRedirection_get_target_net_address", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultVoidIronRdpError GetTargetNetAddress(ServerRedirection* handle, DiplomatWrite* writeable);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ServerRedirection_get_target_net_addresses_count", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern nuint GetTargetNetAddressesCount(ServerRedirection* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ServerRedirection_get_target_net_addresses_at", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultVoidIronRdpError GetTargetNetAddressesAt(ServerRedirection* handle, nuint index, DiplomatWrite* writeable);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ServerRedirection_get_username", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultVoidIronRdpError GetUsername(ServerRedirection* handle, DiplomatWrite* writeable);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ServerRedirection_get_domain", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultVoidIronRdpError GetDomain(ServerRedirection* handle, DiplomatWrite* writeable);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ServerRedirection_get_load_balance_info", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern VecU8* GetLoadBalanceInfo(ServerRedirection* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ServerRedirection_get_password", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern VecU8* GetPassword(ServerRedirection* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ServerRedirection_get_target_certificate", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern VecU8* GetTargetCertificate(ServerRedirection* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ServerRedirection_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ServerRedirection* handle);
}