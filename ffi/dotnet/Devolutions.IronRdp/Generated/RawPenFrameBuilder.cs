using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct PenFrameBuilder
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PenFrameBuilder_new", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern PenFrameBuilder* New();

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PenFrameBuilder_push", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Push(PenFrameBuilder* handle, ulong externalId, PenPhase phase, int x, int y);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PenFrameBuilder_push_with_attributes", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void PushWithAttributes(PenFrameBuilder* handle, ulong externalId, PenPhase phase, int x, int y, PenAttributes attributes);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PenFrameBuilder_cancel_all", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void CancelAll(PenFrameBuilder* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PenFrameBuilder_reset", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Reset(PenFrameBuilder* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PenFrameBuilder_get_has_pending_frames", CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.U1)]
    internal static unsafe extern bool GetHasPendingFrames(PenFrameBuilder* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PenFrameBuilder_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(PenFrameBuilder* handle);
}