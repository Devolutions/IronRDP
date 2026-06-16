using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct IronRdpPdu
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "IronRdpPdu_new", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern IronRdpPdu* New();

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "IronRdpPdu_find_size", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultPduInfoIronRdpError FindSize(IronRdpPdu* handle, DiplomatSliceU8 bytes);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "IronRdpPdu_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(IronRdpPdu* handle);
}