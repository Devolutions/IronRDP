#![cfg(windows)]
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

mod adapter;
#[cfg(ironrdp_idd_link)]
mod iddcx;
mod monitor;
mod remote;
mod swapchain;
#[cfg(ironrdp_idd_link)]
mod wdf;

pub use adapter::IronRdpIddAdapter;
pub use monitor::IronRdpIddMonitor;
pub use remote::{handle_session_transition, set_display_config};
pub use swapchain::SwapChainProcessor;

use core::ffi::c_void;
use windows::Win32::Foundation::HINSTANCE;
use windows_core::BOOL;

pub type NTSTATUS = i32;

pub const STATUS_SUCCESS: NTSTATUS = 0;

pub const fn ntstatus_from_u32(value: u32) -> NTSTATUS {
    i32::from_ne_bytes(value.to_ne_bytes())
}

pub const fn ntstatus_to_u32(value: NTSTATUS) -> u32 {
    u32::from_ne_bytes(value.to_ne_bytes())
}

pub const STATUS_NOT_SUPPORTED: NTSTATUS = ntstatus_from_u32(0xC000_00BB);

// Opaque stand-ins for WDK types. The real UMDF/KMDF integration will replace these.
#[repr(C)]
pub struct DRIVER_OBJECT {
    _private: [u8; 0],
}

#[repr(C)]
pub struct UNICODE_STRING {
    _private: [u8; 0],
}

// Opaque IDDCX handles (IddCx types come from WDK headers / import libs).
pub type IDDCX_ADAPTER = *mut c_void;
pub type IDDCX_MONITOR = *mut c_void;
pub type IDDCX_SWAPCHAIN = *mut c_void;

#[unsafe(no_mangle)]
pub extern "system" fn DllMain(_: HINSTANCE, _: u32, _: *mut c_void) -> BOOL {
    BOOL(1) // TRUE
}

#[unsafe(no_mangle)]
#[allow(unused_variables)]
pub extern "system" fn DriverEntry(
    driver_object: *mut DRIVER_OBJECT,
    registry_path: *const UNICODE_STRING,
) -> NTSTATUS {
    #[cfg(ironrdp_idd_link)]
    {
        use core::mem::size_of;
        let config = wdf::WDF_DRIVER_CONFIG {
            Size: size_of::<wdf::WDF_DRIVER_CONFIG>() as u32,
            EvtDriverDeviceAdd: Some(adapter::device_add),
            EvtDriverUnload: None,
            DriverInitFlags: 0,
            // Pool tag "iIDD" — identifies driver allocations in WDK memory tools.
            DriverPoolTag: u32::from_ne_bytes(*b"iIDD"),
        };
        tracing::info!(
            config_size = config.Size,
            device_add_set = config.EvtDriverDeviceAdd.is_some(),
            "DriverEntry invoking WdfDriverCreate"
        );
        // SAFETY: driver_object and registry_path are valid pointers from the WDF kernel reflector.
        let status = unsafe { wdf::driver_create(driver_object, registry_path, &config) };
        if status < 0 {
            tracing::error!(
                status,
                status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
                "WdfDriverCreate failed"
            );
        } else {
            tracing::info!(
                status,
                status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
                "WdfDriverCreate succeeded"
            );
        }
        return status;
    }

    // Stub path for cargo check / non-WDK builds.
    let _ = (driver_object, registry_path);
    // Exercise the callback table to catch type mismatches at compile time.
    let _ = iddcx_callback_table();
    tracing::info!("ironrdp-idd DriverEntry called (stub — build with IRONRDP_IDD_LINK=1 for real driver)");
    STATUS_SUCCESS
}

#[derive(Clone, Copy)]
struct IddCxCallbackTable {
    adapter_init_finished:
        extern "system" fn(IDDCX_ADAPTER, *const adapter::IDARG_IN_ADAPTER_INIT_FINISHED) -> NTSTATUS,
    adapter_commit_modes: extern "system" fn(IDDCX_ADAPTER, *const adapter::IDARG_IN_COMMITMODES) -> NTSTATUS,

    parse_monitor_description: extern "system" fn(
        *const monitor::IDARG_IN_PARSEMONITORDESCRIPTION,
        *mut monitor::IDARG_OUT_PARSEMONITORDESCRIPTION,
    ) -> NTSTATUS,
    monitor_get_default_description_modes: extern "system" fn(
        IDDCX_MONITOR,
        *const monitor::IDARG_IN_GETDEFAULTDESCRIPTIONMODES,
        *mut monitor::IDARG_OUT_GETDEFAULTDESCRIPTIONMODES,
    ) -> NTSTATUS,
    monitor_query_target_modes: extern "system" fn(
        IDDCX_MONITOR,
        *const monitor::IDARG_IN_QUERYTARGETMODES,
        *mut monitor::IDARG_OUT_QUERYTARGETMODES,
    ) -> NTSTATUS,
    monitor_assign_swapchain: extern "system" fn(IDDCX_MONITOR, *const monitor::IDARG_IN_SETSWAPCHAIN) -> NTSTATUS,
    monitor_unassign_swapchain: extern "system" fn(IDDCX_MONITOR) -> NTSTATUS,
}

fn iddcx_callback_table() -> IddCxCallbackTable {
    IddCxCallbackTable {
        adapter_init_finished: adapter::adapter_init_finished,
        adapter_commit_modes: adapter::adapter_commit_modes,
        parse_monitor_description: monitor::parse_monitor_description,
        monitor_get_default_description_modes: monitor::monitor_get_default_description_modes,
        monitor_query_target_modes: monitor::monitor_query_target_modes,
        monitor_assign_swapchain: monitor::monitor_assign_swapchain,
        monitor_unassign_swapchain: monitor::monitor_unassign_swapchain,
    }
}
