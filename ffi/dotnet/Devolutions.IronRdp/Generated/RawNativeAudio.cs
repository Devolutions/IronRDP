using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct NativeAudio
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "NativeAudio_new", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultNativeAudioIronRdpError New([MarshalAs(UnmanagedType.U1)] bool playback, [MarshalAs(UnmanagedType.U1)] bool capture, DiplomatSliceU8 outputDevice, DiplomatSliceU8 inputDevice, uint latencyMs);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "NativeAudio_next_status", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern AudioStatus* NextStatus(NativeAudio* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "NativeAudio_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(NativeAudio* handle);
}