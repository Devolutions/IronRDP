using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct LicenseExchangeSequence
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "LicenseExchangeSequence_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(LicenseExchangeSequence* handle);
}