using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ActiveStageOutputIterator
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutputIterator_len", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern nuint Len(ActiveStageOutputIterator* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutputIterator_is_empty", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool IsEmpty(ActiveStageOutputIterator* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutputIterator_next", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ActiveStageOutput* Next(ActiveStageOutputIterator* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutputIterator_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ActiveStageOutputIterator* handle);
}