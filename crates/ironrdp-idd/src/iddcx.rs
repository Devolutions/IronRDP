use crate::wdf::{WDFDEVICE, WDFDEVICE_INIT, WDF_OBJECT_ATTRIBUTES};
use crate::{IDDCX_ADAPTER, IDDCX_MONITOR, IDDCX_SWAPCHAIN, NTSTATUS};
use core::ffi::c_void;
use core::mem::size_of;
use windows::Win32::Foundation::LUID;
use windows_core::HRESULT;

#[repr(C)]
pub(crate) struct IDD_DRIVER_GLOBALS {
    _private: [u8; 0],
}

type PFN_IDD_CX = unsafe extern "system" fn();

// Matches IddCx 1.2 (`IddCx0102`) headers.
const IDD_FUNCTION_TABLE_NUM_ENTRIES: usize = 23;

unsafe extern "C" {
    static mut IddDriverGlobals: *mut IDD_DRIVER_GLOBALS;
    static mut IddFunctions: [PFN_IDD_CX; IDD_FUNCTION_TABLE_NUM_ENTRIES];
}

const IDDCX_SWAPCHAIN_SET_DEVICE_TABLE_INDEX: usize = 10;
const IDDCX_SWAPCHAIN_RELEASE_AND_ACQUIRE_BUFFER_TABLE_INDEX: usize = 11;
const IDDCX_SWAPCHAIN_FINISHED_PROCESSING_FRAME_TABLE_INDEX: usize = 14;

const IDDCX_MONITOR_CREATE_TABLE_INDEX: usize = 3;
const IDDCX_MONITOR_ARRIVAL_TABLE_INDEX: usize = 4;
const IDDCX_MONITOR_DEPARTURE_TABLE_INDEX: usize = 5;

#[repr(C)]
pub(crate) struct IDDCX_MONITOR_DESCRIPTION {
    pub(crate) Size: u32,
    pub(crate) Type: u32,
    pub(crate) DataSize: u32,
    pub(crate) pData: *mut c_void,
}

const _: () = {
    assert!(
        size_of::<IDDCX_MONITOR_DESCRIPTION>() == 24,
        "IDDCX_MONITOR_DESCRIPTION size mismatch"
    );
};

#[repr(C)]
pub(crate) struct IDDCX_MONITOR_INFO {
    pub(crate) Size: u32,
    pub(crate) MonitorType: u32,
    pub(crate) ConnectorIndex: u32,
    pub(crate) _pad: u32,
    pub(crate) MonitorDescription: IDDCX_MONITOR_DESCRIPTION,
    pub(crate) MonitorContainerId: windows_core::GUID,
}

const _: () = {
    assert!(
        size_of::<IDDCX_MONITOR_INFO>() == 56,
        "IDDCX_MONITOR_INFO size mismatch"
    );
};

#[repr(C)]
pub(crate) struct IDARG_IN_MONITORCREATE {
    pub(crate) ObjectAttributes: *const WDF_OBJECT_ATTRIBUTES,
    pub(crate) pMonitorInfo: *mut IDDCX_MONITOR_INFO,
}

const _: () = {
    assert!(
        size_of::<IDARG_IN_MONITORCREATE>() == 16,
        "IDARG_IN_MONITORCREATE size mismatch"
    );
};

#[repr(C)]
pub(crate) struct IDARG_OUT_MONITORCREATE {
    pub(crate) MonitorObject: IDDCX_MONITOR,
}

const _: () = {
    assert!(
        size_of::<IDARG_OUT_MONITORCREATE>() == 8,
        "IDARG_OUT_MONITORCREATE size mismatch"
    );
};

#[repr(C)]
pub(crate) struct IDARG_OUT_MONITORARRIVAL {
    pub(crate) OsAdapterLuid: LUID,
    pub(crate) OsTargetId: u32,
}

#[repr(C)]
pub(crate) struct IDARG_IN_SWAPCHAINSETDEVICE {
    pub(crate) pDevice: *mut c_void,
}

#[repr(C)]
pub(crate) struct IDDCX_METADATA {
    pub(crate) Size: u32,
    pub(crate) PresentationFrameNumber: u32,
    pub(crate) DirtyRectCount: u32,
    pub(crate) MoveRegionCount: u32,
    pub(crate) HwProtectedSurface: i32,
    pub(crate) PresentDisplayQPCTime: u64,
    pub(crate) pSurface: *mut c_void,
}

#[repr(C)]
pub(crate) struct IDARG_OUT_RELEASEANDACQUIREBUFFER {
    pub(crate) MetaData: IDDCX_METADATA,
}

type PFN_IDDCX_SWAPCHAIN_SET_DEVICE =
    unsafe extern "system" fn(*mut IDD_DRIVER_GLOBALS, IDDCX_SWAPCHAIN, *const IDARG_IN_SWAPCHAINSETDEVICE) -> HRESULT;

type PFN_IDDCX_SWAPCHAIN_RELEASE_AND_ACQUIRE_BUFFER = unsafe extern "system" fn(
    *mut IDD_DRIVER_GLOBALS,
    IDDCX_SWAPCHAIN,
    *mut IDARG_OUT_RELEASEANDACQUIREBUFFER,
) -> HRESULT;

type PFN_IDDCX_SWAPCHAIN_FINISHED_PROCESSING_FRAME =
    unsafe extern "system" fn(*mut IDD_DRIVER_GLOBALS, IDDCX_SWAPCHAIN) -> HRESULT;

type PFN_IDDCX_MONITOR_CREATE = unsafe extern "system" fn(
    *mut IDD_DRIVER_GLOBALS,
    IDDCX_ADAPTER,
    *const IDARG_IN_MONITORCREATE,
    *mut IDARG_OUT_MONITORCREATE,
) -> NTSTATUS;

type PFN_IDDCX_MONITOR_ARRIVAL =
    unsafe extern "system" fn(*mut IDD_DRIVER_GLOBALS, IDDCX_MONITOR, *mut IDARG_OUT_MONITORARRIVAL) -> NTSTATUS;

type PFN_IDDCX_MONITOR_DEPARTURE = unsafe extern "system" fn(*mut IDD_DRIVER_GLOBALS, IDDCX_MONITOR) -> NTSTATUS;

pub(crate) unsafe fn swapchain_set_device(swapchain: IDDCX_SWAPCHAIN, dxgi_device: *mut c_void) -> HRESULT {
    let in_args = IDARG_IN_SWAPCHAINSETDEVICE { pDevice: dxgi_device };

    // SAFETY: read from a mutable static.
    let raw = unsafe { IddFunctions[IDDCX_SWAPCHAIN_SET_DEVICE_TABLE_INDEX] };
    // SAFETY: function pointer table uses a generic PFN type; we cast to the typed signature.
    let func: PFN_IDDCX_SWAPCHAIN_SET_DEVICE =
        unsafe { core::mem::transmute::<PFN_IDD_CX, PFN_IDDCX_SWAPCHAIN_SET_DEVICE>(raw) };
    // SAFETY: read from a mutable static.
    let globals = unsafe { IddDriverGlobals };
    // SAFETY: calls into the IddCx function table.
    unsafe { func(globals, swapchain, &in_args) }
}

pub(crate) unsafe fn swapchain_release_and_acquire_buffer(
    swapchain: IDDCX_SWAPCHAIN,
    out_args: &mut IDARG_OUT_RELEASEANDACQUIREBUFFER,
) -> HRESULT {
    let meta_size = match u32::try_from(size_of::<IDDCX_METADATA>()) {
        Ok(value) => value,
        Err(_) => return HRESULT(-2147467259), // E_FAIL
    };
    out_args.MetaData.Size = meta_size;

    // SAFETY: read from a mutable static.
    let raw = unsafe { IddFunctions[IDDCX_SWAPCHAIN_RELEASE_AND_ACQUIRE_BUFFER_TABLE_INDEX] };
    // SAFETY: function pointer table uses a generic PFN type; we cast to the typed signature.
    let func: PFN_IDDCX_SWAPCHAIN_RELEASE_AND_ACQUIRE_BUFFER =
        unsafe { core::mem::transmute::<PFN_IDD_CX, PFN_IDDCX_SWAPCHAIN_RELEASE_AND_ACQUIRE_BUFFER>(raw) };
    // SAFETY: read from a mutable static.
    let globals = unsafe { IddDriverGlobals };
    // SAFETY: calls into the IddCx function table.
    unsafe { func(globals, swapchain, out_args) }
}

pub(crate) unsafe fn swapchain_finished_processing_frame(swapchain: IDDCX_SWAPCHAIN) -> HRESULT {
    // SAFETY: read from a mutable static.
    let raw = unsafe { IddFunctions[IDDCX_SWAPCHAIN_FINISHED_PROCESSING_FRAME_TABLE_INDEX] };
    // SAFETY: function pointer table uses a generic PFN type; we cast to the typed signature.
    let func: PFN_IDDCX_SWAPCHAIN_FINISHED_PROCESSING_FRAME =
        unsafe { core::mem::transmute::<PFN_IDD_CX, PFN_IDDCX_SWAPCHAIN_FINISHED_PROCESSING_FRAME>(raw) };
    // SAFETY: read from a mutable static.
    let globals = unsafe { IddDriverGlobals };
    // SAFETY: calls into the IddCx function table.
    unsafe { func(globals, swapchain) }
}

pub(crate) unsafe fn monitor_create(
    adapter: IDDCX_ADAPTER,
    in_args: *const IDARG_IN_MONITORCREATE,
    out_args: *mut IDARG_OUT_MONITORCREATE,
) -> NTSTATUS {
    // SAFETY: read from a mutable static.
    let raw = unsafe { IddFunctions[IDDCX_MONITOR_CREATE_TABLE_INDEX] };
    // SAFETY: function pointer table uses a generic PFN type; we cast to the typed signature.
    let func: PFN_IDDCX_MONITOR_CREATE = unsafe { core::mem::transmute::<PFN_IDD_CX, PFN_IDDCX_MONITOR_CREATE>(raw) };
    // SAFETY: read from a mutable static.
    let globals = unsafe { IddDriverGlobals };
    // SAFETY: calls into the IddCx function table.
    unsafe { func(globals, adapter, in_args, out_args) }
}

pub(crate) unsafe fn monitor_arrival(monitor: IDDCX_MONITOR, out_args: *mut IDARG_OUT_MONITORARRIVAL) -> NTSTATUS {
    // SAFETY: read from a mutable static.
    let raw = unsafe { IddFunctions[IDDCX_MONITOR_ARRIVAL_TABLE_INDEX] };
    // SAFETY: function pointer table uses a generic PFN type; we cast to the typed signature.
    let func: PFN_IDDCX_MONITOR_ARRIVAL = unsafe { core::mem::transmute::<PFN_IDD_CX, PFN_IDDCX_MONITOR_ARRIVAL>(raw) };
    // SAFETY: read from a mutable static.
    let globals = unsafe { IddDriverGlobals };
    // SAFETY: calls into the IddCx function table.
    unsafe { func(globals, monitor, out_args) }
}

pub(crate) unsafe fn monitor_departure(monitor: IDDCX_MONITOR) -> NTSTATUS {
    // SAFETY: read from a mutable static.
    let raw = unsafe { IddFunctions[IDDCX_MONITOR_DEPARTURE_TABLE_INDEX] };
    // SAFETY: function pointer table uses a generic PFN type; we cast to the typed signature.
    let func: PFN_IDDCX_MONITOR_DEPARTURE =
        unsafe { core::mem::transmute::<PFN_IDD_CX, PFN_IDDCX_MONITOR_DEPARTURE>(raw) };
    // SAFETY: read from a mutable static.
    let globals = unsafe { IddDriverGlobals };
    // SAFETY: calls into the IddCx function table.
    unsafe { func(globals, monitor) }
}

// ────────────────────── Device init / adapter init dispatch ──────────────────────────────────

/// `IddCxDeviceInitConfigTableIndex` from `IddCxFuncEnum.h` (IddCx 1.2 / `IddCx0102`).
const IDDCX_DEVICE_INIT_CONFIG_TABLE_INDEX: usize = 0;
/// `IddCxDeviceInitializeTableIndex` from `IddCxFuncEnum.h` (IddCx 1.2 / `IddCx0102`).
const IDDCX_DEVICE_INITIALIZE_TABLE_INDEX: usize = 1;
/// `IddCxAdapterInitAsyncTableIndex` from `IddCxFuncEnum.h` (IddCx 1.2 / `IddCx0102`).
const IDDCX_ADAPTER_INIT_ASYNC_TABLE_INDEX: usize = 2;

type PFN_IDDCX_DEVICE_INIT_CONFIG =
    unsafe extern "system" fn(*mut IDD_DRIVER_GLOBALS, *mut WDFDEVICE_INIT, *const IDD_CX_CLIENT_CONFIG) -> NTSTATUS;

type PFN_IDDCX_DEVICE_INITIALIZE = unsafe extern "system" fn(*mut IDD_DRIVER_GLOBALS, WDFDEVICE) -> NTSTATUS;

type PFN_IDDCX_ADAPTER_INIT_ASYNC = unsafe extern "system" fn(
    *mut IDD_DRIVER_GLOBALS,
    *const IDARG_IN_ADAPTER_INIT,
    *mut IDARG_OUT_ADAPTER_INIT,
) -> NTSTATUS;

// ────────────────────────────── IddCx init structs ───────────────────────────────────────────

/// `IDD_CX_CLIENT_CONFIG` — passed to `IddCxDeviceInitConfig`.
///
/// Layout (x64, 160 bytes):
/// ```text
/// offset   0 | Size: ULONG             (4 bytes)
/// offset   4 | [4-byte padding]
/// offset   8 | EvtIddCxDeviceIoControl (8 bytes)
/// ...
/// offset 152 | EvtIddCxMonitorOPMDestroyProtectedOutput (8 bytes)
/// total: 160 bytes
/// ```
#[repr(C)]
pub(crate) struct IDD_CX_CLIENT_CONFIG {
    /// Must be set to `size_of::<IDD_CX_CLIENT_CONFIG>()`.
    pub Size: u32,
    // 4-byte implicit padding for fn-pointer alignment.
    pub EvtIddCxDeviceIoControl: Option<unsafe extern "system" fn()>,
    pub EvtIddCxParseMonitorDescription: Option<unsafe extern "system" fn()>,
    pub EvtIddCxAdapterInitFinished: Option<unsafe extern "system" fn()>,
    pub EvtIddCxAdapterCommitModes: Option<unsafe extern "system" fn()>,
    pub EvtIddCxMonitorGetDefaultDescriptionModes: Option<unsafe extern "system" fn()>,
    pub EvtIddCxMonitorQueryTargetModes: Option<unsafe extern "system" fn()>,
    pub EvtIddCxMonitorAssignSwapChain: Option<unsafe extern "system" fn()>,
    pub EvtIddCxMonitorUnassignSwapChain: Option<unsafe extern "system" fn()>,
    pub EvtIddCxMonitorI2CTransmit: Option<unsafe extern "system" fn()>,
    pub EvtIddCxMonitorI2CReceive: Option<unsafe extern "system" fn()>,
    pub EvtIddCxMonitorSetGammaRamp: Option<unsafe extern "system" fn()>,
    pub EvtIddCxMonitorOPMGetCertificateSize: Option<unsafe extern "system" fn()>,
    pub EvtIddCxMonitorOPMGetCertificate: Option<unsafe extern "system" fn()>,
    pub EvtIddCxMonitorOPMCreateProtectedOutput: Option<unsafe extern "system" fn()>,
    pub EvtIddCxMonitorOPMGetRandomNumber: Option<unsafe extern "system" fn()>,
    pub EvtIddCxMonitorOPMSetSigningKeyAndSequenceNumbers: Option<unsafe extern "system" fn()>,
    pub EvtIddCxMonitorOPMGetInformation: Option<unsafe extern "system" fn()>,
    pub EvtIddCxMonitorOPMConfigureProtectedOutput: Option<unsafe extern "system" fn()>,
    pub EvtIddCxMonitorOPMDestroyProtectedOutput: Option<unsafe extern "system" fn()>,
}

const _: () = {
    assert!(
        size_of::<IDD_CX_CLIENT_CONFIG>() == 160,
        "IDD_CX_CLIENT_CONFIG size mismatch"
    );
};

/// `IDDCX_ENDPOINT_VERSION` — version info for endpoint hardware/firmware.
///
/// Layout (24 bytes):
/// `Size(4) + MajorVer(4) + MinorVer(4) + Build(4) + SKU(8)`
#[repr(C)]
pub(crate) struct IDDCX_ENDPOINT_VERSION {
    pub Size: u32,
    pub MajorVer: u32,
    pub MinorVer: u32,
    pub Build: u32,
    pub SKU: u64,
}

const _: () = {
    assert!(
        size_of::<IDDCX_ENDPOINT_VERSION>() == 24,
        "IDDCX_ENDPOINT_VERSION size mismatch"
    );
};

/// `IDDCX_ENDPOINT_DIAGNOSTIC_INFO` — adapter endpoint metadata for telemetry.
///
/// Layout (x64, 56 bytes):
/// ```text
/// offset  0 | Size: UINT                  (4 bytes)
/// offset  4 | TransmissionType: UINT       (4 bytes)
/// offset  8 | pEndPointFriendlyName: PCWSTR (8 bytes, may be NULL)
/// offset 16 | pEndPointModelName: PCWSTR   (8 bytes, must be non-empty)
/// offset 24 | pEndPointManufacturerName    (8 bytes, must be non-empty)
/// offset 32 | pHardwareVersion: *IDDCX_ENDPOINT_VERSION (8 bytes)
/// offset 40 | pFirmwareVersion: *IDDCX_ENDPOINT_VERSION (8 bytes)
/// offset 48 | GammaSupport: UINT           (4 bytes)
/// offset 52 | [4-byte trailing padding]
/// total: 56 bytes
/// ```
#[repr(C)]
pub(crate) struct IDDCX_ENDPOINT_DIAGNOSTIC_INFO {
    pub Size: u32,
    /// `IDDCX_TRANSMISSION_TYPE_NETWORK_OTHER = 0x9`
    pub TransmissionType: u32,
    /// Optional friendly name (may be NULL).
    pub pEndPointFriendlyName: *const u16,
    /// Required non-empty model name string (UTF-16 null-terminated).
    pub pEndPointModelName: *const u16,
    /// Required non-empty manufacturer name string (UTF-16 null-terminated).
    pub pEndPointManufacturerName: *const u16,
    pub pHardwareVersion: *const IDDCX_ENDPOINT_VERSION,
    pub pFirmwareVersion: *const IDDCX_ENDPOINT_VERSION,
    /// `IDDCX_FEATURE_IMPLEMENTATION_NONE = 1`
    pub GammaSupport: u32,
    // 4 bytes trailing padding (struct alignment = 8).
    pub(crate) _pad: u32,
}

const _: () = {
    assert!(
        size_of::<IDDCX_ENDPOINT_DIAGNOSTIC_INFO>() == 56,
        "IDDCX_ENDPOINT_DIAGNOSTIC_INFO size mismatch"
    );
};

/// `IDDCX_ADAPTER_CAPS` — adapter capabilities passed to `IddCxAdapterInitAsync`.
///
/// Layout (x64, 88 bytes):
/// ```text
/// offset  0 | Size: UINT                        (4)
/// offset  4 | Flags: UINT                       (4)
/// offset  8 | MaxDisplayPipelineRate: UINT64    (8)
/// offset 16 | MaxMonitorsSupported: UINT        (4)
/// offset 20 | [4-byte padding]
/// offset 24 | EndPointDiagnostics (56 bytes)
/// offset 80 | StaticDesktopReencodeFrameCount   (4)
/// offset 84 | [4-byte trailing padding]
/// total: 88 bytes
/// ```
#[repr(C)]
pub(crate) struct IDDCX_ADAPTER_CAPS {
    pub Size: u32,
    /// `IDDCX_ADAPTER_FLAGS_REMOTE_SESSION_DRIVER(4) | IDDCX_ADAPTER_FLAGS_USE_SMALLEST_MODE(1)`
    pub Flags: u32,
    pub MaxDisplayPipelineRate: u64,
    pub MaxMonitorsSupported: u32,
    // 4-byte padding for IDDCX_ENDPOINT_DIAGNOSTIC_INFO alignment.
    pub(crate) _pad: u32,
    pub EndPointDiagnostics: IDDCX_ENDPOINT_DIAGNOSTIC_INFO,
    pub StaticDesktopReencodeFrameCount: u32,
    // 4 bytes trailing padding (struct alignment = 8).
    pub(crate) _pad2: u32,
}

const _: () = {
    assert!(
        size_of::<IDDCX_ADAPTER_CAPS>() == 88,
        "IDDCX_ADAPTER_CAPS size mismatch"
    );
};

/// `IDARG_IN_ADAPTER_INIT` — input to `IddCxAdapterInitAsync`.
///
/// Layout (24 bytes): WdfDevice(8) + pCaps(8) + ObjectAttributes(8)
#[repr(C)]
pub(crate) struct IDARG_IN_ADAPTER_INIT {
    pub WdfDevice: WDFDEVICE,
    pub pCaps: *mut IDDCX_ADAPTER_CAPS,
    pub ObjectAttributes: *const WDF_OBJECT_ATTRIBUTES,
}

const _: () = {
    assert!(
        size_of::<IDARG_IN_ADAPTER_INIT>() == 24,
        "IDARG_IN_ADAPTER_INIT size mismatch"
    );
};

/// `IDARG_OUT_ADAPTER_INIT` — output from `IddCxAdapterInitAsync`.
///
/// Layout (8 bytes): AdapterObject(*mut c_void)
#[repr(C)]
pub(crate) struct IDARG_OUT_ADAPTER_INIT {
    pub AdapterObject: *mut c_void,
}

const _: () = {
    assert!(
        size_of::<IDARG_OUT_ADAPTER_INIT>() == 8,
        "IDARG_OUT_ADAPTER_INIT size mismatch"
    );
};

// UTF-16 null-terminated string constants for endpoint diagnostics.
// "IronRDP IDD" (11 chars + null = 12 elements)
pub(crate) static ENDPOINT_MODEL_NAME_UTF16: [u16; 12] = [73, 114, 111, 110, 82, 68, 80, 32, 73, 68, 68, 0];
// "Devolutions" (11 chars + null = 12 elements)
pub(crate) static ENDPOINT_MANUFACTURER_UTF16: [u16; 12] = [68, 101, 118, 111, 108, 117, 116, 105, 111, 110, 115, 0];

/// IddCx version binding: the driver exports this symbol so `iddcxstub.lib` can verify version
/// compatibility. Value is `IDDCX_VERSION_MINOR` = 2 for IddCx 1.2 (`IddCx0102`).
#[unsafe(no_mangle)]
pub static IddMinimumVersionRequired: u32 = 2;

/// Calls `IddCxDeviceInitConfig` through the IddCx 1.2 dispatch table.
///
/// Must be called from `EvtDriverDeviceAdd` before `WdfDeviceCreate`.
///
/// # Safety
/// `device_init` must be the `PWDFDEVICE_INIT` pointer received by `EvtDriverDeviceAdd`.
pub(crate) unsafe fn device_init_config(
    device_init: *mut WDFDEVICE_INIT,
    config: *const IDD_CX_CLIENT_CONFIG,
) -> NTSTATUS {
    // SAFETY: read from mutable static populated by the IddCx framework.
    let raw = unsafe { IddFunctions[IDDCX_DEVICE_INIT_CONFIG_TABLE_INDEX] };
    // SAFETY: table entry is a valid PFN_IDDCX_DEVICE_INIT_CONFIG.
    let func: PFN_IDDCX_DEVICE_INIT_CONFIG =
        unsafe { core::mem::transmute::<PFN_IDD_CX, PFN_IDDCX_DEVICE_INIT_CONFIG>(raw) };
    // SAFETY: populated by the IddCx framework on load.
    let globals = unsafe { IddDriverGlobals };
    // SAFETY: caller guarantees device_init and config are valid.
    unsafe { func(globals, device_init, config) }
}

/// Calls `IddCxDeviceInitialize` through the IddCx 1.2 dispatch table.
///
/// Must be called from `EvtDriverDeviceAdd` after `WdfDeviceCreate`.
///
/// # Safety
/// `device` must be a valid `WDFDEVICE` returned by `WdfDeviceCreate`.
pub(crate) unsafe fn device_initialize(device: WDFDEVICE) -> NTSTATUS {
    // SAFETY: read from mutable static populated by the IddCx framework.
    let raw = unsafe { IddFunctions[IDDCX_DEVICE_INITIALIZE_TABLE_INDEX] };
    // SAFETY: table entry is a valid PFN_IDDCX_DEVICE_INITIALIZE.
    let func: PFN_IDDCX_DEVICE_INITIALIZE =
        unsafe { core::mem::transmute::<PFN_IDD_CX, PFN_IDDCX_DEVICE_INITIALIZE>(raw) };
    // SAFETY: populated by the IddCx framework on load.
    let globals = unsafe { IddDriverGlobals };
    // SAFETY: caller guarantees device is a valid WDFDEVICE handle.
    unsafe { func(globals, device) }
}

/// Calls `IddCxAdapterInitAsync` through the IddCx 1.2 dispatch table.
///
/// Must be called from `EvtDriverDeviceAdd` after `IddCxDeviceInitialize`.
///
/// # Safety
/// `in_args` and `out_args` must point to valid initialized structures.
pub(crate) unsafe fn adapter_init_async(
    in_args: *const IDARG_IN_ADAPTER_INIT,
    out_args: *mut IDARG_OUT_ADAPTER_INIT,
) -> NTSTATUS {
    // SAFETY: read from mutable static populated by the IddCx framework.
    let raw = unsafe { IddFunctions[IDDCX_ADAPTER_INIT_ASYNC_TABLE_INDEX] };
    // SAFETY: table entry is a valid PFN_IDDCX_ADAPTER_INIT_ASYNC.
    let func: PFN_IDDCX_ADAPTER_INIT_ASYNC =
        unsafe { core::mem::transmute::<PFN_IDD_CX, PFN_IDDCX_ADAPTER_INIT_ASYNC>(raw) };
    // SAFETY: populated by the IddCx framework on load.
    let globals = unsafe { IddDriverGlobals };
    // SAFETY: caller guarantees in_args and out_args are valid.
    unsafe { func(globals, in_args, out_args) }
}
