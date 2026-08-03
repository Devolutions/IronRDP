using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct FastPathInputEventIterator
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "FastPathInputEventIterator_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(FastPathInputEventIterator* handle);
}