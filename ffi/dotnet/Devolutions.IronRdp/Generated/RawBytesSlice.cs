using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct BytesSlice
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "BytesSlice_get_size", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern nuint GetSize(BytesSlice* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "BytesSlice_fill", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVoidIronRdpError Fill(BytesSlice* handle, DiplomatSliceMutU8 buffer);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "BytesSlice_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(BytesSlice* handle);
}