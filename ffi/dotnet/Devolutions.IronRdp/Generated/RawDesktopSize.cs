using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct DesktopSize
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DesktopSize_get_width", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetWidth(DesktopSize* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DesktopSize_get_height", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetHeight(DesktopSize* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DesktopSize_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(DesktopSize* handle);
}