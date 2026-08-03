using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct KerberosConfig
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "KerberosConfig_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(KerberosConfig* handle);
}