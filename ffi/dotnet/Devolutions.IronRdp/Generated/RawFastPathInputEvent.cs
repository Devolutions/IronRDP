using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct FastPathInputEvent
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "FastPathInputEvent_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(FastPathInputEvent* handle);
}