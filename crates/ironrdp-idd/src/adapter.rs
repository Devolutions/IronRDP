use crate::{ntstatus_to_u32, IDDCX_ADAPTER, IDDCX_MONITOR, NTSTATUS, STATUS_NOT_SUPPORTED, STATUS_SUCCESS};
use crate::monitor::DISPLAYCONFIG_VIDEO_SIGNAL_INFO;

#[cfg(ironrdp_idd_link)]
const IDDCX_ADAPTER_FLAGS_USE_SMALLEST_MODE: u32 = 1;
#[cfg(ironrdp_idd_link)]
const IDDCX_ADAPTER_FLAGS_REMOTE_SESSION_DRIVER: u32 = 4;
const STATUS_INVALID_PARAMETER: NTSTATUS = crate::ntstatus_from_u32(0xC000_000D);

const IDDCX_PATH_FLAGS_CHANGED: u32 = 1;
const IDDCX_PATH_FLAGS_ACTIVE: u32 = 2;

#[repr(C)]
pub struct IDDCX_PATH {
    pub(crate) Size: u32,
    pub(crate) MonitorObject: IDDCX_MONITOR,
    pub(crate) Flags: u32,
    pub(crate) TargetVideoSignalInfo: DISPLAYCONFIG_VIDEO_SIGNAL_INFO,
}

#[repr(C)]
pub(crate) struct IDARG_IN_ADAPTER_INIT_FINISHED {
    pub(crate) AdapterInitStatus: NTSTATUS,
}

#[repr(C)]
pub(crate) struct IDARG_IN_COMMITMODES {
    pub(crate) PathCount: u32,
    pub(crate) pPaths: *mut IDDCX_PATH,
}

#[derive(Debug, Clone, Copy)]
pub struct IronRdpIddAdapter {
    pub iddcx_adapter: IDDCX_ADAPTER,
    pub is_remote: bool,
}

impl IronRdpIddAdapter {
    pub fn new(iddcx_adapter: IDDCX_ADAPTER, is_remote: bool) -> Self {
        Self {
            iddcx_adapter,
            is_remote,
        }
    }

    pub fn init_async(&self) -> NTSTATUS {
        tracing::info!(is_remote = self.is_remote, "IddCxAdapterInitAsync (stub)");
        STATUS_NOT_SUPPORTED
    }
}

pub(crate) extern "system" fn adapter_init_finished(
    adapter: IDDCX_ADAPTER,
    args: *const IDARG_IN_ADAPTER_INIT_FINISHED,
) -> NTSTATUS {
    #[cfg(not(ironrdp_idd_link))]
    {
        let _ = (adapter, args);
        tracing::info!("EvtIddCxAdapterInitFinished (stub)");
        return STATUS_SUCCESS;
    }

    #[cfg(ironrdp_idd_link)]
    {
        if args.is_null() {
            tracing::warn!("EvtIddCxAdapterInitFinished called with null args");
            return STATUS_NOT_SUPPORTED;
        }

        // SAFETY: `args` is non-null and points to callback input provided by IddCx.
        let adapter_init_status = unsafe { (*args).AdapterInitStatus };
        if adapter_init_status < 0 {
            tracing::error!(
                status = adapter_init_status,
                status_hex = format_args!("0x{:08X}", ntstatus_to_u32(adapter_init_status)),
                "EvtIddCxAdapterInitFinished reported adapter initialization failure"
            );
            return adapter_init_status;
        }

        let status = crate::monitor::IronRdpIddMonitor::create_and_arrive(adapter, 0, None);
        if status < 0 {
            tracing::error!(
                status,
                status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
                "failed to create and arrive monitor after adapter init"
            );
            return status;
        }

        tracing::info!("EvtIddCxAdapterInitFinished completed with monitor arrival");
        STATUS_SUCCESS
    }
}

pub(crate) extern "system" fn adapter_commit_modes(
    adapter: IDDCX_ADAPTER,
    args: *const IDARG_IN_COMMITMODES,
) -> NTSTATUS {
    if args.is_null() {
        tracing::warn!("EvtIddCxAdapterCommitModes called with null args");
        return STATUS_INVALID_PARAMETER;
    }

    // SAFETY: `args` is non-null and points to callback input provided by IddCx.
    let args = unsafe { &*args };

    let path_count = match usize::try_from(args.PathCount) {
        Ok(value) => value,
        Err(_) => {
            tracing::warn!(path_count = args.PathCount, "EvtIddCxAdapterCommitModes path count conversion failed");
            return STATUS_INVALID_PARAMETER;
        }
    };

    if path_count > 0 && args.pPaths.is_null() {
        tracing::warn!(path_count = args.PathCount, "EvtIddCxAdapterCommitModes missing paths pointer");
        return STATUS_INVALID_PARAMETER;
    }

    let paths = if path_count == 0 {
        &[]
    } else {
        // SAFETY: `pPaths` is non-null when `path_count > 0`, and points to a caller-owned
        // array for the callback duration.
        unsafe { core::slice::from_raw_parts(args.pPaths.cast_const(), path_count) }
    };

    let display_update_status = crate::remote::set_display_config(adapter, paths);
    if display_update_status < 0 {
        tracing::warn!(
            status = display_update_status,
            status_hex = format_args!("0x{:08X}", ntstatus_to_u32(display_update_status)),
            "IddCxAdapterDisplayConfigUpdate failed during commit modes"
        );
        return display_update_status;
    }

    let mut changed_paths = 0u32;
    let mut active_paths = 0u32;
    let mut inactive_changed_paths = 0u32;
    let mut stopped_swapchains = 0u32;

    for path in paths {
        let is_changed = (path.Flags & IDDCX_PATH_FLAGS_CHANGED) != 0;
        let is_active = (path.Flags & IDDCX_PATH_FLAGS_ACTIVE) != 0;

        if is_changed {
            changed_paths = changed_paths.saturating_add(1);
        }

        if is_active {
            active_paths = active_paths.saturating_add(1);
        }

        if is_changed && !is_active {
            inactive_changed_paths = inactive_changed_paths.saturating_add(1);
            if crate::monitor::stop_swapchain_for_monitor(path.MonitorObject) {
                stopped_swapchains = stopped_swapchains.saturating_add(1);
            }
        }
    }

    tracing::info!(
        path_count = args.PathCount,
        changed_paths,
        active_paths,
        inactive_changed_paths,
        stopped_swapchains,
        "EvtIddCxAdapterCommitModes applied display paths"
    );

    STATUS_SUCCESS
}

// ──────────────── EvtDriverDeviceAdd — real IddCx device initialization ──────────────────────

#[cfg(ironrdp_idd_link)]
pub(crate) unsafe extern "system" fn device_add(
    _driver: crate::wdf::WDFDRIVER,
    device_init: *mut crate::wdf::WDFDEVICE_INIT,
) -> NTSTATUS {
    use crate::iddcx::{
        ENDPOINT_MANUFACTURER_UTF16, ENDPOINT_MODEL_NAME_UTF16, IDARG_IN_ADAPTER_INIT, IDARG_OUT_ADAPTER_INIT,
        IDDCX_ADAPTER_CAPS, IDDCX_ENDPOINT_DIAGNOSTIC_INFO, IDDCX_ENDPOINT_VERSION, IDD_CX_CLIENT_CONFIG,
    };
    use core::mem::{size_of, transmute};

    tracing::info!("EvtDriverDeviceAdd started");

    // ── 1. Build IDD_CX_CLIENT_CONFIG ────────────────────────────────────────────────────────
    // Zero-initialize then populate mandatory callbacks.
    // SAFETY: IDD_CX_CLIENT_CONFIG is a POD struct of function pointers — zero-initialization
    // produces all-None (null) entries, which is valid C ABI for unused callbacks.
    let mut config: IDD_CX_CLIENT_CONFIG = unsafe { core::mem::zeroed() };
    config.Size = size_of::<IDD_CX_CLIENT_CONFIG>() as u32;
    tracing::info!(idd_client_config_size = config.Size, "Prepared IDD_CX_CLIENT_CONFIG");

    // Assign mandatory callbacks by transmuting typed fn pointers → Option<unsafe extern "system" fn()>.
    // SAFETY: all function pointer types share the same calling convention and pointer size.
    config.EvtIddCxAdapterInitFinished =
        Some(unsafe { transmute::<_, unsafe extern "system" fn()>(adapter_init_finished as usize) });
    config.EvtIddCxAdapterCommitModes =
        Some(unsafe { transmute::<_, unsafe extern "system" fn()>(adapter_commit_modes as usize) });
    config.EvtIddCxParseMonitorDescription = Some(unsafe {
        transmute::<_, unsafe extern "system" fn()>(crate::monitor::parse_monitor_description as usize)
    });
    config.EvtIddCxMonitorGetDefaultDescriptionModes = Some(unsafe {
        transmute::<_, unsafe extern "system" fn()>(crate::monitor::monitor_get_default_description_modes as usize)
    });
    config.EvtIddCxMonitorQueryTargetModes = Some(unsafe {
        transmute::<_, unsafe extern "system" fn()>(crate::monitor::monitor_query_target_modes as usize)
    });
    config.EvtIddCxMonitorAssignSwapChain =
        Some(unsafe { transmute::<_, unsafe extern "system" fn()>(crate::monitor::monitor_assign_swapchain as usize) });
    config.EvtIddCxMonitorUnassignSwapChain = Some(unsafe {
        transmute::<_, unsafe extern "system" fn()>(crate::monitor::monitor_unassign_swapchain as usize)
    });

    // ── 2. IddCxDeviceInitConfig ─────────────────────────────────────────────────────────────
    // SAFETY: device_init is a valid PWDFDEVICE_INIT provided by WDF via EvtDriverDeviceAdd.
    let status = unsafe { crate::iddcx::device_init_config(device_init, &config) };
    if status < 0 {
        tracing::error!(
            status,
            status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
            "IddCxDeviceInitConfig failed"
        );
        return status;
    }
    tracing::info!(
        status,
        status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
        "IddCxDeviceInitConfig succeeded"
    );

    // ── 3. WdfDeviceCreate ───────────────────────────────────────────────────────────────────
    let mut device_init_ptr = device_init;
    // SAFETY: device_init_ptr is a valid PWDFDEVICE_INIT that has been configured above.
    // Use WDF_NO_OBJECT_ATTRIBUTES to let WDF apply defaults.
    let (status, device) = unsafe { crate::wdf::device_create(&mut device_init_ptr, core::ptr::null()) };
    if status < 0 {
        tracing::error!(
            status,
            status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
            "WdfDeviceCreate failed"
        );
        return status;
    }
    tracing::info!(
        status,
        status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
        device = ?device,
        "WdfDeviceCreate succeeded"
    );

    // ── 4. IddCxDeviceInitialize ─────────────────────────────────────────────────────────────
    // SAFETY: device is a valid WDFDEVICE handle created by WdfDeviceCreate.
    let status = unsafe { crate::iddcx::device_initialize(device) };
    if status < 0 {
        tracing::error!(
            status,
            status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
            "IddCxDeviceInitialize failed"
        );
        return status;
    }
    tracing::info!(
        status,
        status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
        "IddCxDeviceInitialize succeeded"
    );

    // ── 5. Build IDDCX_ADAPTER_CAPS ──────────────────────────────────────────────────────────
    let endpoint_version = IDDCX_ENDPOINT_VERSION {
        Size: size_of::<IDDCX_ENDPOINT_VERSION>() as u32,
        MajorVer: 1,
        MinorVer: 0,
        Build: 0,
        SKU: 0,
    };

    // IDDCX_TRANSMISSION_TYPE_OTHER = 0xFFFFFFFF (IddCx 1.2)
    let diag = IDDCX_ENDPOINT_DIAGNOSTIC_INFO {
        Size: size_of::<IDDCX_ENDPOINT_DIAGNOSTIC_INFO>() as u32,
        TransmissionType: 0xFFFF_FFFF,
        pEndPointFriendlyName: core::ptr::null(),
        pEndPointModelName: ENDPOINT_MODEL_NAME_UTF16.as_ptr(),
        pEndPointManufacturerName: ENDPOINT_MANUFACTURER_UTF16.as_ptr(),
        pHardwareVersion: &endpoint_version,
        pFirmwareVersion: &endpoint_version,
        // IDDCX_FEATURE_IMPLEMENTATION_NONE = 1
        GammaSupport: 1,
        _pad: 0,
    };

    // Prefer remote-session mode when available (IddCx 1.4+), but keep compatibility with
    // older IddCx runtimes that reject this bit.
    let mut caps = IDDCX_ADAPTER_CAPS {
        Size: size_of::<IDDCX_ADAPTER_CAPS>() as u32,
        Flags: IDDCX_ADAPTER_FLAGS_USE_SMALLEST_MODE | IDDCX_ADAPTER_FLAGS_REMOTE_SESSION_DRIVER,
        MaxDisplayPipelineRate: 0,
        MaxMonitorsSupported: 1,
        _pad: 0,
        EndPointDiagnostics: diag,
        StaticDesktopReencodeFrameCount: 0,
        _pad2: 0,
    };
    tracing::info!(
        adapter_caps_size = caps.Size,
        transmission_type = caps.EndPointDiagnostics.TransmissionType,
        max_monitors = caps.MaxMonitorsSupported,
        "Prepared IDDCX_ADAPTER_CAPS"
    );

    // ── 6. IddCxAdapterInitAsync ─────────────────────────────────────────────────────────────
    let in_args = IDARG_IN_ADAPTER_INIT {
        WdfDevice: device,
        pCaps: &mut caps,
        ObjectAttributes: core::ptr::null(),
    };
    let mut out_args = IDARG_OUT_ADAPTER_INIT {
        AdapterObject: core::ptr::null_mut(),
    };
    // SAFETY: in_args and out_args are valid, stack-allocated structures.
    let mut status = unsafe { crate::iddcx::adapter_init_async(&in_args, &mut out_args) };
    if status == STATUS_INVALID_PARAMETER {
        tracing::warn!(
            attempted_flags = caps.Flags,
            fallback_flags = IDDCX_ADAPTER_FLAGS_USE_SMALLEST_MODE,
            "IddCxAdapterInitAsync rejected remote-session adapter flag, retrying with fallback flags"
        );

        caps.Flags = IDDCX_ADAPTER_FLAGS_USE_SMALLEST_MODE;
        out_args.AdapterObject = core::ptr::null_mut();
        // SAFETY: in_args and out_args remain valid across retries.
        status = unsafe { crate::iddcx::adapter_init_async(&in_args, &mut out_args) };
    }

    if status < 0 {
        tracing::error!(
            status,
            status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
            "IddCxAdapterInitAsync failed"
        );
        return status;
    }

    tracing::info!(
        status,
        status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
        adapter_object = ?out_args.AdapterObject,
        adapter_flags = caps.Flags,
        "IddCxAdapterInitAsync succeeded"
    );

    tracing::info!("EvtDriverDeviceAdd: IDD device and adapter initialized successfully");
    STATUS_SUCCESS
}
