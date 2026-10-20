using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ClipboardSvcMessage
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClipboardSvcMessage_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ClipboardSvcMessage* handle);
}