use crate::monitor::DISPLAYCONFIG_VIDEO_SIGNAL_INFO;
use crate::{ntstatus_to_u32, IDDCX_ADAPTER, IDDCX_MONITOR, NTSTATUS, STATUS_NOT_SUPPORTED, STATUS_SUCCESS};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

#[cfg(ironrdp_idd_link)]
const IDDCX_ADAPTER_FLAGS_USE_SMALLEST_MODE: u32 = 1;
#[cfg(ironrdp_idd_link)]
const IDDCX_ADAPTER_FLAGS_REMOTE_SESSION_DRIVER: u32 = 4;
#[cfg(ironrdp_idd_link)]
const IDDCX_TRANSMISSION_TYPE_WIRED_OTHER: u32 = 0x0000_0003;
const STATUS_INVALID_PARAMETER: NTSTATUS = crate::ntstatus_from_u32(0xC000_000D);
const STATUS_OBJECT_NAME_COLLISION: NTSTATUS = crate::ntstatus_from_u32(0xC000_0035);
const STATUS_OPERATION_IN_PROGRESS: NTSTATUS = crate::ntstatus_from_u32(0xC000_0476);
const STATUS_DEVICE_REMOVED: NTSTATUS = crate::ntstatus_from_u32(0xC000_02B6);
#[cfg(ironrdp_idd_link)]
const GUID_DEVINTERFACE_IRONRDP_IDD_VIDEO: windows_core::GUID =
    windows_core::GUID::from_u128(0x1ea642e3_6a78_4f4b_9a19_2eb4f0f33b82);

const IDDCX_PATH_FLAGS_CHANGED: u32 = 1;
const IDDCX_PATH_FLAGS_ACTIVE: u32 = 2;

static ADAPTER_INIT_COMPLETED: AtomicBool = AtomicBool::new(false);
static ADAPTER_INIT_FINISHED_SIGNALED: AtomicBool = AtomicBool::new(false);
static ADAPTER_INIT_PROBE_RUNNING: AtomicBool = AtomicBool::new(false);
static ADAPTER_MONITOR_ARRIVAL_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static ADAPTER_OBJECT_RAW: AtomicUsize = AtomicUsize::new(0);

const ADAPTER_INIT_POLL_ATTEMPTS: u32 = 60;
const ADAPTER_INIT_POLL_INTERVAL_MS: u64 = 100;

fn optional_u32_text(value: Option<u32>) -> String {
    value.map(|value| value.to_string()).unwrap_or_else(|| "none".to_owned())
}

#[cfg(ironrdp_idd_link)]
fn enable_video_device_interface(device: crate::wdf::WDFDEVICE, source: &str) {
    crate::debug_trace(&format!(
        "{source}: enabling custom video device interface guid={GUID_DEVINTERFACE_IRONRDP_IDD_VIDEO:?}"
    ));
    // SAFETY: `device` is a live WDFDEVICE handle and the GUID pointer remains valid for the call.
    unsafe {
        crate::wdf::device_set_device_interface_state(
            device,
            &GUID_DEVINTERFACE_IRONRDP_IDD_VIDEO,
            core::ptr::null(),
            true,
        )
    };
    crate::debug_trace(&format!(
        "{source}: invoked WdfDeviceSetDeviceInterfaceState enabled=true"
    ));
}

fn reset_adapter_initialization_state() {
    ADAPTER_INIT_COMPLETED.store(false, Ordering::Release);
    ADAPTER_INIT_FINISHED_SIGNALED.store(false, Ordering::Release);
    ADAPTER_INIT_PROBE_RUNNING.store(false, Ordering::Release);
    ADAPTER_MONITOR_ARRIVAL_IN_PROGRESS.store(false, Ordering::Release);
    ADAPTER_OBJECT_RAW.store(0, Ordering::Release);
}

fn should_retry_monitor_arrival(status: NTSTATUS) -> bool {
    status == STATUS_OPERATION_IN_PROGRESS || status == STATUS_DEVICE_REMOVED
}

fn complete_adapter_initialization_with_retry(
    adapter: IDDCX_ADAPTER,
    adapter_init_status: NTSTATUS,
    source: &str,
) -> NTSTATUS {
    if ADAPTER_INIT_COMPLETED.load(Ordering::Acquire) {
        crate::debug_trace(&format!(
            "{source}: skipped retry because adapter initialization already completed"
        ));
        return STATUS_SUCCESS;
    }

    for attempt in 1..=ADAPTER_INIT_POLL_ATTEMPTS {
        let config = crate::remote::load_runtime_config();
        let adapter_init_finished = ADAPTER_INIT_FINISHED_SIGNALED.load(Ordering::Acquire);
        let session_ready = config.wddm_idd_enabled && config.session_id.is_some();
        crate::debug_trace(&format!(
            "{source}: attempt={} session_id={} wddm_enabled={} driver_loaded={} session_ready={} adapter_init_finished={}",
            attempt,
            optional_u32_text(config.session_id),
            config.wddm_idd_enabled,
            config.driver_loaded,
            session_ready,
            adapter_init_finished,
        ));

        if !session_ready || !adapter_init_finished {
            crate::debug_trace(&format!(
                "{source}: deferring monitor arrival session_ready={} driver_loaded={} adapter_init_finished={}",
                session_ready,
                config.driver_loaded,
                adapter_init_finished,
            ));
            std::thread::sleep(Duration::from_millis(ADAPTER_INIT_POLL_INTERVAL_MS));
            continue;
        }

        let status = complete_adapter_initialization(adapter, adapter_init_status, source);
        crate::debug_trace(&format!(
            "{source}: attempt={} create status=0x{:08X}",
            attempt,
            ntstatus_to_u32(status)
        ));
        if status >= 0 || !should_retry_monitor_arrival(status) {
            return status;
        }

        std::thread::sleep(Duration::from_millis(ADAPTER_INIT_POLL_INTERVAL_MS));
    }

    crate::debug_trace(&format!(
        "{source}: deferred monitor arrival after {} attempts",
        ADAPTER_INIT_POLL_ATTEMPTS
    ));
    STATUS_SUCCESS
}

fn complete_adapter_initialization(adapter: IDDCX_ADAPTER, adapter_init_status: NTSTATUS, source: &str) -> NTSTATUS {
    crate::debug_trace(&format!(
        "{source}: AdapterInitStatus=0x{:08X}",
        ntstatus_to_u32(adapter_init_status)
    ));
    if adapter_init_status < 0 {
        tracing::error!(
            status = adapter_init_status,
            status_hex = format_args!("0x{:08X}", ntstatus_to_u32(adapter_init_status)),
            source,
            "adapter initialization reported failure"
        );
        return adapter_init_status;
    }

    if ADAPTER_INIT_COMPLETED.load(Ordering::Acquire) {
        crate::debug_trace(&format!("{source}: adapter initialization already completed"));
        return STATUS_SUCCESS;
    }

    if ADAPTER_MONITOR_ARRIVAL_IN_PROGRESS
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        crate::debug_trace(&format!(
            "{source}: monitor arrival already in progress completed={}",
            ADAPTER_INIT_COMPLETED.load(Ordering::Acquire)
        ));
        if ADAPTER_INIT_COMPLETED.load(Ordering::Acquire) {
            return STATUS_SUCCESS;
        }
        return STATUS_OPERATION_IN_PROGRESS;
    }

    let status = crate::monitor::IronRdpIddMonitor::create_and_arrive(adapter, 0, None);
    crate::debug_trace(&format!(
        "{source}: create_and_arrive status=0x{:08X}",
        ntstatus_to_u32(status)
    ));
    if status < 0 {
        ADAPTER_MONITOR_ARRIVAL_IN_PROGRESS.store(false, Ordering::Release);
        return status;
    }

    ADAPTER_INIT_COMPLETED.store(true, Ordering::Release);
    ADAPTER_MONITOR_ARRIVAL_IN_PROGRESS.store(false, Ordering::Release);
    crate::remote::note_adapter_init_finished(adapter);
    let runtime_config = crate::remote::load_runtime_config();
    let is_remote = runtime_config.wddm_idd_enabled && runtime_config.session_id.is_some();
    crate::handle_session_transition(adapter, is_remote);
    tracing::info!(source, "adapter initialization completed with monitor arrival");
    tracing::info!("SESSION_PROOF_IDD_ADAPTER_INIT_FINISHED");
    STATUS_SUCCESS
}

fn schedule_adapter_initialization_probe(source: &'static str) {
    if ADAPTER_INIT_COMPLETED.load(Ordering::Acquire) {
        return;
    }

    let adapter_raw = ADAPTER_OBJECT_RAW.load(Ordering::Acquire);
    if adapter_raw == 0 {
        crate::debug_trace(&format!(
            "{source}: background adapter init probe skipped because adapter is not registered"
        ));
        return;
    }

    if ADAPTER_INIT_PROBE_RUNNING.swap(true, Ordering::AcqRel) {
        crate::debug_trace(&format!(
            "{source}: background adapter init probe already running"
        ));
        return;
    }

    let source_owned = source.to_owned();
    if let Err(error) = std::thread::Builder::new()
        .name("irdp-idd-adapter-probe".to_owned())
        .spawn(move || {
            let adapter = adapter_raw as IDDCX_ADAPTER;
            crate::debug_trace(&format!(
                "{source_owned}: background adapter init probe started"
            ));
            let status =
                complete_adapter_initialization_with_retry(adapter, STATUS_SUCCESS, source_owned.as_str());
            crate::debug_trace(&format!(
                "{source_owned}: background adapter init probe completed status=0x{:08X} completed={}",
                ntstatus_to_u32(status),
                ADAPTER_INIT_COMPLETED.load(Ordering::Acquire),
            ));
            ADAPTER_INIT_PROBE_RUNNING.store(false, Ordering::Release);
        })
    {
        ADAPTER_INIT_PROBE_RUNNING.store(false, Ordering::Release);
        crate::debug_trace(&format!(
            "{source}: failed to spawn background adapter init probe: {error}"
        ));
    }
}


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
        crate::debug_trace("EvtIddCxAdapterInitFinished: entered");
        tracing::info!("SESSION_PROOF_IDD_ADAPTER_INIT_ENTERED");
        if args.is_null() {
            crate::debug_trace("EvtIddCxAdapterInitFinished: args is null");
            tracing::warn!("EvtIddCxAdapterInitFinished called with null args");
            return STATUS_NOT_SUPPORTED;
        }

        // SAFETY: `args` is non-null and points to callback input provided by IddCx.
        let adapter_init_status = unsafe { (*args).AdapterInitStatus };
        ADAPTER_INIT_FINISHED_SIGNALED.store(true, Ordering::Release);
        ADAPTER_OBJECT_RAW.store(adapter as usize, Ordering::Release);
        crate::debug_trace("EvtIddCxAdapterInitFinished: signaled adapter init completion");
        let status =
            complete_adapter_initialization_with_retry(adapter, adapter_init_status, "EvtIddCxAdapterInitFinished");
        if !ADAPTER_INIT_COMPLETED.load(Ordering::Acquire) {
            schedule_adapter_initialization_probe("EvtIddCxAdapterInitFinished");
        }
        status
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
unsafe fn init_adapter_async(device: crate::wdf::WDFDEVICE) -> NTSTATUS {
    use crate::iddcx::{
        ENDPOINT_FRIENDLY_NAME_UTF16, ENDPOINT_MANUFACTURER_UTF16, ENDPOINT_MODEL_NAME_UTF16, IDARG_IN_ADAPTER_INIT,
        IDARG_OUT_ADAPTER_INIT, IDDCX_ADAPTER_CAPS, IDDCX_ENDPOINT_DIAGNOSTIC_INFO, IDDCX_ENDPOINT_VERSION,
    };
    use core::mem::size_of;

    let hardware_version = IDDCX_ENDPOINT_VERSION {
        Size: size_of::<IDDCX_ENDPOINT_VERSION>() as u32,
        MajorVer: 1,
        MinorVer: 0,
        Build: 0,
        SKU: 0,
    };
    let firmware_version = IDDCX_ENDPOINT_VERSION {
        Size: size_of::<IDDCX_ENDPOINT_VERSION>() as u32,
        MajorVer: 1,
        MinorVer: 0,
        Build: 0,
        SKU: 0,
    };

    let diag = IDDCX_ENDPOINT_DIAGNOSTIC_INFO {
        Size: size_of::<IDDCX_ENDPOINT_DIAGNOSTIC_INFO>() as u32,
        TransmissionType: IDDCX_TRANSMISSION_TYPE_WIRED_OTHER,
        pEndPointFriendlyName: ENDPOINT_FRIENDLY_NAME_UTF16.as_ptr(),
        pEndPointModelName: ENDPOINT_MODEL_NAME_UTF16.as_ptr(),
        pEndPointManufacturerName: ENDPOINT_MANUFACTURER_UTF16.as_ptr(),
        pHardwareVersion: &hardware_version,
        pFirmwareVersion: &firmware_version,
        // IDDCX_FEATURE_IMPLEMENTATION_NONE = 1
        GammaSupport: 1,
        _pad: 0,
    };

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

    let adapter_object_attributes = crate::wdf::WDF_OBJECT_ATTRIBUTES::init_no_context();
    let in_args = IDARG_IN_ADAPTER_INIT {
        WdfDevice: device,
        pCaps: &mut caps,
        ObjectAttributes: &adapter_object_attributes,
    };
    let mut out_args = IDARG_OUT_ADAPTER_INIT {
        AdapterObject: core::ptr::null_mut(),
    };


    let mut version_args = crate::iddcx::IDARG_OUT_GETVERSION { IddCxVersion: 0 };
    let version_status = unsafe { crate::iddcx::get_version(&mut version_args) };
    crate::debug_trace(&format!(
        "EvtDeviceD0Entry: adapter caps size={} flags=0x{:08X} max_pipeline_rate={} max_monitors={} transmission=0x{:08X} gamma_support={} version_status=0x{:08X} iddcx_version=0x{:08X}",
        caps.Size,
        caps.Flags,
        caps.MaxDisplayPipelineRate,
        caps.MaxMonitorsSupported,
        caps.EndPointDiagnostics.TransmissionType,
        caps.EndPointDiagnostics.GammaSupport,
        ntstatus_to_u32(version_status),
        version_args.IddCxVersion,
    ));
    // SAFETY: in/out args are valid stack structures for this call.
    let status = unsafe { crate::iddcx::adapter_init_async(&in_args, &mut out_args) };
    crate::debug_trace(&format!(
        "EvtDeviceD0Entry: IddCxAdapterInitAsync status=0x{:08X}",
        ntstatus_to_u32(status)
    ));
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
    ADAPTER_OBJECT_RAW.store(out_args.AdapterObject as usize, Ordering::Release);


    let config = crate::remote::load_runtime_config();
    crate::debug_trace(&format!(
        "IddCxAdapterInitAsync: runtime session_id={} wddm_enabled={} driver_loaded={} adapter_init_finished={}",
        optional_u32_text(config.session_id),
        config.wddm_idd_enabled,
        config.driver_loaded,
        ADAPTER_INIT_FINISHED_SIGNALED.load(Ordering::Acquire),
    ));
    let completion_status =
        complete_adapter_initialization_with_retry(out_args.AdapterObject, STATUS_SUCCESS, "IddCxAdapterInitAsync");
    crate::debug_trace(&format!(
        "IddCxAdapterInitAsync: completion status=0x{:08X}",
        ntstatus_to_u32(completion_status)
    ));
    if completion_status < 0 && completion_status != STATUS_OPERATION_IN_PROGRESS {
        return completion_status;
    }
    STATUS_SUCCESS
}

#[cfg(ironrdp_idd_link)]
pub(crate) unsafe extern "system" fn device_d0_entry(
    device: crate::wdf::WDFDEVICE,
    previous_state: crate::wdf::WDF_POWER_DEVICE_STATE,
) -> NTSTATUS {
    let _ = previous_state;
    reset_adapter_initialization_state();

    crate::debug_trace("EvtDeviceD0Entry: entered");
    crate::debug_trace("SESSION_PROOF_IDD_D0_ENTRY");
    tracing::info!("EvtDeviceD0Entry started");
    tracing::info!("SESSION_PROOF_IDD_D0_ENTRY");

    enable_video_device_interface(device, "EvtDeviceD0Entry");

    // SAFETY: WDF invokes this callback with a valid WDFDEVICE handle in D0 transition.
    let status = unsafe { init_adapter_async(device) };
    crate::debug_trace(&format!(
        "EvtDeviceD0Entry: completed status=0x{:08X}",
        ntstatus_to_u32(status)
    ));
    status
}

#[cfg(ironrdp_idd_link)]
pub(crate) unsafe extern "system" fn device_file_create(
    device: crate::wdf::WDFDEVICE,
    request: crate::wdf::WDFREQUEST,
    file_object: crate::wdf::WDFFILEOBJECT,
) {
    let _ = (device, file_object);
    crate::debug_trace("EvtDeviceFileCreate: completing open request with STATUS_SUCCESS");
    unsafe { crate::wdf::request_complete(request, STATUS_SUCCESS) };
    if !ADAPTER_INIT_COMPLETED.load(Ordering::Acquire) {
        schedule_adapter_initialization_probe("EvtDeviceFileCreate");
    }
}

#[cfg(ironrdp_idd_link)]
pub(crate) unsafe extern "system" fn device_add(
    _driver: crate::wdf::WDFDRIVER,
    device_init: *mut crate::wdf::WDFDEVICE_INIT,
) -> NTSTATUS {
    use crate::iddcx::IDD_CX_CLIENT_CONFIG;
    use core::mem::{size_of, transmute};

    tracing::info!("EvtDriverDeviceAdd started");
    crate::debug_trace("EvtDriverDeviceAdd: entered");

    // ── 0. Register PnP/power callbacks ─────────────────────────────────────────────────────
    let mut pnp_power_callbacks = crate::wdf::WDF_PNPPOWER_EVENT_CALLBACKS::init();
    pnp_power_callbacks.EvtDeviceD0Entry = Some(device_d0_entry);
    // SAFETY: device_init is provided by WDF in EvtDriverDeviceAdd and callbacks point to valid functions.
    unsafe { crate::wdf::device_init_set_pnp_power_event_callbacks(device_init, &pnp_power_callbacks) };
    crate::debug_trace("EvtDriverDeviceAdd: WdfDeviceInitSetPnpPowerEventCallbacks configured");

    let file_object_config = crate::wdf::WDF_FILEOBJECT_CONFIG::init(Some(device_file_create));
    unsafe { crate::wdf::device_init_set_file_object_config(device_init, &file_object_config, core::ptr::null()) };
    crate::debug_trace("EvtDriverDeviceAdd: WdfDeviceInitSetFileObjectConfig configured");

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
    config.EvtIddCxMonitorGetPhysicalSize = Some(unsafe {
        transmute::<_, unsafe extern "system" fn()>(crate::monitor::monitor_get_physical_size as usize)
    });

    // ── 2. IddCxDeviceInitConfig ─────────────────────────────────────────────────────────────
    // SAFETY: device_init is a valid PWDFDEVICE_INIT provided by WDF via EvtDriverDeviceAdd.
    let status = unsafe { crate::iddcx::device_init_config(device_init, &config) };
    crate::debug_trace(&format!(
        "EvtDriverDeviceAdd: IddCxDeviceInitConfig status=0x{:08X}",
        ntstatus_to_u32(status)
    ));
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
    crate::debug_trace(&format!(
        "EvtDriverDeviceAdd: WdfDeviceCreate status=0x{:08X}",
        ntstatus_to_u32(status)
    ));
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


    let device_interface_status = unsafe {
        crate::wdf::device_create_device_interface(
            device,
            &GUID_DEVINTERFACE_IRONRDP_IDD_VIDEO,
            core::ptr::null(),
        )
    };
    crate::debug_trace(&format!(
        "EvtDriverDeviceAdd: WdfDeviceCreateDeviceInterface status=0x{:08X}",
        ntstatus_to_u32(device_interface_status)
    ));
    if device_interface_status < 0 {
        tracing::error!(
            status = device_interface_status,
            status_hex = format_args!("0x{:08X}", ntstatus_to_u32(device_interface_status)),
            "WdfDeviceCreateDeviceInterface failed"
        );
        return device_interface_status;
    }
    enable_video_device_interface(device, "EvtDriverDeviceAdd");
    // Expose stable DOS symbolic links so the provider can open from Session 0 and
    // from regular session namespaces.
    let mut symbolic_link_status = STATUS_SUCCESS;
    let mut symbolic_link_created = false;

    for symbolic_link_name in [
        r"\DosDevices\Global\IronRdpIddVideo",
        r"\DosDevices\IronRdpIddVideo",
        r"\??\Global\IronRdpIddVideo",
        r"\??\IronRdpIddVideo",
        r"\Global??\IronRdpIddVideo",
    ] {
        let mut symbolic_link_utf16: Vec<u16> = symbolic_link_name.encode_utf16().collect();
        let symbolic_link = crate::UNICODE_STRING {
            Length: (symbolic_link_utf16.len() * core::mem::size_of::<u16>()) as u16,
            MaximumLength: (symbolic_link_utf16.len() * core::mem::size_of::<u16>()) as u16,
            Buffer: symbolic_link_utf16.as_mut_ptr(),
        };

        // SAFETY: `device` is valid and UNICODE_STRING points to stack-local UTF-16 for call duration.
        symbolic_link_status = unsafe { crate::wdf::device_create_symbolic_link(device, &symbolic_link) };
        crate::debug_trace(&format!(
            "EvtDriverDeviceAdd: WdfDeviceCreateSymbolicLink name={} status=0x{:08X}",
            symbolic_link_name,
            ntstatus_to_u32(symbolic_link_status)
        ));

        if symbolic_link_status >= 0 || symbolic_link_status == STATUS_OBJECT_NAME_COLLISION {
            symbolic_link_created = true;
            tracing::info!(
                status = symbolic_link_status,
                status_hex = format_args!("0x{:08X}", ntstatus_to_u32(symbolic_link_status)),
                symbolic_link = symbolic_link_name,
                "WdfDeviceCreateSymbolicLink completed"
            );
        } else {
            tracing::warn!(
                status = symbolic_link_status,
                status_hex = format_args!("0x{:08X}", ntstatus_to_u32(symbolic_link_status)),
                symbolic_link = symbolic_link_name,
                "WdfDeviceCreateSymbolicLink failed"
            );
        }
    }

    if !symbolic_link_created {
        tracing::error!(
            status = symbolic_link_status,
            status_hex = format_args!("0x{:08X}", ntstatus_to_u32(symbolic_link_status)),
            "all WdfDeviceCreateSymbolicLink attempts failed"
        );
        return symbolic_link_status;
    }

    // ── 4. IddCxDeviceInitialize ─────────────────────────────────────────────────────────────
    // SAFETY: device is a valid WDFDEVICE handle created by WdfDeviceCreate.
    let status = unsafe { crate::iddcx::device_initialize(device) };
    crate::debug_trace(&format!(
        "EvtDriverDeviceAdd: IddCxDeviceInitialize status=0x{:08X}",
        ntstatus_to_u32(status)
    ));
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
    enable_video_device_interface(device, "EvtDriverDeviceAddPostInitialize");
    crate::debug_trace("EvtDriverDeviceAdd: completed successfully");

    tracing::info!("EvtDriverDeviceAdd: IDD device and adapter initialized successfully");
    STATUS_SUCCESS
}

