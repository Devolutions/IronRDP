// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp.Raw;

#nullable enable

[StructLayout(LayoutKind.Sequential)]
public partial struct ClipboardMessage
{
    private const string NativeLib = "DevolutionsIronRdp";

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ClipboardMessage_get_enum_type", ExactSpelling = true)]
    public static unsafe extern ClipboardMessageType GetEnumType(ClipboardMessage* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ClipboardMessage_get_send_initiate_copy", ExactSpelling = true)]
    public static unsafe extern ClipboardFormatIterator* GetSendInitiateCopy(ClipboardMessage* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ClipboardMessage_get_send_format_data", ExactSpelling = true)]
    public static unsafe extern OwndFormatDataResponse* GetSendFormatData(ClipboardMessage* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ClipboardMessage_get_send_initiate_paste", ExactSpelling = true)]
    public static unsafe extern ClipboardFormatId* GetSendInitiatePaste(ClipboardMessage* self);

    [DllImport(NativeLib, CallingConvention = CallingConvention.Cdecl, EntryPoint = "ClipboardMessage_destroy", ExactSpelling = true)]
    public static unsafe extern void Destroy(ClipboardMessage* self);
}
