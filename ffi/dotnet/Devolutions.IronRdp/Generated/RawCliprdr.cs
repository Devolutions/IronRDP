using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct Cliprdr
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Cliprdr_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(Cliprdr* handle);
}