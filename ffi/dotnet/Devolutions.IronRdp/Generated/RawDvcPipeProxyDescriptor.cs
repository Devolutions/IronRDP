using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct DvcPipeProxyDescriptor
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DvcPipeProxyDescriptor_new", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DvcPipeProxyDescriptor* New(DiplomatSliceU8 channelName, DiplomatSliceU8 pipeName);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DvcPipeProxyDescriptor_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(DvcPipeProxyDescriptor* handle);
}