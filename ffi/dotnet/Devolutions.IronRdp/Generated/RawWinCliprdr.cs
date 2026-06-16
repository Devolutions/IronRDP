using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct WinCliprdr
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "WinCliprdr_new", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultWinCliprdrIronRdpError New();

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "WinCliprdr_next_clipboard_message", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultClipboardMessageIronRdpError NextClipboardMessage(WinCliprdr* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "WinCliprdr_next_clipboard_message_blocking", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultClipboardMessageIronRdpError NextClipboardMessageBlocking(WinCliprdr* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "WinCliprdr_backend_factory", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultCliprdrBackendFactoryIronRdpError BackendFactory(WinCliprdr* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "WinCliprdr_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(WinCliprdr* handle);
}