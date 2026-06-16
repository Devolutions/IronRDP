using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ConnectionResult
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionResult_get_io_channel_id", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultUShortIronRdpError GetIoChannelId(ConnectionResult* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionResult_get_user_channel_id", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultUShortIronRdpError GetUserChannelId(ConnectionResult* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionResult_get_share_id", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultUIntIronRdpError GetShareId(ConnectionResult* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionResult_get_desktop_size", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultDesktopSizeIronRdpError GetDesktopSize(ConnectionResult* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionResult_get_enable_server_pointer", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultBoolIronRdpError GetEnableServerPointer(ConnectionResult* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionResult_get_pointer_software_rendering", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultBoolIronRdpError GetPointerSoftwareRendering(ConnectionResult* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionResult_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ConnectionResult* handle);
}