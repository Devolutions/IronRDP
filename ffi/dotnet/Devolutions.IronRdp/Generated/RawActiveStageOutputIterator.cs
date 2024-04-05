// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp.Raw;

#nullable enable

[StructLayout(LayoutKind.Sequential)]
public partial struct ActiveStageOutputIterator
{
    private const string NativeLib = "DevolutionsIronRdp";

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ActiveStageOutputIterator_len", ExactSpelling = true)]
    public static unsafe extern nuint Len(ActiveStageOutputIterator* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ActiveStageOutputIterator_next", ExactSpelling = true)]
    public static unsafe extern ActiveStageOutput* Next(ActiveStageOutputIterator* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ActiveStageOutputIterator_destroy", ExactSpelling = true)]
    public static unsafe extern void Destroy(ActiveStageOutputIterator* self);
}
