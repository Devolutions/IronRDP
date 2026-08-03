using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct GracefulDisconnectReason
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "GracefulDisconnectReason_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(GracefulDisconnectReason* handle);
}