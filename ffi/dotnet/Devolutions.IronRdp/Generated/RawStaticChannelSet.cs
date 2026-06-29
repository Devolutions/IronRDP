using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct StaticChannelSet
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "StaticChannelSet_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(StaticChannelSet* handle);
}