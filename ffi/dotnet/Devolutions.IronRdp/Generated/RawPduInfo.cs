using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct PduInfo
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PduInfo_get_action", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern Action* GetAction(PduInfo* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PduInfo_get_length", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern nuint GetLength(PduInfo* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "PduInfo_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(PduInfo* handle);
}