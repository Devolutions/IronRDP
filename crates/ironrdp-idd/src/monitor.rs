use crate::{IDDCX_ADAPTER, IDDCX_MONITOR, NTSTATUS, STATUS_NOT_SUPPORTED, STATUS_SUCCESS};

#[cfg(feature = "iddcx-experimental-layout")]
use crate::IDDCX_SWAPCHAIN;

#[cfg(feature = "iddcx-experimental-layout")]
use std::collections::HashMap;

#[cfg(feature = "iddcx-experimental-layout")]
use std::sync::{Mutex, OnceLock};

#[cfg(feature = "iddcx-experimental-layout")]
use windows::Win32::Foundation::{HANDLE, LUID};

#[cfg(feature = "iddcx-experimental-layout")]
use crate::SwapChainProcessor;

#[cfg(feature = "iddcx-experimental-layout")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct MonitorKey(IDDCX_MONITOR);

// SAFETY: monitor handles are opaque identifiers provided by IddCx; they can be used as keys across threads.
#[cfg(feature = "iddcx-experimental-layout")]
unsafe impl Send for MonitorKey {}

#[cfg(feature = "iddcx-experimental-layout")]
static ACTIVE_SWAPCHAINS: OnceLock<Mutex<HashMap<MonitorKey, SwapChainProcessor>>> = OnceLock::new();

#[cfg(feature = "iddcx-experimental-layout")]
fn active_swapchains() -> &'static Mutex<HashMap<MonitorKey, SwapChainProcessor>> {
    ACTIVE_SWAPCHAINS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[repr(C)]
pub(crate) struct IDARG_IN_PARSEMONITORDESCRIPTION {
    _private: [u8; 0],
}

#[repr(C)]
pub(crate) struct IDARG_OUT_PARSEMONITORDESCRIPTION {
    _private: [u8; 0],
}

#[repr(C)]
pub(crate) struct IDARG_IN_GETDEFAULTDESCRIPTIONMODES {
    _private: [u8; 0],
}

#[repr(C)]
pub(crate) struct IDARG_OUT_GETDEFAULTDESCRIPTIONMODES {
    _private: [u8; 0],
}

#[repr(C)]
pub(crate) struct IDARG_IN_QUERYTARGETMODES {
    _private: [u8; 0],
}

#[repr(C)]
pub(crate) struct IDARG_OUT_QUERYTARGETMODES {
    _private: [u8; 0],
}

#[repr(C)]
pub(crate) struct IDARG_IN_SETSWAPCHAIN {
    #[cfg(feature = "iddcx-experimental-layout")]
    pub(crate) swapchain: IDDCX_SWAPCHAIN,
    #[cfg(feature = "iddcx-experimental-layout")]
    pub(crate) new_frame_event: HANDLE,
    #[cfg(feature = "iddcx-experimental-layout")]
    pub(crate) render_adapter_luid: LUID,

    #[cfg(not(feature = "iddcx-experimental-layout"))]
    _private: [u8; 0],
}

#[derive(Debug, Clone, Copy)]
pub struct IronRdpIddMonitor {
    pub monitor: IDDCX_MONITOR,
    pub connector_index: u32,
}

impl IronRdpIddMonitor {
    pub fn create_and_arrive(_adapter: IDDCX_ADAPTER, connector_idx: u32, _edid: Option<&[u8]>) -> NTSTATUS {
        tracing::info!(connector_idx, "IddCxMonitorArrival (stub)");
        STATUS_NOT_SUPPORTED
    }

    pub fn departure(&self) -> NTSTATUS {
        tracing::info!(connector_idx = self.connector_index, "IddCxMonitorDeparture (stub)");
        STATUS_NOT_SUPPORTED
    }
}

pub(crate) extern "system" fn parse_monitor_description(
    _in_args: *const IDARG_IN_PARSEMONITORDESCRIPTION,
    _out_args: *mut IDARG_OUT_PARSEMONITORDESCRIPTION,
) -> NTSTATUS {
    tracing::info!("EvtIddCxParseMonitorDescription (stub)");
    STATUS_NOT_SUPPORTED
}

pub(crate) extern "system" fn monitor_get_default_description_modes(
    _monitor: IDDCX_MONITOR,
    _in_args: *const IDARG_IN_GETDEFAULTDESCRIPTIONMODES,
    _out_args: *mut IDARG_OUT_GETDEFAULTDESCRIPTIONMODES,
) -> NTSTATUS {
    tracing::info!("EvtIddCxMonitorGetDefaultDescriptionModes (stub)");
    STATUS_NOT_SUPPORTED
}

pub(crate) extern "system" fn monitor_query_target_modes(
    _monitor: IDDCX_MONITOR,
    _in_args: *const IDARG_IN_QUERYTARGETMODES,
    _out_args: *mut IDARG_OUT_QUERYTARGETMODES,
) -> NTSTATUS {
    tracing::info!("EvtIddCxMonitorQueryTargetModes (stub)");
    STATUS_NOT_SUPPORTED
}

pub(crate) extern "system" fn monitor_assign_swapchain(
    _monitor: IDDCX_MONITOR,
    _in_args: *const IDARG_IN_SETSWAPCHAIN,
) -> NTSTATUS {
    #[cfg(not(feature = "iddcx-experimental-layout"))]
    {
        let _ = (_monitor, _in_args);
        tracing::info!("EvtIddCxMonitorAssignSwapChain (stub; iddcx-experimental-layout disabled)");
        STATUS_NOT_SUPPORTED
    }

    #[cfg(feature = "iddcx-experimental-layout")]
    {
        let monitor = _monitor;
        let in_args = _in_args;

        tracing::info!("EvtIddCxMonitorAssignSwapChain (experimental)");

        if in_args.is_null() {
            tracing::warn!("monitor_assign_swapchain called with null args");
            return STATUS_NOT_SUPPORTED;
        }

        // SAFETY: `in_args` is non-null and expected to be a valid pointer provided by IddCx.
        let args = unsafe { &*in_args };

        let processor = match SwapChainProcessor::new(args.swapchain, args.render_adapter_luid, args.new_frame_event) {
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
}

pub(crate) extern "system" fn monitor_unassign_swapchain(monitor: IDDCX_MONITOR) -> NTSTATUS {
    #[cfg(not(feature = "iddcx-experimental-layout"))]
    {
        let _ = monitor;
        tracing::info!("EvtIddCxMonitorUnassignSwapChain (stub; iddcx-experimental-layout disabled)");
        STATUS_SUCCESS
    }

    #[cfg(feature = "iddcx-experimental-layout")]
    {
        tracing::info!("EvtIddCxMonitorUnassignSwapChain (experimental)");

        let mut swapchains = match active_swapchains().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        if let Some(processor) = swapchains.remove(&MonitorKey(monitor)) {
            let _ = processor.stop();
        }

        STATUS_SUCCESS
    }
}
