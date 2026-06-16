using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct CliprdrBackendFactory
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CliprdrBackendFactory_build_cliprdr", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern Cliprdr* BuildCliprdr(CliprdrBackendFactory* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CliprdrBackendFactory_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(CliprdrBackendFactory* handle);
}