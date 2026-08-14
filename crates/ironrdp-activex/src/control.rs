use core::cell::{Cell, RefCell};
use core::ffi::c_void;
use core::mem::ManuallyDrop;
use core::ptr;
use core::slice;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex, mpsc as std_mpsc};
use std::time::Duration;

use ironrdp_cfg::{AudioMode, GatewayCredentialsSource, GatewayUsageMethod};
use ironrdp_client::config::{ClipboardType, ConfigBuilder, Destination, RDCleanPathConfig, Transport, TransportKind};
use ironrdp_client::rail::RailInputEvent;
use ironrdp_client::rdp::{
    AutoReconnectDecision, CliprdrBackendFactory, RdpClient, RdpInputEvent, RdpInputSender, RdpOutputEvent,
};
use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy};
use ironrdp_cliprdr_native::WinClipboard;
use ironrdp_connector::{ConnectorError, ConnectorErrorKind, Credentials};
use ironrdp_core::{DecodeError, DecodeErrorKind, ReadCursor, encode_vec};
use ironrdp_input::{Database as InputDatabase, MouseButton, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp_pdu::PduResult;
use ironrdp_pdu::gcc::{
    ChannelName, ChannelOptions, ClientMonitorData, ConnectionType, KeyboardType, Monitor, MonitorFlags,
};
use ironrdp_pdu::rdp::{
    capability_sets::{MajorPlatformType, RailSupportLevel},
    client_info::PerformanceFlags,
};
use ironrdp_pdu::window::try_decode_slow_path_windowing_orders;
use ironrdp_propertyset::PropertySet;
use ironrdp_rail::pdu::{ActivatePdu, ExecutePdu, RailPdu, SystemCommand, SystemCommandPdu};
use ironrdp_rdpei::pdu::{
    PenContact, PenContactDataFlags, PenContactFlags, PenEventPdu, PenFlags, PenFrame, TouchContact, TouchContactFlags,
    TouchEventPdu, TouchFrame,
};
use ironrdp_session::{GracefulDisconnectReason, SessionError, SessionErrorKind};
use ironrdp_svc::{SvcClientProcessor, SvcMessage, SvcProcessor, impl_as_any};
use ironrdp_tls::CertificateValidation;
use sha2::{Digest as _, Sha256};
use tokio::sync::{mpsc, oneshot};
use windows::Win32::Foundation::{
    DATA_S_SAMEFORMATETC, DISP_E_BADPARAMCOUNT, DISP_E_MEMBERNOTFOUND, DISP_E_TYPEMISMATCH, DISP_E_UNKNOWNNAME,
    DV_E_DVASPECT, DV_E_DVTARGETDEVICE, DV_E_FORMATETC, DV_E_LINDEX, DV_E_TYMED, E_FAIL, E_INVALIDARG, E_NOTIMPL,
    E_OUTOFMEMORY, E_POINTER, E_UNEXPECTED, ERROR_CANCELLED, ERROR_CLASS_DOES_NOT_EXIST, FreeLibrary, GlobalFree,
    HGLOBAL, HMODULE, HWND, LPARAM, LRESULT, OLE_E_ADVISENOTSUPPORTED, OLE_E_NOCONNECTION, OLE_E_NOTRUNNING,
    OLEOBJ_S_INVALIDVERB, POINT, RECT, RECTL, S_FALSE, S_OK, SIZE, SysStringLen, VARIANT_BOOL, VARIANT_FALSE,
    VARIANT_TRUE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLACKNESS, BeginPaint, CreateCompatibleDC, CreateDIBSection, CreateRectRgn,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, EndPaint, EnumDisplayMonitors, GdiFlush, GetMonitorInfoW, HBITMAP, HDC,
    HGDIOBJ, HMONITOR, InvalidateRect, MONITORINFO, PAINTSTRUCT, PatBlt, SRCCOPY, ScreenToClient, SelectObject,
    SetWindowRgn, StretchBlt, StretchDIBits,
};
use windows::Win32::Security::Credentials::{
    CREDUI_FLAGS_ALWAYS_SHOW_UI, CREDUI_FLAGS_DO_NOT_PERSIST, CREDUI_FLAGS_GENERIC_CREDENTIALS, CREDUI_INFOW,
    CredUIPromptForCredentialsW,
};
use windows::Win32::Storage::FileSystem::GetLogicalDrives;
use windows::Win32::System::Com::{
    CONNECTDATA, CoTaskMemAlloc, DATADIR_GET, DATADIR_SET, DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET,
    DISPATCH_PROPERTYPUT, DISPPARAMS, DVASPECT, DVASPECT_CONTENT, DVTARGETDEVICE, EXCEPINFO, FORMATETC, IAdviseSink,
    IConnectionPoint, IConnectionPoint_Impl, IConnectionPointContainer, IConnectionPointContainer_Impl, IDataObject,
    IDataObject_Impl, IDispatch, IDispatch_Impl, IDispatch_Vtbl, IEnumConnectionPoints, IEnumConnectionPoints_Impl,
    IEnumConnections, IEnumConnections_Impl, IEnumFORMATETC, IEnumFORMATETC_Impl, IEnumSTATDATA, IEnumSTATDATA_Impl,
    IPersist_Impl, IPersistStreamInit, IPersistStreamInit_Impl, IStream, ITypeInfo, STATDATA, STGMEDIUM, STGMEDIUM_0,
    TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_UNICODETEXT, CONTROLINFO, DVEXTENTINFO, HITRESULT_HIT, HITRESULT_OUTSIDE, IEnumOLEVERB, IEnumOLEVERB_Impl,
    IOleClientSite, IOleControl, IOleControl_Impl, IOleControlSite, IOleControlSite_Vtbl, IOleInPlaceActiveObject,
    IOleInPlaceActiveObject_Impl, IOleInPlaceObject, IOleInPlaceObject_Impl, IOleInPlaceSite, IOleInPlaceUIWindow,
    IOleObject, IOleObject_Impl, IOleWindow_Impl, IViewObject, IViewObject_Impl, IViewObject2, IViewObject2_Impl,
    IViewObjectEx, IViewObjectEx_Impl, KEYMODIFIERS, OLECLOSE, OLEGETMONIKER, OLEMISC, OLEVERB, OLEVERB_PRIMARY,
    OLEVERBATTRIB_NEVERDIRTIES, OLEWHICHMK, USERCLASSTYPE, VIEWSTATUS_OPAQUE, VIEWSTATUS_SOLIDBKGND,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_BINARY, REG_OPTION_NON_VOLATILE, RegCloseKey, RegCreateKeyExW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::Win32::System::Variant::{
    VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BOOL, VT_BSTR, VT_BYREF, VT_EMPTY, VT_I4, VT_UI4,
};
use windows::Win32::UI::Controls::{
    TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TDCBF_CANCEL_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION, TDF_SIZE_TO_CONTENT,
    WM_MOUSELEAVE,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, IsWindowEnabled, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VIRTUAL_KEY, VK_CANCEL, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_PAUSE, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::Input::Pointer::{
    GetPointerFrameTouchInfo, GetPointerInfo, POINTER_FLAG_CANCELED, POINTER_FLAG_INCONTACT, POINTER_FLAG_INRANGE,
    POINTER_FLAG_UP, POINTER_INFO, POINTER_TOUCH_INFO, SkipPointerFrameMessages,
};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_PUSHBUTTON, CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, EnumWindows, GA_ROOT, GA_ROOTOWNER,
    GWL_EXSTYLE, GWL_STYLE, GWLP_HWNDPARENT, GWLP_USERDATA, GetAncestor, GetClassNameW, GetClientRect, GetCursorPos,
    GetDlgItem, GetForegroundWindow, GetParent, GetWindowLongPtrW, GetWindowRect, GetWindowTextW,
    GetWindowThreadProcessId, HMENU, HWND_MESSAGE, IsIconic, IsWindow, IsWindowVisible, KillTimer, PT_TOUCH,
    PostMessageW, RegisterClassW, SC_MAXIMIZE, SC_MINIMIZE, SC_MOVE, SC_RESTORE, SC_SIZE, SIZE_MINIMIZED, SW_HIDE,
    SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SW_SHOWNA, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOZORDER,
    SendMessageW, SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, TOUCH_MASK_CONTACTAREA,
    TOUCH_MASK_ORIENTATION, TOUCH_MASK_PRESSURE, UnregisterClassW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_ACTIVATE, WM_APP,
    WM_CANCELMODE, WM_CAPTURECHANGED, WM_CLOSE, WM_COMMAND, WM_DPICHANGED, WM_ENABLE, WM_KEYDOWN, WM_KEYUP,
    WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_MOVE, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_POINTERCAPTURECHANGED, WM_POINTERDOWN,
    WM_POINTERLEAVE, WM_POINTERUP, WM_POINTERUPDATE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SHOWWINDOW, WM_SIZE,
    WM_SYSCOMMAND, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER, WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSW, WS_CHILD,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{s, w};
use windows_core::{
    BOOL as WinBool, BSTR, Error, GUID, HRESULT, HSTRING, IUnknown, IUnknown_Vtbl, IUnknownImpl as _, Interface as _,
    PCWSTR, Ref, Result, implement,
};

use crate::com;
use crate::mstsc::{
    Bstr, BstrOut, IMsRdpCameraRedirConfigCollection, IMsRdpCameraRedirConfigCollection_Impl, IMsRdpClient,
    IMsRdpClient_Impl, IMsRdpClient2, IMsRdpClient2_Impl, IMsRdpClient3, IMsRdpClient3_Impl, IMsRdpClient4,
    IMsRdpClient4_Impl, IMsRdpClient5, IMsRdpClient5_Impl, IMsRdpClient6, IMsRdpClient6_Impl, IMsRdpClient7,
    IMsRdpClient7_Impl, IMsRdpClient8, IMsRdpClient8_Impl, IMsRdpClient9, IMsRdpClient9_Impl, IMsRdpClient10,
    IMsRdpClient10_Impl, IMsRdpClientNonScriptable, IMsRdpClientNonScriptable_Impl, IMsRdpClientNonScriptable2,
    IMsRdpClientNonScriptable2_Impl, IMsRdpClientNonScriptable3, IMsRdpClientNonScriptable3_Impl,
    IMsRdpClientNonScriptable4, IMsRdpClientNonScriptable4_Impl, IMsRdpClientNonScriptable5,
    IMsRdpClientNonScriptable5_Impl, IMsRdpClientNonScriptable6, IMsRdpClientNonScriptable6_Impl,
    IMsRdpClientNonScriptable7, IMsRdpClientNonScriptable7_Impl, IMsRdpClientNonScriptable8,
    IMsRdpClientNonScriptable8_Impl, IMsRdpClipboard, IMsRdpClipboard_Impl, IMsRdpDeviceCollection,
    IMsRdpDeviceCollection_Impl, IMsRdpDrive, IMsRdpDrive_Impl, IMsRdpDriveCollection, IMsRdpDriveCollection_Impl,
    IMsRdpExtendedSettings, IMsRdpExtendedSettings_Impl, IMsRdpPreferredRedirectionInfo,
    IMsRdpPreferredRedirectionInfo_Impl, IMsTscAx_Impl, IMsTscAx_Redist_Impl, IMsTscNonScriptable,
    IMsTscNonScriptable_Impl, InterfaceOut,
};
use crate::rpc::{self, ActiveXRpc, Command as RpcCommand};
use crate::touch::{TouchContactTracker, TouchSample};
use ironrdp_rpc as ironrdp_agent;

/// The IronRDP-owned class identifier registered by this DLL.
pub(crate) const CLSID_IRONRDP_ACTIVEX: GUID = GUID::from_u128(0x5d3e_2b4c_6860_462e_8e9d_0c4d_2b09_4c5f);

const TSC_SHELL_CONTAINER_CLASS: &[u16] = &[
    0x0054, 0x0073, 0x0063, 0x0053, 0x0068, 0x0065, 0x006c, 0x006c, 0x0043, 0x006f, 0x006e, 0x0074, 0x0061, 0x0069,
    0x006e, 0x0065, 0x0072, 0x0043, 0x006c, 0x0061, 0x0073, 0x0073,
];
const DISPLAY_RESIZE_TIMER_ID: usize = 0x4952_4450;
const DISPLAY_RESIZE_DEBOUNCE_MILLISECONDS: u32 = 250;
const NATIVE_MSTSC_LAYOUT_TIMER_ID: usize = 0x4952_4451;
const NATIVE_MSTSC_LAYOUT_POLL_MILLISECONDS: u32 = 100;
const PROJECTED_RAIL_INPUT_RETRY_TIMER_ID: usize = 0x4952_4452;
const PROJECTED_RAIL_INPUT_RETRY_MILLISECONDS: u32 = 25;
const ACTIVEX_DVC_PLUGIN_PATHS_PROPERTY: &str = "IronRdpDvcPluginPaths";
const ACTIVEX_ENABLE_TLS_PROPERTY: &str = "IronRdpEnableTls";
const ACTIVEX_AUTOLOGON_PROPERTY: &str = "IronRdpAutoLogon";
const ACTIVEX_DESKTOP_SCALE_FACTOR_PROPERTY: &str = "IronRdpDesktopScaleFactor";
const ACTIVEX_COMPRESSION_LEVEL_PROPERTY: &str = "IronRdpCompressionLevel";
const ACTIVEX_CLIENT_BUILD_PROPERTY: &str = "IronRdpClientBuild";
const ACTIVEX_CLIENT_DIRECTORY_PROPERTY: &str = "IronRdpClientDirectory";
const ACTIVEX_IME_FILE_NAME_PROPERTY: &str = "IronRdpImeFileName";
const ACTIVEX_DIGITAL_PRODUCT_ID_PROPERTY: &str = "IronRdpDigitalProductId";
const ACTIVEX_FAKE_EVENTS_INTERVAL_PROPERTY: &str = "IronRdpFakeEventsIntervalMinutes";
const ACTIVEX_RDCLEANPATH_URL_PROPERTY: &str = "RDCleanPathUrl";
const ACTIVEX_RDCLEANPATH_TOKEN_PROPERTY: &str = "RDCleanPathToken";
const ACTIVEX_REMOTE_PROGRAM_MODE_PROPERTY: &str = "IronRdpRemoteProgramMode";
const ACTIVEX_REMOTE_APPLICATION_PROGRAM_PROPERTY: &str = "IronRdpRemoteApplicationProgram";
const ACTIVEX_REMOTE_APPLICATION_ARGS_PROPERTY: &str = "IronRdpRemoteApplicationArgs";
const MAX_ACTIVEX_EXTENDED_SETTING_STRING_BYTES: usize = 8 * 1024;
const ACTIVEX_DVC_PLUGIN_OPT_IN: &str = "IRONRDP_ACTIVEX_ENABLE_DVC_PLUGINS";
const MAX_ACTIVEX_DVC_PLUGINS: usize = 16;

#[derive(Default)]
struct RemoteApplicationConfiguration {
    enabled: bool,
    program: String,
    arguments: String,
}

fn configured_remote_application_execute(configuration: &RemoteApplicationConfiguration) -> Result<Option<ExecutePdu>> {
    if !configuration.enabled {
        return Ok(None);
    }
    if configuration.program.is_empty() {
        return Err(Error::new(
            E_INVALIDARG,
            "set IronRdpRemoteApplicationProgram before connecting in RemoteApp mode",
        ));
    }
    Ok(Some(ExecutePdu {
        flags: 0,
        executable: configuration.program.clone(),
        working_directory: String::new(),
        arguments: configuration.arguments.clone(),
    }))
}

fn validate_rail_execute(execute: &ExecutePdu) -> Result<()> {
    encode_vec(&RailPdu::Execute(execute.clone()))
        .map(|_| ())
        .map_err(|error| Error::new(E_INVALIDARG, error.to_string()))
}

fn rail_window_input_event(window_id: u32, message: u32, wparam: WPARAM) -> Option<RailInputEvent> {
    match message {
        WM_ACTIVATE => Some(RailInputEvent::Activate(ActivatePdu {
            window_id,
            enabled: wparam.0 & 0xffff != 0,
        })),
        WM_CLOSE => Some(RailInputEvent::SystemCommand(SystemCommandPdu {
            window_id,
            command: SystemCommand::Close,
        })),
        _ => None,
    }
}

fn is_unsupported_projected_rail_system_command(wparam: WPARAM) -> bool {
    let command = wparam.0 & 0xfff0;
    command == SC_MOVE as usize
        || command == SC_SIZE as usize
        || command == SC_MINIMIZE as usize
        || command == SC_MAXIMIZE as usize
        || command == SC_RESTORE as usize
}

pub(crate) const CLSID_MS_RDP_CLIENT: GUID = GUID::from_u128(0x791f_a017_2de3_492e_acc5_53c6_7a2b_94d0);
pub(crate) const CLSID_MS_RDP_CLIENT_6_NOT_SAFE_FOR_SCRIPTING: GUID =
    GUID::from_u128(0xd2ea_46a7_c2bf_426b_af24_e19c_4445_6399);
pub(crate) const CLSID_MS_RDP_CLIENT_7_NOT_SAFE_FOR_SCRIPTING: GUID =
    GUID::from_u128(0x54d3_8bf7_b1ef_4479_9674_1bd6_ea46_5258);
pub(crate) const CLSID_MS_RDP_CLIENT_8_NOT_SAFE_FOR_SCRIPTING: GUID =
    GUID::from_u128(0xa3bc_03a0_041d_42e3_ad22_882b_7865_c9c5);
pub(crate) const CLSID_MS_RDP_CLIENT_9_NOT_SAFE_FOR_SCRIPTING: GUID =
    GUID::from_u128(0x8b91_8b82_7985_4c24_89df_c33a_d2bb_fbcd);
const CLSID_MS_RDP_CLIENT_10: GUID = GUID::from_u128(0xc0ef_a91a_eeb7_41c7_97fa_f0ed_645e_fb24);
pub(crate) const CLSID_MS_RDP_CLIENT_10_NOT_SAFE_FOR_SCRIPTING: GUID =
    GUID::from_u128(0xa0c6_3c30_f08d_4ab4_907c_3490_5d77_0c7d);
pub(crate) const CLSID_MS_RDP_CLIENT_11_NOT_SAFE_FOR_SCRIPTING: GUID =
    GUID::from_u128(0x1df7_c823_b2d4_4b54_975a_f2ac_5d7c_f8b8);

pub(crate) const RDM_COMPATIBILITY_CLSIDS: &[GUID] = &[
    CLSID_MS_RDP_CLIENT_6_NOT_SAFE_FOR_SCRIPTING,
    CLSID_MS_RDP_CLIENT_7_NOT_SAFE_FOR_SCRIPTING,
    CLSID_MS_RDP_CLIENT_8_NOT_SAFE_FOR_SCRIPTING,
    CLSID_MS_RDP_CLIENT_9_NOT_SAFE_FOR_SCRIPTING,
    CLSID_MS_RDP_CLIENT_10_NOT_SAFE_FOR_SCRIPTING,
    CLSID_MS_RDP_CLIENT_11_NOT_SAFE_FOR_SCRIPTING,
];

const COMPATIBILITY_CLSIDS: &[GUID] = &[
    GUID::from_u128(0xa41a_4187_5a86_4e26_b40a_856f_9035_d9cb),
    GUID::from_u128(0x1fb4_64c8_09bb_4017_a2f5_eb74_2f04_392f),
    GUID::from_u128(0x7cac_bd7b_0d99_468f_ac33_22e4_95c0_afe5),
    CLSID_MS_RDP_CLIENT,
    GUID::from_u128(0x3523_c2fb_4031_44e4_9a3b_f1e9_4986_ee7f),
    GUID::from_u128(0x9059_f30f_4eb1_4bd2_9fdc_36f4_3a21_8f4a),
    GUID::from_u128(0x9711_27bb_259f_48c2_bd75_5f97_a333_1551),
    GUID::from_u128(0xace5_75fd_1fcf_4074_9401_ebab_990f_a9de),
    GUID::from_u128(0x7584_c670_2274_4efb_b00b_d6aa_ba6d_3850),
    GUID::from_u128(0x6a6f_4b83_45c5_4ca9_bdd9_0d81_c122_95e4),
    GUID::from_u128(0x6ae2_9350_321b_42be_bbe5_12fb_5270_c0de),
    GUID::from_u128(0x4edc_b26c_d24c_4e72_af07_b576_699a_c0de),
    GUID::from_u128(0x54ce_37e0_9834_41ae_9896_4dab_69dc_022b),
    GUID::from_u128(0x4eb2_f086_c818_447e_b32c_c51c_e2b3_0d31),
    GUID::from_u128(0x4eb8_9ff4_7f78_4a0f_8b8d_2bf0_2e94_e4b2),
    GUID::from_u128(0x7390_f3d8_0439_4c05_91e3_cf5c_b290_c3d0),
    GUID::from_u128(0xa9d7_038d_b5ed_472e_9c47_94be_a90a_5910),
    GUID::from_u128(0x5f68_1803_2900_4c43_a1cc_cf40_5404_a676),
    GUID::from_u128(0x301b_94ba_5d25_4a12_bffe_3b6e_7a61_6585),
    CLSID_MS_RDP_CLIENT_10,
];

const IID_MSTSCLIB_EVENTS: GUID = GUID::from_u128(0x336d_5562_efa8_482e_8cb3_c5c0_fc7a_7db6);

const OLEIVERB_SHOW: i32 = -1;
const OLEIVERB_OPEN: i32 = -2;
const OLEIVERB_HIDE: i32 = -3;
const OLEIVERB_UIACTIVATE: i32 = -4;
const OLEIVERB_INPLACEACTIVATE: i32 = -5;
const OLEIVERB_DISCARDUNDOSTATE: i32 = -6;
const OLEIVERB_PROPERTIES: i32 = -7;
const KEYMOD_SHIFT: u32 = 0x01;
const KEYMOD_CONTROL: u32 = 0x02;
const KEYMOD_ALT: u32 = 0x04;
const CONNECTION_BAR_BUTTON_IDS: &[usize] = &[
    CONNECTION_BAR_PIN_BUTTON_ID,
    CONNECTION_BAR_INFORMATION_BUTTON_ID,
    CONNECTION_BAR_MINIMIZE_BUTTON_ID,
    CONNECTION_BAR_RESTORE_BUTTON_ID,
    CONNECTION_BAR_FULLSCREEN_BUTTON_ID,
    CONNECTION_BAR_CLOSE_BUTTON_ID,
    CONNECTION_BAR_DISCONNECT_BUTTON_ID,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OleVerbAction {
    Activate,
    Hide,
    DiscardUndoState,
}

fn ole_verb_action(verb: i32) -> Result<OleVerbAction> {
    match verb {
        verb if verb == OLEVERB_PRIMARY as i32
            || matches!(
                verb,
                OLEIVERB_SHOW | OLEIVERB_OPEN | OLEIVERB_UIACTIVATE | OLEIVERB_INPLACEACTIVATE
            ) =>
        {
            Ok(OleVerbAction::Activate)
        }
        OLEIVERB_HIDE => Ok(OleVerbAction::Hide),
        OLEIVERB_DISCARDUNDOSTATE => Ok(OleVerbAction::DiscardUndoState),
        OLEIVERB_PROPERTIES => Err(Error::from_hresult(OLEOBJ_S_INVALIDVERB)),
        _ => Err(Error::from_hresult(OLEOBJ_S_INVALIDVERB)),
    }
}

fn active_key_modifiers() -> u32 {
    let mut modifiers = 0;
    if unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0 {
        modifiers |= KEYMOD_SHIFT;
    }
    if unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0 {
        modifiers |= KEYMOD_CONTROL;
    }
    if unsafe { GetKeyState(i32::from(VK_MENU.0)) } < 0 {
        modifiers |= KEYMOD_ALT;
    }
    modifiers
}

fn translate_control_site_accelerator(
    site: &IOleControlSite,
    message: *const windows::Win32::UI::WindowsAndMessaging::MSG,
    modifiers: u32,
) -> HRESULT {
    // The projected method maps S_FALSE to Ok(()), but OLE requires that an unhandled
    // accelerator remain distinguishable from S_OK for the container.
    let vtable = unsafe { *(site.as_raw() as *const *const IOleControlSite_Vtbl) };
    unsafe { ((*vtable).TranslateAccelerator)(site.as_raw(), message, KEYMODIFIERS(modifiers)) }
}

const DISPID_SERVER: i32 = 1;
const DISPID_DOMAIN: i32 = 2;
const DISPID_USERNAME: i32 = 3;
const DISPID_DISCONNECTED_TEXT: i32 = 4;
const DISPID_CONNECTING_TEXT: i32 = 5;
const DISPID_CONNECTED: i32 = 6;
const DISPID_DESKTOP_WIDTH: i32 = 12;
const DISPID_DESKTOP_HEIGHT: i32 = 13;
const DISPID_START_CONNECTED: i32 = 16;
const DISPID_HORIZONTAL_SCROLLBAR_VISIBLE: i32 = 17;
const DISPID_VERTICAL_SCROLLBAR_VISIBLE: i32 = 18;
const DISPID_FULLSCREEN_TITLE: i32 = 19;
const DISPID_CIPHER_STRENGTH: i32 = 20;
const DISPID_VERSION: i32 = 21;
const DISPID_SECURED_SETTINGS_ENABLED: i32 = 22;
const DISPID_CONNECT: i32 = 30;
const DISPID_DISCONNECT: i32 = 31;
const DISPID_COLOR_DEPTH: i32 = 100;
const DISPID_EXTENDED_DISCONNECT_REASON: i32 = 103;
const DISPID_FULLSCREEN: i32 = 104;
const DISPID_CONNECTED_STATUS_TEXT: i32 = 201;
const DISPID_IRONRDP_PASSWORD: i32 = 0x10000;
const DISPID_PROPERTYPUT: i32 = -3;
const REMOTE_SESSION_ACTION_CHARMS: i32 = 0;
const REMOTE_SESSION_ACTION_APPBAR: i32 = 1;
const REMOTE_SESSION_ACTION_SNAP: i32 = 2;
const REMOTE_SESSION_ACTION_START_SCREEN: i32 = 3;
const REMOTE_SESSION_ACTION_APP_SWITCH: i32 = 4;
const REMOTE_SESSION_ACTION_ACTION_CENTER: i32 = 5;
const REMOTE_SESSION_ACTION_TASK_MANAGER: i32 = 6;
const REMOTE_ACTION_MODIFIERS: &[Scancode] = &[
    Scancode::from_u8(false, 0x1d), // Left Ctrl
    Scancode::from_u8(true, 0x1d),  // Right Ctrl
    Scancode::from_u8(false, 0x2a), // Left Shift
    Scancode::from_u8(false, 0x36), // Right Shift
    Scancode::from_u8(false, 0x38), // Left Alt
    Scancode::from_u8(true, 0x38),  // Right Alt
    Scancode::from_u8(true, 0x5b),  // Left Win
    Scancode::from_u8(true, 0x5c),  // Right Win
];

const DISPID_ON_CONNECTING: i32 = 1;
const DISPID_ON_CONNECTED: i32 = 2;
const DISPID_ON_LOGIN_COMPLETE: i32 = 3;
const DISPID_ON_DISCONNECTED: i32 = 4;
const DISPID_ON_ENTER_FULL_SCREEN_MODE: i32 = 5;
const DISPID_ON_LEAVE_FULL_SCREEN_MODE: i32 = 6;
const DISPID_ON_CHANNEL_RECEIVED_DATA: i32 = 7;
const DISPID_ON_REQUEST_GO_FULL_SCREEN: i32 = 8;
const DISPID_ON_REQUEST_LEAVE_FULL_SCREEN: i32 = 9;
const DISPID_ON_FATAL_ERROR: i32 = 10;
const DISPID_ON_REMOTE_DESKTOP_SIZE_CHANGE: i32 = 12;
const DISPID_ON_CONFIRM_CLOSE: i32 = 15;
const DISPID_ON_AUTO_RECONNECTING: i32 = 17;
const DISPID_ON_AUTHENTICATION_WARNING_DISPLAYED: i32 = 18;
const DISPID_ON_AUTHENTICATION_WARNING_DISMISSED: i32 = 19;
const DISPID_ON_AUTO_RECONNECTED: i32 = 33;
const DISPID_ON_AUTO_RECONNECTING2: i32 = 34;

const WM_DISPATCH_EVENTS: u32 = WM_APP + 0x52;
const WM_DESTROY_CONTROL_WINDOW: u32 = WM_APP + 0x53;
const WM_UPDATE_CONNECTION_BAR: u32 = WM_APP + 0x54;
const CONNECTION_BAR_AUTO_HIDE_TIMER_ID: usize = 0x4952_4452;
const CONNECTION_BAR_AUTO_HIDE_MILLISECONDS: u32 = 3_000;
const CONNECTION_BAR_OWNER_LAYOUT_TIMER_ID: usize = 0x4952_4453;
const CONNECTION_BAR_OWNER_LAYOUT_POLL_MILLISECONDS: u32 = 250;
const CONNECTION_HEALTH_OWNER_LAYOUT_TIMER_ID: usize = 0x4952_4454;
const CONNECTION_HEALTH_OWNER_LAYOUT_POLL_MILLISECONDS: u32 = 250;
const CONNECTION_BAR_PIN_BUTTON_ID: usize = 1;
const CONNECTION_BAR_DISCONNECT_BUTTON_ID: usize = 2;
const CONNECTION_BAR_MINIMIZE_BUTTON_ID: usize = 3;
const CONNECTION_BAR_RESTORE_BUTTON_ID: usize = 4;
const CONNECTION_BAR_FULLSCREEN_BUTTON_ID: usize = 5;
const CONNECTION_BAR_CLOSE_BUTTON_ID: usize = 6;
const CONNECTION_BAR_INFORMATION_BUTTON_ID: usize = 7;
const CONNECTION_BAR_TITLE_ID: usize = 8;
const CONNECTION_BAR_WIDTH: i32 = 800;
const CONNECTION_BAR_HEIGHT: i32 = 36;
const CONNECTION_HEALTH_LABEL_ID: usize = 1;
const CONNECTION_HEALTH_ATTEMPT_ID: usize = 2;
const CONNECTION_HEALTH_WIDTH: i32 = 280;
const CONNECTION_HEALTH_HEIGHT: i32 = 76;
const DEFAULT_DPI: u32 = 96;
const DISPID_ON_CONNECTION_BAR_PULL_DOWN: i32 = 0x1e;
const CONNECT_E_CANNOTCONNECT: HRESULT = HRESULT(-2_147_220_990);
const CONNECT_E_NOCONNECTION: HRESULT = HRESULT(-2_147_220_992);
const CREDUI_MAX_USERNAME_LENGTH: usize = 513;
const CREDUI_MAX_PASSWORD_LENGTH: usize = 256;
const MSTSC_SEND_KEYS_MAX_KEYS: usize = 20;
const CONTROL_RECONNECT_STARTED: i32 = 0;
const CONTROL_RECONNECT_BLOCKED: i32 = 1;
const CONTROL_CLOSE_CAN_PROCEED: i32 = 0;
const CONTROL_CLOSE_WAIT_FOR_EVENTS: i32 = 1;
const MAX_ACTIVEX_STATIC_CHANNELS: usize = 28;
const MAX_RECONNECT_ATTEMPTS: u32 = 200;
const MAX_PENDING_WORKER_EVENTS: usize = 64;
const CERTIFICATE_WARNING_CONTINUE_BUTTON: i32 = 100;
const SECURITY_WARNING_CONTINUE_BUTTON: i32 = 101;
const INFORMATION_DIALOG_CLOSE_BUTTON: i32 = 102;
const CONNECTION_BAR_DISCONNECT_BUTTON: i32 = 103;
const CERTIFICATE_WARNING_TIMEOUT: Duration = Duration::from_secs(120);
const CERTIFICATE_EXCEPTION_REGISTRY_ROOT: &str = "Software\\Devolutions\\IronRDP\\ActiveX\\TrustedCertificates";
const EXTENDED_DISCONNECT_REASON_NO_INFO: i32 = 0;
const EXTENDED_DISCONNECT_REASON_API_INITIATED_DISCONNECT: i32 = 1;
const ACTIVEX_CODEC_CONFIGURATION: &[&str] = &["remotefx:off"];
const ACTIVEX_LOSSY_COMPRESSION: bool = false;
// The observed native mstsc connection dialog exposes its Computer combo box under this dialog ID.
const MSTSC_COMPUTER_FIELD_ID: i32 = 5012;

struct MstscComputerFieldSearch {
    process_id: u32,
    field: HWND,
}

unsafe extern "system" fn find_mstsc_computer_field(window: HWND, context: LPARAM) -> WinBool {
    let search = unsafe { &mut *(context.0 as *mut MstscComputerFieldSearch) };
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
    if process_id != search.process_id {
        return WinBool(1);
    }

    let field = unsafe { GetDlgItem(Some(window), MSTSC_COMPUTER_FIELD_ID).ok() };
    if let Some(field) = field.filter(|field| !field.0.is_null()) {
        search.field = field;
        WinBool(0)
    } else {
        WinBool(1)
    }
}

pub(crate) fn is_supported_class(clsid: &GUID) -> bool {
    *clsid == CLSID_IRONRDP_ACTIVEX || COMPATIBILITY_CLSIDS.contains(clsid) || RDM_COMPATIBILITY_CLSIDS.contains(clsid)
}

#[cfg(test)]
std::thread_local! {
    static TEST_HOST_TRACE_PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

#[cfg(test)]
struct TestHostTracePath {
    previous: Option<PathBuf>,
}

#[cfg(test)]
impl TestHostTracePath {
    fn install(path: PathBuf) -> Self {
        let previous = TEST_HOST_TRACE_PATH.with(|trace_path| trace_path.replace(Some(path)));
        Self { previous }
    }
}

#[cfg(test)]
impl Drop for TestHostTracePath {
    fn drop(&mut self) {
        TEST_HOST_TRACE_PATH.with(|trace_path| {
            trace_path.replace(self.previous.take());
        });
    }
}

fn append_host_trace(path: impl AsRef<Path>, name: &str) {
    // Host startup may carry credentials in Automation values, so log method names only.
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = std::io::Write::write_all(&mut file, format!("{name}\n").as_bytes());
    }
}

fn trace_host_call(name: &str) {
    #[cfg(test)]
    if let Some(path) = TEST_HOST_TRACE_PATH.with(|trace_path| trace_path.borrow().clone()) {
        append_host_trace(path, name);
        return;
    }

    let Ok(path) = std::env::var("IRONRDP_ACTIVEX_HOST_TRACE") else {
        return;
    };

    append_host_trace(path, name);
}

fn environment_flag_enabled(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| value == "1")
}

fn credential_prompt_buffer(value: &str, capacity: usize) -> Vec<u16> {
    let mut buffer = value
        .encode_utf16()
        .take(capacity.saturating_sub(1))
        .collect::<Vec<_>>();
    buffer.resize(capacity, 0);
    buffer
}

fn native_mstsc_credential_bridge_enabled() -> bool {
    environment_flag_enabled("IRONRDP_ACTIVEX_NATIVE_MSTSC_CREDENTIAL_BRIDGE")
}

fn native_mstsc_autologon_enabled() -> bool {
    environment_flag_enabled("RDP_AUTOLOGON")
}

fn autologon_credentials(username: Option<String>, password: Option<String>) -> Option<(String, String)> {
    let username = username.filter(|value| !value.is_empty())?;
    let password = password.filter(|value| !value.is_empty())?;
    Some((username, password))
}

fn activex_dvc_plugins_enabled() -> bool {
    environment_flag_enabled(ACTIVEX_DVC_PLUGIN_OPT_IN)
}

fn validated_dvc_plugin_paths(value: &str) -> Result<Vec<PathBuf>> {
    if value.is_empty() {
        return Ok(Vec::new());
    }

    let paths = value.split(';').collect::<Vec<_>>();
    if paths.len() > MAX_ACTIVEX_DVC_PLUGINS || paths.iter().any(|path| path.is_empty()) {
        return Err(Error::from_hresult(E_INVALIDARG));
    }

    let mut validated = Vec::with_capacity(paths.len());
    for path in paths {
        let path = PathBuf::from(path);
        let path_text = path.to_string_lossy();
        let is_unc_path = (path_text.starts_with(r"\\") && !path_text.starts_with("\\\\?\\"))
            || path_text.starts_with("\\\\?\\UNC\\");
        if !path.is_absolute() || is_unc_path {
            return Err(Error::new(
                E_INVALIDARG,
                "DVC plugin paths must name a local absolute file",
            ));
        }

        let path = path
            .canonicalize()
            .map_err(|_| Error::new(E_INVALIDARG, "DVC plugin path must name an existing local file"))?;
        if !path.is_file()
            || !path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
        {
            return Err(Error::new(E_INVALIDARG, "DVC plugin path must name a DLL file"));
        }
        if validated.contains(&path) {
            return Err(Error::new(E_INVALIDARG, "DVC plugin paths must be unique"));
        }
        validated.push(path);
    }

    Ok(validated)
}

const fn should_intercept_native_mstsc_start_program(bridge_enabled: bool, disconnected: bool) -> bool {
    bridge_enabled && disconnected
}

fn native_mstsc_shell_integration_enabled() -> bool {
    native_mstsc_credential_bridge_enabled()
}

const fn certificate_validation_from_authentication_level(
    authentication_level: u32,
    authentication_level_set: bool,
) -> CertificateValidation {
    if authentication_level_set && authentication_level == 0 {
        CertificateValidation::DangerouslyAcceptInvalidCertificate
    } else {
        CertificateValidation::Strict
    }
}

fn certificate_prompt_enabled(
    certificate_validation: CertificateValidation,
    authentication_level: u32,
    authentication_level_set: bool,
    native_mstsc_credential_bridge: bool,
) -> bool {
    certificate_validation == CertificateValidation::Strict
        && (authentication_level == 2 || (!authentication_level_set && native_mstsc_credential_bridge))
}

fn trace_connection_failure(error: &ConnectorError) {
    let category = match error.kind() {
        ConnectorErrorKind::Encode(_) => "Encode",
        ConnectorErrorKind::Decode(_) => "Decode",
        ConnectorErrorKind::Credssp(_) => "CredSsp",
        ConnectorErrorKind::Reason(_) => "Reason",
        ConnectorErrorKind::AccessDenied => "AccessDenied",
        ConnectorErrorKind::General => "General",
        ConnectorErrorKind::Custom => "Custom",
        ConnectorErrorKind::Negotiation(_) => "Negotiation",
        _ => "Unknown",
    };
    let location = error.location();
    let file = location.file().rsplit(['/', '\\']).next().unwrap_or("unknown");
    trace_host_call(&format!(
        "RdpWorker::ConnectionFailure:{category}:{file}:line_{}",
        location.line(),
    ));
}

fn trace_decode_failure(error: &DecodeError) {
    let location = error.location();
    let file = location.file().rsplit(['/', '\\']).next().unwrap_or("unknown");
    let marker = match error.kind() {
        DecodeErrorKind::NotEnoughBytes { received, expected } => {
            format!("Decode:NotEnoughBytes:received_{received}:expected_{expected}")
        }
        DecodeErrorKind::InvalidField { .. } => "Decode:InvalidField".to_owned(),
        DecodeErrorKind::UnexpectedMessageType { .. } => "Decode:UnexpectedMessageType".to_owned(),
        DecodeErrorKind::UnsupportedVersion { .. } => "Decode:UnsupportedVersion".to_owned(),
        DecodeErrorKind::UnsupportedValue { .. } => "Decode:UnsupportedValue".to_owned(),
        DecodeErrorKind::Other { .. } => "Decode:Other".to_owned(),
        _ => "Decode:Unknown".to_owned(),
    };
    trace_host_call(&format!(
        "RdpWorker::SessionFailure:{marker}:{file}:line_{}",
        location.line()
    ));
}

fn trace_session_failure(error: &SessionError) {
    match error.kind() {
        SessionErrorKind::Pdu(_) => trace_host_call("RdpWorker::SessionFailure:Pdu"),
        SessionErrorKind::Encode(_) => trace_host_call("RdpWorker::SessionFailure:Encode"),
        SessionErrorKind::Decode(decode_error) => trace_decode_failure(decode_error),
        SessionErrorKind::FastPathBulkDecompression(failure) => {
            let location = error.location();
            let file = location.file().rsplit(['/', '\\']).next().unwrap_or("unknown");
            let compression_type = failure
                .compression_type()
                .map_or_else(|| "none".to_owned(), |value| value.to_string());
            trace_host_call(&format!(
                "RdpWorker::SessionFailure:FastPathBulk:flags_0x{:02X}:type_{compression_type}:update_0x{:X}:fragment_{}:payload_{}:error_{}:{file}:line_{}",
                failure.compression_flags(),
                failure.update_code(),
                failure.fragmentation(),
                failure.payload_length(),
                failure.error_kind().as_str(),
                location.line()
            ));
        }
        SessionErrorKind::Reason(_) => {
            let location = error.location();
            let file = location.file().rsplit(['/', '\\']).next().unwrap_or("unknown");
            trace_host_call(&format!(
                "RdpWorker::SessionFailure:Reason:{file}:line_{}",
                location.line(),
            ));
        }
        SessionErrorKind::General => {
            let location = error.location();
            let file = location.file().rsplit(['/', '\\']).next().unwrap_or("unknown");
            trace_host_call(&format!(
                "RdpWorker::SessionFailure:General:{file}:line_{}",
                location.line(),
            ));
        }
        SessionErrorKind::Custom => {
            let location = error.location();
            let file = location.file().rsplit(['/', '\\']).next().unwrap_or("unknown");
            trace_host_call(&format!(
                "RdpWorker::SessionFailure:Custom:{file}:line_{}",
                location.line()
            ));
        }
        _ => trace_host_call("RdpWorker::SessionFailure:Unknown"),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Stopping,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DisconnectInfo {
    event_reason: i32,
    extended_reason: i32,
    description: &'static str,
}

impl DisconnectInfo {
    const fn no_info() -> Self {
        Self {
            event_reason: 0,
            extended_reason: EXTENDED_DISCONNECT_REASON_NO_INFO,
            description: "No additional disconnect information is available.",
        }
    }

    const fn api_initiated() -> Self {
        Self {
            event_reason: 0,
            extended_reason: EXTENDED_DISCONNECT_REASON_API_INITIATED_DISCONNECT,
            description: "The RDP session was disconnected by the client.",
        }
    }

    fn from_connection_failure(error: &ConnectorError) -> Self {
        let description = match error.kind() {
            ConnectorErrorKind::Encode(_) => "The RDP client could not encode a protocol message.",
            ConnectorErrorKind::Decode(_) => "The RDP client received an invalid protocol message.",
            ConnectorErrorKind::Credssp(_) => "CredSSP authentication failed.",
            ConnectorErrorKind::Reason(_) => "The RDP connection ended with a protocol reason.",
            ConnectorErrorKind::AccessDenied => "The RDP server denied access.",
            ConnectorErrorKind::General | ConnectorErrorKind::Custom => "The RDP client encountered an internal error.",
            ConnectorErrorKind::Negotiation(_) => "RDP security negotiation failed.",
            _ => "The RDP client encountered an unknown error.",
        };
        Self {
            description,
            ..Self::no_info()
        }
    }

    fn from_session_failure(error: &SessionError) -> Self {
        let description = match error.kind() {
            SessionErrorKind::Pdu(_) | SessionErrorKind::Decode(_) => {
                "The RDP client received an invalid protocol message."
            }
            SessionErrorKind::Encode(_) => "The RDP client could not encode a protocol message.",
            SessionErrorKind::FastPathBulkDecompression(_) => {
                "The RDP client could not decompress a remote graphics update."
            }
            SessionErrorKind::Reason(_) => "The RDP session ended with a protocol reason.",
            SessionErrorKind::General | SessionErrorKind::Custom => "The RDP client encountered an internal error.",
            _ => "The RDP client encountered an unknown error.",
        };
        Self {
            description,
            ..Self::no_info()
        }
    }

    fn from_graceful_disconnect(reason: &GracefulDisconnectReason) -> Self {
        match reason {
            GracefulDisconnectReason::UserInitiated => Self::api_initiated(),
            GracefulDisconnectReason::ServerInitiated => Self {
                description: "The RDP server ended the session.",
                ..Self::no_info()
            },
            GracefulDisconnectReason::Other(_) => Self {
                description: "The RDP session ended with an unclassified server reason.",
                ..Self::no_info()
            },
        }
    }

    const fn internal_error() -> Self {
        Self {
            description: "The RDP client encountered an internal error.",
            ..Self::no_info()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeMstscPreflight {
    Idle,
    FirstEmptyProperty,
    SecondEmptyProperty,
    Suppressed,
}

impl NativeMstscPreflight {
    fn observe_extended_setting(self, disconnected: bool, name: &str) -> (Self, bool) {
        if !disconnected || !name.is_empty() {
            return (Self::Idle, false);
        }

        match self {
            Self::Idle => (Self::FirstEmptyProperty, false),
            Self::FirstEmptyProperty => (Self::SecondEmptyProperty, false),
            // MsRdpEx supplies the property names as empty BSTRs in this native preflight.
            // The observed third callback is RemoteApplicationFile, before mstsc enters private state.
            Self::SecondEmptyProperty => (Self::Suppressed, true),
            Self::Suppressed => (Self::Suppressed, false),
        }
    }
}

struct Settings {
    server: String,
    domain: String,
    username: String,
    password: Option<String>,
    disconnected_text: String,
    connecting_text: String,
    connected_status_text: String,
    fullscreen: bool,
    fullscreen_title: String,
    desktop_width: u16,
    desktop_height: u16,
    color_depth: u32,
    start_connected: bool,
}

#[derive(Clone, Copy)]
struct DisplayLayout {
    desktop_width: u32,
    desktop_height: u32,
    physical_width: u32,
    physical_height: u32,
    orientation: u32,
    desktop_scale_factor: u32,
    device_scale_factor: u32,
}

const MAX_RDP_MONITORS: usize = 16;
const MAX_RDP_VIRTUAL_DESKTOP_DIMENSION: i64 = 32_766;
const MIN_RDP_VIRTUAL_DESKTOP_DIMENSION: i64 = 200;

#[derive(Clone, Copy, Debug, PartialEq)]
struct HostMonitor {
    rect: RECT,
    primary: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MonitorTopology {
    monitors: Vec<Monitor>,
    desktop_width: u16,
    desktop_height: u16,
}

impl MonitorTopology {
    fn from_host_monitors(host_monitors: Vec<HostMonitor>) -> Result<Self> {
        if host_monitors.is_empty() || host_monitors.len() > MAX_RDP_MONITORS {
            return Err(Error::from_hresult(E_INVALIDARG));
        }

        let primary_monitors = host_monitors
            .iter()
            .filter(|monitor| monitor.primary)
            .collect::<Vec<_>>();
        if primary_monitors.len() != 1 {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        let primary = primary_monitors[0].rect;

        let mut monitors = Vec::with_capacity(host_monitors.len());
        for monitor in host_monitors {
            let width = i64::from(monitor.rect.right) - i64::from(monitor.rect.left);
            let height = i64::from(monitor.rect.bottom) - i64::from(monitor.rect.top);
            if width <= 0 || height <= 0 {
                return Err(Error::from_hresult(E_INVALIDARG));
            }

            let left = i64::from(monitor.rect.left) - i64::from(primary.left);
            let top = i64::from(monitor.rect.top) - i64::from(primary.top);
            let right = left
                .checked_add(width - 1)
                .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
            let bottom = top
                .checked_add(height - 1)
                .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
            let (Ok(left), Ok(top), Ok(right), Ok(bottom)) = (
                i32::try_from(left),
                i32::try_from(top),
                i32::try_from(right),
                i32::try_from(bottom),
            ) else {
                return Err(Error::from_hresult(E_INVALIDARG));
            };

            monitors.push(Monitor {
                left,
                top,
                right,
                bottom,
                flags: if monitor.primary {
                    MonitorFlags::PRIMARY
                } else {
                    MonitorFlags::empty()
                },
            });
        }

        let primary_monitor = monitors
            .iter()
            .find(|monitor| monitor.flags.contains(MonitorFlags::PRIMARY))
            .expect("a validated topology has a primary monitor");
        if primary_monitor.left != 0 || primary_monitor.top != 0 {
            return Err(Error::from_hresult(E_INVALIDARG));
        }

        for (index, monitor) in monitors.iter().enumerate() {
            if monitors[..index].iter().any(|other| monitors_overlap(other, monitor)) {
                return Err(Error::from_hresult(E_INVALIDARG));
            }
        }

        let left = monitors
            .iter()
            .map(|monitor| i64::from(monitor.left))
            .min()
            .expect("nonempty topology");
        let top = monitors
            .iter()
            .map(|monitor| i64::from(monitor.top))
            .min()
            .expect("nonempty topology");
        let right = monitors
            .iter()
            .map(|monitor| i64::from(monitor.right))
            .max()
            .expect("nonempty topology");
        let bottom = monitors
            .iter()
            .map(|monitor| i64::from(monitor.bottom))
            .max()
            .expect("nonempty topology");
        let width = right
            .checked_sub(left)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
        let height = bottom
            .checked_sub(top)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
        if !(MIN_RDP_VIRTUAL_DESKTOP_DIMENSION..=MAX_RDP_VIRTUAL_DESKTOP_DIMENSION).contains(&width)
            || !(MIN_RDP_VIRTUAL_DESKTOP_DIMENSION..=MAX_RDP_VIRTUAL_DESKTOP_DIMENSION).contains(&height)
        {
            return Err(Error::from_hresult(E_INVALIDARG));
        }

        Ok(Self {
            monitors,
            desktop_width: u16::try_from(width).expect("RDP virtual desktop width is within u16 range"),
            desktop_height: u16::try_from(height).expect("RDP virtual desktop height is within u16 range"),
        })
    }

    fn client_monitor_data(&self) -> ClientMonitorData {
        ClientMonitorData {
            monitors: self.monitors.clone(),
        }
    }

    fn bounds(&self) -> (i32, i32, i32, i32) {
        (
            self.monitors
                .iter()
                .map(|monitor| monitor.left)
                .min()
                .expect("nonempty topology"),
            self.monitors
                .iter()
                .map(|monitor| monitor.top)
                .min()
                .expect("nonempty topology"),
            self.monitors
                .iter()
                .map(|monitor| monitor.right)
                .max()
                .expect("nonempty topology"),
            self.monitors
                .iter()
                .map(|monitor| monitor.bottom)
                .max()
                .expect("nonempty topology"),
        )
    }
}

fn monitors_overlap(left: &Monitor, right: &Monitor) -> bool {
    left.left <= right.right && right.left <= left.right && left.top <= right.bottom && right.top <= left.bottom
}

fn local_monitor_topology() -> Result<MonitorTopology> {
    let mut host_monitors = Vec::new();
    let result = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect_local_monitor),
            LPARAM((&raw mut host_monitors).cast::<c_void>() as isize),
        )
    };
    if !result.as_bool() {
        return Err(Error::from_hresult(E_FAIL));
    }

    MonitorTopology::from_host_monitors(host_monitors)
}

unsafe extern "system" fn collect_local_monitor(
    monitor: HMONITOR,
    _device_context: HDC,
    _clip: *mut RECT,
    context: LPARAM,
) -> WinBool {
    let host_monitors = unsafe { &mut *(context.0 as *mut Vec<HostMonitor>) };
    let mut monitor_info = MONITORINFO {
        cbSize: u32::try_from(size_of::<MONITORINFO>()).expect("MONITORINFO size fits in u32"),
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut monitor_info) }.as_bool() {
        return WinBool(0);
    }

    host_monitors.push(HostMonitor {
        rect: monitor_info.rcMonitor,
        primary: monitor_info.dwFlags & 0x0000_0001 != 0,
    });
    WinBool(1)
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct ConnectionBarOwnerLayout {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    dpi: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConnectionHealthStatus {
    Hidden,
    Connecting,
    UpdatingDisplay,
    Reconnecting { attempt: u32, maximum: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConnectionSecurityWarning {
    SendingCredentials,
    ClipboardRedirection,
}

fn connection_security_warnings(
    warn_about_credentials: bool,
    warn_about_clipboard: bool,
) -> Vec<ConnectionSecurityWarning> {
    [
        (warn_about_credentials, ConnectionSecurityWarning::SendingCredentials),
        (warn_about_clipboard, ConnectionSecurityWarning::ClipboardRedirection),
    ]
    .into_iter()
    .filter_map(|(enabled, warning)| enabled.then_some(warning))
    .collect()
}

impl ConnectionSecurityWarning {
    fn text(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::SendingCredentials => (
                "IronRDP security warning",
                "Credentials will be sent to establish the remote session.",
                "Continue only if you trust the remote session configuration.",
            ),
            Self::ClipboardRedirection => (
                "IronRDP security warning",
                "Clipboard redirection will be enabled for the remote session.",
                "Text and files copied in either session can become available to the other session.",
            ),
        }
    }
}

fn connection_information_content(
    state: ConnectionState,
    remote_size: Option<(i32, i32)>,
    clipboard_enabled: bool,
) -> Option<String> {
    if state != ConnectionState::Connected {
        return None;
    }

    let mut details = String::from("Connection status: Connected");
    if let Some((width, height)) = remote_size {
        details.push_str(&format!("\r\nDesktop size: {width} x {height}"));
    }
    details.push_str(if clipboard_enabled {
        "\r\nClipboard redirection: Enabled"
    } else {
        "\r\nClipboard redirection: Disabled"
    });
    Some(details)
}

fn connection_bar_disconnect_prompt() -> (&'static str, &'static str, &'static str) {
    (
        "IronRDP disconnect",
        "Disconnect from the remote desktop session?",
        "The remote desktop session will be disconnected.",
    )
}

impl ConnectionHealthStatus {
    fn reconnecting(attempt: u32, maximum: u32) -> Option<Self> {
        (attempt != 0 && attempt <= maximum).then_some(Self::Reconnecting { attempt, maximum })
    }

    fn text(self) -> (&'static str, Option<String>) {
        match self {
            Self::Hidden => ("", None),
            Self::Connecting => ("Connecting...", None),
            Self::UpdatingDisplay => ("Updating remote display...", None),
            Self::Reconnecting { attempt, maximum } => {
                ("Reconnecting...", Some(format!("Attempt {attempt} of {maximum}")))
            }
        }
    }
}

fn clear_connection_health_status(status: &Cell<ConnectionHealthStatus>) -> bool {
    status.replace(ConnectionHealthStatus::Hidden) != ConnectionHealthStatus::Hidden
}

fn display_layout_from_renderer_size(width: i32, height: i32) -> Option<DisplayLayout> {
    let (Ok(desktop_width), Ok(desktop_height)) = (u32::try_from(width), u32::try_from(height)) else {
        return None;
    };
    (desktop_width != 0 && desktop_height != 0).then_some(DisplayLayout {
        desktop_width,
        desktop_height,
        physical_width: 0,
        physical_height: 0,
        orientation: 0,
        desktop_scale_factor: 100,
        device_scale_factor: 100,
    })
}

fn native_shell_presentation_enabled(native_shell_integration: bool, native_shell: bool) -> bool {
    native_shell_integration && native_shell
}

struct CompatibilitySettings {
    renderer_window: HWND,
    container_handled_fullscreen: i32,
    allow_background_input: i32,
    display_connection_bar: i16,
    display_connection_bar_set: bool,
    pin_connection_bar: i16,
    connection_bar_show_minimize_button: i16,
    connection_bar_show_restore_button: i16,
    connection_bar_show_pin_button: i16,
    connection_bar_disabled: bool,
    connection_bar_text: String,
    clear_text_password: Option<String>,
    smart_sizing: bool,
    grab_focus_on_connect: bool,
    enable_credssp: Option<bool>,
    compression: Option<bool>,
    rdp_port: Option<u16>,
    enable_mouse: bool,
    enable_windows_key: bool,
    redirect_clipboard: bool,
    redirect_webauthn: bool,
    redirect_drives: bool,
    redirect_smart_cards: bool,
    disable_rdpdr: bool,
    drive_catalog: Rc<RefCell<DriveCatalog>>,
    warn_about_sending_credentials: bool,
    warn_about_clipboard_redirection: bool,
    performance_flags: PerformanceFlags,
    keyboard_type: KeyboardType,
    keyboard_subtype: u32,
    keyboard_functional_keys_count: u32,
    keyboard_layout: u32,
    network_connection_type: ConnectionType,
    keyboard_hook_mode: i32,
    zoom_level: i32,
    gateway_hostname: String,
    gateway_username: String,
    gateway_domain: String,
    gateway_password: String,
    gateway_usage_method: u32,
    gateway_creds_source: u32,
    secured_start_program: String,
    secured_work_dir: String,
    secured_fullscreen: i32,
    audio_redirection_mode: i32,
    /// MSTSC `AudioCaptureRedirectionMode` (VARIANT_BOOL; non-zero enables mic capture).
    audio_capture_redirection_mode: i16,
    remote_program_mode: bool,
    remote_application_name: String,
    remote_application_program: String,
    remote_application_args: String,
    prompt_for_credentials: bool,
    client_name: Option<String>,
    dvc_plugin_paths: Vec<PathBuf>,
    enable_tls: Option<bool>,
    autologon: Option<bool>,
    desktop_scale_factor: Option<u32>,
    compression_level: Option<u32>,
    client_build: u32,
    client_dir: String,
    ime_file_name: String,
    digital_product_id: String,
    fake_events_interval_minutes: Option<u32>,
    authentication_level: u32,
    authentication_level_set: bool,
    public_mode: bool,
    enable_auto_reconnect: bool,
    max_reconnect_attempts: u32,
    use_multimon: bool,
    connection_settings_sealed: bool,
    persistence_dirty: Option<Rc<Cell<bool>>>,
}

impl Default for CompatibilitySettings {
    fn default() -> Self {
        Self {
            renderer_window: HWND(ptr::null_mut()),
            container_handled_fullscreen: 0,
            allow_background_input: 0,
            display_connection_bar: VARIANT_FALSE.0,
            display_connection_bar_set: false,
            pin_connection_bar: VARIANT_FALSE.0,
            connection_bar_show_minimize_button: VARIANT_TRUE.0,
            connection_bar_show_restore_button: VARIANT_TRUE.0,
            connection_bar_show_pin_button: VARIANT_TRUE.0,
            connection_bar_disabled: false,
            connection_bar_text: String::new(),
            clear_text_password: None,
            smart_sizing: false,
            grab_focus_on_connect: false,
            enable_credssp: None,
            compression: None,
            rdp_port: None,
            enable_mouse: true,
            enable_windows_key: true,
            redirect_clipboard: true,
            redirect_webauthn: true,
            redirect_drives: false,
            redirect_smart_cards: false,
            disable_rdpdr: false,
            drive_catalog: Rc::new(RefCell::new(DriveCatalog::new())),
            warn_about_sending_credentials: false,
            warn_about_clipboard_redirection: false,
            performance_flags: PerformanceFlags::default(),
            keyboard_type: KeyboardType::IbmEnhanced,
            keyboard_subtype: 0,
            keyboard_functional_keys_count: 12,
            keyboard_layout: 0,
            network_connection_type: ConnectionType::Lan,
            keyboard_hook_mode: 2,
            zoom_level: 100,
            gateway_hostname: String::new(),
            gateway_username: String::new(),
            gateway_domain: String::new(),
            gateway_password: String::new(),
            gateway_usage_method: 0,
            gateway_creds_source: 0,
            secured_start_program: String::new(),
            secured_work_dir: String::new(),
            secured_fullscreen: 0,
            audio_redirection_mode: 0,
            audio_capture_redirection_mode: VARIANT_FALSE.0,
            remote_program_mode: false,
            remote_application_name: String::new(),
            remote_application_program: String::new(),
            remote_application_args: String::new(),
            prompt_for_credentials: false,
            client_name: None,
            dvc_plugin_paths: Vec::new(),
            enable_tls: None,
            autologon: None,
            desktop_scale_factor: None,
            compression_level: None,
            client_build: 10_000,
            client_dir: "C:\\".to_owned(),
            ime_file_name: String::new(),
            digital_product_id: String::new(),
            fake_events_interval_minutes: None,
            authentication_level: 0,
            authentication_level_set: false,
            public_mode: false,
            enable_auto_reconnect: true,
            max_reconnect_attempts: 20,
            use_multimon: false,
            connection_settings_sealed: false,
            persistence_dirty: None,
        }
    }
}

fn mark_compatibility_persistence_dirty(settings: &CompatibilitySettings) {
    if let Some(persistence_dirty) = &settings.persistence_dirty {
        persistence_dirty.set(true);
    }
}

fn validate_activex_extended_string(value: String) -> Result<String> {
    if value.len() > MAX_ACTIVEX_EXTENDED_SETTING_STRING_BYTES {
        return Err(Error::new(E_INVALIDARG, "extended setting exceeds the maximum length"));
    }
    Ok(value)
}

fn active_x_connection_settings_mutable(state: ConnectionState, settings: &CompatibilitySettings) -> Result<()> {
    if state != ConnectionState::Disconnected || settings.connection_settings_sealed {
        return Err(Error::from_hresult(E_UNEXPECTED));
    }
    Ok(())
}

#[derive(Clone, Default)]
struct RDCleanPathSettings {
    url: Option<String>,
    token: Option<String>,
}

impl RDCleanPathSettings {
    fn set_url(&mut self, value: String) -> Result<()> {
        let value = validate_activex_extended_string(value)?;
        let url = value
            .parse::<url::Url>()
            .map_err(|_| Error::new(E_INVALIDARG, "invalid RDCleanPath URL"))?;
        if !matches!(url.scheme(), "ws" | "wss") {
            return Err(Error::new(
                E_INVALIDARG,
                "RDCleanPath URL must use the ws or wss scheme",
            ));
        }

        self.url = Some(value);
        Ok(())
    }

    fn set_token(&mut self, value: String) -> Result<()> {
        let value = validate_activex_extended_string(value)?;
        if value.is_empty() {
            return Err(Error::new(E_INVALIDARG, "RDCleanPathToken must not be empty"));
        }

        self.token = Some(value);
        Ok(())
    }

    fn transport(&self) -> Result<Option<ActiveXTransport>> {
        match (&self.url, &self.token) {
            (None, None) => Ok(None),
            (Some(url), Some(token)) => {
                let url = url
                    .parse::<url::Url>()
                    .map_err(|_| Error::new(E_INVALIDARG, "invalid RDCleanPath URL"))?;
                Ok(Some(ActiveXTransport::RDCleanPath(RDCleanPathConfig {
                    url,
                    auth_token: token.clone(),
                })))
            }
            _ => Err(Error::new(
                E_INVALIDARG,
                "RDCleanPathUrl and RDCleanPathToken must be configured together",
            )),
        }
    }

    fn apply_to_client_properties(&self, properties: &mut PropertySet) -> Result<()> {
        if let (Some(url), Some(token)) = (&self.url, &self.token) {
            properties.insert("ironrdp_rdcleanpathurl", url.clone());
            properties.insert("ironrdp_rdcleanpathtoken", token.clone());
        } else {
            self.transport()?;
        }
        Ok(())
    }
}

// The settings objects are consumed through their published dual-interface vtables by
// mstsc.exe and MsRdpEx. Keep the complete slot count even where IronRDP has no mapping.
#[repr(C)]
struct CompatibilitySettingsVtable<const SLOTS: usize> {
    dispatch: IDispatch_Vtbl,
    slots: [usize; SLOTS],
}

fn connection_bar_is_eligible(
    state: ConnectionState,
    settings: &Settings,
    compatibility: &CompatibilitySettings,
    native_shell_presentation: bool,
) -> bool {
    state == ConnectionState::Connected
        && settings.fullscreen
        && (compatibility.display_connection_bar != VARIANT_FALSE.0
            || (native_shell_presentation && !compatibility.display_connection_bar_set))
        && !compatibility.connection_bar_disabled
}

fn connection_bar_title<'a>(connection_bar_text: &'a str, server: &'a str) -> &'a str {
    if connection_bar_text.trim().is_empty() {
        server
    } else {
        connection_bar_text
    }
}

fn connection_bar_scale(logical_pixels: i32, dpi: u32) -> i32 {
    (i64::from(logical_pixels) * i64::from(dpi.max(1)) + i64::from(DEFAULT_DPI / 2))
        .div_euclid(i64::from(DEFAULT_DPI))
        .clamp(1, i64::from(i32::MAX)) as i32
}

fn connection_bar_size(dpi: u32) -> (i32, i32) {
    (
        connection_bar_scale(CONNECTION_BAR_WIDTH, dpi),
        connection_bar_scale(CONNECTION_BAR_HEIGHT, dpi),
    )
}

fn connection_health_size(dpi: u32) -> (i32, i32) {
    (
        connection_bar_scale(CONNECTION_HEALTH_WIDTH, dpi),
        connection_bar_scale(CONNECTION_HEALTH_HEIGHT, dpi),
    )
}

fn connection_bar_dpi(window: HWND) -> u32 {
    unsafe { GetDpiForWindow(window) }.max(DEFAULT_DPI)
}

fn connection_health_position(owner: RECT, width: i32, height: i32) -> (i32, i32) {
    let owner_width = i64::from(owner.right).saturating_sub(i64::from(owner.left)).max(0);
    let owner_height = i64::from(owner.bottom).saturating_sub(i64::from(owner.top)).max(0);
    let x = i64::from(owner.left)
        .saturating_add(owner_width.saturating_sub(i64::from(width)) / 2)
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    let y = i64::from(owner.top)
        .saturating_add(owner_height.saturating_sub(i64::from(height)) / 2)
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    (x, y)
}

fn connection_bar_position_for_width(owner: RECT, width: i32) -> (i32, i32) {
    let owner_width = i64::from(owner.right).saturating_sub(i64::from(owner.left)).max(0);
    let x = i64::from(owner.left)
        .saturating_add(owner_width.saturating_sub(i64::from(width)) / 2)
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    (x, owner.top)
}

fn connection_bar_button_rect(button_id: usize, dpi: u32) -> Option<RECT> {
    let (left, top, width, height) = match button_id {
        CONNECTION_BAR_INFORMATION_BUTTON_ID => (180, 6, 76, 24),
        CONNECTION_BAR_PIN_BUTTON_ID => (256, 6, 80, 24),
        CONNECTION_BAR_MINIMIZE_BUTTON_ID => (336, 6, 80, 24),
        CONNECTION_BAR_RESTORE_BUTTON_ID => (416, 6, 72, 24),
        CONNECTION_BAR_FULLSCREEN_BUTTON_ID => (488, 6, 104, 24),
        CONNECTION_BAR_CLOSE_BUTTON_ID => (592, 6, 72, 24),
        CONNECTION_BAR_DISCONNECT_BUTTON_ID => (664, 6, 110, 24),
        _ => return None,
    };
    let left = connection_bar_scale(left, dpi);
    let top = connection_bar_scale(top, dpi);
    Some(RECT {
        left,
        top,
        right: left.saturating_add(connection_bar_scale(width, dpi)),
        bottom: top.saturating_add(connection_bar_scale(height, dpi)),
    })
}

fn connection_bar_title_rect(dpi: u32) -> RECT {
    let left = connection_bar_scale(8, dpi);
    let top = connection_bar_scale(6, dpi);
    RECT {
        left,
        top,
        right: left.saturating_add(connection_bar_scale(172, dpi)),
        bottom: top.saturating_add(connection_bar_scale(24, dpi)),
    }
}

fn connection_bar_button_style(visible: bool) -> WINDOW_STYLE {
    WS_CHILD
        | WS_TABSTOP
        | WINDOW_STYLE(BS_PUSHBUTTON as u32)
        | if visible { WS_VISIBLE } else { WINDOW_STYLE::default() }
}

fn point_is_inside_rect(point: POINT, rect: RECT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

fn ensure_content_aspect(aspect: DVASPECT) -> Result<()> {
    if aspect == DVASPECT_CONTENT {
        Ok(())
    } else {
        Err(Error::from_hresult(DV_E_DVASPECT))
    }
}

fn ensure_content_view(aspect: DVASPECT, index: i32) -> Result<()> {
    ensure_content_aspect(aspect)?;
    if index == -1 {
        Ok(())
    } else {
        Err(Error::from_hresult(DV_E_LINDEX))
    }
}

fn view_extent_rect(extent: SIZE) -> RECTL {
    RECTL {
        left: 0,
        top: 0,
        right: extent.cx.max(0),
        bottom: extent.cy.max(0),
    }
}

fn view_rect_contains_point(rect: RECTL, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

fn view_rects_intersect(left: RECTL, right: RECTL) -> bool {
    left.left < right.right && right.left < left.right && left.top < right.bottom && right.top < left.bottom
}

fn next_connection_bar_button_id(current: usize, visible_buttons: &[usize], reverse: bool) -> Option<usize> {
    visible_buttons.first()?;
    let current_index = visible_buttons.iter().position(|button_id| *button_id == current);
    let index = match (current_index, reverse) {
        (Some(index), false) => (index + 1) % visible_buttons.len(),
        (Some(0), true) => visible_buttons.len() - 1,
        (Some(index), true) => index - 1,
        (None, false) => 0,
        (None, true) => visible_buttons.len() - 1,
    };
    Some(visible_buttons[index])
}

fn request_connection_bar_update(renderer: HWND) {
    if renderer.0.is_null() || !unsafe { IsWindow(Some(renderer)) }.as_bool() {
        return;
    }
    if let Err(error) = unsafe { PostMessageW(Some(renderer), WM_UPDATE_CONNECTION_BAR, WPARAM(0), LPARAM(0)) } {
        tracing::debug!(?error, "Unable to post ActiveX connection bar update");
    }
}

const SECURED_SETTINGS_SLOTS: usize = 12;
const TRANSPORT_SETTINGS_SLOTS: usize = 40;

#[repr(C)]
struct CompatibilitySettingsObject<const SLOTS: usize> {
    vtable: *const CompatibilitySettingsVtable<SLOTS>,
    references: AtomicU32,
    settings: Rc<RefCell<CompatibilitySettings>>,
    native_mstsc_credential_bridge: Option<NativeMstscCredentialBridge>,
    server_object: bool,
}

struct NativeMstscCredentialBridge {
    // The COM reference keeps `control` valid while a child secured-settings object remains live.
    _owner: IUnknown,
    control: *const Control_Impl,
}

enum NativeMstscStartProgramIntercept {
    NotHandled,
    Handled,
}

impl NativeMstscCredentialBridge {
    fn intercept_start_program(&self) -> NativeMstscStartProgramIntercept {
        // SAFETY: `_owner` owns an IUnknown reference to the containing Control, whose immutable
        // implementation address remains valid until this bridge is dropped.
        let Some(control) = (unsafe { self.control.as_ref() }) else {
            return NativeMstscStartProgramIntercept::NotHandled;
        };
        if !should_intercept_native_mstsc_start_program(
            native_mstsc_credential_bridge_enabled(),
            control.state.get() == ConnectionState::Disconnected,
        ) {
            return NativeMstscStartProgramIntercept::NotHandled;
        }

        // Do not let the legacy empty-property fallback show a second prompt after this observed
        // stock-form preflight has already taken ownership of the interaction.
        control.native_mstsc_preflight.set(NativeMstscPreflight::Suppressed);
        let started = match control.prompt_for_credentials() {
            Ok(started) => started,
            Err(error) => {
                tracing::warn!(code = error.code().0, "Native mstsc credential prompt failed");
                false
            }
        };
        if !started {
            trace_host_call("NativeMstscCredentialBridge::StartProgramNotStarted");
        }
        if started && native_mstsc_autologon_enabled() {
            // CredUI normally supplies a modal delay before the native shell observes the bridge's
            // preflight failure. Give the unattended worker the same bounded initialization window.
            std::thread::sleep(Duration::from_secs(3));
        }
        NativeMstscStartProgramIntercept::Handled
    }
}

impl<const SLOTS: usize> Drop for CompatibilitySettingsObject<SLOTS> {
    fn drop(&mut self) {
        if self.server_object {
            com::release_object();
        }
    }
}

unsafe extern "system" fn settings_query_interface<const SLOTS: usize>(
    this: *mut c_void,
    iid: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    if object.is_null() {
        return E_POINTER;
    }
    if iid.is_null() {
        unsafe { *object = ptr::null_mut() };
        return HRESULT(0x8000_4002u32 as i32);
    }
    let iid = unsafe { &*iid };
    trace_host_call(&format!("CompatibilitySettings<{SLOTS}>::QueryInterface({iid:?})"));
    if !settings_supports_interface::<SLOTS>(iid) {
        unsafe { *object = ptr::null_mut() };
        return HRESULT(0x8000_4002u32 as i32);
    }
    unsafe {
        *object = this;
        settings_add_ref::<SLOTS>(this);
    }
    HRESULT(0)
}

fn settings_supports_interface<const SLOTS: usize>(iid: &GUID) -> bool {
    if *iid == IUnknown::IID || *iid == IDispatch::IID {
        return true;
    }

    match SLOTS {
        191 => [
            GUID::from_u128(0x809945cc_4b3b_4a92_a6b0_dbf9b5f2ef2d),
            GUID::from_u128(0x3c65b4ab_12b3_465b_acd4_b8dad3bff9e2),
            GUID::from_u128(0x9ac42117_2b76_4320_aa44_0e616ab8437b),
            GUID::from_u128(0x19cd856b_c542_4c53_acee_f127e3be1a59),
            GUID::from_u128(0xfba7f64e_7345_4405_ae50_fa4a763dc0de),
            GUID::from_u128(0xfba7f64e_6783_4405_da45_fa4a763dabd0),
            GUID::from_u128(0x222c4b5d_45d9_4df0_a7c6_60cf9089d285),
            GUID::from_u128(0x26036036_4010_4578_8091_0db9a1edf9c3),
            GUID::from_u128(0x89acb528_2557_4d16_8625_226a30e97e9a),
        ]
        .contains(iid),
        SECURED_SETTINGS_SLOTS => [
            GUID::from_u128(0xc9d65442_a0f9_45b2_8f73_d61d2db8cbb6),
            GUID::from_u128(0x605befcf_39c1_45cc_a811_068fb7be346d),
            GUID::from_u128(0x25f2ce20_8b1d_4971_a7cd_549dae201fc0),
        ]
        .contains(iid),
        TRANSPORT_SETTINGS_SLOTS => [
            GUID::from_u128(0x720298c0_a099_46f5_9f82_96921bae4701),
            GUID::from_u128(0x67341688_d606_4c73_a5d2_2e0489009319),
            GUID::from_u128(0x3d5b21ac_748d_41de_8f30_e15169586bd4),
            GUID::from_u128(0x011c3236_4d81_4515_9143_067ab630d299),
        ]
        .contains(iid),
        7 => [
            GUID::from_u128(0xfdd029f9_467a_4c49_8529_64b521dbd1b4),
            GUID::from_u128(0x92c38a7d_241a_418c_9936_099872c9af20),
            GUID::from_u128(0x4b84ea77_acea_418c_881a_4a8c28ab1510),
        ]
        .contains(iid),
        _ => false,
    }
}

unsafe extern "system" fn settings_add_ref<const SLOTS: usize>(this: *mut c_void) -> u32 {
    let settings = unsafe { &*(this.cast::<CompatibilitySettingsObject<SLOTS>>()) };
    let mut references = settings.references.load(Ordering::Acquire);
    loop {
        if references == 0 {
            tracing::error!("ActiveX compatibility settings AddRef observed a released object");
            return 0;
        }
        let Some(next) = references.checked_add(1) else {
            tracing::error!("ActiveX compatibility settings reference count overflowed");
            return u32::MAX;
        };
        match settings
            .references
            .compare_exchange_weak(references, next, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => return next,
            Err(current) => references = current,
        }
    }
}

unsafe extern "system" fn settings_release<const SLOTS: usize>(this: *mut c_void) -> u32 {
    let settings = unsafe { &*(this.cast::<CompatibilitySettingsObject<SLOTS>>()) };
    let mut references = settings.references.load(Ordering::Acquire);
    loop {
        if references == 0 || references == u32::MAX {
            tracing::error!("ActiveX compatibility settings Release observed an invalid reference count");
            return references;
        }
        let remaining = references - 1;
        match settings
            .references
            .compare_exchange_weak(references, remaining, Ordering::Release, Ordering::Acquire)
        {
            Ok(_) => {
                if remaining == 0 {
                    core::sync::atomic::fence(Ordering::Acquire);
                    unsafe { drop(Box::from_raw(this.cast::<CompatibilitySettingsObject<SLOTS>>())) };
                }
                return remaining;
            }
            Err(current) => references = current,
        }
    }
}

unsafe extern "system" fn settings_get_type_info_count<const SLOTS: usize>(
    _this: *mut c_void,
    count: *mut u32,
) -> HRESULT {
    if count.is_null() {
        return E_POINTER;
    }
    unsafe { *count = 0 };
    HRESULT(0)
}

unsafe extern "system" fn settings_get_type_info<const SLOTS: usize>(
    _this: *mut c_void,
    _index: u32,
    _lcid: u32,
    info: *mut *mut c_void,
) -> HRESULT {
    if !info.is_null() {
        unsafe { *info = ptr::null_mut() };
    }
    E_NOTIMPL
}

unsafe extern "system" fn settings_get_ids_of_names<const SLOTS: usize>(
    _this: *mut c_void,
    _iid: *const GUID,
    _names: *const PCWSTR,
    _count: u32,
    _lcid: u32,
    _ids: *mut i32,
) -> HRESULT {
    DISP_E_MEMBERNOTFOUND
}

unsafe extern "system" fn settings_invoke<const SLOTS: usize>(
    _this: *mut c_void,
    _dispid: i32,
    _iid: *const GUID,
    _lcid: u32,
    _flags: DISPATCH_FLAGS,
    _params: *const DISPPARAMS,
    _result: *mut VARIANT,
    _exception: *mut EXCEPINFO,
    _argument_error: *mut u32,
) -> HRESULT {
    DISP_E_MEMBERNOTFOUND
}

macro_rules! advanced_settings_stubs {
    ($(($slot:literal, $stub:ident)),+ $(,)?) => {
        $(
            // Every advanced-settings vtable slot is a property accessor, so it has one
            // four-byte (on x86) argument in addition to `this`. Keep the ABI stack-clean
            // even when the property has no IronRDP mapping.
            unsafe extern "system" fn $stub(_this: *mut c_void, _value: usize) -> HRESULT {
                trace_host_call(concat!("E_NOTIMPL:AdvancedSettings::slot_", stringify!($slot)));
                E_NOTIMPL
            }
        )+

        fn advanced_settings_stub_slots() -> [usize; 191] {
            [$($stub as *const () as usize),+]
        }
    };
}

advanced_settings_stubs!(
    (0, advanced_settings_stub_0),
    (1, advanced_settings_stub_1),
    (2, advanced_settings_stub_2),
    (3, advanced_settings_stub_3),
    (4, advanced_settings_stub_4),
    (5, advanced_settings_stub_5),
    (6, advanced_settings_stub_6),
    (7, advanced_settings_stub_7),
    (8, advanced_settings_stub_8),
    (9, advanced_settings_stub_9),
    (10, advanced_settings_stub_10),
    (11, advanced_settings_stub_11),
    (12, advanced_settings_stub_12),
    (13, advanced_settings_stub_13),
    (14, advanced_settings_stub_14),
    (15, advanced_settings_stub_15),
    (16, advanced_settings_stub_16),
    (17, advanced_settings_stub_17),
    (18, advanced_settings_stub_18),
    (19, advanced_settings_stub_19),
    (20, advanced_settings_stub_20),
    (21, advanced_settings_stub_21),
    (22, advanced_settings_stub_22),
    (23, advanced_settings_stub_23),
    (24, advanced_settings_stub_24),
    (25, advanced_settings_stub_25),
    (26, advanced_settings_stub_26),
    (27, advanced_settings_stub_27),
    (28, advanced_settings_stub_28),
    (29, advanced_settings_stub_29),
    (30, advanced_settings_stub_30),
    (31, advanced_settings_stub_31),
    (32, advanced_settings_stub_32),
    (33, advanced_settings_stub_33),
    (34, advanced_settings_stub_34),
    (35, advanced_settings_stub_35),
    (36, advanced_settings_stub_36),
    (37, advanced_settings_stub_37),
    (38, advanced_settings_stub_38),
    (39, advanced_settings_stub_39),
    (40, advanced_settings_stub_40),
    (41, advanced_settings_stub_41),
    (42, advanced_settings_stub_42),
    (43, advanced_settings_stub_43),
    (44, advanced_settings_stub_44),
    (45, advanced_settings_stub_45),
    (46, advanced_settings_stub_46),
    (47, advanced_settings_stub_47),
    (48, advanced_settings_stub_48),
    (49, advanced_settings_stub_49),
    (50, advanced_settings_stub_50),
    (51, advanced_settings_stub_51),
    (52, advanced_settings_stub_52),
    (53, advanced_settings_stub_53),
    (54, advanced_settings_stub_54),
    (55, advanced_settings_stub_55),
    (56, advanced_settings_stub_56),
    (57, advanced_settings_stub_57),
    (58, advanced_settings_stub_58),
    (59, advanced_settings_stub_59),
    (60, advanced_settings_stub_60),
    (61, advanced_settings_stub_61),
    (62, advanced_settings_stub_62),
    (63, advanced_settings_stub_63),
    (64, advanced_settings_stub_64),
    (65, advanced_settings_stub_65),
    (66, advanced_settings_stub_66),
    (67, advanced_settings_stub_67),
    (68, advanced_settings_stub_68),
    (69, advanced_settings_stub_69),
    (70, advanced_settings_stub_70),
    (71, advanced_settings_stub_71),
    (72, advanced_settings_stub_72),
    (73, advanced_settings_stub_73),
    (74, advanced_settings_stub_74),
    (75, advanced_settings_stub_75),
    (76, advanced_settings_stub_76),
    (77, advanced_settings_stub_77),
    (78, advanced_settings_stub_78),
    (79, advanced_settings_stub_79),
    (80, advanced_settings_stub_80),
    (81, advanced_settings_stub_81),
    (82, advanced_settings_stub_82),
    (83, advanced_settings_stub_83),
    (84, advanced_settings_stub_84),
    (85, advanced_settings_stub_85),
    (86, advanced_settings_stub_86),
    (87, advanced_settings_stub_87),
    (88, advanced_settings_stub_88),
    (89, advanced_settings_stub_89),
    (90, advanced_settings_stub_90),
    (91, advanced_settings_stub_91),
    (92, advanced_settings_stub_92),
    (93, advanced_settings_stub_93),
    (94, advanced_settings_stub_94),
    (95, advanced_settings_stub_95),
    (96, advanced_settings_stub_96),
    (97, advanced_settings_stub_97),
    (98, advanced_settings_stub_98),
    (99, advanced_settings_stub_99),
    (100, advanced_settings_stub_100),
    (101, advanced_settings_stub_101),
    (102, advanced_settings_stub_102),
    (103, advanced_settings_stub_103),
    (104, advanced_settings_stub_104),
    (105, advanced_settings_stub_105),
    (106, advanced_settings_stub_106),
    (107, advanced_settings_stub_107),
    (108, advanced_settings_stub_108),
    (109, advanced_settings_stub_109),
    (110, advanced_settings_stub_110),
    (111, advanced_settings_stub_111),
    (112, advanced_settings_stub_112),
    (113, advanced_settings_stub_113),
    (114, advanced_settings_stub_114),
    (115, advanced_settings_stub_115),
    (116, advanced_settings_stub_116),
    (117, advanced_settings_stub_117),
    (118, advanced_settings_stub_118),
    (119, advanced_settings_stub_119),
    (120, advanced_settings_stub_120),
    (121, advanced_settings_stub_121),
    (122, advanced_settings_stub_122),
    (123, advanced_settings_stub_123),
    (124, advanced_settings_stub_124),
    (125, advanced_settings_stub_125),
    (126, advanced_settings_stub_126),
    (127, advanced_settings_stub_127),
    (128, advanced_settings_stub_128),
    (129, advanced_settings_stub_129),
    (130, advanced_settings_stub_130),
    (131, advanced_settings_stub_131),
    (132, advanced_settings_stub_132),
    (133, advanced_settings_stub_133),
    (134, advanced_settings_stub_134),
    (135, advanced_settings_stub_135),
    (136, advanced_settings_stub_136),
    (137, advanced_settings_stub_137),
    (138, advanced_settings_stub_138),
    (139, advanced_settings_stub_139),
    (140, advanced_settings_stub_140),
    (141, advanced_settings_stub_141),
    (142, advanced_settings_stub_142),
    (143, advanced_settings_stub_143),
    (144, advanced_settings_stub_144),
    (145, advanced_settings_stub_145),
    (146, advanced_settings_stub_146),
    (147, advanced_settings_stub_147),
    (148, advanced_settings_stub_148),
    (149, advanced_settings_stub_149),
    (150, advanced_settings_stub_150),
    (151, advanced_settings_stub_151),
    (152, advanced_settings_stub_152),
    (153, advanced_settings_stub_153),
    (154, advanced_settings_stub_154),
    (155, advanced_settings_stub_155),
    (156, advanced_settings_stub_156),
    (157, advanced_settings_stub_157),
    (158, advanced_settings_stub_158),
    (159, advanced_settings_stub_159),
    (160, advanced_settings_stub_160),
    (161, advanced_settings_stub_161),
    (162, advanced_settings_stub_162),
    (163, advanced_settings_stub_163),
    (164, advanced_settings_stub_164),
    (165, advanced_settings_stub_165),
    (166, advanced_settings_stub_166),
    (167, advanced_settings_stub_167),
    (168, advanced_settings_stub_168),
    (169, advanced_settings_stub_169),
    (170, advanced_settings_stub_170),
    (171, advanced_settings_stub_171),
    (172, advanced_settings_stub_172),
    (173, advanced_settings_stub_173),
    (174, advanced_settings_stub_174),
    (175, advanced_settings_stub_175),
    (176, advanced_settings_stub_176),
    (177, advanced_settings_stub_177),
    (178, advanced_settings_stub_178),
    (179, advanced_settings_stub_179),
    (180, advanced_settings_stub_180),
    (181, advanced_settings_stub_181),
    (182, advanced_settings_stub_182),
    (183, advanced_settings_stub_183),
    (184, advanced_settings_stub_184),
    (185, advanced_settings_stub_185),
    (186, advanced_settings_stub_186),
    (187, advanced_settings_stub_187),
    (188, advanced_settings_stub_188),
    (189, advanced_settings_stub_189),
    (190, advanced_settings_stub_190),
);

macro_rules! advanced_put_not_implemented {
    ($(($slot:literal, $name:ident, $value:ty)),+ $(,)?) => {
        $(
            unsafe extern "system" fn $name(_this: *mut c_void, _value: $value) -> HRESULT {
                trace_host_call(concat!("E_NOTIMPL:AdvancedSettings::slot_", stringify!($slot)));
                E_NOTIMPL
            }
        )+
    };
}

macro_rules! advanced_get_not_implemented {
    ($(($slot:literal, $name:ident, $value:ty)),+ $(,)?) => {
        $(
            unsafe extern "system" fn $name(_this: *mut c_void, value: *mut $value) -> HRESULT {
                if let Err(error) = write_out(value, <$value>::default()) {
                    return error.code();
                }
                trace_host_call(concat!("E_NOTIMPL:AdvancedSettings::slot_", stringify!($slot)));
                E_NOTIMPL
            }
        )+
    };
}

advanced_put_not_implemented!(
    (7, advanced_put_plugin_dlls, Bstr),
    (69, advanced_put_min_input_send_interval, i32),
    (75, advanced_put_keep_alive_interval, i32),
    (91, advanced_put_connect_to_server_console, i16),
    (93, advanced_put_bitmap_persistence, i32),
    (95, advanced_put_minutes_to_idle_timeout, i32),
    (112, advanced_put_load_balance_info, Bstr),
    (116, advanced_put_redirect_printers, i16),
    (118, advanced_put_redirect_ports, i16),
    (150, advanced_put_redirect_devices, i16),
    (161, advanced_put_pcb, Bstr),
    (169, advanced_put_connect_to_administer_server, i16),
    (173, advanced_put_video_playback_mode, u32),
    (175, advanced_put_enable_super_pan, i16),
    (179, advanced_put_negotiate_security_layer, i16),
    (181, advanced_put_audio_quality_mode, u32),
);

advanced_get_not_implemented!(
    (70, advanced_get_min_input_send_interval, i32),
    (76, advanced_get_keep_alive_interval, i32),
    (92, advanced_get_connect_to_server_console, i16),
    (94, advanced_get_bitmap_persistence, i32),
    (96, advanced_get_minutes_to_idle_timeout, i32),
    (117, advanced_get_redirect_printers, i16),
    (119, advanced_get_redirect_ports, i16),
    (151, advanced_get_redirect_devices, i16),
    (170, advanced_get_connect_to_administer_server, i16),
    (174, advanced_get_video_playback_mode, u32),
    (176, advanced_get_enable_super_pan, i16),
    (180, advanced_get_negotiate_security_layer, i16),
    (182, advanced_get_audio_quality_mode, u32),
);

unsafe extern "system" fn advanced_get_load_balance_info(_this: *mut c_void, value: BstrOut) -> HRESULT {
    if let Err(error) = write_out(value, ptr::null()) {
        return error.code();
    }
    trace_host_call("E_NOTIMPL:AdvancedSettings::slot_113");
    E_NOTIMPL
}

unsafe extern "system" fn advanced_get_authentication_type(_this: *mut c_void, value: *mut u32) -> HRESULT {
    // IronRDP does not negotiate an MSTSC authentication-type UI value.
    // Zero is the documented "none" value, so hosts can safely omit that UI.
    match write_out(value, 0) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_put_authentication_level(this: *mut c_void, value: u32) -> HRESULT {
    trace_host_call("IMsRdpClientAdvancedSettings4::put_AuthenticationLevel");
    if value > 3 {
        return E_INVALIDARG;
    }
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let mut settings = object.settings.borrow_mut();
    if settings.connection_settings_sealed {
        return E_FAIL;
    }
    settings.authentication_level = value;
    settings.authentication_level_set = true;
    S_OK
}

unsafe extern "system" fn advanced_get_authentication_level(this: *mut c_void, value: *mut u32) -> HRESULT {
    trace_host_call("IMsRdpClientAdvancedSettings4::get_AuthenticationLevel");
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    write_out(value, object.settings.borrow().authentication_level).map_or_else(|error| error.code(), |_| S_OK)
}

unsafe extern "system" fn advanced_put_public_mode(this: *mut c_void, value: i16) -> HRESULT {
    let value = match normalize_variant_bool(value) {
        Ok(value) => value,
        Err(error) => return error.code(),
    };
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let mut settings = object.settings.borrow_mut();
    if settings.connection_settings_sealed {
        return E_FAIL;
    }
    settings.public_mode = value == VARIANT_TRUE.0;
    S_OK
}

unsafe extern "system" fn advanced_get_public_mode(this: *mut c_void, value: *mut i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    write_out(
        value,
        if object.settings.borrow().public_mode {
            VARIANT_TRUE.0
        } else {
            VARIANT_FALSE.0
        },
    )
    .map_or_else(|error| error.code(), |_| S_OK)
}

unsafe extern "system" fn advanced_get_pcb(_this: *mut c_void, value: BstrOut) -> HRESULT {
    if let Err(error) = write_out(value, ptr::null()) {
        return error.code();
    }
    trace_host_call("E_NOTIMPL:AdvancedSettings::slot_160");
    E_NOTIMPL
}

unsafe extern "system" fn advanced_put_hotkey_focus_release_left(_this: *mut c_void, _value: i32) -> HRESULT {
    trace_host_call("E_NOTIMPL:AdvancedSettings::HotKeyFocusReleaseLeft");
    E_NOTIMPL
}

unsafe extern "system" fn advanced_get_hotkey_focus_release_left(_this: *mut c_void, value: *mut i32) -> HRESULT {
    match write_out(value, 0) {
        Ok(()) => {
            trace_host_call("E_NOTIMPL:AdvancedSettings::HotKeyFocusReleaseLeft");
            E_NOTIMPL
        }
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_put_hotkey_focus_release_right(_this: *mut c_void, _value: i32) -> HRESULT {
    trace_host_call("E_NOTIMPL:AdvancedSettings::HotKeyFocusReleaseRight");
    E_NOTIMPL
}

unsafe extern "system" fn advanced_get_hotkey_focus_release_right(_this: *mut c_void, value: *mut i32) -> HRESULT {
    match write_out(value, 0) {
        Ok(()) => {
            trace_host_call("E_NOTIMPL:AdvancedSettings::HotKeyFocusReleaseRight");
            E_NOTIMPL
        }
        Err(error) => error.code(),
    }
}

fn dispatch_vtable<const SLOTS: usize>() -> IDispatch_Vtbl {
    IDispatch_Vtbl {
        base__: IUnknown_Vtbl {
            QueryInterface: settings_query_interface::<SLOTS>,
            AddRef: settings_add_ref::<SLOTS>,
            Release: settings_release::<SLOTS>,
        },
        GetTypeInfoCount: settings_get_type_info_count::<SLOTS>,
        GetTypeInfo: settings_get_type_info::<SLOTS>,
        GetIDsOfNames: settings_get_ids_of_names::<SLOTS>,
        Invoke: settings_invoke::<SLOTS>,
    }
}

type AdvancedSettingsObject = CompatibilitySettingsObject<191>;
type SecuredSettingsObject = CompatibilitySettingsObject<SECURED_SETTINGS_SLOTS>;
type TransportSettingsObject = CompatibilitySettingsObject<TRANSPORT_SETTINGS_SLOTS>;

unsafe extern "system" fn advanced_put_container_handled_fullscreen(this: *mut c_void, value: i32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    object.settings.borrow_mut().container_handled_fullscreen = value;
    HRESULT(0)
}

unsafe extern "system" fn advanced_get_container_handled_fullscreen(this: *mut c_void, value: *mut i32) -> HRESULT {
    if value.is_null() {
        return E_POINTER;
    }
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    unsafe { *value = object.settings.borrow().container_handled_fullscreen };
    HRESULT(0)
}

unsafe extern "system" fn advanced_put_allow_background_input(this: *mut c_void, value: i32) -> HRESULT {
    if value != 0 && value != 1 && value != -1 {
        return E_INVALIDARG;
    }
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    object.settings.borrow_mut().allow_background_input = value;
    S_OK
}

unsafe extern "system" fn advanced_get_allow_background_input(this: *mut c_void, value: *mut i32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    match write_out(value, object.settings.borrow().allow_background_input) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

fn normalize_variant_bool(value: i16) -> Result<i16> {
    match value {
        value if value == VARIANT_FALSE.0 => Ok(VARIANT_FALSE.0),
        value if value == VARIANT_TRUE.0 => Ok(VARIANT_TRUE.0),
        _ => Err(Error::from_hresult(E_INVALIDARG)),
    }
}

unsafe extern "system" fn advanced_put_display_connection_bar(this: *mut c_void, value: i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let renderer = {
        let mut settings = object.settings.borrow_mut();
        settings.display_connection_bar = if value == VARIANT_FALSE.0 {
            VARIANT_FALSE.0
        } else {
            VARIANT_TRUE.0
        };
        settings.display_connection_bar_set = true;
        settings.renderer_window
    };
    request_connection_bar_update(renderer);
    S_OK
}

unsafe extern "system" fn advanced_get_display_connection_bar(this: *mut c_void, value: *mut i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    match write_out(value, object.settings.borrow().display_connection_bar) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_put_pin_connection_bar(this: *mut c_void, value: i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let renderer = {
        let mut settings = object.settings.borrow_mut();
        settings.pin_connection_bar = if value == VARIANT_FALSE.0 {
            VARIANT_FALSE.0
        } else {
            VARIANT_TRUE.0
        };
        settings.renderer_window
    };
    request_connection_bar_update(renderer);
    S_OK
}

unsafe extern "system" fn advanced_get_pin_connection_bar(this: *mut c_void, value: *mut i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    match write_out(value, object.settings.borrow().pin_connection_bar) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_put_connection_bar_show_minimize_button(this: *mut c_void, value: i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let renderer = {
        let mut settings = object.settings.borrow_mut();
        settings.connection_bar_show_minimize_button = if value == VARIANT_FALSE.0 {
            VARIANT_FALSE.0
        } else {
            VARIANT_TRUE.0
        };
        settings.renderer_window
    };
    request_connection_bar_update(renderer);
    S_OK
}

unsafe extern "system" fn advanced_get_connection_bar_show_minimize_button(
    this: *mut c_void,
    value: *mut i16,
) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    match write_out(value, object.settings.borrow().connection_bar_show_minimize_button) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_put_connection_bar_show_restore_button(this: *mut c_void, value: i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let renderer = {
        let mut settings = object.settings.borrow_mut();
        settings.connection_bar_show_restore_button = if value == VARIANT_FALSE.0 {
            VARIANT_FALSE.0
        } else {
            VARIANT_TRUE.0
        };
        settings.renderer_window
    };
    request_connection_bar_update(renderer);
    S_OK
}

unsafe extern "system" fn advanced_get_connection_bar_show_restore_button(
    this: *mut c_void,
    value: *mut i16,
) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    match write_out(value, object.settings.borrow().connection_bar_show_restore_button) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_put_connection_bar_show_pin_button(this: *mut c_void, value: i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let renderer = {
        let mut settings = object.settings.borrow_mut();
        settings.connection_bar_show_pin_button = if value == VARIANT_FALSE.0 {
            VARIANT_FALSE.0
        } else {
            VARIANT_TRUE.0
        };
        settings.renderer_window
    };
    request_connection_bar_update(renderer);
    S_OK
}

unsafe extern "system" fn advanced_get_connection_bar_show_pin_button(this: *mut c_void, value: *mut i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    match write_out(value, object.settings.borrow().connection_bar_show_pin_button) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_put_keyboard_layout_str(this: *mut c_void, value: Bstr) -> HRESULT {
    let value = match string_from_bstr(value) {
        Ok(value) => value,
        Err(error) => return error.code(),
    };
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return E_INVALIDARG;
    }
    let keyboard_layout = match u32::from_str_radix(&value, 16) {
        Ok(keyboard_layout) => keyboard_layout,
        Err(_) => return E_INVALIDARG,
    };
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let mut settings = object.settings.borrow_mut();
    settings.keyboard_layout = keyboard_layout;
    mark_compatibility_persistence_dirty(&settings);
    S_OK
}

unsafe extern "system" fn advanced_put_smart_sizing(this: *mut c_void, value: i16) -> HRESULT {
    trace_host_call("IMsRdpClientAdvancedSettings::put_SmartSizing");
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let renderer_window = {
        let mut settings = object.settings.borrow_mut();
        settings.smart_sizing = value != 0;
        mark_compatibility_persistence_dirty(&settings);
        settings.renderer_window
    };
    invalidate_renderer(renderer_window);
    HRESULT(0)
}

unsafe extern "system" fn advanced_get_smart_sizing(this: *mut c_void, value: *mut i16) -> HRESULT {
    trace_host_call("IMsRdpClientAdvancedSettings::get_SmartSizing");
    if value.is_null() {
        return E_POINTER;
    }
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    unsafe { *value = i16::from(object.settings.borrow().smart_sizing) };
    HRESULT(0)
}

unsafe extern "system" fn advanced_put_grab_focus_on_connect(this: *mut c_void, value: i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    object.settings.borrow_mut().grab_focus_on_connect = value != VARIANT_FALSE.0;
    S_OK
}

unsafe extern "system" fn advanced_get_grab_focus_on_connect(this: *mut c_void, value: *mut i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    match write_out(
        value,
        if object.settings.borrow().grab_focus_on_connect {
            VARIANT_TRUE.0
        } else {
            VARIANT_FALSE.0
        },
    ) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_put_credssp(this: *mut c_void, value: i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    object.settings.borrow_mut().enable_credssp = Some(value != 0);
    HRESULT(0)
}

unsafe extern "system" fn advanced_get_credssp(this: *mut c_void, value: *mut i16) -> HRESULT {
    if value.is_null() {
        return E_POINTER;
    }
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    unsafe { *value = i16::from(object.settings.borrow().enable_credssp.unwrap_or(true)) };
    HRESULT(0)
}

unsafe extern "system" fn advanced_put_compress(this: *mut c_void, value: i32) -> HRESULT {
    let enabled = match value {
        0 => false,
        1 => true,
        _ => return E_INVALIDARG,
    };
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    object.settings.borrow_mut().compression = Some(enabled);
    S_OK
}

unsafe extern "system" fn advanced_get_compress(this: *mut c_void, value: *mut i32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    match write_out(value, i32::from(object.settings.borrow().compression.unwrap_or(true))) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_put_clear_text_password(this: *mut c_void, password: Bstr) -> HRESULT {
    let password = match string_from_bstr(password) {
        Ok(password) => password,
        Err(error) => return error.code(),
    };
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    object.settings.borrow_mut().clear_text_password = Some(password);
    S_OK
}

unsafe extern "system" fn advanced_put_rdp_port(this: *mut c_void, value: i32) -> HRESULT {
    let port = match u16::try_from(value) {
        Ok(port) if port != 0 => port,
        _ => return E_INVALIDARG,
    };
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    object.settings.borrow_mut().rdp_port = Some(port);
    S_OK
}

unsafe extern "system" fn advanced_get_rdp_port(this: *mut c_void, value: *mut i32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    match write_out(value, i32::from(object.settings.borrow().rdp_port.unwrap_or(3389))) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

fn keyboard_type_from_raw(value: i32) -> Result<KeyboardType> {
    match value {
        1 => Ok(KeyboardType::IbmPcXt),
        2 => Ok(KeyboardType::OlivettiIco),
        3 => Ok(KeyboardType::IbmPcAt),
        4 => Ok(KeyboardType::IbmEnhanced),
        5 => Ok(KeyboardType::Nokia1050),
        6 => Ok(KeyboardType::Nokia9140),
        7 => Ok(KeyboardType::Japanese),
        _ => Err(Error::from_hresult(E_INVALIDARG)),
    }
}

unsafe extern "system" fn advanced_put_keyboard_type(this: *mut c_void, value: i32) -> HRESULT {
    let keyboard_type = match keyboard_type_from_raw(value) {
        Ok(keyboard_type) => keyboard_type,
        Err(error) => return error.code(),
    };
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    object.settings.borrow_mut().keyboard_type = keyboard_type;
    S_OK
}

unsafe extern "system" fn advanced_get_keyboard_type(this: *mut c_void, out: *mut i32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let value = match i32::try_from(object.settings.borrow().keyboard_type.as_u32()) {
        Ok(value) => value,
        Err(_) => return E_FAIL,
    };
    match write_out(out, value) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_put_keyboard_subtype(this: *mut c_void, value: i32) -> HRESULT {
    let keyboard_subtype = match u32::try_from(value) {
        Ok(keyboard_subtype) => keyboard_subtype,
        Err(_) => return E_INVALIDARG,
    };
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    object.settings.borrow_mut().keyboard_subtype = keyboard_subtype;
    S_OK
}

unsafe extern "system" fn advanced_get_keyboard_subtype(this: *mut c_void, out: *mut i32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let value = match i32::try_from(object.settings.borrow().keyboard_subtype) {
        Ok(value) => value,
        Err(_) => return E_FAIL,
    };
    match write_out(out, value) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_put_keyboard_function_key(this: *mut c_void, value: i32) -> HRESULT {
    let keyboard_functional_keys_count = match u32::try_from(value) {
        Ok(keyboard_functional_keys_count) => keyboard_functional_keys_count,
        Err(_) => return E_INVALIDARG,
    };
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    object.settings.borrow_mut().keyboard_functional_keys_count = keyboard_functional_keys_count;
    S_OK
}

unsafe extern "system" fn advanced_get_keyboard_function_key(this: *mut c_void, out: *mut i32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let value = match i32::try_from(object.settings.borrow().keyboard_functional_keys_count) {
        Ok(value) => value,
        Err(_) => return E_FAIL,
    };
    match write_out(out, value) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_put_disable_rdpdr(this: *mut c_void, value: i32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let mut settings = object.settings.borrow_mut();
    if settings.connection_settings_sealed {
        return E_FAIL;
    }
    settings.disable_rdpdr = value != 0;
    mark_compatibility_persistence_dirty(&settings);
    S_OK
}

unsafe extern "system" fn advanced_get_disable_rdpdr(this: *mut c_void, output: *mut i32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let value = i32::from(object.settings.borrow().disable_rdpdr);
    write_out(output, value).map_or_else(|error| error.code(), |_| S_OK)
}

unsafe extern "system" fn advanced_put_redirect_drives(this: *mut c_void, value: i16) -> HRESULT {
    let value = match normalize_variant_bool(value) {
        Ok(value) => value == VARIANT_TRUE.0,
        Err(error) => return error.code(),
    };
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let catalog = {
        let mut settings = object.settings.borrow_mut();
        if settings.connection_settings_sealed {
            return E_FAIL;
        }
        settings.redirect_drives = value;
        mark_compatibility_persistence_dirty(&settings);
        Rc::clone(&settings.drive_catalog)
    };
    catalog.borrow().set_redirection_state(value);
    S_OK
}

unsafe extern "system" fn advanced_get_redirect_drives(this: *mut c_void, value: *mut i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    write_out(
        value,
        if object.settings.borrow().redirect_drives {
            VARIANT_TRUE.0
        } else {
            VARIANT_FALSE.0
        },
    )
    .map_or_else(|error| error.code(), |_| S_OK)
}

unsafe extern "system" fn advanced_put_redirect_smart_cards(this: *mut c_void, value: i16) -> HRESULT {
    let value = match normalize_variant_bool(value) {
        Ok(value) => value == VARIANT_TRUE.0,
        Err(error) => return error.code(),
    };
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let mut settings = object.settings.borrow_mut();
    if settings.connection_settings_sealed {
        return E_FAIL;
    }
    settings.redirect_smart_cards = value;
    mark_compatibility_persistence_dirty(&settings);
    S_OK
}

unsafe extern "system" fn advanced_get_redirect_smart_cards(this: *mut c_void, value: *mut i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    write_out(
        value,
        if object.settings.borrow().redirect_smart_cards {
            VARIANT_TRUE.0
        } else {
            VARIANT_FALSE.0
        },
    )
    .map_or_else(|error| error.code(), |_| S_OK)
}

unsafe extern "system" fn advanced_put_enable_mouse(this: *mut c_void, value: i32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    object.settings.borrow_mut().enable_mouse = value != 0;
    S_OK
}

unsafe extern "system" fn advanced_get_enable_mouse(this: *mut c_void, value: *mut i32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    write_out(value, i32::from(object.settings.borrow().enable_mouse)).map_or_else(|error| error.code(), |_| S_OK)
}

unsafe extern "system" fn advanced_put_enable_windows_key(this: *mut c_void, value: i32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    object.settings.borrow_mut().enable_windows_key = value != 0;
    S_OK
}

unsafe extern "system" fn advanced_get_enable_windows_key(this: *mut c_void, value: *mut i32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    write_out(value, i32::from(object.settings.borrow().enable_windows_key)).map_or_else(|error| error.code(), |_| S_OK)
}

unsafe extern "system" fn advanced_put_performance_flags(this: *mut c_void, value: i32) -> HRESULT {
    let Some(flags) = PerformanceFlags::from_bits(value as u32) else {
        return E_INVALIDARG;
    };
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let mut settings = object.settings.borrow_mut();
    settings.performance_flags = flags;
    mark_compatibility_persistence_dirty(&settings);
    S_OK
}

unsafe extern "system" fn advanced_get_performance_flags(this: *mut c_void, value: *mut i32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    match i32::try_from(object.settings.borrow().performance_flags.bits()) {
        Ok(flags) => write_out(value, flags).map_or_else(|error| error.code(), |_| S_OK),
        Err(_) => E_FAIL,
    }
}

unsafe extern "system" fn advanced_put_redirect_clipboard(this: *mut c_void, value: i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    object.settings.borrow_mut().redirect_clipboard = value != VARIANT_FALSE.0;
    S_OK
}

unsafe extern "system" fn advanced_put_enable_auto_reconnect(this: *mut c_void, value: i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let mut settings = object.settings.borrow_mut();
    if settings.connection_settings_sealed {
        return E_FAIL;
    }
    settings.enable_auto_reconnect = value != VARIANT_FALSE.0;
    S_OK
}

unsafe extern "system" fn advanced_get_enable_auto_reconnect(this: *mut c_void, value: *mut i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    match write_out(
        value,
        if object.settings.borrow().enable_auto_reconnect {
            VARIANT_TRUE.0
        } else {
            VARIANT_FALSE.0
        },
    ) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_put_max_reconnect_attempts(this: *mut c_void, value: i32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let mut settings = object.settings.borrow_mut();
    if settings.connection_settings_sealed {
        return E_FAIL;
    }
    let Ok(value) = u32::try_from(value) else {
        return E_INVALIDARG;
    };
    if value > MAX_RECONNECT_ATTEMPTS {
        return E_INVALIDARG;
    }
    settings.max_reconnect_attempts = value;
    S_OK
}

unsafe extern "system" fn advanced_get_max_reconnect_attempts(this: *mut c_void, value: *mut i32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let result = i32::try_from(object.settings.borrow().max_reconnect_attempts)
        .map_err(|_| Error::from_hresult(E_FAIL))
        .and_then(|configured| write_out(value, configured));
    match result {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

fn set_audio_redirection_mode(settings: &mut CompatibilitySettings, value: u32) -> Result<()> {
    if value > 2 {
        return Err(Error::from_hresult(E_INVALIDARG));
    }
    settings.audio_redirection_mode = i32::try_from(value).map_err(|_| Error::from_hresult(E_FAIL))?;
    Ok(())
}

fn audio_mode_from_raw(value: i32) -> Result<AudioMode> {
    match value {
        0 => Ok(AudioMode::RedirectToClient),
        1 => Ok(AudioMode::PlayOnServer),
        2 => Ok(AudioMode::Disabled),
        _ => Err(Error::from_hresult(E_FAIL)),
    }
}

fn should_prompt_for_credentials(server: &str, has_password: bool, prompt_for_credentials: bool) -> bool {
    prompt_for_credentials && !has_password && !server.trim().is_empty()
}

fn keyboard_hooks_apply_remotely(mode: i32, fullscreen: bool) -> bool {
    match mode {
        0 => false,
        1 => true,
        2 => fullscreen,
        _ => false,
    }
}

fn should_forward_windows_key(
    compatibility: &CompatibilitySettings,
    fullscreen: bool,
    input_database: &InputDatabase,
    message: u32,
    scancode: Scancode,
) -> bool {
    let (extended, code) = scancode.as_u8();
    let is_windows_key = extended && matches!(code, 0x5b | 0x5c);
    if !is_windows_key {
        return true;
    }

    // Preserve a release for a key forwarded before the host policy changed.
    compatibility.enable_windows_key && keyboard_hooks_apply_remotely(compatibility.keyboard_hook_mode, fullscreen)
        || matches!(message, WM_KEYUP | WM_SYSKEYUP) && input_database.is_key_pressed(scancode)
}

fn is_fullscreen_hotkey(virtual_key: VIRTUAL_KEY, control_and_alt_pressed: bool) -> bool {
    control_and_alt_pressed && matches!(virtual_key, VK_CANCEL | VK_PAUSE)
}

#[derive(Clone)]
enum ActiveXTransport {
    Direct,
    Gateway {
        endpoint: String,
        username: String,
        password: String,
    },
    RDCleanPath(RDCleanPathConfig),
}

fn active_x_transport_from_client_transport(
    transport: &Transport,
) -> core::result::Result<ActiveXTransport, &'static str> {
    match transport {
        Transport::Direct => Ok(ActiveXTransport::Direct),
        Transport::Gateway(gateway) => Ok(ActiveXTransport::Gateway {
            endpoint: gateway.endpoint.clone(),
            username: gateway.username.clone(),
            password: gateway.password.clone(),
        }),
        Transport::RDCleanPath(rdcleanpath) => Ok(ActiveXTransport::RDCleanPath(rdcleanpath.clone())),
        // Named-pipe RDP (e.g. Windows Sandbox) is agent/desktop-client only.
        Transport::NamedPipe { .. } => Err("Windows named-pipe transport is not supported by the ActiveX host"),
    }
}

fn rdcleanpath_rpc_client_properties(properties: &PropertySet) -> core::result::Result<PropertySet, &'static str> {
    let has_legacy_property = properties
        .iter()
        .any(|(key, _)| matches!(key.as_ref(), "ironrdp_rdcleanpathurl" | "ironrdp_rdcleanpathtoken"));
    if has_legacy_property {
        return Err("use RDCleanPathUrl and RDCleanPathToken for ActiveX RPC connections");
    }

    let url = properties.get::<&str>(ACTIVEX_RDCLEANPATH_URL_PROPERTY);
    let token = properties.get::<&str>(ACTIVEX_RDCLEANPATH_TOKEN_PROPERTY);
    let has_url = properties
        .iter()
        .any(|(key, _)| key.as_ref() == ACTIVEX_RDCLEANPATH_URL_PROPERTY);
    let has_token = properties
        .iter()
        .any(|(key, _)| key.as_ref() == ACTIVEX_RDCLEANPATH_TOKEN_PROPERTY);

    if has_url && url.is_none() {
        return Err("RDCleanPathUrl must be a string");
    }
    if has_token && token.is_none() {
        return Err("RDCleanPathToken must be a string");
    }

    match (url, token) {
        (Some(_), None) => Err("RDCleanPathToken is required when RDCleanPathUrl is configured"),
        (None, Some(_)) => Err("RDCleanPathUrl is required when RDCleanPathToken is configured"),
        (_, Some("")) => Err("RDCleanPathToken must not be empty"),
        (Some(url), Some(token)) => {
            let mut client_properties = properties.clone();
            client_properties.insert("ironrdp_rdcleanpathurl", url.to_owned());
            client_properties.insert("ironrdp_rdcleanpathtoken", token.to_owned());
            Ok(client_properties)
        }
        (None, None) => Ok(properties.clone()),
    }
}

fn domain_qualified_username(domain: &str, username: &str) -> String {
    if domain.is_empty() || username.contains('\\') || username.contains('@') {
        username.to_owned()
    } else {
        format!("{domain}\\{username}")
    }
}

fn active_x_transport(settings: &Settings, compatibility: &CompatibilitySettings) -> Result<ActiveXTransport> {
    let usage_method = GatewayUsageMethod::try_from(i64::from(compatibility.gateway_usage_method))
        .map_err(|error| Error::new(E_INVALIDARG, error.to_string()))?;
    let use_gateway = match usage_method {
        GatewayUsageMethod::Direct | GatewayUsageMethod::DirectBypassLocal => false,
        GatewayUsageMethod::UseAlways => true,
        // IronRDP has no direct-then-gateway fallback. Match its .rdp behavior by selecting an
        // explicitly supplied gateway eagerly for Detect mode.
        GatewayUsageMethod::Detect => !compatibility.gateway_hostname.is_empty(),
        GatewayUsageMethod::UseDefaultSettings => {
            return Err(Error::from_hresult(E_NOTIMPL));
        }
    };
    if !use_gateway {
        return Ok(ActiveXTransport::Direct);
    }

    if compatibility.gateway_hostname.trim().is_empty() {
        return Err(Error::new(
            E_INVALIDARG,
            "set GatewayHostname before using an RD Gateway",
        ));
    }

    let credentials_source = GatewayCredentialsSource::try_from(i64::from(compatibility.gateway_creds_source))
        .map_err(|error| Error::new(E_INVALIDARG, error.to_string()))?;
    let (username, password) = match credentials_source {
        GatewayCredentialsSource::UseServerCredentials => (
            domain_qualified_username(&settings.domain, &settings.username),
            settings
                .password
                .clone()
                .ok_or_else(|| Error::new(E_INVALIDARG, "set IronRdpPassword before connecting"))?,
        ),
        GatewayCredentialsSource::UseUserCredentials => (
            domain_qualified_username(&compatibility.gateway_domain, &compatibility.gateway_username),
            compatibility.gateway_password.clone(),
        ),
        GatewayCredentialsSource::UseProfile
        | GatewayCredentialsSource::Prompt
        | GatewayCredentialsSource::SmartCard
        | GatewayCredentialsSource::UseLogonCredentials => return Err(Error::from_hresult(E_NOTIMPL)),
    };
    if username.trim().is_empty() || password.is_empty() {
        return Err(Error::new(
            E_INVALIDARG,
            "set gateway credentials before using an RD Gateway",
        ));
    }

    Ok(ActiveXTransport::Gateway {
        endpoint: compatibility.gateway_hostname.clone(),
        username,
        password,
    })
}

unsafe extern "system" fn advanced_put_audio_redirection(this: *mut c_void, value: u32) -> HRESULT {
    trace_host_call("IMsRdpClientAdvancedSettings5::put_AudioRedirectionMode");
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    match set_audio_redirection_mode(&mut object.settings.borrow_mut(), value) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_get_audio_redirection(this: *mut c_void, out: *mut u32) -> HRESULT {
    trace_host_call("IMsRdpClientAdvancedSettings5::get_AudioRedirectionMode");
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let value = match u32::try_from(object.settings.borrow().audio_redirection_mode) {
        Ok(value) => value,
        Err(_) => return E_FAIL,
    };
    match write_out(out, value) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_put_audio_capture_redirection_mode(this: *mut c_void, value: i16) -> HRESULT {
    trace_host_call("IMsRdpClientAdvancedSettings6::put_AudioCaptureRedirectionMode");
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    // MSTSC documents VARIANT_BOOL; treat any non-zero value as enable.
    object.settings.borrow_mut().audio_capture_redirection_mode = if value == VARIANT_FALSE.0 {
        VARIANT_FALSE.0
    } else {
        VARIANT_TRUE.0
    };
    S_OK
}

unsafe extern "system" fn advanced_get_audio_capture_redirection_mode(this: *mut c_void, out: *mut i16) -> HRESULT {
    trace_host_call("IMsRdpClientAdvancedSettings6::get_AudioCaptureRedirectionMode");
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    match write_out(out, object.settings.borrow().audio_capture_redirection_mode) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_get_redirect_clipboard(this: *mut c_void, value: *mut i16) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    match write_out(
        value,
        if object.settings.borrow().redirect_clipboard {
            VARIANT_TRUE.0
        } else {
            VARIANT_FALSE.0
        },
    ) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_put_redirect_directx(_this: *mut c_void, value: i16) -> HRESULT {
    trace_host_call("IMsRdpClientAdvancedSettings7::put_RedirectDirectX");
    if value == VARIANT_FALSE.0 { S_OK } else { E_NOTIMPL }
}

unsafe extern "system" fn advanced_get_redirect_directx(_this: *mut c_void, value: *mut i16) -> HRESULT {
    trace_host_call("IMsRdpClientAdvancedSettings7::get_RedirectDirectX");
    match write_out(value, VARIANT_FALSE.0) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn advanced_put_network_connection_type(this: *mut c_void, value: u32) -> HRESULT {
    let connection_type = match value {
        1 => ConnectionType::Modem,
        2 => ConnectionType::BroadbandLow,
        3 => ConnectionType::Satellite,
        4 => ConnectionType::BroadbandHigh,
        5 => ConnectionType::Wan,
        6 => ConnectionType::Lan,
        7 => ConnectionType::Autodetect,
        _ => return E_INVALIDARG,
    };
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    object.settings.borrow_mut().network_connection_type = connection_type;
    S_OK
}

unsafe extern "system" fn advanced_get_network_connection_type(this: *mut c_void, value: *mut u32) -> HRESULT {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let connection_type = match object.settings.borrow().network_connection_type {
        ConnectionType::Modem => 1,
        ConnectionType::BroadbandLow => 2,
        ConnectionType::Satellite => 3,
        ConnectionType::BroadbandHigh => 4,
        ConnectionType::Wan => 5,
        ConnectionType::Lan => 6,
        ConnectionType::Autodetect => 7,
        ConnectionType::NotUsed => return E_FAIL,
    };
    write_out(value, connection_type).map_or_else(|error| error.code(), |_| S_OK)
}

unsafe extern "system" fn advanced_put_bandwidth_detection(_this: *mut c_void, _value: i16) -> HRESULT {
    trace_host_call("IMsRdpClientAdvancedSettings8::put_BandwidthDetection");
    E_NOTIMPL
}

unsafe extern "system" fn advanced_get_bandwidth_detection(_this: *mut c_void, value: *mut i16) -> HRESULT {
    trace_host_call("IMsRdpClientAdvancedSettings8::get_BandwidthDetection");
    if let Err(error) = write_out(value, VARIANT_FALSE.0) {
        return error.code();
    }
    E_NOTIMPL
}

unsafe extern "system" fn advanced_put_client_protocol_spec(_this: *mut c_void, _value: i32) -> HRESULT {
    trace_host_call("IMsRdpClientAdvancedSettings8::put_ClientProtocolSpec");
    E_NOTIMPL
}

unsafe extern "system" fn advanced_get_client_protocol_spec(_this: *mut c_void, value: *mut i32) -> HRESULT {
    trace_host_call("IMsRdpClientAdvancedSettings8::get_ClientProtocolSpec");
    if let Err(error) = write_out(value, 0) {
        return error.code();
    }
    E_NOTIMPL
}

unsafe extern "system" fn secured_get_keyboard_hook(this: *mut c_void, value: *mut i32) -> HRESULT {
    trace_host_call("IMsRdpClientSecuredSettings::get_KeyboardHookMode");
    if value.is_null() {
        return E_POINTER;
    }
    let object = unsafe { &*(this.cast::<SecuredSettingsObject>()) };
    unsafe { *value = object.settings.borrow().keyboard_hook_mode };
    HRESULT(0)
}

unsafe extern "system" fn secured_put_keyboard_hook(this: *mut c_void, value: i32) -> HRESULT {
    trace_host_call("IMsRdpClientSecuredSettings::put_KeyboardHookMode");
    if !(0..=2).contains(&value) {
        return E_INVALIDARG;
    }
    let object = unsafe { &*(this.cast::<SecuredSettingsObject>()) };
    object.settings.borrow_mut().keyboard_hook_mode = value;
    HRESULT(0)
}

unsafe extern "system" fn secured_put_start_program(this: *mut c_void, value: Bstr) -> HRESULT {
    trace_host_call("IMsTscSecuredSettings::put_StartProgram");
    let object = unsafe { &*(this.cast::<SecuredSettingsObject>()) };
    let bridge_enabled = native_mstsc_credential_bridge_enabled();
    trace_host_call(if bridge_enabled {
        "NativeMstscCredentialBridge::StartProgramBridgeEnabled"
    } else {
        "NativeMstscCredentialBridge::StartProgramBridgeDisabled"
    });
    if bridge_enabled {
        trace_host_call(if object.native_mstsc_credential_bridge.is_some() {
            "NativeMstscCredentialBridge::StartProgramBridgeAttached"
        } else {
            "NativeMstscCredentialBridge::StartProgramBridgeUnavailable"
        });
    }
    if let Some(bridge) = object.native_mstsc_credential_bridge.as_ref() {
        match bridge.intercept_start_program() {
            NativeMstscStartProgramIntercept::NotHandled => {}
            NativeMstscStartProgramIntercept::Handled => {
                // Native mstsc has been observed passing a preflight payload that is not a valid BSTR.
                // The bridge uses this call solely as its explicit prompt trigger, so it must take
                // ownership before deserializing that unrelated payload.
                return E_INVALIDARG;
            }
        }
    }

    let value = match string_from_bstr(value) {
        Ok(value) => value,
        Err(error) => return error.code(),
    };
    object.settings.borrow_mut().secured_start_program = value;
    S_OK
}

unsafe extern "system" fn secured_get_start_program(this: *mut c_void, value: BstrOut) -> HRESULT {
    trace_host_call("IMsTscSecuredSettings::get_StartProgram");
    let object = unsafe { &*(this.cast::<SecuredSettingsObject>()) };
    match write_bstr(value, &object.settings.borrow().secured_start_program) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn secured_put_work_dir(this: *mut c_void, value: Bstr) -> HRESULT {
    trace_host_call("IMsTscSecuredSettings::put_WorkDir");
    let value = match string_from_bstr(value) {
        Ok(value) => value,
        Err(error) => return error.code(),
    };
    let object = unsafe { &*(this.cast::<SecuredSettingsObject>()) };
    object.settings.borrow_mut().secured_work_dir = value;
    S_OK
}

unsafe extern "system" fn secured_get_work_dir(this: *mut c_void, value: BstrOut) -> HRESULT {
    trace_host_call("IMsTscSecuredSettings::get_WorkDir");
    let object = unsafe { &*(this.cast::<SecuredSettingsObject>()) };
    match write_bstr(value, &object.settings.borrow().secured_work_dir) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn secured_put_fullscreen(this: *mut c_void, value: i32) -> HRESULT {
    trace_host_call("IMsTscSecuredSettings::put_FullScreen");
    let object = unsafe { &*(this.cast::<SecuredSettingsObject>()) };
    object.settings.borrow_mut().secured_fullscreen = value;
    S_OK
}

unsafe extern "system" fn secured_get_fullscreen(this: *mut c_void, value: *mut i32) -> HRESULT {
    trace_host_call("IMsTscSecuredSettings::get_FullScreen");
    let object = unsafe { &*(this.cast::<SecuredSettingsObject>()) };
    match write_out(value, object.settings.borrow().secured_fullscreen) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn secured_put_audio_redirection(this: *mut c_void, value: i32) -> HRESULT {
    trace_host_call("IMsRdpClientSecuredSettings::put_AudioRedirectionMode");
    let value = match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => return E_INVALIDARG,
    };
    let object = unsafe { &*(this.cast::<SecuredSettingsObject>()) };
    match set_audio_redirection_mode(&mut object.settings.borrow_mut(), value) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn secured_get_audio_redirection(this: *mut c_void, value: *mut i32) -> HRESULT {
    trace_host_call("IMsRdpClientSecuredSettings::get_AudioRedirectionMode");
    let object = unsafe { &*(this.cast::<SecuredSettingsObject>()) };
    match write_out(value, object.settings.borrow().audio_redirection_mode) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn secured_get_pcb(_this: *mut c_void, value: BstrOut) -> HRESULT {
    if let Err(error) = write_out(value, ptr::null()) {
        return error.code();
    }
    trace_host_call("E_NOTIMPL:SecuredSettings::get_PCB");
    E_NOTIMPL
}

unsafe extern "system" fn secured_put_pcb(_this: *mut c_void, _value: Bstr) -> HRESULT {
    trace_host_call("E_NOTIMPL:SecuredSettings::put_PCB");
    E_NOTIMPL
}

unsafe extern "system" fn transport_put_gateway_username(this: *mut c_void, value: Bstr) -> HRESULT {
    let value = match string_from_bstr(value) {
        Ok(value) => value,
        Err(error) => return error.code(),
    };
    let object = unsafe { &*(this.cast::<TransportSettingsObject>()) };
    object.settings.borrow_mut().gateway_username = value;
    S_OK
}

unsafe extern "system" fn transport_get_gateway_hostname(this: *mut c_void, value: BstrOut) -> HRESULT {
    let object = unsafe { &*(this.cast::<TransportSettingsObject>()) };
    match write_bstr(value, &object.settings.borrow().gateway_hostname) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn transport_put_gateway_usage_method(this: *mut c_void, value: u32) -> HRESULT {
    if GatewayUsageMethod::try_from(i64::from(value)).is_err() {
        return E_INVALIDARG;
    }
    let object = unsafe { &*(this.cast::<TransportSettingsObject>()) };
    object.settings.borrow_mut().gateway_usage_method = value;
    S_OK
}

unsafe extern "system" fn transport_get_gateway_usage_method(this: *mut c_void, value: *mut u32) -> HRESULT {
    let object = unsafe { &*(this.cast::<TransportSettingsObject>()) };
    match write_out(value, object.settings.borrow().gateway_usage_method) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn transport_put_gateway_profile_usage_method(_this: *mut c_void, value: u32) -> HRESULT {
    let _ = value;
    trace_host_call("E_NOTIMPL:TransportSettings::put_GatewayProfileUsageMethod");
    E_NOTIMPL
}

unsafe extern "system" fn transport_get_gateway_profile_usage_method(_this: *mut c_void, value: *mut u32) -> HRESULT {
    if let Err(error) = write_out(value, 0) {
        return error.code();
    }
    trace_host_call("E_NOTIMPL:TransportSettings::get_GatewayProfileUsageMethod");
    E_NOTIMPL
}

unsafe extern "system" fn transport_put_gateway_creds_source(this: *mut c_void, value: u32) -> HRESULT {
    if GatewayCredentialsSource::try_from(i64::from(value)).is_err() {
        return E_INVALIDARG;
    }
    let object = unsafe { &*(this.cast::<TransportSettingsObject>()) };
    object.settings.borrow_mut().gateway_creds_source = value;
    S_OK
}

unsafe extern "system" fn transport_get_gateway_creds_source(this: *mut c_void, value: *mut u32) -> HRESULT {
    let object = unsafe { &*(this.cast::<TransportSettingsObject>()) };
    match write_out(value, object.settings.borrow().gateway_creds_source) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn transport_put_gateway_user_selected_creds_source(_this: *mut c_void, _value: u32) -> HRESULT {
    trace_host_call("E_NOTIMPL:TransportSettings::put_GatewayUserSelectedCredsSource");
    E_NOTIMPL
}

unsafe extern "system" fn transport_get_gateway_user_selected_creds_source(
    _this: *mut c_void,
    value: *mut u32,
) -> HRESULT {
    if let Err(error) = write_out(value, 0) {
        return error.code();
    }
    trace_host_call("E_NOTIMPL:TransportSettings::get_GatewayUserSelectedCredsSource");
    E_NOTIMPL
}

unsafe extern "system" fn transport_get_gateway_is_supported(_this: *mut c_void, value: *mut i32) -> HRESULT {
    match write_out(value, 1) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn transport_get_gateway_default_usage_method(_this: *mut c_void, value: *mut u32) -> HRESULT {
    if let Err(error) = write_out(value, 0) {
        return error.code();
    }
    trace_host_call("E_NOTIMPL:TransportSettings::get_GatewayDefaultUsageMethod");
    E_NOTIMPL
}

unsafe extern "system" fn transport_put_u32_not_implemented(_this: *mut c_void, _value: u32) -> HRESULT {
    trace_host_call("E_NOTIMPL:TransportSettings::put_Extension");
    E_NOTIMPL
}

unsafe extern "system" fn transport_get_u32_not_implemented(_this: *mut c_void, value: *mut u32) -> HRESULT {
    if let Err(error) = write_out(value, 0) {
        return error.code();
    }
    trace_host_call("E_NOTIMPL:TransportSettings::get_Extension");
    E_NOTIMPL
}

unsafe extern "system" fn transport_put_bstr_not_implemented(_this: *mut c_void, _value: Bstr) -> HRESULT {
    trace_host_call("E_NOTIMPL:TransportSettings::put_Extension");
    E_NOTIMPL
}

unsafe extern "system" fn transport_get_bstr_not_implemented(_this: *mut c_void, value: BstrOut) -> HRESULT {
    if let Err(error) = write_out(value, ptr::null()) {
        return error.code();
    }
    trace_host_call("E_NOTIMPL:TransportSettings::get_Extension");
    E_NOTIMPL
}

unsafe extern "system" fn transport_put_gateway_hostname(this: *mut c_void, value: Bstr) -> HRESULT {
    let value = match string_from_bstr(value) {
        Ok(value) => value,
        Err(error) => return error.code(),
    };
    let object = unsafe { &*(this.cast::<TransportSettingsObject>()) };
    object.settings.borrow_mut().gateway_hostname = value;
    S_OK
}

unsafe extern "system" fn transport_get_gateway_username(this: *mut c_void, value: BstrOut) -> HRESULT {
    let object = unsafe { &*(this.cast::<TransportSettingsObject>()) };
    match write_bstr(value, &object.settings.borrow().gateway_username) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn transport_put_gateway_domain(this: *mut c_void, value: Bstr) -> HRESULT {
    let value = match string_from_bstr(value) {
        Ok(value) => value,
        Err(error) => return error.code(),
    };
    let object = unsafe { &*(this.cast::<TransportSettingsObject>()) };
    object.settings.borrow_mut().gateway_domain = value;
    S_OK
}

unsafe extern "system" fn transport_get_gateway_domain(this: *mut c_void, value: BstrOut) -> HRESULT {
    let object = unsafe { &*(this.cast::<TransportSettingsObject>()) };
    match write_bstr(value, &object.settings.borrow().gateway_domain) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn transport_put_gateway_password(this: *mut c_void, value: Bstr) -> HRESULT {
    let value = match string_from_bstr(value) {
        Ok(value) => value,
        Err(error) => return error.code(),
    };
    let object = unsafe { &*(this.cast::<TransportSettingsObject>()) };
    object.settings.borrow_mut().gateway_password = value;
    S_OK
}

fn advanced_vtable() -> &'static CompatibilitySettingsVtable<191> {
    static VTABLE: std::sync::OnceLock<CompatibilitySettingsVtable<191>> = std::sync::OnceLock::new();
    VTABLE.get_or_init(|| {
        let mut slots = advanced_settings_stub_slots();
        slots[0] = advanced_put_compress as *const () as usize;
        slots[1] = advanced_get_compress as *const () as usize;
        slots[4] = advanced_put_allow_background_input as *const () as usize;
        slots[5] = advanced_get_allow_background_input as *const () as usize;
        slots[6] = advanced_put_keyboard_layout_str as *const () as usize;
        slots[7] = advanced_put_plugin_dlls as *const () as usize;
        slots[10] = advanced_put_container_handled_fullscreen as *const () as usize;
        slots[11] = advanced_get_container_handled_fullscreen as *const () as usize;
        slots[12] = advanced_put_disable_rdpdr as *const () as usize;
        slots[13] = advanced_get_disable_rdpdr as *const () as usize;
        slots[28] = advanced_put_rdp_port as *const () as usize;
        slots[29] = advanced_get_rdp_port as *const () as usize;
        slots[30] = advanced_put_enable_mouse as *const () as usize;
        slots[31] = advanced_get_enable_mouse as *const () as usize;
        slots[34] = advanced_put_enable_windows_key as *const () as usize;
        slots[35] = advanced_get_enable_windows_key as *const () as usize;
        slots[69] = advanced_put_min_input_send_interval as *const () as usize;
        slots[70] = advanced_get_min_input_send_interval as *const () as usize;
        slots[75] = advanced_put_keep_alive_interval as *const () as usize;
        slots[76] = advanced_get_keep_alive_interval as *const () as usize;
        slots[83] = advanced_put_keyboard_type as *const () as usize;
        slots[84] = advanced_get_keyboard_type as *const () as usize;
        slots[85] = advanced_put_keyboard_subtype as *const () as usize;
        slots[86] = advanced_get_keyboard_subtype as *const () as usize;
        slots[87] = advanced_put_keyboard_function_key as *const () as usize;
        slots[88] = advanced_get_keyboard_function_key as *const () as usize;
        slots[91] = advanced_put_connect_to_server_console as *const () as usize;
        slots[92] = advanced_get_connect_to_server_console as *const () as usize;
        slots[93] = advanced_put_bitmap_persistence as *const () as usize;
        slots[94] = advanced_get_bitmap_persistence as *const () as usize;
        slots[95] = advanced_put_minutes_to_idle_timeout as *const () as usize;
        slots[96] = advanced_get_minutes_to_idle_timeout as *const () as usize;
        slots[97] = advanced_put_smart_sizing as *const () as usize;
        slots[98] = advanced_get_smart_sizing as *const () as usize;
        slots[105] = advanced_put_clear_text_password as *const () as usize;
        slots[106] = advanced_put_display_connection_bar as *const () as usize;
        slots[107] = advanced_get_display_connection_bar as *const () as usize;
        slots[108] = advanced_put_pin_connection_bar as *const () as usize;
        slots[109] = advanced_get_pin_connection_bar as *const () as usize;
        slots[110] = advanced_put_grab_focus_on_connect as *const () as usize;
        slots[111] = advanced_get_grab_focus_on_connect as *const () as usize;
        slots[112] = advanced_put_load_balance_info as *const () as usize;
        slots[113] = advanced_get_load_balance_info as *const () as usize;
        slots[114] = advanced_put_redirect_drives as *const () as usize;
        slots[115] = advanced_get_redirect_drives as *const () as usize;
        slots[116] = advanced_put_redirect_printers as *const () as usize;
        slots[117] = advanced_get_redirect_printers as *const () as usize;
        slots[118] = advanced_put_redirect_ports as *const () as usize;
        slots[119] = advanced_get_redirect_ports as *const () as usize;
        slots[120] = advanced_put_redirect_smart_cards as *const () as usize;
        slots[121] = advanced_get_redirect_smart_cards as *const () as usize;
        slots[126] = advanced_put_performance_flags as *const () as usize;
        slots[127] = advanced_get_performance_flags as *const () as usize;
        slots[132] = advanced_put_enable_auto_reconnect as *const () as usize;
        slots[133] = advanced_get_enable_auto_reconnect as *const () as usize;
        slots[134] = advanced_put_max_reconnect_attempts as *const () as usize;
        slots[135] = advanced_get_max_reconnect_attempts as *const () as usize;
        slots[136] = advanced_put_connection_bar_show_minimize_button as *const () as usize;
        slots[137] = advanced_get_connection_bar_show_minimize_button as *const () as usize;
        slots[138] = advanced_put_connection_bar_show_restore_button as *const () as usize;
        slots[139] = advanced_get_connection_bar_show_restore_button as *const () as usize;
        slots[140] = advanced_put_authentication_level as *const () as usize;
        slots[141] = advanced_get_authentication_level as *const () as usize;
        slots[142] = advanced_put_redirect_clipboard as *const () as usize;
        slots[143] = advanced_get_redirect_clipboard as *const () as usize;
        slots[144] = advanced_put_audio_redirection as *const () as usize;
        slots[145] = advanced_get_audio_redirection as *const () as usize;
        slots[146] = advanced_put_connection_bar_show_pin_button as *const () as usize;
        slots[147] = advanced_get_connection_bar_show_pin_button as *const () as usize;
        slots[148] = advanced_put_public_mode as *const () as usize;
        slots[149] = advanced_get_public_mode as *const () as usize;
        slots[150] = advanced_put_redirect_devices as *const () as usize;
        slots[151] = advanced_get_redirect_devices as *const () as usize;
        slots[160] = advanced_get_pcb as *const () as usize;
        slots[161] = advanced_put_pcb as *const () as usize;
        slots[162] = advanced_put_hotkey_focus_release_left as *const () as usize;
        slots[163] = advanced_get_hotkey_focus_release_left as *const () as usize;
        slots[164] = advanced_put_hotkey_focus_release_right as *const () as usize;
        slots[165] = advanced_get_hotkey_focus_release_right as *const () as usize;
        slots[166] = advanced_put_credssp as *const () as usize;
        slots[167] = advanced_get_credssp as *const () as usize;
        slots[168] = advanced_get_authentication_type as *const () as usize;
        slots[169] = advanced_put_connect_to_administer_server as *const () as usize;
        slots[170] = advanced_get_connect_to_administer_server as *const () as usize;
        slots[171] = advanced_put_audio_capture_redirection_mode as *const () as usize;
        slots[172] = advanced_get_audio_capture_redirection_mode as *const () as usize;
        slots[173] = advanced_put_video_playback_mode as *const () as usize;
        slots[174] = advanced_get_video_playback_mode as *const () as usize;
        slots[175] = advanced_put_enable_super_pan as *const () as usize;
        slots[176] = advanced_get_enable_super_pan as *const () as usize;
        slots[179] = advanced_put_negotiate_security_layer as *const () as usize;
        slots[180] = advanced_get_negotiate_security_layer as *const () as usize;
        slots[181] = advanced_put_audio_quality_mode as *const () as usize;
        slots[182] = advanced_get_audio_quality_mode as *const () as usize;
        slots[183] = advanced_put_redirect_directx as *const () as usize;
        slots[184] = advanced_get_redirect_directx as *const () as usize;
        slots[185] = advanced_put_network_connection_type as *const () as usize;
        slots[186] = advanced_get_network_connection_type as *const () as usize;
        slots[187] = advanced_put_bandwidth_detection as *const () as usize;
        slots[188] = advanced_get_bandwidth_detection as *const () as usize;
        slots[189] = advanced_put_client_protocol_spec as *const () as usize;
        slots[190] = advanced_get_client_protocol_spec as *const () as usize;
        CompatibilitySettingsVtable {
            dispatch: dispatch_vtable::<191>(),
            slots,
        }
    })
}

fn secured_vtable() -> &'static CompatibilitySettingsVtable<SECURED_SETTINGS_SLOTS> {
    static VTABLE: std::sync::OnceLock<CompatibilitySettingsVtable<SECURED_SETTINGS_SLOTS>> =
        std::sync::OnceLock::new();
    VTABLE.get_or_init(|| {
        let mut slots = [secured_put_pcb as *const () as usize; SECURED_SETTINGS_SLOTS];
        slots[0] = secured_put_start_program as *const () as usize;
        slots[1] = secured_get_start_program as *const () as usize;
        slots[2] = secured_put_work_dir as *const () as usize;
        slots[3] = secured_get_work_dir as *const () as usize;
        slots[4] = secured_put_fullscreen as *const () as usize;
        slots[5] = secured_get_fullscreen as *const () as usize;
        slots[6] = secured_put_keyboard_hook as *const () as usize;
        slots[7] = secured_get_keyboard_hook as *const () as usize;
        slots[8] = secured_put_audio_redirection as *const () as usize;
        slots[9] = secured_get_audio_redirection as *const () as usize;
        slots[10] = secured_get_pcb as *const () as usize;
        slots[11] = secured_put_pcb as *const () as usize;
        CompatibilitySettingsVtable {
            dispatch: dispatch_vtable::<SECURED_SETTINGS_SLOTS>(),
            slots,
        }
    })
}

fn transport_vtable() -> &'static CompatibilitySettingsVtable<TRANSPORT_SETTINGS_SLOTS> {
    static VTABLE: std::sync::OnceLock<CompatibilitySettingsVtable<TRANSPORT_SETTINGS_SLOTS>> =
        std::sync::OnceLock::new();
    VTABLE.get_or_init(|| {
        let mut slots = [transport_put_u32_not_implemented as *const () as usize; TRANSPORT_SETTINGS_SLOTS];
        slots[0] = transport_put_gateway_hostname as *const () as usize;
        slots[1] = transport_get_gateway_hostname as *const () as usize;
        slots[2] = transport_put_gateway_usage_method as *const () as usize;
        slots[3] = transport_get_gateway_usage_method as *const () as usize;
        slots[4] = transport_put_gateway_profile_usage_method as *const () as usize;
        slots[5] = transport_get_gateway_profile_usage_method as *const () as usize;
        slots[6] = transport_put_gateway_creds_source as *const () as usize;
        slots[7] = transport_get_gateway_creds_source as *const () as usize;
        slots[8] = transport_put_gateway_user_selected_creds_source as *const () as usize;
        slots[9] = transport_get_gateway_user_selected_creds_source as *const () as usize;
        slots[10] = transport_get_gateway_is_supported as *const () as usize;
        slots[11] = transport_get_gateway_default_usage_method as *const () as usize;
        slots[12] = transport_put_u32_not_implemented as *const () as usize;
        slots[13] = transport_get_u32_not_implemented as *const () as usize;
        slots[14] = transport_put_u32_not_implemented as *const () as usize;
        slots[15] = transport_get_u32_not_implemented as *const () as usize;
        slots[16] = transport_put_bstr_not_implemented as *const () as usize;
        slots[17] = transport_get_bstr_not_implemented as *const () as usize;
        slots[18] = transport_put_bstr_not_implemented as *const () as usize;
        slots[19] = transport_get_bstr_not_implemented as *const () as usize;
        slots[20] = transport_put_bstr_not_implemented as *const () as usize;
        slots[21] = transport_get_bstr_not_implemented as *const () as usize;
        slots[22] = transport_put_u32_not_implemented as *const () as usize;
        slots[23] = transport_get_u32_not_implemented as *const () as usize;
        slots[24] = transport_put_gateway_username as *const () as usize;
        slots[25] = transport_get_gateway_username as *const () as usize;
        slots[26] = transport_put_gateway_domain as *const () as usize;
        slots[27] = transport_get_gateway_domain as *const () as usize;
        slots[28] = transport_put_gateway_password as *const () as usize;
        slots[29] = transport_put_u32_not_implemented as *const () as usize;
        slots[30] = transport_get_u32_not_implemented as *const () as usize;
        slots[31] = transport_put_bstr_not_implemented as *const () as usize;
        slots[32] = transport_get_bstr_not_implemented as *const () as usize;
        slots[33] = transport_put_bstr_not_implemented as *const () as usize;
        slots[34] = transport_get_bstr_not_implemented as *const () as usize;
        slots[35] = transport_put_u32_not_implemented as *const () as usize;
        slots[36] = transport_get_u32_not_implemented as *const () as usize;
        slots[37] = transport_put_bstr_not_implemented as *const () as usize;
        slots[38] = transport_get_bstr_not_implemented as *const () as usize;
        slots[39] = transport_put_u32_not_implemented as *const () as usize;
        CompatibilitySettingsVtable {
            dispatch: dispatch_vtable::<TRANSPORT_SETTINGS_SLOTS>(),
            slots,
        }
    })
}

unsafe extern "system" fn remote_program_put_mode(this: *mut c_void, value: i16) -> HRESULT {
    trace_host_call("ITSRemoteProgram::put_RemoteProgramMode");
    let object = unsafe { &*(this.cast::<CompatibilitySettingsObject<7>>()) };
    object.settings.borrow_mut().remote_program_mode = value != VARIANT_FALSE.0;
    S_OK
}

unsafe extern "system" fn remote_program_get_mode(this: *mut c_void, value: *mut i16) -> HRESULT {
    trace_host_call("ITSRemoteProgram::get_RemoteProgramMode");
    let object = unsafe { &*(this.cast::<CompatibilitySettingsObject<7>>()) };
    match write_out(
        value,
        if object.settings.borrow().remote_program_mode {
            VARIANT_TRUE.0
        } else {
            VARIANT_FALSE.0
        },
    ) {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

unsafe extern "system" fn remote_program_start_program(
    _this: *mut c_void,
    _executable: Bstr,
    _file: Bstr,
    _working_directory: Bstr,
    _expand_working_directory: i16,
    _arguments: Bstr,
    _expand_arguments: i16,
) -> HRESULT {
    // TODO(activex): implement RemoteApp launch/configuration APIs.
    trace_host_call("ITSRemoteProgram::ServerStartProgram");
    E_NOTIMPL
}

unsafe extern "system" fn remote_program_put_application_name(this: *mut c_void, value: Bstr) -> HRESULT {
    trace_host_call("ITSRemoteProgram2::put_RemoteApplicationName");
    let value = match string_from_bstr(value) {
        Ok(value) => value,
        Err(error) => return error.code(),
    };
    let object = unsafe { &*(this.cast::<CompatibilitySettingsObject<7>>()) };
    object.settings.borrow_mut().remote_application_name = value;
    S_OK
}

unsafe extern "system" fn remote_program_put_application_program(this: *mut c_void, value: Bstr) -> HRESULT {
    trace_host_call("ITSRemoteProgram2::put_RemoteApplicationProgram");
    let value = match string_from_bstr(value) {
        Ok(value) => value,
        Err(error) => return error.code(),
    };
    let object = unsafe { &*(this.cast::<CompatibilitySettingsObject<7>>()) };
    object.settings.borrow_mut().remote_application_program = value;
    S_OK
}

unsafe extern "system" fn remote_program_put_application_args(this: *mut c_void, value: Bstr) -> HRESULT {
    trace_host_call("ITSRemoteProgram2::put_RemoteApplicationArgs");
    let value = match string_from_bstr(value) {
        Ok(value) => value,
        Err(error) => return error.code(),
    };
    let object = unsafe { &*(this.cast::<CompatibilitySettingsObject<7>>()) };
    object.settings.borrow_mut().remote_application_args = value;
    S_OK
}

unsafe extern "system" fn remote_program_start_app(
    _this: *mut c_void,
    _app_user_model_id: Bstr,
    _arguments: Bstr,
    _expand_arguments: i16,
) -> HRESULT {
    // TODO(activex): implement RemoteApp launch/configuration APIs.
    trace_host_call("ITSRemoteProgram3::ServerStartApp");
    E_NOTIMPL
}

fn remote_program_vtable() -> &'static CompatibilitySettingsVtable<7> {
    static VTABLE: std::sync::OnceLock<CompatibilitySettingsVtable<7>> = std::sync::OnceLock::new();
    VTABLE.get_or_init(|| CompatibilitySettingsVtable {
        dispatch: dispatch_vtable::<7>(),
        slots: [
            remote_program_put_mode as *const () as usize,
            remote_program_get_mode as *const () as usize,
            remote_program_start_program as *const () as usize,
            remote_program_put_application_name as *const () as usize,
            remote_program_put_application_program as *const () as usize,
            remote_program_put_application_args as *const () as usize,
            remote_program_start_app as *const () as usize,
        ],
    })
}

unsafe fn settings_object<const SLOTS: usize>(
    vtable: &'static CompatibilitySettingsVtable<SLOTS>,
    settings: Rc<RefCell<CompatibilitySettings>>,
    output: *mut *mut c_void,
) -> Result<()> {
    unsafe { settings_object_with_bridge(vtable, settings, None, output) }
}

unsafe fn settings_object_with_bridge<const SLOTS: usize>(
    vtable: &'static CompatibilitySettingsVtable<SLOTS>,
    settings: Rc<RefCell<CompatibilitySettings>>,
    native_mstsc_credential_bridge: Option<NativeMstscCredentialBridge>,
    output: *mut *mut c_void,
) -> Result<()> {
    if output.is_null() {
        return Err(Error::from_hresult(E_POINTER));
    }
    let object = Box::new(CompatibilitySettingsObject {
        vtable,
        references: AtomicU32::new(1),
        settings,
        native_mstsc_credential_bridge,
        server_object: false,
    });
    let mut object = object;
    com::add_object();
    object.server_object = true;
    unsafe { *output = Box::into_raw(object).cast() };
    Ok(())
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server: String::new(),
            domain: String::new(),
            username: String::new(),
            password: None,
            disconnected_text: "Disconnected".to_owned(),
            connecting_text: "Connecting".to_owned(),
            connected_status_text: String::new(),
            fullscreen: false,
            fullscreen_title: String::new(),
            desktop_width: 1024,
            desktop_height: 768,
            // Match IronRDP's proven default color depth. Hosts can explicitly select the legacy
            // 16-bpp Interleaved-RLE path when required.
            color_depth: 32,
            start_connected: false,
        }
    }
}

#[derive(Debug)]
enum WorkerEvent {
    CertificateWarning {
        generation: u64,
        endpoint: String,
        fingerprint: [u8; 32],
        validation_reason: String,
        public_mode: bool,
        response: std_mpsc::SyncSender<CertificateDecision>,
    },
    Connected {
        generation: u64,
    },
    MonitorLayout {
        generation: u64,
        monitors: Vec<Monitor>,
    },
    LoginComplete {
        generation: u64,
    },
    Image {
        generation: u64,
        buffer: Vec<u32>,
        width: u16,
        height: u16,
    },
    DisplayResizeFallback {
        generation: u64,
    },
    RailWindowingOrders {
        generation: u64,
        data: Vec<u8>,
    },
    AutoReconnecting {
        generation: u64,
        disconnect_reason: u32,
        attempt: u32,
        maximum_attempts: u32,
        response: oneshot::Sender<AutoReconnectDecision>,
    },
    AutoReconnected {
        generation: u64,
    },
    FatalError {
        generation: u64,
        disconnect: DisconnectInfo,
    },
    Disconnected {
        generation: u64,
        disconnect: DisconnectInfo,
    },
    StaticChannelData {
        generation: u64,
        channel_name: String,
        data: Vec<u8>,
    },
    Stopped {
        generation: u64,
    },
}

impl WorkerEvent {
    fn generation(&self) -> u64 {
        match self {
            Self::CertificateWarning { generation, .. }
            | Self::Connected { generation }
            | Self::MonitorLayout { generation, .. }
            | Self::LoginComplete { generation }
            | Self::Image { generation, .. }
            | Self::DisplayResizeFallback { generation }
            | Self::RailWindowingOrders { generation, .. }
            | Self::AutoReconnecting { generation, .. }
            | Self::AutoReconnected { generation }
            | Self::FatalError { generation, .. }
            | Self::Disconnected { generation, .. }
            | Self::StaticChannelData { generation, .. }
            | Self::Stopped { generation } => *generation,
        }
    }

    fn reject_certificate_warning(self) {
        match self {
            Self::CertificateWarning { response, .. } => {
                let _ = response.send(CertificateDecision::Reject);
            }
            Self::AutoReconnecting { response, .. } => {
                let _ = response.send(AutoReconnectDecision::Stop);
            }
            _ => {}
        }
    }
}

/// Bounded worker-to-UI event queue.
///
/// RAIL lifecycle orders wait for UI capacity so an authoritative server
/// transition cannot be discarded, while bitmap and static-channel payloads
/// retain the existing lossy behavior.
#[derive(Debug)]
struct WorkerEventQueue {
    events: Mutex<Vec<WorkerEvent>>,
    space_available: Condvar,
    closed: AtomicBool,
}

impl WorkerEventQueue {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            space_available: Condvar::new(),
            closed: AtomicBool::new(false),
        }
    }

    fn take(&self) -> Vec<WorkerEvent> {
        let events = {
            let mut queue = match self.events.lock() {
                Ok(queue) => queue,
                Err(poisoned) => poisoned.into_inner(),
            };
            core::mem::take(&mut *queue)
        };
        self.space_available.notify_all();
        events
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let events = {
            let mut queue = match self.events.lock() {
                Ok(queue) => queue,
                Err(poisoned) => poisoned.into_inner(),
            };
            core::mem::take(&mut *queue)
        };
        self.space_available.notify_all();
        for event in events {
            event.reject_certificate_warning();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CertificateDecision {
    Reject,
    Accept { remember: bool },
}

fn certificate_fingerprint(certificate_der: &[u8]) -> [u8; 32] {
    Sha256::digest(certificate_der).into()
}

fn certificate_fingerprint_text(fingerprint: &[u8; 32]) -> String {
    fingerprint
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn certificate_exception_key(endpoint: &str) -> String {
    let endpoint = endpoint.trim().to_ascii_lowercase();
    let identifier = Sha256::digest(endpoint.as_bytes());
    format!(
        "{CERTIFICATE_EXCEPTION_REGISTRY_ROOT}\\{}",
        identifier.iter().map(|byte| format!("{byte:02X}")).collect::<String>()
    )
}

fn wide_registry_value(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(core::iter::once(0)).collect()
}

fn certificate_exception_is_trusted(endpoint: &str, fingerprint: &[u8; 32]) -> bool {
    let key_path = wide_registry_value(&certificate_exception_key(endpoint));
    let value_name = wide_registry_value("Sha256");
    let mut key = HKEY::default();
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key_path.as_ptr()),
            Some(0),
            KEY_READ,
            &mut key,
        )
    };
    if !opened.is_ok() {
        return false;
    }

    let mut stored = [0u8; 32];
    let mut size = stored.len() as u32;
    let read = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(value_name.as_ptr()),
            None,
            None,
            Some(stored.as_mut_ptr()),
            Some(&mut size),
        )
    };
    let _ = unsafe { RegCloseKey(key) };
    read.is_ok() && size == stored.len() as u32 && stored == *fingerprint
}

fn persist_certificate_exception(endpoint: &str, fingerprint: &[u8; 32]) -> Result<()> {
    let key_path = wide_registry_value(&certificate_exception_key(endpoint));
    let value_name = wide_registry_value("Sha256");
    let mut key = HKEY::default();
    let created = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key_path.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE,
            None,
            &mut key,
            None,
        )
    };
    if !created.is_ok() {
        return Err(Error::from_hresult(HRESULT::from_win32(created.0)));
    }
    let written = unsafe {
        RegSetValueExW(
            key,
            PCWSTR(value_name.as_ptr()),
            None,
            REG_BINARY,
            Some(fingerprint.as_slice()),
        )
    };
    let _ = unsafe { RegCloseKey(key) };
    if written.is_ok() {
        Ok(())
    } else {
        Err(Error::from_hresult(HRESULT::from_win32(written.0)))
    }
}

type TaskDialogIndirectProcedure =
    unsafe extern "system" fn(*const TASKDIALOGCONFIG, *mut i32, *mut i32, *mut WinBool) -> HRESULT;

fn task_dialog_indirect(dialog: &TASKDIALOGCONFIG, button: &mut i32, remember: &mut WinBool) -> Option<Result<()>> {
    let module = unsafe { LoadLibraryW(w!("comctl32.dll")) }.ok()?;
    let procedure = unsafe { GetProcAddress(module, s!("TaskDialogIndirect")) };
    let result = if let Some(procedure) = procedure {
        // TaskDialogIndirect is optional because legacy/unmanifested COM hosts can bind comctl32
        // without this v6 entry point. Resolving it lazily keeps the ActiveX DLL loadable there.
        let procedure: TaskDialogIndirectProcedure = unsafe { core::mem::transmute(procedure) };
        Some(unsafe { procedure(dialog, button, ptr::null_mut(), remember).ok() })
    } else {
        None
    };
    let _ = unsafe { FreeLibrary(module) };
    result
}

fn worker_completion_event(
    generation: u64,
    connection_failed: bool,
    terminal_received: bool,
    client_task_failed: bool,
) -> Option<WorkerEvent> {
    if connection_failed || terminal_received {
        None
    } else if client_task_failed {
        Some(WorkerEvent::FatalError {
            generation,
            disconnect: DisconnectInfo::internal_error(),
        })
    } else {
        Some(WorkerEvent::Disconnected {
            generation,
            disconnect: DisconnectInfo::api_initiated(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClientTaskOutcome {
    Completed,
    Cancelled,
    Panicked,
    Failed,
}

impl ClientTaskOutcome {
    const fn failed(self) -> bool {
        !matches!(self, Self::Completed)
    }

    const fn trace_marker(self) -> Option<&'static str> {
        match self {
            Self::Completed => None,
            Self::Cancelled => Some("RdpWorker::TaskFailure:Cancelled"),
            Self::Panicked => Some("RdpWorker::TaskFailure:Panicked"),
            Self::Failed => Some("RdpWorker::TaskFailure:Unknown"),
        }
    }
}

#[derive(Clone, Debug)]
struct ActiveXStaticChannelSpec {
    display_name: String,
    channel_name: ChannelName,
    options: ChannelOptions,
}

#[derive(Debug)]
struct ActiveXStaticChannel {
    spec: ActiveXStaticChannelSpec,
    events: Arc<WorkerEventQueue>,
    event_posted: Arc<AtomicBool>,
    dispatcher: isize,
    generation: u64,
}

impl_as_any!(ActiveXStaticChannel);

impl SvcProcessor for ActiveXStaticChannel {
    fn channel_name(&self) -> ChannelName {
        self.spec.channel_name.clone()
    }

    fn channel_options(&self) -> ChannelOptions {
        self.spec.options
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        if !queue_worker_event(
            &self.events,
            &self.event_posted,
            HWND(self.dispatcher as *mut c_void),
            WorkerEvent::StaticChannelData {
                generation: self.generation,
                channel_name: self.spec.display_name.clone(),
                data: payload.to_vec(),
            },
        ) {
            return Err(ironrdp_pdu::pdu_other_err!(
                "ActiveX static-channel event queue is full"
            ));
        }

        Ok(Vec::new())
    }
}

impl SvcClientProcessor for ActiveXStaticChannel {}

#[derive(Debug)]
struct Frame {
    sequence: u64,
    width: u16,
    height: u16,
}

struct PresentationSurface {
    device_context: HDC,
    bitmap: HBITMAP,
    previous_bitmap: HGDIOBJ,
    pixels: *mut u32,
    width: u16,
    height: u16,
    sequence: u64,
}

struct PresentationBackbuffer {
    device_context: HDC,
    bitmap: HBITMAP,
    previous_bitmap: HGDIOBJ,
    pixels: *mut u32,
    width: i32,
    height: i32,
}

impl PresentationBackbuffer {
    fn new(width: i32, height: i32) -> Result<Self> {
        let bitmap_info = top_down_rgb32_bitmap_info(width, height);
        let mut pixels = ptr::null_mut();
        let bitmap = unsafe { CreateDIBSection(None, &bitmap_info, DIB_RGB_COLORS, &mut pixels, None, 0)? };
        if pixels.is_null() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
            return Err(Error::from_hresult(E_OUTOFMEMORY));
        }

        let device_context = unsafe { CreateCompatibleDC(None) };
        if device_context.0.is_null() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
            return Err(Error::from_hresult(E_OUTOFMEMORY));
        }

        let previous_bitmap = unsafe { SelectObject(device_context, HGDIOBJ(bitmap.0)) };
        if previous_bitmap.0 as isize == -1 {
            unsafe {
                let _ = DeleteDC(device_context);
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
            return Err(Error::from_hresult(E_FAIL));
        }

        Ok(Self {
            device_context,
            bitmap,
            previous_bitmap,
            pixels: pixels.cast(),
            width,
            height,
        })
    }

    fn matches_extent(&self, width: i32, height: i32) -> bool {
        self.width == width && self.height == height
    }
}

impl Drop for PresentationBackbuffer {
    fn drop(&mut self) {
        let restored = unsafe { SelectObject(self.device_context, self.previous_bitmap) };
        if restored.0 as isize == -1 {
            tracing::debug!("Unable to restore the ActiveX presentation backbuffer selection");
        }
        if !unsafe { DeleteDC(self.device_context) }.as_bool() {
            tracing::debug!("Unable to release the ActiveX presentation backbuffer device context");
        }
        if !unsafe { DeleteObject(HGDIOBJ(self.bitmap.0)) }.as_bool() {
            tracing::debug!("Unable to release the ActiveX presentation backbuffer bitmap");
        }
    }
}

fn top_down_rgb32_bitmap_info(width: i32, height: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

impl PresentationSurface {
    fn new(frame: &Frame, buffer: &[u32]) -> Result<Self> {
        let bitmap_info = top_down_rgb32_bitmap_info(i32::from(frame.width), i32::from(frame.height));
        let mut pixels = ptr::null_mut();
        let bitmap = unsafe { CreateDIBSection(None, &bitmap_info, DIB_RGB_COLORS, &mut pixels, None, 0)? };
        if pixels.is_null() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
            return Err(Error::from_hresult(E_OUTOFMEMORY));
        }

        let device_context = unsafe { CreateCompatibleDC(None) };
        if device_context.0.is_null() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
            return Err(Error::from_hresult(E_OUTOFMEMORY));
        }

        let previous_bitmap = unsafe { SelectObject(device_context, HGDIOBJ(bitmap.0)) };
        if previous_bitmap.0 as isize == -1 {
            unsafe {
                let _ = DeleteDC(device_context);
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
            return Err(Error::from_hresult(E_FAIL));
        }

        let mut surface = Self {
            device_context,
            bitmap,
            previous_bitmap,
            pixels: pixels.cast(),
            width: frame.width,
            height: frame.height,
            sequence: frame.sequence,
        };
        surface.copy_from(frame, buffer);
        Ok(surface)
    }

    fn matches_extent(&self, frame: &Frame) -> bool {
        self.width == frame.width && self.height == frame.height
    }

    fn matches_frame(&self, frame: &Frame) -> bool {
        self.matches_extent(frame) && self.sequence == frame.sequence
    }

    fn copy_from(&mut self, frame: &Frame, buffer: &[u32]) {
        debug_assert!(self.matches_extent(frame));
        debug_assert_eq!(buffer.len(), usize::from(frame.width) * usize::from(frame.height));
        unsafe {
            ptr::copy_nonoverlapping(buffer.as_ptr(), self.pixels, buffer.len());
        }
        self.sequence = frame.sequence;
    }
}

impl Drop for PresentationSurface {
    fn drop(&mut self) {
        let restored = unsafe { SelectObject(self.device_context, self.previous_bitmap) };
        if restored.0 as isize == -1 {
            tracing::debug!("Unable to restore the ActiveX presentation bitmap selection");
        }
        if !unsafe { DeleteDC(self.device_context) }.as_bool() {
            tracing::debug!("Unable to release the ActiveX presentation device context");
        }
        if !unsafe { DeleteObject(HGDIOBJ(self.bitmap.0)) }.as_bool() {
            tracing::debug!("Unable to release the ActiveX presentation bitmap");
        }
    }
}

const MAX_PROJECTED_RAIL_WINDOWS: usize = 256;
const RAIL_WINDOW_CLASS: PCWSTR = w!("IronRDP.ActiveX.RailWindow");

#[derive(Default)]
struct RailWindowClassState {
    registered: bool,
    windows: usize,
}

static RAIL_WINDOW_CLASS_STATE: Mutex<RailWindowClassState> = Mutex::new(RailWindowClassState {
    registered: false,
    windows: 0,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProjectedRailGeometry {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl ProjectedRailGeometry {
    const INITIAL: Self = Self {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProjectedRailContent {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl ProjectedRailContent {
    const fn from_outer(outer: ProjectedRailGeometry) -> Self {
        Self {
            x: outer.x,
            y: outer.y,
            width: outer.width,
            height: outer.height,
        }
    }
}

fn rail_dimension(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX).max(1)
}

fn projected_rail_geometry(
    current: ProjectedRailGeometry,
    window_offset: Option<(i32, i32)>,
    window_size: Option<(u32, u32)>,
) -> ProjectedRailGeometry {
    let (x, y) = window_offset.unwrap_or((current.x, current.y));
    let (width, height) = window_size
        .map(|(width, height)| (rail_dimension(width), rail_dimension(height)))
        .unwrap_or((current.width, current.height));
    ProjectedRailGeometry { x, y, width, height }
}

fn projected_rail_content(
    current: ProjectedRailContent,
    outer: ProjectedRailGeometry,
    client_area_offset: Option<(i32, i32)>,
    client_area_size: Option<(u32, u32)>,
    client_delta: Option<(i32, i32)>,
) -> ProjectedRailContent {
    let (x, y) = client_area_offset
        .or_else(|| {
            client_delta.map(|(delta_x, delta_y)| (outer.x.saturating_add(delta_x), outer.y.saturating_add(delta_y)))
        })
        .unwrap_or((current.x, current.y));
    let (width, height) = if let Some((width, height)) = client_area_size {
        (rail_dimension(width), rail_dimension(height))
    } else if let Some((delta_x, delta_y)) = client_delta {
        (
            outer.width.saturating_sub(delta_x.max(0)),
            outer.height.saturating_sub(delta_y.max(0)),
        )
    } else {
        (current.width, current.height)
    };
    let content = ProjectedRailContent { x, y, width, height };
    if content.width > 0 && content.height > 0 {
        content
    } else {
        ProjectedRailContent::from_outer(outer)
    }
}

struct ProjectedRailWindowContext {
    window_id: u32,
    input_sender: RdpInputSender,
    input_database: Rc<RefCell<InputDatabase>>,
    compatibility: Rc<RefCell<CompatibilitySettings>>,
    frame: Rc<RefCell<Option<Frame>>>,
    presentation_surface: Rc<RefCell<Option<PresentationSurface>>>,
    content: Rc<Cell<ProjectedRailContent>>,
    close_pending: Cell<bool>,
    close_queued: Cell<bool>,
    release_pending: Cell<bool>,
}

struct ProjectedRailWindow {
    hwnd: HWND,
    owner_window_id: Option<u32>,
    geometry: Rc<Cell<ProjectedRailGeometry>>,
    content: Rc<Cell<ProjectedRailContent>>,
    server_style: WINDOW_STYLE,
    server_extended_style: WINDOW_EX_STYLE,
    _context: Box<ProjectedRailWindowContext>,
}

struct ProjectedRailWindowOrder {
    is_new: bool,
    window_id: u32,
    owner_window_id: Option<Option<u32>>,
    style: Option<(u32, u32)>,
    show_state: Option<u8>,
    title: Option<String>,
    client_area_offset: Option<(i32, i32)>,
    client_area_size: Option<(u32, u32)>,
    window_offset: Option<(i32, i32)>,
    client_delta: Option<(i32, i32)>,
    window_size: Option<(u32, u32)>,
}

struct WindowOrderReader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> WindowOrderReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    fn read_u8(&mut self) -> Option<u8> {
        let value = *self.data.get(self.offset)?;
        self.offset = self.offset.checked_add(1)?;
        Some(value)
    }

    fn read_u16(&mut self) -> Option<u16> {
        let bytes = self.take(2)?;
        Some(u16::from_le_bytes(bytes.try_into().ok()?))
    }

    fn read_u32(&mut self) -> Option<u32> {
        let bytes = self.take(4)?;
        Some(u32::from_le_bytes(bytes.try_into().ok()?))
    }

    fn read_i32(&mut self) -> Option<i32> {
        let bytes = self.take(4)?;
        Some(i32::from_le_bytes(bytes.try_into().ok()?))
    }

    fn take(&mut self, length: usize) -> Option<&'a [u8]> {
        let end = self.offset.checked_add(length)?;
        let bytes = self.data.get(self.offset..end)?;
        self.offset = end;
        Some(bytes)
    }

    fn skip(&mut self, length: usize) -> Option<()> {
        self.take(length).map(|_| ())
    }

    fn read_utf16(&mut self) -> Option<String> {
        let length = usize::from(self.read_u16()?);
        let bytes = self.take(length)?;
        let code_units = bytes
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .collect::<Vec<_>>();
        String::from_utf16(&code_units).ok()
    }
}

fn parse_projected_rail_window_order(encoded: &[u8], flags: u32) -> Option<ProjectedRailWindowOrder> {
    const WINDOW_TYPE: u32 = 0x0100_0000;
    const STATE_NEW: u32 = 0x1000_0000;
    const DELETED: u32 = 0x2000_0000;
    const ICON: u32 = 0x4000_0000;
    const CACHED_ICON: u32 = 0x8000_0000;

    if flags & WINDOW_TYPE == 0 || flags & (DELETED | ICON | CACHED_ICON) != 0 {
        return None;
    }

    let mut reader = WindowOrderReader::new(encoded.get(7..)?);
    let window_id = reader.read_u32()?;
    let owner_window_id =
        (flags & 0x0000_0002 != 0).then(|| reader.read_u32().map(|owner| (owner != 0).then_some(owner)))?;
    let style = (flags & 0x0000_0008 != 0).then(|| Some((reader.read_u32()?, reader.read_u32()?)))?;
    let show_state = (flags & 0x0000_0010 != 0).then(|| reader.read_u8())?;
    let title = (flags & 0x0000_0004 != 0).then(|| reader.read_utf16())?;
    let client_area_offset = (flags & 0x0000_4000 != 0).then(|| Some((reader.read_i32()?, reader.read_i32()?)))?;
    let client_area_size = (flags & 0x0001_0000 != 0).then(|| Some((reader.read_u32()?, reader.read_u32()?)))?;
    if flags & 0x0000_0080 != 0 {
        reader.skip(8)?;
    }
    if flags & 0x0800_0000 != 0 {
        reader.skip(8)?;
    }
    if flags & 0x0002_0000 != 0 {
        reader.skip(1)?;
    }
    if flags & 0x0004_0000 != 0 {
        reader.skip(4)?;
    }
    let window_offset = (flags & 0x0000_0800 != 0).then(|| Some((reader.read_i32()?, reader.read_i32()?)))?;
    let client_delta = (flags & 0x0000_8000 != 0).then(|| Some((reader.read_i32()?, reader.read_i32()?)))?;
    let window_size = (flags & 0x0000_0400 != 0).then(|| Some((reader.read_u32()?, reader.read_u32()?)))?;

    Some(ProjectedRailWindowOrder {
        is_new: flags & STATE_NEW != 0,
        window_id,
        owner_window_id,
        style,
        show_state,
        title,
        client_area_offset,
        client_area_size,
        window_offset,
        client_delta,
        window_size,
    })
}

fn resets_projected_rail_windows(fields_present: u32) -> bool {
    const DESKTOP_TYPE: u32 = 0x0400_0000;
    const DESKTOP_NON_MONITORED: u32 = 0x0000_0001;
    const DESKTOP_HOOKED: u32 = 0x0000_0002;
    const DESKTOP_ARC_BEGAN: u32 = 0x0000_0008;

    fields_present & DESKTOP_TYPE != 0
        && (fields_present & DESKTOP_NON_MONITORED != 0
            || fields_present & (DESKTOP_HOOKED | DESKTOP_ARC_BEGAN) == DESKTOP_HOOKED | DESKTOP_ARC_BEGAN)
}

fn acquire_rail_window_class() -> Result<()> {
    let mut state = match RAIL_WINDOW_CLASS_STATE.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    if !state.registered {
        let instance = unsafe { GetModuleHandleW(None) }?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(projected_rail_window_proc),
            hInstance: windows::Win32::Foundation::HINSTANCE(instance.0),
            lpszClassName: RAIL_WINDOW_CLASS,
            ..Default::default()
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            let error = unsafe { windows::Win32::Foundation::GetLastError() };
            return Err(Error::from_hresult(HRESULT::from_win32(error.0)));
        }
        state.registered = true;
    }
    state.windows = state
        .windows
        .checked_add(1)
        .ok_or_else(|| Error::from_hresult(E_OUTOFMEMORY))?;
    Ok(())
}

fn release_rail_window_class() {
    let mut state = match RAIL_WINDOW_CLASS_STATE.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    if state.windows == 0 {
        return;
    }
    state.windows -= 1;
    if state.windows != 0 || !state.registered {
        return;
    }
    let result = unsafe { GetModuleHandleW(None) }.and_then(|instance| unsafe {
        UnregisterClassW(
            RAIL_WINDOW_CLASS,
            Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
        )
    });
    match result {
        Ok(()) => state.registered = false,
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CLASS_DOES_NOT_EXIST.0) => state.registered = false,
        Err(error) => tracing::debug!(?error, "Unable to unregister the RAIL window class"),
    }
}

fn apply_projected_rail_input(context: &ProjectedRailWindowContext, operations: impl IntoIterator<Item = Operation>) {
    let permit = match context.input_sender.try_reserve() {
        Ok(permit) => permit,
        Err(error) => {
            tracing::debug!(
                ?error,
                window_id = context.window_id,
                "Unable to reserve projected RAIL window input"
            );
            return;
        }
    };
    let fast_path = context.input_database.borrow_mut().apply(operations);
    if fast_path.is_empty() {
        return;
    }
    permit.send(RdpInputEvent::FastPath(fast_path));
}

fn schedule_projected_rail_input_retry(hwnd: HWND) {
    unsafe {
        let _ = SetTimer(
            Some(hwnd),
            PROJECTED_RAIL_INPUT_RETRY_TIMER_ID,
            PROJECTED_RAIL_INPUT_RETRY_MILLISECONDS,
            None,
        );
    }
}

fn release_projected_rail_input(hwnd: HWND, context: &ProjectedRailWindowContext) {
    let permit = match context.input_sender.try_reserve() {
        Ok(permit) => permit,
        Err(error) => {
            context.release_pending.set(true);
            schedule_projected_rail_input_retry(hwnd);
            tracing::debug!(
                ?error,
                window_id = context.window_id,
                "Deferring projected RAIL window input release"
            );
            return;
        }
    };
    let fast_path = context.input_database.borrow_mut().release_all();
    context.release_pending.set(false);
    if fast_path.is_empty() {
        return;
    }
    permit.send(RdpInputEvent::FastPath(fast_path));
}

fn queue_projected_rail_close(hwnd: HWND, context: &ProjectedRailWindowContext) {
    if context.close_queued.get() || context.close_pending.get() {
        return;
    }
    let event = RailInputEvent::SystemCommand(SystemCommandPdu {
        window_id: context.window_id,
        command: SystemCommand::Close,
    });
    match context.input_sender.try_reserve() {
        Ok(permit) => {
            permit.send(RdpInputEvent::Rail(event));
            context.close_queued.set(true);
        }
        Err(error) => {
            context.close_pending.set(true);
            schedule_projected_rail_input_retry(hwnd);
            tracing::debug!(
                ?error,
                window_id = context.window_id,
                "Deferring projected RAIL window close request"
            );
        }
    }
}

fn retry_projected_rail_input(hwnd: HWND, context: &ProjectedRailWindowContext) {
    if context.release_pending.get() {
        let Ok(permit) = context.input_sender.try_reserve() else {
            return;
        };
        let fast_path = context.input_database.borrow_mut().release_all();
        context.release_pending.set(false);
        if !fast_path.is_empty() {
            permit.send(RdpInputEvent::FastPath(fast_path));
        }
    }

    if context.close_pending.get() {
        let event = RailInputEvent::SystemCommand(SystemCommandPdu {
            window_id: context.window_id,
            command: SystemCommand::Close,
        });
        if let Ok(permit) = context.input_sender.try_reserve() {
            permit.send(RdpInputEvent::Rail(event));
            context.close_pending.set(false);
            context.close_queued.set(true);
        }
    }

    if !context.close_pending.get() && !context.release_pending.get() {
        unsafe {
            let _ = KillTimer(Some(hwnd), PROJECTED_RAIL_INPUT_RETRY_TIMER_ID);
        }
    }
}

fn queue_projected_rail_lifecycle_input(context: &ProjectedRailWindowContext, event: RailInputEvent) {
    if let Err(error) = context.input_sender.try_send_rail_input(event) {
        tracing::debug!(
            ?error,
            window_id = context.window_id,
            "Unable to forward projected RAIL window lifecycle input"
        );
    }
}

fn projected_rail_mouse_position(
    context: &ProjectedRailWindowContext,
    hwnd: HWND,
    x: i32,
    y: i32,
) -> Option<MousePosition> {
    let mut client_rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client_rect) }.ok()?;
    let client_width = client_rect.right - client_rect.left;
    let client_height = client_rect.bottom - client_rect.top;
    let content_rect = context.content.get();
    let frame = context.frame.borrow();
    let frame = frame.as_ref()?;
    if content_rect.width <= 0
        || content_rect.height <= 0
        || client_width <= 0
        || client_height <= 0
        || x < 0
        || y < 0
        || x >= client_width
        || y >= client_height
    {
        return None;
    }

    let desktop_x = i64::from(content_rect.x) + i64::from(x) * i64::from(content_rect.width) / i64::from(client_width);
    let desktop_y =
        i64::from(content_rect.y) + i64::from(y) * i64::from(content_rect.height) / i64::from(client_height);
    if desktop_x < 0 || desktop_y < 0 || desktop_x >= i64::from(frame.width) || desktop_y >= i64::from(frame.height) {
        return None;
    }
    Some(MousePosition {
        x: u16::try_from(desktop_x).ok()?,
        y: u16::try_from(desktop_y).ok()?,
    })
}

fn paint_projected_rail_window(hwnd: HWND, context: &ProjectedRailWindowContext) {
    let mut paint = PAINTSTRUCT::default();
    let device_context = unsafe { BeginPaint(hwnd, &mut paint) };
    let mut client_rect = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut client_rect) }.is_ok() {
        let client_width = (client_rect.right - client_rect.left).max(0);
        let client_height = (client_rect.bottom - client_rect.top).max(0);
        let content_rect = context.content.get();
        let surface = context.presentation_surface.borrow();
        if let Some(surface) = surface.as_ref()
            && content_rect.width > 0
            && content_rect.height > 0
            && client_width > 0
            && client_height > 0
            && !unsafe {
                StretchBlt(
                    device_context,
                    0,
                    0,
                    client_width,
                    client_height,
                    Some(surface.device_context),
                    content_rect.x,
                    content_rect.y,
                    content_rect.width,
                    content_rect.height,
                    SRCCOPY,
                )
            }
            .as_bool()
        {
            tracing::debug!(window_id = context.window_id, "Unable to paint projected RAIL window");
        }
    }
    unsafe {
        let _ = EndPaint(hwnd, &paint);
    }
}

fn handle_projected_rail_window_message(
    hwnd: HWND,
    context: &ProjectedRailWindowContext,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> bool {
    if message == WM_CLOSE {
        queue_projected_rail_close(hwnd, context);
        // The server remains authoritative for a projected window's lifetime.
        return true;
    }
    if message == WM_TIMER && wparam.0 == PROJECTED_RAIL_INPUT_RETRY_TIMER_ID {
        retry_projected_rail_input(hwnd, context);
        return true;
    }
    if message == WM_SYSCOMMAND && is_unsupported_projected_rail_system_command(wparam) {
        // ActiveX does not implement the server-directed move/size lifecycle.
        return true;
    }
    if let Some(event) = rail_window_input_event(context.window_id, message, wparam) {
        queue_projected_rail_lifecycle_input(context, event);
    }

    match message {
        WM_PAINT => {
            paint_projected_rail_window(hwnd, context);
            true
        }
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            let lparam = lparam.0 as u32;
            let scancode = Scancode::from_u8(lparam & 0x0100_0000 != 0, ((lparam >> 16) & 0xff) as u8);
            let compatibility = context.compatibility.borrow();
            let input_database = context.input_database.borrow();
            if !should_forward_windows_key(&compatibility, false, &input_database, message, scancode) {
                return true;
            }
            drop(input_database);
            drop(compatibility);
            let operation = if matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN) {
                Operation::KeyPressed(scancode)
            } else {
                Operation::KeyReleased(scancode)
            };
            apply_projected_rail_input(context, [operation]);
            true
        }
        WM_MOUSEMOVE => {
            if context.compatibility.borrow().enable_mouse
                && let Some(position) = projected_rail_mouse_position(
                    context,
                    hwnd,
                    i32::from(lparam.0 as i32 as i16),
                    i32::from((lparam.0 >> 16) as i16),
                )
            {
                apply_projected_rail_input(context, [Operation::MouseMove(position)]);
            }
            true
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
            if !context.compatibility.borrow().enable_mouse {
                return true;
            }
            if let Err(error) = unsafe { SetFocus(Some(hwnd)) } {
                tracing::debug!(?error, "Unable to focus projected RAIL window");
            }
            unsafe {
                SetCapture(hwnd);
            }
            let button = match message {
                WM_LBUTTONDOWN => MouseButton::Left,
                WM_RBUTTONDOWN => MouseButton::Right,
                WM_MBUTTONDOWN => MouseButton::Middle,
                _ if (wparam.0 >> 16) & 0xffff == 1 => MouseButton::X1,
                _ => MouseButton::X2,
            };
            let x = i32::from(lparam.0 as i32 as i16);
            let y = i32::from((lparam.0 >> 16) as i16);
            if let Some(position) = projected_rail_mouse_position(context, hwnd, x, y) {
                apply_projected_rail_input(
                    context,
                    [Operation::MouseMove(position), Operation::MouseButtonPressed(button)],
                );
            }
            true
        }
        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP => {
            let button = match message {
                WM_LBUTTONUP => MouseButton::Left,
                WM_RBUTTONUP => MouseButton::Right,
                WM_MBUTTONUP => MouseButton::Middle,
                _ if (wparam.0 >> 16) & 0xffff == 1 => MouseButton::X1,
                _ => MouseButton::X2,
            };
            let x = i32::from(lparam.0 as i32 as i16);
            let y = i32::from((lparam.0 >> 16) as i16);
            if let Some(position) = projected_rail_mouse_position(context, hwnd, x, y) {
                apply_projected_rail_input(
                    context,
                    [Operation::MouseMove(position), Operation::MouseButtonReleased(button)],
                );
            }
            let has_pressed_buttons = {
                let input_database = context.input_database.borrow();
                [
                    MouseButton::Left,
                    MouseButton::Middle,
                    MouseButton::Right,
                    MouseButton::X1,
                    MouseButton::X2,
                ]
                .into_iter()
                .any(|button| input_database.is_mouse_button_pressed(button))
            };
            if !has_pressed_buttons && let Err(error) = unsafe { ReleaseCapture() } {
                tracing::debug!(?error, "Unable to release projected RAIL mouse capture");
            }
            true
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            if !context.compatibility.borrow().enable_mouse {
                return true;
            }
            let mut point = POINT {
                x: i32::from(lparam.0 as i32 as i16),
                y: i32::from((lparam.0 >> 16) as i16),
            };
            if unsafe { ScreenToClient(hwnd, &mut point) }.as_bool()
                && let Some(position) = projected_rail_mouse_position(context, hwnd, point.x, point.y)
            {
                apply_projected_rail_input(context, [Operation::MouseMove(position)]);
            }
            let mut remaining = ((wparam.0 >> 16) as u16) as i16;
            let mut operations = Vec::new();
            while remaining != 0 {
                let rotation_units = remaining.clamp(-256, 255);
                operations.push(Operation::WheelRotations(WheelRotations {
                    is_vertical: message == WM_MOUSEWHEEL,
                    rotation_units,
                }));
                remaining -= rotation_units;
            }
            apply_projected_rail_input(context, operations);
            true
        }
        WM_CANCELMODE | WM_ENABLE if wparam.0 == 0 => {
            release_projected_rail_input(hwnd, context);
            false
        }
        WM_KILLFOCUS | WM_CAPTURECHANGED => {
            release_projected_rail_input(hwnd, context);
            true
        }
        _ => false,
    }
}

unsafe extern "system" fn projected_rail_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match catch_unwind(AssertUnwindSafe(|| unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let context = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const ProjectedRailWindowContext;
        if !context.is_null() && handle_projected_rail_window_message(hwnd, &*context, message, wparam, lparam) {
            LRESULT(0)
        } else {
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
    })) {
        Ok(result) => result,
        Err(_) => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

struct RailWindowManager {
    input_database: Rc<RefCell<InputDatabase>>,
    compatibility: Rc<RefCell<CompatibilitySettings>>,
    frame: Rc<RefCell<Option<Frame>>>,
    presentation_surface: Rc<RefCell<Option<PresentationSurface>>>,
    input_sender: Option<RdpInputSender>,
    windows: BTreeMap<u32, ProjectedRailWindow>,
}

impl RailWindowManager {
    fn new(
        input_database: Rc<RefCell<InputDatabase>>,
        compatibility: Rc<RefCell<CompatibilitySettings>>,
        frame: Rc<RefCell<Option<Frame>>>,
        presentation_surface: Rc<RefCell<Option<PresentationSurface>>>,
    ) -> Self {
        Self {
            input_database,
            compatibility,
            frame,
            presentation_surface,
            input_sender: None,
            windows: BTreeMap::new(),
        }
    }

    fn start(&mut self, input_sender: Option<RdpInputSender>) {
        self.clear();
        self.input_sender = input_sender;
    }

    fn is_enabled(&self) -> bool {
        self.input_sender.is_some()
    }

    fn stop(&mut self) {
        self.clear();
        self.input_sender = None;
    }

    fn consume(&mut self, update: &[u8]) {
        let mut reader = ReadCursor::new(update);
        if reader.len() < 2 {
            tracing::debug!("Ignoring truncated RAIL windowing update");
            return;
        }
        reader.advance(2);
        let Ok(update) = try_decode_slow_path_windowing_orders(&mut reader) else {
            tracing::debug!("Ignoring malformed RAIL windowing update");
            return;
        };
        for order in update.orders {
            const WINDOW_TYPE: u32 = 0x0100_0000;
            const DELETED: u32 = 0x2000_0000;

            if resets_projected_rail_windows(order.fields_present) {
                self.clear();
            } else if order.fields_present & WINDOW_TYPE != 0 && order.fields_present & DELETED != 0 {
                if let Some(window_id) = order
                    .encoded
                    .get(7..11)
                    .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
                    .map(u32::from_le_bytes)
                {
                    self.destroy_window(window_id);
                }
            } else if let Some(order) = parse_projected_rail_window_order(order.encoded, order.fields_present) {
                self.apply_window_order(order);
            }
        }
    }

    fn invalidate_presentation(&self) {
        for window in self.windows.values() {
            if unsafe { IsWindow(Some(window.hwnd)) }.as_bool() {
                unsafe {
                    let _ = InvalidateRect(Some(window.hwnd), None, false);
                }
            }
        }
    }

    fn apply_window_order(&mut self, order: ProjectedRailWindowOrder) {
        let Some(input_sender) = self.input_sender.clone() else {
            return;
        };
        let current_geometry = self
            .windows
            .get(&order.window_id)
            .map_or(ProjectedRailGeometry::INITIAL, |window| window.geometry.get());
        let geometry = projected_rail_geometry(current_geometry, order.window_offset, order.window_size);
        let current_content = self.windows.get(&order.window_id).map_or_else(
            || ProjectedRailContent::from_outer(geometry),
            |window| window.content.get(),
        );
        let content = projected_rail_content(
            current_content,
            geometry,
            order.client_area_offset,
            order.client_area_size,
            order.client_delta,
        );

        if !self.windows.contains_key(&order.window_id) {
            if !order.is_new {
                return;
            }
            if self.windows.len() >= MAX_PROJECTED_RAIL_WINDOWS {
                tracing::warn!(
                    window_id = order.window_id,
                    "Ignoring RAIL window beyond the projection capacity"
                );
                return;
            }
            let owner = order
                .owner_window_id
                .flatten()
                .and_then(|owner_window_id| self.windows.get(&owner_window_id))
                .map(|window| window.hwnd);
            let (server_style, server_extended_style) = order.style.map_or_else(
                || (WS_POPUP, WINDOW_EX_STYLE::default()),
                |style| (WINDOW_STYLE(style.0 & !WS_CHILD.0), WINDOW_EX_STYLE(style.1)),
            );
            let title = HSTRING::from(order.title.as_deref().unwrap_or_default());
            let geometry_cell = Rc::new(Cell::new(geometry));
            let content_cell = Rc::new(Cell::new(content));
            let mut window_context = Box::new(ProjectedRailWindowContext {
                window_id: order.window_id,
                input_sender,
                input_database: Rc::clone(&self.input_database),
                compatibility: Rc::clone(&self.compatibility),
                frame: Rc::clone(&self.frame),
                presentation_surface: Rc::clone(&self.presentation_surface),
                content: Rc::clone(&content_cell),
                close_pending: Cell::new(false),
                close_queued: Cell::new(false),
                release_pending: Cell::new(false),
            });
            if let Err(error) = acquire_rail_window_class() {
                tracing::warn!(
                    ?error,
                    window_id = order.window_id,
                    "Unable to register RAIL window class"
                );
                return;
            }
            let hwnd = match unsafe {
                CreateWindowExW(
                    server_extended_style,
                    RAIL_WINDOW_CLASS,
                    PCWSTR(title.as_ptr()),
                    server_style,
                    geometry.x,
                    geometry.y,
                    geometry.width,
                    geometry.height,
                    owner,
                    None,
                    None,
                    Some((&mut *window_context as *mut ProjectedRailWindowContext).cast()),
                )
            } {
                Ok(hwnd) => hwnd,
                Err(error) => {
                    release_rail_window_class();
                    tracing::warn!(?error, window_id = order.window_id, "Unable to create RAIL window");
                    return;
                }
            };
            self.windows.insert(
                order.window_id,
                ProjectedRailWindow {
                    hwnd,
                    owner_window_id: order.owner_window_id.flatten(),
                    geometry: geometry_cell,
                    content: content_cell,
                    server_style,
                    server_extended_style,
                    _context: window_context,
                },
            );
            self.attach_waiting_children(order.window_id);
        }

        let owner = order
            .owner_window_id
            .flatten()
            .and_then(|owner_window_id| self.windows.get(&owner_window_id))
            .map(|window| window.hwnd);
        let (hwnd, style_changed, server_style, server_extended_style) = {
            let Some(window) = self.windows.get_mut(&order.window_id) else {
                return;
            };
            if let Some(owner_window_id) = order.owner_window_id {
                window.owner_window_id = owner_window_id;
            }
            window.geometry.set(geometry);
            window.content.set(content);
            let style_changed = order.style.is_some();
            if let Some((style, extended_style)) = order.style {
                window.server_style = WINDOW_STYLE(style & !WS_CHILD.0);
                window.server_extended_style = WINDOW_EX_STYLE(extended_style);
            }
            (
                window.hwnd,
                style_changed,
                window.server_style,
                window.server_extended_style,
            )
        };

        if let Some(title) = order.title {
            let title = HSTRING::from(title);
            if let Err(error) = unsafe { SetWindowTextW(hwnd, PCWSTR(title.as_ptr())) } {
                tracing::debug!(?error, window_id = order.window_id, "Unable to set RAIL window title");
            }
        }
        if order.owner_window_id.is_some() {
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner.map_or(0, |owner| owner.0 as isize));
            }
        }
        if style_changed {
            unsafe {
                SetWindowLongPtrW(hwnd, GWL_STYLE, server_style.0 as isize);
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, server_extended_style.0 as isize);
            }
        }
        if let Some(show_state) = order.show_state {
            unsafe {
                let _ = ShowWindow(
                    hwnd,
                    match show_state {
                        0 => SW_HIDE,
                        2 => SW_MINIMIZE,
                        3 => SW_MAXIMIZE,
                        _ => SW_SHOWNA,
                    },
                );
            }
        }
        let mut flags = SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOZORDER;
        if style_changed {
            flags |= SWP_FRAMECHANGED;
        }
        if let Err(error) = unsafe {
            SetWindowPos(
                hwnd,
                None,
                geometry.x,
                geometry.y,
                geometry.width,
                geometry.height,
                flags,
            )
        } {
            tracing::debug!(?error, window_id = order.window_id, "Unable to position RAIL window");
        }
    }

    fn attach_waiting_children(&self, owner_window_id: u32) {
        let Some(owner) = self.windows.get(&owner_window_id).map(|window| window.hwnd) else {
            return;
        };
        for window in self
            .windows
            .values()
            .filter(|window| window.owner_window_id == Some(owner_window_id))
        {
            unsafe {
                SetWindowLongPtrW(window.hwnd, GWLP_HWNDPARENT, owner.0 as isize);
            }
        }
    }

    fn destroy_window(&mut self, window_id: u32) {
        let Some(window) = self.windows.remove(&window_id) else {
            return;
        };
        if unsafe { IsWindow(Some(window.hwnd)) }.as_bool()
            && let Err(error) = unsafe { DestroyWindow(window.hwnd) }
        {
            tracing::debug!(?error, window_id, "Unable to destroy projected RAIL window");
            // Do not leave an HWND with a pointer to context that is about to
            // be dropped when an external component prevents destruction.
            unsafe {
                SetWindowLongPtrW(window.hwnd, GWLP_USERDATA, 0);
            }
        }
        release_rail_window_class();
    }

    fn clear(&mut self) {
        for window_id in self.windows.keys().copied().collect::<Vec<_>>() {
            self.destroy_window(window_id);
        }
    }
}

impl Drop for RailWindowManager {
    fn drop(&mut self) {
        self.clear();
    }
}

struct ClipboardState {
    enabled_for_session: Cell<bool>,
    connected: Cell<bool>,
}

const MAX_OLE_CLIPBOARD_TEXT_BYTES: usize = 16 * 1024 * 1024;

impl ClipboardState {
    fn is_available(&self) -> bool {
        self.enabled_for_session.get() && self.connected.get()
    }
}

#[derive(Clone, Debug)]
struct ActiveXClipboardMessageProxy {
    input_sender: RdpInputSender,
}

impl ClipboardMessageProxy for ActiveXClipboardMessageProxy {
    fn send_clipboard_message(&self, message: ClipboardMessage) {
        if self.input_sender.try_send(RdpInputEvent::Clipboard(message)).is_err() {
            // A lost clipboard protocol message can desynchronize CLIPRDR. Cancel the session
            // rather than leave a stale delayed-rendering request attached to the Windows clipboard.
            self.input_sender.request_close();
            tracing::error!("Unable to enqueue ActiveX clipboard message; cancelling RDP session");
        }
    }
}

// COM can release child objects after their originating control. Keep the server loaded until
// every externally visible child object has completed its own final Release.
struct ServerObjectLifetime;

impl ServerObjectLifetime {
    fn new() -> Self {
        com::add_object();
        Self
    }
}

impl Drop for ServerObjectLifetime {
    fn drop(&mut self) {
        com::release_object();
    }
}

impl Frame {
    fn new(buffer: &[u32], width: u16, height: u16, sequence: u64) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let pixel_count = usize::from(width).checked_mul(usize::from(height))?;
        (buffer.len() == pixel_count).then_some(Self {
            sequence,
            width,
            height,
        })
    }
}

fn renderer_clip_region(position: RECT, clip: RECT) -> RECT {
    let left = clip.left.max(position.left);
    let top = clip.top.max(position.top);
    let right = clip.right.min(position.right);
    let bottom = clip.bottom.min(position.bottom);
    if left >= right || top >= bottom {
        return RECT::default();
    }

    RECT {
        left: left.saturating_sub(position.left),
        top: top.saturating_sub(position.top),
        right: right.saturating_sub(position.left),
        bottom: bottom.saturating_sub(position.top),
    }
}

#[derive(Clone)]
struct EventSink {
    cookie: u32,
    dispatch: IDispatch,
}

#[derive(Clone)]
struct ViewAdvise {
    aspects: DVASPECT,
    flags: u32,
    sink: IAdviseSink,
}

#[implement(IMsRdpDeviceCollection)]
struct EmptyDeviceCollection {
    _lifetime: ServerObjectLifetime,
}

impl EmptyDeviceCollection {
    fn new() -> Self {
        Self {
            _lifetime: ServerObjectLifetime::new(),
        }
    }
}

impl IMsRdpDeviceCollection_Impl for EmptyDeviceCollection_Impl {
    unsafe fn RescanDevices(&self, _dynamic_redirection: i16) -> Result<()> {
        // TODO(activex): enumerate devices after IronRDP RDPDR exposes a host-device backend.
        Ok(())
    }

    unsafe fn get_DeviceByIndex(&self, _index: u32, device: InterfaceOut) -> Result<()> {
        write_out(device, ptr::null_mut())?;
        Err(Error::from_hresult(E_INVALIDARG))
    }

    unsafe fn get_DeviceById(&self, _instance_id: Bstr, device: InterfaceOut) -> Result<()> {
        write_out(device, ptr::null_mut())?;
        Err(Error::from_hresult(E_INVALIDARG))
    }

    unsafe fn get_DeviceCount(&self, count: *mut u32) -> Result<()> {
        write_out(count, 0)
    }
}

struct DriveCatalogEntry {
    device_id: u32,
    name: String,
    root_path: PathBuf,
    redirection_state: Cell<bool>,
}

struct DriveCatalog {
    entries: Vec<Rc<DriveCatalogEntry>>,
    known_entries: BTreeMap<PathBuf, Rc<DriveCatalogEntry>>,
    next_device_id: u32,
}

impl DriveCatalog {
    fn new() -> Self {
        Self::from_roots(logical_volume_roots(), false)
    }

    fn from_roots(roots: Vec<PathBuf>, redirect_new_drives: bool) -> Self {
        let mut catalog = Self {
            entries: Vec::new(),
            known_entries: BTreeMap::new(),
            next_device_id: 1,
        };
        catalog.rescan_from_roots(roots, redirect_new_drives);
        catalog
    }

    fn rescan(&mut self, redirect_new_drives: bool) {
        self.rescan_from_roots(logical_volume_roots(), redirect_new_drives);
    }

    fn rescan_from_roots(&mut self, roots: Vec<PathBuf>, redirect_new_drives: bool) {
        self.entries = roots
            .into_iter()
            .map(|root_path| {
                Rc::clone(self.known_entries.entry(root_path.clone()).or_insert_with(|| {
                    let device_id = self.next_device_id;
                    self.next_device_id = self
                        .next_device_id
                        .checked_add(1)
                        .expect("logical-volume RDPDR device IDs must not exhaust u32");
                    Rc::new(DriveCatalogEntry {
                        device_id,
                        name: logical_volume_name(&root_path),
                        root_path,
                        redirection_state: Cell::new(redirect_new_drives),
                    })
                }))
            })
            .collect();
    }

    fn set_redirection_state(&self, value: bool) {
        for entry in &self.entries {
            entry.redirection_state.set(value);
        }
    }

    fn selected_drives(&self) -> Result<Vec<ironrdp_rdpdr_native::RedirectedDrive>> {
        self.entries
            .iter()
            .filter(|entry| entry.redirection_state.get())
            .map(|entry| {
                ironrdp_rdpdr_native::RedirectedDrive::new(
                    entry.device_id,
                    entry.name.clone(),
                    entry.root_path.clone(),
                    false,
                )
                .map_err(|error| Error::new(E_FAIL, format!("invalid redirected drive: {error}")))
            })
            .collect()
    }

    #[cfg(test)]
    fn selected_drive_names(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| entry.redirection_state.get())
            .map(|entry| entry.name.clone())
            .collect()
    }
}

fn logical_volume_roots() -> Vec<PathBuf> {
    let mask = unsafe { GetLogicalDrives() };
    if mask == 0 {
        tracing::warn!("Unable to enumerate logical drives for ActiveX RDPDR redirection");
    }

    (0..26)
        .filter(|index| mask & (1u32 << index) != 0)
        .map(|index| PathBuf::from(format!("{}:\\", char::from(b'A' + index))))
        .collect()
}

fn logical_volume_name(root_path: &Path) -> String {
    root_path.to_string_lossy().trim_end_matches(['\\', '/']).to_owned()
}

#[implement(IMsRdpDrive)]
struct Drive {
    _lifetime: ServerObjectLifetime,
    catalog: Rc<RefCell<DriveCatalog>>,
    entry: Rc<DriveCatalogEntry>,
    settings: Rc<RefCell<CompatibilitySettings>>,
}

impl Drive {
    fn new(
        catalog: Rc<RefCell<DriveCatalog>>,
        entry: Rc<DriveCatalogEntry>,
        settings: Rc<RefCell<CompatibilitySettings>>,
    ) -> Self {
        Self {
            _lifetime: ServerObjectLifetime::new(),
            catalog,
            entry,
            settings,
        }
    }
}

impl IMsRdpDrive_Impl for Drive_Impl {
    unsafe fn get_Name(&self, name: BstrOut) -> Result<()> {
        // mstscax returns a volume-root name with an embedded terminal NUL in the BSTR payload.
        write_bstr(name, &format!("{}\\\0", self.entry.name))
    }

    unsafe fn put_RedirectionState(&self, state: i16) -> Result<()> {
        let state = normalize_variant_bool(state)? == VARIANT_TRUE.0;
        if !self
            .catalog
            .borrow()
            .entries
            .iter()
            .any(|entry| Rc::ptr_eq(entry, &self.entry))
        {
            return Err(Error::from_hresult(E_FAIL));
        }

        let settings = self.settings.borrow();
        if settings.connection_settings_sealed {
            return Err(Error::from_hresult(E_FAIL));
        }
        self.entry.redirection_state.set(state);
        mark_compatibility_persistence_dirty(&settings);
        Ok(())
    }

    unsafe fn get_RedirectionState(&self, state: *mut i16) -> Result<()> {
        write_out(
            state,
            if self.entry.redirection_state.get() {
                VARIANT_TRUE.0
            } else {
                VARIANT_FALSE.0
            },
        )
    }
}

#[implement(IMsRdpDriveCollection)]
struct DriveCollection {
    _lifetime: ServerObjectLifetime,
    catalog: Rc<RefCell<DriveCatalog>>,
    settings: Rc<RefCell<CompatibilitySettings>>,
}

impl DriveCollection {
    fn new(catalog: Rc<RefCell<DriveCatalog>>, settings: Rc<RefCell<CompatibilitySettings>>) -> Self {
        Self {
            _lifetime: ServerObjectLifetime::new(),
            catalog,
            settings,
        }
    }
}

impl IMsRdpDriveCollection_Impl for DriveCollection_Impl {
    unsafe fn RescanDrives(&self, redirect_new_drives: i16) -> Result<()> {
        let redirect_new_drives = normalize_variant_bool(redirect_new_drives)? == VARIANT_TRUE.0;
        if self.settings.borrow().connection_settings_sealed {
            return Err(Error::from_hresult(E_FAIL));
        }
        self.catalog.borrow_mut().rescan(redirect_new_drives);
        mark_compatibility_persistence_dirty(&self.settings.borrow());
        Ok(())
    }

    unsafe fn get_DriveByIndex(&self, index: u32, output: InterfaceOut) -> Result<()> {
        if output.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        let entry = self
            .catalog
            .borrow()
            .entries
            .get(usize::try_from(index).map_err(|_| Error::from_hresult(E_INVALIDARG))?)
            .cloned()
            // mstscax returns E_UNEXPECTED and does not overwrite the output pointer for an
            // out-of-range index.
            .ok_or_else(|| Error::from_hresult(E_UNEXPECTED))?;
        let drive: IMsRdpDrive = Drive::new(Rc::clone(&self.catalog), entry, Rc::clone(&self.settings)).into();
        write_out(output, drive.into_raw().cast())
    }

    unsafe fn get_DriveCount(&self, count: *mut u32) -> Result<()> {
        write_out(
            count,
            u32::try_from(self.catalog.borrow().entries.len()).map_err(|_| Error::from_hresult(E_FAIL))?,
        )
    }
}

#[implement(IMsRdpCameraRedirConfigCollection)]
struct EmptyCameraRedirConfigCollection {
    _lifetime: ServerObjectLifetime,
}

impl EmptyCameraRedirConfigCollection {
    fn new() -> Self {
        Self {
            _lifetime: ServerObjectLifetime::new(),
        }
    }
}

impl IMsRdpCameraRedirConfigCollection_Impl for EmptyCameraRedirConfigCollection_Impl {
    unsafe fn Rescan(&self) -> Result<()> {
        Ok(())
    }
    unsafe fn get_Count(&self, count: *mut u32) -> Result<()> {
        write_out(count, 0)
    }
    unsafe fn get_ByIndex(&self, _: u32, output: InterfaceOut) -> Result<()> {
        write_out(output, ptr::null_mut())?;
        Err(Error::from_hresult(E_INVALIDARG))
    }
    unsafe fn get_BySymbolicLink(&self, _: Bstr, output: InterfaceOut) -> Result<()> {
        write_out(output, ptr::null_mut())?;
        Err(Error::from_hresult(E_INVALIDARG))
    }
    unsafe fn get_ByInstanceId(&self, _: Bstr, output: InterfaceOut) -> Result<()> {
        write_out(output, ptr::null_mut())?;
        Err(Error::from_hresult(E_INVALIDARG))
    }
    unsafe fn AddConfig(&self, _: Bstr, _: i16) -> Result<()> {
        Err(Error::from_hresult(E_NOTIMPL))
    }
    unsafe fn put_RedirectByDefault(&self, _: i16) -> Result<()> {
        Ok(())
    }
    unsafe fn get_RedirectByDefault(&self, output: *mut i16) -> Result<()> {
        write_out(output, VARIANT_FALSE.0)
    }
    unsafe fn put_EncodeVideo(&self, _: i16) -> Result<()> {
        Ok(())
    }
    unsafe fn get_EncodeVideo(&self, output: *mut i16) -> Result<()> {
        write_out(output, VARIANT_FALSE.0)
    }
    unsafe fn put_EncodingQuality(&self, _: i32) -> Result<()> {
        Ok(())
    }
    unsafe fn get_EncodingQuality(&self, output: *mut i32) -> Result<()> {
        write_out(output, 0)
    }
}

#[implement(IMsRdpClipboard)]
struct ClipboardCapabilities {
    _lifetime: ServerObjectLifetime,
    state: Rc<ClipboardState>,
}

impl ClipboardCapabilities {
    fn new(state: Rc<ClipboardState>) -> Self {
        Self {
            _lifetime: ServerObjectLifetime::new(),
            state,
        }
    }
}

impl IMsRdpClipboard_Impl for ClipboardCapabilities_Impl {
    unsafe fn CanSyncLocalClipboardToRemoteSession(&self, can_sync: *mut i16) -> Result<()> {
        write_out(
            can_sync,
            if self.state.is_available() {
                VARIANT_TRUE.0
            } else {
                VARIANT_FALSE.0
            },
        )
    }

    unsafe fn SyncLocalClipboardToRemoteSession(&self) -> Result<()> {
        if self.state.is_available() {
            // The Windows CLIPRDR backend automatically synchronizes clipboard changes.
            Ok(())
        } else {
            Err(Error::from_hresult(E_UNEXPECTED))
        }
    }

    unsafe fn CanSyncRemoteClipboardToLocalSession(&self, can_sync: *mut i16) -> Result<()> {
        write_out(
            can_sync,
            if self.state.is_available() {
                VARIANT_TRUE.0
            } else {
                VARIANT_FALSE.0
            },
        )
    }

    unsafe fn SyncRemoteClipboardToLocalSession(&self) -> Result<()> {
        if self.state.is_available() {
            // The Windows CLIPRDR backend automatically synchronizes clipboard changes.
            Ok(())
        } else {
            Err(Error::from_hresult(E_UNEXPECTED))
        }
    }
}

#[implement(IEnumFORMATETC)]
struct ClipboardFormatEnumerator {
    _lifetime: ServerObjectLifetime,
    has_unicode_text: bool,
    consumed: Cell<bool>,
}

impl ClipboardFormatEnumerator {
    fn new(has_unicode_text: bool, consumed: bool) -> Self {
        Self {
            _lifetime: ServerObjectLifetime::new(),
            has_unicode_text,
            consumed: Cell::new(consumed),
        }
    }
}

impl IEnumFORMATETC_Impl for ClipboardFormatEnumerator_Impl {
    fn Next(&self, celt: u32, formats: *mut FORMATETC, fetched: *mut u32) -> HRESULT {
        if celt != 0 && formats.is_null() {
            return E_POINTER;
        }
        if celt != 1 && fetched.is_null() {
            return E_POINTER;
        }
        if !fetched.is_null() {
            unsafe {
                fetched.write(0);
            }
        }
        if celt == 0 {
            return S_OK;
        }
        if self.consumed.get() || !self.has_unicode_text {
            return S_FALSE;
        }

        unsafe {
            formats.write(unicode_text_format());
        }
        self.consumed.set(true);
        if !fetched.is_null() {
            unsafe {
                fetched.write(1);
            }
        }
        if celt == 1 { S_OK } else { S_FALSE }
    }

    fn Skip(&self, celt: u32) -> Result<()> {
        if celt == 0 {
            return Ok(());
        }
        if self.consumed.get() || !self.has_unicode_text {
            return Err(Error::from_hresult(S_FALSE));
        }

        self.consumed.set(true);
        if celt == 1 {
            Ok(())
        } else {
            Err(Error::from_hresult(S_FALSE))
        }
    }

    fn Reset(&self) -> Result<()> {
        self.consumed.set(false);
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumFORMATETC> {
        Ok(ClipboardFormatEnumerator::new(self.has_unicode_text, self.consumed.get()).into())
    }
}

#[implement(IDataObject)]
struct ClipboardDataObject {
    _lifetime: ServerObjectLifetime,
    unicode_text: Option<Vec<u8>>,
}

impl ClipboardDataObject {
    fn snapshot() -> Result<Self> {
        let unicode_text = if unsafe { IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT.0)) }.is_ok() {
            unsafe {
                OpenClipboard(None)?;
            }

            let result = (|| {
                let handle = match unsafe { GetClipboardData(u32::from(CF_UNICODETEXT.0)) } {
                    Ok(handle) => HGLOBAL(handle.0),
                    Err(_) => return Ok(None),
                };
                let byte_count = unsafe { GlobalSize(handle) };
                if !(2..=MAX_OLE_CLIPBOARD_TEXT_BYTES).contains(&byte_count) || byte_count % 2 != 0 {
                    return Ok(None);
                }

                let source = unsafe { GlobalLock(handle) }.cast::<u8>();
                if source.is_null() {
                    return Ok(None);
                }
                let snapshot = {
                    let data = unsafe { slice::from_raw_parts(source, byte_count) };
                    validated_unicode_text_snapshot(data)
                };
                match (snapshot, unlock_global_memory(handle)) {
                    (Err(error), _) => Err(error),
                    (Ok(_), Err(error)) => Err(error),
                    (Ok(data), Ok(())) => Ok(data),
                }
            })();

            let close_result = unsafe { CloseClipboard() };
            match (result, close_result) {
                (Ok(data), Ok(())) => data,
                (Ok(_), Err(error)) => return Err(error),
                (Err(error), _) => return Err(error),
            }
        } else {
            None
        };

        Ok(Self {
            _lifetime: ServerObjectLifetime::new(),
            unicode_text,
        })
    }

    #[cfg(test)]
    fn from_unicode_text(unicode_text: Option<Vec<u8>>) -> Self {
        Self {
            _lifetime: ServerObjectLifetime::new(),
            unicode_text,
        }
    }

    fn validate_format(&self, format: *const FORMATETC) -> Result<()> {
        let format = unsafe { format.as_ref() }.ok_or_else(|| Error::from_hresult(E_POINTER))?;
        if format.cfFormat != CF_UNICODETEXT.0 {
            return Err(Error::from_hresult(DV_E_FORMATETC));
        }
        if !format.ptd.is_null() {
            return Err(Error::from_hresult(DV_E_DVTARGETDEVICE));
        }
        if format.dwAspect != DVASPECT_CONTENT.0 {
            return Err(Error::from_hresult(DV_E_DVASPECT));
        }
        if format.lindex != -1 {
            return Err(Error::from_hresult(DV_E_LINDEX));
        }
        if format.tymed & TYMED_HGLOBAL.0 as u32 == 0 {
            return Err(Error::from_hresult(DV_E_TYMED));
        }
        if self.unicode_text.is_none() {
            return Err(Error::from_hresult(DV_E_FORMATETC));
        }
        Ok(())
    }
}

impl IDataObject_Impl for ClipboardDataObject_Impl {
    fn GetData(&self, format: *const FORMATETC) -> Result<STGMEDIUM> {
        self.validate_format(format)?;
        let data = self
            .unicode_text
            .as_ref()
            .ok_or_else(|| Error::from_hresult(DV_E_FORMATETC))?;

        let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, data.len()) }?;
        let destination = unsafe { GlobalLock(memory) }.cast::<u8>();
        if destination.is_null() {
            unsafe {
                GlobalFree(Some(memory))?;
            }
            return Err(Error::from_hresult(E_OUTOFMEMORY));
        }
        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr(), destination, data.len());
        }
        if let Err(error) = unlock_global_memory(memory) {
            unsafe {
                GlobalFree(Some(memory))?;
            }
            return Err(error);
        }

        Ok(STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 { hGlobal: memory },
            pUnkForRelease: ManuallyDrop::new(None),
        })
    }

    fn GetDataHere(&self, format: *const FORMATETC, medium: *mut STGMEDIUM) -> Result<()> {
        self.validate_format(format)?;
        let medium = unsafe { medium.as_ref() }.ok_or_else(|| Error::from_hresult(E_POINTER))?;
        if medium.tymed != TYMED_HGLOBAL.0 as u32 {
            return Err(Error::from_hresult(DV_E_TYMED));
        }
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn QueryGetData(&self, format: *const FORMATETC) -> HRESULT {
        match self.validate_format(format) {
            Ok(()) => S_OK,
            Err(error) => error.code(),
        }
    }

    fn GetCanonicalFormatEtc(&self, format: *const FORMATETC, canonical: *mut FORMATETC) -> HRESULT {
        if canonical.is_null() {
            return E_POINTER;
        }
        if format.is_null() {
            unsafe {
                canonical.write(FORMATETC::default());
            }
            return E_POINTER;
        }
        let format = unsafe { format.read() };
        if let Err(error) = self.validate_format(&format) {
            unsafe {
                canonical.write(FORMATETC::default());
            }
            return error.code();
        }

        unsafe {
            canonical.write(FORMATETC {
                ptd: ptr::null_mut(),
                ..format
            });
        }
        DATA_S_SAMEFORMATETC
    }

    fn SetData(&self, format: *const FORMATETC, medium: *const STGMEDIUM, _release: WinBool) -> Result<()> {
        if format.is_null() || medium.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn EnumFormatEtc(&self, direction: u32) -> Result<IEnumFORMATETC> {
        if direction == DATADIR_GET.0 as u32 {
            Ok(ClipboardFormatEnumerator::new(self.unicode_text.is_some(), false).into())
        } else if direction == DATADIR_SET.0 as u32 {
            Err(Error::from_hresult(E_NOTIMPL))
        } else {
            Err(Error::from_hresult(E_INVALIDARG))
        }
    }

    fn DAdvise(&self, _format: *const FORMATETC, _advf: u32, _sink: Ref<'_, IAdviseSink>) -> Result<u32> {
        Err(Error::from_hresult(OLE_E_ADVISENOTSUPPORTED))
    }

    fn DUnadvise(&self, _connection: u32) -> Result<()> {
        Err(Error::from_hresult(OLE_E_NOCONNECTION))
    }

    fn EnumDAdvise(&self) -> Result<IEnumSTATDATA> {
        Err(Error::from_hresult(OLE_E_ADVISENOTSUPPORTED))
    }
}

fn unicode_text_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_UNICODETEXT.0,
        ptd: ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

fn unlock_global_memory(memory: HGLOBAL) -> Result<()> {
    match unsafe { GlobalUnlock(memory) } {
        Ok(()) => Ok(()),
        Err(error) if error.code() == S_OK => Ok(()),
        Err(error) => Err(error),
    }
}

fn validated_unicode_text_snapshot(data: &[u8]) -> Result<Option<Vec<u8>>> {
    if !(2..=MAX_OLE_CLIPBOARD_TEXT_BYTES).contains(&data.len()) || !data.len().is_multiple_of(2) {
        return Ok(None);
    }

    let Some(terminator) = data.chunks_exact(2).position(|unit| unit == [0, 0]) else {
        return Ok(None);
    };
    let utf16 = data[..terminator * 2]
        .chunks_exact(2)
        .map(|unit| u16::from_le_bytes([unit[0], unit[1]]));
    if !char::decode_utf16(utf16).all(|character| character.is_ok()) {
        return Ok(None);
    }

    let text_byte_count = terminator
        .checked_add(1)
        .and_then(|units| units.checked_mul(2))
        .ok_or_else(|| Error::from_hresult(E_OUTOFMEMORY))?;
    Ok(Some(data[..text_byte_count].to_vec()))
}

#[implement(
    IMsRdpClient10,
    IMsRdpClient9,
    IMsRdpClient8,
    IMsRdpClient7,
    IMsRdpClient6,
    IMsRdpClient5,
    IMsRdpClient4,
    IMsRdpClient3,
    IMsRdpClient2,
    IMsRdpClient,
    IConnectionPointContainer,
    IOleObject,
    IOleInPlaceObject,
    IOleInPlaceActiveObject,
    IOleControl,
    IPersistStreamInit,
    IViewObjectEx,
    IViewObject2,
    IViewObject,
    IMsTscNonScriptable,
    IMsRdpClientNonScriptable,
    IMsRdpClientNonScriptable2,
    IMsRdpClientNonScriptable3,
    IMsRdpClientNonScriptable4,
    IMsRdpClientNonScriptable5,
    IMsRdpClientNonScriptable6,
    IMsRdpClientNonScriptable7,
    IMsRdpClientNonScriptable8,
    IMsRdpPreferredRedirectionInfo,
    IMsRdpExtendedSettings
)]
pub(crate) struct Control {
    class_id: GUID,
    settings: RefCell<Settings>,
    compatibility: Rc<RefCell<CompatibilitySettings>>,
    remote_application: RefCell<RemoteApplicationConfiguration>,
    drive_collection: IMsRdpDriveCollection,
    state: Cell<ConnectionState>,
    last_disconnect: Cell<DisconnectInfo>,
    clipboard_state: Rc<ClipboardState>,
    clipboard_backend: RefCell<Option<WinClipboard>>,
    connection_generation: Cell<u64>,
    login_complete_fired: Cell<bool>,
    remote_size: Cell<Option<(i32, i32)>>,
    configured_monitor_topology: RefCell<Option<MonitorTopology>>,
    active_monitor_topology: RefCell<Option<MonitorTopology>>,
    input_sender: RefCell<Option<RdpInputSender>>,
    static_channels: RefCell<BTreeMap<String, ActiveXStaticChannelSpec>>,
    input_database: Rc<RefCell<InputDatabase>>,
    touch_tracker: RefCell<TouchContactTracker>,
    sinks: Rc<RefCell<BTreeMap<u32, EventSink>>>,
    next_cookie: Rc<Cell<u32>>,
    ole_advise_sinks: Rc<RefCell<BTreeMap<u32, IAdviseSink>>>,
    next_ole_advise_cookie: Rc<Cell<u32>>,
    view_advise: RefCell<Option<ViewAdvise>>,
    events: Arc<WorkerEventQueue>,
    event_posted: Arc<AtomicBool>,
    callback_owner: Cell<*const Control_Impl>,
    dispatcher: Cell<HWND>,
    credential_parent: Cell<HWND>,
    native_mstsc_preflight: Cell<NativeMstscPreflight>,
    event_freeze_count: Cell<u32>,
    client_site: RefCell<Option<IOleClientSite>>,
    activex_window: Cell<HWND>,
    renderer_class_acquired: Cell<bool>,
    connection_bar: Cell<HWND>,
    connection_bar_visible: Cell<bool>,
    connection_bar_owner_layout: Cell<Option<ConnectionBarOwnerLayout>>,
    connection_bar_modeless_enabled: Cell<bool>,
    connection_health_window: Cell<HWND>,
    connection_health_status: Cell<ConnectionHealthStatus>,
    connection_health_owner_layout: Cell<Option<ConnectionBarOwnerLayout>>,
    persistence_dirty: Rc<Cell<bool>>,
    activex_rect: Cell<RECT>,
    activex_clip_rect: Cell<Option<RECT>>,
    activex_extent: Cell<SIZE>,
    pending_display_resize: Cell<Option<DisplayLayout>>,
    native_mstsc_display_layout: Cell<Option<(i32, i32)>>,
    frame: Rc<RefCell<Option<Frame>>>,
    presentation_surface: Rc<RefCell<Option<PresentationSurface>>>,
    rail_windows: RefCell<RailWindowManager>,
    presentation_backbuffer: RefCell<Option<PresentationBackbuffer>>,
    next_frame_sequence: Cell<u64>,
    presentation_layout_generation: Cell<u64>,
    traced_frame_layout_generation: Cell<u64>,
    traced_paint_layout_generation: Cell<u64>,
    rpc: Option<ActiveXRpc>,
    rdcleanpath_settings: RefCell<RDCleanPathSettings>,
    rpc_properties: RefCell<Option<PropertySet>>,
    rpc_transport: RefCell<Option<ActiveXTransport>>,
    rpc_kerberos_config: RefCell<Option<ironrdp_connector::credssp::KerberosConfig>>,
    rpc_log_directive: RefCell<Option<String>>,
}

fn build_rpc_touch_event(
    encode_time: u32,
    frames: Vec<ironrdp_agent::ipc::TouchFrameRequest>,
) -> core::result::Result<TouchEventPdu, ironrdp_agent::ipc::Response> {
    let mut built_frames = Vec::with_capacity(frames.len());
    for frame in frames {
        let mut contacts = Vec::with_capacity(frame.contacts.len());
        for contact in frame.contacts {
            let Some(flags) = TouchContactFlags::from_bits(u32::from(contact.flags)) else {
                return Err(ironrdp_agent::ipc::Response::typed_error(
                    ironrdp_agent::ipc::AgentErrorCategory::InvalidRequest,
                    "touch contact flags contain unknown bits",
                ));
            };
            if !flags.is_legal() {
                return Err(ironrdp_agent::ipc::Response::typed_error(
                    ironrdp_agent::ipc::AgentErrorCategory::InvalidRequest,
                    "touch contact flags are not a legal MS-RDPEI combination",
                ));
            }
            contacts.push(TouchContact::new(contact.contact_id, contact.x, contact.y, flags));
        }
        built_frames.push(TouchFrame::new(frame.frame_offset, contacts));
    }
    Ok(TouchEventPdu::new(encode_time, built_frames))
}

fn build_rpc_pen_event(
    encode_time: u32,
    frames: Vec<ironrdp_agent::ipc::PenFrameRequest>,
) -> core::result::Result<PenEventPdu, ironrdp_agent::ipc::Response> {
    let mut built_frames = Vec::with_capacity(frames.len());
    for frame in frames {
        let mut contacts = Vec::with_capacity(frame.contacts.len());
        for contact in frame.contacts {
            let Some(flags) = PenContactFlags::from_bits(u32::from(contact.flags)) else {
                return Err(ironrdp_agent::ipc::Response::typed_error(
                    ironrdp_agent::ipc::AgentErrorCategory::InvalidRequest,
                    "pen contact flags contain unknown bits",
                ));
            };
            if !flags.is_legal() {
                return Err(ironrdp_agent::ipc::Response::typed_error(
                    ironrdp_agent::ipc::AgentErrorCategory::InvalidRequest,
                    "pen contact flags are not a legal MS-RDPEI combination",
                ));
            }
            let mut pen = PenContact::new(contact.device_id, contact.x, contact.y, flags);
            if let Some(pen_flags_bits) = contact.pen_flags {
                let Some(pen_flags) = PenFlags::from_bits(pen_flags_bits) else {
                    return Err(ironrdp_agent::ipc::Response::typed_error(
                        ironrdp_agent::ipc::AgentErrorCategory::InvalidRequest,
                        "pen flags contain unknown bits",
                    ));
                };
                pen = pen.with_pen_flags(pen_flags);
            }
            if let Some(pressure) = contact.pressure {
                pen = pen.with_pressure(pressure);
            }
            if let Some(rotation) = contact.rotation {
                pen = pen.with_rotation(rotation);
            }
            match (contact.tilt_x, contact.tilt_y) {
                (Some(tilt_x), Some(tilt_y)) => pen = pen.with_tilt(tilt_x, tilt_y),
                (Some(tilt_x), None) => {
                    pen.fields_present.insert(PenContactDataFlags::TILTX_PRESENT);
                    pen.fields.tilt_x = Some(tilt_x);
                }
                (None, Some(tilt_y)) => {
                    pen.fields_present.insert(PenContactDataFlags::TILTY_PRESENT);
                    pen.fields.tilt_y = Some(tilt_y);
                }
                (None, None) => {}
            }
            contacts.push(pen);
        }
        built_frames.push(PenFrame::new(frame.frame_offset, contacts));
    }
    Ok(PenEventPdu::new(encode_time, built_frames))
}

fn rpc_control_error(error: Error) -> ironrdp_agent::ipc::Response {
    let category = if error.code() == E_INVALIDARG || error.code() == E_NOTIMPL {
        ironrdp_agent::ipc::AgentErrorCategory::InvalidRequest
    } else {
        ironrdp_agent::ipc::AgentErrorCategory::Unavailable
    };
    ironrdp_agent::ipc::Response::typed_error(category, format!("ActiveX control rejected the request: {error}"))
}

fn active_x_property_snapshot(settings: &Settings, compatibility: &CompatibilitySettings) -> PropertySet {
    let mut properties = PropertySet::new();
    if !settings.server.is_empty() {
        properties.insert("full address", settings.server.clone());
    }
    if !settings.username.is_empty() {
        properties.insert("username", settings.username.clone());
    }
    if !settings.domain.is_empty() {
        properties.insert("domain", settings.domain.clone());
    }
    properties.insert("desktopwidth", settings.desktop_width);
    properties.insert("desktopheight", settings.desktop_height);
    properties.insert("ironrdp_colordepth", settings.color_depth);
    if let Some(value) = compatibility.enable_credssp {
        properties.insert("enablecredsspsupport", value);
    }
    if let Some(value) = compatibility.enable_tls {
        properties.insert("ironrdp_tls", value);
    }
    if let Some(value) = compatibility.autologon {
        properties.insert("ironrdp_autologon", value);
    }
    if let Some(value) = compatibility.desktop_scale_factor {
        properties.insert("desktopscalefactor", value);
    }
    properties.insert("redirectclipboard", compatibility.redirect_clipboard);
    properties.insert("redirectwebauthn", compatibility.redirect_webauthn);
    properties.insert("ironrdp_smartcard", compatibility.redirect_smart_cards);
    properties.insert("compression", compatibility.compression.unwrap_or(true));
    properties
}

impl Control {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::new_for_class(CLSID_IRONRDP_ACTIVEX)
    }

    pub(crate) fn new_for_class(class_id: GUID) -> Self {
        com::add_object();
        let persistence_dirty = Rc::new(Cell::new(false));
        let compatibility = Rc::new(RefCell::new(CompatibilitySettings::default()));
        compatibility.borrow_mut().persistence_dirty = Some(Rc::clone(&persistence_dirty));
        let drive_catalog = Rc::clone(&compatibility.borrow().drive_catalog);
        let drive_collection: IMsRdpDriveCollection =
            DriveCollection::new(drive_catalog, Rc::clone(&compatibility)).into();
        let input_database = Rc::new(RefCell::new(InputDatabase::new()));
        let frame = Rc::new(RefCell::new(None));
        let presentation_surface = Rc::new(RefCell::new(None));
        let rail_windows = RefCell::new(RailWindowManager::new(
            Rc::clone(&input_database),
            Rc::clone(&compatibility),
            Rc::clone(&frame),
            Rc::clone(&presentation_surface),
        ));
        Self {
            class_id,
            settings: RefCell::new(Settings::default()),
            compatibility,
            remote_application: RefCell::new(RemoteApplicationConfiguration::default()),
            drive_collection,
            state: Cell::new(ConnectionState::Disconnected),
            last_disconnect: Cell::new(DisconnectInfo::no_info()),
            clipboard_state: Rc::new(ClipboardState {
                enabled_for_session: Cell::new(false),
                connected: Cell::new(false),
            }),
            clipboard_backend: RefCell::new(None),
            connection_generation: Cell::new(0),
            login_complete_fired: Cell::new(false),
            remote_size: Cell::new(None),
            configured_monitor_topology: RefCell::new(None),
            active_monitor_topology: RefCell::new(None),
            input_sender: RefCell::new(None),
            static_channels: RefCell::new(BTreeMap::new()),
            input_database,
            touch_tracker: RefCell::new(TouchContactTracker::new()),
            sinks: Rc::new(RefCell::new(BTreeMap::new())),
            next_cookie: Rc::new(Cell::new(1)),
            ole_advise_sinks: Rc::new(RefCell::new(BTreeMap::new())),
            next_ole_advise_cookie: Rc::new(Cell::new(1)),
            view_advise: RefCell::new(None),
            events: Arc::new(WorkerEventQueue::new()),
            event_posted: Arc::new(AtomicBool::new(false)),
            callback_owner: Cell::new(ptr::null()),
            dispatcher: Cell::new(HWND(ptr::null_mut())),
            credential_parent: Cell::new(HWND(ptr::null_mut())),
            native_mstsc_preflight: Cell::new(NativeMstscPreflight::Idle),
            event_freeze_count: Cell::new(0),
            client_site: RefCell::new(None),
            activex_window: Cell::new(HWND(ptr::null_mut())),
            renderer_class_acquired: Cell::new(false),
            connection_bar: Cell::new(HWND(ptr::null_mut())),
            connection_bar_visible: Cell::new(false),
            connection_bar_owner_layout: Cell::new(None),
            connection_bar_modeless_enabled: Cell::new(true),
            connection_health_window: Cell::new(HWND(ptr::null_mut())),
            connection_health_status: Cell::new(ConnectionHealthStatus::Hidden),
            connection_health_owner_layout: Cell::new(None),
            persistence_dirty,
            activex_rect: Cell::new(RECT::default()),
            activex_clip_rect: Cell::new(None),
            activex_extent: Cell::new(SIZE { cx: 27_093, cy: 20_320 }),
            pending_display_resize: Cell::new(None),
            native_mstsc_display_layout: Cell::new(None),
            frame,
            presentation_surface,
            rail_windows,
            presentation_backbuffer: RefCell::new(None),
            next_frame_sequence: Cell::new(0),
            presentation_layout_generation: Cell::new(0),
            traced_frame_layout_generation: Cell::new(0),
            traced_paint_layout_generation: Cell::new(0),
            rpc: ActiveXRpc::from_environment(),
            rdcleanpath_settings: RefCell::new(RDCleanPathSettings::default()),
            rpc_properties: RefCell::new(None),
            rpc_transport: RefCell::new(None),
            rpc_kerberos_config: RefCell::new(None),
            rpc_log_directive: RefCell::new(None),
        }
    }

    fn replace_rdcleanpath_settings(&self, url: String, token: String) -> Result<()> {
        let mut settings = self.rdcleanpath_settings.borrow_mut();
        let mut replacement = settings.clone();
        replacement.set_url(url)?;
        replacement.set_token(token)?;
        *settings = replacement;
        Ok(())
    }

    fn rdcleanpath_transport(&self) -> Result<Option<ActiveXTransport>> {
        self.rdcleanpath_settings.borrow().transport()
    }

    fn apply_rdcleanpath_settings_to_client_properties(&self, properties: &mut PropertySet) -> Result<()> {
        self.rdcleanpath_settings
            .borrow()
            .apply_to_client_properties(properties)
    }

    fn clear_rdcleanpath_token(&self) {
        self.rdcleanpath_settings.borrow_mut().token = None;
    }

    fn remember_callback_owner(&self, owner: *const Control_Impl) {
        self.callback_owner.set(owner);
        if let Some(rpc) = &self.rpc
            && let Ok(dispatcher) = self.ensure_dispatcher()
            && let Err(error) = rpc.start(dispatcher)
        {
            tracing::warn!(?error, "Unable to start ActiveX RPC listener");
        }
    }

    fn connection_bar_owner(&self) -> Option<HWND> {
        let renderer = self.activex_window.get();
        if renderer.0.is_null() || !unsafe { IsWindow(Some(renderer)) }.as_bool() {
            return None;
        }

        let owner = unsafe { GetAncestor(renderer, GA_ROOTOWNER) };
        (!owner.0.is_null() && unsafe { IsWindow(Some(owner)) }.as_bool()).then_some(owner)
    }

    fn connection_health_owner(&self) -> Option<HWND> {
        let renderer = self.activex_window.get();
        (!renderer.0.is_null() && unsafe { IsWindow(Some(renderer)) }.as_bool()).then_some(renderer)
    }

    fn current_connection_health_owner_layout(&self) -> Option<ConnectionBarOwnerLayout> {
        let owner = self.connection_health_owner()?;
        let mut rect = RECT::default();
        unsafe { GetWindowRect(owner, &mut rect) }.ok()?;
        Some(ConnectionBarOwnerLayout {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
            dpi: connection_bar_dpi(owner),
        })
    }

    fn current_connection_bar_owner_layout(&self) -> Option<ConnectionBarOwnerLayout> {
        let owner = self.connection_bar_owner()?;
        let mut rect = RECT::default();
        unsafe { GetWindowRect(owner, &mut rect) }.ok()?;
        Some(ConnectionBarOwnerLayout {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
            dpi: connection_bar_dpi(owner),
        })
    }

    fn native_connection_bar_actions_enabled(&self) -> bool {
        native_mstsc_shell_integration_enabled() && self.native_mstsc_shell_window().is_some()
    }

    fn update_connection_bar(&self) {
        let native_shell_presentation = self.native_connection_bar_actions_enabled();
        let eligible = {
            let settings = self.settings.borrow();
            let compatibility = self.compatibility.borrow();
            connection_bar_is_eligible(self.state.get(), &settings, &compatibility, native_shell_presentation)
        };
        if !eligible {
            if let Err(error) = self.destroy_connection_bar() {
                tracing::debug!(?error, "Unable to destroy ActiveX connection bar");
            }
            return;
        }

        let window = self.connection_bar.get();
        if !window.0.is_null() && unsafe { IsWindow(Some(window)) }.as_bool() {
            self.refresh_connection_bar(window);
            if self.connection_bar_visible.get() {
                self.reset_connection_bar_auto_hide(window);
            }
            return;
        }

        self.connection_bar.set(HWND(ptr::null_mut()));
        self.connection_bar_visible.set(false);
        self.connection_bar_owner_layout.set(None);
        if let Err(error) = self.create_connection_bar() {
            tracing::warn!(?error, "Unable to create ActiveX connection bar");
        }
    }

    fn create_connection_bar(&self) -> Result<()> {
        let Some(owner) = self.connection_bar_owner() else {
            return Ok(());
        };
        let callback_owner = self.callback_owner.get();
        if callback_owner.is_null() {
            return Err(Error::from_hresult(E_UNEXPECTED));
        }

        let mut owner_rect = RECT::default();
        unsafe { GetWindowRect(owner, &mut owner_rect) }?;
        let dpi = connection_bar_dpi(owner);
        let (width, height) = connection_bar_size(dpi);
        let (x, y) = connection_bar_position_for_width(owner_rect, width);
        let (title, pin_text, show_pin_button, show_minimize_button, show_restore_button, native_actions) = {
            let settings = self.settings.borrow();
            let compatibility = self.compatibility.borrow();
            let title = connection_bar_title(&compatibility.connection_bar_text, &settings.server);
            (
                if title.is_empty() {
                    "IronRDP".to_owned()
                } else {
                    title.to_owned()
                },
                if compatibility.pin_connection_bar != VARIANT_FALSE.0 {
                    "Unpin"
                } else {
                    "Pin"
                },
                compatibility.connection_bar_show_pin_button != VARIANT_FALSE.0,
                compatibility.connection_bar_show_minimize_button != VARIANT_FALSE.0,
                compatibility.connection_bar_show_restore_button != VARIANT_FALSE.0,
                self.native_connection_bar_actions_enabled(),
            )
        };

        acquire_connection_bar_class()?;
        let module = match com::retain_module_reference() {
            Ok(module) => module,
            Err(error) => {
                release_connection_bar_class();
                return Err(error);
            }
        };
        let instance = match unsafe { GetModuleHandleW(None) } {
            Ok(instance) => instance,
            Err(error) => {
                com::release_module_reference(module);
                release_connection_bar_class();
                return Err(error);
            }
        };
        let callback_context = Rc::new(ControlWindowContext {
            control: callback_owner,
            module,
            closing: AtomicBool::new(false),
            window_reference_released: Cell::new(false),
            orphaned: AtomicBool::new(false),
        });
        let callback_context_raw = Rc::into_raw(Rc::clone(&callback_context));
        let title = HSTRING::from(title);
        let window = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("IronRDP.ActiveX.ConnectionBar"),
                PCWSTR(title.as_ptr()),
                WS_POPUP,
                x,
                y,
                width,
                height,
                Some(owner),
                None,
                Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                Some(callback_context_raw.cast()),
            )
        };
        let window = match window {
            Ok(window) => {
                drop(callback_context);
                window
            }
            Err(error) => {
                if !callback_context.window_reference_released.get() {
                    unsafe {
                        drop(Rc::from_raw(callback_context_raw));
                    }
                }
                drop(callback_context);
                release_connection_bar_class();
                return Err(error);
            }
        };

        let title_label = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                PCWSTR(title.as_ptr()),
                WS_CHILD | WS_VISIBLE,
                8,
                6,
                172,
                24,
                Some(window),
                Some(HMENU(CONNECTION_BAR_TITLE_ID as *mut c_void)),
                Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                None,
            )
        };
        let pin_text = HSTRING::from(pin_text);
        let information_button = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                w!("Info"),
                connection_bar_button_style(true),
                180,
                6,
                76,
                24,
                Some(window),
                Some(HMENU(CONNECTION_BAR_INFORMATION_BUTTON_ID as *mut c_void)),
                Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                None,
            )
        };
        let pin_button = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                PCWSTR(pin_text.as_ptr()),
                connection_bar_button_style(show_pin_button),
                256,
                6,
                80,
                24,
                Some(window),
                Some(HMENU(CONNECTION_BAR_PIN_BUTTON_ID as *mut c_void)),
                Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                None,
            )
        };
        let disconnect_button = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                w!("Disconnect"),
                connection_bar_button_style(true),
                664,
                6,
                110,
                24,
                Some(window),
                Some(HMENU(CONNECTION_BAR_DISCONNECT_BUTTON_ID as *mut c_void)),
                Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                None,
            )
        };
        let native_buttons = if native_actions {
            (|| unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("BUTTON"),
                    w!("Minimize"),
                    connection_bar_button_style(show_minimize_button),
                    336,
                    6,
                    80,
                    24,
                    Some(window),
                    Some(HMENU(CONNECTION_BAR_MINIMIZE_BUTTON_ID as *mut c_void)),
                    Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                    None,
                )?;
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("BUTTON"),
                    w!("Restore"),
                    connection_bar_button_style(show_restore_button),
                    416,
                    6,
                    72,
                    24,
                    Some(window),
                    Some(HMENU(CONNECTION_BAR_RESTORE_BUTTON_ID as *mut c_void)),
                    Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                    None,
                )?;
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("BUTTON"),
                    w!("Full screen"),
                    connection_bar_button_style(true),
                    488,
                    6,
                    104,
                    24,
                    Some(window),
                    Some(HMENU(CONNECTION_BAR_FULLSCREEN_BUTTON_ID as *mut c_void)),
                    Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                    None,
                )?;
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("BUTTON"),
                    w!("Close"),
                    connection_bar_button_style(true),
                    592,
                    6,
                    72,
                    24,
                    Some(window),
                    Some(HMENU(CONNECTION_BAR_CLOSE_BUTTON_ID as *mut c_void)),
                    Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                    None,
                )
            })()
        } else {
            Ok(HWND(ptr::null_mut()))
        };
        if let Err(error) = title_label
            .and(information_button)
            .and(pin_button)
            .and(disconnect_button)
            .and(native_buttons)
            .map(|_| ())
        {
            unsafe {
                let _ = DestroyWindow(window);
            }
            return Err(error);
        }
        if let Err(error) = Self::subclass_connection_bar_buttons(window) {
            unsafe {
                let _ = DestroyWindow(window);
            }
            return Err(error);
        }

        self.connection_bar.set(window);
        unsafe {
            let _ = EnableWindow(window, self.connection_bar_modeless_enabled.get());
        }
        self.position_connection_bar(window);
        self.show_connection_bar(window, false);
        Ok(())
    }

    fn refresh_connection_bar(&self, window: HWND) {
        let (title, pin_text, show_pin_button, show_minimize_button, show_restore_button) = {
            let settings = self.settings.borrow();
            let compatibility = self.compatibility.borrow();
            let title = connection_bar_title(&compatibility.connection_bar_text, &settings.server);
            (
                if title.is_empty() {
                    "IronRDP".to_owned()
                } else {
                    title.to_owned()
                },
                if compatibility.pin_connection_bar != VARIANT_FALSE.0 {
                    "Unpin"
                } else {
                    "Pin"
                },
                compatibility.connection_bar_show_pin_button != VARIANT_FALSE.0,
                compatibility.connection_bar_show_minimize_button != VARIANT_FALSE.0,
                compatibility.connection_bar_show_restore_button != VARIANT_FALSE.0,
            )
        };
        let title = HSTRING::from(title);
        unsafe {
            let _ = SetWindowTextW(window, PCWSTR(title.as_ptr()));
        }
        if let Ok(title_label) = unsafe { GetDlgItem(Some(window), CONNECTION_BAR_TITLE_ID as i32) } {
            unsafe {
                let _ = SetWindowTextW(title_label, PCWSTR(title.as_ptr()));
            }
        }
        if let Ok(pin_button) = unsafe { GetDlgItem(Some(window), CONNECTION_BAR_PIN_BUTTON_ID as i32) } {
            let pin_text = HSTRING::from(pin_text);
            unsafe {
                let _ = SetWindowTextW(pin_button, PCWSTR(pin_text.as_ptr()));
                let _ = ShowWindow(pin_button, if show_pin_button { SW_SHOWNA } else { SW_HIDE });
            }
        }
        for (button_id, visible) in [
            (CONNECTION_BAR_MINIMIZE_BUTTON_ID, show_minimize_button),
            (CONNECTION_BAR_RESTORE_BUTTON_ID, show_restore_button),
        ] {
            if let Ok(button) = unsafe { GetDlgItem(Some(window), button_id as i32) } {
                unsafe {
                    let _ = ShowWindow(button, if visible { SW_SHOWNA } else { SW_HIDE });
                }
            }
        }
        self.position_connection_bar(window);
    }

    fn subclass_connection_bar_buttons(window: HWND) -> Result<()> {
        for button_id in CONNECTION_BAR_BUTTON_IDS {
            let button = unsafe { GetDlgItem(Some(window), *button_id as i32) }?;
            if !unsafe { SetWindowSubclass(button, Some(connection_bar_button_subclass_proc), *button_id, 0) }.as_bool()
            {
                return Err(Error::from_thread());
            }
        }
        Ok(())
    }

    fn focus_connection_bar_button(&self, window: HWND, current: HWND, reverse: bool) -> bool {
        let mut visible_buttons = Vec::with_capacity(CONNECTION_BAR_BUTTON_IDS.len());
        for button_id in CONNECTION_BAR_BUTTON_IDS {
            let Ok(button) = (unsafe { GetDlgItem(Some(window), *button_id as i32) }) else {
                continue;
            };
            if unsafe { IsWindowVisible(button) }.as_bool() && unsafe { IsWindowEnabled(button) }.as_bool() {
                visible_buttons.push((*button_id, button));
            }
        }
        let current_id = visible_buttons
            .iter()
            .find_map(|(button_id, button)| (*button == current).then_some(*button_id))
            .unwrap_or_default();
        let visible_ids = visible_buttons
            .iter()
            .map(|(button_id, _)| *button_id)
            .collect::<Vec<_>>();
        let Some(next_id) = next_connection_bar_button_id(current_id, &visible_ids, reverse) else {
            return false;
        };
        let Some((_, next)) = visible_buttons.iter().find(|(button_id, _)| *button_id == next_id) else {
            return false;
        };
        if unsafe { SetFocus(Some(*next)) }.is_err() {
            return false;
        }
        self.reset_connection_bar_auto_hide(window);
        true
    }

    fn position_connection_bar(&self, window: HWND) {
        let Some(owner_layout) = self.current_connection_bar_owner_layout() else {
            return;
        };
        let (width, height) = connection_bar_size(owner_layout.dpi);
        let (x, y) = connection_bar_position_for_width(
            RECT {
                left: owner_layout.left,
                top: owner_layout.top,
                right: owner_layout.right,
                bottom: owner_layout.bottom,
            },
            width,
        );
        if let Err(error) = unsafe {
            SetWindowPos(
                window,
                None,
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOZORDER,
            )
        } {
            tracing::debug!(?error, "Unable to position ActiveX connection bar");
            return;
        }
        self.connection_bar_owner_layout.set(Some(owner_layout));
        if let Ok(title_label) = unsafe { GetDlgItem(Some(window), CONNECTION_BAR_TITLE_ID as i32) } {
            let rect = connection_bar_title_rect(owner_layout.dpi);
            if let Err(error) = unsafe {
                SetWindowPos(
                    title_label,
                    None,
                    rect.left,
                    rect.top,
                    rect.right.saturating_sub(rect.left),
                    rect.bottom.saturating_sub(rect.top),
                    SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOZORDER,
                )
            } {
                tracing::debug!(?error, "Unable to position ActiveX connection bar title");
            }
        }
        for button_id in [
            CONNECTION_BAR_INFORMATION_BUTTON_ID,
            CONNECTION_BAR_PIN_BUTTON_ID,
            CONNECTION_BAR_DISCONNECT_BUTTON_ID,
            CONNECTION_BAR_MINIMIZE_BUTTON_ID,
            CONNECTION_BAR_RESTORE_BUTTON_ID,
            CONNECTION_BAR_FULLSCREEN_BUTTON_ID,
            CONNECTION_BAR_CLOSE_BUTTON_ID,
        ] {
            let Some(rect) = connection_bar_button_rect(button_id, owner_layout.dpi) else {
                continue;
            };
            let Ok(button) = (unsafe { GetDlgItem(Some(window), button_id as i32) }) else {
                continue;
            };
            if let Err(error) = unsafe {
                SetWindowPos(
                    button,
                    None,
                    rect.left,
                    rect.top,
                    rect.right.saturating_sub(rect.left),
                    rect.bottom.saturating_sub(rect.top),
                    SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOZORDER,
                )
            } {
                tracing::debug!(?error, button_id, "Unable to position ActiveX connection bar button");
            }
        }
    }

    fn show_connection_bar(&self, window: HWND, user_pulled_down: bool) {
        self.position_connection_bar(window);
        unsafe {
            let _ = ShowWindow(window, SW_SHOWNA);
            let _ = SetTimer(
                Some(window),
                CONNECTION_BAR_OWNER_LAYOUT_TIMER_ID,
                CONNECTION_BAR_OWNER_LAYOUT_POLL_MILLISECONDS,
                None,
            );
        }
        self.connection_bar_visible.set(true);
        if user_pulled_down {
            self.fire_event(DISPID_ON_CONNECTION_BAR_PULL_DOWN, &[]);
        }
        self.reset_connection_bar_auto_hide(window);
    }

    fn expose_connection_bar(&self) {
        let window = self.connection_bar.get();
        if self.connection_bar_visible.get() || window.0.is_null() || !unsafe { IsWindow(Some(window)) }.as_bool() {
            return;
        }
        self.show_connection_bar(window, true);
    }

    fn reset_connection_bar_auto_hide(&self, window: HWND) {
        if self.compatibility.borrow().pin_connection_bar != VARIANT_FALSE.0 {
            unsafe {
                let _ = KillTimer(Some(window), CONNECTION_BAR_AUTO_HIDE_TIMER_ID);
            }
        } else {
            unsafe {
                let _ = SetTimer(
                    Some(window),
                    CONNECTION_BAR_AUTO_HIDE_TIMER_ID,
                    CONNECTION_BAR_AUTO_HIDE_MILLISECONDS,
                    None,
                );
            }
        }
    }

    fn pause_connection_bar_auto_hide(&self, window: HWND) {
        if self.compatibility.borrow().pin_connection_bar == VARIANT_FALSE.0 {
            unsafe {
                let _ = KillTimer(Some(window), CONNECTION_BAR_AUTO_HIDE_TIMER_ID);
            }
        }
    }

    fn track_connection_bar_mouse_leave(window: HWND) {
        let mut tracking = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: window,
            dwHoverTime: 0,
        };
        if let Err(error) = unsafe { TrackMouseEvent(&mut tracking) } {
            tracing::debug!(?error, "Unable to track ActiveX connection bar mouse leave");
        }
    }

    fn connection_bar_mouse_move(&self, window: HWND) {
        self.pause_connection_bar_auto_hide(window);
        Self::track_connection_bar_mouse_leave(window);
    }

    fn connection_bar_mouse_leave(&self, window: HWND) {
        let mut cursor = POINT::default();
        let mut rect = RECT::default();
        let cursor_is_still_over_bar = unsafe { GetCursorPos(&mut cursor) }.is_ok()
            && unsafe { GetWindowRect(window, &mut rect) }.is_ok()
            && point_is_inside_rect(cursor, rect);
        if !cursor_is_still_over_bar {
            self.reset_connection_bar_auto_hide(window);
        }
    }

    fn hide_connection_bar(&self, window: HWND) {
        if self.connection_bar.get() != window || self.compatibility.borrow().pin_connection_bar != VARIANT_FALSE.0 {
            return;
        }
        unsafe {
            let _ = KillTimer(Some(window), CONNECTION_BAR_AUTO_HIDE_TIMER_ID);
            let _ = KillTimer(Some(window), CONNECTION_BAR_OWNER_LAYOUT_TIMER_ID);
            let _ = ShowWindow(window, SW_HIDE);
        }
        self.connection_bar_visible.set(false);
    }

    fn handle_connection_bar_command(&self, window: HWND, wparam: WPARAM) {
        match wparam.0 & 0xffff {
            CONNECTION_BAR_INFORMATION_BUTTON_ID => self.show_connection_information_dialog(),
            CONNECTION_BAR_PIN_BUTTON_ID => {
                let pinned = {
                    let mut compatibility = self.compatibility.borrow_mut();
                    compatibility.pin_connection_bar = if compatibility.pin_connection_bar == VARIANT_FALSE.0 {
                        VARIANT_TRUE.0
                    } else {
                        VARIANT_FALSE.0
                    };
                    compatibility.pin_connection_bar != VARIANT_FALSE.0
                };
                self.refresh_connection_bar(window);
                if pinned {
                    unsafe {
                        let _ = KillTimer(Some(window), CONNECTION_BAR_AUTO_HIDE_TIMER_ID);
                    }
                } else {
                    self.reset_connection_bar_auto_hide(window);
                }
            }
            CONNECTION_BAR_DISCONNECT_BUTTON_ID => {
                self.pause_connection_bar_auto_hide(window);
                match self.confirm_connection_bar_disconnect() {
                    Ok(true) => {
                        if let Err(error) = self.stop_connection() {
                            tracing::debug!(?error, "Unable to disconnect from ActiveX connection bar");
                        } else {
                            self.update_connection_bar();
                        }
                    }
                    Ok(false) => self.reset_connection_bar_auto_hide(window),
                    Err(error) => {
                        tracing::warn!(?error, "Unable to display ActiveX disconnect confirmation");
                        self.reset_connection_bar_auto_hide(window);
                    }
                }
            }
            CONNECTION_BAR_MINIMIZE_BUTTON_ID => {
                if let Some(shell) = self.native_mstsc_shell_window() {
                    unsafe {
                        let _ = ShowWindow(shell, SW_MINIMIZE);
                    }
                }
            }
            CONNECTION_BAR_RESTORE_BUTTON_ID => {
                if let Some(shell) = self.native_mstsc_shell_window() {
                    unsafe {
                        let _ = ShowWindow(shell, SW_RESTORE);
                    }
                }
            }
            CONNECTION_BAR_FULLSCREEN_BUTTON_ID => {
                self.request_fullscreen_toggle();
            }
            CONNECTION_BAR_CLOSE_BUTTON_ID => {
                if let Some(shell) = self.native_mstsc_shell_window()
                    && let Err(error) = unsafe { PostMessageW(Some(shell), WM_CLOSE, WPARAM(0), LPARAM(0)) }
                {
                    tracing::debug!(?error, "Unable to request native mstsc host close from connection bar");
                }
            }
            _ => return,
        }
        self.restore_renderer_focus();
    }

    fn restore_renderer_focus(&self) {
        let renderer = self.activex_window.get();
        if !renderer.0.is_null()
            && unsafe { IsWindow(Some(renderer)) }.as_bool()
            && let Err(error) = unsafe { SetFocus(Some(renderer)) }
        {
            tracing::debug!(
                ?error,
                "Unable to restore ActiveX renderer focus after connection bar command"
            );
        }
    }

    fn destroy_connection_bar(&self) -> Result<()> {
        let window = self.connection_bar.replace(HWND(ptr::null_mut()));
        self.connection_bar_visible.set(false);
        self.connection_bar_owner_layout.set(None);
        if window.0.is_null() || !unsafe { IsWindow(Some(window)) }.as_bool() {
            return Ok(());
        }
        unsafe {
            let _ = KillTimer(Some(window), CONNECTION_BAR_AUTO_HIDE_TIMER_ID);
            let _ = KillTimer(Some(window), CONNECTION_BAR_OWNER_LAYOUT_TIMER_ID);
        }
        if let Err(error) = destroy_control_window(window) {
            defer_window_resource_release(window);
            return Err(error);
        }
        Ok(())
    }

    fn deactivate_owned_ui(&self) {
        self.release_input();
        self.clear_connection_health_window();
        if let Err(error) = self.destroy_connection_bar() {
            tracing::debug!(
                ?error,
                "Unable to destroy ActiveX connection bar during UI deactivation"
            );
        }
    }

    fn renderer_visibility_changed(&self, visible: bool) {
        if visible {
            self.update_connection_bar();
        } else {
            self.deactivate_owned_ui();
        }
    }

    fn renderer_geometry_changed(&self) {
        let connection_bar = self.connection_bar.get();
        if !connection_bar.0.is_null() && unsafe { IsWindow(Some(connection_bar)) }.as_bool() {
            self.position_connection_bar(connection_bar);
        }

        let connection_health = self.connection_health_window.get();
        if !connection_health.0.is_null() && unsafe { IsWindow(Some(connection_health)) }.as_bool() {
            self.refresh_connection_health_window(connection_health);
        }
    }

    fn renderer_dpi_changed(&self) {
        self.renderer_geometry_changed();
    }

    fn set_connection_bar_modeless_enabled(&self, enabled: bool) {
        self.connection_bar_modeless_enabled.set(enabled);
        let window = self.connection_bar.get();
        if !window.0.is_null() && unsafe { IsWindow(Some(window)) }.as_bool() {
            unsafe {
                let _ = EnableWindow(window, enabled);
            }
        }
    }

    fn in_place_window_activation_changed(&self, active: bool) {
        if !active {
            self.release_input();
        }
    }

    fn set_connection_health_status(&self, status: ConnectionHealthStatus) {
        self.connection_health_status.set(status);
        self.update_connection_health_window();
    }

    fn report_reconnect_worker_progress(&self, attempt: u32, maximum: u32) {
        let Some(status) = ConnectionHealthStatus::reconnecting(attempt, maximum) else {
            tracing::warn!(attempt, maximum, "Ignoring invalid reconnect worker progress");
            return;
        };
        self.set_connection_health_status(status);
    }

    fn report_display_resize_fallback(&self) {
        if self.state.get() == ConnectionState::Connected {
            // Display Control has already reported that it is reconnecting with the requested
            // size. The worker supplies no retry count, so do not present reconnect attempts.
            self.set_connection_health_status(ConnectionHealthStatus::UpdatingDisplay);
        }
    }

    fn update_connection_health_window(&self) {
        if self.connection_health_status.get() == ConnectionHealthStatus::Hidden {
            self.clear_connection_health_window();
            return;
        }

        let window = self.connection_health_window.get();
        if !window.0.is_null() && unsafe { IsWindow(Some(window)) }.as_bool() {
            self.refresh_connection_health_window(window);
            return;
        }

        self.connection_health_window.set(HWND(ptr::null_mut()));
        self.connection_health_owner_layout.set(None);
        if let Err(error) = self.create_connection_health_window() {
            tracing::warn!(?error, "Unable to create ActiveX connection health window");
        }
    }

    fn create_connection_health_window(&self) -> Result<()> {
        let Some(owner) = self.connection_health_owner() else {
            return Ok(());
        };
        let callback_owner = self.callback_owner.get();
        if callback_owner.is_null() {
            return Err(Error::from_hresult(E_UNEXPECTED));
        }

        let mut owner_rect = RECT::default();
        unsafe { GetWindowRect(owner, &mut owner_rect) }?;
        let dpi = connection_bar_dpi(owner);
        let (width, height) = connection_health_size(dpi);
        let (x, y) = connection_health_position(owner_rect, width, height);

        acquire_connection_health_class()?;
        let module = match com::retain_module_reference() {
            Ok(module) => module,
            Err(error) => {
                release_connection_health_class();
                return Err(error);
            }
        };
        let instance = match unsafe { GetModuleHandleW(None) } {
            Ok(instance) => instance,
            Err(error) => {
                com::release_module_reference(module);
                release_connection_health_class();
                return Err(error);
            }
        };
        let callback_context = Rc::new(ControlWindowContext {
            control: callback_owner,
            module,
            closing: AtomicBool::new(false),
            window_reference_released: Cell::new(false),
            orphaned: AtomicBool::new(false),
        });
        let callback_context_raw = Rc::into_raw(Rc::clone(&callback_context));
        let window = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("IronRDP.ActiveX.ConnectionHealth"),
                w!("IronRDP connection status"),
                WS_POPUP,
                x,
                y,
                width,
                height,
                Some(owner),
                None,
                Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                Some(callback_context_raw.cast()),
            )
        };
        let window = match window {
            Ok(window) => {
                drop(callback_context);
                window
            }
            Err(error) => {
                if !callback_context.window_reference_released.get() {
                    unsafe {
                        drop(Rc::from_raw(callback_context_raw));
                    }
                }
                drop(callback_context);
                release_connection_health_class();
                return Err(error);
            }
        };

        let main_label = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!(""),
                WS_CHILD | WS_VISIBLE,
                0,
                0,
                0,
                0,
                Some(window),
                Some(HMENU(CONNECTION_HEALTH_LABEL_ID as *mut c_void)),
                Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                None,
            )
        };
        let attempt_label = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!(""),
                WS_CHILD,
                0,
                0,
                0,
                0,
                Some(window),
                Some(HMENU(CONNECTION_HEALTH_ATTEMPT_ID as *mut c_void)),
                Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                None,
            )
        };
        if let Err(error) = main_label.and(attempt_label).map(|_| ()) {
            unsafe {
                let _ = DestroyWindow(window);
            }
            return Err(error);
        }

        self.connection_health_window.set(window);
        self.refresh_connection_health_window(window);
        unsafe {
            let _ = ShowWindow(window, SW_SHOWNA);
            let _ = SetTimer(
                Some(window),
                CONNECTION_HEALTH_OWNER_LAYOUT_TIMER_ID,
                CONNECTION_HEALTH_OWNER_LAYOUT_POLL_MILLISECONDS,
                None,
            );
        }
        Ok(())
    }

    fn refresh_connection_health_window(&self, window: HWND) {
        if self.connection_health_window.get() != window {
            return;
        }
        let Some(owner_layout) = self.current_connection_health_owner_layout() else {
            self.clear_connection_health_window();
            return;
        };
        let (width, height) = connection_health_size(owner_layout.dpi);
        let (x, y) = connection_health_position(
            RECT {
                left: owner_layout.left,
                top: owner_layout.top,
                right: owner_layout.right,
                bottom: owner_layout.bottom,
            },
            width,
            height,
        );
        if let Err(error) = unsafe {
            SetWindowPos(
                window,
                None,
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOZORDER,
            )
        } {
            tracing::debug!(?error, "Unable to position ActiveX connection health window");
            return;
        }
        self.connection_health_owner_layout.set(Some(owner_layout));

        let (label, attempt) = self.connection_health_status.get().text();
        if let Ok(main_label) = unsafe { GetDlgItem(Some(window), CONNECTION_HEALTH_LABEL_ID as i32) } {
            let label = HSTRING::from(label);
            unsafe {
                let _ = SetWindowTextW(main_label, PCWSTR(label.as_ptr()));
                let _ = SetWindowPos(
                    main_label,
                    None,
                    connection_bar_scale(12, owner_layout.dpi),
                    connection_bar_scale(14, owner_layout.dpi),
                    connection_bar_scale(256, owner_layout.dpi),
                    connection_bar_scale(20, owner_layout.dpi),
                    SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOZORDER,
                );
            }
        }
        if let Ok(attempt_label) = unsafe { GetDlgItem(Some(window), CONNECTION_HEALTH_ATTEMPT_ID as i32) } {
            let attempt_text = HSTRING::from(attempt.unwrap_or_default());
            unsafe {
                let _ = SetWindowTextW(attempt_label, PCWSTR(attempt_text.as_ptr()));
                let _ = SetWindowPos(
                    attempt_label,
                    None,
                    connection_bar_scale(12, owner_layout.dpi),
                    connection_bar_scale(40, owner_layout.dpi),
                    connection_bar_scale(256, owner_layout.dpi),
                    connection_bar_scale(20, owner_layout.dpi),
                    SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOZORDER,
                );
                let _ = ShowWindow(attempt_label, if attempt_text.is_empty() { SW_HIDE } else { SW_SHOWNA });
            }
        }
    }

    fn clear_connection_health_window(&self) {
        clear_connection_health_status(&self.connection_health_status);
        self.connection_health_owner_layout.set(None);
        let window = self.connection_health_window.replace(HWND(ptr::null_mut()));
        if window.0.is_null() || !unsafe { IsWindow(Some(window)) }.as_bool() {
            return;
        }
        unsafe {
            let _ = KillTimer(Some(window), CONNECTION_HEALTH_OWNER_LAYOUT_TIMER_ID);
        }
        if let Err(error) = destroy_control_window(window) {
            defer_window_resource_release(window);
            tracing::debug!(?error, "Unable to destroy ActiveX connection health window");
        }
    }

    fn set_fullscreen(&self, fullscreen: bool) -> Result<()> {
        if self.settings.borrow().fullscreen == fullscreen {
            return Ok(());
        }

        if self.native_mstsc_shell_window().is_some() {
            self.set_native_mstsc_maximized(fullscreen)?;
        }

        self.settings.borrow_mut().fullscreen = fullscreen;
        self.persistence_dirty.set(true);
        if self.native_mstsc_shell_window().is_none() {
            self.fire_event(
                if fullscreen {
                    DISPID_ON_ENTER_FULL_SCREEN_MODE
                } else {
                    DISPID_ON_LEAVE_FULL_SCREEN_MODE
                },
                &[],
            );
        }
        self.update_connection_bar();
        Ok(())
    }

    fn request_fullscreen_toggle(&self) {
        if self.compatibility.borrow().container_handled_fullscreen != 0 && self.native_mstsc_shell_window().is_none() {
            self.fire_event(
                if self.settings.borrow().fullscreen {
                    DISPID_ON_REQUEST_LEAVE_FULL_SCREEN
                } else {
                    DISPID_ON_REQUEST_GO_FULL_SCREEN
                },
                &[],
            );
            return;
        }

        if let Err(error) = self.set_fullscreen(!self.settings.borrow().fullscreen) {
            trace_host_call("Renderer::NativeMstscFullScreenFailed");
            tracing::warn!(?error, "Unable to change the native mstsc full-screen state");
        }
    }

    fn native_mstsc_shell_window(&self) -> Option<HWND> {
        let renderer = self.activex_window.get();
        if renderer.0.is_null() || !unsafe { IsWindow(Some(renderer)) }.as_bool() {
            return None;
        }

        let window = unsafe { GetAncestor(renderer, GA_ROOTOWNER) };
        if window.0.is_null() || !unsafe { IsWindow(Some(window)) }.as_bool() {
            return None;
        }

        let mut class_name = [0u16; 32];
        let length = unsafe { GetClassNameW(window, &mut class_name) };
        let length = usize::try_from(length).ok()?;
        (class_name[..length] == TSC_SHELL_CONTAINER_CLASS[..]).then_some(window)
    }

    fn set_native_mstsc_maximized(&self, fullscreen: bool) -> Result<()> {
        let Some(window) = self.native_mstsc_shell_window() else {
            return Ok(());
        };
        if !native_mstsc_shell_integration_enabled() {
            trace_host_call("Renderer::NativeMstscFullScreenUnsupported");
            return Err(Error::from_hresult(E_NOTIMPL));
        }

        // mstsc routes its title-bar Maximize action through put_FullScreen. A real native full
        // screen session needs host-owned connection-bar and restoration behavior, so preserve
        // standard window chrome while providing the corresponding maximized host presentation.
        unsafe {
            let _ = ShowWindow(window, if fullscreen { SW_MAXIMIZE } else { SW_RESTORE });
        }
        trace_host_call(if fullscreen {
            "Renderer::NativeMstscShellMaximized"
        } else {
            "Renderer::NativeMstscShellRestored"
        });
        Ok(())
    }

    fn ensure_dispatcher(&self) -> Result<HWND> {
        let hwnd = self.dispatcher.get();
        if !hwnd.0.is_null() {
            return Ok(hwnd);
        }
        let callback_owner = self.callback_owner.get();
        if callback_owner.is_null() {
            return Err(Error::from_hresult(E_UNEXPECTED));
        }

        acquire_dispatcher_class()?;
        let module = match com::retain_module_reference() {
            Ok(module) => module,
            Err(error) => {
                release_dispatcher_class();
                return Err(error);
            }
        };
        let instance = match unsafe { GetModuleHandleW(None) } {
            Ok(instance) => instance,
            Err(error) => {
                com::release_module_reference(module);
                release_dispatcher_class();
                return Err(error);
            }
        };
        let callback_context = Rc::new(ControlWindowContext {
            control: callback_owner,
            module,
            closing: AtomicBool::new(false),
            window_reference_released: Cell::new(false),
            orphaned: AtomicBool::new(false),
        });
        let callback_context_raw = Rc::into_raw(Rc::clone(&callback_context));
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("IronRDP.ActiveX.EventDispatcher"),
                w!(""),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                Some(callback_context_raw.cast()),
            )
        };
        let hwnd = match hwnd {
            Ok(hwnd) => {
                drop(callback_context);
                hwnd
            }
            Err(error) => {
                if !callback_context.window_reference_released.get() {
                    unsafe {
                        drop(Rc::from_raw(callback_context_raw));
                    }
                }
                drop(callback_context);
                release_dispatcher_class();
                return Err(error);
            }
        };

        self.dispatcher.set(hwnd);
        Ok(hwnd)
    }

    fn prompt_for_credentials(&self) -> Result<bool> {
        trace_host_call("ActiveXCredentialPrompt::Prompt");
        if self.state.get() != ConnectionState::Disconnected {
            trace_host_call("ActiveXCredentialPrompt::AlreadyConnected");
            return Ok(false);
        }

        let (server, configured_username) = {
            let settings = self.settings.borrow();
            let server = if settings.server.trim().is_empty() {
                self.native_mstsc_server_from_host_ui().unwrap_or_else(|| {
                    let server = std::env::var("RDP_HOSTNAME").unwrap_or_default();
                    if !server.trim().is_empty() {
                        trace_host_call("NativeMstscCredentialBridge::ServerFromEnvironment");
                    }
                    server
                })
            } else {
                settings.server.clone()
            };
            (server, settings.username.clone())
        };
        if server.trim().is_empty() {
            trace_host_call("NativeMstscCredentialBridge::MissingServer");
            return Ok(false);
        }
        if native_mstsc_autologon_enabled() {
            let Some((username, password)) =
                autologon_credentials(std::env::var("RDP_USERNAME").ok(), std::env::var("RDP_PASSWORD").ok())
            else {
                trace_host_call("NativeMstscCredentialBridge::AutoLogonMissingCredentials");
                return Ok(false);
            };
            {
                let mut settings = self.settings.borrow_mut();
                settings.server = server;
                settings.domain.clear();
                settings.username = username;
                settings.password = Some(password);
            }
            self.compatibility.borrow_mut().autologon = Some(true);
            trace_host_call("NativeMstscCredentialBridge::AutoLogon");
            self.start_connection()?;
            return Ok(self.state.get() != ConnectionState::Disconnected);
        }
        let target = HSTRING::from(format!("IronRDP:{server}"));
        let message = HSTRING::from(format!("Enter credentials for {server}"));
        let caption = HSTRING::from("IronRDP Remote Desktop Connection");
        let parent = if self.credential_parent.get().0.is_null() {
            self.activex_window.get()
        } else {
            self.credential_parent.get()
        };
        // Environment values only populate the local CredUI buffers. The user must still confirm
        // the prompt, and CredUI's non-persistent flag prevents it from writing credential state.
        let initial_username = std::env::var("RDP_USERNAME").unwrap_or(configured_username);
        let initial_password = std::env::var("RDP_PASSWORD").unwrap_or_default();
        let mut username = credential_prompt_buffer(&initial_username, CREDUI_MAX_USERNAME_LENGTH);
        let mut password = credential_prompt_buffer(&initial_password, CREDUI_MAX_PASSWORD_LENGTH);
        let mut save = windows_core::BOOL(0);
        let prompt = CREDUI_INFOW {
            cbSize: size_of::<CREDUI_INFOW>() as u32,
            hwndParent: parent,
            pszMessageText: PCWSTR(message.as_ptr()),
            pszCaptionText: PCWSTR(caption.as_ptr()),
            hbmBanner: Default::default(),
        };
        let result = unsafe {
            CredUIPromptForCredentialsW(
                Some(&prompt),
                PCWSTR(target.as_ptr()),
                None,
                0,
                &mut username,
                &mut password,
                Some(&mut save),
                CREDUI_FLAGS_GENERIC_CREDENTIALS | CREDUI_FLAGS_ALWAYS_SHOW_UI | CREDUI_FLAGS_DO_NOT_PERSIST,
            )
        };

        if result == ERROR_CANCELLED {
            trace_host_call("NativeMstscCredentialBridge::Cancelled");
            username.fill(0);
            password.fill(0);
            return Ok(false);
        }
        if result.0 != 0 {
            trace_host_call("NativeMstscCredentialBridge::PromptFailed");
            username.fill(0);
            password.fill(0);
            return Err(Error::new(
                E_FAIL,
                format!("credential prompt failed with Win32 error {}", result.0),
            ));
        }

        let prompted_username = String::from_utf16_lossy(
            &username[..username
                .iter()
                .position(|character| *character == 0)
                .unwrap_or(username.len())],
        );
        let prompted_password = String::from_utf16_lossy(
            &password[..password
                .iter()
                .position(|character| *character == 0)
                .unwrap_or(password.len())],
        );
        username.fill(0);
        password.fill(0);

        if prompted_password.is_empty() {
            trace_host_call("NativeMstscCredentialBridge::EmptyPassword");
            return Ok(false);
        }
        trace_host_call(if prompted_username.contains('\\') {
            "NativeMstscCredentialBridge::QualifiedUsername"
        } else {
            "NativeMstscCredentialBridge::UnqualifiedUsername"
        });

        {
            let mut settings = self.settings.borrow_mut();
            settings.server = server;
            // Preserve CredUI's account syntax exactly. This follows the direct Automation path
            // and lets the CredSSP username parser handle `DOMAIN\user` and UPN forms.
            settings.domain.clear();
            settings.username = prompted_username;
            settings.password = Some(prompted_password);
        }
        trace_host_call("ActiveXCredentialPrompt::CredentialsAccepted");
        self.start_connection()?;
        Ok(self.state.get() != ConnectionState::Disconnected)
    }

    fn native_mstsc_server_from_host_ui(&self) -> Option<String> {
        let activex_window = self.activex_window.get();
        if activex_window.0.is_null() {
            trace_host_call("NativeMstscCredentialBridge::NoActiveXWindow");
            return None;
        }

        let host_root = unsafe { GetAncestor(activex_window, GA_ROOT) };
        let foreground = unsafe { GetForegroundWindow() };
        let direct_candidates = [
            activex_window,
            host_root,
            self.credential_parent.get(),
            unsafe { GetAncestor(self.credential_parent.get(), GA_ROOT) },
            foreground,
        ];
        if let Some(server) = direct_candidates
            .into_iter()
            .filter(|window| !window.0.is_null())
            .filter_map(|window| unsafe { GetDlgItem(Some(window), MSTSC_COMPUTER_FIELD_ID).ok() })
            .filter(|field| !field.0.is_null())
            .find_map(Self::native_mstsc_server_from_computer_field)
        {
            return Some(server);
        }

        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(activex_window, Some(&mut process_id)) };
        if process_id == 0 {
            trace_host_call("NativeMstscCredentialBridge::NoComputerField");
            return None;
        }
        let mut search = MstscComputerFieldSearch {
            process_id,
            field: HWND(ptr::null_mut()),
        };
        unsafe {
            let _ = EnumWindows(
                Some(find_mstsc_computer_field),
                LPARAM((&mut search as *mut MstscComputerFieldSearch) as isize),
            );
        }
        if search.field.0.is_null() {
            trace_host_call("NativeMstscCredentialBridge::NoComputerField");
            return None;
        }
        let server = Self::native_mstsc_server_from_computer_field(search.field);
        if server.is_none() {
            trace_host_call("NativeMstscCredentialBridge::EmptyComputerField");
        }
        server
    }

    fn native_mstsc_server_from_computer_field(computer_field: HWND) -> Option<String> {
        let mut text = [0u16; 1024];
        let length = unsafe { GetWindowTextW(computer_field, &mut text) };
        if length <= 0 {
            return None;
        }

        String::from_utf16(&text[..length as usize])
            .ok()
            .filter(|server| !server.trim().is_empty())
    }

    fn events_are_frozen(&self) -> bool {
        self.event_freeze_count.get() != 0
    }

    fn set_events_frozen(&self, freeze: bool) {
        let count = self.event_freeze_count.get();
        self.event_freeze_count.set(if freeze {
            count.saturating_add(1)
        } else {
            count.saturating_sub(1)
        });
    }

    fn fire_event(&self, dispid: i32, args: &[i32]) {
        if self.events_are_frozen() {
            return;
        }

        let mut variants = args.iter().rev().map(|value| variant_i32(*value)).collect::<Vec<_>>();
        let params = DISPPARAMS {
            rgvarg: variants.as_mut_ptr(),
            rgdispidNamedArgs: ptr::null_mut(),
            cArgs: variants.len() as u32,
            cNamedArgs: 0,
        };
        let iid_null = GUID::zeroed();

        let sinks = self
            .sinks
            .borrow()
            .values()
            .map(|sink| sink.dispatch.clone())
            .collect::<Vec<_>>();

        for sink in sinks {
            let result = unsafe { sink.Invoke(dispid, &iid_null, 0, DISPATCH_METHOD, &params, None, None, None) };
            if let Err(error) = result {
                tracing::debug!(?error, event_dispid = dispid, "ActiveX event sink rejected event");
            }
        }
    }

    fn fire_auto_reconnecting_event(&self, disconnect_reason: i32, attempt: i32) -> i32 {
        if self.events_are_frozen() {
            return 0;
        }

        let mut continuation = 0;
        let mut variants = [
            variant_i32_byref(&mut continuation),
            variant_i32(attempt),
            variant_i32(disconnect_reason),
        ];
        let params = DISPPARAMS {
            rgvarg: variants.as_mut_ptr(),
            rgdispidNamedArgs: ptr::null_mut(),
            cArgs: variants.len() as u32,
            cNamedArgs: 0,
        };
        let iid_null = GUID::zeroed();
        let sinks = self
            .sinks
            .borrow()
            .values()
            .map(|sink| sink.dispatch.clone())
            .collect::<Vec<_>>();

        for sink in sinks {
            let result = unsafe {
                sink.Invoke(
                    DISPID_ON_AUTO_RECONNECTING,
                    &iid_null,
                    0,
                    DISPATCH_METHOD,
                    &params,
                    None,
                    None,
                    None,
                )
            };
            if let Err(error) = result {
                tracing::debug!(?error, "ActiveX event sink rejected automatic reconnect notification");
            }
        }

        continuation
    }

    fn fire_auto_reconnecting2_event(
        &self,
        disconnect_reason: i32,
        network_available: bool,
        attempt: i32,
        maximum_attempts: i32,
    ) {
        if self.events_are_frozen() {
            return;
        }

        let mut variants = [
            variant_i32(maximum_attempts),
            variant_i32(attempt),
            variant_bool_value(network_available),
            variant_i32(disconnect_reason),
        ];
        let params = DISPPARAMS {
            rgvarg: variants.as_mut_ptr(),
            rgdispidNamedArgs: ptr::null_mut(),
            cArgs: variants.len() as u32,
            cNamedArgs: 0,
        };
        let iid_null = GUID::zeroed();
        let sinks = self
            .sinks
            .borrow()
            .values()
            .map(|sink| sink.dispatch.clone())
            .collect::<Vec<_>>();

        for sink in sinks {
            let result = unsafe {
                sink.Invoke(
                    DISPID_ON_AUTO_RECONNECTING2,
                    &iid_null,
                    0,
                    DISPATCH_METHOD,
                    &params,
                    None,
                    None,
                    None,
                )
            };
            if let Err(error) = result {
                tracing::debug!(?error, "ActiveX event sink rejected automatic reconnect notification");
            }
        }
    }

    fn certificate_warning_parent(&self) -> HWND {
        let renderer = self.activex_window.get();
        if !renderer.0.is_null() && unsafe { IsWindow(Some(renderer)) }.as_bool() {
            renderer
        } else {
            self.credential_parent.get()
        }
    }

    fn prompt_for_certificate_exception(
        &self,
        endpoint: &str,
        fingerprint: &[u8; 32],
        validation_reason: &str,
    ) -> CertificateDecision {
        self.fire_event(DISPID_ON_AUTHENTICATION_WARNING_DISPLAYED, &[]);

        let fingerprint = certificate_fingerprint_text(fingerprint);
        let validation_reason = validation_reason.chars().take(512).collect::<String>();
        let title = HSTRING::from("IronRDP certificate warning");
        let instruction = HSTRING::from("The identity of the remote computer cannot be verified.");
        let content = HSTRING::from(format!(
            "The certificate presented by {endpoint} failed validation.\r\n\r\n\
             SHA-256 fingerprint:\r\n{fingerprint}\r\n\r\n\
             Continue only if you trust this server."
        ));
        let expanded_information = HSTRING::from(format!("Validation detail: {validation_reason}"));
        let continue_text = HSTRING::from("Continue");
        let remember_text = HSTRING::from("Remember this certificate for this server");
        let buttons = [TASKDIALOG_BUTTON {
            nButtonID: CERTIFICATE_WARNING_CONTINUE_BUTTON,
            pszButtonText: PCWSTR(continue_text.as_ptr()),
        }];
        let mut button = 0;
        let mut remember = WinBool(0);
        let dialog = TASKDIALOGCONFIG {
            cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
            hwndParent: self.certificate_warning_parent(),
            hInstance: Default::default(),
            dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_SIZE_TO_CONTENT,
            dwCommonButtons: TDCBF_CANCEL_BUTTON,
            pszWindowTitle: PCWSTR(title.as_ptr()),
            Anonymous1: Default::default(),
            pszMainInstruction: PCWSTR(instruction.as_ptr()),
            pszContent: PCWSTR(content.as_ptr()),
            cButtons: buttons.len() as u32,
            pButtons: buttons.as_ptr(),
            nDefaultButton: 0,
            cRadioButtons: 0,
            pRadioButtons: ptr::null(),
            nDefaultRadioButton: 0,
            pszVerificationText: PCWSTR(remember_text.as_ptr()),
            pszExpandedInformation: PCWSTR(expanded_information.as_ptr()),
            pszExpandedControlText: PCWSTR::null(),
            pszCollapsedControlText: PCWSTR::null(),
            Anonymous2: Default::default(),
            pszFooter: PCWSTR::null(),
            pfCallback: None,
            lpCallbackData: 0,
            cxWidth: 0,
        };
        let decision = match task_dialog_indirect(&dialog, &mut button, &mut remember) {
            Some(Ok(())) if button == CERTIFICATE_WARNING_CONTINUE_BUTTON => CertificateDecision::Accept {
                remember: remember.as_bool(),
            },
            Some(Ok(())) => CertificateDecision::Reject,
            Some(Err(error)) => {
                tracing::warn!(?error, "Unable to display ActiveX certificate warning");
                CertificateDecision::Reject
            }
            None => {
                tracing::warn!("The host does not provide the TaskDialogIndirect certificate warning UI");
                CertificateDecision::Reject
            }
        };

        self.fire_event(DISPID_ON_AUTHENTICATION_WARNING_DISMISSED, &[]);
        decision
    }

    fn confirm_connection_security_warning(&self, warning: ConnectionSecurityWarning) -> Result<bool> {
        let (title, instruction, content) = warning.text();
        let title = HSTRING::from(title);
        let instruction = HSTRING::from(instruction);
        let content = HSTRING::from(content);
        let continue_text = HSTRING::from("Continue");
        let buttons = [TASKDIALOG_BUTTON {
            nButtonID: SECURITY_WARNING_CONTINUE_BUTTON,
            pszButtonText: PCWSTR(continue_text.as_ptr()),
        }];
        let mut button = 0;
        let mut ignored_verification = WinBool(0);
        let dialog = TASKDIALOGCONFIG {
            cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
            hwndParent: self.certificate_warning_parent(),
            hInstance: Default::default(),
            dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_SIZE_TO_CONTENT,
            dwCommonButtons: TDCBF_CANCEL_BUTTON,
            pszWindowTitle: PCWSTR(title.as_ptr()),
            Anonymous1: Default::default(),
            pszMainInstruction: PCWSTR(instruction.as_ptr()),
            pszContent: PCWSTR(content.as_ptr()),
            cButtons: buttons.len() as u32,
            pButtons: buttons.as_ptr(),
            nDefaultButton: 0,
            cRadioButtons: 0,
            pRadioButtons: ptr::null(),
            nDefaultRadioButton: 0,
            pszVerificationText: PCWSTR::null(),
            pszExpandedInformation: PCWSTR::null(),
            pszExpandedControlText: PCWSTR::null(),
            pszCollapsedControlText: PCWSTR::null(),
            Anonymous2: Default::default(),
            pszFooter: PCWSTR::null(),
            pfCallback: None,
            lpCallbackData: 0,
            cxWidth: 0,
        };
        match task_dialog_indirect(&dialog, &mut button, &mut ignored_verification) {
            Some(Ok(())) => Ok(button == SECURITY_WARNING_CONTINUE_BUTTON),
            Some(Err(error)) => Err(Error::new(E_FAIL, format!("security warning dialog failed: {error}"))),
            None => Err(Error::new(E_FAIL, "security warning dialog is unavailable")),
        }
    }

    fn confirm_connection_security_warnings(
        &self,
        warn_about_credentials: bool,
        warn_about_clipboard: bool,
    ) -> Result<bool> {
        for warning in connection_security_warnings(warn_about_credentials, warn_about_clipboard) {
            if !self.confirm_connection_security_warning(warning)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn show_connection_information_dialog(&self) {
        let Some(details) = connection_information_content(
            self.state.get(),
            self.remote_size.get(),
            self.clipboard_state.connected.get() && self.clipboard_state.enabled_for_session.get(),
        ) else {
            return;
        };
        self.show_information_dialog("IronRDP connection information", "Remote desktop session", &details);
    }

    fn show_connection_failure_dialog(&self) {
        self.show_information_dialog(
            "IronRDP connection error",
            "The remote desktop connection could not be established.",
            "No additional connection details are available.",
        );
    }

    fn confirm_connection_bar_disconnect(&self) -> Result<bool> {
        if self.state.get() != ConnectionState::Connected {
            return Ok(false);
        }

        let (title, instruction, content) = connection_bar_disconnect_prompt();
        let title = HSTRING::from(title);
        let instruction = HSTRING::from(instruction);
        let content = HSTRING::from(content);
        let disconnect_text = HSTRING::from("Disconnect");
        let buttons = [TASKDIALOG_BUTTON {
            nButtonID: CONNECTION_BAR_DISCONNECT_BUTTON,
            pszButtonText: PCWSTR(disconnect_text.as_ptr()),
        }];
        let mut button = 0;
        let mut ignored_verification = WinBool(0);
        let dialog = TASKDIALOGCONFIG {
            cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
            hwndParent: self.certificate_warning_parent(),
            hInstance: Default::default(),
            dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_SIZE_TO_CONTENT,
            dwCommonButtons: TDCBF_CANCEL_BUTTON,
            pszWindowTitle: PCWSTR(title.as_ptr()),
            Anonymous1: Default::default(),
            pszMainInstruction: PCWSTR(instruction.as_ptr()),
            pszContent: PCWSTR(content.as_ptr()),
            cButtons: buttons.len() as u32,
            pButtons: buttons.as_ptr(),
            nDefaultButton: 0,
            cRadioButtons: 0,
            pRadioButtons: ptr::null(),
            nDefaultRadioButton: 0,
            pszVerificationText: PCWSTR::null(),
            pszExpandedInformation: PCWSTR::null(),
            pszExpandedControlText: PCWSTR::null(),
            pszCollapsedControlText: PCWSTR::null(),
            Anonymous2: Default::default(),
            pszFooter: PCWSTR::null(),
            pfCallback: None,
            lpCallbackData: 0,
            cxWidth: 0,
        };
        match task_dialog_indirect(&dialog, &mut button, &mut ignored_verification) {
            Some(Ok(())) => Ok(button == CONNECTION_BAR_DISCONNECT_BUTTON),
            Some(Err(error)) => Err(Error::new(
                E_FAIL,
                format!("disconnect confirmation dialog failed: {error}"),
            )),
            None => Err(Error::new(E_FAIL, "disconnect confirmation dialog is unavailable")),
        }
    }

    fn show_information_dialog(&self, title: &str, instruction: &str, content: &str) {
        let title = HSTRING::from(title);
        let instruction = HSTRING::from(instruction);
        let content = HSTRING::from(content);
        let close_text = HSTRING::from("Close");
        let buttons = [TASKDIALOG_BUTTON {
            nButtonID: INFORMATION_DIALOG_CLOSE_BUTTON,
            pszButtonText: PCWSTR(close_text.as_ptr()),
        }];
        let mut button = 0;
        let mut ignored_verification = WinBool(0);
        let dialog = TASKDIALOGCONFIG {
            cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
            hwndParent: self.certificate_warning_parent(),
            hInstance: Default::default(),
            dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_SIZE_TO_CONTENT,
            dwCommonButtons: Default::default(),
            pszWindowTitle: PCWSTR(title.as_ptr()),
            Anonymous1: Default::default(),
            pszMainInstruction: PCWSTR(instruction.as_ptr()),
            pszContent: PCWSTR(content.as_ptr()),
            cButtons: buttons.len() as u32,
            pButtons: buttons.as_ptr(),
            nDefaultButton: INFORMATION_DIALOG_CLOSE_BUTTON,
            cRadioButtons: 0,
            pRadioButtons: ptr::null(),
            nDefaultRadioButton: 0,
            pszVerificationText: PCWSTR::null(),
            pszExpandedInformation: PCWSTR::null(),
            pszExpandedControlText: PCWSTR::null(),
            pszCollapsedControlText: PCWSTR::null(),
            Anonymous2: Default::default(),
            pszFooter: PCWSTR::null(),
            pfCallback: None,
            lpCallbackData: 0,
            cxWidth: 0,
        };
        match task_dialog_indirect(&dialog, &mut button, &mut ignored_verification) {
            Some(Ok(())) => {}
            Some(Err(error)) => tracing::warn!(?error, "Unable to display ActiveX connection information"),
            None => tracing::warn!("The host does not provide the TaskDialogIndirect information UI"),
        }
    }

    fn ole_advise_sinks(&self) -> Vec<IAdviseSink> {
        self.ole_advise_sinks.borrow().values().cloned().collect()
    }

    fn notify_ole_advise_view_change(&self) {
        for sink in self.ole_advise_sinks() {
            unsafe {
                sink.OnViewChange(DVASPECT_CONTENT.0, -1);
            }
        }
        if let Some(sink) = self.view_advise.borrow().as_ref().map(|advise| advise.sink.clone()) {
            unsafe {
                sink.OnViewChange(DVASPECT_CONTENT.0, -1);
            }
        }
    }

    fn notify_ole_advise_save(&self) {
        for sink in self.ole_advise_sinks() {
            unsafe {
                sink.OnSave();
            }
        }
    }

    fn notify_ole_advise_close(&self) {
        for sink in self.ole_advise_sinks() {
            unsafe {
                sink.OnClose();
            }
        }
    }

    fn disconnect_description(&self, disconnect_reason: u32, extended_reason: u32) -> &'static str {
        let disconnect = self.last_disconnect.get();
        if disconnect_reason == disconnect.event_reason as u32 && extended_reason == disconnect.extended_reason as u32 {
            disconnect.description
        } else {
            DisconnectInfo::no_info().description
        }
    }

    fn fire_channel_received_data(&self, channel_name: &str, data: &[u8]) {
        if self.events_are_frozen() {
            return;
        }

        // Automation arguments are supplied right-to-left: data, then channel name.
        let mut variants = [
            variant_bstr(channel_data_to_automation_string(data)),
            variant_bstr(channel_name.to_owned()),
        ];
        let params = DISPPARAMS {
            rgvarg: variants.as_mut_ptr(),
            rgdispidNamedArgs: ptr::null_mut(),
            cArgs: variants.len() as u32,
            cNamedArgs: 0,
        };
        let iid_null = GUID::zeroed();
        let sinks = self
            .sinks
            .borrow()
            .values()
            .map(|sink| sink.dispatch.clone())
            .collect::<Vec<_>>();

        for sink in sinks {
            let result = unsafe {
                sink.Invoke(
                    DISPID_ON_CHANNEL_RECEIVED_DATA,
                    &iid_null,
                    0,
                    DISPATCH_METHOD,
                    &params,
                    None,
                    None,
                    None,
                )
            };
            if let Err(error) = result {
                tracing::debug!(?error, "ActiveX event sink rejected static channel data");
            }
        }

        for value in &mut variants {
            free_owned_bstr_variant(value);
        }
    }

    fn request_close_status(&self) -> i32 {
        if self.events_are_frozen() {
            return CONTROL_CLOSE_CAN_PROCEED;
        }

        let mut allow_close = VARIANT_TRUE;
        let mut argument = variant_bool_byref(&mut allow_close);
        let params = DISPPARAMS {
            rgvarg: &mut argument,
            rgdispidNamedArgs: ptr::null_mut(),
            cArgs: 1,
            cNamedArgs: 0,
        };
        let iid_null = GUID::zeroed();
        let sinks = self
            .sinks
            .borrow()
            .values()
            .map(|sink| sink.dispatch.clone())
            .collect::<Vec<_>>();

        for sink in sinks {
            let result = unsafe {
                sink.Invoke(
                    DISPID_ON_CONFIRM_CLOSE,
                    &iid_null,
                    0,
                    DISPATCH_METHOD,
                    &params,
                    None,
                    None,
                    None,
                )
            };
            if let Err(error) = result {
                tracing::debug!(?error, "ActiveX event sink rejected close confirmation");
            }
            if allow_close == VARIANT_FALSE {
                return CONTROL_CLOSE_WAIT_FOR_EVENTS;
            }
        }

        CONTROL_CLOSE_CAN_PROCEED
    }

    fn dispatch_pending_events(&self) {
        self.event_posted.store(false, Ordering::Release);
        let events = self.events.take();

        for event in events {
            if event.generation() != self.connection_generation.get() {
                event.reject_certificate_warning();
                continue;
            }

            match event {
                WorkerEvent::CertificateWarning {
                    endpoint,
                    fingerprint,
                    validation_reason,
                    public_mode,
                    response,
                    ..
                } => {
                    let decision = self.prompt_for_certificate_exception(&endpoint, &fingerprint, &validation_reason);
                    if let CertificateDecision::Accept { remember: true } = decision
                        && !public_mode
                        && let Err(error) = persist_certificate_exception(&endpoint, &fingerprint)
                    {
                        tracing::warn!(?error, "Unable to persist ActiveX certificate exception");
                    }
                    let _ = response.send(decision);
                }
                WorkerEvent::Connected { .. } => {
                    if self.state.get() == ConnectionState::Connecting {
                        self.state.set(ConnectionState::Connected);
                        if let Some(rpc) = &self.rpc {
                            rpc.session_connected();
                        }
                        self.clipboard_state.connected.set(true);
                        self.fire_event(DISPID_ON_CONNECTED, &[]);
                        self.clear_connection_health_window();
                        self.update_connection_bar();
                        let renderer_window = self.activex_window.get();
                        if !renderer_window.0.is_null() && unsafe { IsWindow(Some(renderer_window)) }.as_bool() {
                            self.synchronize_native_mstsc_renderer_layout(renderer_window);
                        }
                        if self.compatibility.borrow().grab_focus_on_connect
                            && !renderer_window.0.is_null()
                            && unsafe { IsWindow(Some(renderer_window)) }.as_bool()
                            && let Err(error) = unsafe { SetFocus(Some(renderer_window)) }
                        {
                            tracing::debug!(?error, "Unable to focus ActiveX renderer after connecting");
                        }
                    } else if self.state.get() == ConnectionState::Connected {
                        // A Display Control fallback reconnect completed. It is not an automatic
                        // reconnect contract and must not raise additional lifecycle events.
                        self.clear_connection_health_window();
                    }
                }
                WorkerEvent::MonitorLayout { monitors, .. } => {
                    let topology = self.configured_monitor_topology.borrow().clone();
                    if topology.as_ref().is_some_and(|topology| topology.monitors == monitors) {
                        *self.active_monitor_topology.borrow_mut() = topology;
                    } else {
                        self.active_monitor_topology.borrow_mut().take();
                    }
                }
                WorkerEvent::LoginComplete { .. } => {
                    if self.state.get() == ConnectionState::Connected && !self.login_complete_fired.replace(true) {
                        self.fire_event(DISPID_ON_LOGIN_COMPLETE, &[]);
                    }
                }
                WorkerEvent::Image {
                    buffer, width, height, ..
                } => {
                    let width = i32::from(width);
                    let height = i32::from(height);
                    if self.state.get() == ConnectionState::Connected
                        && self.remote_size.replace(Some((width, height))) != Some((width, height))
                    {
                        self.fire_event(DISPID_ON_REMOTE_DESKTOP_SIZE_CHANGE, &[width, height]);
                    }
                    if let (Some(rpc), Ok(width), Ok(height)) = (&self.rpc, u16::try_from(width), u16::try_from(height))
                    {
                        rpc.retain_frame(width, height, &buffer);
                    }
                    self.present_frame(buffer, width, height);
                }
                WorkerEvent::DisplayResizeFallback { .. } => self.report_display_resize_fallback(),
                WorkerEvent::RailWindowingOrders { data, .. } => {
                    if self.rail_windows.borrow().is_enabled() {
                        self.rail_windows.borrow_mut().consume(&data);
                    }
                }
                WorkerEvent::AutoReconnecting {
                    disconnect_reason,
                    attempt,
                    maximum_attempts,
                    response,
                    ..
                } => {
                    self.report_reconnect_worker_progress(attempt, maximum_attempts);
                    let disconnect_reason = i32::try_from(disconnect_reason).unwrap_or(i32::MAX);
                    let attempt = i32::try_from(attempt).unwrap_or(i32::MAX);
                    let maximum_attempts = i32::try_from(maximum_attempts).unwrap_or(i32::MAX);
                    let decision = match self.fire_auto_reconnecting_event(disconnect_reason, attempt) {
                        0 => {
                            self.fire_auto_reconnecting2_event(disconnect_reason, false, attempt, maximum_attempts);
                            AutoReconnectDecision::Continue
                        }
                        _ => AutoReconnectDecision::Stop,
                    };
                    let _ = response.send(decision);
                }
                WorkerEvent::AutoReconnected { .. } => {
                    if self.state.get() == ConnectionState::Connected {
                        self.clear_connection_health_window();
                        self.fire_event(DISPID_ON_AUTO_RECONNECTED, &[]);
                    }
                }
                WorkerEvent::FatalError { disconnect, .. } => {
                    if let Some(rpc) = &self.rpc {
                        rpc.session_failed(disconnect.description.to_owned());
                    }
                    self.state.set(ConnectionState::Stopping);
                    self.clear_connection_health_window();
                    if let Err(error) = self.destroy_connection_bar() {
                        tracing::debug!(
                            ?error,
                            "Unable to destroy ActiveX connection bar after a connection failure"
                        );
                    }
                    self.last_disconnect.set(disconnect);
                    self.clipboard_state.connected.set(false);
                    self.remote_size.set(None);
                    self.active_monitor_topology.borrow_mut().take();
                    self.clear_frame();
                    self.rail_windows.borrow_mut().stop();
                    self.fire_event(DISPID_ON_FATAL_ERROR, &[disconnect.event_reason]);
                    self.fire_event(DISPID_ON_DISCONNECTED, &[disconnect.event_reason]);
                    self.show_connection_failure_dialog();
                }
                WorkerEvent::Disconnected { disconnect, .. } => {
                    if let Some(rpc) = &self.rpc {
                        rpc.session_disconnected(disconnect.description.to_owned());
                    }
                    self.state.set(ConnectionState::Stopping);
                    self.clear_connection_health_window();
                    if let Err(error) = self.destroy_connection_bar() {
                        tracing::debug!(?error, "Unable to destroy ActiveX connection bar after disconnecting");
                    }
                    self.last_disconnect.set(disconnect);
                    self.clipboard_state.connected.set(false);
                    self.remote_size.set(None);
                    self.active_monitor_topology.borrow_mut().take();
                    self.clear_frame();
                    self.rail_windows.borrow_mut().stop();
                    self.fire_event(DISPID_ON_DISCONNECTED, &[disconnect.event_reason]);
                }
                WorkerEvent::StaticChannelData { channel_name, data, .. } => {
                    self.fire_channel_received_data(&channel_name, &data)
                }
                WorkerEvent::Stopped { .. } => {
                    self.clear_connection_health_window();
                    if let Err(error) = self.destroy_connection_bar() {
                        tracing::debug!(
                            ?error,
                            "Unable to destroy ActiveX connection bar after the connection stopped"
                        );
                    }
                    self.release_input();
                    self.stop_clipboard_redirection();
                    self.input_sender.borrow_mut().take();
                    self.remote_size.set(None);
                    self.active_monitor_topology.borrow_mut().take();
                    self.configured_monitor_topology.borrow_mut().take();
                    self.clear_frame();
                    self.rail_windows.borrow_mut().stop();
                    self.state.set(ConnectionState::Disconnected);
                    self.native_mstsc_preflight.set(NativeMstscPreflight::Idle);
                    self.compatibility.borrow_mut().connection_settings_sealed = false;
                    if let Some(rpc) = &self.rpc {
                        rpc.session_stopped();
                    }
                }
            }
        }
    }

    fn dispatch_rpc_commands(&self) {
        let Some(rpc) = &self.rpc else {
            return;
        };
        rpc.drain_commands(|command| self.handle_rpc_command(command));
    }

    fn handle_rpc_command(&self, command: RpcCommand) {
        match command {
            RpcCommand::Connect {
                properties,
                log_directive,
                response,
            } => {
                let _ = response.send(self.rpc_connect(properties, log_directive));
            }
            RpcCommand::Disconnect { response } => {
                let response_value = if self.state.get() == ConnectionState::Disconnected {
                    ironrdp_agent::ipc::Response::typed_error(
                        ironrdp_agent::ipc::AgentErrorCategory::Unavailable,
                        "no active RDP session",
                    )
                } else {
                    self.stop_connection()
                        .map_or_else(rpc_control_error, |_| ironrdp_agent::ipc::Response::ok())
                };
                let _ = response.send(response_value);
            }
            RpcCommand::Input { operation, response } => {
                let _ = response.send(self.rpc_input(operation));
            }
            RpcCommand::Touch {
                encode_time,
                frames,
                response,
            } => {
                let _ = response.send(self.rpc_touch(encode_time, frames));
            }
            RpcCommand::Pen {
                encode_time,
                frames,
                response,
            } => {
                let _ = response.send(self.rpc_pen(encode_time, frames));
            }
            RpcCommand::DismissHoveringTouchContact { contact_id, response } => {
                let _ = response.send(self.rpc_dismiss_hovering_touch_contact(contact_id));
            }
            RpcCommand::Resize {
                width,
                height,
                response,
            } => {
                let response_value = if width == 0 || height == 0 {
                    ironrdp_agent::ipc::Response::typed_error(
                        ironrdp_agent::ipc::AgentErrorCategory::InvalidRequest,
                        "width and height must be non-zero",
                    )
                } else {
                    self.update_display_layout(DisplayLayout {
                        desktop_width: u32::from(width),
                        desktop_height: u32::from(height),
                        physical_width: 0,
                        physical_height: 0,
                        orientation: 0,
                        desktop_scale_factor: 100,
                        device_scale_factor: 100,
                    })
                    .map_or_else(rpc_control_error, |_| ironrdp_agent::ipc::Response::ok())
                };
                let _ = response.send(response_value);
            }
        }
    }

    fn rpc_connect(&self, properties: PropertySet, log_directive: Option<String>) -> ironrdp_agent::ipc::Response {
        if self.state.get() != ConnectionState::Disconnected {
            return ironrdp_agent::ipc::Response::typed_error(
                ironrdp_agent::ipc::AgentErrorCategory::Conflict,
                "a session is already active; disconnect first",
            );
        }
        let mut client_properties = match rdcleanpath_rpc_client_properties(&properties) {
            Ok(properties) => properties,
            Err(message) => {
                return ironrdp_agent::ipc::Response::typed_error(
                    ironrdp_agent::ipc::AgentErrorCategory::InvalidRequest,
                    message,
                );
            }
        };
        if let Some((url, token)) = client_properties
            .get::<&str>("ironrdp_rdcleanpathurl")
            .zip(client_properties.get::<&str>("ironrdp_rdcleanpathtoken"))
        {
            if let Err(error) = self.replace_rdcleanpath_settings(url.to_owned(), token.to_owned()) {
                return rpc_control_error(error);
            }
        } else if let Err(error) = self.apply_rdcleanpath_settings_to_client_properties(&mut client_properties) {
            return rpc_control_error(error);
        }
        if ["ironrdp_dvcpipeproxy", "ironrdp_dvcplugin"]
            .into_iter()
            .any(|key| properties.get::<&str>(key).is_some())
            || [
                "ironrdp_qoi",
                "ironrdp_qoiz",
                "ironrdp_rdpdr",
                "ironrdp_smartcard",
                "ironrdp_serverpointer",
            ]
            .into_iter()
            .any(|key| properties.get::<i64>(key).is_some_and(|value| value != 0))
        {
            return ironrdp_agent::ipc::Response::typed_error(
                ironrdp_agent::ipc::AgentErrorCategory::InvalidRequest,
                "the requested transport extension is not supported by the ActiveX host",
            );
        };

        let certificate_validation = match properties.get::<&str>("ironrdp_certificate_validation") {
            None | Some("strict") => CertificateValidation::Strict,
            Some("dangerously_accept_invalid_certificate") => {
                CertificateValidation::DangerouslyAcceptInvalidCertificate
            }
            Some(_) => {
                return ironrdp_agent::ipc::Response::typed_error(
                    ironrdp_agent::ipc::AgentErrorCategory::InvalidRequest,
                    "invalid certificate validation policy",
                );
            }
        };
        let builder = match ConfigBuilder::from_property_set(&client_properties) {
            Ok(builder) => builder,
            Err(error) => {
                return ironrdp_agent::ipc::Response::typed_error(
                    ironrdp_agent::ipc::AgentErrorCategory::InvalidRequest,
                    format!("invalid configuration: {error:#}"),
                );
            }
        }
        .with_client_build(self.compatibility.borrow().client_build)
        .with_client_dir(self.compatibility.borrow().client_dir.clone())
        .with_client_name(
            self.compatibility
                .borrow()
                .client_name
                .clone()
                .unwrap_or_else(|| "IronRDP ActiveX".to_owned()),
        )
        .with_platform(MajorPlatformType::WINDOWS)
        .with_certificate_validation(certificate_validation)
        .with_pointer_software_rendering(true);
        let missing = builder.missing();
        if !missing.is_empty() {
            return ironrdp_agent::ipc::Response::typed_error(
                ironrdp_agent::ipc::AgentErrorCategory::InvalidRequest,
                format!(
                    "missing required fields: {}",
                    missing
                        .iter()
                        .map(ironrdp_client::config::MissingField::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
        let config = match builder.build() {
            Ok(config) => config,
            Err(error) => {
                return ironrdp_agent::ipc::Response::typed_error(
                    ironrdp_agent::ipc::AgentErrorCategory::InvalidRequest,
                    format!("{error:#}"),
                );
            }
        };
        if let Transport::RDCleanPath(rdcleanpath) = config.transport()
            && !matches!(rdcleanpath.url.scheme(), "ws" | "wss")
        {
            return ironrdp_agent::ipc::Response::typed_error(
                ironrdp_agent::ipc::AgentErrorCategory::InvalidRequest,
                "RDCleanPath URL must use the ws or wss scheme",
            );
        }
        let rpc_transport = match active_x_transport_from_client_transport(config.transport()) {
            Ok(transport) => transport,
            Err(message) => {
                return ironrdp_agent::ipc::Response::typed_error(
                    ironrdp_agent::ipc::AgentErrorCategory::InvalidRequest,
                    message,
                );
            }
        };
        let Credentials::UsernamePassword { username, password } = &config.connector().credentials else {
            return ironrdp_agent::ipc::Response::typed_error(
                ironrdp_agent::ipc::AgentErrorCategory::InvalidRequest,
                "smart card credentials are not supported by the ActiveX host",
            );
        };
        {
            let mut settings = self.settings.borrow_mut();
            settings.server = config.destination().name().to_owned();
            settings.username = username.clone();
            settings.password = Some(password.clone());
            settings.domain = config.connector().domain.clone().unwrap_or_default();
            settings.desktop_width = config.connector().desktop_size.width;
            settings.desktop_height = config.connector().desktop_size.height;
            settings.color_depth = config
                .connector()
                .bitmap
                .as_ref()
                .map(|bitmap| bitmap.color_depth)
                .unwrap_or(32);
        }
        {
            let connector = config.connector();
            let mut compatibility = self.compatibility.borrow_mut();
            compatibility.enable_credssp = Some(connector.enable_credssp);
            compatibility.enable_tls = Some(connector.enable_tls);
            compatibility.compression = Some(connector.compression_type.is_some());
            compatibility.compression_level = connector.compression_type.map(|compression| match compression {
                ironrdp_pdu::rdp::client_info::CompressionType::K8 => 0,
                ironrdp_pdu::rdp::client_info::CompressionType::K64 => 1,
                ironrdp_pdu::rdp::client_info::CompressionType::Rdp6 => 2,
                ironrdp_pdu::rdp::client_info::CompressionType::Rdp61 => 3,
            });
            compatibility.redirect_clipboard = matches!(config.channels().clipboard, ClipboardType::Enable);
            compatibility.redirect_webauthn = config.channels().webauthn;
            compatibility.performance_flags = connector.performance_flags;
            compatibility.keyboard_type = connector.keyboard_type;
            compatibility.keyboard_subtype = connector.keyboard_subtype;
            compatibility.keyboard_functional_keys_count = connector.keyboard_functional_keys_count;
            compatibility.keyboard_layout = connector.keyboard_layout;
            compatibility.network_connection_type = connector.connection_type;
            compatibility.desktop_scale_factor = Some(connector.desktop_scale_factor);
            compatibility.client_build = connector.client_build;
            compatibility.client_dir = connector.client_dir.clone();
            compatibility.client_name = Some(connector.client_name.clone());
            compatibility.ime_file_name = connector.ime_file_name.clone();
            compatibility.digital_product_id = connector.dig_product_id.clone();
            compatibility.autologon = Some(connector.autologon);
            compatibility.rdp_port = Some(config.destination().port());
            compatibility.fake_events_interval_minutes = config
                .fake_events_interval()
                .map(|interval| u32::try_from(interval.as_secs() / 60).unwrap_or(u32::MAX));
            compatibility.audio_redirection_mode = if connector.enable_audio_playback { 0 } else { 2 };
            compatibility.audio_capture_redirection_mode = if connector.enable_audio_capture {
                VARIANT_TRUE.0
            } else {
                VARIANT_FALSE.0
            };
            compatibility.secured_start_program = connector.alternate_shell.clone();
            compatibility.secured_work_dir = connector.work_dir.clone();
            compatibility.authentication_level_set =
                certificate_validation == CertificateValidation::DangerouslyAcceptInvalidCertificate;
            compatibility.authentication_level = if compatibility.authentication_level_set { 0 } else { 1 };
            match config.transport() {
                Transport::Direct => {
                    compatibility.gateway_usage_method = GatewayUsageMethod::Direct.as_i64() as u32;
                }
                Transport::Gateway(gateway) => {
                    compatibility.gateway_hostname = gateway.endpoint.clone();
                    compatibility.gateway_username = gateway.username.clone();
                    compatibility.gateway_password = gateway.password.clone();
                    compatibility.gateway_domain.clear();
                    compatibility.gateway_usage_method = GatewayUsageMethod::UseAlways.as_i64() as u32;
                    compatibility.gateway_creds_source = GatewayCredentialsSource::UseUserCredentials.as_i64() as u32;
                }
                Transport::RDCleanPath(_) => {
                    compatibility.gateway_hostname.clear();
                    compatibility.gateway_username.clear();
                    compatibility.gateway_password.clear();
                    compatibility.gateway_domain.clear();
                    compatibility.gateway_usage_method = GatewayUsageMethod::Direct.as_i64() as u32;
                    compatibility.gateway_creds_source = GatewayCredentialsSource::UseServerCredentials.as_i64() as u32;
                }
                // Rejected by `active_x_transport_from_client_transport` before settings apply.
                Transport::NamedPipe { .. } => {
                    unreachable!("NamedPipe must fail RPC connect before compatibility settings")
                }
            }
        }

        *self.rpc_properties.borrow_mut() = Some(properties);
        *self.rpc_transport.borrow_mut() = Some(rpc_transport);
        *self.rpc_kerberos_config.borrow_mut() = config.kerberos_config().cloned();
        *self.rpc_log_directive.borrow_mut() = log_directive;
        match self.start_connection() {
            Ok(()) if self.state.get() == ConnectionState::Disconnected => {
                self.rpc_properties.borrow_mut().take();
                self.rpc_transport.borrow_mut().take();
                self.rpc_kerberos_config.borrow_mut().take();
                let _ = self.rpc_log_directive.borrow_mut().take();
                ironrdp_agent::ipc::Response::typed_error(
                    ironrdp_agent::ipc::AgentErrorCategory::Unavailable,
                    "connection cancelled",
                )
            }
            Ok(()) => ironrdp_agent::ipc::Response::ok(),
            Err(error) => {
                self.rpc_properties.borrow_mut().take();
                self.rpc_transport.borrow_mut().take();
                self.rpc_kerberos_config.borrow_mut().take();
                let _ = self.rpc_log_directive.borrow_mut().take();
                rpc_control_error(error)
            }
        }
    }

    fn rpc_touch(
        &self,
        encode_time: u32,
        frames: Vec<ironrdp_agent::ipc::TouchFrameRequest>,
    ) -> ironrdp_agent::ipc::Response {
        if self.state.get() != ConnectionState::Connected {
            return ironrdp_agent::ipc::Response::typed_error(
                ironrdp_agent::ipc::AgentErrorCategory::Unavailable,
                "no active RDP session",
            );
        }
        let Some(sender) = self.input_sender.borrow().as_ref().cloned() else {
            return ironrdp_agent::ipc::Response::typed_error(
                ironrdp_agent::ipc::AgentErrorCategory::Unavailable,
                "session input channel is unavailable",
            );
        };
        let event = match build_rpc_touch_event(encode_time, frames) {
            Ok(event) => event,
            Err(response) => return response,
        };
        match sender.try_reserve() {
            Ok(permit) => {
                permit.send(RdpInputEvent::Touch(event));
                ironrdp_agent::ipc::Response::ok()
            }
            Err(_) => ironrdp_agent::ipc::Response::typed_error(
                ironrdp_agent::ipc::AgentErrorCategory::Unavailable,
                "session input channel is unavailable",
            ),
        }
    }

    fn rpc_pen(
        &self,
        encode_time: u32,
        frames: Vec<ironrdp_agent::ipc::PenFrameRequest>,
    ) -> ironrdp_agent::ipc::Response {
        if self.state.get() != ConnectionState::Connected {
            return ironrdp_agent::ipc::Response::typed_error(
                ironrdp_agent::ipc::AgentErrorCategory::Unavailable,
                "no active RDP session",
            );
        }
        let Some(sender) = self.input_sender.borrow().as_ref().cloned() else {
            return ironrdp_agent::ipc::Response::typed_error(
                ironrdp_agent::ipc::AgentErrorCategory::Unavailable,
                "session input channel is unavailable",
            );
        };
        let event = match build_rpc_pen_event(encode_time, frames) {
            Ok(event) => event,
            Err(response) => return response,
        };
        match sender.try_reserve() {
            Ok(permit) => {
                permit.send(RdpInputEvent::Pen(event));
                ironrdp_agent::ipc::Response::ok()
            }
            Err(_) => ironrdp_agent::ipc::Response::typed_error(
                ironrdp_agent::ipc::AgentErrorCategory::Unavailable,
                "session input channel is unavailable",
            ),
        }
    }

    fn rpc_dismiss_hovering_touch_contact(&self, contact_id: u8) -> ironrdp_agent::ipc::Response {
        if self.state.get() != ConnectionState::Connected {
            return ironrdp_agent::ipc::Response::typed_error(
                ironrdp_agent::ipc::AgentErrorCategory::Unavailable,
                "no active RDP session",
            );
        }
        let Some(sender) = self.input_sender.borrow().as_ref().cloned() else {
            return ironrdp_agent::ipc::Response::typed_error(
                ironrdp_agent::ipc::AgentErrorCategory::Unavailable,
                "session input channel is unavailable",
            );
        };
        match sender.try_reserve() {
            Ok(permit) => {
                permit.send(RdpInputEvent::DismissHoveringTouchContact { contact_id });
                ironrdp_agent::ipc::Response::ok()
            }
            Err(_) => ironrdp_agent::ipc::Response::typed_error(
                ironrdp_agent::ipc::AgentErrorCategory::Unavailable,
                "session input channel is unavailable",
            ),
        }
    }
    fn rpc_input(&self, operation: Operation) -> ironrdp_agent::ipc::Response {
        if self.state.get() != ConnectionState::Connected {
            return ironrdp_agent::ipc::Response::typed_error(
                ironrdp_agent::ipc::AgentErrorCategory::Unavailable,
                "no active RDP session",
            );
        }
        let Some(sender) = self.input_sender.borrow().as_ref().cloned() else {
            return ironrdp_agent::ipc::Response::typed_error(
                ironrdp_agent::ipc::AgentErrorCategory::Unavailable,
                "session input channel is unavailable",
            );
        };
        let permit = match sender.try_reserve() {
            Ok(permit) => permit,
            Err(_) => {
                return ironrdp_agent::ipc::Response::typed_error(
                    ironrdp_agent::ipc::AgentErrorCategory::Unavailable,
                    "session input channel is unavailable",
                );
            }
        };
        let fast_path = self.input_database.borrow_mut().apply([operation]);
        if fast_path.is_empty() {
            return ironrdp_agent::ipc::Response::ok();
        }
        permit.send(RdpInputEvent::FastPath(fast_path));
        ironrdp_agent::ipc::Response::ok()
    }

    fn start_connection(&self) -> Result<()> {
        if self.state.get() != ConnectionState::Disconnected {
            return Err(Error::from_hresult(E_FAIL));
        }
        self.last_disconnect.set(DisconnectInfo::no_info());

        let settings = self.settings.borrow();
        if settings.server.trim().is_empty() {
            // mstsc creates the ActiveX control before the user supplies credentials. Keeping the
            // control idle lets its native connection form remain available until it has enough
            // information to start an IronRDP session.
            return Ok(());
        }
        let password = settings
            .password
            .clone()
            .or_else(|| self.compatibility.borrow().clear_text_password.clone());
        if password.is_none() {
            let prompt_for_credentials = self.compatibility.borrow().prompt_for_credentials;
            let should_prompt = should_prompt_for_credentials(&settings.server, false, prompt_for_credentials);
            drop(settings);
            if should_prompt {
                let _ = self.prompt_for_credentials()?;
            }
            return Ok(());
        }
        drop(settings);
        let hwnd = self.ensure_dispatcher()?;
        let settings = self.settings.borrow();
        let destination = Destination::new(settings.server.clone())
            .map_err(|error| Error::new(E_INVALIDARG, format!("invalid RDP destination: {error}")))?;
        let compatibility = self.compatibility.borrow();
        let destination = compatibility
            .rdp_port
            .map(|port| Destination::from_parts(destination.name().to_owned(), port))
            .unwrap_or(destination);
        let password = password.ok_or_else(|| Error::new(E_INVALIDARG, "set IronRdpPassword before connecting"))?;

        let enable_credssp = compatibility.enable_credssp;
        let compression = compatibility.compression;
        let clipboard = compatibility.redirect_clipboard;
        let redirect_webauthn = compatibility.redirect_webauthn;
        let warn_about_credentials = compatibility.warn_about_sending_credentials;
        let warn_about_clipboard = clipboard && compatibility.warn_about_clipboard_redirection;
        let redirected_drives = if compatibility.disable_rdpdr {
            Vec::new()
        } else {
            compatibility.drive_catalog.borrow().selected_drives()?
        };
        let redirect_smart_cards = !compatibility.disable_rdpdr && compatibility.redirect_smart_cards;
        let rdpdr_factory = if redirected_drives.is_empty() && !redirect_smart_cards {
            None
        } else {
            Some(
                ironrdp_rdpdr_native::WindowsRdpdrBackendFactory::from_drives(redirected_drives)
                    .map_err(|error| Error::new(E_FAIL, format!("invalid redirected-drive configuration: {error}")))?
                    .with_smartcard(redirect_smart_cards),
            )
        };
        let rdpdr_enabled = rdpdr_factory.is_some();
        let audio_redirection_mode = audio_mode_from_raw(compatibility.audio_redirection_mode)?;
        let audio_capture_enabled = compatibility.audio_capture_redirection_mode != VARIANT_FALSE.0;
        let keyboard_type = compatibility.keyboard_type;
        let keyboard_subtype = compatibility.keyboard_subtype;
        let keyboard_functional_keys_count = compatibility.keyboard_functional_keys_count;
        let alternate_shell = compatibility.secured_start_program.clone();
        let work_dir = compatibility.secured_work_dir.clone();
        let transport = match self.rpc_transport.borrow_mut().take() {
            Some(transport) => transport,
            None => match self.rdcleanpath_transport()? {
                Some(transport) => transport,
                None => active_x_transport(&settings, &compatibility)?,
            },
        };
        let performance_flags = compatibility.performance_flags;
        let keyboard_layout = compatibility.keyboard_layout;
        let connection_type = compatibility.network_connection_type;
        let client_name = compatibility
            .client_name
            .clone()
            .unwrap_or_else(|| "IronRDP ActiveX".to_owned());
        let dvc_plugin_paths = if redirect_webauthn {
            let mut filtered = Vec::new();
            for path in &compatibility.dvc_plugin_paths {
                let is_webauthn_plugin = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("webauthn.dll"));
                if is_webauthn_plugin {
                    tracing::warn!(
                        dll = %path.display(),
                        "Skipping webauthn.dll COM DVC plugin because native RedirectWebAuthn is enabled"
                    );
                } else {
                    filtered.push(path.clone());
                }
            }
            filtered
        } else {
            compatibility.dvc_plugin_paths.clone()
        };
        let enable_tls = compatibility.enable_tls;
        let autologon = compatibility.autologon;
        let desktop_scale_factor = compatibility.desktop_scale_factor;
        let compression_level = compatibility.compression_level;
        let client_build = compatibility.client_build;
        let client_dir = compatibility.client_dir.clone();
        let ime_file_name = compatibility.ime_file_name.clone();
        let digital_product_id = compatibility.digital_product_id.clone();
        let fake_events_interval_minutes = compatibility.fake_events_interval_minutes;
        let authentication_level = compatibility.authentication_level;
        let authentication_level_set = compatibility.authentication_level_set;
        let public_mode = compatibility.public_mode;
        let use_multimon = compatibility.use_multimon;
        let auto_reconnect_maximum_attempts = compatibility
            .enable_auto_reconnect
            .then_some(compatibility.max_reconnect_attempts);
        let direct_rpc_properties = active_x_property_snapshot(&settings, &compatibility);
        drop(compatibility);
        let monitor_topology = use_multimon.then(local_monitor_topology).transpose()?;
        let (desktop_width, desktop_height) = monitor_topology
            .as_ref()
            .map(|topology| (topology.desktop_width, topology.desktop_height))
            .unwrap_or((settings.desktop_width, settings.desktop_height));
        let remote_application = self.remote_application.borrow();
        let remote_program_mode = remote_application.enabled;
        let remote_application_execute = configured_remote_application_execute(&remote_application)?;
        drop(remote_application);
        if let Some(execute) = &remote_application_execute {
            validate_rail_execute(execute)?;
        }
        if !self.confirm_connection_security_warnings(warn_about_credentials, warn_about_clipboard)? {
            return Ok(());
        }
        let certificate_validation =
            certificate_validation_from_authentication_level(authentication_level, authentication_level_set);
        let certificate_prompt_enabled = certificate_prompt_enabled(
            certificate_validation,
            authentication_level,
            authentication_level_set,
            native_mstsc_credential_bridge_enabled(),
        );
        trace_host_call(match (certificate_validation, certificate_prompt_enabled) {
            (CertificateValidation::Strict, true) => "RdpWorker::TlsCertificateValidation:PromptOnFailure",
            (CertificateValidation::Strict, false) => "RdpWorker::TlsCertificateValidation:Strict",
            (CertificateValidation::DangerouslyAcceptInvalidCertificate, _) => {
                "RdpWorker::TlsCertificateValidation:DangerouslyAcceptInvalidCertificate"
            }
        });
        trace_host_call(match settings.color_depth {
            16 => "RdpWorker::GraphicsProfile:ColorDepth16:Lossless:RemoteFxDisabled",
            32 => "RdpWorker::GraphicsProfile:ColorDepth32:Lossless:RemoteFxDisabled",
            _ => "RdpWorker::GraphicsProfile:InvalidColorDepth",
        });
        let generation = self.connection_generation.get().wrapping_add(1);
        let certificate_validation_callback = if certificate_prompt_enabled {
            let certificate_events = Arc::clone(&self.events);
            let certificate_event_posted = Arc::clone(&self.event_posted);
            let certificate_dispatcher = hwnd.0 as isize;
            let endpoint = destination.to_string();
            let callback: ironrdp_tls::CertificateValidationCallback =
                Arc::new(move |certificate_der, validation_reason| {
                    let fingerprint = certificate_fingerprint(certificate_der);
                    if !public_mode && certificate_exception_is_trusted(&endpoint, &fingerprint) {
                        trace_host_call("RdpWorker::TlsCertificateValidation:TrustedException");
                        return true;
                    }

                    let (response, receiver) = std_mpsc::sync_channel(1);
                    if !queue_worker_event(
                        &certificate_events,
                        &certificate_event_posted,
                        HWND(certificate_dispatcher as *mut c_void),
                        WorkerEvent::CertificateWarning {
                            generation,
                            endpoint: endpoint.clone(),
                            fingerprint,
                            validation_reason: validation_reason.to_owned(),
                            public_mode,
                            response,
                        },
                    ) {
                        trace_host_call("RdpWorker::TlsCertificateValidation:PromptQueueFailed");
                        return false;
                    }

                    match receiver.recv_timeout(CERTIFICATE_WARNING_TIMEOUT) {
                        Ok(CertificateDecision::Accept { .. }) => true,
                        Ok(CertificateDecision::Reject) => false,
                        Err(_) => {
                            trace_host_call("RdpWorker::TlsCertificateValidation:PromptTimedOut");
                            false
                        }
                    }
                });
            Some(callback)
        } else {
            None
        };
        let static_channels = self.static_channels.borrow().values().cloned().collect::<Vec<_>>();
        let channel_events = Arc::clone(&self.events);
        let channel_event_posted = Arc::clone(&self.event_posted);
        let static_channel_dispatcher = hwnd.0 as isize;
        let rpc_now_endpoint = self
            .rpc
            .as_ref()
            .map(|_| ActiveXRpc::allocate_now_endpoint())
            .transpose()
            .map_err(|response| match response {
                ironrdp_agent::ipc::Response::Err(error) => Error::new(E_FAIL, error.message),
                ironrdp_agent::ipc::Response::Ok(_) => Error::from_hresult(E_FAIL),
            })?;
        let builder = ConfigBuilder::new()
            .with_destination(destination)
            .with_username(settings.username.clone())
            .with_domain(settings.domain.clone())
            .with_password(password)
            .with_desktop_width(desktop_width)
            .with_desktop_height(desktop_height)
            .with_color_depth(settings.color_depth)
            // Keep ActiveX bitmap drawing lossless and avoid RemoteFX until its live display-update
            // stream transitions are fully validated.
            .with_lossy_compression(ACTIVEX_LOSSY_COMPRESSION)
            .with_codecs(
                ACTIVEX_CODEC_CONFIGURATION
                    .iter()
                    .map(|option| (*option).to_owned())
                    .collect(),
            )
            .with_client_build(client_build)
            .with_client_dir(client_dir)
            .with_client_name(client_name)
            .with_platform(MajorPlatformType::WINDOWS)
            .with_keyboard_type(keyboard_type)
            .with_keyboard_subtype(keyboard_subtype)
            .with_keyboard_functional_keys_count(keyboard_functional_keys_count)
            .with_ime_file_name(ime_file_name)
            .with_dig_product_id(digital_product_id)
            .with_alternate_shell(alternate_shell)
            .with_work_dir(work_dir)
            .with_performance_flags(performance_flags)
            .with_keyboard_layout(keyboard_layout)
            .with_connection_type(connection_type)
            .with_audio_mode(audio_redirection_mode)
            .with_audio_capture(audio_capture_enabled)
            .with_certificate_validation(certificate_validation)
            // The GDI presenter has no hardware-cursor overlay, so cursor updates must be
            // composited into the decoded framebuffer before it receives image events.
            .with_pointer_software_rendering(true)
            .with_clipboard(if clipboard {
                ClipboardType::Enable
            } else {
                ClipboardType::Disable
            })
            .with_webauthn(redirect_webauthn)
            .with_webauthn_parent_hwnd(hwnd.0 as isize)
            .with_rdpdr(rdpdr_enabled)
            .with_smartcard(redirect_smart_cards);
        let builder = if let Some(topology) = &monitor_topology {
            builder.with_monitor_layout(topology.client_monitor_data())
        } else {
            builder
        };
        let builder = if remote_program_mode {
            builder
                .with_remote_application_mode(true)
                .with_rail_support_level(RailSupportLevel::SUPPORTED)
                .with_rail_client_status_flags(0)
        } else {
            builder
        };
        let builder = if let Some(kerberos_config) = self.rpc_kerberos_config.borrow_mut().take() {
            builder.with_kerberos_config(kerberos_config)
        } else {
            builder
        };
        let builder = if let Some(callback) = certificate_validation_callback {
            builder.with_certificate_validation_callback(callback)
        } else {
            builder
        };
        let builder = if let Some(compression) = compression {
            builder.with_compression(compression)
        } else {
            builder
        };
        let builder = if let Some(enable_credssp) = enable_credssp {
            builder.with_credssp(enable_credssp)
        } else {
            builder
        };
        let builder = if let Some(enable_tls) = enable_tls {
            builder.with_tls(enable_tls)
        } else {
            builder
        };
        let builder = if let Some(autologon) = autologon {
            builder.with_autologon(autologon)
        } else {
            builder
        };
        let builder = if let Some(desktop_scale_factor) = desktop_scale_factor {
            builder.with_desktop_scale_factor(desktop_scale_factor)
        } else {
            builder
        };
        let builder = if let Some(compression_level) = compression_level {
            builder.with_compression(true).with_compression_level(compression_level)
        } else {
            builder
        };
        let builder = if let Some(minutes) = fake_events_interval_minutes {
            builder.with_fake_events_interval(Duration::from_secs(u64::from(minutes) * 60))
        } else {
            builder
        };
        let using_rdcleanpath = matches!(&transport, ActiveXTransport::RDCleanPath(_));
        let builder = match transport {
            ActiveXTransport::Direct => builder,
            ActiveXTransport::Gateway {
                endpoint,
                username,
                password,
            } => builder
                .with_transport(TransportKind::Gateway { endpoint })
                .with_gateway_username(username)
                .with_gateway_password(password),
            ActiveXTransport::RDCleanPath(rdcleanpath) => builder
                .with_transport(TransportKind::RDCleanPath { url: rdcleanpath.url })
                .with_rdcleanpath_token(rdcleanpath.auth_token),
        };
        let builder = dvc_plugin_paths
            .into_iter()
            .fold(builder, |builder, path| builder.with_dvc_plugin(path));
        let builder = builder.with_static_channel_instances(move |_| {
            static_channels
                .iter()
                .cloned()
                .map(|spec| ActiveXStaticChannel {
                    spec,
                    events: Arc::clone(&channel_events),
                    event_posted: Arc::clone(&channel_event_posted),
                    dispatcher: static_channel_dispatcher,
                    generation,
                })
                .collect()
        });
        let builder = if let Some(now_endpoint) = &rpc_now_endpoint {
            builder.with_dvc_pipe_proxy(now_endpoint.dvc_proxy_info())
        } else {
            builder
        };
        let config = builder
            .build()
            .map_err(|error| Error::new(E_INVALIDARG, format!("invalid RDP configuration: {error}")))?;
        *self.configured_monitor_topology.borrow_mut() = monitor_topology;
        self.active_monitor_topology.borrow_mut().take();
        if using_rdcleanpath {
            self.clear_rdcleanpath_token();
        }
        let rpc_destination = config.destination().to_string();
        drop(settings);
        let (output_sender, mut output_receiver) = mpsc::channel(32);
        let client = RdpClient::new(config, output_sender);
        let client = if let Some(maximum_attempts) = auto_reconnect_maximum_attempts {
            client.with_auto_reconnect(maximum_attempts)
        } else {
            client
        };
        let input_sender = client.input_sender();
        if let Some(execute) = remote_application_execute {
            input_sender
                .try_send_rail_execute(execute)
                .map_err(|error| Error::new(E_FAIL, error.to_string()))?;
        }
        let client = if clipboard {
            let factory = self.start_clipboard_redirection(input_sender.clone())?;
            client.with_cliprdr_backend_factory(factory)
        } else {
            self.stop_clipboard_redirection();
            client
        };
        let client = if let Some(factory) = rdpdr_factory {
            client.with_rdpdr_backend_factory(Box::new(factory))
        } else {
            client
        };
        self.compatibility.borrow_mut().connection_settings_sealed = true;
        self.clipboard_state.enabled_for_session.set(clipboard);
        self.clipboard_state.connected.set(false);

        let events = Arc::clone(&self.events);
        let event_posted = Arc::clone(&self.event_posted);
        let hwnd_raw = hwnd.0 as isize;
        let rpc_log_directive = self.rpc_log_directive.borrow().clone();
        let rpc_dispatch = self
            .rpc
            .as_ref()
            .map(|rpc| rpc.session_dispatch(rpc_log_directive.as_deref()));
        let module = match com::retain_module_for_worker() {
            Ok(module) => module,
            Err(error) => {
                self.stop_clipboard_redirection();
                self.compatibility.borrow_mut().connection_settings_sealed = false;
                self.configured_monitor_topology.borrow_mut().take();
                return Err(error);
            }
        };
        let module_raw = module.0 as isize;

        let rpc_properties = self.rpc_properties.borrow_mut().take().unwrap_or(direct_rpc_properties);
        self.rpc_log_directive.borrow_mut().take();
        if let (Some(rpc), Some(now_endpoint)) = (&self.rpc, rpc_now_endpoint.as_ref()) {
            rpc.session_started(rpc_destination, rpc_properties, Arc::clone(now_endpoint));
        }

        com::add_worker();
        let spawn = std::thread::Builder::new()
            .name("ironrdp-activex-rdp".to_owned())
            .spawn(move || {
                let module = HMODULE(module_raw as *mut c_void);
                {
                    let hwnd = HWND(hwnd_raw as *mut c_void);
                    let worker_events = Arc::clone(&events);
                    let worker_event_posted = Arc::clone(&event_posted);
                    let worker = catch_unwind(AssertUnwindSafe(|| {
                        // Each control owns this worker thread, so a current-thread runtime keeps
                        // its connection tasks and teardown on that one module-pinned thread.
                        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
                        match runtime {
                            Ok(runtime) => {
                                let local = tokio::task::LocalSet::new();
                                let worker = local.run_until(async move {
                                    let client_task = tokio::task::spawn_local(client.run());
                                    let mut connection_failed = false;
                                    let mut terminal_received = false;
                                    while let Some(output) = output_receiver.recv().await {
                                        match output {
                                            RdpOutputEvent::Image { buffer, width, height } => {
                                                let _ = queue_worker_event(
                                                    &worker_events,
                                                    &worker_event_posted,
                                                    hwnd,
                                                    WorkerEvent::Image {
                                                        generation,
                                                        buffer,
                                                        width: width.get(),
                                                        height: height.get(),
                                                    },
                                                );
                                            }
                                            RdpOutputEvent::WindowingOrders(data) => {
                                                if !queue_worker_event(
                                                    &worker_events,
                                                    &worker_event_posted,
                                                    hwnd,
                                                    WorkerEvent::RailWindowingOrders { generation, data },
                                                ) {
                                                    break;
                                                }
                                            }
                                            RdpOutputEvent::Connected => {
                                                queue_worker_event(
                                                    &worker_events,
                                                    &worker_event_posted,
                                                    hwnd,
                                                    WorkerEvent::Connected { generation },
                                                );
                                            }
                                            RdpOutputEvent::MonitorLayout(monitors) => {
                                                queue_worker_event(
                                                    &worker_events,
                                                    &worker_event_posted,
                                                    hwnd,
                                                    WorkerEvent::MonitorLayout { generation, monitors },
                                                );
                                            }
                                            RdpOutputEvent::LoginComplete => {
                                                queue_worker_event(
                                                    &worker_events,
                                                    &worker_event_posted,
                                                    hwnd,
                                                    WorkerEvent::LoginComplete { generation },
                                                );
                                            }
                                            RdpOutputEvent::PostLogonDisplayRedraw => {
                                                trace_host_call("RdpWorker::PostLogonDisplayRedraw");
                                            }
                                            RdpOutputEvent::MalformedBitmapDisplayRedraw => {
                                                trace_host_call("RdpWorker::MalformedBitmapDisplayRedraw");
                                            }
                                            RdpOutputEvent::ConnectionFailure(error) => {
                                                connection_failed = true;
                                                trace_connection_failure(&error);
                                                let disconnect = DisconnectInfo::from_connection_failure(&error);
                                                queue_worker_event(
                                                    &worker_events,
                                                    &worker_event_posted,
                                                    hwnd,
                                                    WorkerEvent::FatalError { generation, disconnect },
                                                );
                                            }
                                            RdpOutputEvent::DisplayResizeFallback(reason) => {
                                                let marker = match reason {
                                                    ironrdp_client::rdp::DisplayResizeFallbackReason::DisplayControlUnavailable => {
                                                        "RdpWorker::DisplayResizeFallback:DisplayControlUnavailable"
                                                    }
                                                    ironrdp_client::rdp::DisplayResizeFallbackReason::CapabilitiesTimedOut => {
                                                        "RdpWorker::DisplayResizeFallback:CapabilitiesTimedOut"
                                                    }
                                                    ironrdp_client::rdp::DisplayResizeFallbackReason::ReactivationTimedOut => {
                                                        "RdpWorker::DisplayResizeFallback:ReactivationTimedOut"
                                                    }
                                                };
                                                trace_host_call(marker);
                                                queue_worker_event(
                                                    &worker_events,
                                                    &worker_event_posted,
                                                    hwnd,
                                                    WorkerEvent::DisplayResizeFallback { generation },
                                                );
                                            }
                                            RdpOutputEvent::AutoReconnecting {
                                                disconnect_reason,
                                                attempt,
                                                maximum_attempts,
                                                response,
                                            } => {
                                                if !queue_worker_event(
                                                    &worker_events,
                                                    &worker_event_posted,
                                                    hwnd,
                                                    WorkerEvent::AutoReconnecting {
                                                        generation,
                                                        disconnect_reason,
                                                        attempt,
                                                        maximum_attempts,
                                                        response,
                                                    },
                                                ) {
                                                    // The host cannot make a decision if its dispatcher is gone.
                                                    // Fail closed rather than starting an unobservable retry.
                                                    // `response` has been moved into the rejected queue request.
                                                }
                                            }
                                            RdpOutputEvent::AutoReconnected => {
                                                queue_worker_event(
                                                    &worker_events,
                                                    &worker_event_posted,
                                                    hwnd,
                                                    WorkerEvent::AutoReconnected { generation },
                                                );
                                            }
                                            RdpOutputEvent::Terminated(result) => {
                                                terminal_received = true;
                                                match &result {
                                                    Ok(_) => trace_host_call("RdpWorker::Terminated:Graceful"),
                                                    Err(error) => trace_session_failure(error),
                                                }
                                                if !connection_failed {
                                                    let disconnect = match result {
                                                        Ok(reason) => DisconnectInfo::from_graceful_disconnect(&reason),
                                                        Err(error) => DisconnectInfo::from_session_failure(&error),
                                                    };
                                                    queue_worker_event(
                                                        &worker_events,
                                                        &worker_event_posted,
                                                        hwnd,
                                                        WorkerEvent::Disconnected { generation, disconnect },
                                                    );
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                    let client_task_outcome = match client_task.await {
                                        Ok(()) => ClientTaskOutcome::Completed,
                                        Err(error) if error.is_cancelled() => ClientTaskOutcome::Cancelled,
                                        Err(error) if error.is_panic() => ClientTaskOutcome::Panicked,
                                        Err(_) => ClientTaskOutcome::Failed,
                                    };
                                    let client_task_failed = client_task_outcome.failed();
                                    if let Some(marker) = client_task_outcome.trace_marker() {
                                        trace_host_call(marker);
                                    }
                                    if let Some(event) = worker_completion_event(
                                        generation,
                                        connection_failed,
                                        terminal_received,
                                        client_task_failed,
                                    ) {
                                        queue_worker_event(&worker_events, &worker_event_posted, hwnd, event);
                                    }
                                    queue_worker_event(
                                        &worker_events,
                                        &worker_event_posted,
                                        hwnd,
                                        WorkerEvent::Stopped { generation },
                                    );
                                });
                                if let Some(dispatch) = rpc_dispatch {
                                    tracing::dispatcher::with_default(&dispatch, || runtime.block_on(worker));
                                } else {
                                    runtime.block_on(worker);
                                }
                            }
                            Err(error) => {
                                tracing::error!(?error, "Unable to create RDP worker runtime");
                                queue_worker_event(
                                    &worker_events,
                                    &worker_event_posted,
                                    hwnd,
                                    WorkerEvent::FatalError {
                                        generation,
                                        disconnect: DisconnectInfo::internal_error(),
                                    },
                                );
                                queue_worker_event(
                                    &worker_events,
                                    &worker_event_posted,
                                    hwnd,
                                    WorkerEvent::Stopped { generation },
                                );
                            }
                        }
                    }));
                    if worker.is_err() {
                        queue_worker_event(
                            &events,
                            &event_posted,
                            hwnd,
                            WorkerEvent::FatalError {
                                generation,
                                disconnect: DisconnectInfo::internal_error(),
                            },
                        );
                        queue_worker_event(&events, &event_posted, hwnd, WorkerEvent::Stopped { generation });
                    }
                }

                com::release_worker();
                unsafe { com::release_module_and_exit_worker(module) };
            });

        if let Err(error) = spawn {
            com::release_worker();
            com::release_module_reference(module);
            self.stop_clipboard_redirection();
            self.compatibility.borrow_mut().connection_settings_sealed = false;
            self.configured_monitor_topology.borrow_mut().take();
            let message = format!("unable to start RDP worker: {error}");
            if let Some(rpc) = &self.rpc {
                rpc.session_failed(message.clone());
                rpc.session_stopped();
            }
            return Err(Error::new(E_FAIL, message));
        }

        *self.input_sender.borrow_mut() = Some(input_sender);
        self.rail_windows.borrow_mut().start(
            remote_program_mode
                .then(|| self.input_sender.borrow().as_ref().cloned())
                .flatten(),
        );
        self.connection_generation.set(generation);
        self.login_complete_fired.set(false);
        self.remote_size.set(None);
        self.active_monitor_topology.borrow_mut().take();
        self.clear_frame();
        self.input_database.borrow_mut().release_all();
        *self.touch_tracker.borrow_mut() = TouchContactTracker::new();
        self.state.set(ConnectionState::Connecting);
        self.fire_event(DISPID_ON_CONNECTING, &[]);
        if self.state.get() == ConnectionState::Connecting {
            self.set_connection_health_status(ConnectionHealthStatus::Connecting);
        }
        Ok(())
    }

    fn stop_connection(&self) -> Result<()> {
        self.release_input();
        let sender = self
            .input_sender
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| Error::from_hresult(E_FAIL))?;
        self.last_disconnect.set(DisconnectInfo::api_initiated());
        sender.request_close();
        self.state.set(ConnectionState::Stopping);
        if let Some(rpc) = &self.rpc {
            rpc.session_disconnecting();
        }
        self.clear_connection_health_window();
        Ok(())
    }

    fn update_display_layout(&self, layout: DisplayLayout) -> Result<()> {
        if self.state.get() != ConnectionState::Connected {
            return Err(Error::from_hresult(E_UNEXPECTED));
        }
        let configured_monitor_count = self
            .configured_monitor_topology
            .borrow()
            .as_ref()
            .map_or(0, |topology| topology.monitors.len());
        if configured_monitor_count > 1 {
            return Err(Error::from_hresult(E_NOTIMPL));
        }
        let desktop_width = u16::try_from(layout.desktop_width)
            .map_err(|_| Error::new(E_INVALIDARG, "desktop width must fit in u16"))?;
        let desktop_height = u16::try_from(layout.desktop_height)
            .map_err(|_| Error::new(E_INVALIDARG, "desktop height must fit in u16"))?;
        if desktop_width == 0 || desktop_height == 0 {
            return Err(Error::new(E_INVALIDARG, "desktop dimensions must be nonzero"));
        }
        if layout.physical_width == 0 && layout.physical_height != 0
            || layout.physical_width != 0 && layout.physical_height == 0
        {
            return Err(Error::new(
                E_INVALIDARG,
                "physical display dimensions must both be specified",
            ));
        }
        if layout.orientation != 0 {
            return Err(Error::from_hresult(E_NOTIMPL));
        }
        // IronRDP's active-session resize path carries desktop scaling, but not the separate
        // device scale and rotation exposed by the mstscax API.
        if layout.device_scale_factor != 0 && layout.device_scale_factor != 100 {
            return Err(Error::from_hresult(E_NOTIMPL));
        }

        let scale_factor = if layout.desktop_scale_factor == 0 {
            100
        } else {
            layout.desktop_scale_factor
        };
        let physical_size = (layout.physical_width != 0).then_some((layout.physical_width, layout.physical_height));
        let sender = self
            .input_sender
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| Error::from_hresult(E_UNEXPECTED))?;
        sender
            .try_send(RdpInputEvent::Resize {
                width: desktop_width,
                height: desktop_height,
                scale_factor,
                physical_size,
            })
            .map_err(|_| Error::from_hresult(E_UNEXPECTED))?;

        if configured_monitor_count == 1 {
            self.active_monitor_topology.borrow_mut().take();
            self.configured_monitor_topology.borrow_mut().take();
        }
        let mut settings = self.settings.borrow_mut();
        settings.desktop_width = desktop_width;
        settings.desktop_height = desktop_height;
        Ok(())
    }

    fn reconnect(&self, width: u32, height: u32, status: *mut i32) -> Result<()> {
        // Initialize the required out parameter before attempting the update so callers can
        // distinguish an invalid or inactive request from one queued for the RDP session.
        write_out(status, CONTROL_RECONNECT_BLOCKED)?;
        if self.state.get() != ConnectionState::Connected {
            return Err(Error::from_hresult(E_FAIL));
        }
        self.update_display_layout(DisplayLayout {
            desktop_width: width,
            desktop_height: height,
            physical_width: 0,
            physical_height: 0,
            orientation: 0,
            desktop_scale_factor: 100,
            device_scale_factor: 100,
        })?;
        write_out(status, CONTROL_RECONNECT_STARTED)
    }

    fn remote_monitor_bounds(&self) -> Result<(i32, i32, i32, i32)> {
        if self.state.get() == ConnectionState::Connected
            && let Some(topology) = self.active_monitor_topology.borrow().as_ref()
        {
            let (left, top, right, bottom) = topology.bounds();
            return Ok((
                left,
                top,
                right.checked_add(1).ok_or_else(|| Error::from_hresult(E_UNEXPECTED))?,
                bottom.checked_add(1).ok_or_else(|| Error::from_hresult(E_UNEXPECTED))?,
            ));
        }
        let Some((width, height)) = self.remote_size.get() else {
            return Err(Error::from_hresult(E_UNEXPECTED));
        };
        Ok((0, 0, width, height))
    }

    fn start_clipboard_redirection(
        &self,
        input_sender: RdpInputSender,
    ) -> Result<Box<dyn CliprdrBackendFactory + Send>> {
        if self.clipboard_backend.borrow().is_some() {
            return Err(Error::from_hresult(E_UNEXPECTED));
        }

        let backend = WinClipboard::new(ActiveXClipboardMessageProxy { input_sender }).map_err(|error| {
            Error::new(
                E_FAIL,
                format!("unable to initialize Windows clipboard redirection: {error}"),
            )
        })?;
        let factory = backend.backend_factory();
        *self.clipboard_backend.borrow_mut() = Some(backend);
        trace_host_call("ActiveXClipboard::Started");
        Ok(factory)
    }

    fn stop_clipboard_redirection(&self) {
        if self.clipboard_backend.borrow_mut().take().is_some() {
            trace_host_call("ActiveXClipboard::Stopped");
        }
        self.clipboard_state.enabled_for_session.set(false);
        self.clipboard_state.connected.set(false);
    }

    fn get_property(&self, dispid: i32, result: *mut VARIANT) -> Result<()> {
        if result.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }

        let settings = self.settings.borrow();
        let value = match dispid {
            DISPID_SERVER => VariantValue::String(settings.server.clone()),
            DISPID_DOMAIN => VariantValue::String(settings.domain.clone()),
            DISPID_USERNAME => VariantValue::String(settings.username.clone()),
            DISPID_DISCONNECTED_TEXT => VariantValue::String(settings.disconnected_text.clone()),
            DISPID_CONNECTING_TEXT => VariantValue::String(settings.connecting_text.clone()),
            DISPID_CONNECTED => VariantValue::Bool(self.state.get() == ConnectionState::Connected),
            DISPID_DESKTOP_WIDTH => VariantValue::Integer(i32::from(settings.desktop_width)),
            DISPID_DESKTOP_HEIGHT => VariantValue::Integer(i32::from(settings.desktop_height)),
            DISPID_START_CONNECTED => VariantValue::Bool(settings.start_connected),
            DISPID_HORIZONTAL_SCROLLBAR_VISIBLE | DISPID_VERTICAL_SCROLLBAR_VISIBLE => VariantValue::Integer(0),
            DISPID_CIPHER_STRENGTH => VariantValue::Integer(128),
            DISPID_VERSION => VariantValue::String("IronRDP ActiveX 0.1".to_owned()),
            DISPID_SECURED_SETTINGS_ENABLED => VariantValue::Integer(i32::from(VARIANT_TRUE.0)),
            DISPID_COLOR_DEPTH => VariantValue::Integer(
                i32::try_from(settings.color_depth).map_err(|_| Error::from_hresult(DISP_E_TYPEMISMATCH))?,
            ),
            DISPID_EXTENDED_DISCONNECT_REASON => VariantValue::Integer(self.last_disconnect.get().extended_reason),
            DISPID_FULLSCREEN => VariantValue::Bool(settings.fullscreen),
            DISPID_CONNECTED_STATUS_TEXT => VariantValue::String(settings.connected_status_text.clone()),
            DISPID_IRONRDP_PASSWORD => return Err(Error::from_hresult(E_FAIL)),
            _ => return Err(Error::from_hresult(DISP_E_MEMBERNOTFOUND)),
        };
        drop(settings);

        // SAFETY: result was checked above and is owned by the caller. Each constructor transfers
        // ownership of any allocated BSTR to the caller as required by Automation.
        unsafe {
            result.write(value.into_variant());
        }
        Ok(())
    }

    fn put_property(&self, dispid: i32, params: &DISPPARAMS, argument_error: *mut u32) -> Result<()> {
        let value = property_put_value(params)?;
        if dispid == DISPID_FULLSCREEN {
            return self.set_fullscreen(variant_bool(value, argument_error)?);
        }

        let mut settings = self.settings.borrow_mut();

        match dispid {
            DISPID_SERVER => settings.server = variant_string(value, argument_error)?,
            DISPID_DOMAIN => settings.domain = variant_string(value, argument_error)?,
            DISPID_USERNAME => settings.username = variant_string(value, argument_error)?,
            DISPID_DESKTOP_WIDTH => settings.desktop_width = variant_dimension(value, argument_error, "desktop width")?,
            DISPID_DESKTOP_HEIGHT => {
                settings.desktop_height = variant_dimension(value, argument_error, "desktop height")?
            }
            DISPID_START_CONNECTED => settings.start_connected = variant_bool(value, argument_error)?,
            DISPID_FULLSCREEN_TITLE => settings.fullscreen_title = variant_string(value, argument_error)?,
            DISPID_COLOR_DEPTH => {
                let depth = variant_i32_value(value, argument_error)?;
                if depth != 16 && depth != 32 {
                    return Err(Error::new(E_INVALIDARG, "color depth must be 16 or 32"));
                }
                settings.color_depth = depth as u32;
            }
            DISPID_CONNECTED_STATUS_TEXT => settings.connected_status_text = variant_string(value, argument_error)?,
            DISPID_IRONRDP_PASSWORD => {
                settings.password = Some(variant_string(value, argument_error)?);
            }
            _ => return Err(Error::from_hresult(DISP_E_MEMBERNOTFOUND)),
        }

        self.persistence_dirty.set(true);
        Ok(())
    }

    fn connection_point(&self, container: IConnectionPointContainer) -> Result<IConnectionPoint> {
        let point: IConnectionPoint =
            ConnectionPoint::new(Rc::clone(&self.sinks), Rc::clone(&self.next_cookie), container).into();
        Ok(point)
    }

    fn destroy_activex_window(&self) -> Result<()> {
        self.release_input();
        self.clear_connection_health_window();
        if let Err(error) = self.destroy_connection_bar() {
            tracing::debug!(?error, "Unable to destroy ActiveX connection bar with its renderer");
        }
        let window = self.activex_window.get();
        if window.0.is_null() {
            return Ok(());
        }

        unsafe {
            let _ = KillTimer(Some(window), NATIVE_MSTSC_LAYOUT_TIMER_ID);
        }
        if unsafe { IsWindow(Some(window)) }.as_bool() {
            if let Err(error) = destroy_control_window(window) {
                // A final COM release must never leave a window procedure in an unloadable DLL.
                // Keep the callback context's module and class references until WM_NCDESTROY.
                defer_window_resource_release(window);
                self.activex_window.set(HWND(ptr::null_mut()));
                self.compatibility.borrow_mut().renderer_window = HWND(ptr::null_mut());
                self.renderer_class_acquired.set(false);
                return Err(error);
            }
        }
        self.activex_window.set(HWND(ptr::null_mut()));
        self.compatibility.borrow_mut().renderer_window = HWND(ptr::null_mut());
        if self.renderer_class_acquired.replace(false) {
            release_renderer_class();
        }
        Ok(())
    }

    fn renderer_destroyed_unexpectedly(&self, window: HWND) -> bool {
        if self.activex_window.get() != window {
            return false;
        }

        self.release_input();
        self.clear_connection_health_window();
        if let Err(error) = self.destroy_connection_bar() {
            tracing::debug!(?error, "Unable to destroy ActiveX connection bar with its renderer");
        }
        self.activex_window.set(HWND(ptr::null_mut()));
        self.compatibility.borrow_mut().renderer_window = HWND(ptr::null_mut());
        self.renderer_class_acquired.replace(false)
    }

    fn destroy_dispatcher_window(&self) -> Result<()> {
        let window = self.dispatcher.get();
        if window.0.is_null() {
            return Ok(());
        }

        let result = if unsafe { IsWindow(Some(window)) }.as_bool() {
            match destroy_control_window(window) {
                Ok(()) => {
                    release_dispatcher_class();
                    Ok(())
                }
                Err(error) => {
                    defer_window_resource_release(window);
                    Err(error)
                }
            }
        } else {
            release_dispatcher_class();
            Ok(())
        };
        self.dispatcher.set(HWND(ptr::null_mut()));
        result
    }

    fn resize_activex_window(&self, rect: RECT) -> Result<()> {
        self.activex_rect.set(rect);
        let window = self.activex_window.get();
        if window.0.is_null() {
            return Ok(());
        }

        let width = (rect.right - rect.left).max(1);
        let height = (rect.bottom - rect.top).max(1);
        unsafe {
            SetWindowPos(
                window,
                None,
                rect.left,
                rect.top,
                width,
                height,
                SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOZORDER,
            )?;
            self.apply_activex_window_clip(window, rect)?;
            let _ = InvalidateRect(Some(window), None, false);
        }
        Ok(())
    }

    fn start_native_mstsc_layout_observer(&self, window: HWND) {
        if !native_shell_presentation_enabled(
            native_mstsc_shell_integration_enabled(),
            self.native_mstsc_shell_window().is_some(),
        ) {
            return;
        }

        unsafe {
            let _ = SetTimer(
                Some(window),
                NATIVE_MSTSC_LAYOUT_TIMER_ID,
                NATIVE_MSTSC_LAYOUT_POLL_MILLISECONDS,
                None,
            );
        }
        self.synchronize_native_mstsc_renderer_layout(window);
        trace_host_call("Renderer::NativeMstscLayoutObserverStarted");
    }

    fn synchronize_native_mstsc_renderer_layout(&self, window: HWND) {
        let Some(shell) = self.native_mstsc_shell_window() else {
            self.native_mstsc_display_layout.set(None);
            unsafe {
                let _ = KillTimer(Some(window), NATIVE_MSTSC_LAYOUT_TIMER_ID);
            }
            return;
        };
        if unsafe { IsIconic(shell) }.as_bool() {
            self.native_mstsc_display_layout.set(None);
            self.cancel_pending_display_resize(window);
            return;
        }

        let mut shell_client = RECT::default();
        if unsafe { GetClientRect(shell, &mut shell_client) }.is_err() {
            return;
        }
        let width = (shell_client.right - shell_client.left).max(1);
        let height = (shell_client.bottom - shell_client.top).max(1);
        let Ok(host) = (unsafe { GetParent(window) }) else {
            return;
        };
        if host.0.is_null() || !unsafe { IsWindow(Some(host)) }.as_bool() {
            return;
        }
        let mut host_client = RECT::default();
        let host_needs_resize = unsafe { GetClientRect(host, &mut host_client) }.is_err()
            || host_client.right - host_client.left != width
            || host_client.bottom - host_client.top != height;
        let mut renderer_client = RECT::default();
        let renderer_needs_resize = unsafe { GetClientRect(window, &mut renderer_client) }.is_err()
            || renderer_client.right - renderer_client.left != width
            || renderer_client.bottom - renderer_client.top != height;
        if host_needs_resize || renderer_needs_resize {
            self.native_mstsc_display_layout.set(None);
            self.pending_display_resize.set(None);
            unsafe {
                let _ = KillTimer(Some(window), DISPLAY_RESIZE_TIMER_ID);
            }
            self.activex_rect.set(RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            });
            self.activex_clip_rect.set(None);
            let result = (|| unsafe {
                if host_needs_resize {
                    SetWindowPos(
                        host,
                        None,
                        0,
                        0,
                        width,
                        height,
                        SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOZORDER,
                    )?;
                }
                SetWindowPos(
                    window,
                    None,
                    0,
                    0,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOZORDER,
                )
            })();
            match result {
                Ok(()) => {
                    unsafe {
                        let _ = InvalidateRect(Some(window), None, false);
                    }
                    trace_host_call("Renderer::NativeMstscShellLayoutSynced");
                }
                Err(error) => {
                    tracing::debug!(?error, "Unable to synchronize the native mstsc renderer layout");
                }
            }
        } else if self.state.get() == ConnectionState::Connected
            && self.native_mstsc_display_layout.replace(Some((width, height))) != Some((width, height))
        {
            self.schedule_display_resize(window, 0);
        }
    }

    fn cancel_pending_display_resize(&self, window: HWND) {
        let had_pending_resize = self.pending_display_resize.replace(None).is_some();
        unsafe {
            let _ = KillTimer(Some(window), DISPLAY_RESIZE_TIMER_ID);
        }
        if had_pending_resize {
            trace_host_call("Renderer::DisplayResizeMinimized");
        }
    }

    fn schedule_display_resize(&self, window: HWND, size_kind: usize) {
        if size_kind == SIZE_MINIMIZED as usize {
            self.cancel_pending_display_resize(window);
            return;
        }

        if self.state.get() != ConnectionState::Connected {
            return;
        }

        let mut client_rect = RECT::default();
        if unsafe { GetClientRect(window, &mut client_rect) }.is_err() {
            return;
        }
        let Some(layout) = display_layout_from_renderer_size(
            client_rect.right - client_rect.left,
            client_rect.bottom - client_rect.top,
        ) else {
            return;
        };

        self.pending_display_resize.set(Some(layout));
        unsafe {
            SetTimer(
                Some(window),
                DISPLAY_RESIZE_TIMER_ID,
                DISPLAY_RESIZE_DEBOUNCE_MILLISECONDS,
                None,
            );
        }
    }

    fn apply_pending_display_resize(&self, window: HWND) {
        unsafe {
            let _ = KillTimer(Some(window), DISPLAY_RESIZE_TIMER_ID);
        }
        let Some(layout) = self.pending_display_resize.take() else {
            return;
        };
        if self.state.get() != ConnectionState::Connected {
            return;
        }

        let result = self.update_display_layout(layout);
        match result {
            Ok(()) => trace_host_call("Renderer::DisplayResizeRequested"),
            Err(error) => {
                trace_host_call("Renderer::DisplayResizeRejected");
                tracing::debug!(
                    ?error,
                    desktop_width = layout.desktop_width,
                    desktop_height = layout.desktop_height,
                    "Unable to queue ActiveX display resize"
                );
            }
        }
    }

    fn present_frame(&self, buffer: Vec<u32>, width: i32, height: i32) {
        let (Ok(width), Ok(height)) = (u16::try_from(width), u16::try_from(height)) else {
            tracing::debug!(width, height, "Discarding ActiveX frame with invalid dimensions");
            return;
        };
        let sequence = self.next_frame_sequence.get().wrapping_add(1).max(1);
        let Some(frame) = Frame::new(&buffer, width, height, sequence) else {
            tracing::debug!(width, height, "Discarding ActiveX frame with invalid pixel count");
            return;
        };

        if let Err(error) = self.update_presentation_surface(&frame, &buffer) {
            trace_host_call("Renderer::SurfaceUpdateFailed");
            tracing::warn!(?error, "Unable to retain ActiveX frame presentation surface");
            return;
        }

        self.next_frame_sequence.set(sequence);
        *self.frame.borrow_mut() = Some(frame);
        let layout_generation = self.presentation_layout_generation.get();
        if layout_generation != 0 && self.traced_frame_layout_generation.replace(layout_generation) != layout_generation
        {
            trace_host_call("Renderer::FrameAcceptedAfterLayout");
        }
        self.notify_ole_advise_view_change();

        let window = self.activex_window.get();
        if !window.0.is_null() && unsafe { IsWindow(Some(window)) }.as_bool() {
            unsafe {
                let _ = InvalidateRect(Some(window), None, false);
            }
        }
        self.rail_windows.borrow().invalidate_presentation();
    }

    fn clear_frame(&self) {
        let had_frame = self.frame.borrow_mut().take().is_some();
        self.presentation_surface.borrow_mut().take();
        self.presentation_backbuffer.borrow_mut().take();
        if !had_frame {
            return;
        }
        self.notify_ole_advise_view_change();

        let window = self.activex_window.get();
        if !window.0.is_null() && unsafe { IsWindow(Some(window)) }.as_bool() {
            unsafe {
                let _ = InvalidateRect(Some(window), None, true);
            }
        }
    }

    fn update_presentation_surface(&self, frame: &Frame, buffer: &[u32]) -> Result<()> {
        let mut surface = self.presentation_surface.borrow_mut();
        if let Some(surface) = surface.as_mut()
            && surface.matches_extent(frame)
        {
            surface.copy_from(frame, buffer);
            return Ok(());
        }

        let surface_for_frame = PresentationSurface::new(frame, buffer)?;
        *surface = Some(surface_for_frame);
        Ok(())
    }

    fn update_presentation_backbuffer(&self, width: i32, height: i32) -> Result<()> {
        let mut backbuffer = self.presentation_backbuffer.borrow_mut();
        if backbuffer
            .as_ref()
            .is_some_and(|backbuffer| backbuffer.matches_extent(width, height))
        {
            return Ok(());
        }

        *backbuffer = Some(PresentationBackbuffer::new(width, height)?);
        Ok(())
    }

    fn paint_activex_window(&self, window: HWND) {
        let mut paint = PAINTSTRUCT::default();
        let device_context = unsafe { BeginPaint(window, &mut paint) };
        let mut client_rect = RECT::default();

        if unsafe { GetClientRect(window, &mut client_rect) }.is_ok() {
            let client_width = (client_rect.right - client_rect.left).max(0);
            let client_height = (client_rect.bottom - client_rect.top).max(0);
            if client_width == 0 || client_height == 0 {
                unsafe {
                    let _ = EndPaint(window, &paint);
                }
                return;
            }
            if let Err(error) = self.update_presentation_backbuffer(client_width, client_height) {
                trace_host_call("Renderer::BackbufferUpdateFailed");
                tracing::warn!(?error, "Unable to update the ActiveX presentation backbuffer");
                unsafe {
                    let _ = EndPaint(window, &paint);
                }
                return;
            }

            let backbuffer = self.presentation_backbuffer.borrow();
            let Some(backbuffer) = backbuffer.as_ref() else {
                trace_host_call("Renderer::BackbufferMissing");
                unsafe {
                    let _ = EndPaint(window, &paint);
                }
                return;
            };
            // Compose clear letterbox areas and the scaled remote frame off-screen, then copy the
            // completed image to the visible DC so paint ordering cannot expose an intermediate black frame.
            if !unsafe { PatBlt(backbuffer.device_context, 0, 0, client_width, client_height, BLACKNESS) }.as_bool() {
                trace_host_call("Renderer::PaintClearFailed");
                tracing::debug!("Unable to clear the ActiveX paint target");
            }
            let frame = self.frame.borrow();
            let surface = self.presentation_surface.borrow();
            if let (Some(frame), Some(surface)) = (frame.as_ref(), surface.as_ref()) {
                if !surface.matches_frame(frame) {
                    trace_host_call("Renderer::SurfaceFrameMismatch");
                    tracing::warn!("ActiveX presentation surface did not match the retained frame");
                } else {
                    let layout_generation = self.presentation_layout_generation.get();
                    if layout_generation != 0
                        && self.traced_paint_layout_generation.replace(layout_generation) != layout_generation
                    {
                        trace_host_call("Renderer::PaintRetainedFrameAfterLayout");
                    }
                    let (destination_x, destination_y, destination_width, destination_height) =
                        self.frame_viewport(&client_rect, frame.width, frame.height);
                    if !unsafe {
                        StretchBlt(
                            backbuffer.device_context,
                            destination_x,
                            destination_y,
                            destination_width,
                            destination_height,
                            Some(surface.device_context),
                            0,
                            0,
                            i32::from(frame.width),
                            i32::from(frame.height),
                            SRCCOPY,
                        )
                    }
                    .as_bool()
                    {
                        trace_host_call("Renderer::PaintFrameFailed");
                        tracing::debug!("Unable to paint the ActiveX retained presentation surface");
                    }
                }
            }
            if !unsafe { GdiFlush() }.as_bool() {
                trace_host_call("Renderer::BackbufferFlushFailed");
                tracing::debug!("Unable to flush the ActiveX presentation backbuffer");
                unsafe {
                    let _ = EndPaint(window, &paint);
                }
                return;
            }
            let bitmap_info = top_down_rgb32_bitmap_info(client_width, client_height);
            if unsafe {
                StretchDIBits(
                    device_context,
                    0,
                    0,
                    client_width,
                    client_height,
                    0,
                    0,
                    client_width,
                    client_height,
                    Some(backbuffer.pixels.cast()),
                    &bitmap_info,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                )
            } <= 0
            {
                trace_host_call("Renderer::PresentBackbufferFailed");
                tracing::debug!("Unable to present the ActiveX presentation backbuffer");
            }
        }

        unsafe {
            let _ = EndPaint(window, &paint);
        }
    }

    fn apply_activex_window_clip(&self, window: HWND, position: RECT) -> Result<()> {
        let result = if let Some(clip) = self.activex_clip_rect.get() {
            let local_clip = renderer_clip_region(position, clip);
            let region = unsafe { CreateRectRgn(local_clip.left, local_clip.top, local_clip.right, local_clip.bottom) };
            if region.0.is_null() {
                return Err(Error::from_hresult(E_OUTOFMEMORY));
            }

            let result = unsafe { SetWindowRgn(window, Some(region), true) };
            if result == 0 {
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(region.0));
                }
            }
            result
        } else {
            unsafe { SetWindowRgn(window, None, true) }
        };
        if result != 0 {
            return Ok(());
        }

        let error = unsafe { windows::Win32::Foundation::GetLastError() };
        Err(Error::from_hresult(if error.0 == 0 {
            E_FAIL
        } else {
            HRESULT::from_win32(error.0)
        }))
    }

    fn apply_input(&self, operations: impl IntoIterator<Item = Operation>) {
        let fast_path = self.input_database.borrow_mut().apply(operations);
        if fast_path.is_empty() {
            return;
        }

        if let Some(sender) = self.input_sender.borrow().as_ref()
            && sender.try_send(RdpInputEvent::FastPath(fast_path)).is_err()
        {
            tracing::warn!("Unable to enqueue ActiveX input for the RDP session");
        }
    }

    /// Reserves input-queue capacity, then commits tracker transitions and sends.
    ///
    /// Capacity is reserved before mutating the tracker so a full queue cannot
    /// desynchronize local contact state from the server.
    fn send_touch_event_with<F>(&self, build: F)
    where
        F: FnOnce(&mut TouchContactTracker) -> Option<TouchEventPdu>,
    {
        let Some(sender) = self.input_sender.borrow().as_ref().cloned() else {
            return;
        };
        let Ok(permit) = sender.try_reserve() else {
            tracing::warn!("Unable to enqueue ActiveX RDPEI touch event for the RDP session");
            return;
        };
        if let Some(event) = build(&mut self.touch_tracker.borrow_mut()) {
            permit.send(RdpInputEvent::Touch(event));
        }
    }

    fn release_touch_contacts(&self) {
        self.send_touch_event_with(|tracker| tracker.release_all());
    }

    /// Maps client-area coordinates onto the remote desktop (signed RDPEI space).
    fn desktop_position(&self, window: HWND, x: i32, y: i32) -> Option<(i32, i32)> {
        let position = self.mouse_position(window, x, y)?;
        Some((i32::from(position.x), i32::from(position.y)))
    }

    fn suppress_mouse_for_touch(&self) -> bool {
        self.touch_tracker.borrow().has_active_contacts()
    }

    fn handle_pointer_message(&self, window: HWND, message: u32, wparam: WPARAM, _lparam: LPARAM) -> bool {
        let pointer_id = (wparam.0 & 0xffff) as u32;

        let mut info = POINTER_INFO::default();
        if unsafe { GetPointerInfo(pointer_id, &mut info) }.is_err() {
            return false;
        }
        if info.pointerType != PT_TOUCH {
            // Pen and other pointer types are not remoted in this cut.
            return false;
        }

        let mut pointer_count = 0u32;
        if unsafe { GetPointerFrameTouchInfo(pointer_id, &mut pointer_count, None) }.is_err() || pointer_count == 0 {
            return true;
        }

        let mut touch_infos = vec![POINTER_TOUCH_INFO::default(); pointer_count as usize];
        if unsafe { GetPointerFrameTouchInfo(pointer_id, &mut pointer_count, Some(touch_infos.as_mut_ptr())) }.is_err()
        {
            return true;
        }
        touch_infos.truncate(pointer_count as usize);

        let mut samples = Vec::with_capacity(touch_infos.len());
        for touch in &touch_infos {
            let ptr = touch.pointerInfo;
            if ptr.pointerType != PT_TOUCH {
                continue;
            }

            let mut client_point = ptr.ptPixelLocation;
            if !unsafe { ScreenToClient(window, &mut client_point) }.as_bool() {
                continue;
            }
            let mapped = self.desktop_position(window, client_point.x, client_point.y);
            let canceled = (ptr.pointerFlags & POINTER_FLAG_CANCELED).0 != 0
                || message == WM_POINTERCAPTURECHANGED
                || (message == WM_POINTERLEAVE && (ptr.pointerFlags & POINTER_FLAG_INCONTACT).0 == 0);
            let leaving = (ptr.pointerFlags & POINTER_FLAG_UP).0 != 0 || canceled;

            let Some((x, y)) = mapped else {
                // Outside the rendered desktop: still emit terminal transitions, keeping
                // the last tracked coordinates rather than inventing (0, 0).
                if leaving {
                    samples.push(TouchSample {
                        pointer_id: ptr.pointerId,
                        x: 0,
                        y: 0,
                        preserve_position: true,
                        in_range: false,
                        in_contact: false,
                        canceled,
                        orientation: None,
                        pressure: None,
                        contact_rect: None,
                    });
                }
                continue;
            };

            let contact_rect = if touch.touchMask & TOUCH_MASK_CONTACTAREA != 0 {
                let mut tl = POINT {
                    x: touch.rcContact.left,
                    y: touch.rcContact.top,
                };
                let mut br = POINT {
                    x: touch.rcContact.right,
                    y: touch.rcContact.bottom,
                };
                if unsafe { ScreenToClient(window, &mut tl) }.as_bool()
                    && unsafe { ScreenToClient(window, &mut br) }.as_bool()
                {
                    // Contact rect is exclusive bounds relative to the contact point.
                    match (
                        self.desktop_position(window, tl.x, tl.y),
                        self.desktop_position(window, br.x, br.y),
                    ) {
                        (Some((l, t)), Some((r, b))) => {
                            let left = i16::try_from(l.saturating_sub(x)).unwrap_or(i16::MIN);
                            let top = i16::try_from(t.saturating_sub(y)).unwrap_or(i16::MIN);
                            let right = i16::try_from(r.saturating_sub(x)).unwrap_or(i16::MAX);
                            let bottom = i16::try_from(b.saturating_sub(y)).unwrap_or(i16::MAX);
                            Some((left, top, right, bottom))
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            } else {
                None
            };

            let orientation = if touch.touchMask & TOUCH_MASK_ORIENTATION != 0 {
                Some(TouchContactTracker::win32_orientation_to_rdpei(touch.orientation))
            } else {
                None
            };
            let pressure = if touch.touchMask & TOUCH_MASK_PRESSURE != 0 {
                Some(touch.pressure)
            } else {
                None
            };

            samples.push(TouchSample {
                pointer_id: ptr.pointerId,
                x,
                y,
                preserve_position: false,
                in_range: (ptr.pointerFlags & POINTER_FLAG_INRANGE).0 != 0,
                in_contact: (ptr.pointerFlags & POINTER_FLAG_INCONTACT).0 != 0,
                canceled,
                orientation,
                pressure,
                contact_rect,
            });
        }

        self.send_touch_event_with(|tracker| tracker.process_samples(&samples));

        let _ = unsafe { SkipPointerFrameMessages(pointer_id) };
        true
    }

    fn send_keys(&self, key_count: i32, key_up: *const i16, key_data: *const i32) -> Result<()> {
        let key_count = usize::try_from(key_count)
            .ok()
            .filter(|count| *count <= MSTSC_SEND_KEYS_MAX_KEYS)
            .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
        if key_count == 0 {
            return Ok(());
        }
        if key_up.is_null() || key_data.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        if self.state.get() != ConnectionState::Connected {
            return Err(Error::from_hresult(E_UNEXPECTED));
        }
        let key_up = unsafe { slice::from_raw_parts(key_up, key_count) };
        let key_data = unsafe { slice::from_raw_parts(key_data, key_count) };
        let mut operations = key_up.iter().zip(key_data).map(|(key_up, key_data)| {
            let key_data = *key_data as u32;
            let scancode = Scancode::from_u8(key_data & (1 << 24) != 0, (key_data >> 16) as u8);
            if *key_up == VARIANT_FALSE.0 {
                Operation::KeyPressed(scancode)
            } else {
                Operation::KeyReleased(scancode)
            }
        });

        self.send_input_operations(&mut operations)
    }

    fn send_input_operations(&self, operations: &mut dyn Iterator<Item = Operation>) -> Result<()> {
        if self.state.get() != ConnectionState::Connected {
            return Err(Error::from_hresult(E_UNEXPECTED));
        }
        let sender = self
            .input_sender
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| Error::from_hresult(E_UNEXPECTED))?;
        let permit = sender.try_reserve().map_err(|_| Error::from_hresult(E_UNEXPECTED))?;
        let fast_path = self.input_database.borrow_mut().apply(operations);
        if fast_path.is_empty() {
            return Ok(());
        }
        permit.send(RdpInputEvent::FastPath(fast_path));
        Ok(())
    }

    fn send_remote_action(&self, action: i32) -> Result<()> {
        let shortcut: &[Scancode] = match action {
            REMOTE_SESSION_ACTION_CHARMS => &[Scancode::from_u8(true, 0x5b), Scancode::from_u8(false, 0x2e)],
            REMOTE_SESSION_ACTION_APPBAR => &[Scancode::from_u8(true, 0x5b), Scancode::from_u8(false, 0x2c)],
            REMOTE_SESSION_ACTION_SNAP => return Err(Error::from_hresult(E_NOTIMPL)),
            REMOTE_SESSION_ACTION_START_SCREEN => &[Scancode::from_u8(true, 0x5b)],
            REMOTE_SESSION_ACTION_APP_SWITCH => &[Scancode::from_u8(false, 0x38), Scancode::from_u8(false, 0x0f)],
            REMOTE_SESSION_ACTION_ACTION_CENTER => &[Scancode::from_u8(true, 0x5b), Scancode::from_u8(false, 0x1e)],
            REMOTE_SESSION_ACTION_TASK_MANAGER => &[
                Scancode::from_u8(false, 0x1d),
                Scancode::from_u8(false, 0x2a),
                Scancode::from_u8(false, 0x01),
            ],
            _ => return Err(Error::from_hresult(E_INVALIDARG)),
        };

        let previously_pressed = {
            let input_database = self.input_database.borrow();
            let mut pressed = Vec::new();
            for key in REMOTE_ACTION_MODIFIERS.iter().chain(shortcut) {
                if input_database.is_key_pressed(*key) && !pressed.contains(key) {
                    pressed.push(*key);
                }
            }
            pressed
        };
        let mut operations = Vec::with_capacity(previously_pressed.len() * 2 + shortcut.len() * 2);
        operations.extend(previously_pressed.iter().copied().map(Operation::KeyReleased));
        operations.extend(shortcut.iter().copied().map(Operation::KeyPressed));
        operations.extend(shortcut.iter().rev().copied().map(Operation::KeyReleased));
        operations.extend(previously_pressed.iter().copied().map(Operation::KeyPressed));
        let mut operations = operations.into_iter();
        self.send_input_operations(&mut operations)
    }

    fn release_input(&self) {
        self.release_touch_contacts();
        let fast_path = self.input_database.borrow_mut().release_all();
        if fast_path.is_empty() {
            return;
        }

        if let Some(sender) = self.input_sender.borrow().as_ref()
            && sender.try_send(RdpInputEvent::FastPath(fast_path)).is_err()
        {
            tracing::warn!("Unable to enqueue ActiveX input release for the RDP session");
        }
    }

    fn mouse_position(&self, window: HWND, x: i32, y: i32) -> Option<MousePosition> {
        let frame = self.frame.borrow();
        let frame = frame.as_ref()?;
        let mut client_rect = RECT::default();
        unsafe { GetClientRect(window, &mut client_rect) }.ok()?;

        let (destination_x, destination_y, destination_width, destination_height) =
            self.frame_viewport(&client_rect, frame.width, frame.height);
        if x < destination_x
            || y < destination_y
            || x >= destination_x + destination_width
            || y >= destination_y + destination_height
        {
            return None;
        }
        let x = (i64::from(x - destination_x) * i64::from(frame.width) / i64::from(destination_width))
            .min(i64::from(frame.width.saturating_sub(1)));
        let y = (i64::from(y - destination_y) * i64::from(frame.height) / i64::from(destination_height))
            .min(i64::from(frame.height.saturating_sub(1)));

        Some(MousePosition {
            x: u16::try_from(x).ok()?,
            y: u16::try_from(y).ok()?,
        })
    }

    fn frame_viewport(&self, client_rect: &RECT, frame_width: u16, frame_height: u16) -> (i32, i32, i32, i32) {
        let client_width = (client_rect.right - client_rect.left).max(1);
        let client_height = (client_rect.bottom - client_rect.top).max(1);
        let compatibility = self.compatibility.borrow();
        let zoom_level = i64::from(compatibility.zoom_level);
        let fit_to_client = compatibility.smart_sizing
            || native_shell_presentation_enabled(
                native_mstsc_shell_integration_enabled(),
                self.native_mstsc_shell_window().is_some(),
            );
        let (base_width, base_height) = if fit_to_client {
            let scale_width = i64::from(client_width) * i64::from(frame_height);
            let scale_height = i64::from(client_height) * i64::from(frame_width);
            if scale_width <= scale_height {
                (i64::from(client_width), scale_width / i64::from(frame_width))
            } else {
                (scale_height / i64::from(frame_height), i64::from(client_height))
            }
        } else {
            (i64::from(frame_width), i64::from(frame_height))
        };
        let width = (base_width * zoom_level / 100).clamp(1, i64::from(i32::MAX)) as i32;
        let height = (base_height * zoom_level / 100).clamp(1, i64::from(i32::MAX)) as i32;
        ((client_width - width) / 2, (client_height - height) / 2, width, height)
    }

    fn apply_mouse_operation(&self, window: HWND, lparam: LPARAM, operation: Operation) {
        let x = i32::from(lparam.0 as i32 as i16);
        let y = i32::from((lparam.0 >> 16) as i16);
        if let Some(position) = self.mouse_position(window, x, y) {
            self.apply_input([Operation::MouseMove(position), operation]);
        }
    }

    fn release_mouse_capture_if_idle(&self) {
        let has_pressed_buttons = {
            let input_database = self.input_database.borrow();
            [
                MouseButton::Left,
                MouseButton::Middle,
                MouseButton::Right,
                MouseButton::X1,
                MouseButton::X2,
            ]
            .into_iter()
            .any(|button| input_database.is_mouse_button_pressed(button))
        };

        if !has_pressed_buttons && unsafe { ReleaseCapture() }.is_err() {
            tracing::debug!("Unable to release ActiveX mouse capture");
        }
    }

    fn handle_activex_window_message(&self, window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
        match message {
            WM_UPDATE_CONNECTION_BAR => {
                self.update_connection_bar();
                true
            }
            WM_SHOWWINDOW => {
                self.renderer_visibility_changed(wparam.0 != 0);
                false
            }
            WM_DPICHANGED => {
                // The suggested rectangle is pointer-backed and belongs to the OLE container.
                // Reflow only IronRDP-owned UI from current window DPI, then let the host retain
                // ownership of the renderer's rectangle.
                self.renderer_dpi_changed();
                false
            }
            WM_MOVE | WM_SIZE => {
                if message == WM_SIZE
                    && wparam.0 == SIZE_MINIMIZED as usize
                    && native_shell_presentation_enabled(
                        native_mstsc_shell_integration_enabled(),
                        self.native_mstsc_shell_window().is_some(),
                    )
                {
                    self.native_mstsc_display_layout.set(None);
                    self.cancel_pending_display_resize(window);
                }
                self.renderer_geometry_changed();
                false
            }
            WM_TIMER if wparam.0 == DISPLAY_RESIZE_TIMER_ID => {
                self.apply_pending_display_resize(window);
                true
            }
            WM_TIMER if wparam.0 == NATIVE_MSTSC_LAYOUT_TIMER_ID => {
                self.synchronize_native_mstsc_renderer_layout(window);
                true
            }
            WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
                let virtual_key = VIRTUAL_KEY(wparam.0 as u16);
                let control_and_alt_pressed = unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0
                    && unsafe { GetKeyState(i32::from(VK_MENU.0)) } < 0;
                if matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN)
                    && is_fullscreen_hotkey(virtual_key, control_and_alt_pressed)
                {
                    trace_host_call("IMsTscAx::FullScreenHotKey");
                    self.request_fullscreen_toggle();
                    return true;
                }
                let lparam = lparam.0 as u32;
                let scancode = Scancode::from_u8(lparam & 0x0100_0000 != 0, ((lparam >> 16) & 0xff) as u8);
                let compatibility = self.compatibility.borrow();
                let input_database = self.input_database.borrow();
                if !should_forward_windows_key(
                    &compatibility,
                    self.settings.borrow().fullscreen,
                    &input_database,
                    message,
                    scancode,
                ) {
                    return true;
                }
                drop(input_database);
                drop(compatibility);
                let operation = match message {
                    WM_KEYDOWN | WM_SYSKEYDOWN => Operation::KeyPressed(scancode),
                    _ => Operation::KeyReleased(scancode),
                };
                self.apply_input([operation]);
                true
            }
            WM_POINTERUPDATE | WM_POINTERDOWN | WM_POINTERUP | WM_POINTERLEAVE | WM_POINTERCAPTURECHANGED => {
                self.handle_pointer_message(window, message, wparam, lparam)
            }
            WM_MOUSEMOVE => {
                if i32::from((lparam.0 >> 16) as i16) <= 4 {
                    self.expose_connection_bar();
                }
                if !self.compatibility.borrow().enable_mouse || self.suppress_mouse_for_touch() {
                    return true;
                }
                let x = i32::from(lparam.0 as i32 as i16);
                let y = i32::from((lparam.0 >> 16) as i16);
                if let Some(position) = self.mouse_position(window, x, y) {
                    self.apply_input([Operation::MouseMove(position)]);
                }
                true
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
                if !self.compatibility.borrow().enable_mouse || self.suppress_mouse_for_touch() {
                    return true;
                }
                if let Err(error) = unsafe { SetFocus(Some(window)) } {
                    tracing::debug!(?error, "Unable to focus ActiveX rendering window");
                }
                unsafe {
                    SetCapture(window);
                }
                let button = match message {
                    WM_LBUTTONDOWN => MouseButton::Left,
                    WM_RBUTTONDOWN => MouseButton::Right,
                    WM_MBUTTONDOWN => MouseButton::Middle,
                    _ if ((wparam.0 >> 16) & 0xffff) == 1 => MouseButton::X1,
                    _ => MouseButton::X2,
                };
                self.apply_mouse_operation(window, lparam, Operation::MouseButtonPressed(button));
                true
            }
            WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP => {
                if self.suppress_mouse_for_touch() {
                    return true;
                }
                let button = match message {
                    WM_LBUTTONUP => MouseButton::Left,
                    WM_RBUTTONUP => MouseButton::Right,
                    WM_MBUTTONUP => MouseButton::Middle,
                    _ if ((wparam.0 >> 16) & 0xffff) == 1 => MouseButton::X1,
                    _ => MouseButton::X2,
                };
                self.apply_mouse_operation(window, lparam, Operation::MouseButtonReleased(button));
                self.release_mouse_capture_if_idle();
                true
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                if !self.compatibility.borrow().enable_mouse || self.suppress_mouse_for_touch() {
                    return true;
                }
                let mut point = POINT {
                    x: i32::from(lparam.0 as i32 as i16),
                    y: i32::from((lparam.0 >> 16) as i16),
                };
                if unsafe { ScreenToClient(window, &mut point) }.as_bool()
                    && let Some(position) = self.mouse_position(window, point.x, point.y)
                {
                    self.apply_input([Operation::MouseMove(position)]);
                }

                let mut remaining = ((wparam.0 >> 16) as u16) as i16;
                let mut operations = Vec::new();
                while remaining != 0 {
                    let rotation_units = remaining.clamp(-256, 255);
                    operations.push(Operation::WheelRotations(WheelRotations {
                        is_vertical: message == WM_MOUSEWHEEL,
                        rotation_units,
                    }));
                    remaining -= rotation_units;
                }
                self.apply_input(operations);
                true
            }
            WM_CANCELMODE | WM_ENABLE if wparam.0 == 0 => {
                self.release_input();
                false
            }
            WM_KILLFOCUS | WM_CAPTURECHANGED => {
                self.release_input();
                true
            }
            _ => false,
        }
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        trace_host_call("Control::Drop");
        self.events.close();
        self.rail_windows.get_mut().stop();
        if let Some(rpc) = &self.rpc {
            rpc.stop();
        }
        let _ = self.stop_connection();
        self.stop_clipboard_redirection();
        if let Err(error) = self.destroy_connection_bar() {
            tracing::error!(?error, "Unable to destroy ActiveX connection bar");
        }
        self.clear_connection_health_window();
        if let Err(error) = self.destroy_activex_window() {
            tracing::error!(?error, "Unable to destroy ActiveX child window");
        }
        if let Err(error) = self.destroy_dispatcher_window() {
            tracing::error!(?error, "Unable to destroy ActiveX event dispatcher window");
        }
        com::release_object();
    }
}

impl IDispatch_Impl for Control_Impl {
    fn GetTypeInfoCount(&self) -> Result<u32> {
        Ok(0)
    }

    fn GetTypeInfo(&self, _itinfo: u32, _lcid: u32) -> Result<ITypeInfo> {
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn GetIDsOfNames(
        &self,
        riid: *const GUID,
        names: *const PCWSTR,
        count: u32,
        _lcid: u32,
        dispids: *mut i32,
    ) -> Result<()> {
        if names.is_null() || dispids.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        if !riid.is_null() && unsafe { *riid } != GUID::zeroed() {
            return Err(Error::from_hresult(DISP_E_MEMBERNOTFOUND));
        }

        for index in 0..count as usize {
            let name =
                unsafe { (*names.add(index)).to_string() }.map_err(|_| Error::from_hresult(DISP_E_UNKNOWNNAME))?;
            let dispid = dispid_for_name(&name).ok_or_else(|| Error::from_hresult(DISP_E_UNKNOWNNAME))?;
            unsafe {
                dispids.add(index).write(dispid);
            }
        }
        Ok(())
    }

    fn Invoke(
        &self,
        dispid: i32,
        riid: *const GUID,
        _lcid: u32,
        flags: DISPATCH_FLAGS,
        params: *const DISPPARAMS,
        result: *mut VARIANT,
        _exception: *mut EXCEPINFO,
        argument_error: *mut u32,
    ) -> Result<()> {
        trace_host_call(&format!("IDispatch::Invoke({dispid}, {flags:?})"));
        self.remember_callback_owner(self);
        if params.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        if !riid.is_null() && unsafe { *riid } != GUID::zeroed() {
            return Err(Error::from_hresult(DISP_E_MEMBERNOTFOUND));
        }

        self.dispatch_pending_events();
        let params = unsafe { &*params };

        if flags.contains(DISPATCH_PROPERTYGET) {
            if params.cArgs != 0 {
                return Err(Error::from_hresult(DISP_E_BADPARAMCOUNT));
            }
            return self.get_property(dispid, result);
        }

        if flags.contains(DISPATCH_PROPERTYPUT) {
            return self.put_property(dispid, params, argument_error);
        }

        if flags.contains(DISPATCH_METHOD) {
            if params.cArgs != 0 || params.cNamedArgs != 0 {
                return Err(Error::from_hresult(DISP_E_BADPARAMCOUNT));
            }

            return match dispid {
                DISPID_CONNECT => self.start_connection(),
                DISPID_DISCONNECT => self.stop_connection(),
                _ => Err(Error::from_hresult(DISP_E_MEMBERNOTFOUND)),
            };
        }

        Err(Error::from_hresult(DISP_E_MEMBERNOTFOUND))
    }
}

impl IMsTscAx_Redist_Impl for Control_Impl {}

impl IMsTscAx_Impl for Control_Impl {
    unsafe fn put_Server(&self, server: Bstr) -> Result<()> {
        trace_host_call("IMsTscAx::put_Server");
        self.settings.borrow_mut().server = string_from_bstr(server)?;
        self.persistence_dirty.set(true);
        Ok(())
    }

    unsafe fn get_Server(&self, server: BstrOut) -> Result<()> {
        write_bstr(server, &self.settings.borrow().server)
    }

    unsafe fn put_Domain(&self, domain: Bstr) -> Result<()> {
        self.settings.borrow_mut().domain = string_from_bstr(domain)?;
        self.persistence_dirty.set(true);
        Ok(())
    }

    unsafe fn get_Domain(&self, domain: BstrOut) -> Result<()> {
        write_bstr(domain, &self.settings.borrow().domain)
    }

    unsafe fn put_UserName(&self, username: Bstr) -> Result<()> {
        self.settings.borrow_mut().username = string_from_bstr(username)?;
        self.persistence_dirty.set(true);
        Ok(())
    }

    unsafe fn get_UserName(&self, username: BstrOut) -> Result<()> {
        write_bstr(username, &self.settings.borrow().username)
    }

    unsafe fn put_DisconnectedText(&self, text: Bstr) -> Result<()> {
        self.settings.borrow_mut().disconnected_text = string_from_bstr(text)?;
        self.persistence_dirty.set(true);
        Ok(())
    }

    unsafe fn get_DisconnectedText(&self, text: BstrOut) -> Result<()> {
        write_bstr(text, &self.settings.borrow().disconnected_text)
    }

    unsafe fn put_ConnectingText(&self, text: Bstr) -> Result<()> {
        self.settings.borrow_mut().connecting_text = string_from_bstr(text)?;
        self.persistence_dirty.set(true);
        Ok(())
    }

    unsafe fn get_ConnectingText(&self, text: BstrOut) -> Result<()> {
        write_bstr(text, &self.settings.borrow().connecting_text)
    }

    unsafe fn get_Connected(&self, connected: *mut i16) -> Result<()> {
        trace_host_call("IMsTscAx::get_Connected");
        write_out(connected, i16::from(self.state.get() == ConnectionState::Connected))
    }

    unsafe fn put_DesktopWidth(&self, width: i32) -> Result<()> {
        self.settings.borrow_mut().desktop_width = raw_dimension(width, "desktop width")?;
        self.persistence_dirty.set(true);
        Ok(())
    }

    unsafe fn get_DesktopWidth(&self, width: *mut i32) -> Result<()> {
        write_out(width, i32::from(self.settings.borrow().desktop_width))
    }

    unsafe fn put_DesktopHeight(&self, height: i32) -> Result<()> {
        self.settings.borrow_mut().desktop_height = raw_dimension(height, "desktop height")?;
        self.persistence_dirty.set(true);
        Ok(())
    }

    unsafe fn get_DesktopHeight(&self, height: *mut i32) -> Result<()> {
        write_out(height, i32::from(self.settings.borrow().desktop_height))
    }

    unsafe fn put_StartConnected(&self, start_connected: i32) -> Result<()> {
        self.settings.borrow_mut().start_connected = start_connected != 0;
        self.persistence_dirty.set(true);
        Ok(())
    }

    unsafe fn get_StartConnected(&self, start_connected: *mut i32) -> Result<()> {
        write_out(start_connected, i32::from(self.settings.borrow().start_connected))
    }

    unsafe fn get_HorizontalScrollBarVisible(&self, visible: *mut i32) -> Result<()> {
        write_out(visible, 0)
    }

    unsafe fn get_VerticalScrollBarVisible(&self, visible: *mut i32) -> Result<()> {
        write_out(visible, 0)
    }

    unsafe fn put_FullScreenTitle(&self, title: Bstr) -> Result<()> {
        self.settings.borrow_mut().fullscreen_title = string_from_bstr(title)?;
        self.persistence_dirty.set(true);
        Ok(())
    }

    unsafe fn get_CipherStrength(&self, strength: *mut i32) -> Result<()> {
        write_out(strength, 128)
    }

    unsafe fn get_Version(&self, version: BstrOut) -> Result<()> {
        write_bstr(version, "IronRDP ActiveX 0.1")
    }

    unsafe fn get_SecuredSettingsEnabled(&self, enabled: *mut i32) -> Result<()> {
        trace_host_call("IMsTscAx::get_SecuredSettingsEnabled");
        write_out(enabled, VARIANT_TRUE.0.into())
    }

    unsafe fn get_SecuredSettings(&self, settings: InterfaceOut) -> Result<()> {
        trace_host_call("IMsTscAx::get_SecuredSettings");
        let owner: IUnknown = self.to_interface();
        unsafe {
            settings_object_with_bridge(
                secured_vtable(),
                Rc::clone(&self.compatibility),
                Some(NativeMstscCredentialBridge {
                    _owner: owner,
                    control: self,
                }),
                settings,
            )
        }
    }

    unsafe fn get_AdvancedSettings(&self, settings: InterfaceOut) -> Result<()> {
        trace_host_call("IMsTscAx::get_AdvancedSettings");
        unsafe { settings_object(advanced_vtable(), Rc::clone(&self.compatibility), settings) }
    }

    unsafe fn get_Debugger(&self, debugger: InterfaceOut) -> Result<()> {
        unsupported_out(debugger)
    }

    unsafe fn Connect(&self) -> Result<()> {
        trace_host_call("IMsTscAx::Connect");
        self.remember_callback_owner(self);
        self.start_connection()
    }

    unsafe fn Disconnect(&self) -> Result<()> {
        trace_host_call("IMsTscAx::Disconnect");
        self.stop_connection()
    }

    unsafe fn CreateVirtualChannels(&self, channels: Bstr) -> Result<()> {
        if self.state.get() != ConnectionState::Disconnected {
            return Err(Error::from_hresult(E_UNEXPECTED));
        }

        let channels = static_channel_names_from_bstr(channels)?;
        let mut static_channels = self.static_channels.borrow_mut();
        let requested_names = channels.iter().map(|(name, _)| name).collect::<BTreeSet<_>>();
        if static_channels.len().saturating_add(channels.len()) > MAX_ACTIVEX_STATIC_CHANNELS
            || requested_names.len() != channels.len()
            || channels
                .iter()
                .any(|(name, _)| static_channels.contains_key(name) || is_reserved_static_channel_name(name))
        {
            return Err(Error::from_hresult(E_INVALIDARG));
        }

        for (display_name, channel_name) in channels {
            static_channels.insert(
                display_name.clone(),
                ActiveXStaticChannelSpec {
                    display_name,
                    channel_name,
                    options: ChannelOptions::empty(),
                },
            );
        }
        Ok(())
    }

    unsafe fn SendOnVirtualChannel(&self, channel: Bstr, data: Bstr) -> Result<()> {
        let (channel_name, protocol_name) = static_channel_name_from_bstr(channel)?;
        if !self.static_channels.borrow().contains_key(&channel_name) {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        let data = channel_data_from_bstr(data)?;

        // mstscax accepts a send before a channel is established. Keep that compatibility
        // behavior while never queueing data against a stale session.
        if self.state.get() != ConnectionState::Connected {
            return Ok(());
        }

        let sender = self
            .input_sender
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| Error::from_hresult(E_FAIL))?;
        sender
            .try_send(RdpInputEvent::SendStaticChannelData {
                channel_name: protocol_name,
                data,
            })
            .map_err(|_| Error::from_hresult(E_FAIL))
    }
}

impl IMsRdpClient_Impl for Control_Impl {
    unsafe fn put_ColorDepth(&self, color_depth: i32) -> Result<()> {
        if color_depth != 16 && color_depth != 32 {
            return Err(Error::new(E_INVALIDARG, "color depth must be 16 or 32"));
        }
        self.settings.borrow_mut().color_depth = color_depth as u32;
        self.persistence_dirty.set(true);
        Ok(())
    }

    unsafe fn get_ColorDepth(&self, color_depth: *mut i32) -> Result<()> {
        write_out(
            color_depth,
            i32::try_from(self.settings.borrow().color_depth).map_err(|_| Error::from_hresult(E_FAIL))?,
        )
    }

    unsafe fn get_AdvancedSettings2(&self, settings: InterfaceOut) -> Result<()> {
        unsafe { settings_object(advanced_vtable(), Rc::clone(&self.compatibility), settings) }
    }

    unsafe fn get_SecuredSettings2(&self, settings: InterfaceOut) -> Result<()> {
        let owner: IUnknown = self.to_interface();
        unsafe {
            settings_object_with_bridge(
                secured_vtable(),
                Rc::clone(&self.compatibility),
                Some(NativeMstscCredentialBridge {
                    _owner: owner,
                    control: self,
                }),
                settings,
            )
        }
    }

    unsafe fn get_ExtendedDisconnectReason(&self, reason: *mut i32) -> Result<()> {
        write_out(reason, self.last_disconnect.get().extended_reason)
    }

    unsafe fn put_FullScreen(&self, fullscreen: i16) -> Result<()> {
        trace_host_call("IMsRdpClient::put_FullScreen");
        self.set_fullscreen(fullscreen != 0)
    }

    unsafe fn get_FullScreen(&self, fullscreen: *mut i16) -> Result<()> {
        write_out(fullscreen, i16::from(self.settings.borrow().fullscreen))
    }

    unsafe fn SetVirtualChannelOptions(&self, channel: Bstr, options: i32) -> Result<()> {
        if self.state.get() != ConnectionState::Disconnected {
            return Err(Error::from_hresult(E_UNEXPECTED));
        }

        let (channel_name, _) = static_channel_name_from_bstr(channel)?;
        let options = ChannelOptions::from_bits(u32::from_ne_bytes(options.to_ne_bytes()))
            .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
        let mut static_channels = self.static_channels.borrow_mut();
        let channel = static_channels
            .get_mut(&channel_name)
            .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
        channel.options = options;
        Ok(())
    }

    unsafe fn GetVirtualChannelOptions(&self, channel: Bstr, options_out: *mut i32) -> Result<()> {
        write_out(options_out, 0)?;
        let (channel_name, _) = static_channel_name_from_bstr(channel)?;
        let channel_options = self
            .static_channels
            .borrow()
            .get(&channel_name)
            .map(|channel| channel.options.bits())
            .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
        write_out(options_out, i32::from_ne_bytes(channel_options.to_ne_bytes()))
    }

    unsafe fn RequestClose(&self, status: *mut i32) -> Result<()> {
        write_out(status, self.request_close_status())
    }
}

impl IMsRdpClient2_Impl for Control_Impl {
    unsafe fn get_AdvancedSettings3(&self, settings: InterfaceOut) -> Result<()> {
        unsafe { settings_object(advanced_vtable(), Rc::clone(&self.compatibility), settings) }
    }

    unsafe fn put_ConnectedStatusText(&self, text: Bstr) -> Result<()> {
        self.settings.borrow_mut().connected_status_text = string_from_bstr(text)?;
        self.persistence_dirty.set(true);
        Ok(())
    }

    unsafe fn get_ConnectedStatusText(&self, text: BstrOut) -> Result<()> {
        write_bstr(text, &self.settings.borrow().connected_status_text)
    }
}

impl IMsRdpClient3_Impl for Control_Impl {
    unsafe fn get_AdvancedSettings4(&self, settings: InterfaceOut) -> Result<()> {
        unsafe { settings_object(advanced_vtable(), Rc::clone(&self.compatibility), settings) }
    }
}

impl IMsRdpClient4_Impl for Control_Impl {
    unsafe fn get_AdvancedSettings5(&self, settings: InterfaceOut) -> Result<()> {
        unsafe { settings_object(advanced_vtable(), Rc::clone(&self.compatibility), settings) }
    }
}

impl IMsRdpClient5_Impl for Control_Impl {
    unsafe fn get_TransportSettings(&self, settings: InterfaceOut) -> Result<()> {
        unsafe { settings_object(transport_vtable(), Rc::clone(&self.compatibility), settings) }
    }

    unsafe fn get_AdvancedSettings6(&self, settings: InterfaceOut) -> Result<()> {
        unsafe { settings_object(advanced_vtable(), Rc::clone(&self.compatibility), settings) }
    }

    unsafe fn GetErrorDescription(&self, disconnect_reason: u32, extended_reason: u32, message: BstrOut) -> Result<()> {
        write_bstr(message, self.disconnect_description(disconnect_reason, extended_reason))
    }

    unsafe fn get_RemoteProgram(&self, program: InterfaceOut) -> Result<()> {
        unsupported_out(program)
    }

    unsafe fn get_MsRdpClientShell(&self, shell: InterfaceOut) -> Result<()> {
        unsupported_out(shell)
    }
}

impl IMsRdpClient6_Impl for Control_Impl {
    unsafe fn get_AdvancedSettings7(&self, settings: InterfaceOut) -> Result<()> {
        unsafe { settings_object(advanced_vtable(), Rc::clone(&self.compatibility), settings) }
    }

    unsafe fn get_TransportSettings2(&self, settings: InterfaceOut) -> Result<()> {
        unsafe { settings_object(transport_vtable(), Rc::clone(&self.compatibility), settings) }
    }
}

impl IMsRdpClient7_Impl for Control_Impl {
    unsafe fn get_AdvancedSettings8(&self, settings: InterfaceOut) -> Result<()> {
        unsafe { settings_object(advanced_vtable(), Rc::clone(&self.compatibility), settings) }
    }

    unsafe fn get_TransportSettings3(&self, settings: InterfaceOut) -> Result<()> {
        unsafe { settings_object(transport_vtable(), Rc::clone(&self.compatibility), settings) }
    }

    unsafe fn GetStatusText(&self, _status: u32, text: BstrOut) -> Result<()> {
        write_bstr(text, "")
    }

    unsafe fn get_SecuredSettings3(&self, settings: InterfaceOut) -> Result<()> {
        let owner: IUnknown = self.to_interface();
        unsafe {
            settings_object_with_bridge(
                secured_vtable(),
                Rc::clone(&self.compatibility),
                Some(NativeMstscCredentialBridge {
                    _owner: owner,
                    control: self,
                }),
                settings,
            )
        }
    }

    unsafe fn get_RemoteProgram2(&self, program: InterfaceOut) -> Result<()> {
        unsupported_out(program)
    }
}

impl IMsRdpClient8_Impl for Control_Impl {
    unsafe fn SendRemoteAction(&self, action: i32) -> Result<()> {
        self.send_remote_action(action)
    }

    unsafe fn get_AdvancedSettings9(&self, settings: InterfaceOut) -> Result<()> {
        unsafe { settings_object(advanced_vtable(), Rc::clone(&self.compatibility), settings) }
    }

    unsafe fn Reconnect(&self, width: u32, height: u32, status: *mut i32) -> Result<()> {
        self.reconnect(width, height, status)
    }
}

impl IMsRdpClient9_Impl for Control_Impl {
    unsafe fn get_TransportSettings4(&self, settings: InterfaceOut) -> Result<()> {
        unsafe { settings_object(transport_vtable(), Rc::clone(&self.compatibility), settings) }
    }

    unsafe fn SyncSessionDisplaySettings(&self) -> Result<()> {
        let settings = self.settings.borrow();
        let desktop_width = u32::from(settings.desktop_width);
        let desktop_height = u32::from(settings.desktop_height);
        drop(settings);
        self.update_display_layout(DisplayLayout {
            desktop_width,
            desktop_height,
            physical_width: 0,
            physical_height: 0,
            orientation: 0,
            desktop_scale_factor: 100,
            device_scale_factor: 100,
        })
    }

    unsafe fn UpdateSessionDisplaySettings(
        &self,
        desktop_width: u32,
        desktop_height: u32,
        physical_width: u32,
        physical_height: u32,
        orientation: u32,
        desktop_scale_factor: u32,
        device_scale_factor: u32,
    ) -> Result<()> {
        self.update_display_layout(DisplayLayout {
            desktop_width,
            desktop_height,
            physical_width,
            physical_height,
            orientation,
            desktop_scale_factor,
            device_scale_factor,
        })
    }

    unsafe fn attachEvent(&self, _event_name: Bstr, _callback: *mut c_void) -> Result<()> {
        unsupported()
    }

    unsafe fn detachEvent(&self, _event_name: Bstr, _callback: *mut c_void) -> Result<()> {
        unsupported()
    }
}

impl IMsRdpClient10_Impl for Control_Impl {
    unsafe fn get_RemoteProgram3(&self, program: InterfaceOut) -> Result<()> {
        unsafe { settings_object(remote_program_vtable(), Rc::clone(&self.compatibility), program) }
    }
}

impl IMsRdpPreferredRedirectionInfo_Impl for Control_Impl {
    unsafe fn put_UseRedirectionServerName(&self, value: i16) -> Result<()> {
        if value == VARIANT_FALSE.0 {
            Ok(())
        } else {
            Err(Error::from_hresult(E_NOTIMPL))
        }
    }

    unsafe fn get_UseRedirectionServerName(&self, value: *mut i16) -> Result<()> {
        write_out(value, VARIANT_FALSE.0)
    }
}

impl IMsTscNonScriptable_Impl for Control_Impl {
    unsafe fn put_ClearTextPassword(&self, password: Bstr) -> Result<()> {
        self.settings.borrow_mut().password = Some(string_from_bstr(password)?);
        Ok(())
    }

    unsafe fn put_PortablePassword(&self, _password: Bstr) -> Result<()> {
        unsupported()
    }

    unsafe fn get_PortablePassword(&self, password: BstrOut) -> Result<()> {
        unsupported_out(password)
    }

    unsafe fn put_PortableSalt(&self, _salt: Bstr) -> Result<()> {
        unsupported()
    }

    unsafe fn get_PortableSalt(&self, salt: BstrOut) -> Result<()> {
        unsupported_out(salt)
    }

    unsafe fn put_BinaryPassword(&self, _password: Bstr) -> Result<()> {
        unsupported()
    }

    unsafe fn get_BinaryPassword(&self, password: BstrOut) -> Result<()> {
        unsupported_out(password)
    }

    unsafe fn put_BinarySalt(&self, _salt: Bstr) -> Result<()> {
        unsupported()
    }

    unsafe fn get_BinarySalt(&self, salt: BstrOut) -> Result<()> {
        unsupported_out(salt)
    }

    unsafe fn ResetPassword(&self) -> Result<()> {
        self.settings.borrow_mut().password = None;
        Ok(())
    }
}

impl IMsRdpClientNonScriptable_Impl for Control_Impl {
    unsafe fn NotifyRedirectDeviceChange(&self, _wparam: usize, _lparam: isize) -> Result<()> {
        // This control does not expose a configured ActiveX device collection, so a host device
        // change cannot affect the RDPDR state that this control advertises.
        Ok(())
    }

    unsafe fn SendKeys(&self, key_count: i32, key_up: *mut i16, key_data: *mut i32) -> Result<()> {
        self.send_keys(key_count, key_up.cast_const(), key_data.cast_const())
    }
}

impl IMsRdpClientNonScriptable2_Impl for Control_Impl {
    unsafe fn put_UIParentWindowHandle(&self, _parent: isize) -> Result<()> {
        self.credential_parent.set(HWND(_parent as *mut c_void));
        Ok(())
    }

    unsafe fn get_UIParentWindowHandle(&self, parent: *mut isize) -> Result<()> {
        write_out(parent, self.credential_parent.get().0 as isize)
    }
}

impl IMsRdpClientNonScriptable3_Impl for Control_Impl {
    unsafe fn put_ShowRedirectionWarningDialog(&self, _value: i16) -> Result<()> {
        unsupported()
    }

    unsafe fn get_ShowRedirectionWarningDialog(&self, value: *mut i16) -> Result<()> {
        unsupported_out(value)
    }

    unsafe fn put_PromptForCredentials(&self, value: i16) -> Result<()> {
        self.compatibility.borrow_mut().prompt_for_credentials = value != VARIANT_FALSE.0;
        Ok(())
    }

    unsafe fn get_PromptForCredentials(&self, value: *mut i16) -> Result<()> {
        write_out(
            value,
            if self.compatibility.borrow().prompt_for_credentials {
                VARIANT_TRUE.0
            } else {
                VARIANT_FALSE.0
            },
        )
    }

    unsafe fn put_NegotiateSecurityLayer(&self, _value: i16) -> Result<()> {
        unsupported()
    }

    unsafe fn get_NegotiateSecurityLayer(&self, value: *mut i16) -> Result<()> {
        unsupported_out(value)
    }

    unsafe fn put_EnableCredSspSupport(&self, value: i16) -> Result<()> {
        self.compatibility.borrow_mut().enable_credssp = Some(value != VARIANT_FALSE.0);
        Ok(())
    }

    unsafe fn get_EnableCredSspSupport(&self, value: *mut i16) -> Result<()> {
        write_out(
            value,
            if self.compatibility.borrow().enable_credssp.unwrap_or(true) {
                VARIANT_TRUE.0
            } else {
                VARIANT_FALSE.0
            },
        )
    }

    unsafe fn put_RedirectDynamicDrives(&self, _value: i16) -> Result<()> {
        // TODO(activex): map dynamic-drive redirection to IronRDP RDPDR support.
        unsupported()
    }

    unsafe fn get_RedirectDynamicDrives(&self, value: *mut i16) -> Result<()> {
        unsupported_out(value)
    }

    unsafe fn put_RedirectDynamicDevices(&self, _value: i16) -> Result<()> {
        // TODO(activex): map dynamic-device redirection to IronRDP RDPDR support.
        unsupported()
    }

    unsafe fn get_RedirectDynamicDevices(&self, value: *mut i16) -> Result<()> {
        unsupported_out(value)
    }

    unsafe fn get_DeviceCollection(&self, output: InterfaceOut) -> Result<()> {
        if output.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        let collection: IMsRdpDeviceCollection = EmptyDeviceCollection::new().into();
        write_out(output, collection.into_raw().cast())
    }

    unsafe fn get_DriveCollection(&self, output: InterfaceOut) -> Result<()> {
        if output.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        write_out(output, self.drive_collection.clone().into_raw().cast())
    }

    unsafe fn put_WarnAboutSendingCredentials(&self, value: i16) -> Result<()> {
        self.compatibility.borrow_mut().warn_about_sending_credentials = value != VARIANT_FALSE.0;
        Ok(())
    }

    unsafe fn get_WarnAboutSendingCredentials(&self, value: *mut i16) -> Result<()> {
        write_out(
            value,
            if self.compatibility.borrow().warn_about_sending_credentials {
                VARIANT_TRUE.0
            } else {
                VARIANT_FALSE.0
            },
        )
    }

    unsafe fn put_WarnAboutClipboardRedirection(&self, value: i16) -> Result<()> {
        self.compatibility.borrow_mut().warn_about_clipboard_redirection = value != VARIANT_FALSE.0;
        Ok(())
    }

    unsafe fn get_WarnAboutClipboardRedirection(&self, value: *mut i16) -> Result<()> {
        write_out(
            value,
            if self.compatibility.borrow().warn_about_clipboard_redirection {
                VARIANT_TRUE.0
            } else {
                VARIANT_FALSE.0
            },
        )
    }

    unsafe fn put_ConnectionBarText(&self, value: Bstr) -> Result<()> {
        let value = string_from_bstr(value)?;
        self.compatibility.borrow_mut().connection_bar_text = value;
        let window = self.connection_bar.get();
        if !window.0.is_null() && unsafe { IsWindow(Some(window)) }.as_bool() {
            self.refresh_connection_bar(window);
        }
        Ok(())
    }

    unsafe fn get_ConnectionBarText(&self, value: BstrOut) -> Result<()> {
        write_bstr(value, &self.compatibility.borrow().connection_bar_text)
    }
}

impl IMsRdpClientNonScriptable4_Impl for Control_Impl {
    unsafe fn put_RedirectionWarningType(&self, _value: i32) -> Result<()> {
        unsupported()
    }

    unsafe fn get_RedirectionWarningType(&self, value: *mut i32) -> Result<()> {
        unsupported_out(value)
    }

    unsafe fn put_MarkRdpSettingsSecure(&self, _value: i16) -> Result<()> {
        unsupported()
    }

    unsafe fn get_MarkRdpSettingsSecure(&self, value: *mut i16) -> Result<()> {
        unsupported_out(value)
    }

    unsafe fn put_PublisherCertificateChain(&self, _value: *mut VARIANT) -> Result<()> {
        unsupported()
    }

    unsafe fn get_PublisherCertificateChain(&self, value: *mut VARIANT) -> Result<()> {
        unsupported_out(value)
    }

    unsafe fn put_WarnAboutPrinterRedirection(&self, _value: i16) -> Result<()> {
        unsupported()
    }

    unsafe fn get_WarnAboutPrinterRedirection(&self, value: *mut i16) -> Result<()> {
        unsupported_out(value)
    }

    unsafe fn put_AllowCredentialSaving(&self, _value: i16) -> Result<()> {
        unsupported()
    }

    unsafe fn get_AllowCredentialSaving(&self, value: *mut i16) -> Result<()> {
        unsupported_out(value)
    }

    unsafe fn put_PromptForCredsOnClient(&self, value: i16) -> Result<()> {
        self.compatibility.borrow_mut().prompt_for_credentials = value != VARIANT_FALSE.0;
        Ok(())
    }

    unsafe fn get_PromptForCredsOnClient(&self, value: *mut i16) -> Result<()> {
        write_out(
            value,
            if self.compatibility.borrow().prompt_for_credentials {
                VARIANT_TRUE.0
            } else {
                VARIANT_FALSE.0
            },
        )
    }

    unsafe fn put_LaunchedViaClientShellInterface(&self, _value: i16) -> Result<()> {
        unsupported()
    }

    unsafe fn get_LaunchedViaClientShellInterface(&self, value: *mut i16) -> Result<()> {
        unsupported_out(value)
    }

    unsafe fn put_TrustedZoneSite(&self, _value: i16) -> Result<()> {
        unsupported()
    }

    unsafe fn get_TrustedZoneSite(&self, value: *mut i16) -> Result<()> {
        unsupported_out(value)
    }
}

impl IMsRdpClientNonScriptable5_Impl for Control_Impl {
    unsafe fn put_UseMultimon(&self, value: i16) -> Result<()> {
        let use_multimon = normalize_variant_bool(value)? == VARIANT_TRUE.0;
        let mut compatibility = self.compatibility.borrow_mut();
        active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
        if use_multimon {
            local_monitor_topology()?;
        }
        compatibility.use_multimon = use_multimon;
        Ok(())
    }

    unsafe fn get_UseMultimon(&self, value: *mut i16) -> Result<()> {
        write_out(
            value,
            if self.compatibility.borrow().use_multimon {
                VARIANT_TRUE.0
            } else {
                VARIANT_FALSE.0
            },
        )
    }

    unsafe fn get_RemoteMonitorCount(&self, count: *mut u32) -> Result<()> {
        let monitor_count = if self.state.get() == ConnectionState::Connected {
            self.active_monitor_topology.borrow().as_ref().map_or_else(
                || u32::from(self.remote_size.get().is_some()),
                |topology| topology.monitors.len() as u32,
            )
        } else {
            0
        };
        write_out(count, monitor_count)
    }

    unsafe fn GetRemoteMonitorsBoundingBox(
        &self,
        left: *mut i32,
        top: *mut i32,
        right: *mut i32,
        bottom: *mut i32,
    ) -> Result<()> {
        match self.remote_monitor_bounds() {
            Ok((remote_left, remote_top, remote_right, remote_bottom)) => {
                write_out(left, remote_left)?;
                write_out(top, remote_top)?;
                write_out(right, remote_right)?;
                write_out(bottom, remote_bottom)
            }
            Err(error) => {
                write_out(left, 0)?;
                write_out(top, 0)?;
                write_out(right, 0)?;
                write_out(bottom, 0)?;
                Err(error)
            }
        }
    }

    unsafe fn get_RemoteMonitorLayoutMatchesLocal(&self, value: *mut i16) -> Result<()> {
        let matches_local = self.state.get() == ConnectionState::Connected
            && self
                .active_monitor_topology
                .borrow()
                .as_ref()
                .is_some_and(|topology| local_monitor_topology().is_ok_and(|local| local == *topology));
        write_out(value, if matches_local { VARIANT_TRUE.0 } else { VARIANT_FALSE.0 })
    }

    unsafe fn put_DisableConnectionBar(&self, value: i16) -> Result<()> {
        self.compatibility.borrow_mut().connection_bar_disabled = value != VARIANT_FALSE.0;
        self.update_connection_bar();
        Ok(())
    }

    unsafe fn put_DisableRemoteAppCapsCheck(&self, _value: i16) -> Result<()> {
        unsupported()
    }

    unsafe fn get_DisableRemoteAppCapsCheck(&self, value: *mut i16) -> Result<()> {
        unsupported_out(value)
    }

    unsafe fn put_WarnAboutDirectXRedirection(&self, _value: i16) -> Result<()> {
        unsupported()
    }

    unsafe fn get_WarnAboutDirectXRedirection(&self, value: *mut i16) -> Result<()> {
        unsupported_out(value)
    }

    unsafe fn put_AllowPromptingForCredentials(&self, value: i16) -> Result<()> {
        self.compatibility.borrow_mut().prompt_for_credentials = value != VARIANT_FALSE.0;
        Ok(())
    }

    unsafe fn get_AllowPromptingForCredentials(&self, value: *mut i16) -> Result<()> {
        write_out(
            value,
            if self.compatibility.borrow().prompt_for_credentials {
                VARIANT_TRUE.0
            } else {
                VARIANT_FALSE.0
            },
        )
    }
}

impl IMsRdpClientNonScriptable6_Impl for Control_Impl {
    unsafe fn SendLocation2D(&self, _latitude: f64, _longitude: f64) -> Result<()> {
        // TODO(activex): forward client location through an IronRDP location-redirection implementation.
        unsupported()
    }

    unsafe fn SendLocation3D(&self, _latitude: f64, _longitude: f64, _altitude: i32) -> Result<()> {
        // TODO(activex): forward client location through an IronRDP location-redirection implementation.
        unsupported()
    }
}

impl IMsRdpClientNonScriptable7_Impl for Control_Impl {
    unsafe fn get_CameraRedirConfigCollection(&self, output: InterfaceOut) -> Result<()> {
        if output.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        let collection: IMsRdpCameraRedirConfigCollection = EmptyCameraRedirConfigCollection::new().into();
        write_out(output, collection.into_raw().cast())
    }

    unsafe fn DisableDpiCursorScalingForProcess(&self) -> Result<()> {
        // The IronRDP renderer maps pointer coordinates itself, so no process-wide cursor policy is
        // required. mstsc requests this during its pre-connection UI initialization.
        Ok(())
    }

    unsafe fn get_Clipboard(&self, output: InterfaceOut) -> Result<()> {
        if output.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        let clipboard: IMsRdpClipboard = ClipboardCapabilities::new(Rc::clone(&self.clipboard_state)).into();
        write_out(output, clipboard.into_raw().cast())
    }
}

impl IMsRdpClientNonScriptable8_Impl for Control_Impl {
    unsafe fn get_CorrelationId(&self, correlation_id: *mut GUID) -> Result<()> {
        trace_host_call("IMsRdpClientNonScriptable8::get_CorrelationId");
        write_out(correlation_id, CLSID_IRONRDP_ACTIVEX)
    }

    unsafe fn StartWorkspaceExtension(
        &self,
        _is_web_hosted: i16,
        _workspace_id: Bstr,
        _publisher_thumbprint: *const u8,
        _publisher_thumbprint_length: u32,
    ) -> Result<()> {
        // IronRDP does not implement the Microsoft workspace extension protocol.
        trace_host_call("IMsRdpClientNonScriptable8::StartWorkspaceExtension");
        unsupported()
    }

    unsafe fn put_SupportsWorkspaceReconnect(&self, _value: i16) -> Result<()> {
        trace_host_call("IMsRdpClientNonScriptable8::put_SupportsWorkspaceReconnect");
        Ok(())
    }
}

impl IMsRdpExtendedSettings_Impl for Control_Impl {
    unsafe fn put_Property(&self, name: Bstr, value: *mut VARIANT) -> Result<()> {
        trace_host_call("IMsRdpExtendedSettings::put_Property");
        self.remember_callback_owner(self);
        let name = string_from_bstr(name)?;
        if name.eq_ignore_ascii_case("ZoomLevel") {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let zoom_level = variant_zoom_level(unsafe { &*value }, ptr::null_mut())?;
            if !(10..=500).contains(&zoom_level) {
                return Err(Error::new(E_INVALIDARG, "zoom level must be between 10 and 500"));
            }
            let renderer_window = {
                let mut compatibility = self.compatibility.borrow_mut();
                compatibility.zoom_level = zoom_level;
                mark_compatibility_persistence_dirty(&compatibility);
                compatibility.renderer_window
            };
            trace_host_call("IMsRdpExtendedSettings::put_ZoomLevel");
            invalidate_renderer(renderer_window);
            return Ok(());
        }
        if native_mstsc_credential_bridge_enabled() {
            let (preflight, intercept) = self
                .native_mstsc_preflight
                .get()
                .observe_extended_setting(self.state.get() == ConnectionState::Disconnected, &name);
            self.native_mstsc_preflight.set(preflight);
            if intercept {
                let started = match self.prompt_for_credentials() {
                    Ok(started) => started,
                    Err(error) => {
                        tracing::warn!(code = error.code().0, "Native mstsc credential prompt failed");
                        false
                    }
                };
                if !started {
                    self.native_mstsc_preflight.set(NativeMstscPreflight::Idle);
                }
                return Err(Error::from_hresult(E_INVALIDARG));
            }
            if name.is_empty() {
                return Ok(());
            }
        }
        if name.eq_ignore_ascii_case(ACTIVEX_RDCLEANPATH_URL_PROPERTY) {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let url = variant_string(unsafe { &*value }, ptr::null_mut())?;
            let compatibility = self.compatibility.borrow();
            active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
            drop(compatibility);
            self.rdcleanpath_settings.borrow_mut().set_url(url)?;
            trace_host_call("IMsRdpExtendedSettings::put_RDCleanPathUrl");
            return Ok(());
        }
        if name.eq_ignore_ascii_case(ACTIVEX_RDCLEANPATH_TOKEN_PROPERTY) {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let token = variant_string(unsafe { &*value }, ptr::null_mut())?;
            let compatibility = self.compatibility.borrow();
            active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
            drop(compatibility);
            self.rdcleanpath_settings.borrow_mut().set_token(token)?;
            trace_host_call("IMsRdpExtendedSettings::put_RDCleanPathToken");
            return Ok(());
        }
        if name.eq_ignore_ascii_case("ClientDeviceName") {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let client_name = variant_string(unsafe { &*value }, ptr::null_mut())?;
            if client_name.encode_utf16().count() > 15 {
                return Err(Error::new(
                    E_INVALIDARG,
                    "client device name must be at most 15 UTF-16 code units",
                ));
            }
            let mut compatibility = self.compatibility.borrow_mut();
            compatibility.client_name = (!client_name.is_empty()).then_some(client_name);
            mark_compatibility_persistence_dirty(&compatibility);
            trace_host_call("IMsRdpExtendedSettings::put_ClientDeviceName");
            return Ok(());
        }
        if name.eq_ignore_ascii_case("DisableUdpTransport") {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            if !variant_bool(unsafe { &*value }, ptr::null_mut())? {
                return Err(Error::from_hresult(E_NOTIMPL));
            }
            // IronRDP's ActiveX client exposes no UDP transport, so the only truthful value is true.
            trace_host_call("IMsRdpExtendedSettings::put_DisableUdpTransport");
            return Ok(());
        }
        if name.eq_ignore_ascii_case(ACTIVEX_ENABLE_TLS_PROPERTY) {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let enable_tls = variant_bool(unsafe { &*value }, ptr::null_mut())?;
            let mut compatibility = self.compatibility.borrow_mut();
            active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
            compatibility.enable_tls = Some(enable_tls);
            trace_host_call("IMsRdpExtendedSettings::put_IronRdpEnableTls");
            return Ok(());
        }
        if name.eq_ignore_ascii_case(ACTIVEX_AUTOLOGON_PROPERTY) {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let autologon = variant_bool(unsafe { &*value }, ptr::null_mut())?;
            let mut compatibility = self.compatibility.borrow_mut();
            active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
            compatibility.autologon = Some(autologon);
            trace_host_call("IMsRdpExtendedSettings::put_IronRdpAutoLogon");
            return Ok(());
        }
        if name.eq_ignore_ascii_case(ACTIVEX_REMOTE_PROGRAM_MODE_PROPERTY) {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let remote_program_mode = variant_bool(unsafe { &*value }, ptr::null_mut())?;
            let compatibility = self.compatibility.borrow();
            active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
            drop(compatibility);
            self.remote_application.borrow_mut().enabled = remote_program_mode;
            trace_host_call("IMsRdpExtendedSettings::put_IronRdpRemoteProgramMode");
            return Ok(());
        }
        if name.eq_ignore_ascii_case(ACTIVEX_REMOTE_APPLICATION_PROGRAM_PROPERTY) {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let remote_application_program =
                validate_activex_extended_string(variant_string(unsafe { &*value }, ptr::null_mut())?)?;
            let compatibility = self.compatibility.borrow();
            active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
            drop(compatibility);
            self.remote_application.borrow_mut().program = remote_application_program;
            trace_host_call("IMsRdpExtendedSettings::put_IronRdpRemoteApplicationProgram");
            return Ok(());
        }
        if name.eq_ignore_ascii_case(ACTIVEX_REMOTE_APPLICATION_ARGS_PROPERTY) {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let remote_application_args =
                validate_activex_extended_string(variant_string(unsafe { &*value }, ptr::null_mut())?)?;
            let compatibility = self.compatibility.borrow();
            active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
            drop(compatibility);
            self.remote_application.borrow_mut().arguments = remote_application_args;
            trace_host_call("IMsRdpExtendedSettings::put_IronRdpRemoteApplicationArgs");
            return Ok(());
        }
        if name.eq_ignore_ascii_case(ACTIVEX_DESKTOP_SCALE_FACTOR_PROPERTY) {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let desktop_scale_factor = u32::try_from(variant_i32_value(unsafe { &*value }, ptr::null_mut())?)
                .map_err(|_| Error::from_hresult(E_INVALIDARG))?;
            if desktop_scale_factor != 0 && !(100..=500).contains(&desktop_scale_factor) {
                return Err(Error::new(
                    E_INVALIDARG,
                    "desktop scale factor must be zero or between 100 and 500",
                ));
            }
            let mut compatibility = self.compatibility.borrow_mut();
            active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
            compatibility.desktop_scale_factor = (desktop_scale_factor != 0).then_some(desktop_scale_factor);
            trace_host_call("IMsRdpExtendedSettings::put_IronRdpDesktopScaleFactor");
            return Ok(());
        }
        if name.eq_ignore_ascii_case(ACTIVEX_COMPRESSION_LEVEL_PROPERTY) {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let compression_level = u32::try_from(variant_i32_value(unsafe { &*value }, ptr::null_mut())?)
                .map_err(|_| Error::from_hresult(E_INVALIDARG))?;
            if compression_level > 3 {
                return Err(Error::new(
                    E_INVALIDARG,
                    "compression level must be between zero and three",
                ));
            }
            let mut compatibility = self.compatibility.borrow_mut();
            active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
            compatibility.compression_level = Some(compression_level);
            trace_host_call("IMsRdpExtendedSettings::put_IronRdpCompressionLevel");
            return Ok(());
        }
        if name.eq_ignore_ascii_case(ACTIVEX_CLIENT_BUILD_PROPERTY) {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let client_build = u32::try_from(variant_i32_value(unsafe { &*value }, ptr::null_mut())?)
                .map_err(|_| Error::from_hresult(E_INVALIDARG))?;
            let mut compatibility = self.compatibility.borrow_mut();
            active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
            compatibility.client_build = client_build;
            trace_host_call("IMsRdpExtendedSettings::put_IronRdpClientBuild");
            return Ok(());
        }
        if name.eq_ignore_ascii_case(ACTIVEX_CLIENT_DIRECTORY_PROPERTY) {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let client_dir = validate_activex_extended_string(variant_string(unsafe { &*value }, ptr::null_mut())?)?;
            let mut compatibility = self.compatibility.borrow_mut();
            active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
            compatibility.client_dir = client_dir;
            trace_host_call("IMsRdpExtendedSettings::put_IronRdpClientDirectory");
            return Ok(());
        }
        if name.eq_ignore_ascii_case(ACTIVEX_IME_FILE_NAME_PROPERTY) {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let ime_file_name = validate_activex_extended_string(variant_string(unsafe { &*value }, ptr::null_mut())?)?;
            let mut compatibility = self.compatibility.borrow_mut();
            active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
            compatibility.ime_file_name = ime_file_name;
            trace_host_call("IMsRdpExtendedSettings::put_IronRdpImeFileName");
            return Ok(());
        }
        if name.eq_ignore_ascii_case(ACTIVEX_DIGITAL_PRODUCT_ID_PROPERTY) {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let digital_product_id =
                validate_activex_extended_string(variant_string(unsafe { &*value }, ptr::null_mut())?)?;
            let mut compatibility = self.compatibility.borrow_mut();
            active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
            compatibility.digital_product_id = digital_product_id;
            trace_host_call("IMsRdpExtendedSettings::put_IronRdpDigitalProductId");
            return Ok(());
        }
        if name.eq_ignore_ascii_case(ACTIVEX_FAKE_EVENTS_INTERVAL_PROPERTY) {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let minutes = u32::try_from(variant_i32_value(unsafe { &*value }, ptr::null_mut())?)
                .map_err(|_| Error::from_hresult(E_INVALIDARG))?;
            if minutes > 1_440 {
                return Err(Error::new(
                    E_INVALIDARG,
                    "fake events interval must be at most 1440 minutes",
                ));
            }
            let mut compatibility = self.compatibility.borrow_mut();
            active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
            compatibility.fake_events_interval_minutes = (minutes != 0).then_some(minutes);
            trace_host_call("IMsRdpExtendedSettings::put_IronRdpFakeEventsIntervalMinutes");
            return Ok(());
        }
        if name.eq_ignore_ascii_case(ACTIVEX_DVC_PLUGIN_PATHS_PROPERTY) {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            if !activex_dvc_plugins_enabled() {
                return Err(Error::from_hresult(E_NOTIMPL));
            }
            if self.state.get() != ConnectionState::Disconnected
                || self.compatibility.borrow().connection_settings_sealed
            {
                return Err(Error::from_hresult(E_UNEXPECTED));
            }

            let paths = validated_dvc_plugin_paths(&variant_string(unsafe { &*value }, ptr::null_mut())?)?;
            self.compatibility.borrow_mut().dvc_plugin_paths = paths;
            trace_host_call("IMsRdpExtendedSettings::put_IronRdpDvcPluginPaths");
            return Ok(());
        }
        if name.eq_ignore_ascii_case("RedirectWebAuthn") {
            if value.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let redirect_webauthn = variant_bool(unsafe { &*value }, ptr::null_mut())?;
            let mut compatibility = self.compatibility.borrow_mut();
            active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
            compatibility.redirect_webauthn = redirect_webauthn;
            mark_compatibility_persistence_dirty(&compatibility);
            trace_host_call("IMsRdpExtendedSettings::put_RedirectWebAuthn");
            return Ok(());
        }

        Err(Error::from_hresult(E_NOTIMPL))
    }

    unsafe fn get_Property(&self, name: Bstr, value: *mut VARIANT) -> Result<()> {
        trace_host_call("IMsRdpExtendedSettings::get_Property");
        let name = string_from_bstr(name)?;
        if name.eq_ignore_ascii_case("ZoomLevel") {
            trace_host_call("IMsRdpExtendedSettings::get_ZoomLevel");
            return write_out(value, variant_i32(self.compatibility.borrow().zoom_level));
        }
        if name.eq_ignore_ascii_case(ACTIVEX_RDCLEANPATH_URL_PROPERTY) {
            let compatibility = self.compatibility.borrow();
            active_x_connection_settings_mutable(self.state.get(), &compatibility)?;
            drop(compatibility);
            let url = self.rdcleanpath_settings.borrow().url.clone().unwrap_or_default();
            trace_host_call("IMsRdpExtendedSettings::get_RDCleanPathUrl");
            return write_out(value, variant_bstr(url));
        }
        if name.eq_ignore_ascii_case(ACTIVEX_RDCLEANPATH_TOKEN_PROPERTY) {
            write_out(value, VARIANT::default())?;
            return Err(Error::from_hresult(E_NOTIMPL));
        }
        if name.eq_ignore_ascii_case("ClientDeviceName") {
            let client_name = self
                .compatibility
                .borrow()
                .client_name
                .clone()
                .unwrap_or_else(|| "IronRDP ActiveX".to_owned());
            trace_host_call("IMsRdpExtendedSettings::get_ClientDeviceName");
            return write_out(value, variant_bstr(client_name));
        }
        if name.eq_ignore_ascii_case("DisableUdpTransport") {
            trace_host_call("IMsRdpExtendedSettings::get_DisableUdpTransport");
            return write_out(value, variant_bool_value(true));
        }
        if name.eq_ignore_ascii_case(ACTIVEX_ENABLE_TLS_PROPERTY) {
            trace_host_call("IMsRdpExtendedSettings::get_IronRdpEnableTls");
            return write_out(
                value,
                variant_bool_value(self.compatibility.borrow().enable_tls.unwrap_or(true)),
            );
        }
        if name.eq_ignore_ascii_case(ACTIVEX_AUTOLOGON_PROPERTY) {
            trace_host_call("IMsRdpExtendedSettings::get_IronRdpAutoLogon");
            return write_out(
                value,
                variant_bool_value(self.compatibility.borrow().autologon.unwrap_or(false)),
            );
        }
        if name.eq_ignore_ascii_case(ACTIVEX_REMOTE_PROGRAM_MODE_PROPERTY) {
            trace_host_call("IMsRdpExtendedSettings::get_IronRdpRemoteProgramMode");
            return write_out(value, variant_bool_value(self.remote_application.borrow().enabled));
        }
        if name.eq_ignore_ascii_case(ACTIVEX_REMOTE_APPLICATION_PROGRAM_PROPERTY) {
            trace_host_call("IMsRdpExtendedSettings::get_IronRdpRemoteApplicationProgram");
            return write_out(value, variant_bstr(self.remote_application.borrow().program.clone()));
        }
        if name.eq_ignore_ascii_case(ACTIVEX_REMOTE_APPLICATION_ARGS_PROPERTY) {
            trace_host_call("IMsRdpExtendedSettings::get_IronRdpRemoteApplicationArgs");
            return write_out(value, variant_bstr(self.remote_application.borrow().arguments.clone()));
        }
        if name.eq_ignore_ascii_case(ACTIVEX_DESKTOP_SCALE_FACTOR_PROPERTY) {
            trace_host_call("IMsRdpExtendedSettings::get_IronRdpDesktopScaleFactor");
            return write_out(
                value,
                variant_i32(
                    self.compatibility
                        .borrow()
                        .desktop_scale_factor
                        .map_or(0, |scale| scale as i32),
                ),
            );
        }
        if name.eq_ignore_ascii_case(ACTIVEX_COMPRESSION_LEVEL_PROPERTY) {
            trace_host_call("IMsRdpExtendedSettings::get_IronRdpCompressionLevel");
            return write_out(
                value,
                variant_i32(
                    self.compatibility
                        .borrow()
                        .compression_level
                        .map_or(-1, |level| level as i32),
                ),
            );
        }
        if name.eq_ignore_ascii_case(ACTIVEX_CLIENT_BUILD_PROPERTY) {
            trace_host_call("IMsRdpExtendedSettings::get_IronRdpClientBuild");
            return write_out(value, variant_i32(self.compatibility.borrow().client_build as i32));
        }
        if name.eq_ignore_ascii_case(ACTIVEX_CLIENT_DIRECTORY_PROPERTY) {
            trace_host_call("IMsRdpExtendedSettings::get_IronRdpClientDirectory");
            return write_out(value, variant_bstr(self.compatibility.borrow().client_dir.clone()));
        }
        if name.eq_ignore_ascii_case(ACTIVEX_IME_FILE_NAME_PROPERTY) {
            trace_host_call("IMsRdpExtendedSettings::get_IronRdpImeFileName");
            return write_out(value, variant_bstr(self.compatibility.borrow().ime_file_name.clone()));
        }
        if name.eq_ignore_ascii_case(ACTIVEX_DIGITAL_PRODUCT_ID_PROPERTY) {
            trace_host_call("IMsRdpExtendedSettings::get_IronRdpDigitalProductId");
            return write_out(
                value,
                variant_bstr(self.compatibility.borrow().digital_product_id.clone()),
            );
        }
        if name.eq_ignore_ascii_case(ACTIVEX_FAKE_EVENTS_INTERVAL_PROPERTY) {
            trace_host_call("IMsRdpExtendedSettings::get_IronRdpFakeEventsIntervalMinutes");
            return write_out(
                value,
                variant_i32(
                    self.compatibility
                        .borrow()
                        .fake_events_interval_minutes
                        .map_or(0, |minutes| minutes as i32),
                ),
            );
        }
        if name.eq_ignore_ascii_case(ACTIVEX_DVC_PLUGIN_PATHS_PROPERTY) {
            if !activex_dvc_plugins_enabled() {
                write_out(value, VARIANT::default())?;
                return Err(Error::from_hresult(E_NOTIMPL));
            }
            let paths = self
                .compatibility
                .borrow()
                .dvc_plugin_paths
                .iter()
                .map(|path| path.to_string_lossy())
                .collect::<Vec<_>>()
                .join(";");
            trace_host_call("IMsRdpExtendedSettings::get_IronRdpDvcPluginPaths");
            return write_out(value, variant_bstr(paths));
        }
        if name.eq_ignore_ascii_case("RedirectWebAuthn") {
            trace_host_call("IMsRdpExtendedSettings::get_RedirectWebAuthn");
            return write_out(value, variant_bool_value(self.compatibility.borrow().redirect_webauthn));
        }
        write_out(value, VARIANT::default())?;
        Err(Error::from_hresult(E_NOTIMPL))
    }
}

impl IViewObject_Impl for Control_Impl {
    fn Draw(
        &self,
        aspect: DVASPECT,
        index: i32,
        _aspect_info: *mut c_void,
        _target_device: *const DVTARGETDEVICE,
        _target_device_context: HDC,
        _draw_context: HDC,
        _bounds: *const RECTL,
        _window_bounds: *const RECTL,
        _continue_callback: isize,
        _continue_data: usize,
    ) -> Result<()> {
        ensure_content_view(aspect, index)?;
        // The windowed renderer is the authoritative presentation path. It cannot safely paint a
        // detached HDC without duplicating its live frame and clip lifecycle.
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn GetColorSet(
        &self,
        aspect: DVASPECT,
        index: i32,
        _aspect_info: *mut c_void,
        _target_device: *const DVTARGETDEVICE,
        _target_device_context: HDC,
        palette: *mut *mut windows::Win32::Graphics::Gdi::LOGPALETTE,
    ) -> Result<()> {
        ensure_content_view(aspect, index)?;
        if palette.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        unsafe {
            palette.write(ptr::null_mut());
        }
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn Freeze(&self, aspect: DVASPECT, index: i32, _aspect_info: *mut c_void, freeze: *mut u32) -> Result<()> {
        ensure_content_view(aspect, index)?;
        if freeze.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        unsafe {
            freeze.write(0);
        }
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn Unfreeze(&self, _freeze: u32) -> Result<()> {
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn SetAdvise(&self, aspects: DVASPECT, flags: u32, sink: Ref<'_, IAdviseSink>) -> Result<()> {
        if sink.is_null() {
            self.view_advise.borrow_mut().take();
            return Ok(());
        }
        let sink = sink.ok()?;
        ensure_content_aspect(aspects)?;
        *self.view_advise.borrow_mut() = Some(ViewAdvise {
            aspects,
            flags,
            sink: sink.clone(),
        });
        Ok(())
    }

    fn GetAdvise(&self, aspects: *mut u32, flags: *mut u32, sink: windows_core::OutRef<'_, IAdviseSink>) -> Result<()> {
        let advise = self.view_advise.borrow();
        if !aspects.is_null() {
            unsafe {
                aspects.write(advise.as_ref().map_or(0, |advise| advise.aspects.0));
            }
        }
        if !flags.is_null() {
            unsafe {
                flags.write(advise.as_ref().map_or(0, |advise| advise.flags));
            }
        }
        if !sink.is_null() {
            sink.write(advise.as_ref().map(|advise| advise.sink.clone()))?;
        }
        Ok(())
    }
}

impl IViewObject2_Impl for Control_Impl {
    fn GetExtent(&self, aspect: DVASPECT, index: i32, _target_device: *const DVTARGETDEVICE) -> Result<SIZE> {
        ensure_content_view(aspect, index)?;
        Ok(self.activex_extent.get())
    }
}

impl IViewObjectEx_Impl for Control_Impl {
    fn GetRect(&self, aspect: u32) -> Result<RECTL> {
        ensure_content_aspect(DVASPECT(aspect))?;
        Ok(view_extent_rect(self.activex_extent.get()))
    }

    fn GetViewStatus(&self) -> Result<u32> {
        Ok((VIEWSTATUS_OPAQUE.0 | VIEWSTATUS_SOLIDBKGND.0) as u32)
    }

    fn QueryHitPoint(&self, aspect: u32, bounds: *const RECT, point: &POINT, _close_hint: i32) -> Result<u32> {
        ensure_content_aspect(DVASPECT(aspect))?;
        let bounds = unsafe { bounds.as_ref() }.ok_or_else(|| Error::from_hresult(E_POINTER))?;
        let bounds = RECTL {
            left: bounds.left,
            top: bounds.top,
            right: bounds.right,
            bottom: bounds.bottom,
        };
        Ok(if view_rect_contains_point(bounds, *point) {
            HITRESULT_HIT.0 as u32
        } else {
            HITRESULT_OUTSIDE.0 as u32
        })
    }

    fn QueryHitRect(&self, aspect: u32, bounds: *const RECT, location: *const RECT, _close_hint: i32) -> Result<u32> {
        ensure_content_aspect(DVASPECT(aspect))?;
        let bounds = unsafe { bounds.as_ref() }.ok_or_else(|| Error::from_hresult(E_POINTER))?;
        let location = unsafe { location.as_ref() }.ok_or_else(|| Error::from_hresult(E_POINTER))?;
        let bounds = RECTL {
            left: bounds.left,
            top: bounds.top,
            right: bounds.right,
            bottom: bounds.bottom,
        };
        let location = RECTL {
            left: location.left,
            top: location.top,
            right: location.right,
            bottom: location.bottom,
        };
        Ok(if view_rects_intersect(bounds, location) {
            HITRESULT_HIT.0 as u32
        } else {
            HITRESULT_OUTSIDE.0 as u32
        })
    }

    fn GetNaturalExtent(
        &self,
        aspect: DVASPECT,
        index: i32,
        _target_device: *const DVTARGETDEVICE,
        _target_device_context: HDC,
        _extent_info: *const DVEXTENTINFO,
    ) -> Result<SIZE> {
        ensure_content_view(aspect, index)?;
        Ok(self.activex_extent.get())
    }
}

impl IOleObject_Impl for Control_Impl {
    fn SetClientSite(&self, site: Ref<'_, IOleClientSite>) -> Result<()> {
        trace_host_call("IOleObject::SetClientSite");
        if site.is_some() {
            self.remember_callback_owner(self);
        } else {
            self.release_input();
        }
        *self.client_site.borrow_mut() = site.cloned();
        Ok(())
    }

    fn GetClientSite(&self) -> Result<IOleClientSite> {
        trace_host_call("IOleObject::GetClientSite");
        self.client_site
            .borrow()
            .clone()
            .ok_or_else(|| Error::from_hresult(E_FAIL))
    }

    fn SetHostNames(&self, _container_application: &PCWSTR, _container_object: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn Close(&self, _save_option: &OLECLOSE) -> Result<()> {
        let _ = self.stop_connection();
        let client_site = self.client_site.borrow().clone();
        if let Some(site) = &client_site
            && let Ok(in_place_site) = site.cast::<IOleInPlaceSite>()
        {
            unsafe {
                in_place_site.OnUIDeactivate(false)?;
                in_place_site.OnInPlaceDeactivate()?;
            }
        }
        self.destroy_activex_window()?;
        self.notify_ole_advise_close();
        if let Some(site) = client_site {
            unsafe {
                site.OnShowWindow(false)?;
            }
        }
        *self.client_site.borrow_mut() = None;
        Ok(())
    }

    fn SetMoniker(
        &self,
        _which_moniker: &OLEWHICHMK,
        _moniker: Ref<'_, windows::Win32::System::Com::IMoniker>,
    ) -> Result<()> {
        unsupported()
    }

    fn GetMoniker(
        &self,
        _assign: &OLEGETMONIKER,
        _which_moniker: &OLEWHICHMK,
    ) -> Result<windows::Win32::System::Com::IMoniker> {
        unsupported_value()
    }

    fn InitFromData(
        &self,
        _data_object: Ref<'_, IDataObject>,
        _creation: windows_core::BOOL,
        _reserved: u32,
    ) -> Result<()> {
        unsupported()
    }

    fn GetClipboardData(&self, reserved: u32) -> Result<IDataObject> {
        if reserved != 0 {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        if !self.clipboard_state.is_available() {
            return Err(Error::from_hresult(OLE_E_NOTRUNNING));
        }
        ClipboardDataObject::snapshot().map(Into::into)
    }

    fn DoVerb(
        &self,
        verb: i32,
        _message: *const windows::Win32::UI::WindowsAndMessaging::MSG,
        active_site: Ref<'_, IOleClientSite>,
        _index: i32,
        parent: HWND,
        position: *const RECT,
    ) -> Result<()> {
        trace_host_call("IOleObject::DoVerb");
        self.remember_callback_owner(self);
        let action = ole_verb_action(verb)?;
        let active_site = active_site.cloned();
        let recorded_site = self.client_site.borrow().clone();
        let client_site = match (active_site, recorded_site) {
            (Some(active_site), Some(recorded_site)) => {
                let active_identity: IUnknown = active_site.cast()?;
                let recorded_identity: IUnknown = recorded_site.cast()?;
                if active_identity.as_raw() != recorded_identity.as_raw() {
                    return Err(Error::from_hresult(E_UNEXPECTED));
                }
                recorded_site
            }
            (Some(active_site), None) => {
                *self.client_site.borrow_mut() = Some(active_site.clone());
                active_site
            }
            (None, Some(recorded_site)) => recorded_site,
            (None, None) => return Err(Error::from_hresult(E_FAIL)),
        };
        trace_host_call("IOleObject::DoVerb: client site");

        match action {
            OleVerbAction::DiscardUndoState => return Ok(()),
            OleVerbAction::Hide => {
                self.destroy_activex_window()?;
                if let Ok(in_place_site) = client_site.cast::<IOleInPlaceSite>() {
                    unsafe {
                        in_place_site.OnUIDeactivate(false)?;
                        in_place_site.OnInPlaceDeactivate()?;
                    }
                }
                unsafe {
                    client_site.OnShowWindow(false)?;
                }
                return Ok(());
            }
            OleVerbAction::Activate => {}
        }

        if parent.0.is_null() || position.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }

        let in_place_site: Option<IOleInPlaceSite> = client_site.cast().ok();
        if let Some(in_place_site) = &in_place_site {
            trace_host_call("IOleObject::DoVerb: in-place site");
            unsafe {
                in_place_site.CanInPlaceActivate()?;
                trace_host_call("IOleObject::DoVerb: CanInPlaceActivate");
                in_place_site.OnInPlaceActivate()?;
                trace_host_call("IOleObject::DoVerb: OnInPlaceActivate");
            }
        }

        let rect = unsafe { *position };
        let width = (rect.right - rect.left).max(1);
        let height = (rect.bottom - rect.top).max(1);
        if self.activex_window.get().0.is_null() {
            trace_host_call("IOleObject::DoVerb: create renderer");
            acquire_renderer_class()?;
            self.renderer_class_acquired.set(true);
            let module = match com::retain_module_reference() {
                Ok(module) => module,
                Err(error) => {
                    self.renderer_class_acquired.set(false);
                    release_renderer_class();
                    return Err(error);
                }
            };
            let instance = match unsafe { GetModuleHandleW(None) } {
                Ok(instance) => instance,
                Err(error) => {
                    com::release_module_reference(module);
                    self.renderer_class_acquired.set(false);
                    release_renderer_class();
                    return Err(error);
                }
            };
            let callback_context = Rc::new(ControlWindowContext {
                control: self as *const Control_Impl,
                module,
                closing: AtomicBool::new(false),
                window_reference_released: Cell::new(false),
                orphaned: AtomicBool::new(false),
            });
            let callback_context_raw = Rc::into_raw(Rc::clone(&callback_context));
            let window = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("IronRDP.ActiveX.Renderer"),
                    w!(""),
                    WS_CHILD | WS_VISIBLE,
                    rect.left,
                    rect.top,
                    width,
                    height,
                    Some(parent),
                    None,
                    Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                    Some(callback_context_raw.cast()),
                )
            };
            let window = match window {
                Ok(window) => {
                    drop(callback_context);
                    window
                }
                Err(error) => {
                    if !callback_context.window_reference_released.get() {
                        unsafe {
                            drop(Rc::from_raw(callback_context_raw));
                        }
                    }
                    drop(callback_context);
                    self.renderer_class_acquired.set(false);
                    release_renderer_class();
                    return Err(error);
                }
            };
            self.activex_window.set(window);
            self.compatibility.borrow_mut().renderer_window = window;
            if let Err(error) = self.apply_activex_window_clip(window, rect) {
                if let Err(cleanup_error) = self.destroy_activex_window() {
                    tracing::error!(
                        ?cleanup_error,
                        "Unable to destroy ActiveX child window after clipping setup failure"
                    );
                }
                return Err(error);
            }
            trace_host_call("IOleObject::DoVerb: renderer created");
            self.start_native_mstsc_layout_observer(window);
            self.update_connection_health_window();
        } else {
            self.resize_activex_window(rect)?;
            self.update_connection_health_window();
        }
        self.activex_rect.set(rect);

        let activation_result = (|| unsafe {
            if let Some(in_place_site) = &in_place_site {
                in_place_site.OnUIActivate()?;
                trace_host_call("IOleObject::DoVerb: OnUIActivate");
            }
            client_site.ShowObject()?;
            trace_host_call("IOleObject::DoVerb: ShowObject");
            client_site.OnShowWindow(true)
        })();
        if let Err(error) = activation_result {
            if let Err(cleanup_error) = self.destroy_activex_window() {
                tracing::error!(
                    ?cleanup_error,
                    "Unable to destroy ActiveX child window after activation failure"
                );
            }
            return Err(error);
        }

        self.update_connection_bar();
        trace_host_call("IOleObject::DoVerb: success");
        Ok(())
    }

    fn EnumVerbs(&self) -> Result<IEnumOLEVERB> {
        let enumerator: IEnumOLEVERB = OleVerbEnumerator::new(0).into();
        Ok(enumerator)
    }

    fn Update(&self) -> Result<()> {
        Ok(())
    }

    fn IsUpToDate(&self) -> Result<()> {
        Ok(())
    }

    fn GetUserClassID(&self) -> Result<GUID> {
        Ok(self.class_id)
    }

    fn GetUserType(&self, _form: &USERCLASSTYPE) -> Result<windows_core::PWSTR> {
        ole_user_type()
    }

    fn SetExtent(&self, aspect: DVASPECT, extent: *const SIZE) -> Result<()> {
        ensure_content_aspect(aspect)?;
        if extent.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        self.activex_extent.set(unsafe { *extent });
        self.notify_ole_advise_view_change();
        Ok(())
    }

    fn GetExtent(&self, aspect: DVASPECT) -> Result<SIZE> {
        ensure_content_aspect(aspect)?;
        Ok(self.activex_extent.get())
    }

    fn Advise(&self, sink: Ref<'_, IAdviseSink>) -> Result<u32> {
        let sink = sink.ok()?.clone();
        let cookie = loop {
            let candidate = self.next_ole_advise_cookie.get();
            self.next_ole_advise_cookie.set(candidate.wrapping_add(1).max(1));
            if candidate != 0 && !self.ole_advise_sinks.borrow().contains_key(&candidate) {
                break candidate;
            }
        };
        self.ole_advise_sinks.borrow_mut().insert(cookie, sink);
        Ok(cookie)
    }

    fn Unadvise(&self, connection: u32) -> Result<()> {
        self.ole_advise_sinks
            .borrow_mut()
            .remove(&connection)
            .ok_or_else(|| Error::from_hresult(OLE_E_NOCONNECTION))?;
        Ok(())
    }

    fn EnumAdvise(&self) -> Result<IEnumSTATDATA> {
        let entries = self
            .ole_advise_sinks
            .borrow()
            .iter()
            .map(|(cookie, sink)| (*cookie, sink.clone()))
            .collect();
        let enumerator: IEnumSTATDATA = OleAdviseEnumerator::new(entries, 0).into();
        Ok(enumerator)
    }

    fn GetMiscStatus(&self, aspect: DVASPECT) -> Result<OLEMISC> {
        trace_host_call("IOleObject::GetMiscStatus");
        ensure_content_aspect(aspect)?;
        // These flags describe a windowed in-place control. In particular, mstsc uses
        // ACTIVATEWHENVISIBLE and SETCLIENTSITEFIRST to decide whether to call DoVerb.
        Ok(OLEMISC(
            0x0000_0010 // OLEMISC_CANTLINKINSIDE
                | 0x0000_0080 // OLEMISC_INSIDEOUT
                | 0x0000_0100 // OLEMISC_ACTIVATEWHENVISIBLE
                | 0x0002_0000, // OLEMISC_SETCLIENTSITEFIRST
        ))
    }

    fn SetColorScheme(&self, _palette: *const windows::Win32::Graphics::Gdi::LOGPALETTE) -> Result<()> {
        Ok(())
    }
}

impl IOleWindow_Impl for Control_Impl {
    fn GetWindow(&self) -> Result<HWND> {
        trace_host_call("IOleWindow::GetWindow");
        let window = self.activex_window.get();
        if window.0.is_null() {
            Err(Error::from_hresult(E_FAIL))
        } else {
            Ok(window)
        }
    }

    fn ContextSensitiveHelp(&self, _enter_mode: windows_core::BOOL) -> Result<()> {
        trace_host_call("IOleWindow::ContextSensitiveHelp");
        unsupported()
    }
}

impl IOleInPlaceObject_Impl for Control_Impl {
    fn InPlaceDeactivate(&self) -> Result<()> {
        self.destroy_activex_window()?;
        if let Some(site) = self.client_site.borrow().clone() {
            let in_place_site: IOleInPlaceSite = site.cast()?;
            unsafe {
                in_place_site.OnUIDeactivate(false)?;
                in_place_site.OnInPlaceDeactivate()?;
            }
        }
        Ok(())
    }

    fn UIDeactivate(&self) -> Result<()> {
        self.deactivate_owned_ui();
        if let Some(site) = self.client_site.borrow().clone() {
            let in_place_site: IOleInPlaceSite = site.cast()?;
            unsafe {
                in_place_site.OnUIDeactivate(false)?;
            }
        }
        Ok(())
    }

    fn SetObjectRects(&self, position: *const RECT, clip: *const RECT) -> Result<()> {
        trace_host_call("IOleInPlaceObject::SetObjectRects");
        if position.is_null() || clip.is_null() {
            return Err(Error::from_hresult(E_FAIL));
        }
        let position = unsafe { *position };
        let clip = unsafe { *clip };
        if self.activex_rect.get() != position || self.activex_clip_rect.get() != Some(clip) {
            self.presentation_layout_generation
                .set(self.presentation_layout_generation.get().wrapping_add(1).max(1));
            trace_host_call("Renderer::SetObjectRectsChanged");
        }
        self.activex_rect.set(position);
        self.activex_clip_rect.set(Some(clip));

        // OLE hosts may negotiate the control rectangle before activating it with DoVerb.
        // Retain the requested geometry until the ActiveX child window exists.
        if self.activex_window.get().0.is_null() {
            return Ok(());
        }

        self.resize_activex_window(position)?;
        self.renderer_geometry_changed();
        Ok(())
    }

    fn ReactivateAndUndo(&self) -> Result<()> {
        unsupported()
    }
}

impl IOleInPlaceActiveObject_Impl for Control_Impl {
    fn TranslateAccelerator(&self, message: *const windows::Win32::UI::WindowsAndMessaging::MSG) -> Result<()> {
        trace_host_call("IOleInPlaceActiveObject::TranslateAccelerator");
        if message.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        let Some(client_site) = self.client_site.borrow().clone() else {
            return Err(Error::from_hresult(S_FALSE));
        };
        let Ok(control_site) = client_site.cast::<IOleControlSite>() else {
            return Err(Error::from_hresult(S_FALSE));
        };
        let message = unsafe { *message };
        let result = translate_control_site_accelerator(&control_site, &message, active_key_modifiers());
        if result != S_OK {
            return Err(Error::from_hresult(S_FALSE));
        }
        Ok(())
    }

    fn OnFrameWindowActivate(&self, activate: windows_core::BOOL) -> Result<()> {
        trace_host_call("IOleInPlaceActiveObject::OnFrameWindowActivate");
        let renderer_window = self.activex_window.get();
        if activate.as_bool()
            && !renderer_window.0.is_null()
            && unsafe { IsWindow(Some(renderer_window)) }.as_bool()
            && let Err(error) = unsafe { SetFocus(Some(renderer_window)) }
        {
            tracing::debug!(?error, "Unable to focus ActiveX renderer after frame activation");
        }
        self.in_place_window_activation_changed(activate.as_bool());
        Ok(())
    }

    fn OnDocWindowActivate(&self, activate: windows_core::BOOL) -> Result<()> {
        self.in_place_window_activation_changed(activate.as_bool());
        Ok(())
    }

    fn ResizeBorder(
        &self,
        _border: *const RECT,
        _ui_window: Ref<'_, IOleInPlaceUIWindow>,
        _frame_window: windows_core::BOOL,
    ) -> Result<()> {
        Ok(())
    }

    fn EnableModeless(&self, enable: windows_core::BOOL) -> Result<()> {
        self.set_connection_bar_modeless_enabled(enable.as_bool());
        Ok(())
    }
}

impl IOleControl_Impl for Control_Impl {
    fn GetControlInfo(&self, _info: *mut CONTROLINFO) -> Result<()> {
        unsupported()
    }

    fn OnMnemonic(&self, _message: *const windows::Win32::UI::WindowsAndMessaging::MSG) -> Result<()> {
        unsupported()
    }

    fn OnAmbientPropertyChange(&self, _dispid: i32) -> Result<()> {
        Ok(())
    }

    fn FreezeEvents(&self, freeze: windows_core::BOOL) -> Result<()> {
        self.set_events_frozen(freeze.as_bool());
        Ok(())
    }
}

impl IPersist_Impl for Control_Impl {
    fn GetClassID(&self) -> Result<GUID> {
        trace_host_call("IPersist::GetClassID");
        Ok(self.class_id)
    }
}

impl IPersistStreamInit_Impl for Control_Impl {
    fn IsDirty(&self) -> HRESULT {
        trace_host_call("IPersistStreamInit::IsDirty");
        if self.persistence_dirty.get() { S_OK } else { S_FALSE }
    }

    fn Load(&self, stream: Ref<'_, IStream>) -> Result<()> {
        trace_host_call("IPersistStreamInit::Load");
        if self.state.get() != ConnectionState::Disconnected || !self.activex_window.get().0.is_null() {
            return Err(Error::from_hresult(E_UNEXPECTED));
        }

        let persisted = load_persisted_settings(stream.ok()?)?;
        *self.settings.borrow_mut() = persisted.settings;
        persisted.compatibility.apply_to(&mut self.compatibility.borrow_mut());
        self.persistence_dirty.set(false);
        Ok(())
    }

    fn Save(&self, stream: Ref<'_, IStream>, clear_dirty: windows_core::BOOL) -> Result<()> {
        trace_host_call("IPersistStreamInit::Save");
        let bytes = persisted_settings_bytes(&self.settings.borrow(), &self.compatibility.borrow())?;
        stream_write_all(stream.ok()?, &bytes)?;
        self.notify_ole_advise_save();
        if clear_dirty.as_bool() {
            self.persistence_dirty.set(false);
        }
        Ok(())
    }

    fn GetSizeMax(&self) -> Result<u64> {
        trace_host_call("IPersistStreamInit::GetSizeMax");
        Ok(persisted_settings_bytes(&self.settings.borrow(), &self.compatibility.borrow())?.len() as u64)
    }

    fn InitNew(&self) -> Result<()> {
        trace_host_call("IPersistStreamInit::InitNew");
        if self.state.get() != ConnectionState::Disconnected || !self.activex_window.get().0.is_null() {
            return Err(Error::from_hresult(E_UNEXPECTED));
        }

        *self.settings.borrow_mut() = Settings::default();
        PersistedCompatibilitySettings::default().apply_to(&mut self.compatibility.borrow_mut());
        self.persistence_dirty.set(false);
        Ok(())
    }
}

const PERSISTENCE_MAGIC: [u8; 4] = *b"IRAX";
const PERSISTENCE_VERSION: u16 = 2;
const PERSISTENCE_VERSION_1: u16 = 1;
const MAX_PERSISTED_SETTINGS_BYTES: usize = 64 * 1024;
const MAX_PERSISTED_STRING_BYTES: usize = 8 * 1024;

struct PersistedSettings {
    settings: Settings,
    compatibility: PersistedCompatibilitySettings,
}

struct PersistedCompatibilitySettings {
    smart_sizing: bool,
    zoom_level: i32,
    client_name: Option<String>,
    performance_flags: PerformanceFlags,
    keyboard_layout: u32,
}

impl Default for PersistedCompatibilitySettings {
    fn default() -> Self {
        Self {
            smart_sizing: false,
            zoom_level: 100,
            client_name: None,
            performance_flags: PerformanceFlags::default(),
            keyboard_layout: 0,
        }
    }
}

impl PersistedCompatibilitySettings {
    fn from_compatibility(compatibility: &CompatibilitySettings) -> Self {
        Self {
            smart_sizing: compatibility.smart_sizing,
            zoom_level: compatibility.zoom_level,
            client_name: compatibility.client_name.clone(),
            performance_flags: compatibility.performance_flags,
            keyboard_layout: compatibility.keyboard_layout,
        }
    }

    fn apply_to(self, compatibility: &mut CompatibilitySettings) {
        compatibility.smart_sizing = self.smart_sizing;
        compatibility.zoom_level = self.zoom_level;
        compatibility.client_name = self.client_name;
        compatibility.performance_flags = self.performance_flags;
        compatibility.keyboard_layout = self.keyboard_layout;
    }
}

fn persisted_settings_bytes(settings: &Settings, compatibility: &CompatibilitySettings) -> Result<Vec<u8>> {
    let compatibility = PersistedCompatibilitySettings::from_compatibility(compatibility);
    let strings = [
        &settings.server,
        &settings.domain,
        &settings.username,
        &settings.disconnected_text,
        &settings.connecting_text,
        &settings.connected_status_text,
        &settings.fullscreen_title,
        compatibility.client_name.as_deref().unwrap_or_default(),
    ];
    let string_bytes = strings.iter().try_fold(0usize, |total, value| {
        if value.len() > MAX_PERSISTED_STRING_BYTES {
            return Err(Error::new(E_FAIL, "persisted setting exceeds the maximum length"));
        }
        total
            .checked_add(4 + value.len())
            .ok_or_else(|| Error::new(E_FAIL, "persisted settings size overflow"))
    })?;
    let payload_size = 2 /* DesktopWidth */
        + 2 /* DesktopHeight */
        + 4 /* ColorDepth */
        + 1 /* SettingsFlags */
        + 1 /* CompatibilityFlags */
        + 4 /* ZoomLevel */
        + 4 /* PerformanceFlags */
        + 4 /* KeyboardLayout */
        + string_bytes;
    if payload_size > MAX_PERSISTED_SETTINGS_BYTES {
        return Err(Error::new(E_FAIL, "persisted settings exceed the maximum size"));
    }

    let mut bytes = Vec::with_capacity(10 /* Header */ + payload_size);
    bytes.extend_from_slice(&PERSISTENCE_MAGIC);
    bytes.extend_from_slice(&PERSISTENCE_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(payload_size as u32).to_le_bytes());
    bytes.extend_from_slice(&settings.desktop_width.to_le_bytes());
    bytes.extend_from_slice(&settings.desktop_height.to_le_bytes());
    bytes.extend_from_slice(&settings.color_depth.to_le_bytes());
    bytes.push(u8::from(settings.fullscreen) | (u8::from(settings.start_connected) << 1));
    bytes.push(u8::from(compatibility.smart_sizing));
    bytes.extend_from_slice(&compatibility.zoom_level.to_le_bytes());
    bytes.extend_from_slice(&compatibility.performance_flags.bits().to_le_bytes());
    bytes.extend_from_slice(&compatibility.keyboard_layout.to_le_bytes());
    for value in strings {
        bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    Ok(bytes)
}

fn load_persisted_settings(stream: &IStream) -> Result<PersistedSettings> {
    let mut header = [0u8; 10];
    stream_read_exact(stream, &mut header)?;
    if header[..4] != PERSISTENCE_MAGIC {
        return Err(Error::new(E_INVALIDARG, "unrecognized persisted settings format"));
    }
    let version = u16::from_le_bytes([header[4], header[5]]);
    if !matches!(version, PERSISTENCE_VERSION_1 | PERSISTENCE_VERSION) {
        return Err(Error::new(E_NOTIMPL, "unsupported persisted settings version"));
    }
    let payload_size = u32::from_le_bytes([header[6], header[7], header[8], header[9]]) as usize;
    if payload_size > MAX_PERSISTED_SETTINGS_BYTES {
        return Err(Error::new(E_INVALIDARG, "persisted settings exceed the maximum size"));
    }

    let mut payload = vec![0; payload_size];
    stream_read_exact(stream, &mut payload)?;
    let mut reader = PersistedSettingsReader::new(&payload);
    let desktop_width = reader.read_u16()?;
    let desktop_height = reader.read_u16()?;
    let color_depth = reader.read_u32()?;
    if desktop_width == 0 || desktop_height == 0 || !matches!(color_depth, 16 | 32) {
        return Err(Error::new(
            E_INVALIDARG,
            "persisted settings contain invalid display configuration",
        ));
    }
    let flags = reader.read_u8()?;
    if flags & !0b11 != 0 {
        return Err(Error::new(E_INVALIDARG, "persisted settings contain unknown flags"));
    }
    let compatibility = if version == PERSISTENCE_VERSION {
        let flags = reader.read_u8()?;
        if flags & !1 != 0 {
            return Err(Error::new(
                E_INVALIDARG,
                "persisted settings contain unknown compatibility flags",
            ));
        }
        let zoom_level = reader.read_u32()? as i32;
        if !(10..=500).contains(&zoom_level) {
            return Err(Error::new(
                E_INVALIDARG,
                "persisted settings contain an invalid zoom level",
            ));
        }
        let performance_flags = PerformanceFlags::from_bits(reader.read_u32()?)
            .ok_or_else(|| Error::new(E_INVALIDARG, "persisted settings contain invalid performance flags"))?;
        let keyboard_layout = reader.read_u32()?;
        Some((flags & 1 != 0, zoom_level, performance_flags, keyboard_layout))
    } else {
        None
    };
    let server = reader.read_string()?;
    let domain = reader.read_string()?;
    let username = reader.read_string()?;
    let disconnected_text = reader.read_string()?;
    let connecting_text = reader.read_string()?;
    let connected_status_text = reader.read_string()?;
    let fullscreen_title = reader.read_string()?;
    let client_name = if version == PERSISTENCE_VERSION {
        let client_name = reader.read_string()?;
        if client_name.encode_utf16().count() > 15 {
            return Err(Error::new(
                E_INVALIDARG,
                "persisted settings contain an oversized client device name",
            ));
        }
        (!client_name.is_empty()).then_some(client_name)
    } else {
        None
    };
    if !reader.is_empty() {
        return Err(Error::new(E_INVALIDARG, "persisted settings contain trailing data"));
    }

    Ok(PersistedSettings {
        settings: Settings {
            server,
            domain,
            username,
            // Passwords are never persisted, even when supplied by a caller before Save.
            password: None,
            disconnected_text,
            connecting_text,
            connected_status_text,
            fullscreen: flags & 1 != 0,
            fullscreen_title,
            desktop_width,
            desktop_height,
            color_depth,
            start_connected: flags & 2 != 0,
        },
        compatibility: match compatibility {
            Some((smart_sizing, zoom_level, performance_flags, keyboard_layout)) => PersistedCompatibilitySettings {
                smart_sizing,
                zoom_level,
                client_name,
                performance_flags,
                keyboard_layout,
            },
            None => PersistedCompatibilitySettings::default(),
        },
    })
}

fn stream_read_exact(stream: &IStream, buffer: &mut [u8]) -> Result<()> {
    let mut offset = 0;
    while offset < buffer.len() {
        let mut read = 0;
        unsafe {
            stream
                .Read(
                    buffer[offset..].as_mut_ptr().cast(),
                    u32::try_from(buffer.len() - offset).map_err(|_| Error::from_hresult(E_FAIL))?,
                    Some(&mut read),
                )
                .ok()?;
        }
        if read == 0 {
            return Err(Error::new(E_FAIL, "persisted settings stream ended unexpectedly"));
        }
        offset = stream_transfer_offset(offset, read as usize, buffer.len())?;
    }
    Ok(())
}

fn stream_write_all(stream: &IStream, buffer: &[u8]) -> Result<()> {
    let mut offset = 0;
    while offset < buffer.len() {
        let mut written = 0;
        unsafe {
            stream
                .Write(
                    buffer[offset..].as_ptr().cast(),
                    u32::try_from(buffer.len() - offset).map_err(|_| Error::from_hresult(E_FAIL))?,
                    Some(&mut written),
                )
                .ok()?;
        }
        if written == 0 {
            return Err(Error::new(E_FAIL, "persisted settings stream could not accept data"));
        }
        offset = stream_transfer_offset(offset, written as usize, buffer.len())?;
    }
    Ok(())
}

fn stream_transfer_offset(offset: usize, transferred: usize, length: usize) -> Result<usize> {
    let remaining = length
        .checked_sub(offset)
        .ok_or_else(|| Error::new(E_FAIL, "persisted settings stream offset is invalid"))?;
    if transferred > remaining {
        return Err(Error::new(E_FAIL, "persisted settings stream reported too many bytes"));
    }
    offset
        .checked_add(transferred)
        .ok_or_else(|| Error::new(E_FAIL, "persisted settings stream offset overflowed"))
}

struct PersistedSettingsReader<'a> {
    remaining: &'a [u8],
}

impl<'a> PersistedSettingsReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { remaining: bytes }
    }

    fn read_u8(&mut self) -> Result<u8> {
        let (value, remaining) = self
            .remaining
            .split_first()
            .ok_or_else(|| Error::new(E_INVALIDARG, "persisted settings are truncated"))?;
        self.remaining = remaining;
        Ok(*value)
    }

    fn read_u16(&mut self) -> Result<u16> {
        let bytes = self.read_array()?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn read_u32(&mut self) -> Result<u32> {
        let bytes = self.read_array()?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_string(&mut self) -> Result<String> {
        let length = self.read_u32()? as usize;
        if length > MAX_PERSISTED_STRING_BYTES || length > self.remaining.len() {
            return Err(Error::new(E_INVALIDARG, "persisted string is invalid"));
        }
        let (bytes, remaining) = self.remaining.split_at(length);
        self.remaining = remaining;
        String::from_utf8(bytes.to_vec()).map_err(|_| Error::new(E_INVALIDARG, "persisted string is not UTF-8"))
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N]> {
        if self.remaining.len() < N {
            return Err(Error::new(E_INVALIDARG, "persisted settings are truncated"));
        }
        let (bytes, remaining) = self.remaining.split_at(N);
        self.remaining = remaining;
        Ok(bytes.try_into().expect("array length checked"))
    }

    fn is_empty(&self) -> bool {
        self.remaining.is_empty()
    }
}

#[track_caller]
fn unsupported() -> Result<()> {
    let caller = std::panic::Location::caller();
    trace_host_call(&format!("E_NOTIMPL:{}:{}", caller.file(), caller.line()));
    Err(Error::from_hresult(E_NOTIMPL))
}

#[track_caller]
fn unsupported_value<T>() -> Result<T> {
    let caller = std::panic::Location::caller();
    trace_host_call(&format!("E_NOTIMPL:{}:{}", caller.file(), caller.line()));
    Err(Error::from_hresult(E_NOTIMPL))
}

#[track_caller]
fn unsupported_out<T: Default>(out: *mut T) -> Result<()> {
    write_out(out, T::default())?;
    let caller = std::panic::Location::caller();
    trace_host_call(&format!("E_NOTIMPL:{}:{}", caller.file(), caller.line()));
    Err(Error::from_hresult(E_NOTIMPL))
}

fn write_out<T>(out: *mut T, value: T) -> Result<()> {
    if out.is_null() {
        return Err(Error::from_hresult(E_POINTER));
    }
    unsafe {
        out.write(value);
    }
    Ok(())
}

fn string_from_bstr(value: Bstr) -> Result<String> {
    if value.is_null() {
        return Ok(String::new());
    }

    // BSTR::from_raw would otherwise free the caller's Automation allocation when it is dropped.
    let value = ManuallyDrop::new(unsafe { BSTR::from_raw(value) });
    String::try_from(&*value).map_err(|_| Error::from_hresult(E_INVALIDARG))
}

fn static_channel_name_from_bstr(value: Bstr) -> Result<(String, ChannelName)> {
    let name = string_from_bstr(value)?;
    if name.is_empty() || name.len() > ChannelName::SIZE - 1 || !name.is_ascii() || name.contains([',', ';', '\0']) {
        return Err(Error::from_hresult(E_INVALIDARG));
    }

    let channel_name = ChannelName::from_utf8(&name).ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
    Ok((name, channel_name))
}

fn static_channel_names_from_bstr(value: Bstr) -> Result<Vec<(String, ChannelName)>> {
    let names = string_from_bstr(value)?;
    if names.is_empty() {
        return Err(Error::from_hresult(E_INVALIDARG));
    }

    names
        .split(',')
        .map(|name| {
            let bstr = BSTR::from(name);
            static_channel_name_from_bstr(bstr.as_ptr())
        })
        .collect()
}

fn is_reserved_static_channel_name(name: &str) -> bool {
    ["drdynvc", "cliprdr", "rdpsnd"]
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
}

fn channel_data_from_bstr(value: Bstr) -> Result<Vec<u8>> {
    if value.is_null() {
        return Ok(Vec::new());
    }

    // BSTR::from_raw would otherwise free the caller's Automation allocation when it is dropped.
    let value = ManuallyDrop::new(unsafe { BSTR::from_raw(value) });
    let length = unsafe { SysStringLen(&value) } as usize;
    let code_units = unsafe { slice::from_raw_parts(value.as_ptr(), length) };
    code_units
        .iter()
        .copied()
        .map(|code_unit| u8::try_from(code_unit).map_err(|_| Error::from_hresult(E_INVALIDARG)))
        .collect()
}

fn channel_data_to_automation_string(data: &[u8]) -> String {
    let code_units = data.iter().copied().map(u16::from).collect::<Vec<_>>();
    String::from_utf16_lossy(&code_units)
}

fn write_bstr(out: BstrOut, value: &str) -> Result<()> {
    write_out(out, BSTR::from(value).into_raw())
}

fn ole_user_type() -> Result<windows_core::PWSTR> {
    co_task_mem_wide_string("IronRDP ActiveX Control")
}

fn co_task_mem_wide_string(value: &str) -> Result<windows_core::PWSTR> {
    let mut value = value.encode_utf16().collect::<Vec<_>>();
    value.push(0);
    let size = value
        .len()
        .checked_mul(size_of::<u16>())
        .ok_or_else(|| Error::from_hresult(E_OUTOFMEMORY))?;
    let allocation = unsafe { CoTaskMemAlloc(size) };
    if allocation.is_null() {
        return Err(Error::from_hresult(E_OUTOFMEMORY));
    }
    unsafe {
        ptr::copy_nonoverlapping(value.as_ptr(), allocation.cast::<u16>(), value.len());
    }
    Ok(windows_core::PWSTR(allocation.cast()))
}

#[implement(IEnumOLEVERB)]
struct OleVerbEnumerator {
    _lifetime: ServerObjectLifetime,
    index: Cell<usize>,
}

impl OleVerbEnumerator {
    fn new(index: usize) -> Self {
        Self {
            _lifetime: ServerObjectLifetime::new(),
            index: Cell::new(index),
        }
    }
}

impl IEnumOLEVERB_Impl for OleVerbEnumerator_Impl {
    fn Next(&self, requested: u32, verbs: *mut OLEVERB, fetched: *mut u32) -> Result<()> {
        if verbs.is_null() || (requested != 1 && fetched.is_null()) {
            return Err(Error::from_hresult(E_POINTER));
        }

        const VERBS: &[(i32, &str)] = &[(OLEVERB_PRIMARY as i32, "&Open")];
        let available = VERBS.len().saturating_sub(self.index.get());
        let returned = available.min(requested as usize);
        for offset in 0..returned {
            let (verb, name) = VERBS[self.index.get() + offset];
            unsafe {
                verbs.add(offset).write(OLEVERB {
                    lVerb: windows::Win32::System::Ole::OLEIVERB(verb),
                    lpszVerbName: co_task_mem_wide_string(name)?,
                    fuFlags: Default::default(),
                    // Opening this control performs in-place activation only; it never mutates
                    // the persisted control settings.
                    grfAttribs: OLEVERBATTRIB_NEVERDIRTIES.0 as u32,
                });
            }
        }
        self.index.set(self.index.get() + returned);
        if !fetched.is_null() {
            unsafe {
                fetched.write(returned as u32);
            }
        }
        if returned == requested as usize {
            Ok(())
        } else {
            Err(Error::from_hresult(S_FALSE))
        }
    }

    fn Skip(&self, requested: u32) -> Result<()> {
        let available = 1usize.saturating_sub(self.index.get());
        let skipped = available.min(requested as usize);
        self.index.set(self.index.get() + skipped);
        if skipped == requested as usize {
            Ok(())
        } else {
            Err(Error::from_hresult(S_FALSE))
        }
    }

    fn Reset(&self) -> Result<()> {
        self.index.set(0);
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumOLEVERB> {
        let clone: IEnumOLEVERB = OleVerbEnumerator::new(self.index.get()).into();
        Ok(clone)
    }
}

#[implement(IEnumSTATDATA)]
struct OleAdviseEnumerator {
    _lifetime: ServerObjectLifetime,
    entries: Vec<(u32, IAdviseSink)>,
    index: Cell<usize>,
}

impl OleAdviseEnumerator {
    fn new(entries: Vec<(u32, IAdviseSink)>, index: usize) -> Self {
        Self {
            _lifetime: ServerObjectLifetime::new(),
            entries,
            index: Cell::new(index),
        }
    }
}

impl IEnumSTATDATA_Impl for OleAdviseEnumerator_Impl {
    fn Next(&self, requested: u32, entries: *mut STATDATA, fetched: *mut u32) -> Result<()> {
        if entries.is_null() || (requested != 1 && fetched.is_null()) {
            return Err(Error::from_hresult(E_POINTER));
        }

        let available = self.entries.len().saturating_sub(self.index.get());
        let returned = available.min(requested as usize);
        for offset in 0..returned {
            let (cookie, sink) = &self.entries[self.index.get() + offset];
            unsafe {
                entries.add(offset).write(STATDATA {
                    formatetc: FORMATETC::default(),
                    advf: 0,
                    pAdvSink: ManuallyDrop::new(Some(sink.clone())),
                    dwConnection: *cookie,
                });
            }
        }
        self.index.set(self.index.get() + returned);
        if !fetched.is_null() {
            unsafe {
                fetched.write(returned as u32);
            }
        }
        if returned == requested as usize {
            Ok(())
        } else {
            Err(Error::from_hresult(S_FALSE))
        }
    }

    fn Skip(&self, requested: u32) -> Result<()> {
        let available = self.entries.len().saturating_sub(self.index.get());
        let skipped = available.min(requested as usize);
        self.index.set(self.index.get() + skipped);
        if skipped == requested as usize {
            Ok(())
        } else {
            Err(Error::from_hresult(S_FALSE))
        }
    }

    fn Reset(&self) -> Result<()> {
        self.index.set(0);
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumSTATDATA> {
        let clone: IEnumSTATDATA = OleAdviseEnumerator::new(self.entries.clone(), self.index.get()).into();
        Ok(clone)
    }
}

fn invalidate_renderer(window: HWND) {
    if !window.0.is_null() && unsafe { IsWindow(Some(window)) }.as_bool() {
        trace_host_call("Renderer::InvalidateViewport");
        unsafe {
            let _ = InvalidateRect(Some(window), None, false);
        }
    }
}

fn raw_dimension(value: i32, name: &'static str) -> Result<u16> {
    u16::try_from(value).map_err(|_| Error::new(E_INVALIDARG, format!("{name} must fit in u16")))
}

impl IConnectionPointContainer_Impl for Control_Impl {
    fn EnumConnectionPoints(&self) -> Result<IEnumConnectionPoints> {
        trace_host_call("IConnectionPointContainer::EnumConnectionPoints");
        let point = self.connection_point(self.to_interface::<IConnectionPointContainer>())?;
        let enumerator: IEnumConnectionPoints = ConnectionPointEnumerator::new(vec![point], 0).into();
        Ok(enumerator)
    }

    fn FindConnectionPoint(&self, riid: *const GUID) -> Result<IConnectionPoint> {
        trace_host_call("IConnectionPointContainer::FindConnectionPoint");
        if riid.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        if unsafe { *riid } != IID_MSTSCLIB_EVENTS {
            return Err(Error::from_hresult(CONNECT_E_NOCONNECTION));
        }
        self.connection_point(self.to_interface::<IConnectionPointContainer>())
    }
}

#[implement(IConnectionPoint)]
struct ConnectionPoint {
    _lifetime: ServerObjectLifetime,
    sinks: Rc<RefCell<BTreeMap<u32, EventSink>>>,
    next_cookie: Rc<Cell<u32>>,
    container: IConnectionPointContainer,
}

impl ConnectionPoint {
    fn new(
        sinks: Rc<RefCell<BTreeMap<u32, EventSink>>>,
        next_cookie: Rc<Cell<u32>>,
        container: IConnectionPointContainer,
    ) -> Self {
        Self {
            _lifetime: ServerObjectLifetime::new(),
            sinks,
            next_cookie,
            container,
        }
    }
}

impl IConnectionPoint_Impl for ConnectionPoint_Impl {
    fn GetConnectionInterface(&self) -> Result<GUID> {
        trace_host_call("IConnectionPoint::GetConnectionInterface");
        Ok(IID_MSTSCLIB_EVENTS)
    }

    fn GetConnectionPointContainer(&self) -> Result<IConnectionPointContainer> {
        Ok(self.container.clone())
    }

    fn Advise(&self, sink: Ref<'_, IUnknown>) -> Result<u32> {
        trace_host_call("IConnectionPoint::Advise");
        let sink = sink.ok()?;
        let mut raw = ptr::null_mut();
        let result = unsafe { sink.query(&IID_MSTSCLIB_EVENTS, &mut raw) };
        result.ok().map_err(|_| Error::from_hresult(CONNECT_E_CANNOTCONNECT))?;
        let dispatch = unsafe { IDispatch::from_raw(raw) };

        let cookie = loop {
            let candidate = self.next_cookie.get();
            self.next_cookie.set(candidate.wrapping_add(1).max(1));
            if candidate != 0 && !self.sinks.borrow().contains_key(&candidate) {
                break candidate;
            }
        };

        self.sinks.borrow_mut().insert(cookie, EventSink { cookie, dispatch });
        Ok(cookie)
    }

    fn Unadvise(&self, cookie: u32) -> Result<()> {
        trace_host_call("IConnectionPoint::Unadvise");
        self.sinks
            .borrow_mut()
            .remove(&cookie)
            .ok_or_else(|| Error::from_hresult(CONNECT_E_NOCONNECTION))?;
        Ok(())
    }

    fn EnumConnections(&self) -> Result<IEnumConnections> {
        let connections = self
            .sinks
            .borrow()
            .values()
            .map(|sink| (sink.cookie, sink.dispatch.clone()))
            .collect();
        let enumerator: IEnumConnections = ConnectionEnumerator::new(connections, 0).into();
        Ok(enumerator)
    }
}

#[implement(IEnumConnectionPoints)]
struct ConnectionPointEnumerator {
    _lifetime: ServerObjectLifetime,
    points: Vec<IConnectionPoint>,
    index: Cell<usize>,
}

impl ConnectionPointEnumerator {
    fn new(points: Vec<IConnectionPoint>, index: usize) -> Self {
        Self {
            _lifetime: ServerObjectLifetime::new(),
            points,
            index: Cell::new(index),
        }
    }
}

impl IEnumConnectionPoints_Impl for ConnectionPointEnumerator_Impl {
    fn Next(&self, requested: u32, points: *mut Option<IConnectionPoint>, fetched: *mut u32) -> HRESULT {
        if points.is_null() || (requested != 1 && fetched.is_null()) {
            return E_POINTER;
        }

        let available = self.points.len().saturating_sub(self.index.get());
        let returned = available.min(requested as usize);
        for offset in 0..returned {
            unsafe {
                points
                    .add(offset)
                    .write(Some(self.points[self.index.get() + offset].clone()));
            }
        }
        self.index.set(self.index.get() + returned);
        if !fetched.is_null() {
            unsafe {
                fetched.write(returned as u32);
            }
        }
        if returned == requested as usize { S_OK } else { S_FALSE }
    }

    fn Skip(&self, requested: u32) -> Result<()> {
        let available = self.points.len().saturating_sub(self.index.get());
        let skipped = available.min(requested as usize);
        self.index.set(self.index.get() + skipped);
        if skipped == requested as usize {
            Ok(())
        } else {
            Err(Error::from_hresult(S_FALSE))
        }
    }

    fn Reset(&self) -> Result<()> {
        self.index.set(0);
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumConnectionPoints> {
        let clone: IEnumConnectionPoints = ConnectionPointEnumerator::new(self.points.clone(), self.index.get()).into();
        Ok(clone)
    }
}

#[implement(IEnumConnections)]
struct ConnectionEnumerator {
    _lifetime: ServerObjectLifetime,
    connections: Vec<(u32, IDispatch)>,
    index: Cell<usize>,
}

impl ConnectionEnumerator {
    fn new(connections: Vec<(u32, IDispatch)>, index: usize) -> Self {
        Self {
            _lifetime: ServerObjectLifetime::new(),
            connections,
            index: Cell::new(index),
        }
    }
}

impl IEnumConnections_Impl for ConnectionEnumerator_Impl {
    fn Next(&self, requested: u32, connections: *mut CONNECTDATA, fetched: *mut u32) -> HRESULT {
        if connections.is_null() || (requested != 1 && fetched.is_null()) {
            return E_POINTER;
        }

        let available = self.connections.len().saturating_sub(self.index.get());
        let returned = available.min(requested as usize);
        for offset in 0..returned {
            let (cookie, dispatch) = &self.connections[self.index.get() + offset];
            let unknown = dispatch.cast::<IUnknown>();
            let unknown = match unknown {
                Ok(unknown) => unknown,
                Err(error) => return error.code(),
            };
            unsafe {
                connections.add(offset).write(CONNECTDATA {
                    pUnk: ManuallyDrop::new(Some(unknown)),
                    dwCookie: *cookie,
                });
            }
        }
        self.index.set(self.index.get() + returned);
        if !fetched.is_null() {
            unsafe {
                fetched.write(returned as u32);
            }
        }
        if returned == requested as usize { S_OK } else { S_FALSE }
    }

    fn Skip(&self, requested: u32) -> Result<()> {
        let available = self.connections.len().saturating_sub(self.index.get());
        let skipped = available.min(requested as usize);
        self.index.set(self.index.get() + skipped);
        if skipped == requested as usize {
            Ok(())
        } else {
            Err(Error::from_hresult(S_FALSE))
        }
    }

    fn Reset(&self) -> Result<()> {
        self.index.set(0);
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumConnections> {
        let clone: IEnumConnections = ConnectionEnumerator::new(self.connections.clone(), self.index.get()).into();
        Ok(clone)
    }
}

fn queue_worker_event(
    events: &Arc<WorkerEventQueue>,
    event_posted: &Arc<AtomicBool>,
    hwnd: HWND,
    event: WorkerEvent,
) -> bool {
    let auto_reconnect_key = match &event {
        WorkerEvent::AutoReconnecting {
            generation, attempt, ..
        } => Some((*generation, *attempt)),
        _ => None,
    };
    if events.closed.load(Ordering::Acquire) {
        return false;
    }
    let mut queue = match events.events.lock() {
        Ok(queue) => queue,
        Err(poisoned) => poisoned.into_inner(),
    };
    if events.closed.load(Ordering::Acquire) {
        return false;
    }

    let queued = match &event {
        WorkerEvent::Image { generation, .. } => {
            if let Some(pending) = queue.iter_mut().rev().find(|pending| {
                matches!(pending, WorkerEvent::Image { generation: pending_generation, .. } if *pending_generation == *generation)
            }) {
                *pending = event;
                true
            } else {
                if queue.len() >= MAX_PENDING_WORKER_EVENTS
                    && let Some(index) = queue.iter().position(|pending| matches!(pending, WorkerEvent::Image { .. }))
                {
                    queue.remove(index);
                }
                if queue.len() < MAX_PENDING_WORKER_EVENTS {
                    queue.push(event);
                    true
                } else {
                    false
                }
            }
        }
        WorkerEvent::StaticChannelData { .. } => {
            if queue.len() >= MAX_PENDING_WORKER_EVENTS
                && let Some(index) = queue.iter().position(|pending| matches!(pending, WorkerEvent::Image { .. }))
            {
                queue.remove(index);
            }
            if queue.len() < MAX_PENDING_WORKER_EVENTS {
                queue.push(event);
                true
            } else {
                false
            }
        }
        WorkerEvent::AutoReconnecting { .. } => {
            while queue.len() >= MAX_PENDING_WORKER_EVENTS {
                if let Some(index) = queue
                    .iter()
                    .position(|pending| matches!(pending, WorkerEvent::Image { .. } | WorkerEvent::StaticChannelData { .. }))
                {
                    queue.remove(index);
                } else {
                    return false;
                }
            }
            queue.push(event);
            true
        }
        WorkerEvent::Connected { .. }
        | WorkerEvent::MonitorLayout { .. }
        | WorkerEvent::LoginComplete { .. }
        | WorkerEvent::DisplayResizeFallback { .. }
        | WorkerEvent::AutoReconnected { .. } => {
            if queue.iter().any(|pending| {
                pending.generation() == event.generation()
                    && core::mem::discriminant(pending) == core::mem::discriminant(&event)
            }) {
                true
            } else {
                while queue.len() >= MAX_PENDING_WORKER_EVENTS {
                    queue = match events.space_available.wait(queue) {
                        Ok(queue) => queue,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    if events.closed.load(Ordering::Acquire) {
                        return false;
                    }
                }
                queue.push(event);
                true
            }
        }
        WorkerEvent::RailWindowingOrders { .. }
        | WorkerEvent::CertificateWarning { .. }
        | WorkerEvent::FatalError { .. }
        | WorkerEvent::Disconnected { .. }
        | WorkerEvent::Stopped { .. } => {
            while queue.len() >= MAX_PENDING_WORKER_EVENTS {
                queue = match events.space_available.wait(queue) {
                    Ok(queue) => queue,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if events.closed.load(Ordering::Acquire) {
                    return false;
                }
            }
            queue.push(event);
            true
        }
    };
    if !queued {
        return false;
    }

    if !event_posted.swap(true, Ordering::AcqRel)
        && let Err(error) = unsafe { PostMessageW(Some(hwnd), WM_DISPATCH_EVENTS, WPARAM(0), LPARAM(0)) }
    {
        event_posted.store(false, Ordering::Release);
        tracing::debug!(?error, "Unable to post ActiveX event dispatch message");
        if let Some((generation, attempt)) = auto_reconnect_key {
            if let Some(index) = queue.iter().rposition(|pending| {
                matches!(
                    pending,
                    WorkerEvent::AutoReconnecting {
                        generation: pending_generation,
                        attempt: pending_attempt,
                        ..
                    } if *pending_generation == generation && *pending_attempt == attempt
                )
            }) {
                queue.remove(index);
                events.space_available.notify_all();
            }
            return false;
        }
    }
    drop(queue);
    true
}

struct RendererClassState {
    registered: bool,
    windows: u32,
}

static RENDERER_CLASS_STATE: Mutex<RendererClassState> = Mutex::new(RendererClassState {
    registered: false,
    windows: 0,
});

fn acquire_renderer_class() -> Result<()> {
    let mut state = match RENDERER_CLASS_STATE.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };

    if !state.registered {
        let instance = unsafe { GetModuleHandleW(None) }?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(renderer_window_proc),
            hInstance: windows::Win32::Foundation::HINSTANCE(instance.0),
            lpszClassName: w!("IronRDP.ActiveX.Renderer"),
            ..Default::default()
        };
        let atom = unsafe { RegisterClassW(&class) };
        if atom == 0 {
            let error = unsafe { windows::Win32::Foundation::GetLastError() };
            return Err(Error::from_hresult(HRESULT::from_win32(error.0)));
        }
        state.registered = true;
    }

    state.windows = state
        .windows
        .checked_add(1)
        .ok_or_else(|| Error::from_hresult(E_FAIL))?;
    Ok(())
}

fn release_renderer_class() {
    let mut state = match RENDERER_CLASS_STATE.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };

    if state.windows == 0 {
        return;
    }
    state.windows -= 1;
    if state.windows != 0 || !state.registered {
        return;
    }

    let instance = match unsafe { GetModuleHandleW(None) } {
        Ok(instance) => instance,
        Err(error) => {
            tracing::error!(
                ?error,
                "Unable to find module while unregistering ActiveX renderer class"
            );
            return;
        }
    };
    match unsafe {
        UnregisterClassW(
            w!("IronRDP.ActiveX.Renderer"),
            Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
        )
    } {
        Ok(()) => state.registered = false,
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CLASS_DOES_NOT_EXIST.0) => state.registered = false,
        Err(error) => tracing::error!(?error, "Unable to unregister ActiveX renderer class"),
    }
}

struct ConnectionBarClassState {
    registered: bool,
    windows: u32,
}

static CONNECTION_BAR_CLASS_STATE: Mutex<ConnectionBarClassState> = Mutex::new(ConnectionBarClassState {
    registered: false,
    windows: 0,
});

fn acquire_connection_bar_class() -> Result<()> {
    let mut state = match CONNECTION_BAR_CLASS_STATE.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };

    if !state.registered {
        let instance = unsafe { GetModuleHandleW(None) }?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(connection_bar_window_proc),
            hInstance: windows::Win32::Foundation::HINSTANCE(instance.0),
            lpszClassName: w!("IronRDP.ActiveX.ConnectionBar"),
            ..Default::default()
        };
        let atom = unsafe { RegisterClassW(&class) };
        if atom == 0 {
            let error = unsafe { windows::Win32::Foundation::GetLastError() };
            return Err(Error::from_hresult(HRESULT::from_win32(error.0)));
        }
        state.registered = true;
    }

    state.windows = state
        .windows
        .checked_add(1)
        .ok_or_else(|| Error::from_hresult(E_FAIL))?;
    Ok(())
}

fn release_connection_bar_class() {
    let mut state = match CONNECTION_BAR_CLASS_STATE.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };

    if state.windows == 0 {
        return;
    }
    state.windows -= 1;
    if state.windows != 0 || !state.registered {
        return;
    }

    let instance = match unsafe { GetModuleHandleW(None) } {
        Ok(instance) => instance,
        Err(error) => {
            tracing::error!(
                ?error,
                "Unable to find module while unregistering ActiveX connection bar class"
            );
            return;
        }
    };
    match unsafe {
        UnregisterClassW(
            w!("IronRDP.ActiveX.ConnectionBar"),
            Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
        )
    } {
        Ok(()) => state.registered = false,
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CLASS_DOES_NOT_EXIST.0) => state.registered = false,
        Err(error) => tracing::error!(?error, "Unable to unregister ActiveX connection bar class"),
    }
}

struct ConnectionHealthClassState {
    registered: bool,
    windows: u32,
}

static CONNECTION_HEALTH_CLASS_STATE: Mutex<ConnectionHealthClassState> = Mutex::new(ConnectionHealthClassState {
    registered: false,
    windows: 0,
});

fn acquire_connection_health_class() -> Result<()> {
    let mut state = match CONNECTION_HEALTH_CLASS_STATE.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };

    if !state.registered {
        let instance = unsafe { GetModuleHandleW(None) }?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(connection_health_window_proc),
            hInstance: windows::Win32::Foundation::HINSTANCE(instance.0),
            lpszClassName: w!("IronRDP.ActiveX.ConnectionHealth"),
            ..Default::default()
        };
        let atom = unsafe { RegisterClassW(&class) };
        if atom == 0 {
            let error = unsafe { windows::Win32::Foundation::GetLastError() };
            return Err(Error::from_hresult(HRESULT::from_win32(error.0)));
        }
        state.registered = true;
    }

    state.windows = state
        .windows
        .checked_add(1)
        .ok_or_else(|| Error::from_hresult(E_FAIL))?;
    Ok(())
}

fn release_connection_health_class() {
    let mut state = match CONNECTION_HEALTH_CLASS_STATE.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };

    if state.windows == 0 {
        return;
    }
    state.windows -= 1;
    if state.windows != 0 || !state.registered {
        return;
    }

    let instance = match unsafe { GetModuleHandleW(None) } {
        Ok(instance) => instance,
        Err(error) => {
            tracing::error!(
                ?error,
                "Unable to find module while unregistering ActiveX connection health class"
            );
            return;
        }
    };
    match unsafe {
        UnregisterClassW(
            w!("IronRDP.ActiveX.ConnectionHealth"),
            Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
        )
    } {
        Ok(()) => state.registered = false,
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CLASS_DOES_NOT_EXIST.0) => state.registered = false,
        Err(error) => tracing::error!(?error, "Unable to unregister ActiveX connection health class"),
    }
}

struct DispatcherClassState {
    registered: bool,
    windows: u32,
}

static DISPATCHER_CLASS_STATE: Mutex<DispatcherClassState> = Mutex::new(DispatcherClassState {
    registered: false,
    windows: 0,
});

pub(crate) fn dispatcher_class_is_registered() -> bool {
    match DISPATCHER_CLASS_STATE.lock() {
        Ok(state) => state.registered,
        Err(poisoned) => poisoned.into_inner().registered,
    }
}

fn acquire_dispatcher_class() -> Result<()> {
    let mut state = match DISPATCHER_CLASS_STATE.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };

    if !state.registered {
        let instance = unsafe { GetModuleHandleW(None) }?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(dispatcher_window_proc),
            hInstance: windows::Win32::Foundation::HINSTANCE(instance.0),
            lpszClassName: w!("IronRDP.ActiveX.EventDispatcher"),
            ..Default::default()
        };
        let atom = unsafe { RegisterClassW(&class) };
        if atom == 0 {
            let error = unsafe { windows::Win32::Foundation::GetLastError() };
            return Err(Error::from_hresult(HRESULT::from_win32(error.0)));
        }
        state.registered = true;
    }

    state.windows = state
        .windows
        .checked_add(1)
        .ok_or_else(|| Error::from_hresult(E_FAIL))?;
    Ok(())
}

fn release_dispatcher_class() {
    let mut state = match DISPATCHER_CLASS_STATE.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };

    if state.windows == 0 {
        return;
    }
    state.windows -= 1;
    if state.windows != 0 || !state.registered {
        return;
    }

    let instance = match unsafe { GetModuleHandleW(None) } {
        Ok(instance) => instance,
        Err(error) => {
            tracing::error!(
                ?error,
                "Unable to find module while unregistering event dispatcher class"
            );
            return;
        }
    };
    match unsafe {
        UnregisterClassW(
            w!("IronRDP.ActiveX.EventDispatcher"),
            Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
        )
    } {
        Ok(()) => state.registered = false,
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CLASS_DOES_NOT_EXIST.0) => state.registered = false,
        Err(error) => tracing::error!(?error, "Unable to unregister ActiveX event dispatcher class"),
    }
}

struct ControlWindowContext {
    control: *const Control_Impl,
    module: HMODULE,
    closing: AtomicBool,
    window_reference_released: Cell<bool>,
    orphaned: AtomicBool,
}

impl Drop for ControlWindowContext {
    fn drop(&mut self) {
        com::release_module_reference(self.module);
    }
}

unsafe fn control_from_window(hwnd: HWND) -> *const Control_Impl {
    let context = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const ControlWindowContext;
    let Some(context) = (unsafe { context.as_ref() }) else {
        return ptr::null();
    };
    if context.closing.load(Ordering::Acquire) {
        return ptr::null();
    }
    context.control
}

unsafe fn control_from_context(context: *const ControlWindowContext) -> *const Control_Impl {
    match unsafe { context.as_ref() } {
        Some(context) => context.control,
        None => ptr::null(),
    }
}

fn mark_window_context_closing(hwnd: HWND) {
    let context = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const ControlWindowContext;
    if let Some(context) = unsafe { context.as_ref() } {
        context.closing.store(true, Ordering::Release);
    }
}

fn defer_window_resource_release(hwnd: HWND) {
    let context = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const ControlWindowContext;
    if let Some(context) = unsafe { context.as_ref() } {
        // DestroyWindow failed after this context was detached from its Control. Retaining the
        // context permanently pins the module until process exit, which is safer than allowing a
        // remaining or returning window procedure to execute from an unloaded DLL.
        context.orphaned.store(true, Ordering::Release);
    }
}

fn destroy_control_window(hwnd: HWND) -> Result<()> {
    let result = unsafe { SendMessageW(hwnd, WM_DESTROY_CONTROL_WINDOW, Some(WPARAM(0)), Some(LPARAM(0))) };
    if result.0 == 0 {
        Ok(())
    } else {
        Err(Error::from_hresult(HRESULT(result.0 as i32)))
    }
}

fn destroy_control_window_on_owner_thread(hwnd: HWND) -> LRESULT {
    mark_window_context_closing(hwnd);
    match unsafe { DestroyWindow(hwnd) } {
        Ok(()) => LRESULT(0),
        Err(error) => LRESULT(error.code().0 as isize),
    }
}

unsafe extern "system" fn renderer_window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_DESTROY_CONTROL_WINDOW => destroy_control_window_on_owner_thread(hwnd),
        WM_NCCREATE => {
            if lparam.0 == 0 || unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } != 0 {
                return LRESULT(0);
            }
            let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            if create.lpCreateParams.is_null() {
                return LRESULT(0);
            }
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            LRESULT(1)
        }
        WM_PAINT => {
            if let Some(owner) = unsafe { control_from_window(hwnd).as_ref() } {
                let _keep_alive: IUnknown = owner.to_interface();
                let control: &Control = owner;
                control.paint_activex_window(hwnd);
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let context = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const ControlWindowContext;
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            let orphaned = unsafe { context.as_ref() }.is_some_and(|context| context.orphaned.load(Ordering::Acquire));
            let mut release_renderer_class_after_destroy = false;
            if let Some(context) = unsafe { context.as_ref() } {
                if !context.closing.load(Ordering::Acquire) {
                    if let Some(owner) = unsafe { control_from_context(context).as_ref() } {
                        let control: &Control = owner;
                        release_renderer_class_after_destroy = control.renderer_destroyed_unexpectedly(hwnd);
                    }
                }
            }
            let result = unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            if release_renderer_class_after_destroy {
                release_renderer_class();
            }
            if !context.is_null() && !orphaned {
                unsafe {
                    (*context).window_reference_released.set(true);
                    drop(Rc::from_raw(context));
                }
            }
            result
        }
        _ => {
            let handled = if let Some(owner) = unsafe { control_from_window(hwnd).as_ref() } {
                let _keep_alive: IUnknown = owner.to_interface();
                let control: &Control = owner;
                control.handle_activex_window_message(hwnd, message, wparam, lparam)
            } else {
                false
            };
            if handled {
                if matches!(message, WM_XBUTTONDOWN | WM_XBUTTONUP) {
                    LRESULT(1)
                } else {
                    LRESULT(0)
                }
            } else {
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
    }
}

unsafe extern "system" fn connection_bar_button_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    let Ok(parent) = (unsafe { GetParent(hwnd) }) else {
        return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    };
    if let Some(owner) = unsafe { control_from_window(parent).as_ref() } {
        let _keep_alive: IUnknown = owner.to_interface();
        let control: &Control = owner;
        match message {
            WM_KEYDOWN if wparam.0 == usize::from(VK_TAB.0) => {
                let reverse = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
                if control.focus_connection_bar_button(parent, hwnd, reverse) {
                    return LRESULT(0);
                }
            }
            WM_KEYDOWN if wparam.0 == usize::from(VK_ESCAPE.0) => {
                control.restore_renderer_focus();
                return LRESULT(0);
            }
            WM_MOUSEMOVE => {
                control.connection_bar_mouse_move(parent);
            }
            WM_MOUSELEAVE => {
                control.connection_bar_mouse_leave(parent);
            }
            _ => {}
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

unsafe extern "system" fn connection_bar_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_DESTROY_CONTROL_WINDOW => destroy_control_window_on_owner_thread(hwnd),
        WM_NCCREATE => {
            if lparam.0 == 0 || unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } != 0 {
                return LRESULT(0);
            }
            let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            if create.lpCreateParams.is_null() {
                return LRESULT(0);
            }
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            LRESULT(1)
        }
        WM_COMMAND => {
            if let Some(owner) = unsafe { control_from_window(hwnd).as_ref() } {
                let _keep_alive: IUnknown = owner.to_interface();
                let control: &Control = owner;
                control.handle_connection_bar_command(hwnd, wparam);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == CONNECTION_BAR_AUTO_HIDE_TIMER_ID => {
            if let Some(owner) = unsafe { control_from_window(hwnd).as_ref() } {
                let _keep_alive: IUnknown = owner.to_interface();
                let control: &Control = owner;
                control.hide_connection_bar(hwnd);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == CONNECTION_BAR_OWNER_LAYOUT_TIMER_ID => {
            if let Some(owner) = unsafe { control_from_window(hwnd).as_ref() } {
                let _keep_alive: IUnknown = owner.to_interface();
                let control: &Control = owner;
                if control.connection_bar_visible.get()
                    && control.current_connection_bar_owner_layout() != control.connection_bar_owner_layout.get()
                {
                    control.position_connection_bar(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if let Some(owner) = unsafe { control_from_window(hwnd).as_ref() } {
                let _keep_alive: IUnknown = owner.to_interface();
                let control: &Control = owner;
                control.connection_bar_mouse_move(hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if let Some(owner) = unsafe { control_from_window(hwnd).as_ref() } {
                let _keep_alive: IUnknown = owner.to_interface();
                let control: &Control = owner;
                control.connection_bar_mouse_leave(hwnd);
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            if let Some(owner) = unsafe { control_from_window(hwnd).as_ref() } {
                let _keep_alive: IUnknown = owner.to_interface();
                let control: &Control = owner;
                control.position_connection_bar(hwnd);
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let context = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const ControlWindowContext;
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            let orphaned = unsafe { context.as_ref() }.is_some_and(|context| context.orphaned.load(Ordering::Acquire));
            if let Some(context) = unsafe { context.as_ref() } {
                if !context.closing.load(Ordering::Acquire) {
                    if let Some(owner) = unsafe { control_from_context(context).as_ref() } {
                        let control: &Control = owner;
                        if control.connection_bar.get() == hwnd {
                            control.connection_bar.set(HWND(ptr::null_mut()));
                            control.connection_bar_visible.set(false);
                        }
                    }
                }
            }
            let result = unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            release_connection_bar_class();
            if !context.is_null() && !orphaned {
                unsafe {
                    (*context).window_reference_released.set(true);
                    drop(Rc::from_raw(context));
                }
            }
            result
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

unsafe extern "system" fn connection_health_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_DESTROY_CONTROL_WINDOW => destroy_control_window_on_owner_thread(hwnd),
        WM_NCCREATE => {
            if lparam.0 == 0 || unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } != 0 {
                return LRESULT(0);
            }
            let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            if create.lpCreateParams.is_null() {
                return LRESULT(0);
            }
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            LRESULT(1)
        }
        WM_TIMER if wparam.0 == CONNECTION_HEALTH_OWNER_LAYOUT_TIMER_ID => {
            if let Some(owner) = unsafe { control_from_window(hwnd).as_ref() } {
                let _keep_alive: IUnknown = owner.to_interface();
                let control: &Control = owner;
                if control.current_connection_health_owner_layout() != control.connection_health_owner_layout.get() {
                    control.refresh_connection_health_window(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // The suggested rectangle is pointer-backed. Recompute from the owned renderer instead
            // of retaining or synthesizing a host-owned message payload.
            if let Some(owner) = unsafe { control_from_window(hwnd).as_ref() } {
                let _keep_alive: IUnknown = owner.to_interface();
                let control: &Control = owner;
                control.refresh_connection_health_window(hwnd);
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            unsafe {
                let _ = KillTimer(Some(hwnd), CONNECTION_HEALTH_OWNER_LAYOUT_TIMER_ID);
            }
            let context = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const ControlWindowContext;
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            let orphaned = unsafe { context.as_ref() }.is_some_and(|context| context.orphaned.load(Ordering::Acquire));
            if let Some(context) = unsafe { context.as_ref() } {
                if !context.closing.load(Ordering::Acquire) {
                    if let Some(owner) = unsafe { control_from_context(context).as_ref() } {
                        let control: &Control = owner;
                        if control.connection_health_window.get() == hwnd {
                            control.connection_health_window.set(HWND(ptr::null_mut()));
                            control.connection_health_owner_layout.set(None);
                            clear_connection_health_status(&control.connection_health_status);
                        }
                    }
                }
            }
            let result = unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            release_connection_health_class();
            if !context.is_null() && !orphaned {
                unsafe {
                    (*context).window_reference_released.set(true);
                    drop(Rc::from_raw(context));
                }
            }
            result
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

unsafe extern "system" fn dispatcher_window_proc(hwnd: HWND, message: u32, _wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_DESTROY_CONTROL_WINDOW => destroy_control_window_on_owner_thread(hwnd),
        WM_NCCREATE => {
            if lparam.0 == 0 || unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } != 0 {
                return LRESULT(0);
            }
            let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            if create.lpCreateParams.is_null() {
                return LRESULT(0);
            }
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            LRESULT(1)
        }
        WM_DISPATCH_EVENTS => {
            if let Some(owner) = unsafe { control_from_window(hwnd).as_ref() } {
                let _keep_alive: IUnknown = owner.to_interface();
                let control: &Control = owner;
                control.dispatch_pending_events();
            }
            LRESULT(0)
        }
        rpc::WM_DISPATCH_RPC => {
            if let Some(owner) = unsafe { control_from_window(hwnd).as_ref() } {
                let _keep_alive: IUnknown = owner.to_interface();
                let control: &Control = owner;
                control.dispatch_rpc_commands();
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let context = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const ControlWindowContext;
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            let orphaned = unsafe { context.as_ref() }.is_some_and(|context| context.orphaned.load(Ordering::Acquire));
            if let Some(context) = unsafe { context.as_ref() } {
                if !context.closing.load(Ordering::Acquire) {
                    if let Some(owner) = unsafe { control_from_context(context).as_ref() } {
                        let control: &Control = owner;
                        if control.dispatcher.get() == hwnd {
                            control.dispatcher.set(HWND(ptr::null_mut()));
                        }
                    }
                }
            }
            let result = unsafe { DefWindowProcW(hwnd, message, WPARAM(0), lparam) };
            if !context.is_null() && !orphaned {
                unsafe {
                    (*context).window_reference_released.set(true);
                    drop(Rc::from_raw(context));
                }
            }
            result
        }
        _ => unsafe { DefWindowProcW(hwnd, message, WPARAM(0), lparam) },
    }
}

enum VariantValue {
    String(String),
    Integer(i32),
    Bool(bool),
}

impl VariantValue {
    unsafe fn into_variant(self) -> VARIANT {
        match self {
            Self::String(value) => {
                let bstr = BSTR::from(value);
                VARIANT {
                    Anonymous: VARIANT_0 {
                        Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                            vt: VT_BSTR,
                            wReserved1: 0,
                            wReserved2: 0,
                            wReserved3: 0,
                            Anonymous: VARIANT_0_0_0 {
                                bstrVal: ManuallyDrop::new(bstr),
                            },
                        }),
                    },
                }
            }
            Self::Integer(value) => VARIANT {
                Anonymous: VARIANT_0 {
                    Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                        vt: VT_I4,
                        wReserved1: 0,
                        wReserved2: 0,
                        wReserved3: 0,
                        Anonymous: VARIANT_0_0_0 { lVal: value },
                    }),
                },
            },
            Self::Bool(value) => VARIANT {
                Anonymous: VARIANT_0 {
                    Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                        vt: VT_BOOL,
                        wReserved1: 0,
                        wReserved2: 0,
                        wReserved3: 0,
                        Anonymous: VARIANT_0_0_0 {
                            boolVal: if value { VARIANT_TRUE } else { VARIANT_FALSE },
                        },
                    }),
                },
            },
        }
    }
}

fn variant_i32(value: i32) -> VARIANT {
    unsafe { VariantValue::Integer(value).into_variant() }
}

fn variant_bool_value(value: bool) -> VARIANT {
    unsafe { VariantValue::Bool(value).into_variant() }
}

fn variant_bstr(value: String) -> VARIANT {
    unsafe { VariantValue::String(value).into_variant() }
}

fn free_owned_bstr_variant(value: &mut VARIANT) {
    let header = variant_header_mut(value);
    if header.vt == VT_BSTR {
        unsafe {
            ManuallyDrop::drop(&mut header.Anonymous.bstrVal);
        }
        header.vt = VT_EMPTY;
    }
}

fn variant_bool_byref(value: &mut VARIANT_BOOL) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt: VT_BOOL | VT_BYREF,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: VARIANT_0_0_0 {
                    pboolVal: value as *mut VARIANT_BOOL,
                },
            }),
        },
    }
}

fn variant_i32_byref(value: &mut i32) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt: VT_I4 | VT_BYREF,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: VARIANT_0_0_0 {
                    plVal: value as *mut i32,
                },
            }),
        },
    }
}

fn property_put_value(params: &DISPPARAMS) -> Result<&VARIANT> {
    if params.cArgs != 1 || params.cNamedArgs != 1 || params.rgvarg.is_null() || params.rgdispidNamedArgs.is_null() {
        return Err(Error::from_hresult(DISP_E_BADPARAMCOUNT));
    }
    if unsafe { *params.rgdispidNamedArgs } != DISPID_PROPERTYPUT {
        return Err(Error::from_hresult(DISP_E_MEMBERNOTFOUND));
    }
    Ok(unsafe { &*params.rgvarg })
}

fn variant_header(value: &VARIANT) -> &VARIANT_0_0 {
    unsafe { &*(&value.Anonymous as *const VARIANT_0 as *const VARIANT_0_0) }
}

fn variant_header_mut(value: &mut VARIANT) -> &mut VARIANT_0_0 {
    unsafe { &mut *(&mut value.Anonymous as *mut VARIANT_0 as *mut VARIANT_0_0) }
}

fn variant_string(value: &VARIANT, argument_error: *mut u32) -> Result<String> {
    let header = variant_header(value);
    if header.vt != VT_BSTR {
        set_argument_error(argument_error);
        return Err(Error::from_hresult(DISP_E_TYPEMISMATCH));
    }
    let bstr = unsafe { &*(&header.Anonymous.bstrVal as *const ManuallyDrop<BSTR> as *const BSTR) };
    String::try_from(bstr).map_err(|_| Error::from_hresult(DISP_E_TYPEMISMATCH))
}

fn variant_i32_value(value: &VARIANT, argument_error: *mut u32) -> Result<i32> {
    let header = variant_header(value);
    if header.vt != VT_I4 {
        set_argument_error(argument_error);
        return Err(Error::from_hresult(DISP_E_TYPEMISMATCH));
    }

    Ok(unsafe { header.Anonymous.lVal })
}

fn variant_zoom_level(value: &VARIANT, argument_error: *mut u32) -> Result<i32> {
    let header = variant_header(value);
    match header.vt {
        VT_I4 => Ok(unsafe { header.Anonymous.lVal }),
        VT_UI4 => i32::try_from(unsafe { header.Anonymous.ulVal })
            .map_err(|_| Error::new(E_INVALIDARG, "zoom level must fit in i32")),
        _ => {
            set_argument_error(argument_error);
            Err(Error::from_hresult(DISP_E_TYPEMISMATCH))
        }
    }
}

fn variant_bool(value: &VARIANT, argument_error: *mut u32) -> Result<bool> {
    let header = variant_header(value);
    if header.vt != VT_BOOL {
        set_argument_error(argument_error);
        return Err(Error::from_hresult(DISP_E_TYPEMISMATCH));
    }
    Ok(unsafe { header.Anonymous.boolVal }.0 != 0)
}

fn variant_dimension(value: &VARIANT, argument_error: *mut u32, name: &'static str) -> Result<u16> {
    let value = variant_i32_value(value, argument_error)?;
    u16::try_from(value).map_err(|_| Error::new(E_INVALIDARG, format!("{name} must fit in u16")))
}

fn set_argument_error(argument_error: *mut u32) {
    if !argument_error.is_null() {
        unsafe {
            argument_error.write(0);
        }
    }
}

fn dispid_for_name(name: &str) -> Option<i32> {
    match name.to_ascii_lowercase().as_str() {
        "server" => Some(DISPID_SERVER),
        "domain" => Some(DISPID_DOMAIN),
        "username" => Some(DISPID_USERNAME),
        "disconnectedtext" => Some(DISPID_DISCONNECTED_TEXT),
        "connectingtext" => Some(DISPID_CONNECTING_TEXT),
        "connected" => Some(DISPID_CONNECTED),
        "desktopwidth" => Some(DISPID_DESKTOP_WIDTH),
        "desktopheight" => Some(DISPID_DESKTOP_HEIGHT),
        "startconnected" => Some(DISPID_START_CONNECTED),
        "horizontalscrollbarvisible" => Some(DISPID_HORIZONTAL_SCROLLBAR_VISIBLE),
        "verticalscrollbarvisible" => Some(DISPID_VERTICAL_SCROLLBAR_VISIBLE),
        "fullscreentitle" => Some(DISPID_FULLSCREEN_TITLE),
        "cipherstrength" => Some(DISPID_CIPHER_STRENGTH),
        "version" => Some(DISPID_VERSION),
        "securedsettingsenabled" => Some(DISPID_SECURED_SETTINGS_ENABLED),
        "connect" => Some(DISPID_CONNECT),
        "disconnect" => Some(DISPID_DISCONNECT),
        "colordepth" => Some(DISPID_COLOR_DEPTH),
        "extendeddisconnectreason" => Some(DISPID_EXTENDED_DISCONNECT_REASON),
        "fullscreen" => Some(DISPID_FULLSCREEN),
        "connectedstatustext" => Some(DISPID_CONNECTED_STATUS_TEXT),
        "ironrdppassword" => Some(DISPID_IRONRDP_PASSWORD),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_pdu::input::fast_path::{FastPathInputEvent, KeyboardFlags};
    use ironrdp_pdu::rdp::capability_sets::{CodecProperty, client_codecs_capabilities};
    use windows::Win32::System::Com::{
        CoTaskMemFree, IAdviseSink_Impl, IMoniker, IPersist, STREAM_SEEK_SET, StructuredStorage::CreateStreamOnHGlobal,
    };
    use windows::Win32::System::Ole::{OLECLOSE_NOSAVE, ReleaseStgMedium};
    use windows::Win32::UI::WindowsAndMessaging::WS_OVERLAPPEDWINDOW;

    use crate::mstsc::{
        IMsRdpClient6_Vtbl, IMsRdpClient7_Vtbl, IMsRdpClient8_Vtbl, IMsRdpClient9_Vtbl, IMsRdpClient10_Vtbl,
        IMsRdpClientNonScriptable7_Vtbl, IMsRdpClientNonScriptable8_Vtbl, IMsTscAx,
    };

    #[test]
    fn activex_does_not_advertise_remotefx() {
        let codecs =
            client_codecs_capabilities(ACTIVEX_CODEC_CONFIGURATION).expect("ActiveX codec configuration is valid");

        assert!(
            !codecs
                .0
                .iter()
                .any(|codec| matches!(&codec.property, CodecProperty::RemoteFx(_)))
        );
    }

    #[test]
    fn activex_uses_core_compatible_lossless_bitmaps() {
        let color_depth = Settings::default().color_depth;
        assert_eq!(color_depth, 32);

        let config = ConfigBuilder::new()
            .with_destination(Destination::from_parts("rdp.example.test", 3389))
            .with_username("user")
            .with_password("password")
            .with_color_depth(color_depth)
            .with_client_build(10_000)
            .with_client_dir("C:\\")
            .with_client_name("IronRDP ActiveX")
            .with_platform(MajorPlatformType::WINDOWS)
            .with_lossy_compression(ACTIVEX_LOSSY_COMPRESSION)
            .with_codecs(
                ACTIVEX_CODEC_CONFIGURATION
                    .iter()
                    .map(|option| (*option).to_owned())
                    .collect(),
            )
            .build()
            .expect("ActiveX configuration is valid");

        let bitmap = config
            .connector()
            .bitmap
            .as_ref()
            .expect("ActiveX config includes bitmap capabilities");
        assert_eq!(bitmap.color_depth, 32);
        assert!(!bitmap.lossy_compression);
    }

    #[test]
    fn monitor_topology_normalizes_primary_relative_coordinates() {
        let topology = MonitorTopology::from_host_monitors(vec![
            HostMonitor {
                rect: RECT {
                    left: 100,
                    top: 200,
                    right: 1_100,
                    bottom: 1_000,
                },
                primary: true,
            },
            HostMonitor {
                rect: RECT {
                    left: -700,
                    top: 200,
                    right: 100,
                    bottom: 1_000,
                },
                primary: false,
            },
        ])
        .expect("a valid two-monitor topology");

        assert_eq!(topology.desktop_width, 1_800);
        assert_eq!(topology.desktop_height, 800);
        assert_eq!(topology.bounds(), (-800, 0, 999, 799));
        assert_eq!(
            topology.client_monitor_data().monitors,
            vec![
                Monitor {
                    left: 0,
                    top: 0,
                    right: 999,
                    bottom: 799,
                    flags: MonitorFlags::PRIMARY,
                },
                Monitor {
                    left: -800,
                    top: 0,
                    right: -1,
                    bottom: 799,
                    flags: MonitorFlags::empty(),
                },
            ]
        );
    }

    #[test]
    fn monitor_topology_rejects_invalid_geometry() {
        let invalid_primary = MonitorTopology::from_host_monitors(vec![HostMonitor {
            rect: RECT {
                left: 0,
                top: 0,
                right: 800,
                bottom: 600,
            },
            primary: false,
        }])
        .expect_err("a monitor topology needs exactly one primary monitor");
        assert_eq!(invalid_primary.code(), E_INVALIDARG);

        let overlapping = MonitorTopology::from_host_monitors(vec![
            HostMonitor {
                rect: RECT {
                    left: 0,
                    top: 0,
                    right: 800,
                    bottom: 600,
                },
                primary: true,
            },
            HostMonitor {
                rect: RECT {
                    left: 700,
                    top: 0,
                    right: 1_500,
                    bottom: 600,
                },
                primary: false,
            },
        ])
        .expect_err("overlapping monitor rectangles are invalid");
        assert_eq!(overlapping.code(), E_INVALIDARG);

        let too_many = MonitorTopology::from_host_monitors(vec![
            HostMonitor {
                rect: RECT {
                    left: 0,
                    top: 0,
                    right: 800,
                    bottom: 600,
                },
                primary: true,
            };
            MAX_RDP_MONITORS + 1
        ])
        .expect_err("Client Monitor Data supports at most sixteen monitors");
        assert_eq!(too_many.code(), E_INVALIDARG);
    }

    #[test]
    fn non_scriptable5_reports_the_connected_monitor_topology() {
        let control = Control::new();
        let topology = MonitorTopology::from_host_monitors(vec![
            HostMonitor {
                rect: RECT {
                    left: 100,
                    top: 200,
                    right: 1_100,
                    bottom: 1_000,
                },
                primary: true,
            },
            HostMonitor {
                rect: RECT {
                    left: -700,
                    top: 200,
                    right: 100,
                    bottom: 1_000,
                },
                primary: false,
            },
        ])
        .expect("a valid two-monitor topology");
        control.state.set(ConnectionState::Connected);
        *control.active_monitor_topology.borrow_mut() = Some(topology);
        assert_eq!(
            control.remote_monitor_bounds().expect("read remote monitor bounds"),
            (-800, 0, 1_000, 800)
        );
        let non_scriptable: IMsRdpClientNonScriptable5 = control.into();

        let mut count = 0;
        unsafe { non_scriptable.get_RemoteMonitorCount(&mut count) }.expect("read remote monitor count");
        assert_eq!(count, 2);
    }

    #[test]
    fn activex_maps_client_rdcleanpath_transport() {
        let config = ConfigBuilder::new()
            .with_destination(Destination::from_parts("rdp.example.test", 3389))
            .with_username("user")
            .with_password("password")
            .with_client_build(10_000)
            .with_client_dir("C:\\")
            .with_client_name("IronRDP ActiveX")
            .with_platform(MajorPlatformType::WINDOWS)
            .with_transport(TransportKind::RDCleanPath {
                url: "wss://rdcleanpath.example.test/rdp"
                    .parse()
                    .expect("RDCleanPath URL is valid"),
            })
            .with_rdcleanpath_token("test-token")
            .build()
            .expect("ActiveX RDCleanPath configuration is valid");

        let ActiveXTransport::RDCleanPath(rdcleanpath) =
            active_x_transport_from_client_transport(config.transport()).expect("RDCleanPath maps")
        else {
            panic!("client RDCleanPath transport must be retained");
        };
        assert_eq!(rdcleanpath.url.as_str(), "wss://rdcleanpath.example.test/rdp");
        assert_eq!(rdcleanpath.auth_token, "test-token");
    }

    #[test]
    fn activex_rejects_named_pipe_transport_mapping() {
        let config = ConfigBuilder::new()
            .with_destination(Destination::from_parts("sandbox", 3389))
            .with_username("user")
            .with_password("password")
            .with_client_build(10_000)
            .with_client_dir("C:\\")
            .with_client_name("IronRDP ActiveX")
            .with_platform(MajorPlatformType::WINDOWS)
            .with_transport(TransportKind::NamedPipe {
                path: r"\\.\pipe\test".into(),
            })
            .build()
            .expect("NamedPipe configuration is valid for client builders");

        assert!(matches!(
            active_x_transport_from_client_transport(config.transport()),
            Err("Windows named-pipe transport is not supported by the ActiveX host")
        ));
    }

    #[test]
    fn activex_rpc_rdcleanpath_configuration_uses_activex_property_names() {
        let mut properties = PropertySet::new();
        properties.insert("RDCleanPathUrl", "wss://rdcleanpath.example.test/rdp");
        assert_eq!(
            rdcleanpath_rpc_client_properties(&properties),
            Err("RDCleanPathToken is required when RDCleanPathUrl is configured")
        );

        properties.remove("RDCleanPathUrl");
        properties.insert("RDCleanPathToken", "test-token");
        assert_eq!(
            rdcleanpath_rpc_client_properties(&properties),
            Err("RDCleanPathUrl is required when RDCleanPathToken is configured")
        );

        properties.insert("RDCleanPathUrl", "wss://rdcleanpath.example.test/rdp");
        let client_properties =
            rdcleanpath_rpc_client_properties(&properties).expect("complete ActiveX configuration is valid");
        assert_eq!(
            client_properties.get::<&str>("ironrdp_rdcleanpathurl"),
            Some("wss://rdcleanpath.example.test/rdp")
        );
        assert_eq!(
            client_properties.get::<&str>("ironrdp_rdcleanpathtoken"),
            Some("test-token")
        );

        properties.insert("RDCleanPathToken", "");
        assert_eq!(
            rdcleanpath_rpc_client_properties(&properties),
            Err("RDCleanPathToken must not be empty")
        );

        properties.remove("RDCleanPathUrl");
        properties.remove("RDCleanPathToken");
        properties.insert("ironrdp_rdcleanpathurl", "wss://rdcleanpath.example.test/rdp");
        assert_eq!(
            rdcleanpath_rpc_client_properties(&properties),
            Err("use RDCleanPathUrl and RDCleanPathToken for ActiveX RPC connections")
        );

        properties.remove("ironrdp_rdcleanpathurl");
        properties.insert("RDCleanPathUrl", 42i32);
        assert_eq!(
            rdcleanpath_rpc_client_properties(&properties),
            Err("RDCleanPathUrl must be a string")
        );
    }

    #[test]
    fn extended_settings_expose_rdcleanpath_url_and_protect_token() {
        let control: IMsRdpClient10 = Control::new().into();
        let extended = control
            .cast::<IMsRdpExtendedSettings>()
            .expect("control supports IMsRdpExtendedSettings");

        let mut invalid_url = variant_i32(42);
        let error = unsafe {
            extended
                .put_Property(BSTR::from(ACTIVEX_RDCLEANPATH_URL_PROPERTY).as_ptr(), &mut invalid_url)
                .expect_err("RDCleanPathUrl only accepts VT_BSTR")
        };
        assert_eq!(error.code(), DISP_E_TYPEMISMATCH);

        let mut invalid_scheme = variant_bstr("https://rdcleanpath.example.test/rdp".to_owned());
        let error = unsafe {
            extended
                .put_Property(
                    BSTR::from(ACTIVEX_RDCLEANPATH_URL_PROPERTY).as_ptr(),
                    &mut invalid_scheme,
                )
                .expect_err("RDCleanPathUrl only accepts ws and wss URLs")
        };
        free_owned_bstr_variant(&mut invalid_scheme);
        assert_eq!(error.code(), E_INVALIDARG);

        let mut url = variant_bstr("wss://rdcleanpath.example.test/rdp".to_owned());
        unsafe {
            extended
                .put_Property(BSTR::from(ACTIVEX_RDCLEANPATH_URL_PROPERTY).as_ptr(), &mut url)
                .expect("set RDCleanPath URL");
        }
        free_owned_bstr_variant(&mut url);

        let mut returned_url = VARIANT::default();
        unsafe {
            extended
                .get_Property(BSTR::from(ACTIVEX_RDCLEANPATH_URL_PROPERTY).as_ptr(), &mut returned_url)
                .expect("get RDCleanPath URL");
        }
        assert_eq!(
            variant_bstr_value(&returned_url).expect("RDCleanPath URL BSTR"),
            "wss://rdcleanpath.example.test/rdp"
        );
        free_owned_bstr_variant(&mut returned_url);

        let mut token = variant_bstr("test-token".to_owned());
        unsafe {
            extended
                .put_Property(BSTR::from(ACTIVEX_RDCLEANPATH_TOKEN_PROPERTY).as_ptr(), &mut token)
                .expect("set RDCleanPath token");
        }
        free_owned_bstr_variant(&mut token);

        let mut returned_token = VARIANT::default();
        let error = unsafe {
            extended
                .get_Property(
                    BSTR::from(ACTIVEX_RDCLEANPATH_TOKEN_PROPERTY).as_ptr(),
                    &mut returned_token,
                )
                .expect_err("RDCleanPathToken is write-only")
        };
        assert_eq!(error.code(), E_NOTIMPL);
        assert_eq!(variant_header(&returned_token).vt, VT_EMPTY);
    }

    #[test]
    fn extended_settings_redirect_webauthn_round_trip() {
        let control: IMsRdpClient10 = Control::new().into();
        let extended = control
            .cast::<IMsRdpExtendedSettings>()
            .expect("control supports IMsRdpExtendedSettings");

        let mut default_value = VARIANT::default();
        unsafe {
            extended
                .get_Property(BSTR::from("RedirectWebAuthn").as_ptr(), &mut default_value)
                .expect("get default RedirectWebAuthn");
        }
        assert!(
            variant_bool(&default_value, ptr::null_mut()).expect("RedirectWebAuthn boolean"),
            "RedirectWebAuthn defaults to true"
        );

        let mut disabled = variant_bool_value(false);
        unsafe {
            extended
                .put_Property(BSTR::from("RedirectWebAuthn").as_ptr(), &mut disabled)
                .expect("disable RedirectWebAuthn");
        }
        let mut returned = VARIANT::default();
        unsafe {
            extended
                .get_Property(BSTR::from("RedirectWebAuthn").as_ptr(), &mut returned)
                .expect("get RedirectWebAuthn after disable");
        }
        assert!(!variant_bool(&returned, ptr::null_mut()).expect("RedirectWebAuthn boolean"));

        let mut enabled = variant_bool_value(true);
        unsafe {
            extended
                .put_Property(BSTR::from("RedirectWebAuthn").as_ptr(), &mut enabled)
                .expect("enable RedirectWebAuthn");
        }
        let mut returned = VARIANT::default();
        unsafe {
            extended
                .get_Property(BSTR::from("RedirectWebAuthn").as_ptr(), &mut returned)
                .expect("get RedirectWebAuthn after enable");
        }
        assert!(variant_bool(&returned, ptr::null_mut()).expect("RedirectWebAuthn boolean"));
    }

    #[test]
    fn rdcleanpath_settings_require_a_complete_pair_and_mutable_connection_settings() {
        let mut settings = RDCleanPathSettings::default();
        settings
            .set_url("wss://rdcleanpath.example.test/rdp".to_owned())
            .expect("RDCleanPath URL is valid");
        let error = match settings.transport() {
            Ok(_) => panic!("RDCleanPath token is required"),
            Err(error) => error,
        };
        assert_eq!(error.code(), E_INVALIDARG);

        let control = Control::new();
        control.compatibility.borrow_mut().connection_settings_sealed = true;
        let extended: IMsRdpExtendedSettings = control.into();
        let mut url = variant_bstr("wss://rdcleanpath.example.test/rdp".to_owned());
        let error = unsafe {
            extended
                .put_Property(BSTR::from(ACTIVEX_RDCLEANPATH_URL_PROPERTY).as_ptr(), &mut url)
                .expect_err("RDCleanPath settings are immutable after connection settings are sealed")
        };
        free_owned_bstr_variant(&mut url);
        assert_eq!(error.code(), E_UNEXPECTED);
    }

    #[implement(IDispatch)]
    struct ConfirmCloseVetoSink;

    impl IDispatch_Impl for ConfirmCloseVetoSink_Impl {
        fn GetTypeInfoCount(&self) -> Result<u32> {
            Ok(0)
        }

        fn GetTypeInfo(&self, _itinfo: u32, _lcid: u32) -> Result<ITypeInfo> {
            Err(Error::from_hresult(E_NOTIMPL))
        }

        fn GetIDsOfNames(
            &self,
            _riid: *const GUID,
            _names: *const PCWSTR,
            _count: u32,
            _lcid: u32,
            _dispids: *mut i32,
        ) -> Result<()> {
            Err(Error::from_hresult(DISP_E_UNKNOWNNAME))
        }

        fn Invoke(
            &self,
            dispid: i32,
            _riid: *const GUID,
            _lcid: u32,
            flags: DISPATCH_FLAGS,
            params: *const DISPPARAMS,
            _result: *mut VARIANT,
            _exception: *mut EXCEPINFO,
            _argument_error: *mut u32,
        ) -> Result<()> {
            assert_eq!(dispid, DISPID_ON_CONFIRM_CLOSE);
            assert!(flags.contains(DISPATCH_METHOD));
            let params = unsafe { params.as_ref() }.ok_or_else(|| Error::from_hresult(E_POINTER))?;
            assert_eq!(params.cArgs, 1);
            let argument = unsafe { params.rgvarg.as_ref() }.ok_or_else(|| Error::from_hresult(E_POINTER))?;
            let header = variant_header(argument);
            assert_eq!(header.vt, VT_BOOL | VT_BYREF);
            let allow_close =
                unsafe { header.Anonymous.pboolVal.as_mut() }.ok_or_else(|| Error::from_hresult(E_POINTER))?;
            *allow_close = VARIANT_FALSE;
            Ok(())
        }
    }

    #[implement(IAdviseSink)]
    struct OleAdviseSink {
        views: Arc<AtomicU32>,
        saves: Arc<AtomicU32>,
        closes: Arc<AtomicU32>,
    }

    impl IAdviseSink_Impl for OleAdviseSink_Impl {
        fn OnDataChange(&self, _format: *const FORMATETC, _storage: *const STGMEDIUM) {}

        fn OnViewChange(&self, aspect: u32, index: i32) {
            assert_eq!(aspect, DVASPECT_CONTENT.0);
            assert_eq!(index, -1);
            self.views.fetch_add(1, Ordering::Relaxed);
        }

        fn OnRename(&self, _moniker: Ref<'_, IMoniker>) {}

        fn OnSave(&self) {
            self.saves.fetch_add(1, Ordering::Relaxed);
        }

        fn OnClose(&self) {
            self.closes.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[implement(IDispatch)]
    struct ChannelDataSink {
        seen: Arc<Mutex<Option<(String, String)>>>,
    }

    impl IDispatch_Impl for ChannelDataSink_Impl {
        fn GetTypeInfoCount(&self) -> Result<u32> {
            Ok(0)
        }

        fn GetTypeInfo(&self, _itinfo: u32, _lcid: u32) -> Result<ITypeInfo> {
            Err(Error::from_hresult(E_NOTIMPL))
        }

        fn GetIDsOfNames(
            &self,
            _riid: *const GUID,
            _names: *const PCWSTR,
            _count: u32,
            _lcid: u32,
            _dispids: *mut i32,
        ) -> Result<()> {
            Err(Error::from_hresult(DISP_E_UNKNOWNNAME))
        }

        fn Invoke(
            &self,
            dispid: i32,
            _riid: *const GUID,
            _lcid: u32,
            flags: DISPATCH_FLAGS,
            params: *const DISPPARAMS,
            _result: *mut VARIANT,
            _exception: *mut EXCEPINFO,
            _argument_error: *mut u32,
        ) -> Result<()> {
            assert_eq!(dispid, DISPID_ON_CHANNEL_RECEIVED_DATA);
            assert!(flags.contains(DISPATCH_METHOD));
            let params = unsafe { params.as_ref() }.ok_or_else(|| Error::from_hresult(E_POINTER))?;
            assert_eq!(params.cArgs, 2);
            let arguments = unsafe { slice::from_raw_parts(params.rgvarg, params.cArgs as usize) };
            let data = variant_bstr_value(&arguments[0])?;
            let channel_name = variant_bstr_value(&arguments[1])?;
            *self.seen.lock().expect("event sink state") = Some((channel_name, data));
            Ok(())
        }
    }

    #[implement(IDispatch)]
    struct LifecycleSink {
        seen: Arc<Mutex<Vec<i32>>>,
    }

    impl IDispatch_Impl for LifecycleSink_Impl {
        fn GetTypeInfoCount(&self) -> Result<u32> {
            Ok(0)
        }

        fn GetTypeInfo(&self, _itinfo: u32, _lcid: u32) -> Result<ITypeInfo> {
            Err(Error::from_hresult(E_NOTIMPL))
        }

        fn GetIDsOfNames(
            &self,
            _riid: *const GUID,
            _names: *const PCWSTR,
            _count: u32,
            _lcid: u32,
            _dispids: *mut i32,
        ) -> Result<()> {
            Err(Error::from_hresult(DISP_E_UNKNOWNNAME))
        }

        fn Invoke(
            &self,
            dispid: i32,
            _riid: *const GUID,
            _lcid: u32,
            flags: DISPATCH_FLAGS,
            params: *const DISPPARAMS,
            _result: *mut VARIANT,
            _exception: *mut EXCEPINFO,
            _argument_error: *mut u32,
        ) -> Result<()> {
            assert!(matches!(dispid, DISPID_ON_CONNECTED | DISPID_ON_LOGIN_COMPLETE));
            assert!(flags.contains(DISPATCH_METHOD));
            let params = unsafe { params.as_ref() }.ok_or_else(|| Error::from_hresult(E_POINTER))?;
            assert_eq!(params.cArgs, 0);
            self.seen.lock().expect("lifecycle events are available").push(dispid);
            Ok(())
        }
    }

    fn variant_bstr_value(value: &VARIANT) -> Result<String> {
        let header = variant_header(value);
        if header.vt != VT_BSTR {
            return Err(Error::from_hresult(DISP_E_TYPEMISMATCH));
        }
        let bstr = unsafe { &*(&header.Anonymous.bstrVal as *const ManuallyDrop<BSTR> as *const BSTR) };
        String::try_from(bstr).map_err(|_| Error::from_hresult(DISP_E_TYPEMISMATCH))
    }

    fn variant_u32(value: u32) -> VARIANT {
        VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                    vt: VT_UI4,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: VARIANT_0_0_0 { ulVal: value },
                }),
            },
        }
    }

    #[test]
    fn published_automation_names_use_their_classic_dispids() {
        assert_eq!(dispid_for_name("Server"), Some(DISPID_SERVER));
        assert_eq!(dispid_for_name("CONNECT"), Some(DISPID_CONNECT));
        assert_eq!(dispid_for_name("FullScreen"), Some(DISPID_FULLSCREEN));
        assert_eq!(
            dispid_for_name("ConnectedStatusText"),
            Some(DISPID_CONNECTED_STATUS_TEXT)
        );
        assert_eq!(dispid_for_name("FullScreenTitle"), Some(DISPID_FULLSCREEN_TITLE));
        assert_eq!(dispid_for_name("CipherStrength"), Some(DISPID_CIPHER_STRENGTH));
        assert_eq!(
            dispid_for_name("ExtendedDisconnectReason"),
            Some(DISPID_EXTENDED_DISCONNECT_REASON)
        );
        assert_eq!(dispid_for_name("unknown"), None);
    }

    #[test]
    fn confirm_close_uses_a_boolean_by_reference_automation_argument() {
        let mut allow_close = VARIANT_TRUE;
        let argument = variant_bool_byref(&mut allow_close);
        let header = variant_header(&argument);
        assert_eq!(header.vt, VT_BOOL | VT_BYREF);

        unsafe {
            *header.Anonymous.pboolVal = VARIANT_FALSE;
        }
        assert_eq!(allow_close, VARIANT_FALSE);
    }

    #[test]
    fn request_close_allows_closing_when_no_event_sink_vetoes_it() {
        let control = Control::new();
        assert_eq!(control.request_close_status(), CONTROL_CLOSE_CAN_PROCEED);

        control.set_events_frozen(true);
        assert_eq!(control.request_close_status(), CONTROL_CLOSE_CAN_PROCEED);
    }

    #[test]
    fn freeze_events_requires_a_matching_final_unfreeze() {
        let control = Control::new();
        let lifecycle_events = Arc::new(Mutex::new(Vec::new()));
        let dispatch: IDispatch = LifecycleSink {
            seen: Arc::clone(&lifecycle_events),
        }
        .into();
        control.sinks.borrow_mut().insert(1, EventSink { cookie: 1, dispatch });

        control.fire_event(DISPID_ON_CONNECTED, &[]);
        assert_eq!(
            *lifecycle_events.lock().expect("lifecycle events are available"),
            [DISPID_ON_CONNECTED]
        );

        control.set_events_frozen(true);
        control.set_events_frozen(true);
        assert!(control.events_are_frozen());
        control.fire_event(DISPID_ON_CONNECTED, &[]);
        assert_eq!(
            *lifecycle_events.lock().expect("lifecycle events are available"),
            [DISPID_ON_CONNECTED]
        );

        control.set_events_frozen(false);
        assert!(control.events_are_frozen());
        control.fire_event(DISPID_ON_CONNECTED, &[]);
        assert_eq!(
            *lifecycle_events.lock().expect("lifecycle events are available"),
            [DISPID_ON_CONNECTED]
        );

        control.set_events_frozen(false);
        assert!(!control.events_are_frozen());
        control.fire_event(DISPID_ON_CONNECTED, &[]);
        assert_eq!(
            *lifecycle_events.lock().expect("lifecycle events are available"),
            [DISPID_ON_CONNECTED, DISPID_ON_CONNECTED]
        );

        control.set_events_frozen(false);
        assert!(!control.events_are_frozen());

        let channel_data = Arc::new(Mutex::new(None));
        let dispatch: IDispatch = ChannelDataSink {
            seen: Arc::clone(&channel_data),
        }
        .into();
        control.sinks.borrow_mut().clear();
        control.sinks.borrow_mut().insert(2, EventSink { cookie: 2, dispatch });
        control.set_events_frozen(true);
        control.fire_channel_received_data("alpha", &[0, 0xff]);
        assert!(channel_data.lock().expect("channel event state").is_none());
        control.set_events_frozen(false);
        control.fire_channel_received_data("alpha", &[0, 0xff]);
        assert_eq!(
            channel_data.lock().expect("channel event state").as_ref(),
            Some(&("alpha".to_owned(), "\0\u{ff}".to_owned()))
        );

        control.sinks.borrow_mut().clear();
        let dispatch: IDispatch = ConfirmCloseVetoSink.into();
        control.sinks.borrow_mut().insert(3, EventSink { cookie: 3, dispatch });
        control.set_events_frozen(true);
        assert_eq!(control.request_close_status(), CONTROL_CLOSE_CAN_PROCEED);
        control.set_events_frozen(false);
        assert_eq!(control.request_close_status(), CONTROL_CLOSE_WAIT_FOR_EVENTS);
    }

    #[test]
    fn request_close_honors_a_connection_point_veto() {
        let control = Control::new();
        let dispatch: IDispatch = ConfirmCloseVetoSink.into();
        control.sinks.borrow_mut().insert(1, EventSink { cookie: 1, dispatch });

        assert_eq!(control.request_close_status(), CONTROL_CLOSE_WAIT_FOR_EVENTS);
    }

    #[test]
    fn pre_credential_mstsc_startup_remains_idle() {
        let control = Control::new();

        control
            .start_connection()
            .expect("a startup form without credentials must not attempt a connection");
        assert_eq!(control.state.get(), ConnectionState::Disconnected);

        control.settings.borrow_mut().server = "example.test".to_owned();
        control
            .start_connection()
            .expect("a pre-credential destination must remain idle");
        assert_eq!(control.state.get(), ConnectionState::Disconnected);
    }

    #[test]
    fn credssp_uses_the_secure_default_until_the_host_overrides_it() {
        let control = Control::new();
        assert_eq!(control.compatibility.borrow().enable_credssp, None);

        control.compatibility.borrow_mut().enable_credssp = Some(false);
        assert_eq!(control.compatibility.borrow().enable_credssp, Some(false));
    }

    #[test]
    fn gateway_transport_maps_only_honored_public_modes() {
        let settings = Settings {
            domain: "RDP".to_owned(),
            username: "server-user".to_owned(),
            password: Some("server-password".to_owned()),
            ..Settings::default()
        };
        let mut compatibility = CompatibilitySettings {
            gateway_hostname: "gateway.example.test:443".to_owned(),
            gateway_usage_method: GatewayUsageMethod::UseAlways.as_i64() as u32,
            ..CompatibilitySettings::default()
        };

        let ActiveXTransport::Gateway {
            endpoint,
            username,
            password,
        } = active_x_transport(&settings, &compatibility).expect("explicit gateway is supported")
        else {
            panic!("expected gateway transport");
        };
        assert_eq!(endpoint, "gateway.example.test:443");
        assert_eq!(username, "RDP\\server-user");
        assert_eq!(password, "server-password");

        compatibility.gateway_creds_source = GatewayCredentialsSource::UseUserCredentials.as_i64() as u32;
        compatibility.gateway_domain = "GATEWAY".to_owned();
        compatibility.gateway_username = "gateway-user".to_owned();
        compatibility.gateway_password = "gateway-password".to_owned();
        let ActiveXTransport::Gateway { username, password, .. } =
            active_x_transport(&settings, &compatibility).expect("gateway user credentials are supported")
        else {
            panic!("expected gateway transport");
        };
        assert_eq!(username, "GATEWAY\\gateway-user");
        assert_eq!(password, "gateway-password");

        compatibility.gateway_usage_method = GatewayUsageMethod::UseDefaultSettings.as_i64() as u32;
        let system_policy_error = match active_x_transport(&settings, &compatibility) {
            Ok(_) => panic!("system policy must not be silently approximated"),
            Err(error) => error,
        };
        assert_eq!(system_policy_error.code(), E_NOTIMPL);

        compatibility.gateway_usage_method = GatewayUsageMethod::UseAlways.as_i64() as u32;
        compatibility.gateway_creds_source = GatewayCredentialsSource::Prompt.as_i64() as u32;
        let prompt_error = match active_x_transport(&settings, &compatibility) {
            Ok(_) => panic!("gateway prompting is not implemented"),
            Err(error) => error,
        };
        assert_eq!(prompt_error.code(), E_NOTIMPL);
    }

    #[test]
    fn gateway_transport_setters_validate_public_enums() {
        let settings = Rc::new(RefCell::new(CompatibilitySettings::default()));
        let mut object = CompatibilitySettingsObject {
            vtable: transport_vtable(),
            references: AtomicU32::new(1),
            settings: Rc::clone(&settings),
            native_mstsc_credential_bridge: None,
            server_object: false,
        };
        let this = (&mut object as *mut TransportSettingsObject).cast::<c_void>();

        assert_eq!(
            unsafe { transport_put_gateway_usage_method(this, GatewayUsageMethod::Detect.as_i64() as u32) },
            S_OK
        );
        assert_eq!(unsafe { transport_put_gateway_usage_method(this, 99) }, E_INVALIDARG);
        assert_eq!(
            unsafe {
                transport_put_gateway_creds_source(this, GatewayCredentialsSource::UseUserCredentials.as_i64() as u32)
            },
            S_OK
        );
        assert_eq!(unsafe { transport_put_gateway_creds_source(this, 99) }, E_INVALIDARG);

        let hostname = BSTR::from("gateway.example.test:443");
        let username = BSTR::from("gateway-user\0suffix");
        let domain = BSTR::from("GATEWAY");
        let password = BSTR::from("gateway-password");
        assert_eq!(unsafe { transport_put_gateway_hostname(this, hostname.as_ptr()) }, S_OK);
        assert_eq!(unsafe { transport_put_gateway_username(this, username.as_ptr()) }, S_OK);
        assert_eq!(unsafe { transport_put_gateway_domain(this, domain.as_ptr()) }, S_OK);
        assert_eq!(unsafe { transport_put_gateway_password(this, password.as_ptr()) }, S_OK);

        let mut returned_username = ptr::null();
        let mut returned_domain = ptr::null();
        assert_eq!(
            unsafe { transport_get_gateway_username(this, &mut returned_username) },
            S_OK
        );
        assert_eq!(
            unsafe { transport_get_gateway_domain(this, &mut returned_domain) },
            S_OK
        );
        let returned_username = unsafe { BSTR::from_raw(returned_username) };
        let returned_domain = unsafe { BSTR::from_raw(returned_domain) };
        assert_eq!(
            String::try_from(&returned_username).expect("valid gateway username BSTR"),
            "gateway-user\0suffix"
        );
        assert_eq!(
            String::try_from(&returned_domain).expect("valid gateway domain BSTR"),
            "GATEWAY"
        );

        let mut auth_cookie_size = u32::MAX;
        assert_eq!(
            unsafe { transport_get_u32_not_implemented(this, &mut auth_cookie_size) },
            E_NOTIMPL
        );
        assert_eq!(auth_cookie_size, 0);
        let mut auth_cookie = ptr::dangling::<u16>();
        assert_eq!(
            unsafe { transport_get_bstr_not_implemented(this, &mut auth_cookie) },
            E_NOTIMPL
        );
        assert!(auth_cookie.is_null());

        let vtable = transport_vtable();
        assert_eq!(vtable.slots.len(), TRANSPORT_SETTINGS_SLOTS);
        assert_eq!(vtable.slots[0], transport_put_gateway_hostname as *const () as usize);
        assert_eq!(
            vtable.slots[2],
            transport_put_gateway_usage_method as *const () as usize
        );
        assert_eq!(
            vtable.slots[6],
            transport_put_gateway_creds_source as *const () as usize
        );
        assert_eq!(vtable.slots[24], transport_put_gateway_username as *const () as usize);
        assert_eq!(vtable.slots[25], transport_get_gateway_username as *const () as usize);
        assert_eq!(vtable.slots[26], transport_put_gateway_domain as *const () as usize);
        assert_eq!(vtable.slots[27], transport_get_gateway_domain as *const () as usize);
        assert_eq!(vtable.slots[28], transport_put_gateway_password as *const () as usize);
        assert_eq!(
            vtable.slots[29],
            transport_put_u32_not_implemented as *const () as usize
        );
        assert_eq!(
            vtable.slots[30],
            transport_get_u32_not_implemented as *const () as usize
        );
        assert_eq!(
            vtable.slots[31],
            transport_put_bstr_not_implemented as *const () as usize
        );
        assert_eq!(
            vtable.slots[32],
            transport_get_bstr_not_implemented as *const () as usize
        );
        assert_eq!(
            vtable.slots[33],
            transport_put_bstr_not_implemented as *const () as usize
        );
        assert_eq!(
            vtable.slots[34],
            transport_get_bstr_not_implemented as *const () as usize
        );
        assert_eq!(
            vtable.slots[35],
            transport_put_u32_not_implemented as *const () as usize
        );
        assert_eq!(
            vtable.slots[36],
            transport_get_u32_not_implemented as *const () as usize
        );
        assert_eq!(
            vtable.slots[37],
            transport_put_bstr_not_implemented as *const () as usize
        );
        assert_eq!(
            vtable.slots[38],
            transport_get_bstr_not_implemented as *const () as usize
        );
        assert_eq!(
            vtable.slots[39],
            transport_put_u32_not_implemented as *const () as usize
        );
        assert!(settings_supports_interface::<TRANSPORT_SETTINGS_SLOTS>(
            &GUID::from_u128(0x3d5b21ac_748d_41de_8f30_e15169586bd4)
        ));
        assert!(settings_supports_interface::<TRANSPORT_SETTINGS_SLOTS>(
            &GUID::from_u128(0x011c3236_4d81_4515_9143_067ab630d299)
        ));
    }

    #[test]
    fn advanced_settings_map_preconnect_transport_and_capability_slots() {
        let persistence_dirty = Rc::new(Cell::new(false));
        let compatibility = CompatibilitySettings {
            persistence_dirty: Some(Rc::clone(&persistence_dirty)),
            ..Default::default()
        };
        let settings = Rc::new(RefCell::new(compatibility));
        let mut object = CompatibilitySettingsObject {
            vtable: advanced_vtable(),
            references: AtomicU32::new(1),
            settings: Rc::clone(&settings),
            native_mstsc_credential_bridge: None,
            server_object: false,
        };
        let this = (&mut object as *mut AdvancedSettingsObject).cast::<c_void>();

        let mut auto_reconnect = VARIANT_FALSE.0;
        assert_eq!(
            unsafe { advanced_get_enable_auto_reconnect(this, &mut auto_reconnect) },
            S_OK
        );
        assert_eq!(auto_reconnect, VARIANT_TRUE.0);
        assert_eq!(
            unsafe { advanced_put_enable_auto_reconnect(this, VARIANT_FALSE.0) },
            S_OK
        );
        assert!(!settings.borrow().enable_auto_reconnect);

        let mut max_reconnect_attempts = 0;
        assert_eq!(
            unsafe { advanced_get_max_reconnect_attempts(this, &mut max_reconnect_attempts) },
            S_OK
        );
        assert_eq!(max_reconnect_attempts, 20);
        assert_eq!(unsafe { advanced_put_max_reconnect_attempts(this, 3) }, S_OK);
        assert_eq!(settings.borrow().max_reconnect_attempts, 3);
        assert_eq!(unsafe { advanced_put_max_reconnect_attempts(this, -1) }, E_INVALIDARG);
        assert_eq!(
            unsafe {
                advanced_put_max_reconnect_attempts(
                    this,
                    i32::try_from(MAX_RECONNECT_ATTEMPTS + 1).expect("limit fits in i32"),
                )
            },
            E_INVALIDARG
        );
        settings.borrow_mut().connection_settings_sealed = true;
        assert_eq!(
            unsafe { advanced_put_enable_auto_reconnect(this, VARIANT_TRUE.0) },
            E_FAIL
        );
        assert_eq!(unsafe { advanced_put_max_reconnect_attempts(this, 4) }, E_FAIL);
        assert_eq!(unsafe { advanced_put_max_reconnect_attempts(this, -1) }, E_FAIL);
        assert!(!settings.borrow().enable_auto_reconnect);
        assert_eq!(settings.borrow().max_reconnect_attempts, 3);
        settings.borrow_mut().connection_settings_sealed = false;

        assert_eq!(unsafe { advanced_put_compress(this, 0) }, S_OK);
        let mut compression = -1;
        assert_eq!(unsafe { advanced_get_compress(this, &mut compression) }, S_OK);
        assert_eq!(compression, 0);
        assert_eq!(unsafe { advanced_put_compress(this, 2) }, E_INVALIDARG);

        let password = BSTR::from("test-password");
        assert_eq!(
            unsafe { advanced_put_clear_text_password(this, password.as_ptr()) },
            S_OK
        );
        assert_eq!(settings.borrow().clear_text_password.as_deref(), Some("test-password"));

        assert_eq!(unsafe { advanced_put_allow_background_input(this, -1) }, S_OK);
        let mut allow_background_input = 0;
        assert_eq!(
            unsafe { advanced_get_allow_background_input(this, &mut allow_background_input) },
            S_OK
        );
        assert_eq!(allow_background_input, -1);
        assert_eq!(unsafe { advanced_put_allow_background_input(this, 1) }, S_OK);
        assert_eq!(unsafe { advanced_put_allow_background_input(this, 2) }, E_INVALIDARG);

        assert_eq!(
            unsafe { advanced_put_display_connection_bar(this, VARIANT_TRUE.0) },
            S_OK
        );
        let mut display_connection_bar = VARIANT_FALSE.0;
        assert_eq!(
            unsafe { advanced_get_display_connection_bar(this, &mut display_connection_bar) },
            S_OK
        );
        assert_eq!(display_connection_bar, VARIANT_TRUE.0);
        assert_eq!(unsafe { advanced_put_display_connection_bar(this, 1) }, S_OK);
        assert_eq!(settings.borrow().display_connection_bar, VARIANT_TRUE.0);
        assert!(settings.borrow().display_connection_bar_set);

        assert_eq!(unsafe { advanced_put_pin_connection_bar(this, VARIANT_TRUE.0) }, S_OK);
        let mut pin_connection_bar = VARIANT_FALSE.0;
        assert_eq!(
            unsafe { advanced_get_pin_connection_bar(this, &mut pin_connection_bar) },
            S_OK
        );
        assert_eq!(pin_connection_bar, VARIANT_TRUE.0);
        assert_eq!(unsafe { advanced_put_pin_connection_bar(this, 1) }, S_OK);
        assert_eq!(settings.borrow().pin_connection_bar, VARIANT_TRUE.0);

        assert_eq!(
            unsafe { advanced_put_connection_bar_show_minimize_button(this, VARIANT_TRUE.0) },
            S_OK
        );
        let mut show_minimize_button = VARIANT_FALSE.0;
        assert_eq!(
            unsafe { advanced_get_connection_bar_show_minimize_button(this, &mut show_minimize_button) },
            S_OK
        );
        assert_eq!(show_minimize_button, VARIANT_TRUE.0);
        assert_eq!(
            unsafe { advanced_get_connection_bar_show_minimize_button(this, ptr::null_mut()) },
            E_POINTER
        );

        assert_eq!(
            unsafe { advanced_put_connection_bar_show_restore_button(this, 1) },
            S_OK
        );
        let mut show_restore_button = VARIANT_FALSE.0;
        assert_eq!(
            unsafe { advanced_get_connection_bar_show_restore_button(this, &mut show_restore_button) },
            S_OK
        );
        assert_eq!(show_restore_button, VARIANT_TRUE.0);
        assert_eq!(
            unsafe { advanced_get_connection_bar_show_restore_button(this, ptr::null_mut()) },
            E_POINTER
        );

        assert_eq!(
            unsafe { advanced_put_connection_bar_show_pin_button(this, VARIANT_FALSE.0) },
            S_OK
        );
        let mut show_pin_button = VARIANT_TRUE.0;
        assert_eq!(
            unsafe { advanced_get_connection_bar_show_pin_button(this, &mut show_pin_button) },
            S_OK
        );
        assert_eq!(show_pin_button, VARIANT_FALSE.0);
        assert_eq!(
            unsafe { advanced_get_connection_bar_show_pin_button(this, ptr::null_mut()) },
            E_POINTER
        );

        let mut authentication_type = u32::MAX;
        assert_eq!(
            unsafe { advanced_get_authentication_type(this, &mut authentication_type) },
            S_OK
        );
        assert_eq!(authentication_type, 0);

        assert_eq!(unsafe { advanced_put_rdp_port(this, 3390) }, S_OK);
        let mut port = 0;
        assert_eq!(unsafe { advanced_get_rdp_port(this, &mut port) }, S_OK);
        assert_eq!(port, 3390);
        assert_eq!(unsafe { advanced_put_rdp_port(this, 0) }, E_INVALIDARG);

        let mut disable_rdpdr = 0;
        assert_eq!(unsafe { advanced_get_disable_rdpdr(this, &mut disable_rdpdr) }, S_OK);
        assert_eq!(disable_rdpdr, 0);
        assert_eq!(unsafe { advanced_put_disable_rdpdr(this, 1) }, S_OK);
        assert_eq!(unsafe { advanced_get_disable_rdpdr(this, &mut disable_rdpdr) }, S_OK);
        assert_eq!(disable_rdpdr, 1);
        assert_eq!(unsafe { advanced_put_disable_rdpdr(this, 0) }, S_OK);

        assert_eq!(unsafe { advanced_put_redirect_drives(this, VARIANT_TRUE.0) }, S_OK);
        let mut redirect_drives = VARIANT_FALSE.0;
        assert_eq!(
            unsafe { advanced_get_redirect_drives(this, &mut redirect_drives) },
            S_OK
        );
        assert_eq!(redirect_drives, VARIANT_TRUE.0);

        assert_eq!(unsafe { advanced_put_redirect_smart_cards(this, VARIANT_TRUE.0) }, S_OK);
        let mut redirect_smart_cards = VARIANT_FALSE.0;
        assert_eq!(
            unsafe { advanced_get_redirect_smart_cards(this, &mut redirect_smart_cards) },
            S_OK
        );
        assert_eq!(redirect_smart_cards, VARIANT_TRUE.0);
        assert_eq!(
            unsafe { advanced_get_redirect_smart_cards(this, ptr::null_mut()) },
            E_POINTER
        );

        assert_eq!(unsafe { advanced_put_enable_mouse(this, 0) }, S_OK);
        let mut enable_mouse = -1;
        assert_eq!(unsafe { advanced_get_enable_mouse(this, &mut enable_mouse) }, S_OK);
        assert_eq!(enable_mouse, 0);
        assert_eq!(unsafe { advanced_put_enable_mouse(this, -1) }, S_OK);
        assert_eq!(unsafe { advanced_get_enable_mouse(this, &mut enable_mouse) }, S_OK);
        assert_eq!(enable_mouse, 1);

        assert_eq!(unsafe { advanced_put_enable_windows_key(this, 0) }, S_OK);
        let mut enable_windows_key = -1;
        assert_eq!(
            unsafe { advanced_get_enable_windows_key(this, &mut enable_windows_key) },
            S_OK
        );
        assert_eq!(enable_windows_key, 0);
        assert_eq!(unsafe { advanced_put_enable_windows_key(this, -1) }, S_OK);
        assert_eq!(
            unsafe { advanced_get_enable_windows_key(this, &mut enable_windows_key) },
            S_OK
        );
        assert_eq!(enable_windows_key, 1);

        let performance_flags = (PerformanceFlags::DISABLE_WALLPAPER | PerformanceFlags::ENABLE_FONT_SMOOTHING).bits();
        assert_eq!(
            unsafe { advanced_put_performance_flags(this, performance_flags as i32) },
            S_OK
        );
        let mut observed_performance_flags = 0;
        assert_eq!(
            unsafe { advanced_get_performance_flags(this, &mut observed_performance_flags) },
            S_OK
        );
        assert_eq!(observed_performance_flags as u32, performance_flags);
        assert_eq!(unsafe { advanced_put_performance_flags(this, i32::MAX) }, E_INVALIDARG);
        assert!(persistence_dirty.get());

        assert_eq!(unsafe { advanced_put_keyboard_type(this, 7) }, S_OK);
        let mut keyboard_type = 0;
        assert_eq!(unsafe { advanced_get_keyboard_type(this, &mut keyboard_type) }, S_OK);
        assert_eq!(keyboard_type, 7);
        assert_eq!(unsafe { advanced_put_keyboard_type(this, 8) }, E_INVALIDARG);

        assert_eq!(unsafe { advanced_put_keyboard_subtype(this, 42) }, S_OK);
        let mut keyboard_subtype = 0;
        assert_eq!(
            unsafe { advanced_get_keyboard_subtype(this, &mut keyboard_subtype) },
            S_OK
        );
        assert_eq!(keyboard_subtype, 42);
        assert_eq!(unsafe { advanced_put_keyboard_subtype(this, -1) }, E_INVALIDARG);

        assert_eq!(unsafe { advanced_put_keyboard_function_key(this, 24) }, S_OK);
        let mut keyboard_functional_keys_count = 0;
        assert_eq!(
            unsafe { advanced_get_keyboard_function_key(this, &mut keyboard_functional_keys_count) },
            S_OK
        );
        assert_eq!(keyboard_functional_keys_count, 24);
        assert_eq!(unsafe { advanced_put_keyboard_function_key(this, -1) }, E_INVALIDARG);

        assert_eq!(
            unsafe { advanced_put_grab_focus_on_connect(this, VARIANT_TRUE.0) },
            S_OK
        );
        let mut grab_focus_on_connect = VARIANT_FALSE.0;
        assert_eq!(
            unsafe { advanced_get_grab_focus_on_connect(this, &mut grab_focus_on_connect) },
            S_OK
        );
        assert_eq!(grab_focus_on_connect, VARIANT_TRUE.0);

        assert_eq!(unsafe { advanced_put_redirect_clipboard(this, VARIANT_FALSE.0) }, S_OK);
        let mut redirect_clipboard = VARIANT_TRUE.0;
        assert_eq!(
            unsafe { advanced_get_redirect_clipboard(this, &mut redirect_clipboard) },
            S_OK
        );
        assert_eq!(redirect_clipboard, VARIANT_FALSE.0);

        assert_eq!(unsafe { advanced_put_audio_redirection(this, 2) }, S_OK);
        let mut audio_redirection_mode = u32::MAX;
        assert_eq!(
            unsafe { advanced_get_audio_redirection(this, &mut audio_redirection_mode) },
            S_OK
        );
        assert_eq!(audio_redirection_mode, 2);
        assert_eq!(unsafe { advanced_put_audio_redirection(this, 3) }, E_INVALIDARG);

        assert_eq!(
            unsafe { advanced_put_audio_capture_redirection_mode(this, VARIANT_TRUE.0) },
            S_OK
        );
        let mut audio_capture_mode = VARIANT_FALSE.0;
        assert_eq!(
            unsafe { advanced_get_audio_capture_redirection_mode(this, &mut audio_capture_mode) },
            S_OK
        );
        assert_eq!(audio_capture_mode, VARIANT_TRUE.0);
        // Non-zero values normalize to VARIANT_TRUE.
        assert_eq!(unsafe { advanced_put_audio_capture_redirection_mode(this, 1) }, S_OK);
        assert_eq!(
            unsafe { advanced_get_audio_capture_redirection_mode(this, &mut audio_capture_mode) },
            S_OK
        );
        assert_eq!(audio_capture_mode, VARIANT_TRUE.0);
        assert_eq!(
            unsafe { advanced_put_audio_capture_redirection_mode(this, VARIANT_FALSE.0) },
            S_OK
        );

        assert_eq!(unsafe { advanced_put_authentication_level(this, 2) }, S_OK);
        let mut authentication_level = u32::MAX;
        assert_eq!(
            unsafe { advanced_get_authentication_level(this, &mut authentication_level) },
            S_OK
        );
        assert_eq!(authentication_level, 2);
        assert_eq!(unsafe { advanced_put_authentication_level(this, 4) }, E_INVALIDARG);

        assert_eq!(unsafe { advanced_put_public_mode(this, VARIANT_TRUE.0) }, S_OK);
        let mut public_mode = VARIANT_FALSE.0;
        assert_eq!(unsafe { advanced_get_public_mode(this, &mut public_mode) }, S_OK);
        assert_eq!(public_mode, VARIANT_TRUE.0);
        assert_eq!(unsafe { advanced_put_public_mode(this, 1) }, E_INVALIDARG);

        let mut disabled = VARIANT_TRUE.0;
        assert_eq!(unsafe { advanced_get_redirect_directx(this, &mut disabled) }, S_OK);
        assert_eq!(disabled, VARIANT_FALSE.0);
        assert_eq!(
            unsafe { advanced_put_redirect_directx(this, VARIANT_TRUE.0) },
            E_NOTIMPL
        );
        let keyboard_layout = BSTR::from("00000409");
        assert_eq!(
            unsafe { advanced_put_keyboard_layout_str(this, keyboard_layout.as_ptr()) },
            S_OK
        );
        assert_eq!(settings.borrow().keyboard_layout, 0x0000_0409);
        let invalid_keyboard_layout = BSTR::from("409");
        assert_eq!(
            unsafe { advanced_put_keyboard_layout_str(this, invalid_keyboard_layout.as_ptr()) },
            E_INVALIDARG
        );
        assert_eq!(unsafe { advanced_put_network_connection_type(this, 2) }, S_OK);
        let mut network_connection_type = 0;
        assert_eq!(
            unsafe { advanced_get_network_connection_type(this, &mut network_connection_type) },
            S_OK
        );
        assert_eq!(network_connection_type, 2);
        assert_eq!(unsafe { advanced_put_network_connection_type(this, 0) }, E_INVALIDARG);
        let mut bandwidth_detection = VARIANT_TRUE.0;
        assert_eq!(
            unsafe { advanced_get_bandwidth_detection(this, &mut bandwidth_detection) },
            E_NOTIMPL
        );
        assert_eq!(bandwidth_detection, VARIANT_FALSE.0);
        let mut client_protocol_spec = -1;
        assert_eq!(
            unsafe { advanced_get_client_protocol_spec(this, &mut client_protocol_spec) },
            E_NOTIMPL
        );
        assert_eq!(client_protocol_spec, 0);
        assert_eq!(
            unsafe { advanced_put_bandwidth_detection(this, VARIANT_FALSE.0) },
            E_NOTIMPL
        );
        assert_eq!(unsafe { advanced_put_client_protocol_spec(this, 0) }, E_NOTIMPL);
        assert_eq!(unsafe { advanced_put_client_protocol_spec(this, 1) }, E_NOTIMPL);

        let vtable = advanced_vtable();
        assert_eq!(vtable.slots[0], advanced_put_compress as *const () as usize);
        assert_eq!(
            vtable.slots[4],
            advanced_put_allow_background_input as *const () as usize
        );
        assert_eq!(
            vtable.slots[5],
            advanced_get_allow_background_input as *const () as usize
        );
        assert_eq!(vtable.slots[6], advanced_put_keyboard_layout_str as *const () as usize);
        assert_eq!(vtable.slots[12], advanced_put_disable_rdpdr as *const () as usize);
        assert_eq!(vtable.slots[28], advanced_put_rdp_port as *const () as usize);
        assert_eq!(vtable.slots[30], advanced_put_enable_mouse as *const () as usize);
        assert_eq!(vtable.slots[34], advanced_put_enable_windows_key as *const () as usize);
        assert_eq!(
            vtable.slots[132],
            advanced_put_enable_auto_reconnect as *const () as usize
        );
        assert_eq!(
            vtable.slots[133],
            advanced_get_enable_auto_reconnect as *const () as usize
        );
        assert_eq!(
            vtable.slots[134],
            advanced_put_max_reconnect_attempts as *const () as usize
        );
        assert_eq!(
            vtable.slots[135],
            advanced_get_max_reconnect_attempts as *const () as usize
        );
        assert_eq!(vtable.slots[83], advanced_put_keyboard_type as *const () as usize);
        assert_eq!(vtable.slots[85], advanced_put_keyboard_subtype as *const () as usize);
        assert_eq!(
            vtable.slots[87],
            advanced_put_keyboard_function_key as *const () as usize
        );
        assert_eq!(
            vtable.slots[110],
            advanced_put_grab_focus_on_connect as *const () as usize
        );
        assert_eq!(
            vtable.slots[106],
            advanced_put_display_connection_bar as *const () as usize
        );
        assert_eq!(
            vtable.slots[107],
            advanced_get_display_connection_bar as *const () as usize
        );
        assert_eq!(vtable.slots[108], advanced_put_pin_connection_bar as *const () as usize);
        assert_eq!(vtable.slots[109], advanced_get_pin_connection_bar as *const () as usize);
        assert_eq!(
            vtable.slots[105],
            advanced_put_clear_text_password as *const () as usize
        );
        assert_eq!(
            vtable.slots[168],
            advanced_get_authentication_type as *const () as usize
        );
        assert_eq!(vtable.slots[126], advanced_put_performance_flags as *const () as usize);
        assert_eq!(
            vtable.slots[140],
            advanced_put_authentication_level as *const () as usize
        );
        assert_eq!(vtable.slots[142], advanced_put_redirect_clipboard as *const () as usize);
        assert_eq!(vtable.slots[150], advanced_put_redirect_devices as *const () as usize);
        assert_eq!(vtable.slots[160], advanced_get_pcb as *const () as usize);
        assert_eq!(
            vtable.slots[162],
            advanced_put_hotkey_focus_release_left as *const () as usize
        );
        assert_eq!(
            vtable.slots[163],
            advanced_get_hotkey_focus_release_left as *const () as usize
        );
        assert_eq!(
            vtable.slots[164],
            advanced_put_hotkey_focus_release_right as *const () as usize
        );
        assert_eq!(
            vtable.slots[165],
            advanced_get_hotkey_focus_release_right as *const () as usize
        );
        assert_eq!(vtable.slots[144], advanced_put_audio_redirection as *const () as usize);
        assert_eq!(vtable.slots[183], advanced_put_redirect_directx as *const () as usize);
        assert_eq!(
            vtable.slots[185],
            advanced_put_network_connection_type as *const () as usize
        );
        assert_eq!(
            vtable.slots[186],
            advanced_get_network_connection_type as *const () as usize
        );
        assert_eq!(
            vtable.slots[187],
            advanced_put_bandwidth_detection as *const () as usize
        );
        assert_eq!(
            vtable.slots[189],
            advanced_put_client_protocol_spec as *const () as usize
        );
    }

    #[test]
    fn rdm_unmapped_advanced_settings_slots_use_typed_failures() {
        let settings = Rc::new(RefCell::new(CompatibilitySettings::default()));
        let mut object = CompatibilitySettingsObject {
            vtable: advanced_vtable(),
            references: AtomicU32::new(1),
            settings,
            native_mstsc_credential_bridge: None,
            server_object: false,
        };
        let this = (&mut object as *mut AdvancedSettingsObject).cast::<c_void>();

        let mut focus_release_hotkey = i32::MAX;
        assert_eq!(
            unsafe { advanced_put_hotkey_focus_release_left(this, 0x1234) },
            E_NOTIMPL
        );
        assert_eq!(
            unsafe { advanced_get_hotkey_focus_release_left(this, &mut focus_release_hotkey) },
            E_NOTIMPL
        );
        assert_eq!(focus_release_hotkey, 0);
        assert_eq!(
            unsafe { advanced_put_hotkey_focus_release_right(this, 0x5678) },
            E_NOTIMPL
        );
        focus_release_hotkey = i32::MAX;
        assert_eq!(
            unsafe { advanced_get_hotkey_focus_release_right(this, &mut focus_release_hotkey) },
            E_NOTIMPL
        );
        assert_eq!(focus_release_hotkey, 0);

        let load_balance_info = BSTR::from("tsv://example");
        assert_eq!(
            unsafe { advanced_put_load_balance_info(this, load_balance_info.as_ptr()) },
            E_NOTIMPL
        );
        let mut load_balance_info = ptr::dangling::<u16>();
        assert_eq!(
            unsafe { advanced_get_load_balance_info(this, &mut load_balance_info) },
            E_NOTIMPL
        );
        assert!(load_balance_info.is_null());

        let mut authentication_level = u32::MAX;
        assert_eq!(
            unsafe { advanced_get_authentication_level(this, &mut authentication_level) },
            S_OK
        );
        assert_eq!(authentication_level, 0);

        let mut redirect_devices = VARIANT_TRUE.0;
        assert_eq!(
            unsafe { advanced_get_redirect_devices(this, &mut redirect_devices) },
            E_NOTIMPL
        );
        assert_eq!(redirect_devices, VARIANT_FALSE.0);

        let mut keep_alive_interval = i32::MAX;
        assert_eq!(
            unsafe { advanced_get_keep_alive_interval(this, &mut keep_alive_interval) },
            E_NOTIMPL
        );
        assert_eq!(keep_alive_interval, 0);
    }

    #[test]
    fn certificate_prompt_settings_are_sealed_after_connect_starts() {
        let settings = Rc::new(RefCell::new(CompatibilitySettings::default()));
        let mut object = CompatibilitySettingsObject {
            vtable: advanced_vtable(),
            references: AtomicU32::new(1),
            settings: Rc::clone(&settings),
            native_mstsc_credential_bridge: None,
            server_object: false,
        };
        let this = (&mut object as *mut AdvancedSettingsObject).cast::<c_void>();

        assert_eq!(unsafe { advanced_put_authentication_level(this, 2) }, S_OK);
        assert_eq!(unsafe { advanced_put_public_mode(this, VARIANT_TRUE.0) }, S_OK);
        settings.borrow_mut().connection_settings_sealed = true;

        assert_eq!(unsafe { advanced_put_authentication_level(this, 0) }, E_FAIL);
        assert_eq!(unsafe { advanced_put_public_mode(this, VARIANT_FALSE.0) }, E_FAIL);
    }

    #[test]
    fn native_credential_bridge_defaults_to_certificate_prompting_only_when_unconfigured() {
        assert!(!certificate_prompt_enabled(
            CertificateValidation::Strict,
            0,
            false,
            false
        ));
        assert!(certificate_prompt_enabled(
            CertificateValidation::Strict,
            0,
            false,
            true
        ));
        assert!(!certificate_prompt_enabled(
            CertificateValidation::Strict,
            0,
            true,
            true
        ));
        assert!(certificate_prompt_enabled(
            CertificateValidation::Strict,
            2,
            true,
            false
        ));
        assert!(!certificate_prompt_enabled(
            CertificateValidation::DangerouslyAcceptInvalidCertificate,
            2,
            true,
            true
        ));
    }

    #[test]
    fn renderer_resize_layout_ignores_minimized_bounds_and_uses_standard_scaling() {
        assert!(display_layout_from_renderer_size(0, 1080).is_none());
        assert!(display_layout_from_renderer_size(1920, 0).is_none());
        assert!(display_layout_from_renderer_size(-1, 1080).is_none());

        let layout = display_layout_from_renderer_size(1920, 1080).expect("nonzero client bounds");
        assert_eq!(layout.desktop_width, 1920);
        assert_eq!(layout.desktop_height, 1080);
        assert_eq!(layout.physical_width, 0);
        assert_eq!(layout.physical_height, 0);
        assert_eq!(layout.orientation, 0);
        assert_eq!(layout.desktop_scale_factor, 100);
        assert_eq!(layout.device_scale_factor, 100);
    }

    #[test]
    fn credential_prompt_buffer_is_terminated_and_bounded() {
        assert_eq!(
            credential_prompt_buffer("user", 5),
            [u16::from(b'u'), u16::from(b's'), u16::from(b'e'), u16::from(b'r'), 0]
        );
        assert_eq!(
            credential_prompt_buffer("long-name", 5),
            [u16::from(b'l'), u16::from(b'o'), u16::from(b'n'), u16::from(b'g'), 0]
        );
        assert!(credential_prompt_buffer("ignored", 0).is_empty());
    }

    #[test]
    fn autologon_credentials_require_nonempty_username_and_password() {
        assert_eq!(
            autologon_credentials(Some("user".to_owned()), Some("password".to_owned())),
            Some(("user".to_owned(), "password".to_owned()))
        );
        assert_eq!(autologon_credentials(None, Some("password".to_owned())), None);
        assert_eq!(autologon_credentials(Some("user".to_owned()), None), None);
        assert_eq!(
            autologon_credentials(Some(String::new()), Some("password".to_owned())),
            None
        );
        assert_eq!(
            autologon_credentials(Some("user".to_owned()), Some(String::new())),
            None
        );
    }

    #[test]
    fn native_shell_presentation_is_enabled_only_by_explicit_opt_in() {
        assert!(native_shell_presentation_enabled(true, true));
        assert!(!native_shell_presentation_enabled(true, false));
        assert!(!native_shell_presentation_enabled(false, true));
        assert!(!native_shell_presentation_enabled(false, false));
    }

    #[test]
    fn certificate_fingerprint_and_trust_key_are_endpoint_bound() {
        let first = certificate_fingerprint(b"first certificate");
        let second = certificate_fingerprint(b"second certificate");
        assert_ne!(first, second);
        assert_eq!(
            certificate_exception_key("SERVER.EXAMPLE.TEST:3389"),
            certificate_exception_key("server.example.test:3389")
        );
        assert_ne!(
            certificate_exception_key("server.example.test:3389"),
            certificate_exception_key("server.example.test:3390")
        );
    }

    #[test]
    fn advanced_settings_stubs_trace_the_exact_unmapped_slot() {
        let trace_path = std::env::temp_dir().join(format!(
            "ironrdp-activex-advanced-settings-{}.trace",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&trace_path);

        let trace_guard = TestHostTracePath::install(trace_path.clone());
        let first = unsafe { advanced_settings_stub_2(ptr::null_mut(), 0) };
        let second = unsafe { advanced_settings_stub_82(ptr::null_mut(), 0) };
        drop(trace_guard);
        let trace = std::fs::read_to_string(&trace_path).expect("advanced settings trace must be written");
        let _ = std::fs::remove_file(trace_path);

        assert_eq!(first, E_NOTIMPL);
        assert_eq!(second, E_NOTIMPL);
        assert_eq!(
            trace,
            "E_NOTIMPL:AdvancedSettings::slot_2\nE_NOTIMPL:AdvancedSettings::slot_82\n"
        );
    }

    #[test]
    fn connection_failure_trace_omits_error_context() {
        let trace_path = std::env::temp_dir().join(format!(
            "ironrdp-activex-connection-failure-{}.trace",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&trace_path);

        let trace_guard = TestHostTracePath::install(trace_path.clone());
        trace_connection_failure(&ConnectorError::new("must not be traced", ConnectorErrorKind::Custom));
        drop(trace_guard);
        let trace = std::fs::read_to_string(&trace_path).expect("connection failure trace must be written");
        let _ = std::fs::remove_file(trace_path);

        assert!(trace.starts_with("RdpWorker::ConnectionFailure:Custom:control.rs:line_"));
        assert!(!trace.contains("must not be traced"));
    }

    #[test]
    fn session_failure_trace_omits_error_contexts() {
        let trace_path =
            std::env::temp_dir().join(format!("ironrdp-activex-session-failure-{}.trace", std::process::id()));
        let _ = std::fs::remove_file(&trace_path);

        let trace_guard = TestHostTracePath::install(trace_path.clone());
        let decode_error = DecodeError::new(
            "nested decode context must not be traced",
            DecodeErrorKind::Other {
                description: "decode detail must not be traced",
            },
        );
        trace_session_failure(&SessionError::new(
            "outer decode context must not be traced",
            SessionErrorKind::Decode(decode_error),
        ));
        trace_session_failure(&SessionError::new(
            "reason context must not be traced",
            SessionErrorKind::Reason("reason detail must not be traced".to_owned()),
        ));
        trace_session_failure(&SessionError::new(
            "general context must not be traced",
            SessionErrorKind::General,
        ));
        drop(trace_guard);
        let trace = std::fs::read_to_string(&trace_path).expect("session failure trace must be written");
        let _ = std::fs::remove_file(trace_path);

        let mut lines = trace.lines();
        for prefix in [
            "RdpWorker::SessionFailure:Decode:Other:control.rs:line_",
            "RdpWorker::SessionFailure:Reason:control.rs:line_",
            "RdpWorker::SessionFailure:General:control.rs:line_",
        ] {
            assert!(lines.next().is_some_and(|line| line.starts_with(prefix)));
        }
        assert_eq!(lines.next(), None);
        for secret in [
            "nested decode context must not be traced",
            "decode detail must not be traced",
            "outer decode context must not be traced",
            "reason context must not be traced",
            "reason detail must not be traced",
            "general context must not be traced",
        ] {
            assert!(!trace.contains(secret));
        }
    }

    #[test]
    fn authentication_level_zero_requires_an_explicit_opt_in() {
        assert_eq!(
            certificate_validation_from_authentication_level(0, false),
            CertificateValidation::Strict
        );
        assert_eq!(
            certificate_validation_from_authentication_level(0, true),
            CertificateValidation::DangerouslyAcceptInvalidCertificate
        );
        assert_eq!(
            certificate_validation_from_authentication_level(1, true),
            CertificateValidation::Strict
        );
        assert_eq!(
            certificate_validation_from_authentication_level(2, true),
            CertificateValidation::Strict
        );
    }

    #[test]
    fn advanced_settings_vtable_preserves_mapped_and_individual_stub_slots() {
        let stub_slots = advanced_settings_stub_slots();
        assert_eq!(stub_slots.len(), 191);
        assert_ne!(stub_slots[2], stub_slots[3]);
        assert_eq!(stub_slots[2], advanced_settings_stub_2 as *const () as usize);
        assert_eq!(stub_slots[82], advanced_settings_stub_82 as *const () as usize);

        let vtable = advanced_vtable();
        assert_eq!(vtable.slots[2], advanced_settings_stub_2 as *const () as usize);
        assert_eq!(vtable.slots[82], advanced_settings_stub_82 as *const () as usize);
        assert_eq!(vtable.slots[0], advanced_put_compress as *const () as usize);
        assert_eq!(vtable.slots[97], advanced_put_smart_sizing as *const () as usize);
        assert_eq!(
            vtable.slots[136],
            advanced_put_connection_bar_show_minimize_button as *const () as usize
        );
        assert_eq!(
            vtable.slots[137],
            advanced_get_connection_bar_show_minimize_button as *const () as usize
        );
        assert_eq!(
            vtable.slots[138],
            advanced_put_connection_bar_show_restore_button as *const () as usize
        );
        assert_eq!(
            vtable.slots[139],
            advanced_get_connection_bar_show_restore_button as *const () as usize
        );
        assert_eq!(
            vtable.slots[146],
            advanced_put_connection_bar_show_pin_button as *const () as usize
        );
        assert_eq!(
            vtable.slots[147],
            advanced_get_connection_bar_show_pin_button as *const () as usize
        );
        assert_eq!(
            vtable.slots[185],
            advanced_put_network_connection_type as *const () as usize
        );
    }

    #[test]
    fn secured_settings_retain_and_expose_alternate_shell_configuration() {
        let settings = Rc::new(RefCell::new(CompatibilitySettings::default()));
        let mut object = CompatibilitySettingsObject {
            vtable: secured_vtable(),
            references: AtomicU32::new(1),
            settings: Rc::clone(&settings),
            native_mstsc_credential_bridge: None,
            server_object: false,
        };
        let this = (&mut object as *mut SecuredSettingsObject).cast::<c_void>();
        let start_program = BSTR::from("C:\\Windows\\System32\\cmd.exe");
        let work_dir = BSTR::from("C:\\Windows");

        assert_eq!(unsafe { secured_put_start_program(this, start_program.as_ptr()) }, S_OK);
        assert_eq!(unsafe { secured_put_work_dir(this, work_dir.as_ptr()) }, S_OK);
        assert_eq!(
            settings.borrow().secured_start_program,
            "C:\\Windows\\System32\\cmd.exe"
        );
        assert_eq!(settings.borrow().secured_work_dir, "C:\\Windows");

        assert_eq!(unsafe { secured_put_audio_redirection(this, 1) }, S_OK);
        let mut audio_redirection_mode = -1;
        assert_eq!(
            unsafe { secured_get_audio_redirection(this, &mut audio_redirection_mode) },
            S_OK
        );
        assert_eq!(audio_redirection_mode, 1);
        assert_eq!(unsafe { secured_put_audio_redirection(this, -1) }, E_INVALIDARG);
        assert_eq!(unsafe { secured_put_audio_redirection(this, 3) }, E_INVALIDARG);

        let mut pcb = ptr::dangling::<u16>();
        assert_eq!(unsafe { secured_get_pcb(this, &mut pcb) }, E_NOTIMPL);
        assert!(pcb.is_null());
        let pcb = BSTR::from("pcb");
        assert_eq!(unsafe { secured_put_pcb(this, pcb.as_ptr()) }, E_NOTIMPL);

        let mut returned_start_program = ptr::null();
        let mut returned_work_dir = ptr::null();
        assert_eq!(
            unsafe { secured_get_start_program(this, &mut returned_start_program) },
            S_OK
        );
        assert_eq!(unsafe { secured_get_work_dir(this, &mut returned_work_dir) }, S_OK);
        let returned_start_program = unsafe { BSTR::from_raw(returned_start_program) };
        let returned_work_dir = unsafe { BSTR::from_raw(returned_work_dir) };
        assert_eq!(
            String::try_from(&returned_start_program).expect("valid alternate-shell BSTR"),
            "C:\\Windows\\System32\\cmd.exe"
        );
        assert_eq!(
            String::try_from(&returned_work_dir).expect("valid work-directory BSTR"),
            "C:\\Windows"
        );

        let vtable = secured_vtable();
        assert_eq!(vtable.slots.len(), SECURED_SETTINGS_SLOTS);
        assert_eq!(vtable.slots[0], secured_put_start_program as *const () as usize);
        assert_eq!(vtable.slots[2], secured_put_work_dir as *const () as usize);
        assert_eq!(vtable.slots[8], secured_put_audio_redirection as *const () as usize);
        assert_eq!(vtable.slots[10], secured_get_pcb as *const () as usize);
        assert_eq!(vtable.slots[11], secured_put_pcb as *const () as usize);
        assert!(settings_supports_interface::<SECURED_SETTINGS_SLOTS>(&GUID::from_u128(
            0x25f2ce20_8b1d_4971_a7cd_549dae201fc0
        )));
    }

    #[test]
    fn compatibility_settings_reference_counts_fail_closed() {
        let settings = Rc::new(RefCell::new(CompatibilitySettings::default()));
        let mut object = CompatibilitySettingsObject {
            vtable: transport_vtable(),
            references: AtomicU32::new(1),
            settings,
            native_mstsc_credential_bridge: None,
            server_object: false,
        };
        let this = (&mut object as *mut TransportSettingsObject).cast::<c_void>();

        assert_eq!(unsafe { settings_add_ref::<TRANSPORT_SETTINGS_SLOTS>(this) }, 2);
        assert_eq!(unsafe { settings_release::<TRANSPORT_SETTINGS_SLOTS>(this) }, 1);

        object.references.store(u32::MAX, Ordering::Release);
        assert_eq!(unsafe { settings_add_ref::<TRANSPORT_SETTINGS_SLOTS>(this) }, u32::MAX);
        assert_eq!(unsafe { settings_release::<TRANSPORT_SETTINGS_SLOTS>(this) }, u32::MAX);

        object.references.store(0, Ordering::Release);
        assert_eq!(unsafe { settings_add_ref::<TRANSPORT_SETTINGS_SLOTS>(this) }, 0);
        assert_eq!(unsafe { settings_release::<TRANSPORT_SETTINGS_SLOTS>(this) }, 0);
    }

    #[test]
    fn returned_settings_objects_keep_the_server_loaded() {
        let settings = Rc::new(RefCell::new(CompatibilitySettings::default()));
        let mut object = ptr::null_mut();
        unsafe {
            settings_object(transport_vtable(), settings, &mut object).expect("settings object allocation succeeds");
        }

        let settings_object = unsafe { &*object.cast::<TransportSettingsObject>() };
        assert!(settings_object.server_object);
        assert_eq!(com::DllCanUnloadNow(), S_FALSE);
        assert_eq!(unsafe { settings_release::<TRANSPORT_SETTINGS_SLOTS>(object) }, 0);
    }

    #[test]
    fn independently_returned_com_children_keep_the_server_loaded() {
        let _devices: IMsRdpDeviceCollection = EmptyDeviceCollection::new().into();
        let settings = Rc::new(RefCell::new(CompatibilitySettings::default()));
        let _drives: IMsRdpDriveCollection =
            DriveCollection::new(Rc::clone(&settings.borrow().drive_catalog), Rc::clone(&settings)).into();
        let _cameras: IMsRdpCameraRedirConfigCollection = EmptyCameraRedirConfigCollection::new().into();
        let _clipboard: IMsRdpClipboard = ClipboardCapabilities::new(Rc::new(ClipboardState {
            enabled_for_session: Cell::new(false),
            connected: Cell::new(false),
        }))
        .into();

        let verbs: IEnumOLEVERB = OleVerbEnumerator::new(0).into();
        let _verb_clone = unsafe { verbs.Clone() }.expect("clone OLE verb enumerator");
        let advise: IEnumSTATDATA = OleAdviseEnumerator::new(Vec::new(), 0).into();
        let _advise_clone = unsafe { advise.Clone() }.expect("clone OLE advise enumerator");

        let container: IConnectionPointContainer = Control::new().into();
        let point =
            unsafe { container.FindConnectionPoint(&IID_MSTSCLIB_EVENTS) }.expect("find event connection point");
        let points = unsafe { container.EnumConnectionPoints() }.expect("enumerate event connection points");
        let _point_clone = unsafe { points.Clone() }.expect("clone connection-point enumerator");
        let connections = unsafe { point.EnumConnections() }.expect("enumerate event connections");
        let _connection_clone = unsafe { connections.Clone() }.expect("clone connection enumerator");

        assert_eq!(com::DllCanUnloadNow(), S_FALSE);
    }

    #[test]
    fn native_mstsc_preflight_requires_three_contiguous_empty_settings() {
        let (preflight, intercept) = NativeMstscPreflight::Idle.observe_extended_setting(true, "");
        assert_eq!(preflight, NativeMstscPreflight::FirstEmptyProperty);
        assert!(!intercept);

        let (preflight, intercept) = preflight.observe_extended_setting(true, "ZoomLevel");
        assert_eq!(preflight, NativeMstscPreflight::Idle);
        assert!(!intercept);

        let (preflight, intercept) = preflight.observe_extended_setting(true, "");
        assert_eq!(preflight, NativeMstscPreflight::FirstEmptyProperty);
        assert!(!intercept);
        let (preflight, intercept) = preflight.observe_extended_setting(true, "");
        assert_eq!(preflight, NativeMstscPreflight::SecondEmptyProperty);
        assert!(!intercept);
        let (preflight, intercept) = preflight.observe_extended_setting(true, "");
        assert_eq!(preflight, NativeMstscPreflight::Suppressed);
        assert!(intercept);

        let (preflight, intercept) = preflight.observe_extended_setting(true, "");
        assert_eq!(preflight, NativeMstscPreflight::Suppressed);
        assert!(!intercept);

        let (preflight, intercept) = preflight.observe_extended_setting(false, "");
        assert_eq!(preflight, NativeMstscPreflight::Idle);
        assert!(!intercept);
    }

    #[test]
    fn start_program_bridge_requires_explicit_opt_in_while_disconnected() {
        assert!(!should_intercept_native_mstsc_start_program(false, true));
        assert!(!should_intercept_native_mstsc_start_program(true, false));
        assert!(should_intercept_native_mstsc_start_program(true, true));
    }

    #[test]
    fn compatibility_classes_are_explicit() {
        assert!(is_supported_class(&CLSID_IRONRDP_ACTIVEX));
        assert!(is_supported_class(&CLSID_MS_RDP_CLIENT));
        assert!(is_supported_class(&CLSID_MS_RDP_CLIENT_10));
        assert!(is_supported_class(&CLSID_MS_RDP_CLIENT_11_NOT_SAFE_FOR_SCRIPTING));
        assert!(!is_supported_class(&GUID::zeroed()));
    }

    #[test]
    fn property_put_requires_the_named_property_argument() {
        let mut value = variant_i32(16);
        let named = DISPID_PROPERTYPUT;
        let params = DISPPARAMS {
            rgvarg: &mut value,
            rgdispidNamedArgs: &named as *const i32 as *mut i32,
            cArgs: 1,
            cNamedArgs: 1,
        };
        assert!(property_put_value(&params).is_ok());

        let invalid = DISPPARAMS {
            cNamedArgs: 0,
            ..params
        };
        let error = match property_put_value(&invalid) {
            Ok(_) => panic!("property put must reject missing named argument"),
            Err(error) => error,
        };
        assert_eq!(error.code(), DISP_E_BADPARAMCOUNT);
    }

    #[test]
    fn late_bound_fullscreen_and_status_text_match_the_raw_client_properties() {
        let control = Control::new();
        let named = DISPID_PROPERTYPUT;
        let mut fullscreen = unsafe { VariantValue::Bool(true).into_variant() };
        let fullscreen_params = DISPPARAMS {
            rgvarg: &mut fullscreen,
            rgdispidNamedArgs: &named as *const i32 as *mut i32,
            cArgs: 1,
            cNamedArgs: 1,
        };
        control
            .put_property(DISPID_FULLSCREEN, &fullscreen_params, ptr::null_mut())
            .expect("set late-bound FullScreen");
        assert!(control.settings.borrow().fullscreen);

        let mut status_text = variant_bstr("Connected".to_owned());
        let status_text_params = DISPPARAMS {
            rgvarg: &mut status_text,
            rgdispidNamedArgs: &named as *const i32 as *mut i32,
            cArgs: 1,
            cNamedArgs: 1,
        };
        control
            .put_property(DISPID_CONNECTED_STATUS_TEXT, &status_text_params, ptr::null_mut())
            .expect("set late-bound ConnectedStatusText");

        let mut output = VARIANT::default();
        control
            .get_property(DISPID_CONNECTED_STATUS_TEXT, &mut output)
            .expect("get late-bound ConnectedStatusText");
        assert_eq!(variant_bstr_value(&output).expect("status text BSTR"), "Connected");
        free_owned_bstr_variant(&mut output);

        let mut fullscreen_title = variant_bstr("IronRDP Desktop".to_owned());
        let fullscreen_title_params = DISPPARAMS {
            rgvarg: &mut fullscreen_title,
            rgdispidNamedArgs: &named as *const i32 as *mut i32,
            cArgs: 1,
            cNamedArgs: 1,
        };
        control
            .put_property(DISPID_FULLSCREEN_TITLE, &fullscreen_title_params, ptr::null_mut())
            .expect("set late-bound FullScreenTitle");
        assert_eq!(control.settings.borrow().fullscreen_title, "IronRDP Desktop");

        for (dispid, expected) in [
            (DISPID_HORIZONTAL_SCROLLBAR_VISIBLE, 0),
            (DISPID_VERTICAL_SCROLLBAR_VISIBLE, 0),
            (DISPID_CIPHER_STRENGTH, 128),
            (DISPID_SECURED_SETTINGS_ENABLED, i32::from(VARIANT_TRUE.0)),
            (DISPID_EXTENDED_DISCONNECT_REASON, 0),
        ] {
            let mut output = VARIANT::default();
            control
                .get_property(dispid, &mut output)
                .expect("get late-bound read-only client property");
            assert_eq!(
                variant_i32_value(&output, ptr::null_mut()).expect("integer client property"),
                expected
            );
        }
    }

    #[test]
    fn fullscreen_hotkey_requires_control_alt_break() {
        assert!(is_fullscreen_hotkey(VK_CANCEL, true));
        assert!(is_fullscreen_hotkey(VK_PAUSE, true));
        assert!(!is_fullscreen_hotkey(VK_CANCEL, false));
        assert!(!is_fullscreen_hotkey(VIRTUAL_KEY(0x42), true));
    }

    #[test]
    fn native_mstsc_fullscreen_is_rejected_without_shell_mutation() {
        unsafe extern "system" fn test_window_proc(
            window: HWND,
            message: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }

        let instance = unsafe { GetModuleHandleW(None) }.expect("current module handle");
        let class = WNDCLASSW {
            hInstance: windows::Win32::Foundation::HINSTANCE(instance.0),
            lpfnWndProc: Some(test_window_proc),
            lpszClassName: w!("TscShellContainerClass"),
            ..Default::default()
        };
        let atom = unsafe { RegisterClassW(&class) };
        assert_ne!(atom, 0, "register the native mstsc shell test window class");

        let root = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("TscShellContainerClass"),
                w!(""),
                WS_OVERLAPPEDWINDOW,
                100,
                100,
                320,
                240,
                None,
                None,
                Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                None,
            )
        }
        .expect("create native mstsc shell test window");
        let host = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!(""),
                WS_POPUP | WS_VISIBLE,
                120,
                120,
                300,
                220,
                Some(root),
                None,
                Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                None,
            )
        }
        .expect("create native mstsc owned host window");
        let renderer = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!(""),
                WS_CHILD,
                0,
                0,
                1,
                1,
                Some(host),
                None,
                Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                None,
            )
        }
        .expect("create renderer child window");

        let control = Control::new();
        control.activex_window.set(renderer);
        assert_eq!(control.native_mstsc_shell_window(), Some(root));
        assert_eq!(
            control
                .set_fullscreen(true)
                .expect_err("native mstsc fullscreen is unsupported")
                .code(),
            E_NOTIMPL
        );
        assert!(!control.settings.borrow().fullscreen);

        control.set_fullscreen(false).expect("leave native mstsc full screen");
        assert!(!control.settings.borrow().fullscreen);

        control.compatibility.borrow_mut().container_handled_fullscreen = 1;
        assert_eq!(
            control
                .set_fullscreen(true)
                .expect_err("native mstsc fullscreen remains unsupported")
                .code(),
            E_NOTIMPL
        );
        control
            .set_fullscreen(false)
            .expect("restore native shell after container handling");

        unsafe {
            DestroyWindow(host).expect("destroy native mstsc owned host window");
            DestroyWindow(root).expect("destroy native mstsc shell test window");
        }
    }

    #[test]
    fn disconnect_reporting_exposes_only_owned_reason_categories() {
        assert_eq!(
            DisconnectInfo::from_graceful_disconnect(&GracefulDisconnectReason::UserInitiated),
            DisconnectInfo::api_initiated()
        );
        assert_eq!(
            DisconnectInfo::from_graceful_disconnect(&GracefulDisconnectReason::ServerInitiated).extended_reason,
            EXTENDED_DISCONNECT_REASON_NO_INFO
        );
        let server_reason = SessionError::new(
            "session shutdown",
            SessionErrorKind::Reason("remote detail must not escape".to_owned()),
        );
        let session_disconnect = DisconnectInfo::from_session_failure(&server_reason);
        assert_eq!(
            session_disconnect.description,
            "The RDP session ended with a protocol reason."
        );
        assert_ne!(session_disconnect.description, "remote detail must not escape");

        let control = Control::new();
        assert_eq!(
            control.disconnect_description(0, EXTENDED_DISCONNECT_REASON_NO_INFO as u32),
            "No additional disconnect information is available."
        );
        control.last_disconnect.set(DisconnectInfo::api_initiated());
        assert_eq!(
            control.disconnect_description(0, EXTENDED_DISCONNECT_REASON_API_INITIATED_DISCONNECT as u32),
            "The RDP session was disconnected by the client."
        );
        assert_eq!(
            control.disconnect_description(0, EXTENDED_DISCONNECT_REASON_NO_INFO as u32),
            "No additional disconnect information is available."
        );

        let mut extended_reason = VARIANT::default();
        control
            .get_property(DISPID_EXTENDED_DISCONNECT_REASON, &mut extended_reason)
            .expect("get late-bound extended disconnect reason");
        assert_eq!(
            variant_i32_value(&extended_reason, ptr::null_mut()).expect("integer disconnect reason"),
            EXTENDED_DISCONNECT_REASON_API_INITIATED_DISCONNECT
        );
    }

    #[test]
    fn worker_completion_preserves_terminal_lifecycle_after_output_backpressure() {
        assert!(matches!(
            worker_completion_event(7, false, false, false),
            Some(WorkerEvent::Disconnected {
                generation: 7,
                disconnect,
            }) if disconnect == DisconnectInfo::api_initiated()
        ));
        assert!(worker_completion_event(7, false, true, false).is_none());
        assert!(worker_completion_event(7, true, false, false).is_none());
        assert!(matches!(
            worker_completion_event(7, false, false, true),
            Some(WorkerEvent::FatalError {
                generation: 7,
                disconnect,
            }) if disconnect == DisconnectInfo::internal_error()
        ));
    }

    #[test]
    fn client_task_outcome_reports_bounded_failure_categories() {
        assert_eq!(ClientTaskOutcome::Completed.trace_marker(), None);
        assert_eq!(
            ClientTaskOutcome::Cancelled.trace_marker(),
            Some("RdpWorker::TaskFailure:Cancelled")
        );
        assert_eq!(
            ClientTaskOutcome::Panicked.trace_marker(),
            Some("RdpWorker::TaskFailure:Panicked")
        );
        assert_eq!(
            ClientTaskOutcome::Failed.trace_marker(),
            Some("RdpWorker::TaskFailure:Unknown")
        );
    }

    #[test]
    fn frame_requires_a_nonempty_exact_pixel_buffer() {
        let pixels = vec![0x00ff_0000; 4];
        let frame = Frame::new(&pixels, 2, 2, 1).expect("valid RGB frame");
        assert_eq!(frame.sequence, 1);
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);

        assert!(Frame::new(&[0; 3], 2, 2, 2).is_none());
        assert!(Frame::new(&[], 0, 1, 2).is_none());
        assert!(Frame::new(&[], 1, 0, 2).is_none());
    }

    #[test]
    fn smart_sizing_fits_the_frame_without_distorting_it() {
        let control = Control::new();
        let client = RECT {
            right: 200,
            bottom: 100,
            ..Default::default()
        };
        control.compatibility.borrow_mut().smart_sizing = true;

        assert_eq!(control.frame_viewport(&client, 400, 300), (33, 0, 133, 100));
    }

    #[test]
    fn zoom_level_scales_the_fixed_size_frame_viewport() {
        let control = Control::new();
        let client = RECT {
            right: 200,
            bottom: 100,
            ..Default::default()
        };
        control.compatibility.borrow_mut().zoom_level = 50;

        assert_eq!(control.frame_viewport(&client, 400, 200), (0, 0, 200, 100));
    }

    #[test]
    fn zoom_level_scales_the_smart_sizing_viewport() {
        let control = Control::new();
        let client = RECT {
            right: 200,
            bottom: 100,
            ..Default::default()
        };
        let mut compatibility = control.compatibility.borrow_mut();
        compatibility.smart_sizing = true;
        compatibility.zoom_level = 50;
        drop(compatibility);

        assert_eq!(control.frame_viewport(&client, 400, 200), (50, 25, 100, 50));
    }

    #[test]
    fn input_database_preserves_extended_keys_and_releases_held_buttons() {
        let mut database = InputDatabase::new();
        let extended_control = Scancode::from_u8(true, 0x1d);
        let key_events = database.apply([
            Operation::KeyPressed(extended_control),
            Operation::KeyPressed(extended_control),
        ]);
        assert_eq!(key_events.len(), 3);
        assert!(matches!(
            key_events[0],
            FastPathInputEvent::KeyboardEvent(flags, 0x1d) if flags == KeyboardFlags::EXTENDED
        ));
        assert!(database.is_key_pressed(extended_control));

        let button_events = database.apply([
            Operation::MouseButtonPressed(MouseButton::Left),
            Operation::MouseButtonPressed(MouseButton::Left),
        ]);
        assert_eq!(button_events.len(), 1);
        assert!(database.is_mouse_button_pressed(MouseButton::Left));

        let releases = database.release_all();
        assert_eq!(releases.len(), 2);
        assert!(!database.is_key_pressed(extended_control));
        assert!(!database.is_mouse_button_pressed(MouseButton::Left));
    }

    #[test]
    fn disabled_windows_keys_are_not_forwarded_but_existing_keys_are_released() {
        let control = Control::new();
        let windows_key = Scancode::from_u8(true, 0x5b);
        let lparam = LPARAM(((0x5bi32 << 16) | 0x0100_0000) as isize);
        control.compatibility.borrow_mut().enable_windows_key = false;

        assert!(control.handle_activex_window_message(HWND::default(), WM_KEYDOWN, WPARAM(0), lparam));
        assert!(!control.input_database.borrow().is_key_pressed(windows_key));

        control.apply_input([Operation::KeyPressed(windows_key)]);
        assert!(control.input_database.borrow().is_key_pressed(windows_key));
        assert!(control.handle_activex_window_message(HWND::default(), WM_KEYUP, WPARAM(0), lparam));
        assert!(!control.input_database.borrow().is_key_pressed(windows_key));
    }

    #[test]
    fn credential_prompt_requires_a_destination_and_missing_password() {
        assert!(should_prompt_for_credentials("rdp.example.test", false, true));
        assert!(!should_prompt_for_credentials("", false, true));
        assert!(!should_prompt_for_credentials("rdp.example.test", true, true));
        assert!(!should_prompt_for_credentials("rdp.example.test", false, false));
    }

    #[test]
    fn keyboard_hook_mode_forwards_windows_keys_only_when_requested() {
        let control = Control::new();
        let windows_key = Scancode::from_u8(true, 0x5b);
        let lparam = LPARAM(((0x5bi32 << 16) | 0x0100_0000) as isize);

        control.compatibility.borrow_mut().keyboard_hook_mode = 0;
        assert!(control.handle_activex_window_message(HWND::default(), WM_KEYDOWN, WPARAM(0), lparam));
        assert!(!control.input_database.borrow().is_key_pressed(windows_key));

        control.compatibility.borrow_mut().keyboard_hook_mode = 1;
        assert!(control.handle_activex_window_message(HWND::default(), WM_KEYDOWN, WPARAM(0), lparam));
        assert!(control.input_database.borrow().is_key_pressed(windows_key));
        control.apply_input([Operation::KeyReleased(windows_key)]);

        control.compatibility.borrow_mut().keyboard_hook_mode = 2;
        assert!(control.handle_activex_window_message(HWND::default(), WM_KEYDOWN, WPARAM(0), lparam));
        assert!(!control.input_database.borrow().is_key_pressed(windows_key));

        control.settings.borrow_mut().fullscreen = true;
        assert!(control.handle_activex_window_message(HWND::default(), WM_KEYDOWN, WPARAM(0), lparam));
        assert!(control.input_database.borrow().is_key_pressed(windows_key));
    }

    #[test]
    fn send_keys_forwards_an_atomic_extended_scancode_batch() {
        let control = Control::new();
        let (sender, mut receiver) = RdpInputSender::channel(16);
        *control.input_sender.borrow_mut() = Some(sender);
        control.state.set(ConnectionState::Connected);
        let key_up = [VARIANT_FALSE.0, VARIANT_TRUE.0];
        // WM_KEYDOWN lParam: repeat count 1, extended-key flag, scan code 0x1d.
        let key_data = [0x011d_0001, 0x011d_0001];

        control
            .send_keys(2, key_up.as_ptr(), key_data.as_ptr())
            .expect("forward a connected SendKeys batch");

        assert!(matches!(
            receiver.try_recv(),
            Ok(RdpInputEvent::FastPath(events))
                if events.as_slice()
                    == [
                        FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED, 0x1d),
                        FastPathInputEvent::KeyboardEvent(
                            KeyboardFlags::EXTENDED | KeyboardFlags::RELEASE,
                            0x1d,
                        ),
                    ]
        ));
    }

    #[test]
    fn send_keys_validates_count_pointers_and_session_state() {
        let control = Control::new();

        control
            .send_keys(0, ptr::null(), ptr::null())
            .expect("an empty SendKeys batch is valid");
        assert_eq!(
            control
                .send_keys(-1, ptr::null(), ptr::null())
                .expect_err("negative count is invalid")
                .code(),
            E_INVALIDARG
        );
        assert_eq!(
            control
                .send_keys(21, ptr::null(), ptr::null())
                .expect_err("more than twenty keys is invalid")
                .code(),
            E_INVALIDARG
        );
        assert_eq!(
            control
                .send_keys(1, ptr::null(), ptr::null())
                .expect_err("nonempty batches require both arrays")
                .code(),
            E_POINTER
        );

        let key_up = [VARIANT_FALSE.0];
        let key_data = [0x001e_0001];
        assert_eq!(
            control
                .send_keys(1, key_up.as_ptr(), key_data.as_ptr())
                .expect_err("a disconnected control cannot send to a remote session")
                .code(),
            E_UNEXPECTED
        );
    }

    #[test]
    fn send_remote_action_forwards_supported_remote_shell_shortcuts() {
        let actions = [
            (
                REMOTE_SESSION_ACTION_CHARMS,
                &[
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED, 0x5b),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x2e),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x2e),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED | KeyboardFlags::RELEASE, 0x5b),
                ][..],
            ),
            (
                REMOTE_SESSION_ACTION_APPBAR,
                &[
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED, 0x5b),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x2c),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x2c),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED | KeyboardFlags::RELEASE, 0x5b),
                ][..],
            ),
            (
                REMOTE_SESSION_ACTION_START_SCREEN,
                &[
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED, 0x5b),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED | KeyboardFlags::RELEASE, 0x5b),
                ][..],
            ),
            (
                REMOTE_SESSION_ACTION_APP_SWITCH,
                &[
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x38),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x0f),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x0f),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x38),
                ][..],
            ),
            (
                REMOTE_SESSION_ACTION_ACTION_CENTER,
                &[
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED, 0x5b),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x1e),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x1e),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED | KeyboardFlags::RELEASE, 0x5b),
                ][..],
            ),
            (
                REMOTE_SESSION_ACTION_TASK_MANAGER,
                &[
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x1d),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x2a),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x01),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x01),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x2a),
                    FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x1d),
                ][..],
            ),
        ];

        for (action, expected) in actions {
            let control = Control::new();
            let (sender, mut receiver) = RdpInputSender::channel(16);
            *control.input_sender.borrow_mut() = Some(sender);
            control.state.set(ConnectionState::Connected);
            let client: IMsRdpClient8 = control.into();

            unsafe { client.SendRemoteAction(action) }.expect("supported remote action is forwarded");

            assert!(matches!(
                receiver.try_recv(),
                Ok(RdpInputEvent::FastPath(events)) if events.as_slice() == expected
            ));
        }
    }

    #[test]
    fn send_remote_action_preserves_held_shortcut_keys() {
        let control = Control::new();
        let (sender, mut receiver) = RdpInputSender::channel(1);
        *control.input_sender.borrow_mut() = Some(sender);
        control.state.set(ConnectionState::Connected);

        let held_keys = [Scancode::from_u8(false, 0x1d), Scancode::from_u8(false, 0x2e)];
        control
            .input_database
            .borrow_mut()
            .apply(held_keys.into_iter().map(Operation::KeyPressed));

        control
            .send_remote_action(REMOTE_SESSION_ACTION_CHARMS)
            .expect("send Charms while Ctrl+C is held");

        assert!(matches!(
            receiver.try_recv(),
            Ok(RdpInputEvent::FastPath(events))
                if events.as_slice()
                    == [
                        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x1d),
                        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x2e),
                        FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED, 0x5b),
                        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x2e),
                        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x2e),
                        FastPathInputEvent::KeyboardEvent(
                            KeyboardFlags::EXTENDED | KeyboardFlags::RELEASE,
                            0x5b,
                        ),
                        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x1d),
                        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x2e),
                    ]
        ));
        let input_database = control.input_database.borrow();
        assert!(input_database.is_key_pressed(held_keys[0]));
        assert!(input_database.is_key_pressed(held_keys[1]));
    }

    #[test]
    fn send_remote_action_rejects_unsupported_and_inactive_actions() {
        let control = Control::new();

        assert_eq!(
            control
                .send_remote_action(REMOTE_SESSION_ACTION_SNAP)
                .expect_err("deprecated snap action is unavailable")
                .code(),
            E_NOTIMPL
        );
        assert_eq!(
            control
                .send_remote_action(-1)
                .expect_err("unknown action is invalid")
                .code(),
            E_INVALIDARG
        );
        assert_eq!(
            control
                .send_remote_action(REMOTE_SESSION_ACTION_CHARMS)
                .expect_err("supported actions require an active session")
                .code(),
            E_UNEXPECTED
        );
    }

    #[test]
    fn active_display_updates_queue_ironrdp_resize_events() {
        let control = Control::new();
        let (sender, mut receiver) = RdpInputSender::channel(16);
        *control.input_sender.borrow_mut() = Some(sender);
        control.state.set(ConnectionState::Connected);
        let topology = MonitorTopology::from_host_monitors(vec![HostMonitor {
            rect: RECT {
                left: 0,
                top: 0,
                right: 1_920,
                bottom: 1_080,
            },
            primary: true,
        }])
        .expect("a valid single-monitor topology");
        *control.configured_monitor_topology.borrow_mut() = Some(topology.clone());
        *control.active_monitor_topology.borrow_mut() = Some(topology);

        control
            .update_display_layout(DisplayLayout {
                desktop_width: 1280,
                desktop_height: 720,
                physical_width: 340,
                physical_height: 190,
                orientation: 0,
                desktop_scale_factor: 150,
                device_scale_factor: 100,
            })
            .expect("queue a supported active-session layout");

        assert!(matches!(
            receiver.try_recv(),
            Ok(RdpInputEvent::Resize {
                width: 1280,
                height: 720,
                scale_factor: 150,
                physical_size: Some((340, 190)),
            })
        ));
        let settings = control.settings.borrow();
        assert_eq!((settings.desktop_width, settings.desktop_height), (1280, 720));
        assert!(control.active_monitor_topology.borrow().is_none());
        assert!(control.configured_monitor_topology.borrow().is_none());
    }

    #[test]
    fn active_multimon_layout_rejects_dynamic_display_updates() {
        let control = Control::new();
        let (sender, mut receiver) = RdpInputSender::channel(16);
        *control.input_sender.borrow_mut() = Some(sender);
        control.state.set(ConnectionState::Connected);
        *control.configured_monitor_topology.borrow_mut() = Some(
            MonitorTopology::from_host_monitors(vec![
                HostMonitor {
                    rect: RECT {
                        left: 0,
                        top: 0,
                        right: 1_920,
                        bottom: 1_080,
                    },
                    primary: true,
                },
                HostMonitor {
                    rect: RECT {
                        left: 1_920,
                        top: 0,
                        right: 3_200,
                        bottom: 1_080,
                    },
                    primary: false,
                },
            ])
            .expect("a valid monitor topology"),
        );

        let error = control
            .update_display_layout(DisplayLayout {
                desktop_width: 1280,
                desktop_height: 720,
                physical_width: 0,
                physical_height: 0,
                orientation: 0,
                desktop_scale_factor: 100,
                device_scale_factor: 100,
            })
            .expect_err("dynamic display updates cannot change a negotiated monitor topology");
        assert_eq!(error.code(), E_NOTIMPL);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn monitor_topology_activates_when_server_layout_matches() {
        let control = Control::new();
        control.connection_generation.set(7);
        control.state.set(ConnectionState::Connecting);
        *control.configured_monitor_topology.borrow_mut() = Some(
            MonitorTopology::from_host_monitors(vec![HostMonitor {
                rect: RECT {
                    left: 0,
                    top: 0,
                    right: 800,
                    bottom: 600,
                },
                primary: true,
            }])
            .expect("a valid monitor topology"),
        );
        let monitors = control
            .configured_monitor_topology
            .borrow()
            .as_ref()
            .expect("the configured topology is available")
            .monitors
            .clone();
        control.events.events.lock().expect("event queue is available").extend([
            WorkerEvent::MonitorLayout {
                generation: 7,
                monitors,
            },
            WorkerEvent::Connected { generation: 7 },
            WorkerEvent::Image {
                generation: 7,
                buffer: vec![0],
                width: 800,
                height: 600,
            },
        ]);

        control.dispatch_pending_events();

        assert_eq!(
            control
                .active_monitor_topology
                .borrow()
                .as_ref()
                .expect("the matching topology is active")
                .monitors
                .len(),
            1
        );
    }

    #[test]
    fn monitor_topology_requires_a_matching_server_layout() {
        let control = Control::new();
        control.connection_generation.set(7);
        control.state.set(ConnectionState::Connecting);
        *control.configured_monitor_topology.borrow_mut() = Some(
            MonitorTopology::from_host_monitors(vec![
                HostMonitor {
                    rect: RECT {
                        left: 0,
                        top: 0,
                        right: 400,
                        bottom: 600,
                    },
                    primary: true,
                },
                HostMonitor {
                    rect: RECT {
                        left: 400,
                        top: 0,
                        right: 800,
                        bottom: 600,
                    },
                    primary: false,
                },
            ])
            .expect("a valid monitor topology"),
        );
        let mut monitors = control
            .configured_monitor_topology
            .borrow()
            .as_ref()
            .expect("the configured topology is available")
            .monitors
            .clone();
        monitors[1].right = 798;
        control.events.events.lock().expect("event queue is available").extend([
            WorkerEvent::MonitorLayout {
                generation: 7,
                monitors,
            },
            WorkerEvent::Connected { generation: 7 },
            WorkerEvent::Image {
                generation: 7,
                buffer: vec![0],
                width: 1,
                height: 1,
            },
        ]);

        control.dispatch_pending_events();

        assert!(control.active_monitor_topology.borrow().is_none());
        assert!(control.configured_monitor_topology.borrow().is_some());
        assert_eq!(
            control
                .update_display_layout(DisplayLayout {
                    desktop_width: 800,
                    desktop_height: 600,
                    physical_width: 0,
                    physical_height: 0,
                    orientation: 0,
                    desktop_scale_factor: 100,
                    device_scale_factor: 100,
                })
                .expect_err("a mismatched multimonitor layout still blocks dynamic display updates")
                .code(),
            E_NOTIMPL
        );
        assert_eq!(
            control.remote_monitor_bounds().expect("read remote frame bounds"),
            (0, 0, 1, 1)
        );
    }

    #[test]
    fn reconnect_queues_an_active_resize_and_reports_its_status() {
        let control = Control::new();
        let (sender, mut receiver) = RdpInputSender::channel(16);
        *control.input_sender.borrow_mut() = Some(sender);
        control.state.set(ConnectionState::Connected);
        let mut status = CONTROL_RECONNECT_BLOCKED;

        control
            .reconnect(1280, 720, &mut status)
            .expect("an active session can accept a reconnect resize");

        assert_eq!(status, CONTROL_RECONNECT_STARTED);
        assert!(matches!(
            receiver.try_recv(),
            Ok(RdpInputEvent::Resize {
                width: 1280,
                height: 720,
                scale_factor: 100,
                physical_size: None,
            })
        ));
    }

    #[test]
    fn reconnect_blocks_inactive_or_invalid_requests() {
        let control = Control::new();
        let mut status = CONTROL_RECONNECT_STARTED;

        let error = control
            .reconnect(1280, 720, &mut status)
            .expect_err("a disconnected session cannot reconnect");
        assert_eq!(error.code(), E_FAIL);
        assert_eq!(status, CONTROL_RECONNECT_BLOCKED);

        let (sender, _) = RdpInputSender::channel(16);
        *control.input_sender.borrow_mut() = Some(sender);
        control.state.set(ConnectionState::Connected);
        let error = control
            .reconnect(0, 720, &mut status)
            .expect_err("a zero display width is invalid");
        assert_eq!(error.code(), E_INVALIDARG);
        assert_eq!(status, CONTROL_RECONNECT_BLOCKED);
    }

    #[test]
    fn display_layout_rejects_invalid_or_unmapped_scaling() {
        let control = Control::new();
        let (sender, _) = RdpInputSender::channel(16);
        *control.input_sender.borrow_mut() = Some(sender);
        control.state.set(ConnectionState::Connected);
        let topology = MonitorTopology::from_host_monitors(vec![HostMonitor {
            rect: RECT {
                left: 0,
                top: 0,
                right: 800,
                bottom: 600,
            },
            primary: true,
        }])
        .expect("a valid single-monitor topology");
        *control.configured_monitor_topology.borrow_mut() = Some(topology.clone());
        *control.active_monitor_topology.borrow_mut() = Some(topology);

        let error = control
            .update_display_layout(DisplayLayout {
                desktop_width: 1280,
                desktop_height: 720,
                physical_width: 340,
                physical_height: 0,
                orientation: 0,
                desktop_scale_factor: 100,
                device_scale_factor: 100,
            })
            .expect_err("a partial physical size is invalid");
        assert_eq!(error.code(), E_INVALIDARG);

        let error = control
            .update_display_layout(DisplayLayout {
                desktop_width: 1280,
                desktop_height: 720,
                physical_width: 0,
                physical_height: 0,
                orientation: 0,
                desktop_scale_factor: 100,
                device_scale_factor: 125,
            })
            .expect_err("device scale without an IronRDP mapping is unsupported");
        assert_eq!(error.code(), E_NOTIMPL);

        let error = control
            .update_display_layout(DisplayLayout {
                desktop_width: 1280,
                desktop_height: 720,
                physical_width: 0,
                physical_height: 0,
                orientation: 90,
                desktop_scale_factor: 100,
                device_scale_factor: 100,
            })
            .expect_err("rotation without an IronRDP mapping is unsupported");
        assert_eq!(error.code(), E_NOTIMPL);
        assert!(control.active_monitor_topology.borrow().is_some());
    }

    #[test]
    fn client10_exposes_complete_last_five_client_interface_chain() {
        let control: IMsRdpClient10 = Control::new().into();

        assert!(control.cast::<IMsRdpClient6>().is_ok());
        assert!(control.cast::<IMsRdpClient7>().is_ok());
        assert!(control.cast::<IMsRdpClient8>().is_ok());
        assert!(control.cast::<IMsRdpClient9>().is_ok());
        assert!(control.cast::<IMsRdpClient10>().is_ok());
        assert!(control.cast::<IMsTscAx>().is_ok());

        let pointer_size = size_of::<usize>();
        assert_eq!(size_of::<IMsRdpClient6_Vtbl>(), 59 * pointer_size);
        assert_eq!(size_of::<IMsRdpClient7_Vtbl>(), 64 * pointer_size);
        assert_eq!(size_of::<IMsRdpClient8_Vtbl>(), 67 * pointer_size);
        assert_eq!(size_of::<IMsRdpClient9_Vtbl>(), 72 * pointer_size);
        assert_eq!(size_of::<IMsRdpClient10_Vtbl>(), 73 * pointer_size);
    }

    #[test]
    fn client12_coclass_probe_exposes_the_complete_non_scriptable8_contract() {
        let control: IMsRdpClient10 = Control::new().into();
        let non_scriptable = control
            .cast::<IMsRdpClientNonScriptable7>()
            .expect("control supports the v12 non-scriptable probe interface");
        let non_scriptable8 = control
            .cast::<IMsRdpClientNonScriptable8>()
            .expect("control supports the non-scriptable v8 interface");

        let pointer_size = size_of::<usize>();
        assert_eq!(size_of::<IMsRdpClientNonScriptable7_Vtbl>(), 70 * pointer_size);
        assert_eq!(size_of::<IMsRdpClientNonScriptable8_Vtbl>(), 73 * pointer_size);

        let mut correlation_id = GUID::zeroed();
        unsafe { non_scriptable8.get_CorrelationId(&mut correlation_id) }.expect("stable correlation identifier");
        assert_eq!(correlation_id, CLSID_IRONRDP_ACTIVEX);

        let mut clipboard = ptr::dangling_mut::<c_void>();
        unsafe { non_scriptable.get_Clipboard(&mut clipboard) }.expect("clipboard capability object");
        assert!(!clipboard.is_null());

        let clipboard = unsafe { IMsRdpClipboard::from_raw(clipboard) };
        let mut can_sync = VARIANT_TRUE.0;
        unsafe { clipboard.CanSyncLocalClipboardToRemoteSession(&mut can_sync) }
            .expect("check local clipboard synchronization");
        assert_eq!(can_sync, VARIANT_FALSE.0);
        let error = unsafe { clipboard.SyncLocalClipboardToRemoteSession() }
            .expect_err("a disconnected session must not claim clipboard synchronization");
        assert_eq!(error.code(), E_UNEXPECTED);

        let mut remote_monitor_count = u32::MAX;
        unsafe { non_scriptable.get_RemoteMonitorCount(&mut remote_monitor_count) }
            .expect("empty remote monitor collection");
        assert_eq!(remote_monitor_count, 0);
    }

    #[test]
    fn ole_clipboard_data_object_is_a_unicode_text_snapshot() {
        let data_object: IDataObject =
            ClipboardDataObject::from_unicode_text(Some(vec![b'i', 0, b'r', 0, b'o', 0, b'n', 0, 0, 0])).into();
        let format = unicode_text_format();

        assert_eq!(unsafe { data_object.QueryGetData(&format) }, S_OK);

        let enumerator =
            unsafe { data_object.EnumFormatEtc(DATADIR_GET.0 as u32) }.expect("enumerate source clipboard formats");
        let mut formats = [FORMATETC::default()];
        let mut fetched = 0;
        assert_eq!(unsafe { enumerator.Next(&mut formats, Some(&mut fetched)) }, S_OK);
        assert_eq!(fetched, 1);
        assert_eq!(formats[0].cfFormat, CF_UNICODETEXT.0);
        assert_eq!(formats[0].tymed, TYMED_HGLOBAL.0 as u32);
        assert_eq!(unsafe { enumerator.Next(&mut formats, Some(&mut fetched)) }, S_FALSE);
        assert_eq!(fetched, 0);

        let alternate_tymed = FORMATETC {
            tymed: TYMED_HGLOBAL.0 as u32 | windows::Win32::System::Com::TYMED_FILE.0 as u32,
            ..format
        };
        let mut canonical = FORMATETC::default();
        assert_eq!(
            unsafe { data_object.GetCanonicalFormatEtc(&alternate_tymed, &mut canonical) },
            DATA_S_SAMEFORMATETC
        );
        assert_eq!(canonical.tymed, alternate_tymed.tymed);
        assert!(canonical.ptd.is_null());

        let mut aliasing_format = alternate_tymed;
        let aliasing_format_pointer = &mut aliasing_format as *mut FORMATETC;
        assert_eq!(
            unsafe { data_object.GetCanonicalFormatEtc(aliasing_format_pointer, aliasing_format_pointer) },
            DATA_S_SAMEFORMATETC
        );
        assert_eq!(aliasing_format.tymed, alternate_tymed.tymed);
        assert!(aliasing_format.ptd.is_null());

        let mut medium = unsafe { data_object.GetData(&format) }.expect("retrieve clipboard snapshot");
        assert_eq!(medium.tymed, TYMED_HGLOBAL.0 as u32);
        let memory = unsafe { medium.u.hGlobal };
        let source = unsafe { GlobalLock(memory) }.cast::<u8>();
        assert!(!source.is_null());
        let copied = unsafe { slice::from_raw_parts(source, GlobalSize(memory)) };
        assert_eq!(copied, [b'i', 0, b'r', 0, b'o', 0, b'n', 0, 0, 0]);
        unlock_global_memory(memory).expect("unlock returned clipboard medium");
        unsafe {
            ReleaseStgMedium(&mut medium);
        }

        let invalid_tymed = FORMATETC { tymed: 0, ..format };
        assert_eq!(unsafe { data_object.QueryGetData(&invalid_tymed) }, DV_E_TYMED);
        let mut caller_medium = STGMEDIUM::default();
        assert_eq!(
            unsafe { data_object.GetDataHere(&format, &mut caller_medium) }
                .expect_err("GetDataHere requires the supported storage medium")
                .code(),
            DV_E_TYMED
        );
        caller_medium.tymed = TYMED_HGLOBAL.0 as u32;
        assert_eq!(
            unsafe { data_object.GetDataHere(&format, &mut caller_medium) }
                .expect_err("the snapshot does not accept caller-owned output storage")
                .code(),
            E_NOTIMPL
        );
        assert_eq!(
            unsafe { data_object.EnumFormatEtc(DATADIR_SET.0 as u32) }
                .expect_err("the snapshot must not advertise a destination")
                .code(),
            E_NOTIMPL
        );
    }

    #[test]
    fn ole_clipboard_snapshot_validation_rejects_malformed_text() {
        assert_eq!(
            validated_unicode_text_snapshot(&[0]).expect("reject undersized text"),
            None
        );
        assert_eq!(
            validated_unicode_text_snapshot(&[b'i', 0, 0]).expect("reject odd-length text"),
            None
        );
        assert_eq!(
            validated_unicode_text_snapshot(&vec![0; MAX_OLE_CLIPBOARD_TEXT_BYTES + 2]).expect("reject oversized text"),
            None
        );
        assert_eq!(
            validated_unicode_text_snapshot(&[b'i', 0, b'r', 0]).expect("reject unterminated text"),
            None
        );
        assert_eq!(
            validated_unicode_text_snapshot(&[0, 0xd8, 0, 0]).expect("reject invalid UTF-16"),
            None
        );
    }

    #[test]
    fn ole_clipboard_snapshot_stops_at_first_terminator() {
        let snapshot = validated_unicode_text_snapshot(&[b'i', 0, 0, 0, b'r', 0])
            .expect("accept valid Unicode text")
            .expect("return the text before the terminator");

        assert_eq!(snapshot, [b'i', 0, 0, 0]);
    }

    #[test]
    fn ole_clipboard_data_requires_active_redirection() {
        let control: IMsRdpClient10 = Control::new().into();
        let ole_object = control.cast::<IOleObject>().expect("control supports OLE data access");

        assert_eq!(
            unsafe { ole_object.GetClipboardData(0) }
                .expect_err("disconnected control must not expose a clipboard snapshot")
                .code(),
            OLE_E_NOTRUNNING
        );
        assert_eq!(
            unsafe { ole_object.GetClipboardData(1) }
                .expect_err("GetClipboardData reserved parameter must be zero")
                .code(),
            E_INVALIDARG
        );
    }

    #[test]
    fn connected_clipboard_and_remote_monitor_contracts_follow_session_state() {
        let control = Control::new();
        control.clipboard_state.enabled_for_session.set(true);
        control.clipboard_state.connected.set(true);
        control.remote_size.set(Some((1920, 1080)));
        assert_eq!(
            control.remote_monitor_bounds().expect("active remote monitor"),
            (0, 0, 1920, 1080)
        );
        let control: IMsRdpClient10 = control.into();
        let non_scriptable = control
            .cast::<IMsRdpClientNonScriptable7>()
            .expect("control supports the non-scriptable monitor and clipboard contract");

        let mut clipboard = ptr::null_mut();
        unsafe { non_scriptable.get_Clipboard(&mut clipboard) }.expect("clipboard capability object");
        let clipboard = unsafe { IMsRdpClipboard::from_raw(clipboard) };
        let mut can_sync = VARIANT_FALSE.0;
        unsafe { clipboard.CanSyncLocalClipboardToRemoteSession(&mut can_sync) }
            .expect("connected clipboard capability");
        assert_eq!(can_sync, VARIANT_TRUE.0);
        unsafe { clipboard.CanSyncRemoteClipboardToLocalSession(&mut can_sync) }
            .expect("connected remote clipboard capability");
        assert_eq!(can_sync, VARIANT_TRUE.0);
        unsafe { clipboard.SyncLocalClipboardToRemoteSession() }
            .expect("the native backend synchronizes local clipboard changes automatically");
        unsafe { clipboard.SyncRemoteClipboardToLocalSession() }
            .expect("the native backend synchronizes remote clipboard changes automatically");
    }

    #[test]
    fn dvc_plugin_paths_require_unique_absolute_existing_dlls() {
        let temporary_directory =
            std::env::temp_dir().join(format!("ironrdp-activex-dvc-plugin-test-{}", std::process::id()));
        std::fs::create_dir_all(&temporary_directory).expect("create temporary test directory");
        let plugin_path = temporary_directory.join("test-plugin.dll");
        std::fs::write(&plugin_path, []).expect("create placeholder plugin DLL");
        let plugin_path = plugin_path.canonicalize().expect("canonicalize placeholder plugin DLL");
        let plugin_path_string = plugin_path.to_string_lossy().into_owned();

        assert_eq!(
            validated_dvc_plugin_paths(&plugin_path_string).expect("accept a local DLL"),
            vec![plugin_path.clone()]
        );
        assert_eq!(
            validated_dvc_plugin_paths(&format!("{plugin_path_string};{plugin_path_string}"))
                .expect_err("reject duplicate plugin DLLs")
                .code(),
            E_INVALIDARG
        );
        assert_eq!(
            validated_dvc_plugin_paths("relative-plugin.dll")
                .expect_err("reject relative plugin DLL")
                .code(),
            E_INVALIDARG
        );
        assert_eq!(
            validated_dvc_plugin_paths(r"\\server\share\plugin.dll")
                .expect_err("reject UNC plugin DLL")
                .code(),
            E_INVALIDARG
        );
        assert_eq!(
            validated_dvc_plugin_paths(&format!("{plugin_path_string};"))
                .expect_err("reject empty plugin path entries")
                .code(),
            E_INVALIDARG
        );

        std::fs::remove_file(plugin_path).expect("remove placeholder plugin DLL");
        std::fs::remove_dir(temporary_directory).expect("remove temporary test directory");
    }

    #[test]
    fn stopping_clipboard_redirection_withdraws_the_session_capability() {
        let control = Control::new();
        control.clipboard_state.enabled_for_session.set(true);
        control.clipboard_state.connected.set(true);

        control.stop_clipboard_redirection();

        assert!(!control.clipboard_state.enabled_for_session.get());
        assert!(!control.clipboard_state.connected.get());
        assert!(control.clipboard_backend.borrow().is_none());
    }

    #[test]
    fn control_exposes_the_axhost_ole_activation_contract() {
        let control: IMsRdpClient10 = Control::new().into();

        let ole_object = control.cast::<IOleObject>().expect("control supports IOleObject");
        assert_eq!(
            unsafe { ole_object.GetUserClassID() }.expect("retrieve ActiveX user class ID"),
            CLSID_IRONRDP_ACTIVEX
        );
        let user_type = unsafe { ole_object.GetUserType(USERCLASSTYPE(0)) }.expect("retrieve ActiveX user type");
        let user_type_text = unsafe { user_type.to_string() }.expect("user type is a valid OLE string");
        unsafe {
            CoTaskMemFree(Some(user_type.0.cast()));
        }
        assert_eq!(user_type_text, "IronRDP ActiveX Control");

        let verbs = unsafe { ole_object.EnumVerbs() }.expect("enumerate ActiveX verbs");
        let mut verb = OLEVERB::default();
        let mut fetched = 0;
        unsafe {
            verbs
                .Next(std::slice::from_mut(&mut verb), Some(&mut fetched))
                .expect("retrieve the primary ActiveX verb");
        }
        assert_eq!(fetched, 1);
        assert_eq!(
            verb.lVerb,
            windows::Win32::System::Ole::OLEIVERB(OLEVERB_PRIMARY as i32)
        );
        assert_eq!(verb.grfAttribs, OLEVERBATTRIB_NEVERDIRTIES.0 as u32);
        let verb_name = unsafe { verb.lpszVerbName.to_string() }.expect("verb name is a valid OLE string");
        unsafe {
            CoTaskMemFree(Some(verb.lpszVerbName.0.cast()));
        }
        assert_eq!(verb_name, "&Open");
        let mut exhausted_verb = OLEVERB::default();
        let mut exhausted_count = u32::MAX;
        // S_FALSE is a successful HRESULT, so windows-core exposes an exhausted Next call as
        // `Ok(())`; the fetched count carries the end-of-enumeration result.
        unsafe {
            verbs
                .Next(std::slice::from_mut(&mut exhausted_verb), Some(&mut exhausted_count))
                .expect("verb enumeration ends after the primary verb");
        }
        assert_eq!(exhausted_count, 0);
        unsafe {
            verbs.Reset().expect("reset verb enumeration");
        }

        assert!(control.cast::<IOleInPlaceObject>().is_ok());
        let active_object = control
            .cast::<IOleInPlaceActiveObject>()
            .expect("control supports IOleInPlaceActiveObject");
        let error = unsafe { active_object.TranslateAccelerator(Some(ptr::null())) }
            .expect_err("an accelerator message is required");
        assert_eq!(error.code(), E_POINTER);

        let ole_control = control.cast::<IOleControl>().expect("control supports IOleControl");
        unsafe {
            ole_control.FreezeEvents(true).expect("freeze events once");
            ole_control.FreezeEvents(true).expect("freeze events twice");
            ole_control.FreezeEvents(false).expect("unfreeze one event level");
            ole_control.FreezeEvents(false).expect("unfreeze the final event level");
            ole_control.FreezeEvents(false).expect("ignore an unmatched unfreeze");
        }

        let persist = control.cast::<IPersist>().expect("control supports IPersist");
        assert_eq!(
            unsafe { persist.GetClassID() }.expect("retrieve ActiveX class ID"),
            CLSID_IRONRDP_ACTIVEX
        );
        let stream = control
            .cast::<IPersistStreamInit>()
            .expect("control supports IPersistStreamInit");
        unsafe {
            stream.InitNew().expect("initialize fresh ActiveX control");
        }
    }

    #[test]
    fn ole_control_reports_unsupported_accelerator_metadata() {
        let control: IMsRdpClient10 = Control::new().into();
        let ole_control = control.cast::<IOleControl>().expect("control supports IOleControl");

        let mut info = CONTROLINFO {
            cb: u32::MAX,
            cAccel: u16::MAX,
            dwFlags: u32::MAX,
            ..Default::default()
        };
        let error = unsafe { ole_control.GetControlInfo(&mut info) }
            .expect_err("ActiveX control does not provide an accelerator table");
        assert_eq!(error.code(), E_NOTIMPL);
        assert_eq!(info.cb, u32::MAX);
        assert_eq!(info.cAccel, u16::MAX);
        assert_eq!(info.dwFlags, u32::MAX);

        let error = unsafe { ole_control.GetControlInfo(ptr::null_mut()) }
            .expect_err("unsupported accelerator metadata does not validate an output pointer");
        assert_eq!(error.code(), E_NOTIMPL);

        let message = windows::Win32::UI::WindowsAndMessaging::MSG::default();
        let error = unsafe { ole_control.OnMnemonic(&message) }.expect_err("control has no mnemonic accelerator");
        assert_eq!(error.code(), E_NOTIMPL);
        unsafe {
            ole_control
                .OnAmbientPropertyChange(-1)
                .expect("unknown ambient properties do not require control work");
        }
    }

    #[test]
    fn ole_verbs_distinguish_safe_activation_from_unsupported_ui() {
        for verb in [
            OLEVERB_PRIMARY as i32,
            OLEIVERB_SHOW,
            OLEIVERB_OPEN,
            OLEIVERB_UIACTIVATE,
            OLEIVERB_INPLACEACTIVATE,
        ] {
            assert_eq!(
                ole_verb_action(verb).expect("public activation verb"),
                OleVerbAction::Activate
            );
        }
        assert_eq!(
            ole_verb_action(OLEIVERB_HIDE).expect("public hide verb"),
            OleVerbAction::Hide
        );
        assert_eq!(
            ole_verb_action(OLEIVERB_DISCARDUNDOSTATE).expect("discard undo state verb"),
            OleVerbAction::DiscardUndoState
        );
        for verb in [OLEIVERB_PROPERTIES, 1, -8] {
            let error = ole_verb_action(verb).expect_err("unsupported OLE verb");
            assert_eq!(error.code(), OLEOBJ_S_INVALIDVERB);
        }
    }

    #[test]
    fn disconnect_requires_an_active_rdp_worker() {
        let control = Control::new();
        let error = control
            .stop_connection()
            .expect_err("inactive control cannot disconnect");
        assert_eq!(error.code(), E_FAIL);

        let (sender, _) = RdpInputSender::channel(16);
        *control.input_sender.borrow_mut() = Some(sender);
        control.state.set(ConnectionState::Connected);
        control
            .stop_connection()
            .expect("active control can request disconnect");
        assert_eq!(control.state.get(), ConnectionState::Stopping);
    }

    #[test]
    fn connection_bar_eligibility_requires_a_connected_enabled_fullscreen_session() {
        let mut settings = Settings {
            fullscreen: true,
            ..Settings::default()
        };
        let mut compatibility = CompatibilitySettings {
            display_connection_bar: VARIANT_TRUE.0,
            ..CompatibilitySettings::default()
        };

        assert!(connection_bar_is_eligible(
            ConnectionState::Connected,
            &settings,
            &compatibility,
            false,
        ));
        assert!(!connection_bar_is_eligible(
            ConnectionState::Connecting,
            &settings,
            &compatibility,
            false,
        ));
        settings.fullscreen = false;
        assert!(!connection_bar_is_eligible(
            ConnectionState::Connected,
            &settings,
            &compatibility,
            false,
        ));
        settings.fullscreen = true;
        compatibility.connection_bar_disabled = true;
        assert!(!connection_bar_is_eligible(
            ConnectionState::Connected,
            &settings,
            &compatibility,
            false,
        ));
        compatibility.connection_bar_disabled = false;
        compatibility.display_connection_bar = VARIANT_FALSE.0;
        compatibility.display_connection_bar_set = true;
        assert!(!connection_bar_is_eligible(
            ConnectionState::Connected,
            &settings,
            &compatibility,
            false,
        ));
        compatibility.display_connection_bar_set = false;
        assert!(connection_bar_is_eligible(
            ConnectionState::Connected,
            &settings,
            &compatibility,
            true,
        ));
    }

    #[test]
    fn connection_bar_helpers_prefer_configured_text_and_center_on_owner_top_edge() {
        assert_eq!(
            connection_bar_title("Production session", "rdp.example.test"),
            "Production session"
        );
        assert_eq!(connection_bar_title("", "rdp.example.test"), "rdp.example.test");
        assert_eq!(
            connection_bar_position_for_width(
                RECT {
                    left: 100,
                    top: 50,
                    right: 1100,
                    bottom: 800,
                },
                CONNECTION_BAR_WIDTH,
            ),
            (200, 50)
        );

        let rect = RECT {
            left: 100,
            top: 50,
            right: 300,
            bottom: 100,
        };
        assert!(point_is_inside_rect(POINT { x: 100, y: 50 }, rect));
        assert!(point_is_inside_rect(POINT { x: 299, y: 99 }, rect));
        assert!(!point_is_inside_rect(POINT { x: 300, y: 99 }, rect));
        assert!(!point_is_inside_rect(POINT { x: 299, y: 100 }, rect));
    }

    #[test]
    fn connection_bar_layout_scales_with_owner_dpi() {
        assert_eq!(connection_bar_size(DEFAULT_DPI), (800, 36));
        assert_eq!(connection_bar_size(144), (1200, 54));
        assert_eq!(
            connection_bar_position_for_width(
                RECT {
                    left: 100,
                    top: 50,
                    right: 1100,
                    bottom: 800,
                },
                1200,
            ),
            (0, 50)
        );
        assert_eq!(
            connection_bar_title_rect(144),
            RECT {
                left: 12,
                top: 9,
                right: 270,
                bottom: 45,
            }
        );
        assert_eq!(
            connection_bar_button_rect(CONNECTION_BAR_PIN_BUTTON_ID, 144),
            Some(RECT {
                left: 384,
                top: 9,
                right: 504,
                bottom: 45,
            })
        );
    }

    #[test]
    fn connection_bar_tab_navigation_cycles_only_visible_buttons() {
        let visible = [
            CONNECTION_BAR_INFORMATION_BUTTON_ID,
            CONNECTION_BAR_PIN_BUTTON_ID,
            CONNECTION_BAR_FULLSCREEN_BUTTON_ID,
            CONNECTION_BAR_DISCONNECT_BUTTON_ID,
        ];

        assert_eq!(
            next_connection_bar_button_id(CONNECTION_BAR_INFORMATION_BUTTON_ID, &visible, false),
            Some(CONNECTION_BAR_PIN_BUTTON_ID)
        );
        assert_eq!(
            next_connection_bar_button_id(CONNECTION_BAR_PIN_BUTTON_ID, &visible, false),
            Some(CONNECTION_BAR_FULLSCREEN_BUTTON_ID)
        );
        assert_eq!(
            next_connection_bar_button_id(CONNECTION_BAR_FULLSCREEN_BUTTON_ID, &visible, false),
            Some(CONNECTION_BAR_DISCONNECT_BUTTON_ID)
        );
        assert_eq!(
            next_connection_bar_button_id(CONNECTION_BAR_DISCONNECT_BUTTON_ID, &visible, false),
            Some(CONNECTION_BAR_INFORMATION_BUTTON_ID)
        );
        assert_eq!(
            next_connection_bar_button_id(CONNECTION_BAR_INFORMATION_BUTTON_ID, &visible, true),
            Some(CONNECTION_BAR_DISCONNECT_BUTTON_ID)
        );
        assert_eq!(
            next_connection_bar_button_id(CONNECTION_BAR_MINIMIZE_BUTTON_ID, &visible, true),
            Some(CONNECTION_BAR_DISCONNECT_BUTTON_ID)
        );
        assert_eq!(
            next_connection_bar_button_id(CONNECTION_BAR_PIN_BUTTON_ID, &[], false),
            None
        );
        assert_ne!(connection_bar_button_style(true).0 & WS_TABSTOP.0, 0);
        assert_ne!(connection_bar_button_style(true).0 & WS_VISIBLE.0, 0);
        assert_ne!(connection_bar_button_style(false).0 & WS_TABSTOP.0, 0);
        assert_eq!(connection_bar_button_style(false).0 & WS_VISIBLE.0, 0);
    }

    #[test]
    fn connection_security_warnings_follow_public_setting_order() {
        assert_eq!(
            connection_security_warnings(true, true),
            vec![
                ConnectionSecurityWarning::SendingCredentials,
                ConnectionSecurityWarning::ClipboardRedirection,
            ]
        );
        assert_eq!(
            connection_security_warnings(false, true),
            vec![ConnectionSecurityWarning::ClipboardRedirection]
        );
        assert!(connection_security_warnings(false, false).is_empty());
    }

    #[test]
    fn connection_information_uses_only_actual_session_state() {
        assert_eq!(
            connection_information_content(ConnectionState::Connected, Some((1920, 1080)), true),
            Some(
                "Connection status: Connected\r\nDesktop size: 1920 x 1080\r\nClipboard redirection: Enabled"
                    .to_owned()
            )
        );
        assert_eq!(
            connection_information_content(ConnectionState::Connected, None, false),
            Some("Connection status: Connected\r\nClipboard redirection: Disabled".to_owned())
        );
        assert_eq!(
            connection_information_content(ConnectionState::Disconnected, Some((1920, 1080)), true),
            None
        );
    }

    #[test]
    fn connection_bar_disconnect_confirmation_is_neutral_and_requires_a_session() {
        assert_eq!(
            connection_bar_disconnect_prompt(),
            (
                "IronRDP disconnect",
                "Disconnect from the remote desktop session?",
                "The remote desktop session will be disconnected.",
            )
        );

        let control = Control::new();
        assert!(
            !control
                .confirm_connection_bar_disconnect()
                .expect("a disconnected control does not require a confirmation dialog")
        );
    }

    #[test]
    fn connection_health_status_uses_only_known_reconnect_attempts() {
        assert_eq!(ConnectionHealthStatus::Connecting.text(), ("Connecting...", None));
        assert_eq!(
            ConnectionHealthStatus::UpdatingDisplay.text(),
            ("Updating remote display...", None)
        );
        assert_eq!(
            ConnectionHealthStatus::reconnecting(2, 3)
                .expect("a bounded worker retry is displayable")
                .text(),
            ("Reconnecting...", Some("Attempt 2 of 3".to_owned()))
        );
        assert_eq!(ConnectionHealthStatus::reconnecting(0, 3), None);
        assert_eq!(ConnectionHealthStatus::reconnecting(4, 3), None);
    }

    #[test]
    fn display_resize_fallback_shows_only_actual_connection_progress() {
        let control = Control::new();

        control.report_display_resize_fallback();
        assert_eq!(control.connection_health_status.get(), ConnectionHealthStatus::Hidden);

        control.state.set(ConnectionState::Connected);
        control.report_display_resize_fallback();
        assert_eq!(
            control.connection_health_status.get(),
            ConnectionHealthStatus::UpdatingDisplay
        );
    }

    #[test]
    fn connection_health_teardown_is_idempotent() {
        let control = Control::new();

        control.set_connection_health_status(ConnectionHealthStatus::Connecting);
        assert_eq!(
            control.connection_health_status.get(),
            ConnectionHealthStatus::Connecting
        );

        control.report_reconnect_worker_progress(2, 3);
        assert_eq!(
            control.connection_health_status.get(),
            ConnectionHealthStatus::Reconnecting { attempt: 2, maximum: 3 }
        );

        control.report_reconnect_worker_progress(0, 3);
        assert_eq!(
            control.connection_health_status.get(),
            ConnectionHealthStatus::Reconnecting { attempt: 2, maximum: 3 }
        );

        control.clear_connection_health_window();
        assert_eq!(control.connection_health_status.get(), ConnectionHealthStatus::Hidden);
        control.clear_connection_health_window();
        assert_eq!(control.connection_health_status.get(), ConnectionHealthStatus::Hidden);
    }

    #[test]
    fn ui_deactivation_releases_transient_connection_ui() {
        let control = Control::new();
        control.set_connection_health_status(ConnectionHealthStatus::Connecting);

        control.deactivate_owned_ui();
        assert_eq!(control.connection_health_status.get(), ConnectionHealthStatus::Hidden);

        control.deactivate_owned_ui();
        assert_eq!(control.connection_health_status.get(), ConnectionHealthStatus::Hidden);
    }

    #[test]
    fn hidden_renderer_releases_transient_connection_ui() {
        let control = Control::new();
        control.set_connection_health_status(ConnectionHealthStatus::UpdatingDisplay);

        control.renderer_visibility_changed(false);
        assert_eq!(control.connection_health_status.get(), ConnectionHealthStatus::Hidden);

        control.renderer_visibility_changed(true);
        assert_eq!(control.connection_health_status.get(), ConnectionHealthStatus::Hidden);
    }

    #[test]
    fn ole_window_deactivation_releases_pressed_remote_input() {
        let control = Control::new();
        let key = Scancode::from_u8(true, 0x5b);

        control.apply_input([Operation::KeyPressed(key)]);
        control.in_place_window_activation_changed(true);
        assert!(control.input_database.borrow().is_key_pressed(key));

        control.in_place_window_activation_changed(false);
        assert!(!control.input_database.borrow().is_key_pressed(key));
    }

    #[test]
    fn renderer_cancellation_and_disable_release_held_remote_input() {
        let control = Control::new();
        let key = Scancode::from_u8(true, 0x5b);

        control.apply_input([
            Operation::KeyPressed(key),
            Operation::MouseButtonPressed(MouseButton::Left),
        ]);
        assert!(!control.handle_activex_window_message(HWND(ptr::null_mut()), WM_CANCELMODE, WPARAM(0), LPARAM(0)));
        assert!(!control.input_database.borrow().is_key_pressed(key));
        assert!(
            !control
                .input_database
                .borrow()
                .is_mouse_button_pressed(MouseButton::Left)
        );

        control.apply_input([
            Operation::KeyPressed(key),
            Operation::MouseButtonPressed(MouseButton::Left),
        ]);
        assert!(!control.handle_activex_window_message(HWND(ptr::null_mut()), WM_ENABLE, WPARAM(0), LPARAM(0)));
        assert!(!control.input_database.borrow().is_key_pressed(key));
        assert!(
            !control
                .input_database
                .borrow()
                .is_mouse_button_pressed(MouseButton::Left)
        );
    }

    #[test]
    fn renderer_teardown_releases_input_without_changing_connection_state() {
        let control = Control::new();
        let key = Scancode::from_u8(true, 0x5b);
        control.state.set(ConnectionState::Connected);
        control.apply_input([Operation::KeyPressed(key)]);

        control
            .destroy_activex_window()
            .expect("windowless teardown is idempotent");

        assert_eq!(control.state.get(), ConnectionState::Connected);
        assert!(!control.input_database.borrow().is_key_pressed(key));
    }

    #[test]
    fn unexpected_renderer_destruction_releases_owned_resources_without_disconnect() {
        let control = Control::new();
        let renderer = HWND(ptr::dangling_mut());
        let key = Scancode::from_u8(true, 0x5b);
        control.state.set(ConnectionState::Connected);
        control.activex_window.set(renderer);
        control.compatibility.borrow_mut().renderer_window = renderer;
        control.renderer_class_acquired.set(true);
        control.set_connection_health_status(ConnectionHealthStatus::UpdatingDisplay);
        control.apply_input([
            Operation::KeyPressed(key),
            Operation::MouseButtonPressed(MouseButton::Left),
        ]);

        assert!(control.renderer_destroyed_unexpectedly(renderer));

        assert_eq!(control.state.get(), ConnectionState::Connected);
        assert!(control.activex_window.get().0.is_null());
        assert!(control.compatibility.borrow().renderer_window.0.is_null());
        assert!(!control.renderer_class_acquired.get());
        assert_eq!(control.connection_health_status.get(), ConnectionHealthStatus::Hidden);
        assert!(!control.input_database.borrow().is_key_pressed(key));
        assert!(
            !control
                .input_database
                .borrow()
                .is_mouse_button_pressed(MouseButton::Left)
        );
    }

    #[test]
    fn modeless_state_is_retained_without_an_active_connection_bar() {
        let control = Control::new();

        assert!(control.connection_bar_modeless_enabled.get());
        control.set_connection_bar_modeless_enabled(false);
        assert!(!control.connection_bar_modeless_enabled.get());
        control.set_connection_bar_modeless_enabled(true);
        assert!(control.connection_bar_modeless_enabled.get());
    }

    #[test]
    fn renderer_geometry_reflow_is_safe_without_owned_ui() {
        let control = Control::new();
        control.renderer_dpi_changed();
        control.renderer_geometry_changed();
        control.renderer_geometry_changed();
    }

    #[test]
    fn ole_advise_sinks_receive_control_owned_notifications() {
        let control: IMsRdpClient10 = Control::new().into();
        let ole_object = control
            .cast::<IOleObject>()
            .expect("control supports OLE object advising");
        let views = Arc::new(AtomicU32::new(0));
        let saves = Arc::new(AtomicU32::new(0));
        let closes = Arc::new(AtomicU32::new(0));
        let sink: IAdviseSink = OleAdviseSink {
            views: Arc::clone(&views),
            saves: Arc::clone(&saves),
            closes: Arc::clone(&closes),
        }
        .into();

        let cookie = unsafe { ole_object.Advise(&sink) }.expect("register OLE advise sink");
        let enumerator = unsafe { ole_object.EnumAdvise() }.expect("enumerate OLE advise sinks");
        let mut entry = STATDATA::default();
        let mut fetched = 0;
        unsafe {
            enumerator
                .Next(std::slice::from_mut(&mut entry), Some(&mut fetched))
                .expect("retrieve registered OLE advise sink");
        }
        assert_eq!(fetched, 1);
        assert_eq!(entry.dwConnection, cookie);
        assert!(unsafe { ManuallyDrop::take(&mut entry.pAdvSink) }.is_some());
        let clone = unsafe { enumerator.Clone() }.expect("clone exhausted advise enumeration");
        let mut exhausted = STATDATA::default();
        let mut exhausted_count = 1;
        unsafe {
            clone
                .Next(std::slice::from_mut(&mut exhausted), Some(&mut exhausted_count))
                .expect("enumerate exhausted OLE advise clone");
        }
        assert_eq!(exhausted_count, 0);
        unsafe {
            enumerator.Reset().expect("reset OLE advise enumeration");
            enumerator.Skip(1).expect("skip registered OLE advise sink");
        }

        let extent = SIZE { cx: 12_700, cy: 12_700 };
        unsafe {
            ole_object
                .SetExtent(DVASPECT_CONTENT, &extent)
                .expect("update ActiveX extent");
        }
        assert_eq!(views.load(Ordering::Relaxed), 1);
        let unsupported_aspect = DVASPECT(2);
        assert_eq!(
            unsafe { ole_object.SetExtent(unsupported_aspect, &extent) }
                .expect_err("unsupported extent aspect must fail")
                .code(),
            DV_E_DVASPECT
        );
        assert_eq!(
            unsafe { ole_object.GetExtent(unsupported_aspect) }
                .expect_err("unsupported extent query must fail")
                .code(),
            DV_E_DVASPECT
        );
        assert_eq!(
            unsafe { ole_object.GetMiscStatus(unsupported_aspect) }
                .expect_err("unsupported misc-status aspect must fail")
                .code(),
            DV_E_DVASPECT
        );

        let persist = control
            .cast::<IPersistStreamInit>()
            .expect("control supports persistence");
        let stream = unsafe { CreateStreamOnHGlobal(HGLOBAL::default(), true) }.expect("create memory stream");
        unsafe {
            persist.Save(&stream, false).expect("save ActiveX settings");
        }
        assert_eq!(saves.load(Ordering::Relaxed), 1);

        unsafe {
            ole_object.Close(OLECLOSE_NOSAVE).expect("close ActiveX control");
        }
        assert_eq!(closes.load(Ordering::Relaxed), 1);
        unsafe {
            ole_object.Unadvise(cookie).expect("release OLE advise sink");
        }
        let error = unsafe { ole_object.Unadvise(cookie) }.expect_err("unknown OLE advise cookie");
        assert_eq!(error.code(), OLE_E_NOCONNECTION);
    }

    #[test]
    fn view_objects_report_the_windowed_extent_without_claiming_offscreen_drawing() {
        let control: IMsRdpClient10 = Control::new().into();
        let ole_object = control
            .cast::<IOleObject>()
            .expect("control supports OLE object contracts");
        let view: IViewObject = control.cast().expect("control supports IViewObject");
        let view2: IViewObject2 = control.cast().expect("control supports IViewObject2");
        let view_ex: IViewObjectEx = control.cast().expect("control supports IViewObjectEx");
        let extent = SIZE { cx: 12_700, cy: 6_350 };
        unsafe {
            ole_object
                .SetExtent(DVASPECT_CONTENT, &extent)
                .expect("set the control extent before querying view objects");
        }

        assert_eq!(
            unsafe { view2.GetExtent(DVASPECT_CONTENT, -1, ptr::null()) }.expect("content extent"),
            extent
        );
        assert_eq!(
            unsafe { view_ex.GetNaturalExtent(DVASPECT_CONTENT, -1, ptr::null(), HDC::default(), ptr::null()) }
                .expect("natural content extent"),
            extent
        );
        assert_eq!(
            unsafe { view_ex.GetRect(DVASPECT_CONTENT.0) }.expect("content view rectangle"),
            RECTL {
                left: 0,
                top: 0,
                right: extent.cx,
                bottom: extent.cy,
            }
        );
        assert_eq!(
            unsafe { view_ex.GetViewStatus() }.expect("view status"),
            (VIEWSTATUS_OPAQUE.0 | VIEWSTATUS_SOLIDBKGND.0) as u32
        );

        let bounds = RECT {
            left: 0,
            top: 0,
            right: extent.cx,
            bottom: extent.cy,
        };
        assert_eq!(
            unsafe { view_ex.QueryHitPoint(DVASPECT_CONTENT.0, &bounds, POINT { x: 1, y: 1 }, 0) }
                .expect("point inside the content bounds"),
            HITRESULT_HIT.0 as u32
        );
        assert_eq!(
            unsafe {
                view_ex.QueryHitRect(
                    DVASPECT_CONTENT.0,
                    &bounds,
                    &RECT {
                        left: extent.cx,
                        top: extent.cy,
                        right: extent.cx + 1,
                        bottom: extent.cy + 1,
                    },
                    0,
                )
            }
            .expect("non-overlapping rectangle query"),
            HITRESULT_OUTSIDE.0 as u32
        );

        let mut freeze = u32::MAX;
        assert_eq!(
            unsafe { view.Freeze(DVASPECT_CONTENT, -1, ptr::null_mut(), &mut freeze) }
                .expect_err("windowed renderer does not expose an off-screen frozen view")
                .code(),
            E_NOTIMPL
        );
        assert_eq!(freeze, 0);
        assert_eq!(
            unsafe {
                view.Draw(
                    DVASPECT_CONTENT,
                    -1,
                    ptr::null_mut(),
                    None,
                    None,
                    HDC::default(),
                    None,
                    None,
                    None,
                    0,
                )
            }
            .expect_err("windowed renderer does not expose an off-screen draw target")
            .code(),
            E_NOTIMPL
        );
        assert_eq!(
            unsafe { view2.GetExtent(DVASPECT_CONTENT, 0, ptr::null()) }
                .expect_err("view index must describe the complete control")
                .code(),
            DV_E_LINDEX
        );
        assert_eq!(
            unsafe { view_ex.GetRect(2) }
                .expect_err("unsupported view aspect")
                .code(),
            DV_E_DVASPECT
        );
    }

    #[test]
    fn view_advise_tracks_its_single_sink_and_releases_it_when_cleared() {
        let control: IMsRdpClient10 = Control::new().into();
        let view: IViewObject = control.cast().expect("control supports IViewObject");
        let views = Arc::new(AtomicU32::new(0));
        let sink: IAdviseSink = OleAdviseSink {
            views: Arc::clone(&views),
            saves: Arc::new(AtomicU32::new(0)),
            closes: Arc::new(AtomicU32::new(0)),
        }
        .into();

        unsafe {
            view.SetAdvise(DVASPECT_CONTENT, 0x42, &sink)
                .expect("set content view advise");
        }
        let mut aspects = 0;
        let mut flags = 0;
        let mut stored_sink = None;
        unsafe {
            view.GetAdvise(Some(&mut aspects), Some(&mut flags), &mut stored_sink)
                .expect("read view advise");
        }
        assert_eq!(aspects, DVASPECT_CONTENT.0);
        assert_eq!(flags, 0x42);
        assert!(stored_sink.is_some());
        drop(stored_sink);

        let ole_object = control
            .cast::<IOleObject>()
            .expect("control supports OLE object contracts");
        unsafe {
            ole_object
                .SetExtent(DVASPECT_CONTENT, &SIZE { cx: 12_700, cy: 12_700 })
                .expect("update extent to notify view advise sink");
        }
        assert_eq!(views.load(Ordering::Relaxed), 1);

        unsafe {
            view.SetAdvise(DVASPECT_CONTENT, 0, None::<&IAdviseSink>)
                .expect("clear view advise");
        }
        let mut cleared_sink = Some(sink);
        unsafe {
            view.GetAdvise(None, None, &mut cleared_sink)
                .expect("read cleared view advise");
        }
        assert!(cleared_sink.is_none());
    }

    #[test]
    fn persist_stream_round_trips_bounded_non_secret_connection_settings() {
        let source = Control::new();
        *source.settings.borrow_mut() = Settings {
            server: "rdp.example.test".to_owned(),
            domain: "EXAMPLE".to_owned(),
            username: "operator".to_owned(),
            disconnected_text: "Offline".to_owned(),
            desktop_width: 1280,
            desktop_height: 720,
            color_depth: 16,
            start_connected: true,
            fullscreen: true,
            ..Settings::default()
        };
        {
            let mut compatibility = source.compatibility.borrow_mut();
            compatibility.smart_sizing = true;
            compatibility.zoom_level = 175;
            compatibility.client_name = Some("IRDX-CLIENT".to_owned());
            compatibility.performance_flags =
                PerformanceFlags::DISABLE_WALLPAPER | PerformanceFlags::ENABLE_FONT_SMOOTHING;
            compatibility.keyboard_layout = 0x0000_0409;
        }
        source.persistence_dirty.set(true);
        let source_persist: IPersistStreamInit = source.into();
        assert_eq!(unsafe { source_persist.IsDirty() }, S_OK);

        let stream = unsafe { CreateStreamOnHGlobal(HGLOBAL::default(), true) }.expect("create memory stream");
        unsafe {
            source_persist.Save(&stream, true).expect("save settings");
            stream.Seek(0, STREAM_SEEK_SET, None).expect("rewind stream");
        }
        assert_eq!(unsafe { source_persist.IsDirty() }, S_FALSE);

        let destination_persist: IPersistStreamInit = Control::new().into();
        unsafe {
            destination_persist.Load(&stream).expect("load settings");
        }
        assert_eq!(unsafe { destination_persist.IsDirty() }, S_FALSE);

        let round_trip_stream =
            unsafe { CreateStreamOnHGlobal(HGLOBAL::default(), true) }.expect("create memory stream");
        unsafe {
            destination_persist
                .Save(&round_trip_stream, true)
                .expect("save restored settings");
            round_trip_stream
                .Seek(0, STREAM_SEEK_SET, None)
                .expect("rewind restored stream");
        }
        let restored = load_persisted_settings(&round_trip_stream).expect("deserialize restored settings");
        assert_eq!(restored.settings.server, "rdp.example.test");
        assert_eq!(restored.settings.domain, "EXAMPLE");
        assert_eq!(restored.settings.username, "operator");
        assert_eq!(restored.settings.disconnected_text, "Offline");
        assert_eq!(
            (
                restored.settings.desktop_width,
                restored.settings.desktop_height,
                restored.settings.color_depth,
                restored.settings.start_connected,
                restored.settings.fullscreen,
            ),
            (1280, 720, 16, true, true)
        );
        assert!(restored.compatibility.smart_sizing);
        assert_eq!(restored.compatibility.zoom_level, 175);
        assert_eq!(restored.compatibility.client_name.as_deref(), Some("IRDX-CLIENT"));
        assert_eq!(
            restored.compatibility.performance_flags,
            PerformanceFlags::DISABLE_WALLPAPER | PerformanceFlags::ENABLE_FONT_SMOOTHING
        );
        assert_eq!(restored.compatibility.keyboard_layout, 0x0000_0409);
    }

    #[test]
    fn persistence_format_excludes_passwords_and_rejects_invalid_headers() {
        let settings = Settings {
            server: "rdp.example.test".to_owned(),
            password: Some("must-not-be-persisted".to_owned()),
            ..Settings::default()
        };
        let compatibility = CompatibilitySettings::default();
        let bytes = persisted_settings_bytes(&settings, &compatibility).expect("serialize settings");
        assert!(
            !bytes
                .windows("must-not-be-persisted".len())
                .any(|window| window == b"must-not-be-persisted")
        );

        let stream = unsafe { CreateStreamOnHGlobal(HGLOBAL::default(), true) }.expect("create memory stream");
        let mut invalid = bytes;
        invalid[4] = (PERSISTENCE_VERSION + 1) as u8;
        stream_write_all(&stream, &invalid).expect("write invalid format");
        unsafe {
            stream.Seek(0, STREAM_SEEK_SET, None).expect("rewind stream");
        }
        let error = match load_persisted_settings(&stream) {
            Ok(_) => panic!("unsupported version must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error.code(), E_NOTIMPL);
    }

    #[test]
    fn persistence_version_one_uses_safe_compatibility_defaults() {
        let strings = ["rdp.example.test", "", "", "", "", "", ""];
        let payload_size = 2 + 2 + 4 + 1 + strings.iter().map(|value| 4 + value.len()).sum::<usize>();
        let mut bytes = Vec::with_capacity(10 + payload_size);
        bytes.extend_from_slice(&PERSISTENCE_MAGIC);
        bytes.extend_from_slice(&PERSISTENCE_VERSION_1.to_le_bytes());
        bytes.extend_from_slice(&(payload_size as u32).to_le_bytes());
        bytes.extend_from_slice(&1024u16.to_le_bytes());
        bytes.extend_from_slice(&768u16.to_le_bytes());
        bytes.extend_from_slice(&32u32.to_le_bytes());
        bytes.push(0);
        for value in strings {
            bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }

        let stream = unsafe { CreateStreamOnHGlobal(HGLOBAL::default(), true) }.expect("create memory stream");
        stream_write_all(&stream, &bytes).expect("write version one settings");
        unsafe {
            stream.Seek(0, STREAM_SEEK_SET, None).expect("rewind stream");
        }
        let restored = load_persisted_settings(&stream).expect("load version one settings");
        assert_eq!(restored.settings.server, "rdp.example.test");
        assert!(!restored.compatibility.smart_sizing);
        assert_eq!(restored.compatibility.zoom_level, 100);
        assert!(restored.compatibility.client_name.is_none());
        assert_eq!(restored.compatibility.performance_flags, PerformanceFlags::default());
        assert_eq!(restored.compatibility.keyboard_layout, 0);
    }

    #[test]
    fn persisted_stream_transfer_rejects_oversized_completion_counts() {
        assert_eq!(
            stream_transfer_offset(2, 3, 4)
                .expect_err("a stream cannot complete more bytes than requested")
                .code(),
            E_FAIL
        );
        assert_eq!(stream_transfer_offset(2, 2, 4).expect("exact completion"), 4);
    }

    #[test]
    fn non_scriptable_password_and_extended_settings_honor_com_output_contracts() {
        let control: IMsRdpClient10 = Control::new().into();
        let non_scriptable = control
            .cast::<IMsTscNonScriptable>()
            .expect("control supports IMsTscNonScriptable");
        let password = BSTR::from("password");
        unsafe {
            non_scriptable
                .put_ClearTextPassword(password.as_ptr())
                .expect("set clear-text password");
        }

        let mut unavailable_password = ptr::dangling::<u16>();
        let error = unsafe { non_scriptable.get_PortablePassword(&mut unavailable_password) }
            .expect_err("portable passwords are unsupported");
        assert_eq!(error.code(), E_NOTIMPL);
        assert!(unavailable_password.is_null());
        unsafe {
            non_scriptable.ResetPassword().expect("clear password");
        }

        let non_scriptable3 = control
            .cast::<IMsRdpClientNonScriptable3>()
            .expect("control supports IMsRdpClientNonScriptable3");
        let connection_bar_text = BSTR::from("Production session");
        unsafe {
            non_scriptable3
                .put_ConnectionBarText(connection_bar_text.as_ptr())
                .expect("set connection bar text");
        }
        let mut returned_connection_bar_text = ptr::null();
        unsafe {
            non_scriptable3
                .get_ConnectionBarText(&mut returned_connection_bar_text)
                .expect("get connection bar text");
        }
        let returned_connection_bar_text = unsafe { BSTR::from_raw(returned_connection_bar_text) };
        assert_eq!(
            String::try_from(&returned_connection_bar_text).expect("connection bar text BSTR"),
            "Production session"
        );

        let non_scriptable4 = control
            .cast::<IMsRdpClientNonScriptable4>()
            .expect("control supports IMsRdpClientNonScriptable4");
        unsafe {
            non_scriptable4
                .put_PromptForCredsOnClient(VARIANT_TRUE.0)
                .expect("enable credential prompting through the client-shell alias");
        }
        let mut prompt_for_credentials = VARIANT_FALSE.0;
        unsafe {
            non_scriptable3
                .get_PromptForCredentials(&mut prompt_for_credentials)
                .expect("get credential-prompt policy");
        }
        assert_eq!(prompt_for_credentials, VARIANT_TRUE.0);

        let non_scriptable5 = control
            .cast::<IMsRdpClientNonScriptable5>()
            .expect("control supports IMsRdpClientNonScriptable5");
        unsafe {
            non_scriptable5
                .put_DisableConnectionBar(1)
                .expect("disable connection bar with a nonzero VARIANT_BOOL");
        }
        unsafe {
            non_scriptable5
                .put_AllowPromptingForCredentials(VARIANT_FALSE.0)
                .expect("disable credential prompting through the newer alias");
        }
        let mut prompt_for_credentials = VARIANT_TRUE.0;
        unsafe {
            non_scriptable4
                .get_PromptForCredsOnClient(&mut prompt_for_credentials)
                .expect("get client-shell credential-prompt policy");
        }
        assert_eq!(prompt_for_credentials, VARIANT_FALSE.0);

        let extended = control
            .cast::<IMsRdpExtendedSettings>()
            .expect("control supports IMsRdpExtendedSettings");
        let mut value = variant_u32(125);
        unsafe {
            extended
                .put_Property(BSTR::from("ZoomLevel").as_ptr(), &mut value)
                .expect("accept the host's unsigned zoom level");
        }
        let mut value = VARIANT::default();
        unsafe {
            extended
                .get_Property(BSTR::from("ZoomLevel").as_ptr(), &mut value)
                .expect("get retained zoom level");
        }
        assert_eq!(
            variant_i32_value(&value, ptr::null_mut()).expect("integer zoom level"),
            125
        );
        assert_eq!(variant_header(&value).vt, VT_I4);

        let mut client_name = variant_bstr("RDM-CLIENT".to_owned());
        unsafe {
            extended
                .put_Property(BSTR::from("ClientDeviceName").as_ptr(), &mut client_name)
                .expect("set RDP client name");
        }
        free_owned_bstr_variant(&mut client_name);

        let mut client_name = VARIANT::default();
        unsafe {
            extended
                .get_Property(BSTR::from("ClientDeviceName").as_ptr(), &mut client_name)
                .expect("get retained RDP client name");
        }
        assert_eq!(
            variant_bstr_value(&client_name).expect("client-name BSTR"),
            "RDM-CLIENT"
        );
        free_owned_bstr_variant(&mut client_name);

        let mut disable_udp = variant_bool_value(true);
        unsafe {
            extended
                .put_Property(BSTR::from("DisableUdpTransport").as_ptr(), &mut disable_udp)
                .expect("confirm no UDP transport");
        }

        let mut enable_tls = variant_bool_value(false);
        unsafe {
            extended
                .put_Property(BSTR::from(ACTIVEX_ENABLE_TLS_PROPERTY).as_ptr(), &mut enable_tls)
                .expect("configure the legacy TLS policy");
        }
        let mut enable_tls = VARIANT::default();
        unsafe {
            extended
                .get_Property(BSTR::from(ACTIVEX_ENABLE_TLS_PROPERTY).as_ptr(), &mut enable_tls)
                .expect("get the legacy TLS policy");
        }
        assert!(!variant_bool(&enable_tls, ptr::null_mut()).expect("TLS policy boolean"));

        let mut autologon = variant_bool_value(true);
        unsafe {
            extended
                .put_Property(BSTR::from(ACTIVEX_AUTOLOGON_PROPERTY).as_ptr(), &mut autologon)
                .expect("configure autologon");
        }
        let mut autologon = VARIANT::default();
        unsafe {
            extended
                .get_Property(BSTR::from(ACTIVEX_AUTOLOGON_PROPERTY).as_ptr(), &mut autologon)
                .expect("get autologon");
        }
        assert!(variant_bool(&autologon, ptr::null_mut()).expect("autologon boolean"));

        for (name, configured, expected) in [
            (ACTIVEX_DESKTOP_SCALE_FACTOR_PROPERTY, 150, 150),
            (ACTIVEX_COMPRESSION_LEVEL_PROPERTY, 3, 3),
            (ACTIVEX_CLIENT_BUILD_PROPERTY, 10_001, 10_001),
            (ACTIVEX_FAKE_EVENTS_INTERVAL_PROPERTY, 10, 10),
        ] {
            let mut configured = variant_i32(configured);
            unsafe {
                extended
                    .put_Property(BSTR::from(name).as_ptr(), &mut configured)
                    .expect("configure a bounded IronRDP integer setting");
            }
            let mut returned = VARIANT::default();
            unsafe {
                extended
                    .get_Property(BSTR::from(name).as_ptr(), &mut returned)
                    .expect("get a bounded IronRDP integer setting");
            }
            assert_eq!(
                variant_i32_value(&returned, ptr::null_mut()).expect("integer setting"),
                expected
            );
        }

        for (name, expected) in [
            (ACTIVEX_CLIENT_DIRECTORY_PROPERTY, "C:\\IronRDP"),
            (ACTIVEX_IME_FILE_NAME_PROPERTY, "ime.dll"),
            (ACTIVEX_DIGITAL_PRODUCT_ID_PROPERTY, "product-id"),
        ] {
            let mut configured = variant_bstr(expected.to_owned());
            unsafe {
                extended
                    .put_Property(BSTR::from(name).as_ptr(), &mut configured)
                    .expect("configure an IronRDP string setting");
            }
            free_owned_bstr_variant(&mut configured);
            let mut returned = VARIANT::default();
            unsafe {
                extended
                    .get_Property(BSTR::from(name).as_ptr(), &mut returned)
                    .expect("get an IronRDP string setting");
            }
            assert_eq!(variant_bstr_value(&returned).expect("string setting"), expected);
            free_owned_bstr_variant(&mut returned);
        }

        let mut unsupported = VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                    vt: VT_I4,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: VARIANT_0_0_0 { lVal: 1 },
                }),
            },
        };
        let error = unsafe { extended.get_Property(BSTR::from("EnableMouseJiggler").as_ptr(), &mut unsupported) }
            .expect_err("unsupported extended setting");
        assert_eq!(error.code(), E_NOTIMPL);
        assert_eq!(variant_header(&unsupported).vt, VT_EMPTY);
    }

    #[test]
    fn raw_client_properties_use_automation_ownership_rules() {
        let control: IMsRdpClient10 = Control::new().into();
        let control = control.cast::<IMsTscAx>().expect("client supports IMsTscAx");
        let server = BSTR::from("rdp.example.test");

        unsafe {
            control.put_Server(server.as_ptr()).expect("set raw server property");
        }

        let mut server = ptr::dangling::<u16>();
        unsafe {
            control.get_Server(&mut server).expect("get raw server property");
        }
        let server = unsafe { BSTR::from_raw(server) };
        assert_eq!(String::try_from(&server).expect("valid BSTR"), "rdp.example.test");

        let control: IMsRdpClient10 = Control::new().into();
        let control = control.cast::<IMsRdpClient6>().expect("client supports IMsRdpClient6");
        let mut settings = ptr::NonNull::<c_void>::dangling().as_ptr();
        unsafe {
            control
                .get_AdvancedSettings7(&mut settings)
                .expect("advanced settings are available");
        }
        assert!(!settings.is_null());
        drop(unsafe { IUnknown::from_raw(settings) });
    }

    #[test]
    fn unsupported_raw_operations_preserve_mstsc_state_hresult_contracts() {
        let control: IMsRdpClient10 = Control::new().into();
        let client = control.cast::<IMsRdpClient9>().expect("client supports IMsRdpClient9");
        let error = unsafe { client.SyncSessionDisplaySettings() }
            .expect_err("display synchronization requires an active session");
        assert_eq!(error.code(), E_UNEXPECTED);
        let error = unsafe { client.SendRemoteAction(REMOTE_SESSION_ACTION_CHARMS) }
            .expect_err("remote actions require an active session");
        assert_eq!(error.code(), E_UNEXPECTED);
        let error = unsafe { client.SendRemoteAction(REMOTE_SESSION_ACTION_SNAP) }
            .expect_err("deprecated remote snap action is unavailable");
        assert_eq!(error.code(), E_NOTIMPL);

        let preferred = control
            .cast::<IMsRdpPreferredRedirectionInfo>()
            .expect("client supports preferred redirection settings");
        let error = unsafe { preferred.put_UseRedirectionServerName(VARIANT_TRUE.0) }
            .expect_err("redirection server names are not implemented");
        assert_eq!(error.code(), E_NOTIMPL);
        unsafe {
            preferred
                .put_UseRedirectionServerName(VARIANT_FALSE.0)
                .expect("disabling an unavailable redirection feature is accepted");
        }
        let mut use_redirection_server_name = VARIANT_TRUE.0;
        unsafe {
            preferred
                .get_UseRedirectionServerName(&mut use_redirection_server_name)
                .expect("query disabled redirection state");
        }
        assert_eq!(use_redirection_server_name, VARIANT_FALSE.0);

        let control = control.cast::<IMsTscAx>().expect("client supports IMsTscAx");
        let empty_channels = ptr::null();
        let error = unsafe { control.CreateVirtualChannels(empty_channels) }
            .expect_err("empty virtual-channel registration is invalid");
        assert_eq!(error.code(), E_INVALIDARG);
    }

    #[test]
    fn static_virtual_channel_contract_validates_and_retains_options() {
        let control: IMsRdpClient10 = Control::new().into();
        let ax = control.cast::<IMsTscAx>().expect("client supports IMsTscAx");
        let client = control.cast::<IMsRdpClient>().expect("client supports IMsRdpClient");
        let channels = BSTR::from("alpha,bravo");
        unsafe {
            ax.CreateVirtualChannels(channels.as_ptr())
                .expect("register static channels");
        }

        let alpha = BSTR::from("alpha");
        let options = i32::from_ne_bytes(ChannelOptions::COMPRESS.bits().to_ne_bytes());
        unsafe {
            client
                .SetVirtualChannelOptions(alpha.as_ptr(), options)
                .expect("retain static channel options");
        }
        let mut returned_options = 0;
        unsafe {
            client
                .GetVirtualChannelOptions(alpha.as_ptr(), &mut returned_options)
                .expect("get static channel options");
        }
        assert_eq!(returned_options, options);

        let error = unsafe { ax.CreateVirtualChannels(BSTR::from("alpha").as_ptr()) }
            .expect_err("duplicate static channels are invalid");
        assert_eq!(error.code(), E_INVALIDARG);
        let error = unsafe { ax.CreateVirtualChannels(BSTR::from("charlie,charlie").as_ptr()) }
            .expect_err("duplicate channels in one registration are invalid");
        assert_eq!(error.code(), E_INVALIDARG);
        let error = unsafe { ax.CreateVirtualChannels(BSTR::from("drdynvc").as_ptr()) }
            .expect_err("IronRDP-owned channels cannot be replaced");
        assert_eq!(error.code(), E_INVALIDARG);
        let error = unsafe { ax.CreateVirtualChannels(BSTR::from("charlie;delta").as_ptr()) }
            .expect_err("channel lists use the published comma delimiter");
        assert_eq!(error.code(), E_INVALIDARG);

        let unknown = BSTR::from("missing");
        let error = unsafe { client.SendOnVirtualChannel(unknown.as_ptr(), ptr::null()) }
            .expect_err("sending on an unregistered channel is invalid");
        assert_eq!(error.code(), E_INVALIDARG);
    }

    #[test]
    fn static_channel_data_preserves_latin1_bstr_code_units() {
        let source = BSTR::from("\0\u{7f}\u{80}\u{ff}");
        let data = channel_data_from_bstr(source.as_ptr()).expect("Latin-1 channel data");
        assert_eq!(data, [0, 0x7f, 0x80, 0xff]);
        assert_eq!(channel_data_to_automation_string(&data), "\0\u{7f}\u{80}\u{ff}");

        let invalid = BSTR::from("\u{100}");
        let error = channel_data_from_bstr(invalid.as_ptr()).expect_err("non-Latin-1 data is invalid");
        assert_eq!(error.code(), E_INVALIDARG);
    }

    #[test]
    fn configured_remote_application_execute_requires_a_program() {
        assert!(
            configured_remote_application_execute(&RemoteApplicationConfiguration::default())
                .expect("disabled RemoteApp is valid")
                .is_none()
        );
        let error = configured_remote_application_execute(&RemoteApplicationConfiguration {
            enabled: true,
            program: String::new(),
            arguments: "--ignored".to_owned(),
        })
        .expect_err("enabled RemoteApp requires a program");
        assert_eq!(error.code(), E_INVALIDARG);
        assert_eq!(
            configured_remote_application_execute(&RemoteApplicationConfiguration {
                enabled: true,
                program: "calc.exe".to_owned(),
                arguments: "/server:example".to_owned(),
            })
            .expect("configured RemoteApp is valid"),
            Some(ExecutePdu {
                flags: 0,
                executable: "calc.exe".to_owned(),
                working_directory: String::new(),
                arguments: "/server:example".to_owned(),
            })
        );
    }

    #[test]
    fn legacy_remote_program_settings_do_not_enable_remoteapp() {
        let control = Control::new();
        {
            let mut compatibility = control.compatibility.borrow_mut();
            compatibility.remote_program_mode = true;
            compatibility.remote_application_program = "calc.exe".to_owned();
            compatibility.remote_application_args = "/server:example".to_owned();
        }
        assert!(
            configured_remote_application_execute(&control.remote_application.borrow())
                .expect("legacy settings do not configure the ActiveX RemoteApp route")
                .is_none()
        );
    }

    #[test]
    fn projected_rail_window_order_retains_incremental_fields() {
        let flags: u32 = 0x1100_0000
            | 0x0000_0002
            | 0x0000_0004
            | 0x0000_0008
            | 0x0000_0010
            | 0x0000_0400
            | 0x0000_0800
            | 0x0000_4000
            | 0x0000_8000
            | 0x0001_0000;
        let mut encoded = vec![0x2e, 0, 0];
        encoded.extend_from_slice(&flags.to_le_bytes());
        encoded.extend_from_slice(&42u32.to_le_bytes());
        encoded.extend_from_slice(&7u32.to_le_bytes());
        encoded.extend_from_slice(&0x00c4_0000u32.to_le_bytes());
        encoded.extend_from_slice(&0x0000_0100u32.to_le_bytes());
        encoded.push(5);
        encoded.extend_from_slice(&8u16.to_le_bytes());
        encoded.extend("Calc".encode_utf16().flat_map(u16::to_le_bytes));
        encoded.extend_from_slice(&110i32.to_le_bytes());
        encoded.extend_from_slice(&120i32.to_le_bytes());
        encoded.extend_from_slice(&300u32.to_le_bytes());
        encoded.extend_from_slice(&200u32.to_le_bytes());
        encoded.extend_from_slice(&100i32.to_le_bytes());
        encoded.extend_from_slice(&90i32.to_le_bytes());
        encoded.extend_from_slice(&10i32.to_le_bytes());
        encoded.extend_from_slice(&30i32.to_le_bytes());
        encoded.extend_from_slice(&320u32.to_le_bytes());
        encoded.extend_from_slice(&240u32.to_le_bytes());
        let order_size = u16::try_from(encoded.len()).expect("test order fits");
        encoded[1..3].copy_from_slice(&order_size.to_le_bytes());

        let order = parse_projected_rail_window_order(&encoded, flags).expect("parse validated window order");
        assert!(order.is_new);
        assert_eq!(order.window_id, 42);
        assert_eq!(order.owner_window_id, Some(Some(7)));
        assert_eq!(order.style, Some((0x00c4_0000, 0x0000_0100)));
        assert_eq!(order.show_state, Some(5));
        assert_eq!(order.title.as_deref(), Some("Calc"));
        assert_eq!(order.client_area_offset, Some((110, 120)));
        assert_eq!(order.client_area_size, Some((300, 200)));
        assert_eq!(order.window_offset, Some((100, 90)));
        assert_eq!(order.client_delta, Some((10, 30)));
        assert_eq!(order.window_size, Some((320, 240)));
    }

    #[test]
    fn projected_rail_geometry_and_content_use_distinct_server_fields() {
        let outer = projected_rail_geometry(ProjectedRailGeometry::INITIAL, Some((30, 30)), Some((420, 330)));
        assert_eq!(
            outer,
            ProjectedRailGeometry {
                x: 30,
                y: 30,
                width: 420,
                height: 330,
            }
        );
        assert_eq!(
            projected_rail_content(
                ProjectedRailContent::from_outer(outer),
                outer,
                Some((50, 60)),
                Some((400, 300)),
                Some((20, 30)),
            ),
            ProjectedRailContent {
                x: 50,
                y: 60,
                width: 400,
                height: 300,
            }
        );
        assert_eq!(
            projected_rail_content(
                ProjectedRailContent::from_outer(outer),
                outer,
                None,
                None,
                Some((20, 30))
            ),
            ProjectedRailContent {
                x: 50,
                y: 60,
                width: 400,
                height: 300,
            }
        );
    }

    #[test]
    fn projected_rail_desktop_synchronization_resets_windows() {
        assert!(resets_projected_rail_windows(0x0400_0001));
        assert!(resets_projected_rail_windows(0x0400_000a));
        assert!(!resets_projected_rail_windows(0x0400_0002));
    }

    #[test]
    fn projected_rail_close_is_server_directed() {
        assert!(matches!(
            rail_window_input_event(42, WM_CLOSE, WPARAM(0)),
            Some(RailInputEvent::SystemCommand(SystemCommandPdu {
                window_id: 42,
                command: SystemCommand::Close,
            }))
        ));
        assert!(rail_window_input_event(42, WM_COMMAND, WPARAM(0)).is_none());
    }

    #[test]
    fn projected_rail_suppresses_unsupported_system_commands() {
        for command in [SC_MOVE, SC_SIZE, SC_MINIMIZE, SC_MAXIMIZE, SC_RESTORE] {
            assert!(is_unsupported_projected_rail_system_command(WPARAM(command as usize)));
        }
        assert!(!is_unsupported_projected_rail_system_command(WPARAM(0xf060)));
    }

    #[test]
    fn windows_key_policy_preserves_a_previously_forwarded_release() {
        let scancode = Scancode::from_u8(true, 0x5b);
        let mut compatibility = CompatibilitySettings {
            enable_windows_key: false,
            ..CompatibilitySettings::default()
        };
        let mut input_database = InputDatabase::new();

        assert!(!should_forward_windows_key(
            &compatibility,
            false,
            &input_database,
            WM_KEYDOWN,
            scancode
        ));
        input_database.apply([Operation::KeyPressed(scancode)]);
        assert!(should_forward_windows_key(
            &compatibility,
            false,
            &input_database,
            WM_KEYUP,
            scancode
        ));

        compatibility.enable_windows_key = true;
        compatibility.keyboard_hook_mode = 1;
        assert!(should_forward_windows_key(
            &compatibility,
            false,
            &input_database,
            WM_KEYDOWN,
            scancode
        ));
        compatibility.keyboard_hook_mode = 2;
        assert!(!should_forward_windows_key(
            &compatibility,
            false,
            &input_database,
            WM_KEYDOWN,
            scancode
        ));
    }

    #[test]
    fn projected_rail_input_retries_release_and_close_when_the_queue_is_full() {
        let (input_sender, mut input_receiver) = RdpInputSender::channel(1);
        let input_database = Rc::new(RefCell::new(InputDatabase::new()));
        let mut manager = RailWindowManager::new(
            Rc::clone(&input_database),
            Rc::new(RefCell::new(CompatibilitySettings::default())),
            Rc::new(RefCell::new(None)),
            Rc::new(RefCell::new(None)),
        );
        manager.start(Some(input_sender.clone()));
        manager.apply_window_order(ProjectedRailWindowOrder {
            is_new: true,
            window_id: 42,
            owner_window_id: None,
            style: None,
            show_state: Some(0),
            title: None,
            client_area_offset: None,
            client_area_size: None,
            window_offset: None,
            client_delta: None,
            window_size: None,
        });
        let window = manager.windows.get(&42).expect("projected window");
        input_sender
            .try_send(RdpInputEvent::FastPath(Vec::new().into()))
            .expect("fill input queue");
        apply_projected_rail_input(
            &window._context,
            [Operation::KeyPressed(Scancode::from_u8(false, 0x1e))],
        );
        assert!(input_database.borrow_mut().release_all().is_empty());
        assert!(matches!(input_receiver.try_recv(), Ok(RdpInputEvent::FastPath(_))));

        apply_projected_rail_input(
            &window._context,
            [Operation::KeyPressed(Scancode::from_u8(false, 0x1e))],
        );
        release_projected_rail_input(window.hwnd, &window._context);
        assert!(window._context.release_pending.get());
        assert!(matches!(input_receiver.try_recv(), Ok(RdpInputEvent::FastPath(_))));
        unsafe {
            let _ = SendMessageW(
                window.hwnd,
                WM_TIMER,
                Some(WPARAM(PROJECTED_RAIL_INPUT_RETRY_TIMER_ID)),
                Some(LPARAM(0)),
            );
        }
        assert!(!window._context.release_pending.get());
        assert!(matches!(input_receiver.try_recv(), Ok(RdpInputEvent::FastPath(_))));
        assert!(input_database.borrow_mut().release_all().is_empty());

        input_sender
            .try_send(RdpInputEvent::FastPath(Vec::new().into()))
            .expect("refill input queue");
        unsafe {
            let _ = SendMessageW(window.hwnd, WM_CLOSE, Some(WPARAM(0)), Some(LPARAM(0)));
        }
        assert!(unsafe { IsWindow(Some(window.hwnd)) }.as_bool());
        assert!(matches!(input_receiver.try_recv(), Ok(RdpInputEvent::FastPath(_))));

        unsafe {
            let _ = SendMessageW(
                window.hwnd,
                WM_TIMER,
                Some(WPARAM(PROJECTED_RAIL_INPUT_RETRY_TIMER_ID)),
                Some(LPARAM(0)),
            );
        }
        assert!(unsafe { IsWindow(Some(window.hwnd)) }.as_bool());
        assert!(matches!(
            input_receiver.try_recv(),
            Ok(RdpInputEvent::Rail(RailInputEvent::SystemCommand(SystemCommandPdu {
                window_id: 42,
                command: SystemCommand::Close,
            })))
        ));
        manager.stop();
    }

    #[test]
    fn projected_rail_windows_follow_server_authoritative_lifecycle() {
        let (input_sender, mut input_receiver) = RdpInputSender::channel(16);
        let mut manager = RailWindowManager::new(
            Rc::new(RefCell::new(InputDatabase::new())),
            Rc::new(RefCell::new(CompatibilitySettings::default())),
            Rc::new(RefCell::new(None)),
            Rc::new(RefCell::new(None)),
        );
        manager.start(Some(input_sender));
        manager.apply_window_order(ProjectedRailWindowOrder {
            is_new: false,
            window_id: 41,
            owner_window_id: None,
            style: None,
            show_state: Some(0),
            title: Some("Ignored".to_owned()),
            client_area_offset: None,
            client_area_size: None,
            window_offset: Some((100, 120)),
            client_delta: None,
            window_size: Some((320, 240)),
        });
        assert!(!manager.windows.contains_key(&41));
        manager.apply_window_order(ProjectedRailWindowOrder {
            is_new: true,
            window_id: 42,
            owner_window_id: None,
            style: None,
            show_state: Some(0),
            title: Some("Original".to_owned()),
            client_area_offset: None,
            client_area_size: None,
            window_offset: Some((100, 120)),
            client_delta: None,
            window_size: Some((320, 240)),
        });
        let hwnd = manager.windows.get(&42).expect("projected window").hwnd;
        assert!(unsafe { IsWindow(Some(hwnd)) }.as_bool());

        manager.apply_window_order(ProjectedRailWindowOrder {
            is_new: false,
            window_id: 42,
            owner_window_id: None,
            style: None,
            show_state: Some(0),
            title: Some("Updated".to_owned()),
            client_area_offset: None,
            client_area_size: None,
            window_offset: Some((160, 180)),
            client_delta: None,
            window_size: Some((400, 300)),
        });
        let window = manager.windows.get(&42).expect("updated projected window");
        assert_eq!(
            window.geometry.get(),
            ProjectedRailGeometry {
                x: 160,
                y: 180,
                width: 400,
                height: 300,
            }
        );
        let mut title = [0u16; 32];
        let title_length = unsafe { GetWindowTextW(hwnd, &mut title) };
        assert_eq!(
            String::from_utf16(&title[..title_length as usize]).expect("valid projected title"),
            "Updated"
        );

        unsafe {
            let _ = SendMessageW(
                hwnd,
                WM_KEYDOWN,
                Some(WPARAM(u16::from(b'A') as usize)),
                Some(LPARAM(0x001e_0000)),
            );
        }
        assert!(matches!(input_receiver.try_recv(), Ok(RdpInputEvent::FastPath(_))));

        unsafe {
            let _ = SendMessageW(hwnd, WM_CLOSE, Some(WPARAM(0)), Some(LPARAM(0)));
        }
        assert!(unsafe { IsWindow(Some(hwnd)) }.as_bool());
        assert!(matches!(
            input_receiver.try_recv(),
            Ok(RdpInputEvent::Rail(RailInputEvent::SystemCommand(SystemCommandPdu {
                window_id: 42,
                command: SystemCommand::Close,
            })))
        ));

        manager.destroy_window(42);
        assert!(!unsafe { IsWindow(Some(hwnd)) }.as_bool());

        manager.apply_window_order(ProjectedRailWindowOrder {
            is_new: true,
            window_id: 43,
            owner_window_id: None,
            style: None,
            show_state: Some(0),
            title: None,
            client_area_offset: None,
            client_area_size: None,
            window_offset: None,
            client_delta: None,
            window_size: None,
        });
        let disconnected_hwnd = manager.windows.get(&43).expect("projected window").hwnd;
        manager.stop();
        assert!(!unsafe { IsWindow(Some(disconnected_hwnd)) }.as_bool());
        assert!(!manager.is_enabled());
    }

    #[test]
    fn projected_rail_windows_attach_when_their_owner_arrives() {
        let (input_sender, _) = RdpInputSender::channel(16);
        let mut manager = RailWindowManager::new(
            Rc::new(RefCell::new(InputDatabase::new())),
            Rc::new(RefCell::new(CompatibilitySettings::default())),
            Rc::new(RefCell::new(None)),
            Rc::new(RefCell::new(None)),
        );
        manager.start(Some(input_sender));
        manager.apply_window_order(ProjectedRailWindowOrder {
            is_new: true,
            window_id: 8,
            owner_window_id: Some(Some(7)),
            style: None,
            show_state: Some(0),
            title: None,
            client_area_offset: None,
            client_area_size: None,
            window_offset: None,
            client_delta: None,
            window_size: None,
        });
        let child = manager.windows.get(&8).expect("projected child window").hwnd;
        assert_eq!(unsafe { GetWindowLongPtrW(child, GWLP_HWNDPARENT) }, 0);

        manager.apply_window_order(ProjectedRailWindowOrder {
            is_new: true,
            window_id: 7,
            owner_window_id: None,
            style: None,
            show_state: Some(0),
            title: None,
            client_area_offset: None,
            client_area_size: None,
            window_offset: None,
            client_delta: None,
            window_size: None,
        });
        let owner = manager.windows.get(&7).expect("projected owner window").hwnd;
        assert_eq!(unsafe { GetWindowLongPtrW(child, GWLP_HWNDPARENT) }, owner.0 as isize);
        manager.stop();
    }

    #[test]
    fn typed_lifecycle_events_do_not_depend_on_framebuffer_arrival() {
        let control = Control::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let dispatch: IDispatch = LifecycleSink {
            seen: Arc::clone(&seen),
        }
        .into();
        control.sinks.borrow_mut().insert(1, EventSink { cookie: 1, dispatch });
        control.connection_generation.set(7);
        control.state.set(ConnectionState::Connecting);

        control
            .events
            .events
            .lock()
            .expect("event queue is available")
            .push(WorkerEvent::Image {
                generation: 7,
                buffer: vec![0],
                width: 1,
                height: 1,
            });
        control.dispatch_pending_events();
        assert_eq!(control.state.get(), ConnectionState::Connecting);
        assert!(!control.clipboard_state.connected.get());
        assert!(seen.lock().expect("lifecycle events are available").is_empty());

        control.events.events.lock().expect("event queue is available").extend([
            WorkerEvent::Connected { generation: 7 },
            WorkerEvent::LoginComplete { generation: 7 },
            WorkerEvent::LoginComplete { generation: 7 },
        ]);
        control.dispatch_pending_events();

        assert_eq!(control.state.get(), ConnectionState::Connected);
        assert!(control.clipboard_state.connected.get());
        assert_eq!(
            *seen.lock().expect("lifecycle events are available"),
            [DISPID_ON_CONNECTED, DISPID_ON_LOGIN_COMPLETE]
        );
    }

    #[test]
    fn static_channel_processor_queues_raw_received_data() {
        let (events, event_posted) = (Arc::new(WorkerEventQueue::new()), Arc::new(AtomicBool::new(false)));
        let mut channel = ActiveXStaticChannel {
            spec: ActiveXStaticChannelSpec {
                display_name: "alpha".to_owned(),
                channel_name: ChannelName::from_utf8("alpha").expect("valid channel name"),
                options: ChannelOptions::PRI_HIGH,
            },
            events: Arc::clone(&events),
            event_posted,
            dispatcher: 0,
            generation: 42,
        };

        channel.process(&[0, 1, 2]).expect("queue received channel data");
        let event = events
            .events
            .lock()
            .expect("event queue is available")
            .pop()
            .expect("queued event");
        assert!(matches!(
            event,
            WorkerEvent::StaticChannelData {
                generation: 42,
                channel_name,
                data,
            } if channel_name == "alpha" && data == [0, 1, 2]
        ));
    }

    #[test]
    fn retained_presentation_surface_tracks_complete_frame_snapshots() {
        let initial_pixels = [0x0011_2233, 0x0044_5566, 0x0077_8899, 0x00aa_bbcc];
        let initial_frame = Frame::new(&initial_pixels, 2, 2, 1).expect("complete frame");
        let mut surface =
            PresentationSurface::new(&initial_frame, &initial_pixels).expect("create presentation surface");

        assert!(surface.matches_frame(&initial_frame));
        assert_eq!(
            unsafe { slice::from_raw_parts(surface.pixels, initial_pixels.len()) },
            initial_pixels
        );

        let updated_pixels = [0x00cc_bbaa, 0x0099_8877, 0x0066_5544, 0x0033_2211];
        let updated_frame = Frame::new(&updated_pixels, 2, 2, 2).expect("complete replacement frame");
        surface.copy_from(&updated_frame, &updated_pixels);

        assert!(surface.matches_frame(&updated_frame));
        assert_eq!(
            unsafe { slice::from_raw_parts(surface.pixels, updated_pixels.len()) },
            updated_pixels
        );
    }

    #[test]
    fn presentation_backbuffer_matches_the_client_extent() {
        let backbuffer = PresentationBackbuffer::new(120, 80).expect("create presentation backbuffer");

        assert!(backbuffer.matches_extent(120, 80));
        assert!(!backbuffer.matches_extent(80, 120));
    }

    #[test]
    fn ole_clip_rect_is_relative_to_the_renderer_window() {
        assert_eq!(
            renderer_clip_region(
                RECT {
                    left: 40,
                    top: 50,
                    right: 240,
                    bottom: 250,
                },
                RECT {
                    left: 80,
                    top: 90,
                    right: 180,
                    bottom: 200,
                },
            ),
            RECT {
                left: 40,
                top: 40,
                right: 140,
                bottom: 150,
            }
        );
        assert_eq!(
            renderer_clip_region(
                RECT {
                    left: 40,
                    top: 50,
                    right: 240,
                    bottom: 250,
                },
                RECT {
                    left: 300,
                    top: 300,
                    right: 400,
                    bottom: 400,
                },
            ),
            RECT::default()
        );
    }

    #[test]
    fn worker_event_queue_coalesces_lossy_events() {
        let events = Arc::new(WorkerEventQueue::new());
        let event_posted = Arc::new(AtomicBool::new(true));
        let dispatcher = HWND(ptr::null_mut());

        assert!(queue_worker_event(
            &events,
            &event_posted,
            dispatcher,
            WorkerEvent::Image {
                generation: 7,
                buffer: vec![1],
                width: 1,
                height: 1,
            },
        ));
        assert!(queue_worker_event(
            &events,
            &event_posted,
            dispatcher,
            WorkerEvent::Image {
                generation: 7,
                buffer: vec![2],
                width: 1,
                height: 1,
            },
        ));
        {
            let queue = events.events.lock().expect("event queue is available");
            assert!(matches!(
                queue.as_slice(),
                [WorkerEvent::Image { generation: 7, buffer, .. }] if buffer == &[2]
            ));
        }

        events.events.lock().expect("event queue is available").clear();
        assert!(queue_worker_event(
            &events,
            &event_posted,
            dispatcher,
            WorkerEvent::Connected { generation: 7 },
        ));
        assert!(queue_worker_event(
            &events,
            &event_posted,
            dispatcher,
            WorkerEvent::Image {
                generation: 7,
                buffer: vec![3],
                width: 1,
                height: 1,
            },
        ));
        {
            let queue = events.events.lock().expect("event queue is available");
            assert!(matches!(
                queue.as_slice(),
                [WorkerEvent::Connected { generation: 7 }, WorkerEvent::Image { buffer, .. }] if buffer == &[3]
            ));
        }

        events.events.lock().expect("event queue is available").clear();
        for index in 0..MAX_PENDING_WORKER_EVENTS {
            assert!(queue_worker_event(
                &events,
                &event_posted,
                dispatcher,
                WorkerEvent::StaticChannelData {
                    generation: 7,
                    channel_name: "alpha".to_owned(),
                    data: vec![u8::try_from(index).expect("queue capacity fits in u8")],
                },
            ));
        }
        assert!(!queue_worker_event(
            &events,
            &event_posted,
            dispatcher,
            WorkerEvent::StaticChannelData {
                generation: 7,
                channel_name: "alpha".to_owned(),
                data: vec![0],
            },
        ));
        assert!(!queue_worker_event(
            &events,
            &event_posted,
            dispatcher,
            WorkerEvent::Image {
                generation: 7,
                buffer: vec![9],
                width: 1,
                height: 1,
            },
        ));
        assert!(
            events
                .events
                .lock()
                .expect("event queue is available")
                .iter()
                .all(|event| matches!(event, WorkerEvent::StaticChannelData { .. }))
        );
    }

    #[test]
    fn worker_event_queue_preserves_auto_reconnect_decisions() {
        let events = Arc::new(WorkerEventQueue::new());
        let event_posted = Arc::new(AtomicBool::new(true));
        let dispatcher = HWND(ptr::null_mut());
        let (first_sender, mut first_receiver) = oneshot::channel();
        let (second_sender, mut second_receiver) = oneshot::channel();

        for (attempt, response) in [(1, first_sender), (2, second_sender)] {
            assert!(queue_worker_event(
                &events,
                &event_posted,
                dispatcher,
                WorkerEvent::AutoReconnecting {
                    generation: 7,
                    disconnect_reason: 0,
                    attempt,
                    maximum_attempts: 2,
                    response,
                },
            ));
        }
        assert_eq!(
            events
                .events
                .lock()
                .expect("event queue is available")
                .iter()
                .filter(|event| matches!(event, WorkerEvent::AutoReconnecting { .. }))
                .count(),
            2
        );
        assert!(first_receiver.try_recv().is_err());
        assert!(second_receiver.try_recv().is_err());

        events.events.lock().expect("event queue is available").clear();
        for index in 0..MAX_PENDING_WORKER_EVENTS {
            assert!(queue_worker_event(
                &events,
                &event_posted,
                dispatcher,
                WorkerEvent::StaticChannelData {
                    generation: 7,
                    channel_name: "alpha".to_owned(),
                    data: vec![u8::try_from(index).expect("queue capacity fits in u8")],
                },
            ));
        }
        let (sender, mut receiver) = oneshot::channel();
        assert!(queue_worker_event(
            &events,
            &event_posted,
            dispatcher,
            WorkerEvent::AutoReconnecting {
                generation: 7,
                disconnect_reason: 0,
                attempt: 1,
                maximum_attempts: 1,
                response: sender,
            },
        ));
        assert!(receiver.try_recv().is_err());
        assert!(
            events
                .events
                .lock()
                .expect("event queue is available")
                .iter()
                .any(|event| matches!(event, WorkerEvent::AutoReconnecting { .. }))
        );
    }

    #[test]
    fn worker_event_queue_rejects_auto_reconnect_when_dispatch_fails() {
        let events = Arc::new(WorkerEventQueue::new());
        let event_posted = Arc::new(AtomicBool::new(false));
        let (sender, mut receiver) = oneshot::channel();

        assert!(!queue_worker_event(
            &events,
            &event_posted,
            HWND(ptr::dangling_mut()),
            WorkerEvent::AutoReconnecting {
                generation: 7,
                disconnect_reason: 0,
                attempt: 1,
                maximum_attempts: 1,
                response: sender,
            },
        ));
        assert!(matches!(receiver.try_recv(), Err(oneshot::error::TryRecvError::Closed)));
        assert!(events.events.lock().expect("event queue is available").is_empty());
        assert!(!event_posted.load(Ordering::Acquire));
    }

    #[test]
    fn worker_event_queue_waits_for_rail_window_orders() {
        let events = Arc::new(WorkerEventQueue::new());
        let event_posted = Arc::new(AtomicBool::new(true));
        let dispatcher = HWND(ptr::null_mut());
        {
            let mut queue = events.events.lock().expect("event queue is available");
            for index in 0..MAX_PENDING_WORKER_EVENTS {
                queue.push(WorkerEvent::StaticChannelData {
                    generation: 7,
                    channel_name: "alpha".to_owned(),
                    data: vec![u8::try_from(index).expect("queue capacity fits in u8")],
                });
            }
        }

        let producer_events = Arc::clone(&events);
        let producer_posted = Arc::clone(&event_posted);
        let dispatcher_raw = dispatcher.0 as isize;
        let (completed_sender, completed_receiver) = std_mpsc::channel();
        let producer = std::thread::spawn(move || {
            let queued = queue_worker_event(
                &producer_events,
                &producer_posted,
                HWND(dispatcher_raw as *mut c_void),
                WorkerEvent::RailWindowingOrders {
                    generation: 7,
                    data: vec![1, 2, 3],
                },
            );
            completed_sender.send(queued).expect("report queued order");
        });

        assert!(matches!(
            completed_receiver.try_recv(),
            Err(std_mpsc::TryRecvError::Empty)
        ));
        assert_eq!(events.take().len(), MAX_PENDING_WORKER_EVENTS);
        assert!(
            completed_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("RAIL event is queued after draining")
        );
        producer.join().expect("RAIL event producer completes");
        assert!(matches!(
            events.take().as_slice(),
            [WorkerEvent::RailWindowingOrders {
                generation: 7,
                data,
            }] if data == &[1, 2, 3]
        ));
    }

    #[test]
    fn static_channel_processor_fails_when_the_host_event_queue_is_full() {
        let events = Arc::new(WorkerEventQueue::new());
        let event_posted = Arc::new(AtomicBool::new(true));
        let dispatcher = HWND(ptr::null_mut());
        for index in 0..MAX_PENDING_WORKER_EVENTS {
            assert!(queue_worker_event(
                &events,
                &event_posted,
                dispatcher,
                WorkerEvent::StaticChannelData {
                    generation: 42,
                    channel_name: "alpha".to_owned(),
                    data: vec![u8::try_from(index).expect("queue capacity fits in u8")],
                },
            ));
        }
        let mut channel = ActiveXStaticChannel {
            spec: ActiveXStaticChannelSpec {
                display_name: "alpha".to_owned(),
                channel_name: ChannelName::from_utf8("alpha").expect("valid channel name"),
                options: ChannelOptions::PRI_HIGH,
            },
            events,
            event_posted,
            dispatcher: 0,
            generation: 42,
        };

        assert!(channel.process(&[0, 1, 2]).is_err());
    }

    #[test]
    fn local_channel_event_bstrs_are_released_after_dispatch() {
        let mut value = variant_bstr("channel data".to_owned());
        assert_eq!(variant_header(&value).vt, VT_BSTR);
        free_owned_bstr_variant(&mut value);
        assert_eq!(variant_header(&value).vt, VT_EMPTY);
    }

    #[test]
    fn static_channel_events_use_reversed_automation_bstr_arguments() {
        let control = Control::new();
        let seen = Arc::new(Mutex::new(None));
        let dispatch: IDispatch = ChannelDataSink {
            seen: Arc::clone(&seen),
        }
        .into();
        control.sinks.borrow_mut().insert(1, EventSink { cookie: 1, dispatch });

        control.fire_channel_received_data("alpha", &[0, 0xff]);

        assert_eq!(
            seen.lock().expect("event sink state").as_ref(),
            Some(&("alpha".to_owned(), "\0\u{ff}".to_owned()))
        );
    }

    #[test]
    fn drive_catalog_preserves_selection_and_defaults_only_new_volumes() {
        let mut catalog = DriveCatalog::from_roots(vec![PathBuf::from(r"C:\"), PathBuf::from(r"D:\")], false);
        catalog.entries[0].redirection_state.set(true);

        catalog.rescan_from_roots(
            vec![PathBuf::from(r"C:\"), PathBuf::from(r"D:\"), PathBuf::from(r"E:\")],
            true,
        );

        assert_eq!(catalog.selected_drive_names(), vec!["C:".to_owned(), "E:".to_owned()]);
        assert!(!catalog.entries[1].redirection_state.get());

        catalog.rescan_from_roots(vec![PathBuf::from(r"D:\"), PathBuf::from(r"E:\")], false);
        catalog.rescan_from_roots(
            vec![PathBuf::from(r"C:\"), PathBuf::from(r"D:\"), PathBuf::from(r"E:\")],
            false,
        );
        assert_eq!(catalog.selected_drive_names(), vec!["C:".to_owned(), "E:".to_owned()]);
    }

    #[test]
    fn drive_collection_exposes_selected_volume_snapshots() {
        let catalog = Rc::new(RefCell::new(DriveCatalog::from_roots(
            vec![PathBuf::from(r"C:\"), PathBuf::from(r"D:\")],
            false,
        )));
        let persistence_dirty = Rc::new(Cell::new(false));
        let settings = Rc::new(RefCell::new(CompatibilitySettings {
            drive_catalog: Rc::clone(&catalog),
            persistence_dirty: Some(Rc::clone(&persistence_dirty)),
            ..Default::default()
        }));
        let collection: IMsRdpDriveCollection = DriveCollection::new(Rc::clone(&catalog), Rc::clone(&settings)).into();

        let mut count = 0;
        unsafe { collection.get_DriveCount(&mut count) }.expect("get drive count");
        assert_eq!(count, 2);

        let mut drive = ptr::null_mut();
        unsafe { collection.get_DriveByIndex(0, &mut drive) }.expect("get first drive");
        let drive = unsafe { IMsRdpDrive::from_raw(drive) };

        let mut name = ptr::null();
        unsafe { drive.get_Name(&mut name) }.expect("get drive name");
        let name = unsafe { BSTR::from_raw(name) };
        assert_eq!(String::try_from(&name).expect("valid drive name"), "C:\\\0");

        let mut state = VARIANT_TRUE.0;
        unsafe { drive.get_RedirectionState(&mut state) }.expect("get initial redirection state");
        assert_eq!(state, VARIANT_FALSE.0);
        unsafe { drive.put_RedirectionState(VARIANT_TRUE.0) }.expect("select first drive");
        assert_eq!(catalog.borrow().selected_drive_names(), vec!["C:".to_owned()]);

        let snapshot = catalog.borrow().selected_drives().expect("snapshot selected drive");
        let factory =
            ironrdp_rdpdr_native::WindowsRdpdrBackendFactory::from_drives(snapshot).expect("create drive factory");
        assert_eq!(factory.initial_drives(), vec![(1, "C:".to_owned())]);

        settings.borrow_mut().connection_settings_sealed = true;
        assert_eq!(
            unsafe { drive.put_RedirectionState(VARIANT_FALSE.0) }
                .expect_err("connection snapshots seal drive selection")
                .code(),
            E_FAIL
        );

        let mut missing: *mut c_void = ptr::dangling_mut();
        assert_eq!(
            unsafe { collection.get_DriveByIndex(2, &mut missing) }
                .expect_err("out-of-range drive index is rejected")
                .code(),
            E_UNEXPECTED
        );
        assert_eq!(missing, ptr::dangling_mut());

        settings.borrow_mut().connection_settings_sealed = false;
        persistence_dirty.set(false);
        unsafe { collection.RescanDrives(VARIANT_TRUE.0) }.expect("rescan drives");
        assert!(persistence_dirty.get());
    }

    #[test]
    fn control_retains_its_drive_collection() {
        let control: IMsRdpClient10 = Control::new().into();
        let non_scriptable = control
            .cast::<IMsRdpClientNonScriptable3>()
            .expect("control supports the drive collection contract");

        let mut first = ptr::null_mut();
        let mut second = ptr::null_mut();
        unsafe { non_scriptable.get_DriveCollection(&mut first) }.expect("get first drive collection");
        unsafe { non_scriptable.get_DriveCollection(&mut second) }.expect("get second drive collection");
        assert_eq!(first, second);
        drop(unsafe { IMsRdpDriveCollection::from_raw(first) });
        drop(unsafe { IMsRdpDriveCollection::from_raw(second) });
    }
}
