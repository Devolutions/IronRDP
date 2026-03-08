use crate::{
    ntstatus_to_u32, IDDCX_ADAPTER, IDDCX_MONITOR, IDDCX_SWAPCHAIN, NTSTATUS, STATUS_NOT_SUPPORTED, STATUS_SUCCESS,
};
use core::cmp::min;
use core::ffi::c_void;
use core::mem::size_of;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
pub(crate) use windows::Win32::Devices::Display::DISPLAYCONFIG_VIDEO_SIGNAL_INFO;
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_2DREGION, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HDMI, DISPLAYCONFIG_RATIONAL,
    DISPLAYCONFIG_SCANLINE_ORDERING_PROGRESSIVE, DISPLAYCONFIG_TARGET_MODE, DISPLAYCONFIG_VIDEO_SIGNAL_INFO_0,
};
use windows::Win32::Foundation::{HANDLE, LUID};

#[cfg(ironrdp_idd_link)]
use crate::iddcx;
#[cfg(ironrdp_idd_link)]
use crate::wdf;
use crate::SwapChainProcessor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct MonitorKey(IDDCX_MONITOR);

// SAFETY: monitor handles are opaque identifiers provided by IddCx; they can be used as keys across threads.
unsafe impl Send for MonitorKey {}

static ACTIVE_SWAPCHAINS: OnceLock<Mutex<HashMap<MonitorKey, SwapChainProcessor>>> = OnceLock::new();
static ACTIVE_MONITOR: OnceLock<Mutex<Option<ActiveMonitor>>> = OnceLock::new();

fn active_swapchains() -> &'static Mutex<HashMap<MonitorKey, SwapChainProcessor>> {
    ACTIVE_SWAPCHAINS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn active_monitor() -> &'static Mutex<Option<ActiveMonitor>> {
    ACTIVE_MONITOR.get_or_init(|| Mutex::new(None))
}

#[derive(Debug, Clone, Copy)]
struct ActiveMonitor {
    handle_raw: usize,
    connector_index: u32,
    os_target_id: u32,
}

pub(crate) fn current_monitor() -> Option<(IDDCX_MONITOR, u32, u32)> {
    let state = match active_monitor().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    state
        .as_ref()
        .map(|value| (value.handle_raw as IDDCX_MONITOR, value.connector_index, value.os_target_id))
}

const NO_PREFERRED_MODE: u32 = 0xFFFF_FFFF;
const IDDCX_MONITOR_MODE_ORIGIN_MONITORDESCRIPTOR: u32 = 1;
const IDDCX_MONITOR_MODE_ORIGIN_DRIVER: u32 = 2;
const IDDCX_MONITOR_DESCRIPTION_TYPE_EDID: u32 = 1;

#[derive(Clone, Copy)]
struct SampleMode {
    width: u32,
    height: u32,
    refresh_hz: u32,
}

const DEFAULT_DESCRIPTION_MODES: [SampleMode; 3] = [
    SampleMode {
        width: 1920,
        height: 1080,
        refresh_hz: 60,
    },
    SampleMode {
        width: 1600,
        height: 900,
        refresh_hz: 60,
    },
    SampleMode {
        width: 1024,
        height: 768,
        refresh_hz: 75,
    },
];

const TARGET_MODES: [SampleMode; 10] = [
    SampleMode {
        width: 3840,
        height: 2160,
        refresh_hz: 60,
    },
    SampleMode {
        width: 2560,
        height: 1440,
        refresh_hz: 144,
    },
    SampleMode {
        width: 2560,
        height: 1440,
        refresh_hz: 90,
    },
    SampleMode {
        width: 2560,
        height: 1440,
        refresh_hz: 60,
    },
    SampleMode {
        width: 1920,
        height: 1080,
        refresh_hz: 144,
    },
    SampleMode {
        width: 1920,
        height: 1080,
        refresh_hz: 90,
    },
    SampleMode {
        width: 1920,
        height: 1080,
        refresh_hz: 60,
    },
    SampleMode {
        width: 1600,
        height: 900,
        refresh_hz: 60,
    },
    SampleMode {
        width: 1024,
        height: 768,
        refresh_hz: 75,
    },
    SampleMode {
        width: 1024,
        height: 768,
        refresh_hz: 60,
    },
];

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

#[cfg(ironrdp_idd_link)]
pub(crate) use crate::iddcx::IDARG_OUT_MONITORGETPHYSICALSIZE;

#[cfg(not(ironrdp_idd_link))]
#[repr(C)]
pub(crate) struct IDARG_OUT_MONITORGETPHYSICALSIZE {
    pub(crate) PhysicalWidth: u32,
    pub(crate) PhysicalHeight: u32,
}

fn additional_signal_info(video_standard: u16, v_sync_freq_divider: u8) -> u32 {
    u32::from(video_standard) | (u32::from(v_sync_freq_divider & 0x3F) << 16)
}

fn additional_signal_info_union(video_standard: u16, v_sync_freq_divider: u8) -> DISPLAYCONFIG_VIDEO_SIGNAL_INFO_0 {
    DISPLAYCONFIG_VIDEO_SIGNAL_INFO_0 {
        videoStandard: additional_signal_info(video_standard, v_sync_freq_divider),
    }
}

fn default_video_signal_info(v_sync_freq_divider: u8) -> DISPLAYCONFIG_VIDEO_SIGNAL_INFO {
    sample_mode_signal_info(DEFAULT_DESCRIPTION_MODES[0], v_sync_freq_divider)
}

fn sample_mode_signal_info(mode: SampleMode, v_sync_freq_divider: u8) -> DISPLAYCONFIG_VIDEO_SIGNAL_INFO {
    let pixel_rate = u64::from(mode.width)
        .saturating_mul(u64::from(mode.height))
        .saturating_mul(u64::from(mode.refresh_hz));

    DISPLAYCONFIG_VIDEO_SIGNAL_INFO {
        pixelRate: pixel_rate,
        hSyncFreq: DISPLAYCONFIG_RATIONAL {
            Numerator: mode.height.saturating_mul(mode.refresh_hz),
            Denominator: 1,
        },
        vSyncFreq: DISPLAYCONFIG_RATIONAL {
            Numerator: mode.refresh_hz,
            Denominator: 1,
        },
        activeSize: DISPLAYCONFIG_2DREGION {
            cx: mode.width,
            cy: mode.height,
        },
        totalSize: DISPLAYCONFIG_2DREGION {
            cx: mode.width,
            cy: mode.height,
        },
        Anonymous: additional_signal_info_union(255, v_sync_freq_divider),
        scanLineOrdering: DISPLAYCONFIG_SCANLINE_ORDERING_PROGRESSIVE,
    }
}

fn default_monitor_mode(origin: u32, mode: SampleMode) -> IDDCX_MONITOR_MODE {
    IDDCX_MONITOR_MODE {
        Size: size_of::<IDDCX_MONITOR_MODE>() as u32,
        Origin: origin,
        // For monitor modes, vSyncFreqDivider must be zero.
        MonitorVideoSignalInfo: sample_mode_signal_info(mode, 0),
    }
}

fn default_target_mode(mode: SampleMode) -> IDDCX_TARGET_MODE {
    let signal_info = sample_mode_signal_info(mode, 1);
    IDDCX_TARGET_MODE {
        Size: size_of::<IDDCX_TARGET_MODE>() as u32,
        TargetVideoSignalInfo: DISPLAYCONFIG_TARGET_MODE {
            targetVideoSignalInfo: signal_info,
        },
        RequiredBandwidth: 0,
    }
}

fn write_mode_list<T: Copy>(buffer_input_count: u32, buffer_ptr: *mut T, modes: &[T]) -> (u32, bool) {
    let total_count = u32::try_from(modes.len()).unwrap_or(u32::MAX);

    if buffer_ptr.is_null() || buffer_input_count == 0 {
        return (total_count, false);
    }

    let copy_count = min(modes.len(), buffer_input_count as usize);
    let copied_all = copy_count == modes.len();
    // SAFETY: `buffer_ptr` points to a writable output array provided by IddCx; `copy_count`
    // is bounded by both the source and destination lengths.
    unsafe {
        core::ptr::copy_nonoverlapping(modes.as_ptr(), buffer_ptr, copy_count);
    }

    (total_count, copied_all)
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
            crate::debug_trace(&format!(
                "Monitor::create_and_arrive: entered connector_idx={} has_edid={}",
                connector_idx,
                edid.is_some_and(|bytes| !bytes.is_empty())
            ));
            let (description_type, description_size, description_ptr) = match edid {
                Some(bytes) if !bytes.is_empty() => (
                    IDDCX_MONITOR_DESCRIPTION_TYPE_EDID,
                    bytes.len() as u32,
                    bytes.as_ptr() as *mut c_void,
                ),
                _ => (IDDCX_MONITOR_DESCRIPTION_TYPE_EDID, 0, core::ptr::null_mut()),
            };

            let mut monitor_info = iddcx::IDDCX_MONITOR_INFO {
                Size: size_of::<iddcx::IDDCX_MONITOR_INFO>() as u32,
                MonitorType: DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HDMI.0 as u32,
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

            let monitor_object_attributes = wdf::WDF_OBJECT_ATTRIBUTES::init_no_context();

            let mut in_args = iddcx::IDARG_IN_MONITORCREATE {
                ObjectAttributes: &monitor_object_attributes,
                pMonitorInfo: &mut monitor_info,
            };
            let mut out_create = iddcx::IDARG_OUT_MONITORCREATE {
                MonitorObject: core::ptr::null_mut(),
            };

            // SAFETY: all pointers in `in_args` and `out_create` are valid for the duration of the call.
            let mut create_status = unsafe { iddcx::monitor_create(adapter, &in_args, &mut out_create) };
            crate::debug_trace(&format!(
                "Monitor::create_and_arrive: IddCxMonitorCreate(initial) status=0x{:08X}",
                ntstatus_to_u32(create_status)
            ));

            if create_status < 0 {
                tracing::warn!(
                    status = create_status,
                    status_hex = format_args!("0x{:08X}", ntstatus_to_u32(create_status)),
                    connector_idx,
                    "IddCxMonitorCreate failed with default object attributes, retrying with null object attributes"
                );

                in_args.ObjectAttributes = core::ptr::null();
                out_create.MonitorObject = core::ptr::null_mut();
                // SAFETY: all pointers in `in_args` and `out_create` are valid for the duration of the call.
                create_status = unsafe { iddcx::monitor_create(adapter, &in_args, &mut out_create) };
                crate::debug_trace(&format!(
                    "Monitor::create_and_arrive: IddCxMonitorCreate(null_object_attrs) status=0x{:08X}",
                    ntstatus_to_u32(create_status)
                ));
            }

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
            crate::debug_trace(&format!(
                "Monitor::create_and_arrive: IddCxMonitorArrival status=0x{:08X}",
                ntstatus_to_u32(arrival_status)
            ));
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
            {
                let mut state = match active_monitor().lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                *state = Some(ActiveMonitor {
                    handle_raw: out_create.MonitorObject as usize,
                    connector_index: connector_idx,
                    os_target_id: out_arrival.OsTargetId,
                });
            }
            crate::remote::note_monitor_arrival(adapter, out_create.MonitorObject, connector_idx, out_arrival.OsTargetId);
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
            {
                let mut state = match active_monitor().lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if state.as_ref().is_some_and(|value| value.handle_raw == self.monitor as usize) {
                    *state = None;
                }
            }

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
            let modes = DEFAULT_DESCRIPTION_MODES.map(|mode| default_monitor_mode(IDDCX_MONITOR_MODE_ORIGIN_DRIVER, mode));
            let (output_count, copied_all) =
                write_mode_list(in_args.MonitorModeBufferInputCount, in_args.pMonitorModes, &modes);
            out_args.MonitorModeBufferOutputCount = output_count;
            out_args.PreferredMonitorModeIdx = if output_count > 0 { 0 } else { NO_PREFERRED_MODE };
            crate::debug_trace(&format!(
                "EvtIddCxParseMonitorDescription: no descriptor provided output_count={output_count} input_capacity={} copied_all={copied_all}",
                in_args.MonitorModeBufferInputCount
            ));
            tracing::info!(
                monitor_mode_output_count = output_count,
                monitor_mode_input_capacity = in_args.MonitorModeBufferInputCount,
                copied_all,
                "EvtIddCxParseMonitorDescription: no descriptor provided; returning fallback driver mode"
            );
            return STATUS_SUCCESS;
        }

        let modes = [default_monitor_mode(
            IDDCX_MONITOR_MODE_ORIGIN_MONITORDESCRIPTOR,
            DEFAULT_DESCRIPTION_MODES[0],
        )];
        let (output_count, copied_all) =
            write_mode_list(in_args.MonitorModeBufferInputCount, in_args.pMonitorModes, &modes);

        out_args.MonitorModeBufferOutputCount = output_count;
        out_args.PreferredMonitorModeIdx = if output_count > 0 { 0 } else { NO_PREFERRED_MODE };

        tracing::info!(
            monitor_mode_output_count = output_count,
            monitor_mode_input_capacity = in_args.MonitorModeBufferInputCount,
            copied_all,
            "EvtIddCxParseMonitorDescription succeeded"
        );
        crate::debug_trace(&format!(
            "EvtIddCxParseMonitorDescription: descriptor output_count={output_count} input_capacity={} copied_all={copied_all}",
            in_args.MonitorModeBufferInputCount
        ));
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

    let modes = DEFAULT_DESCRIPTION_MODES.map(|mode| default_monitor_mode(IDDCX_MONITOR_MODE_ORIGIN_DRIVER, mode));
    let (output_count, copied_all) = write_mode_list(
        in_args.DefaultMonitorModeBufferInputCount,
        in_args.pDefaultMonitorModes,
        &modes,
    );

    out_args.DefaultMonitorModeBufferOutputCount = output_count;
    out_args.PreferredMonitorModeIdx = if output_count > 0 { 0 } else { NO_PREFERRED_MODE };

    tracing::info!(
        default_mode_output_count = output_count,
        default_mode_input_capacity = in_args.DefaultMonitorModeBufferInputCount,
        copied_all,
        "EvtIddCxMonitorGetDefaultDescriptionModes succeeded"
    );
    crate::debug_trace(&format!(
        "EvtIddCxMonitorGetDefaultDescriptionModes: output_count={output_count} input_capacity={} copied_all={copied_all} preferred_idx={}",
        in_args.DefaultMonitorModeBufferInputCount,
        out_args.PreferredMonitorModeIdx
    ));
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

    let modes = TARGET_MODES.map(default_target_mode);
    let (output_count, copied_all) = write_mode_list(in_args.TargetModeBufferInputCount, in_args.pTargetModes, &modes);

    out_args.TargetModeBufferOutputCount = output_count;
    tracing::info!(
        target_mode_output_count = output_count,
        target_mode_input_capacity = in_args.TargetModeBufferInputCount,
        copied_all,
        "EvtIddCxMonitorQueryTargetModes succeeded"
    );
    let primary_mode = TARGET_MODES[0];
    crate::debug_trace(&format!(
        "EvtIddCxMonitorQueryTargetModes: output_count={output_count} input_capacity={} copied_all={copied_all} first_mode={}x{}@{} pixel_rate={} v_sync_num={} v_sync_den={} additional_signal=0x{:08X}",
        in_args.TargetModeBufferInputCount,
        primary_mode.width,
        primary_mode.height,
        primary_mode.refresh_hz,
        modes[0].TargetVideoSignalInfo.targetVideoSignalInfo.pixelRate,
        modes[0].TargetVideoSignalInfo.targetVideoSignalInfo.vSyncFreq.Numerator,
        modes[0].TargetVideoSignalInfo.targetVideoSignalInfo.vSyncFreq.Denominator,
        // SAFETY: we intentionally inspect the union using its raw `videoStandard` view.
        unsafe { modes[0].TargetVideoSignalInfo.targetVideoSignalInfo.Anonymous.videoStandard }
    ));
    STATUS_SUCCESS
}

pub(crate) extern "system" fn monitor_assign_swapchain(
    monitor: IDDCX_MONITOR,
    in_args: *const IDARG_IN_SETSWAPCHAIN,
) -> NTSTATUS {
    tracing::info!("EvtIddCxMonitorAssignSwapChain");
    crate::remote::note_swapchain_assignment(crate::remote::runtime_session_id());

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

pub(crate) extern "system" fn monitor_get_physical_size(
    monitor: IDDCX_MONITOR,
    out_args: *mut IDARG_OUT_MONITORGETPHYSICALSIZE,
) -> NTSTATUS {
    let _ = monitor;

    if out_args.is_null() {
        tracing::warn!("EvtIddCxMonitorGetPhysicalSize called with null args");
        return STATUS_NOT_SUPPORTED;
    }

    // A reasonable 27-inch 16:9 physical size. Remote IDD 1.4 requires this when no EDID is provided.
    unsafe {
        (*out_args).PhysicalWidth = 598;
        (*out_args).PhysicalHeight = 336;
    }

    crate::debug_trace("SESSION_PROOF_IDD_MONITOR_PHYSICAL_SIZE width_mm=598 height_mm=336");
    tracing::info!(width_mm = 598, height_mm = 336, "SESSION_PROOF_IDD_MONITOR_PHYSICAL_SIZE");
    STATUS_SUCCESS
}

pub(crate) extern "system" fn monitor_unassign_swapchain(monitor: IDDCX_MONITOR) -> NTSTATUS {
    tracing::info!("EvtIddCxMonitorUnassignSwapChain");
    crate::remote::note_swapchain_unassignment(crate::remote::runtime_session_id());

    let _ = stop_swapchain_for_monitor(monitor);

    STATUS_SUCCESS
}

pub(crate) fn stop_swapchain_for_monitor(monitor: IDDCX_MONITOR) -> bool {
    let mut swapchains = match active_swapchains().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    if let Some(processor) = swapchains.remove(&MonitorKey(monitor)) {
        let _ = processor.stop();
        return true;
    }

    false
}

