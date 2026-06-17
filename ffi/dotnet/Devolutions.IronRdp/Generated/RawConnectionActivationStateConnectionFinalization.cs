using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ConnectionActivationStateConnectionFinalization
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationStateConnectionFinalization_get_io_channel_id", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetIoChannelId(ConnectionActivationStateConnectionFinalization* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationStateConnectionFinalization_get_user_channel_id", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetUserChannelId(ConnectionActivationStateConnectionFinalization* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationStateConnectionFinalization_get_desktop_size", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DesktopSize* GetDesktopSize(ConnectionActivationStateConnectionFinalization* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationStateConnectionFinalization_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ConnectionActivationStateConnectionFinalization* handle);
}