using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct WriteBuf
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "WriteBuf_new", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern WriteBuf* New();

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "WriteBuf_clear", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void Clear(WriteBuf* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "WriteBuf_read_into_buf", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVoidIronRdpError ReadIntoBuf(WriteBuf* handle, DiplomatSliceMutU8 buf);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "WriteBuf_get_filled", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern VecU8* GetFilled(WriteBuf* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "WriteBuf_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(WriteBuf* handle);
}