using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct AudioStatus
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "AudioStatus_is_capture", CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.U1)]
    internal static unsafe extern bool IsCapture(AudioStatus* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "AudioStatus_message", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultVoidIronRdpError Message(AudioStatus* handle, DiplomatWrite* writeable);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "AudioStatus_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(AudioStatus* handle);
}