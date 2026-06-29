using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ClipboardMessage
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClipboardMessage_get_message_type", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ClipboardMessageType GetMessageType(ClipboardMessage* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClipboardMessage_get_send_initiate_copy", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ClipboardFormatIterator* GetSendInitiateCopy(ClipboardMessage* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClipboardMessage_get_send_format_data", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern FormatDataResponse* GetSendFormatData(ClipboardMessage* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClipboardMessage_get_send_initiate_paste", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ClipboardFormatId* GetSendInitiatePaste(ClipboardMessage* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClipboardMessage_get_send_file_contents_request", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern FfiFileContentsRequest* GetSendFileContentsRequest(ClipboardMessage* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClipboardMessage_get_send_file_contents_response", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern FfiFileContentsResponse* GetSendFileContentsResponse(ClipboardMessage* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClipboardMessage_get_error", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern IronRdpError* GetError(ClipboardMessage* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ClipboardMessage_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ClipboardMessage* handle);
}