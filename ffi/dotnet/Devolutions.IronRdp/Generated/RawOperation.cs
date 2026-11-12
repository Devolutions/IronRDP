using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct Operation
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Operation_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(Operation* handle);
}