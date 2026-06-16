using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct MouseButton
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MouseButton_new", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern MouseButton* New(MouseButtonType button);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MouseButton_as_operation_mouse_button_pressed", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern Operation* AsOperationMouseButtonPressed(MouseButton* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MouseButton_as_operation_mouse_button_released", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern Operation* AsOperationMouseButtonReleased(MouseButton* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "MouseButton_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(MouseButton* handle);
}