using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ChannelConnectionSequence
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ChannelConnectionSequence_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ChannelConnectionSequence* handle);
}