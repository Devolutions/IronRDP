using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct IronRdpError
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "IronRdpError_to_display", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void ToDisplay(IronRdpError* handle, DiplomatWriteable* writeable);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "IronRdpError_get_kind", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern IronRdpErrorKind GetKind(IronRdpError* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "IronRdpError_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(IronRdpError* handle);
}