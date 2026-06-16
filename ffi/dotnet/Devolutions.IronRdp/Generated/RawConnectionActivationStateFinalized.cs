using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ConnectionActivationStateFinalized
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationStateFinalized_get_io_channel_id", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetIoChannelId(ConnectionActivationStateFinalized* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationStateFinalized_get_user_channel_id", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetUserChannelId(ConnectionActivationStateFinalized* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationStateFinalized_get_share_id", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern uint GetShareId(ConnectionActivationStateFinalized* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationStateFinalized_get_desktop_size", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DesktopSize* GetDesktopSize(ConnectionActivationStateFinalized* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationStateFinalized_get_enable_server_pointer", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool GetEnableServerPointer(ConnectionActivationStateFinalized* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationStateFinalized_get_pointer_software_rendering", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool GetPointerSoftwareRendering(ConnectionActivationStateFinalized* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationStateFinalized_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ConnectionActivationStateFinalized* handle);
}