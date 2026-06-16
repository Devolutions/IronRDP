using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct U32Slice
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "U32Slice_get_size", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern nuint GetSize(U32Slice* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "U32Slice_fill", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVoidIronRdpError Fill(U32Slice* handle, DiplomatSliceMutU32 buffer);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "U32Slice_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(U32Slice* handle);
}