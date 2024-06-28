// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp.Raw;

#nullable enable

[StructLayout(LayoutKind.Sequential)]
public partial struct WinCliprdr
{
    private const string NativeLib = "DevolutionsIronRdp";

    /// <summary>
    /// SAFETY: `hwnd` must be a valid window handle
    /// </summary>
    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "WinCliprdr_new", ExactSpelling = true)]
    public static unsafe extern ClipboardWindowsFfiResultBoxWinCliprdrBoxIronRdpError New(nint hwnd);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "WinCliprdr_next_clipboard_message", ExactSpelling = true)]
    public static unsafe extern ClipboardWindowsFfiResultOptBoxClipboardMessageBoxIronRdpError NextClipboardMessage(WinCliprdr* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "WinCliprdr_next_clipboard_message_blocking", ExactSpelling = true)]
    public static unsafe extern ClipboardWindowsFfiResultBoxClipboardMessageBoxIronRdpError NextClipboardMessageBlocking(WinCliprdr* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "WinCliprdr_backend_factory", ExactSpelling = true)]
    public static unsafe extern ClipboardWindowsFfiResultBoxCliprdrBackendFactoryBoxIronRdpError BackendFactory(WinCliprdr* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "WinCliprdr_destroy", ExactSpelling = true)]
    public static unsafe extern void Destroy(WinCliprdr* self);
}
