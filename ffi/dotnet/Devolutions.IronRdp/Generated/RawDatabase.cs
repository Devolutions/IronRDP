// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp.Raw;

#nullable enable

[StructLayout(LayoutKind.Sequential)]
public partial struct Database
{
    private const string NativeLib = "DevolutionsIronRdp";

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "Database_new", ExactSpelling = true)]
    public static unsafe extern Database* New();

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "Database_destroy", ExactSpelling = true)]
    public static unsafe extern void Destroy(Database* self);
}
