using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct TsRequest
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "TsRequest_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(TsRequest* handle);
}