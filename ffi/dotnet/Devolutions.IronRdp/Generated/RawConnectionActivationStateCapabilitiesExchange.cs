using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ConnectionActivationStateCapabilitiesExchange
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationStateCapabilitiesExchange_get_io_channel_id", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetIoChannelId(ConnectionActivationStateCapabilitiesExchange* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationStateCapabilitiesExchange_get_user_channel_id", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetUserChannelId(ConnectionActivationStateCapabilitiesExchange* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationStateCapabilitiesExchange_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ConnectionActivationStateCapabilitiesExchange* handle);
}