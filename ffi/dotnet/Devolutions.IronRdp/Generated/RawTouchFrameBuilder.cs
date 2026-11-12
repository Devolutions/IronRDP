using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct TouchFrameBuilder
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "TouchFrameBuilder_new", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern TouchFrameBuilder* New(ushort maxContacts);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "TouchFrameBuilder_push", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Push(TouchFrameBuilder* handle, ulong externalId, TouchPhase phase, int x, int y);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "TouchFrameBuilder_push_with_pressure", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void PushWithPressure(TouchFrameBuilder* handle, ulong externalId, TouchPhase phase, int x, int y, uint pressure);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "TouchFrameBuilder_cancel_all", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void CancelAll(TouchFrameBuilder* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "TouchFrameBuilder_reset", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Reset(TouchFrameBuilder* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "TouchFrameBuilder_get_has_pending_frames", CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.U1)]
    internal static unsafe extern bool GetHasPendingFrames(TouchFrameBuilder* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "TouchFrameBuilder_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(TouchFrameBuilder* handle);
}