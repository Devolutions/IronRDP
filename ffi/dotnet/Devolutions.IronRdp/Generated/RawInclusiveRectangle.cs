using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct InclusiveRectangle
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "InclusiveRectangle_get_left", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetLeft(InclusiveRectangle* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "InclusiveRectangle_get_top", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetTop(InclusiveRectangle* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "InclusiveRectangle_get_right", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetRight(InclusiveRectangle* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "InclusiveRectangle_get_bottom", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetBottom(InclusiveRectangle* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "InclusiveRectangle_get_width", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetWidth(InclusiveRectangle* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "InclusiveRectangle_get_height", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetHeight(InclusiveRectangle* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "InclusiveRectangle_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(InclusiveRectangle* handle);
}