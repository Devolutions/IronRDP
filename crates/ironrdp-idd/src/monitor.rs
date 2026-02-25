use crate::{
    ntstatus_to_u32, IDDCX_ADAPTER, IDDCX_MONITOR, IDDCX_SWAPCHAIN, NTSTATUS, STATUS_NOT_SUPPORTED, STATUS_SUCCESS,
};
use core::cmp::min;
use core::ffi::c_void;
use core::mem::size_of;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use windows::Win32::Foundation::{HANDLE, LUID};

#[cfg(ironrdp_idd_link)]
use crate::iddcx;
use crate::SwapChainProcessor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct MonitorKey(IDDCX_MONITOR);

// SAFETY: monitor handles are opaque identifiers provided by IddCx; they can be used as keys across threads.
unsafe impl Send for MonitorKey {}

static ACTIVE_SWAPCHAINS: OnceLock<Mutex<HashMap<MonitorKey, SwapChainProcessor>>> = OnceLock::new();

fn active_swapchains() -> &'static Mutex<HashMap<MonitorKey, SwapChainProcessor>> {
    ACTIVE_SWAPCHAINS.get_or_init(|| Mutex::new(HashMap::new()))
}

const NO_PREFERRED_MODE: u32 = 0xFFFF_FFFF;
const IDDCX_MONITOR_MODE_ORIGIN_MONITORDESCRIPTOR: u32 = 1;
const IDDCX_MONITOR_MODE_ORIGIN_DRIVER: u32 = 2;
const IDDCX_MONITOR_DESCRIPTION_TYPE_EDID: u32 = 1;
const DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INDIRECT_WIRED: u32 = 16;
const DISPLAYCONFIG_SCANLINE_ORDERING_PROGRESSIVE: u32 = 1;

const DEFAULT_MODE_WIDTH: u32 = 1920;
const DEFAULT_MODE_HEIGHT: u32 = 1080;
const DEFAULT_MODE_REFRESH_HZ: u32 = 60;

#[repr(C)]
#[derive(Clone, Copy)]
struct DISPLAYCONFIG_RATIONAL {
    Numerator: u32,
    Denominator: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DISPLAYCONFIG_2DREGION {
    cx: u32,
    cy: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DISPLAYCONFIG_VIDEO_SIGNAL_INFO {
    pixelRate: u64,
    hSyncFreq: DISPLAYCONFIG_RATIONAL,
    vSyncFreq: DISPLAYCONFIG_RATIONAL,
    activeSize: DISPLAYCONFIG_2DREGION,
    totalSize: DISPLAYCONFIG_2DREGION,
    AdditionalSignalInfo: u32,
    scanLineOrdering: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DISPLAYCONFIG_TARGET_MODE {
    targetVideoSignalInfo: DISPLAYCONFIG_VIDEO_SIGNAL_INFO,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct IDDCX_MONITOR_DESCRIPTION {
    Size: u32,
    Type: u32,
    DataSize: u32,
    pData: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct IDDCX_MONITOR_MODE {
    Size: u32,
    Origin: u32,
    MonitorVideoSignalInfo: DISPLAYCONFIG_VIDEO_SIGNAL_INFO,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct IDDCX_TARGET_MODE {
    Size: u32,
    TargetVideoSignalInfo: DISPLAYCONFIG_TARGET_MODE,
    RequiredBandwidth: u64,
}

#[repr(C)]
pub(crate) struct IDARG_IN_PARSEMONITORDESCRIPTION {
    MonitorDescription: IDDCX_MONITOR_DESCRIPTION,
    MonitorModeBufferInputCount: u32,
    pMonitorModes: *mut IDDCX_MONITOR_MODE,
}

#[repr(C)]
pub(crate) struct IDARG_OUT_PARSEMONITORDESCRIPTION {
    MonitorModeBufferOutputCount: u32,
    PreferredMonitorModeIdx: u32,
}

#[repr(C)]
pub(crate) struct IDARG_IN_GETDEFAULTDESCRIPTIONMODES {
    DefaultMonitorModeBufferInputCount: u32,
    pDefaultMonitorModes: *mut IDDCX_MONITOR_MODE,
}

#[repr(C)]
pub(crate) struct IDARG_OUT_GETDEFAULTDESCRIPTIONMODES {
    DefaultMonitorModeBufferOutputCount: u32,
    PreferredMonitorModeIdx: u32,
}

#[repr(C)]
pub(crate) struct IDARG_IN_QUERYTARGETMODES {
    MonitorDescription: IDDCX_MONITOR_DESCRIPTION,
    TargetModeBufferInputCount: u32,
    pTargetModes: *mut IDDCX_TARGET_MODE,
}

#[repr(C)]
pub(crate) struct IDARG_OUT_QUERYTARGETMODES {
    TargetModeBufferOutputCount: u32,
}

#[repr(C)]
pub(crate) struct IDARG_IN_SETSWAPCHAIN {
    pub(crate) hSwapChain: IDDCX_SWAPCHAIN,
    pub(crate) hNextSurfaceAvailable: HANDLE,
    pub(crate) RenderAdapterLuid: LUID,
}

fn additional_signal_info(video_standard: u16, v_sync_freq_divider: u8) -> u32 {
    u32::from(video_standard) | (u32::from(v_sync_freq_divider & 0x3F) << 16)
}

fn default_video_signal_info(v_sync_freq_divider: u8) -> DISPLAYCONFIG_VIDEO_SIGNAL_INFO {
    let total_width = DEFAULT_MODE_WIDTH.saturating_add(160);
    let total_height = DEFAULT_MODE_HEIGHT.saturating_add(45);
    let pixel_rate = u64::from(total_width)
        .saturating_mul(u64::from(total_height))
        .saturating_mul(u64::from(DEFAULT_MODE_REFRESH_HZ));

    DISPLAYCONFIG_VIDEO_SIGNAL_INFO {
        pixelRate: pixel_rate,
        hSyncFreq: DISPLAYCONFIG_RATIONAL {
            Numerator: total_height.saturating_mul(DEFAULT_MODE_REFRESH_HZ),
            Denominator: 1,
        },
        vSyncFreq: DISPLAYCONFIG_RATIONAL {
            Numerator: DEFAULT_MODE_REFRESH_HZ,
            Denominator: 1,
        },
        activeSize: DISPLAYCONFIG_2DREGION {
            cx: DEFAULT_MODE_WIDTH,
            cy: DEFAULT_MODE_HEIGHT,
        },
        totalSize: DISPLAYCONFIG_2DREGION {
            cx: total_width,
            cy: total_height,
        },
        AdditionalSignalInfo: additional_signal_info(0, v_sync_freq_divider),
        scanLineOrdering: DISPLAYCONFIG_SCANLINE_ORDERING_PROGRESSIVE,
    }
}

fn default_monitor_mode(origin: u32) -> IDDCX_MONITOR_MODE {
    IDDCX_MONITOR_MODE {
        Size: size_of::<IDDCX_MONITOR_MODE>() as u32,
        Origin: origin,
        // For monitor modes, vSyncFreqDivider must be zero.
        MonitorVideoSignalInfo: default_video_signal_info(0),
    }
}

fn default_target_mode() -> IDDCX_TARGET_MODE {
    let signal_info = default_video_signal_info(1);
    IDDCX_TARGET_MODE {
        Size: size_of::<IDDCX_TARGET_MODE>() as u32,
        TargetVideoSignalInfo: DISPLAYCONFIG_TARGET_MODE {
            targetVideoSignalInfo: signal_info,
        },
        RequiredBandwidth: signal_info.pixelRate,
    }
}

fn write_mode_list<T: Copy>(buffer_input_count: u32, buffer_ptr: *mut T, modes: &[T]) -> u32 {
    if buffer_ptr.is_null() || buffer_input_count == 0 {
        return modes.len() as u32;
    }

    let copy_count = min(modes.len(), buffer_input_count as usize);
    // SAFETY: `buffer_ptr` points to a writable output array provided by IddCx; `copy_count`
    // is bounded by both the source and destination lengths.
    unsafe {
        core::ptr::copy_nonoverlapping(modes.as_ptr(), buffer_ptr, copy_count);
    }

    copy_count as u32
}

#[cfg(ironrdp_idd_link)]
fn monitor_container_id(connector_idx: u32) -> windows_core::GUID {
    windows_core::GUID::from_u128(0x3124f0b9_8233_4a6f_9c48_3f779f8ca0a0u128 ^ u128::from(connector_idx))
}

#[derive(Debug, Clone, Copy)]
pub struct IronRdpIddMonitor {
    pub monitor: IDDCX_MONITOR,
    pub connector_index: u32,
}

impl IronRdpIddMonitor {
    pub fn create_and_arrive(adapter: IDDCX_ADAPTER, connector_idx: u32, edid: Option<&[u8]>) -> NTSTATUS {
        #[cfg(not(ironrdp_idd_link))]
        {
            let _ = (adapter, edid);
            tracing::info!(connector_idx, "IddCxMonitorArrival (stub)");
            return STATUS_NOT_SUPPORTED;
        }

        #[cfg(ironrdp_idd_link)]
        {
            let (description_type, description_size, description_ptr) = match edid {
                Some(bytes) if !bytes.is_empty() => (
                    IDDCX_MONITOR_DESCRIPTION_TYPE_EDID,
                    bytes.len() as u32,
                    bytes.as_ptr() as *mut c_void,
                ),
                _ => (0, 0, core::ptr::null_mut()),
            };

            let mut monitor_info = iddcx::IDDCX_MONITOR_INFO {
                Size: size_of::<iddcx::IDDCX_MONITOR_INFO>() as u32,
                MonitorType: DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INDIRECT_WIRED,
                ConnectorIndex: connector_idx,
                _pad: 0,
                MonitorDescription: iddcx::IDDCX_MONITOR_DESCRIPTION {
                    Size: size_of::<iddcx::IDDCX_MONITOR_DESCRIPTION>() as u32,
                    Type: description_type,
                    DataSize: description_size,
                    pData: description_ptr,
                },
                MonitorContainerId: monitor_container_id(connector_idx),
            };

            let in_args = iddcx::IDARG_IN_MONITORCREATE {
                ObjectAttributes: core::ptr::null(),
                pMonitorInfo: &mut monitor_info,
            };
            let mut out_create = iddcx::IDARG_OUT_MONITORCREATE {
                MonitorObject: core::ptr::null_mut(),
            };

            // SAFETY: all pointers in `in_args` and `out_create` are valid for the duration of the call.
            let create_status = unsafe { iddcx::monitor_create(adapter, &in_args, &mut out_create) };
            if create_status < 0 {
                tracing::error!(
                    status = create_status,
                    status_hex = format_args!("0x{:08X}", ntstatus_to_u32(create_status)),
                    connector_idx,
                    "IddCxMonitorCreate failed"
                );
                return create_status;
            }

            let mut out_arrival = iddcx::IDARG_OUT_MONITORARRIVAL {
                OsAdapterLuid: LUID {
                    LowPart: 0,
                    HighPart: 0,
                },
                OsTargetId: 0,
            };

            // SAFETY: `MonitorObject` is produced by a successful `IddCxMonitorCreate` call.
            let arrival_status = unsafe { iddcx::monitor_arrival(out_create.MonitorObject, &mut out_arrival) };
            if arrival_status < 0 {
                tracing::error!(
                    status = arrival_status,
                    status_hex = format_args!("0x{:08X}", ntstatus_to_u32(arrival_status)),
                    connector_idx,
                    "IddCxMonitorArrival failed"
                );
                // SAFETY: best-effort cleanup for a monitor object created above.
                let _ = unsafe { iddcx::monitor_departure(out_create.MonitorObject) };
                return arrival_status;
            }

            tracing::info!(
                connector_idx,
                os_target_id = out_arrival.OsTargetId,
                "IddCxMonitorCreate/Arrival succeeded"
            );
            STATUS_SUCCESS
        }
    }

    pub fn departure(&self) -> NTSTATUS {
        #[cfg(not(ironrdp_idd_link))]
        {
            tracing::info!(connector_idx = self.connector_index, "IddCxMonitorDeparture (stub)");
            return STATUS_NOT_SUPPORTED;
        }

        #[cfg(ironrdp_idd_link)]
        {
            // SAFETY: `self.monitor` is an IddCx monitor handle returned by `IddCxMonitorCreate`.
            let status = unsafe { iddcx::monitor_departure(self.monitor) };
            if status < 0 {
                tracing::warn!(
                    status,
                    status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
                    connector_idx = self.connector_index,
                    "IddCxMonitorDeparture failed"
                );
            }
            status
        }
    }
}

pub(crate) extern "system" fn parse_monitor_description(
    in_args: *const IDARG_IN_PARSEMONITORDESCRIPTION,
    out_args: *mut IDARG_OUT_PARSEMONITORDESCRIPTION,
) -> NTSTATUS {
    if in_args.is_null() || out_args.is_null() {
        tracing::warn!("EvtIddCxParseMonitorDescription called with null args");
        STATUS_NOT_SUPPORTED
    } else {
        // SAFETY: pointers are validated above and owned by IddCx for callback duration.
        let in_args = unsafe { &*in_args };
        // SAFETY: pointers are validated above and owned by IddCx for callback duration.
        let out_args = unsafe { &mut *out_args };

        let has_monitor_description =
            !in_args.MonitorDescription.pData.is_null() && in_args.MonitorDescription.DataSize != 0;

        if !has_monitor_description {
            out_args.MonitorModeBufferOutputCount = 0;
            out_args.PreferredMonitorModeIdx = NO_PREFERRED_MODE;
            tracing::info!("EvtIddCxParseMonitorDescription: no descriptor provided; returning zero descriptor modes");
            return STATUS_SUCCESS;
        }

        let modes = [default_monitor_mode(IDDCX_MONITOR_MODE_ORIGIN_MONITORDESCRIPTOR)];
        let output_count = write_mode_list(in_args.MonitorModeBufferInputCount, in_args.pMonitorModes, &modes);

        out_args.MonitorModeBufferOutputCount = output_count;
        out_args.PreferredMonitorModeIdx = if output_count > 0 { 0 } else { NO_PREFERRED_MODE };

        tracing::info!(
            monitor_mode_output_count = output_count,
            monitor_mode_input_capacity = in_args.MonitorModeBufferInputCount,
            "EvtIddCxParseMonitorDescription succeeded"
        );
        STATUS_SUCCESS
    }
}

pub(crate) extern "system" fn monitor_get_default_description_modes(
    monitor: IDDCX_MONITOR,
    in_args: *const IDARG_IN_GETDEFAULTDESCRIPTIONMODES,
    out_args: *mut IDARG_OUT_GETDEFAULTDESCRIPTIONMODES,
) -> NTSTATUS {
    let _ = monitor;

    if in_args.is_null() || out_args.is_null() {
        tracing::warn!("EvtIddCxMonitorGetDefaultDescriptionModes called with null args");
        return STATUS_NOT_SUPPORTED;
    }

    // SAFETY: pointers are validated above and owned by IddCx for callback duration.
    let in_args = unsafe { &*in_args };
    // SAFETY: pointers are validated above and owned by IddCx for callback duration.
    let out_args = unsafe { &mut *out_args };

    let modes = [default_monitor_mode(IDDCX_MONITOR_MODE_ORIGIN_DRIVER)];
    let output_count = write_mode_list(
        in_args.DefaultMonitorModeBufferInputCount,
        in_args.pDefaultMonitorModes,
        &modes,
    );

    out_args.DefaultMonitorModeBufferOutputCount = output_count;
    out_args.PreferredMonitorModeIdx = if output_count > 0 { 0 } else { NO_PREFERRED_MODE };

    tracing::info!(
        default_mode_output_count = output_count,
        default_mode_input_capacity = in_args.DefaultMonitorModeBufferInputCount,
        "EvtIddCxMonitorGetDefaultDescriptionModes succeeded"
    );
    STATUS_SUCCESS
}

pub(crate) extern "system" fn monitor_query_target_modes(
    monitor: IDDCX_MONITOR,
    in_args: *const IDARG_IN_QUERYTARGETMODES,
    out_args: *mut IDARG_OUT_QUERYTARGETMODES,
) -> NTSTATUS {
    let _ = monitor;

    if in_args.is_null() || out_args.is_null() {
        tracing::warn!("EvtIddCxMonitorQueryTargetModes called with null args");
        return STATUS_NOT_SUPPORTED;
    }

    // SAFETY: pointers are validated above and owned by IddCx for callback duration.
    let in_args = unsafe { &*in_args };
    // SAFETY: pointers are validated above and owned by IddCx for callback duration.
    let out_args = unsafe { &mut *out_args };

    let modes = [default_target_mode()];
    let output_count = write_mode_list(in_args.TargetModeBufferInputCount, in_args.pTargetModes, &modes);

    out_args.TargetModeBufferOutputCount = output_count;
    tracing::info!(
        target_mode_output_count = output_count,
        target_mode_input_capacity = in_args.TargetModeBufferInputCount,
        "EvtIddCxMonitorQueryTargetModes succeeded"
    );
    STATUS_SUCCESS
}

pub(crate) extern "system" fn monitor_assign_swapchain(
    monitor: IDDCX_MONITOR,
    in_args: *const IDARG_IN_SETSWAPCHAIN,
) -> NTSTATUS {
    tracing::info!("EvtIddCxMonitorAssignSwapChain");

    if in_args.is_null() {
        tracing::warn!("monitor_assign_swapchain called with null args");
        return STATUS_NOT_SUPPORTED;
    }

    // SAFETY: `in_args` is non-null and expected to be a valid pointer provided by IddCx.
    let args = unsafe { &*in_args };

    let processor = match SwapChainProcessor::new(args.hSwapChain, args.RenderAdapterLuid, args.hNextSurfaceAvailable) {
        Ok(processor) => processor,
        Err(error) => {
            tracing::warn!(?error, "failed to create SwapChainProcessor");
            return STATUS_NOT_SUPPORTED;
        }
    };

    let status = processor.start();
    if status != STATUS_SUCCESS {
        tracing::warn!(status, "SwapChainProcessor start returned non-success status");
    }

    let mut swapchains = match active_swapchains().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    let key = MonitorKey(monitor);
    if let Some(previous) = swapchains.remove(&key) {
        let _ = previous.stop();
    }

    swapchains.insert(key, processor);
    STATUS_SUCCESS
}

pub(crate) extern "system" fn monitor_unassign_swapchain(monitor: IDDCX_MONITOR) -> NTSTATUS {
    tracing::info!("EvtIddCxMonitorUnassignSwapChain");

    let mut swapchains = match active_swapchains().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    if let Some(processor) = swapchains.remove(&MonitorKey(monitor)) {
        let _ = processor.stop();
    }

    STATUS_SUCCESS
}
