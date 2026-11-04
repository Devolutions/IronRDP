using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ActiveStageOutput
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutput_get_enum_type", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern ActiveStageOutputType GetEnumType(ActiveStageOutput* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutput_get_response_frame", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultVecU8IronRdpError GetResponseFrame(ActiveStageOutput* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutput_get_graphics_update", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultInclusiveRectangleIronRdpError GetGraphicsUpdate(ActiveStageOutput* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutput_get_pointer_position", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultPositionIronRdpError GetPointerPosition(ActiveStageOutput* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutput_get_pointer_bitmap", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultDecodedPointerIronRdpError GetPointerBitmap(ActiveStageOutput* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutput_get_windowing_orders", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultVecU8IronRdpError GetWindowingOrders(ActiveStageOutput* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutput_get_monitor_layout", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultMonitorLayoutIteratorIronRdpError GetMonitorLayout(ActiveStageOutput* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutput_get_session_id", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultUIntIronRdpError GetSessionId(ActiveStageOutput* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutput_get_server_redirection", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultServerRedirectionIronRdpError GetServerRedirection(ActiveStageOutput* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutput_get_terminate", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultGracefulDisconnectReasonIronRdpError GetTerminate(ActiveStageOutput* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutput_get_multitransport_request", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultMultitransportRequestIronRdpError GetMultitransportRequest(ActiveStageOutput* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutput_get_autodetect_network_characteristics", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern DiplomatResultNetworkCharacteristicsIronRdpError GetAutodetectNetworkCharacteristics(ActiveStageOutput* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ActiveStageOutput_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ActiveStageOutput* handle);
}