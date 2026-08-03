using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ConnectionActivationStateConnectionFinalization
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationStateConnectionFinalization_get_desktop_size", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DesktopSize* GetDesktopSize(ConnectionActivationStateConnectionFinalization* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationStateConnectionFinalization_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ConnectionActivationStateConnectionFinalization* handle);
}