using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct DecodedImage
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DecodedImage_new", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DecodedImage* New(PixelFormat pixelFormat, ushort width, ushort height);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DecodedImage_get_data_size", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern nuint GetDataSize(DecodedImage* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DecodedImage_fill_data", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultVoidIronRdpError FillData(DecodedImage* handle, DiplomatSliceMutU8 buffer);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DecodedImage_get_width", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern ushort GetWidth(DecodedImage* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DecodedImage_get_height", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern ushort GetHeight(DecodedImage* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "DecodedImage_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(DecodedImage* handle);
}