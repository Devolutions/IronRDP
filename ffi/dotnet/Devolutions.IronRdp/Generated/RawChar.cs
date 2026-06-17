using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct Char
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Char_new", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultCharIronRdpError New(uint c);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Char_as_operation_unicode_key_pressed", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern Operation* AsOperationUnicodeKeyPressed(Char* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Char_as_operation_unicode_key_released", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern Operation* AsOperationUnicodeKeyReleased(Char* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Char_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(Char* handle);
}