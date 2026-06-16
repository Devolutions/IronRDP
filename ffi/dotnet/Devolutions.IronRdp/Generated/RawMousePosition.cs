using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct MousePosition
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MousePosition_new", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern MousePosition* New(ushort x, ushort y);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MousePosition_as_move_operation", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern Operation* AsMoveOperation(MousePosition* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MousePosition_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(MousePosition* handle);
}