using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct Written
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Written_get_written_type", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern WrittenType GetWrittenType(Written* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Written_get_size", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern OptionalUsize* GetSize(Written* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Written_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(Written* handle);
}