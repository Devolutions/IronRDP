/// Minimal WDF UMDF v2 type declarations and function dispatch wrappers.
///
/// UMDF v2 uses a versioned function dispatch table (`WdfFunctions_VVVVV`), exported from
/// `WdfDriverStubUm.lib`, to call WDF APIs. Each API is called by looking up its entry in
/// the table (indexed by the enum in `wdffuncenum.h`) and invoking it with `WdfDriverGlobals`
/// prepended to the argument list.
///
/// This module implements the same dispatch pattern as `iddcx.rs` does for IddCx.

use crate::{DRIVER_OBJECT, NTSTATUS, UNICODE_STRING};
use core::ffi::c_void;
use windows_core::GUID;

// ────────────────────────────── Opaque WDF handles ───────────────────────────────────────────

/// Opaque handle to a WDF driver object (WDFDRIVER).
pub type WDFDRIVER = *mut c_void;

/// Opaque handle to a WDF device object (WDFDEVICE).
pub type WDFDEVICE = *mut c_void;

/// Opaque WDFDEVICE_INIT structure — only ever used as `*mut WDFDEVICE_INIT`.
pub type WDFDEVICE_INIT = c_void;
pub type WDFREQUEST = *mut c_void;

pub type WDFFILEOBJECT = *mut c_void;


// ─────────────────────────── Opaque WDF globals ──────────────────────────────────────────────

/// Opaque WDF driver globals (PWDF_DRIVER_GLOBALS payload).
#[repr(C)]
pub struct WDF_DRIVER_GLOBALS {
    _private: [u8; 0],
}

/// Generic untyped WDF function table entry.
pub(crate) type PFN_WDF_FUNCTION = unsafe extern "system" fn();

// ───────────────────────────── Callback function types ───────────────────────────────────────

/// `EVT_WDF_DRIVER_DEVICE_ADD` — called by PnP to attach a device to the driver.
pub type PFN_WDF_DRIVER_DEVICE_ADD =
    unsafe extern "system" fn(driver: WDFDRIVER, device_init: *mut WDFDEVICE_INIT) -> NTSTATUS;

/// `WDF_POWER_DEVICE_STATE` enum underlying type.
pub type WDF_POWER_DEVICE_STATE = u32;

/// `EVT_WDF_DEVICE_D0_ENTRY` callback type.
pub type PFN_WDF_DEVICE_D0_ENTRY =
    unsafe extern "system" fn(device: WDFDEVICE, previous_state: WDF_POWER_DEVICE_STATE) -> NTSTATUS;
pub type PFN_WDF_DEVICE_FILE_CREATE =
    unsafe extern "system" fn(device: WDFDEVICE, request: WDFREQUEST, file_object: WDFFILEOBJECT);


// ───────────────────────────── WDF structures ────────────────────────────────────────────────

/// `WDF_DRIVER_CONFIG` — passed to `WdfDriverCreate`.
///
/// Layout (x64 MSVC-compatible, 32 bytes):
/// ```text
/// offset  0 | Size: u32                     (4 bytes)
/// offset  4 | [implicit 4-byte padding]
/// offset  8 | EvtDriverDeviceAdd: Option<fn> (8 bytes)
/// offset 16 | EvtDriverUnload: Option<fn>    (8 bytes)
/// offset 24 | DriverInitFlags: u32           (4 bytes)
/// offset 28 | DriverPoolTag: u32             (4 bytes)
/// total: 32 bytes
/// ```
#[repr(C)]
pub struct WDF_DRIVER_CONFIG {
    /// Must be set to `size_of::<WDF_DRIVER_CONFIG>()`.
    pub Size: u32,
    // 4-byte implicit padding inserted by Rust #[repr(C)] for pointer alignment.
    /// Required — PnP calls this to add a device to the driver.
    pub EvtDriverDeviceAdd: Option<PFN_WDF_DRIVER_DEVICE_ADD>,
    /// Optional — called when driver is being unloaded.
    pub EvtDriverUnload: Option<unsafe extern "system" fn(WDFDRIVER)>,
    /// Driver initialization flags (0 for normal startup).
    pub DriverInitFlags: u32,
    /// Pool tag for driver allocations.
    pub DriverPoolTag: u32,
}

const _: () = {
    assert!(core::mem::size_of::<WDF_DRIVER_CONFIG>() == 32, "WDF_DRIVER_CONFIG size mismatch");
};

/// `WDF_OBJECT_ATTRIBUTES` — optional attributes passed to WDF object creation functions.
///
/// Layout (x64 MSVC-compatible, 56 bytes):
/// ```text
/// offset  0 | Size: u32                          (4 bytes)
/// offset  4 | [implicit 4-byte padding]
/// offset  8 | EvtCleanupCallback: Option<fn>     (8 bytes)
/// offset 16 | EvtDestroyCallback: Option<fn>     (8 bytes)
/// offset 24 | ExecutionLevel: u32                (4 bytes)
/// offset 28 | SynchronizationScope: u32          (4 bytes)
/// offset 32 | ParentObject: *mut c_void          (8 bytes)
/// offset 40 | ContextSizeOverride: usize         (8 bytes)
/// offset 48 | ContextTypeInfo: *const c_void     (8 bytes)
/// total: 56 bytes
/// ```
#[repr(C)]
pub struct WDF_OBJECT_ATTRIBUTES {
    /// Must be set to `size_of::<WDF_OBJECT_ATTRIBUTES>()`.
    pub Size: u32,
    // 4-byte implicit padding for pointer alignment.
    pub EvtCleanupCallback: Option<unsafe extern "system" fn(*mut c_void)>,
    pub EvtDestroyCallback: Option<unsafe extern "system" fn(*mut c_void)>,
    /// `WdfExecutionLevelInheritFromParent` = 1.
    pub ExecutionLevel: u32,
    /// `WdfSynchronizationScopeNone` = 3.
    pub SynchronizationScope: u32,
    pub ParentObject: *mut c_void,
    pub ContextSizeOverride: usize,
    pub ContextTypeInfo: *const c_void,
}

const _: () = {
    assert!(
        core::mem::size_of::<WDF_OBJECT_ATTRIBUTES>() == 56,
        "WDF_OBJECT_ATTRIBUTES size mismatch",
    );
};

impl WDF_OBJECT_ATTRIBUTES {
    /// Returns a zeroed `WDF_OBJECT_ATTRIBUTES` with `SynchronizationScope` set to
    /// `WdfSynchronizationScopeNone` (3), which is appropriate for IDD devices.
    pub const fn init_no_context() -> Self {
        WDF_OBJECT_ATTRIBUTES {
            Size: core::mem::size_of::<WDF_OBJECT_ATTRIBUTES>() as u32,
            EvtCleanupCallback: None,
            EvtDestroyCallback: None,
            ExecutionLevel: 1,       // WdfExecutionLevelInheritFromParent
            SynchronizationScope: 1, // WdfSynchronizationScopeInheritFromParent
            ParentObject: core::ptr::null_mut(),
            ContextSizeOverride: 0,
            ContextTypeInfo: core::ptr::null(),
        }
    }
}

pub type WDF_TRI_STATE = u32;
pub type WDF_FILEOBJECT_CLASS = u32;

#[repr(C)]
pub struct WDF_FILEOBJECT_CONFIG {
    pub Size: u32,
    pub EvtDeviceFileCreate: Option<PFN_WDF_DEVICE_FILE_CREATE>,
    pub EvtFileClose: Option<unsafe extern "system" fn(WDFFILEOBJECT)>,
    pub EvtFileCleanup: Option<unsafe extern "system" fn(WDFFILEOBJECT)>,
    pub AutoForwardCleanupClose: WDF_TRI_STATE,
    pub FileObjectClass: WDF_FILEOBJECT_CLASS,
}

impl WDF_FILEOBJECT_CONFIG {
    pub const fn init(evt_device_file_create: Option<PFN_WDF_DEVICE_FILE_CREATE>) -> Self {
        WDF_FILEOBJECT_CONFIG {
            Size: core::mem::size_of::<WDF_FILEOBJECT_CONFIG>() as u32,
            EvtDeviceFileCreate: evt_device_file_create,
            EvtFileClose: None,
            EvtFileCleanup: None,
            AutoForwardCleanupClose: 0,
            FileObjectClass: 4,
        }
    }
}

/// `WDF_PNPPOWER_EVENT_CALLBACKS` — device PnP/power callback table.
///
/// Layout (x64 UMDF 2.33, 144 bytes): `Size` + implicit padding + 17 callback pointers.
#[repr(C)]
pub struct WDF_PNPPOWER_EVENT_CALLBACKS {
    pub Size: u32,
    pub EvtDeviceD0Entry: Option<PFN_WDF_DEVICE_D0_ENTRY>,
    pub EvtDeviceD0EntryPostInterruptsEnabled: Option<unsafe extern "system" fn()>,
    pub EvtDeviceD0Exit: Option<unsafe extern "system" fn()>,
    pub EvtDeviceD0ExitPreInterruptsDisabled: Option<unsafe extern "system" fn()>,
    pub EvtDevicePrepareHardware: Option<unsafe extern "system" fn()>,
    pub EvtDeviceReleaseHardware: Option<unsafe extern "system" fn()>,
    pub EvtDeviceSelfManagedIoCleanup: Option<unsafe extern "system" fn()>,
    pub EvtDeviceSelfManagedIoFlush: Option<unsafe extern "system" fn()>,
    pub EvtDeviceSelfManagedIoInit: Option<unsafe extern "system" fn()>,
    pub EvtDeviceSelfManagedIoSuspend: Option<unsafe extern "system" fn()>,
    pub EvtDeviceSelfManagedIoRestart: Option<unsafe extern "system" fn()>,
    pub EvtDeviceSurpriseRemoval: Option<unsafe extern "system" fn()>,
    pub EvtDeviceQueryRemove: Option<unsafe extern "system" fn()>,
    pub EvtDeviceQueryStop: Option<unsafe extern "system" fn()>,
    pub EvtDeviceUsageNotification: Option<unsafe extern "system" fn()>,
    pub EvtDeviceRelationsQuery: Option<unsafe extern "system" fn()>,
    pub EvtDeviceUsageNotificationEx: Option<unsafe extern "system" fn()>,
}

const _: () = {
    assert!(
        core::mem::size_of::<WDF_PNPPOWER_EVENT_CALLBACKS>() == 144,
        "WDF_PNPPOWER_EVENT_CALLBACKS size mismatch",
    );
};

impl WDF_PNPPOWER_EVENT_CALLBACKS {
    pub const fn init() -> Self {
        WDF_PNPPOWER_EVENT_CALLBACKS {
            Size: core::mem::size_of::<WDF_PNPPOWER_EVENT_CALLBACKS>() as u32,
            EvtDeviceD0Entry: None,
            EvtDeviceD0EntryPostInterruptsEnabled: None,
            EvtDeviceD0Exit: None,
            EvtDeviceD0ExitPreInterruptsDisabled: None,
            EvtDevicePrepareHardware: None,
            EvtDeviceReleaseHardware: None,
            EvtDeviceSelfManagedIoCleanup: None,
            EvtDeviceSelfManagedIoFlush: None,
            EvtDeviceSelfManagedIoInit: None,
            EvtDeviceSelfManagedIoSuspend: None,
            EvtDeviceSelfManagedIoRestart: None,
            EvtDeviceSurpriseRemoval: None,
            EvtDeviceQueryRemove: None,
            EvtDeviceQueryStop: None,
            EvtDeviceUsageNotification: None,
            EvtDeviceRelationsQuery: None,
            EvtDeviceUsageNotificationEx: None,
        }
    }
}

// ───────────────────────────── WDF function dispatch ─────────────────────────────────────────

/// `PFN_WDFDRIVERCREATE` — internal typed function pointer for `WdfDriverCreate` dispatch.
type PFN_WDF_DRIVER_CREATE = unsafe extern "system" fn(
    globals: *mut WDF_DRIVER_GLOBALS,
    driver_object: *mut DRIVER_OBJECT,
    registry_path: *const UNICODE_STRING,
    driver_attributes: *const WDF_OBJECT_ATTRIBUTES,
    driver_config: *const WDF_DRIVER_CONFIG,
    driver: *mut WDFDRIVER,
) -> NTSTATUS;

/// `PFN_WDFDEVICECREATE` — internal typed function pointer for `WdfDeviceCreate` dispatch.
type PFN_WDF_DEVICE_CREATE = unsafe extern "system" fn(
    globals: *mut WDF_DRIVER_GLOBALS,
    device_init: *mut *mut WDFDEVICE_INIT,
    device_attributes: *const WDF_OBJECT_ATTRIBUTES,
    device: *mut WDFDEVICE,
) -> NTSTATUS;

type PFN_WDF_DEVICE_CREATE_SYMBOLIC_LINK = unsafe extern "system" fn(
    globals: *mut WDF_DRIVER_GLOBALS,
    device: WDFDEVICE,
    symbolic_link_name: *const UNICODE_STRING,
) -> NTSTATUS;

type PFN_WDF_DEVICE_CREATE_DEVICE_INTERFACE = unsafe extern "system" fn(
    globals: *mut WDF_DRIVER_GLOBALS,
    device: WDFDEVICE,
    interface_class_guid: *const GUID,
    reference_string: *const UNICODE_STRING,
) -> NTSTATUS;
type PFN_WDF_DEVICE_SET_DEVICE_INTERFACE_STATE = unsafe extern "system" fn(
    globals: *mut WDF_DRIVER_GLOBALS,
    device: WDFDEVICE,
    interface_class_guid: *const GUID,
    reference_string: *const UNICODE_STRING,
    is_interface_enabled: u8,
);
type PFN_WDF_DEVICE_INIT_SET_FILE_OBJECT_CONFIG = unsafe extern "system" fn(
    globals: *mut WDF_DRIVER_GLOBALS,
    device_init: *mut WDFDEVICE_INIT,
    file_object_config: *const WDF_FILEOBJECT_CONFIG,
    file_object_attributes: *const WDF_OBJECT_ATTRIBUTES,
);

type PFN_WDF_REQUEST_COMPLETE = unsafe extern "system" fn(
    globals: *mut WDF_DRIVER_GLOBALS,
    request: WDFREQUEST,
    status: NTSTATUS,
);

/// `PFN_WDFDEVICEINITSETPNPPOWEREVENTCALLBACKS` dispatch signature.
type PFN_WDF_DEVICE_INIT_SET_PNPPOWER_EVENT_CALLBACKS = unsafe extern "system" fn(
    globals: *mut WDF_DRIVER_GLOBALS,
    device_init: *mut WDFDEVICE_INIT,
    pnp_power_callbacks: *const WDF_PNPPOWER_EVENT_CALLBACKS,
);

/// `WdfRequestCompleteTableIndex` from `wdffuncenum.h` (UMDF 2.33).
const WDF_REQUEST_COMPLETE_TABLE_INDEX: usize = 163;

/// `WdfDriverCreateTableIndex` from `wdffuncenum.h` (UMDF 2.33).
const WDF_DRIVER_CREATE_TABLE_INDEX: usize = 57;

/// `WdfDeviceInitSetFileObjectConfigTableIndex` from `wdffuncenum.h` (UMDF 2.33).
const WDF_DEVICE_INIT_SET_FILE_OBJECT_CONFIG_TABLE_INDEX: usize = 23;

/// `WdfDeviceCreateTableIndex` from `wdffuncenum.h` (UMDF 2.33).
const WDF_DEVICE_CREATE_TABLE_INDEX: usize = 25;

/// `WdfDeviceCreateDeviceInterfaceTableIndex` from `wdffuncenum.h` (UMDF 2.33).
const WDF_DEVICE_CREATE_DEVICE_INTERFACE_TABLE_INDEX: usize = 27;

/// `WdfDeviceSetDeviceInterfaceStateTableIndex` from `wdffuncenum.h` (UMDF 2.33).
const WDF_DEVICE_SET_DEVICE_INTERFACE_STATE_TABLE_INDEX: usize = 28;

/// `WdfDeviceCreateSymbolicLinkTableIndex` from `wdffuncenum.h` (UMDF 2.33).
const WDF_DEVICE_CREATE_SYMBOLIC_LINK_TABLE_INDEX: usize = 30;

/// `WdfDeviceInitSetPnpPowerEventCallbacksTableIndex` from `wdffuncenum.h` (UMDF 2.33).
const WDF_DEVICE_INIT_SET_PNPPOWER_EVENT_CALLBACKS_TABLE_INDEX: usize = 19;

// External symbols exported from `WdfDriverStubUm.lib`.
// The WDK header `wdf.h` defines `#define WdfFunctions WdfFunctions_02033`;
// we use the mangled symbol name directly to match the linker symbol.
unsafe extern "C" {
    /// Pointer to the WDF 2.33 function dispatch table.
    #[link_name = "WdfFunctions_02033"]
    static WDF_FUNCTION_TABLE: *const Option<PFN_WDF_FUNCTION>;

    /// WDF driver globals pointer — set by the WDF framework on load.
    pub static mut WdfDriverGlobals: *mut WDF_DRIVER_GLOBALS;
}

/// Calls `WdfDriverCreate` through the WDF 2.33 dispatch table.
///
/// This must be called from `DriverEntry` — the WDF framework uses it to register the
/// driver's `EvtDriverDeviceAdd` callback.
///
/// # Safety
/// The caller must provide valid driver object and registry path pointers (as passed to
/// `DriverEntry`).
pub unsafe fn driver_create(
    driver_object: *mut DRIVER_OBJECT,
    registry_path: *const UNICODE_STRING,
    config: *const WDF_DRIVER_CONFIG,
) -> NTSTATUS {
    // SAFETY: WDF_FUNCTION_TABLE is populated by the WDF framework before DriverEntry is called.
    let entry = unsafe { WDF_FUNCTION_TABLE.add(WDF_DRIVER_CREATE_TABLE_INDEX).read() };
    let func: PFN_WDF_DRIVER_CREATE =
        // SAFETY: all WDF function table entries share the same pointer representation.
        unsafe { core::mem::transmute::<Option<PFN_WDF_FUNCTION>, PFN_WDF_DRIVER_CREATE>(entry) };
    // SAFETY: populated by the WDF framework on load.
    let globals = unsafe { WdfDriverGlobals };
    // SAFETY: all arguments are valid (caller guarantees).
    unsafe {
        func(
            globals,
            driver_object,
            registry_path,
            core::ptr::null(), // WDF_NO_OBJECT_ATTRIBUTES
            config,
            core::ptr::null_mut(), // WDF_NO_HANDLE
        )
    }
}

/// Calls `WdfDeviceCreate` through the WDF 2.33 dispatch table.
///
/// Must be called from `EvtDriverDeviceAdd` after `IddCxDeviceInitConfig`.
///
/// # Safety
/// `device_init` must point to a valid `PWDFDEVICE_INIT` obtained from the device-add callback.
pub unsafe fn device_create(
    device_init: *mut *mut WDFDEVICE_INIT,
    attributes: *const WDF_OBJECT_ATTRIBUTES,
) -> (NTSTATUS, WDFDEVICE) {
    let mut device: WDFDEVICE = core::ptr::null_mut();
    // SAFETY: WDF_FUNCTION_TABLE populated by framework.
    let entry = unsafe { WDF_FUNCTION_TABLE.add(WDF_DEVICE_CREATE_TABLE_INDEX).read() };
    let func: PFN_WDF_DEVICE_CREATE =
        // SAFETY: all WDF function table entries share the same pointer representation.
        unsafe { core::mem::transmute::<Option<PFN_WDF_FUNCTION>, PFN_WDF_DEVICE_CREATE>(entry) };
    // SAFETY: populated by the WDF framework on load.
    let globals = unsafe { WdfDriverGlobals };
    // SAFETY: caller guarantees device_init and attributes are valid.
    let status = unsafe { func(globals, device_init, attributes, &mut device) };
    (status, device)
}

pub unsafe fn device_create_device_interface(
    device: WDFDEVICE,
    interface_class_guid: *const GUID,
    reference_string: *const UNICODE_STRING,
) -> NTSTATUS {
    // SAFETY: WDF_FUNCTION_TABLE populated by framework.
    let entry = unsafe { WDF_FUNCTION_TABLE.add(WDF_DEVICE_CREATE_DEVICE_INTERFACE_TABLE_INDEX).read() };
    let func: PFN_WDF_DEVICE_CREATE_DEVICE_INTERFACE =
        // SAFETY: all WDF function table entries share the same pointer representation.
        unsafe { core::mem::transmute::<Option<PFN_WDF_FUNCTION>, PFN_WDF_DEVICE_CREATE_DEVICE_INTERFACE>(entry) };
    // SAFETY: populated by the WDF framework on load.
    let globals = unsafe { WdfDriverGlobals };
    // SAFETY: caller guarantees device and GUID pointer validity.
    unsafe { func(globals, device, interface_class_guid, reference_string) }
}

pub unsafe fn device_set_device_interface_state(
    device: WDFDEVICE,
    interface_class_guid: *const GUID,
    reference_string: *const UNICODE_STRING,
    is_interface_enabled: bool,
) {
    let entry = unsafe { WDF_FUNCTION_TABLE.add(WDF_DEVICE_SET_DEVICE_INTERFACE_STATE_TABLE_INDEX).read() };
    let func: PFN_WDF_DEVICE_SET_DEVICE_INTERFACE_STATE = unsafe {
        core::mem::transmute::<Option<PFN_WDF_FUNCTION>, PFN_WDF_DEVICE_SET_DEVICE_INTERFACE_STATE>(entry)
    };
    let globals = unsafe { WdfDriverGlobals };
    unsafe { func(globals, device, interface_class_guid, reference_string, u8::from(is_interface_enabled)) };
}


pub unsafe fn device_create_symbolic_link(device: WDFDEVICE, symbolic_link_name: *const UNICODE_STRING) -> NTSTATUS {
    // SAFETY: WDF_FUNCTION_TABLE populated by framework.
    let entry = unsafe { WDF_FUNCTION_TABLE.add(WDF_DEVICE_CREATE_SYMBOLIC_LINK_TABLE_INDEX).read() };
    let func: PFN_WDF_DEVICE_CREATE_SYMBOLIC_LINK =
        unsafe { core::mem::transmute::<Option<PFN_WDF_FUNCTION>, PFN_WDF_DEVICE_CREATE_SYMBOLIC_LINK>(entry) };
    // SAFETY: populated by the WDF framework on load.
    let globals = unsafe { WdfDriverGlobals };
    // SAFETY: caller guarantees device and symbolic link pointer validity.
    unsafe { func(globals, device, symbolic_link_name) }
}

pub unsafe fn device_init_set_file_object_config(
    device_init: *mut WDFDEVICE_INIT,
    file_object_config: *const WDF_FILEOBJECT_CONFIG,
    file_object_attributes: *const WDF_OBJECT_ATTRIBUTES,
) {
    let entry = unsafe { WDF_FUNCTION_TABLE.add(WDF_DEVICE_INIT_SET_FILE_OBJECT_CONFIG_TABLE_INDEX).read() };
    let func: PFN_WDF_DEVICE_INIT_SET_FILE_OBJECT_CONFIG =
        unsafe { core::mem::transmute::<Option<PFN_WDF_FUNCTION>, PFN_WDF_DEVICE_INIT_SET_FILE_OBJECT_CONFIG>(entry) };
    let globals = unsafe { WdfDriverGlobals };
    unsafe { func(globals, device_init, file_object_config, file_object_attributes) };
}

pub unsafe fn request_complete(request: WDFREQUEST, status: NTSTATUS) {
    let entry = unsafe { WDF_FUNCTION_TABLE.add(WDF_REQUEST_COMPLETE_TABLE_INDEX).read() };
    let func: PFN_WDF_REQUEST_COMPLETE =
        unsafe { core::mem::transmute::<Option<PFN_WDF_FUNCTION>, PFN_WDF_REQUEST_COMPLETE>(entry) };
    let globals = unsafe { WdfDriverGlobals };
    unsafe { func(globals, request, status) };
}

/// Calls `WdfDeviceInitSetPnpPowerEventCallbacks` through the WDF 2.33 dispatch table.
///
/// Must be called from `EvtDriverDeviceAdd` before `WdfDeviceCreate`.
///
/// # Safety
/// `device_init` must be valid for the current add-device callback.
pub unsafe fn device_init_set_pnp_power_event_callbacks(
    device_init: *mut WDFDEVICE_INIT,
    callbacks: *const WDF_PNPPOWER_EVENT_CALLBACKS,
) {
    // SAFETY: WDF_FUNCTION_TABLE populated by framework.
    let entry = unsafe {
        WDF_FUNCTION_TABLE
            .add(WDF_DEVICE_INIT_SET_PNPPOWER_EVENT_CALLBACKS_TABLE_INDEX)
            .read()
    };
    let func: PFN_WDF_DEVICE_INIT_SET_PNPPOWER_EVENT_CALLBACKS =
        // SAFETY: all WDF function table entries share the same pointer representation.
        unsafe {
            core::mem::transmute::<Option<PFN_WDF_FUNCTION>, PFN_WDF_DEVICE_INIT_SET_PNPPOWER_EVENT_CALLBACKS>(entry)
        };
    // SAFETY: populated by the WDF framework on load.
    let globals = unsafe { WdfDriverGlobals };
    // SAFETY: caller guarantees pointers are valid.
    unsafe { func(globals, device_init, callbacks) };
}
