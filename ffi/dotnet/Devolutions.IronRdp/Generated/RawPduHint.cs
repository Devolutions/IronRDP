using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct PduHint
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PduHint_find_size", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultOptionalUsizeIronRdpError FindSize(PduHint* handle, DiplomatSliceU8 bytes);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PduHint_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(PduHint* handle);
}