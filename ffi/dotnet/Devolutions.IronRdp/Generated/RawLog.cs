using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct Log
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Log_init_with_env", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void InitWithEnv();

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Log_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(Log* handle);
}