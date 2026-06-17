using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct Scancode
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Scancode_from_u8", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern Scancode* FromU8([MarshalAs(UnmanagedType.U1)] bool extended, byte code);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Scancode_from_u16", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern Scancode* FromU16(ushort code);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Scancode_as_operation_key_pressed", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern Operation* AsOperationKeyPressed(Scancode* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Scancode_as_operation_key_released", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern Operation* AsOperationKeyReleased(Scancode* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Scancode_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(Scancode* handle);
}