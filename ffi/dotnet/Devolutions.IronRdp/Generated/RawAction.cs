using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct Action
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Action_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(Action* handle);
}