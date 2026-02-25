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

// ────────────────────────────── Opaque WDF handles ───────────────────────────────────────────

/// Opaque handle to a WDF driver object (WDFDRIVER).
pub type WDFDRIVER = *mut c_void;

/// Opaque handle to a WDF device object (WDFDEVICE).
pub type WDFDEVICE = *mut c_void;

/// Opaque WDFDEVICE_INIT structure — only ever used as `*mut WDFDEVICE_INIT`.
pub type WDFDEVICE_INIT = c_void;

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
            SynchronizationScope: 3, // WdfSynchronizationScopeNone
            ParentObject: core::ptr::null_mut(),
            ContextSizeOverride: 0,
            ContextTypeInfo: core::ptr::null(),
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

/// `WdfDriverCreateTableIndex` from `wdffuncenum.h` (UMDF 2.33).
const WDF_DRIVER_CREATE_TABLE_INDEX: usize = 57;

/// `WdfDeviceCreateTableIndex` from `wdffuncenum.h` (UMDF 2.33).
const WDF_DEVICE_CREATE_TABLE_INDEX: usize = 25;

/// WDF version binding required by `WdfDriverStubUm.lib`.
///
/// In UMDF 2.x headers this resolves to `UMDF_VERSION_MINOR`; for 2.33 this is `33`.
#[unsafe(no_mangle)]
pub static WdfMinimumVersionRequired: u32 = 33;

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
