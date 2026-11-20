using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct KerberosConfig
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "KerberosConfig_new", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultKerberosConfigIronRdpError New(DiplomatSliceU8 hostname);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "KerberosConfig_new_with_kdc_url", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultKerberosConfigIronRdpError NewWithKdcUrl(DiplomatSliceU8 kdcUrl, DiplomatSliceU8 hostname);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "KerberosConfig_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(KerberosConfig* handle);
}