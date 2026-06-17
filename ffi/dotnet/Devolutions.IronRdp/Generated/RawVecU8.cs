using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct VecU8
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "VecU8_from_bytes", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern VecU8* FromBytes(DiplomatSliceU8 bytes);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "VecU8_get_size", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern nuint GetSize(VecU8* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "VecU8_fill", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVoidIronRdpError Fill(VecU8* handle, DiplomatSliceMutU8 buffer);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "VecU8_new_empty", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern VecU8* NewEmpty();

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "VecU8_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(VecU8* handle);
}