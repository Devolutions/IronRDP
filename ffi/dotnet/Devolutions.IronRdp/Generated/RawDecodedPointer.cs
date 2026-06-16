using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct DecodedPointer
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DecodedPointer_get_width", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetWidth(DecodedPointer* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DecodedPointer_get_height", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetHeight(DecodedPointer* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DecodedPointer_get_hotspot_x", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetHotspotX(DecodedPointer* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DecodedPointer_get_hotspot_y", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ushort GetHotspotY(DecodedPointer* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DecodedPointer_get_data", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern BytesSlice* GetData(DecodedPointer* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DecodedPointer_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(DecodedPointer* handle);
}