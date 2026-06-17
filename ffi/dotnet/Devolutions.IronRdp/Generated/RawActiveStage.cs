using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ActiveStage
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStage_new", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultActiveStageIronRdpError New(ConnectionResult* connectionResult);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStage_process", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultActiveStageOutputIteratorIronRdpError Process(ActiveStage* handle, DecodedImage* image, Action* action, DiplomatSliceU8 payload);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStage_process_fastpath_input", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultActiveStageOutputIteratorIronRdpError ProcessFastpathInput(ActiveStage* handle, DecodedImage* image, FastPathInputEventIterator* fastpathInput);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStage_initiate_clipboard_copy", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVecU8IronRdpError InitiateClipboardCopy(ActiveStage* handle, ClipboardFormatIterator* formats);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStage_initiate_clipboard_paste", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVecU8IronRdpError InitiateClipboardPaste(ActiveStage* handle, ClipboardFormatId* formatId);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStage_submit_clipboard_format_data", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVecU8IronRdpError SubmitClipboardFormatData(ActiveStage* handle, FormatDataResponse* formatDataResponse);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStage_send_dvc_pipe_proxy_message", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultVecU8IronRdpError SendDvcPipeProxyMessage(ActiveStage* handle, DvcPipeProxyMessage* message);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStage_graceful_shutdown", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultActiveStageOutputIteratorIronRdpError GracefulShutdown(ActiveStage* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStage_encoded_resize", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultActiveStageOutputIteratorIronRdpError EncodedResize(ActiveStage* handle, uint width, uint height);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStage_set_fastpath_processor", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetFastpathProcessor(ActiveStage* handle, ushort ioChannelId, ushort userChannelId, uint shareId, [MarshalAs(UnmanagedType.U1)] bool enableServerPointer, [MarshalAs(UnmanagedType.U1)] bool pointerSoftwareRendering);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStage_set_enable_server_pointer", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetEnableServerPointer(ActiveStage* handle, [MarshalAs(UnmanagedType.U1)] bool enableServerPointer);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStage_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ActiveStage* handle);
}