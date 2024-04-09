// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp.Raw;

#nullable enable

[StructLayout(LayoutKind.Sequential)]
public partial struct PduInfo
{
    private const string NativeLib = "DevolutionsIronRdp";

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "PduInfo_get_action", ExactSpelling = true)]
    public static unsafe extern Action* GetAction(PduInfo* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "PduInfo_get_length", ExactSpelling = true)]
    public static unsafe extern nuint GetLength(PduInfo* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "PduInfo_destroy", ExactSpelling = true)]
    public static unsafe extern void Destroy(PduInfo* self);
}