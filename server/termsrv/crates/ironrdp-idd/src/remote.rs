#[cfg(ironrdp_idd_link)]
use core::mem::size_of;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

#[cfg(ironrdp_idd_link)]
use ironrdp_wtsprotocol_ipc::display::DISPLAY_REQUEST_DIRECTORY;
use ironrdp_wtsprotocol_ipc::display::DisplaySize;
#[cfg(ironrdp_idd_link)]
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_2DREGION, DISPLAYCONFIG_RATIONAL, DISPLAYCONFIG_ROTATION_IDENTITY,
};
#[cfg(ironrdp_idd_link)]
use windows::Win32::Foundation::POINT;

use crate::{IDDCX_ADAPTER, NTSTATUS, STATUS_SUCCESS};
#[cfg(ironrdp_idd_link)]
use crate::{IDDCX_MONITOR, ntstatus_to_u32};

#[derive(Debug, Clone, Default)]
pub(crate) struct RuntimeConfig {
    pub dump_dir: Option<PathBuf>,
    pub session_id: Option<u32>,
    pub wddm_idd_enabled: bool,
    pub driver_loaded: bool,
    pub hardware_id: Option<String>,
    pub active_video_source: Option<String>,
}

#[derive(Debug, Default)]
struct AdapterRuntimeState {
    adapter_raw: usize,
    monitor_raw: usize,
    timer_raw: usize,
    last_is_remote: bool,
    last_session_id: Option<u32>,
    adapter_initialized: bool,
    monitor_arrived: bool,
    last_monitor_connector_index: Option<u32>,
    last_monitor_os_target_id: Option<u32>,
    display_config_requested: bool,
    display_config_request_session_id: Option<u32>,
    display_config_request_size: Option<DisplaySize>,
    last_path_count: usize,
    last_changed_paths: u32,
    last_active_paths: u32,
    swapchain_assigned: bool,
    first_frame_logged: bool,
    session_ready_logged: bool,
}

static ADAPTER_RUNTIME_STATE: OnceLock<Mutex<AdapterRuntimeState>> = OnceLock::new();
// The INF disables host sharing, so each process binds to one OS-owned device session.
static DEVICE_SESSION_ID: AtomicU32 = AtomicU32::new(0);
static DEVICE_ACTIVE: AtomicBool = AtomicBool::new(false);
#[cfg(ironrdp_idd_link)]
static DISPLAY_CONFIG_UPDATE_LOCK: Mutex<()> = Mutex::new(());

fn adapter_runtime_state() -> &'static Mutex<AdapterRuntimeState> {
    ADAPTER_RUNTIME_STATE.get_or_init(|| Mutex::new(AdapterRuntimeState::default()))
}

fn optional_u32_text(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_owned())
}

fn optional_string_text(value: Option<&str>) -> String {
    value
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "none".to_owned())
}

fn maybe_log_session_ready(config: &RuntimeConfig, state: &mut AdapterRuntimeState) {
    let session_ready =
        config.wddm_idd_enabled && config.driver_loaded && config.session_id.is_some() && state.last_active_paths > 0;

    if session_ready && !state.session_ready_logged {
        if let Some(session_id) = config.session_id {
            crate::debug_trace(&format!(
                "SESSION_PROOF_IDD_SESSION_READY_FOR_CAPTURE session_id={session_id} active_paths={} swapchain_assigned={} first_frame_logged={} driver_loaded={}",
                state.last_active_paths, state.swapchain_assigned, state.first_frame_logged, config.driver_loaded,
            ));
            tracing::info!(
                session_id,
                active_paths = state.last_active_paths,
                swapchain_assigned = state.swapchain_assigned,
                first_frame_logged = state.first_frame_logged,
                driver_loaded = config.driver_loaded,
                "SESSION_PROOF_IDD_SESSION_READY_FOR_CAPTURE"
            );
            state.session_ready_logged = true;
        }
    } else if !session_ready {
        state.session_ready_logged = false;
    }
}

fn reset_display_config_state(config: &RuntimeConfig, state: &mut AdapterRuntimeState, reason: &str) {
    let had_state = state.display_config_requested
        || state.last_path_count > 0
        || state.last_changed_paths > 0
        || state.last_active_paths > 0
        || state.swapchain_assigned
        || state.first_frame_logged
        || state.session_ready_logged;

    state.display_config_requested = false;
    state.display_config_request_session_id = None;
    state.display_config_request_size = None;
    state.last_path_count = 0;
    state.last_changed_paths = 0;
    state.last_active_paths = 0;
    state.swapchain_assigned = false;
    state.first_frame_logged = false;
    state.session_ready_logged = false;

    if had_state {
        crate::debug_trace(&format!(
            "SESSION_PROOF_IDD_DISPLAY_CONFIG_RESET reason={reason} session_id={} wddm_enabled={} driver_loaded={}",
            optional_u32_text(config.session_id),
            config.wddm_idd_enabled,
            config.driver_loaded,
        ));
        tracing::info!(
            reason,
            session_id = config.session_id,
            wddm_enabled = config.wddm_idd_enabled,
            driver_loaded = config.driver_loaded,
            "SESSION_PROOF_IDD_DISPLAY_CONFIG_RESET"
        );
    }
}

pub(crate) fn bind_device_session(session_id: u32) -> bool {
    match DEVICE_SESSION_ID.compare_exchange(0, session_id, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => true,
        Err(bound_session_id) => bound_session_id == session_id,
    }
}

#[cfg(ironrdp_idd_link)]
pub(crate) fn set_device_active(active: bool) {
    let mut state = adapter_runtime_state()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    DEVICE_ACTIVE.store(active, Ordering::Release);
    if state.timer_raw != 0 {
        // SAFETY: D0 exit precedes object cleanup, and the state lock excludes timer publication.
        unsafe { crate::wdf::timer_stop(state.timer_raw as crate::wdf::WDFTIMER) };
    }
    *state = AdapterRuntimeState::default();
}

#[cfg(ironrdp_idd_link)]
pub(crate) fn note_adapter_created(adapter: IDDCX_ADAPTER) {
    let mut state = adapter_runtime_state()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    state.adapter_raw = adapter as usize;
}

#[cfg(ironrdp_idd_link)]
pub(crate) fn adapter_is_active(adapter: IDDCX_ADAPTER) -> bool {
    let state = adapter_runtime_state()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    DEVICE_ACTIVE.load(Ordering::Acquire) && state.adapter_raw == adapter as usize
}

pub(crate) fn load_runtime_config() -> RuntimeConfig {
    let session_id = DEVICE_SESSION_ID.load(Ordering::Acquire);
    let active = DEVICE_ACTIVE.load(Ordering::Acquire);
    let mut config = RuntimeConfig {
        session_id: (session_id != 0).then_some(session_id),
        wddm_idd_enabled: active && session_id != 0,
        driver_loaded: active && session_id != 0,
        ..RuntimeConfig::default()
    };
    let Ok(content) = std::fs::read_to_string(crate::IDD_RUNTIME_STATE_FILE) else {
        return config;
    };

    let mut values = HashMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };

        values.insert(key.trim().to_owned(), value.trim().to_owned());
    }

    config.dump_dir = values
        .get("dump_dir")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    config.hardware_id = values.get("hardware_id").cloned();
    config.active_video_source = values.get("active_video_source").cloned();
    config
}

pub(crate) fn runtime_dump_dir() -> Option<PathBuf> {
    load_runtime_config().dump_dir
}

pub(crate) fn runtime_session_id() -> Option<u32> {
    load_runtime_config().session_id
}

pub(crate) fn swapchain_dump_runtime() -> Result<RuntimeConfig, String> {
    let config = load_runtime_config();
    if config.dump_dir.is_none() {
        return Err("dump_dir missing from runtime config".to_owned());
    }
    if config.session_id.is_none() {
        return Err("session_id missing from runtime config".to_owned());
    }
    if !config.wddm_idd_enabled {
        return Err("wddm_idd_enabled is false in runtime config".to_owned());
    }

    Ok(config)
}

#[cfg(ironrdp_idd_link)]
unsafe extern "system" fn display_timer_tick(timer: crate::wdf::WDFTIMER) {
    // WDF drains this callback before cleaning up its monitor parent or adapter ancestor.
    // Native display calls must stay here, never in a detached thread or a power callback.
    // UNTESTED: A native update must finish or abort during stop without waiting for parent cleanup.
    // Force that overlap when changing this callback; mocked rundown does not prove native progress.
    maybe_request_display_config_update(timer);
}

#[cfg(ironrdp_idd_link)]
fn maybe_request_display_config_update(timer: crate::wdf::WDFTIMER) {
    let source = "display_timer";
    let _update_guard = match DISPLAY_CONFIG_UPDATE_LOCK.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::WouldBlock) => return,
        Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
    };

    let config = load_runtime_config();
    let Some(session_id) = config.session_id else {
        return;
    };
    if !config.wddm_idd_enabled {
        return;
    }

    let size = match DisplaySize::read(std::path::Path::new(DISPLAY_REQUEST_DIRECTORY), session_id) {
        Ok(size) => size,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            DisplaySize::new(1024, 768).expect("valid default display size")
        }
        Err(error) => {
            tracing::error!(%error, session_id, "Failed to read session display size");
            return;
        }
    };

    let (adapter, monitor, connector_index, os_target_id, previous) = {
        let mut state = match adapter_runtime_state().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        if !DEVICE_ACTIVE.load(Ordering::Acquire)
            || state.timer_raw != timer as usize
            || !state.adapter_initialized
            || !state.monitor_arrived
        {
            return;
        }

        // The OS stores one display configuration per remote session and reapplies it on its own,
        // for example on RedrawWindow after StopScreenUpdates deactivated every path.
        // Re-sending it while paths are inactive makes the OS recommit modes in a loop.
        if state.display_config_requested
            && state.display_config_request_session_id == Some(session_id)
            && state.display_config_request_size == Some(size)
        {
            return;
        }

        let previous = state.display_config_request_size;
        state.display_config_requested = true;
        state.display_config_request_session_id = Some(session_id);
        state.display_config_request_size = Some(size);
        (
            state.adapter_raw as IDDCX_ADAPTER,
            state.monitor_raw as IDDCX_MONITOR,
            state.last_monitor_connector_index,
            state.last_monitor_os_target_id,
            previous,
        )
    };

    let mut path = crate::iddcx::IDDCX_DISPLAYCONFIGPATH {
        Size: size_of::<crate::iddcx::IDDCX_DISPLAYCONFIGPATH>() as u32,
        MonitorObject: monitor,
        Position: POINT { x: 0, y: 0 },
        Resolution: DISPLAYCONFIG_2DREGION {
            cx: u32::from(size.width()),
            cy: u32::from(size.height()),
        },
        Rotation: DISPLAYCONFIG_ROTATION_IDENTITY,
        RefreshRate: DISPLAYCONFIG_RATIONAL {
            Numerator: 60,
            Denominator: 1,
        },
        VSyncFreqDivider: 1,
        MonitorScaleFactor: 100,
        PhysicalWidthOverride: 0,
        PhysicalHeightOverride: 0,
    };
    let mut in_args = crate::iddcx::IDARG_IN_ADAPTERDISPLAYCONFIGUPDATE {
        PathCount: 1,
        pPaths: &mut path,
    };

    crate::debug_trace(&format!(
        "SESSION_PROOF_IDD_DISPLAY_CONFIG_UPDATE_REQUEST source={source} session_id={session_id} connector_index={connector_index:?} os_target_id={os_target_id:?} resolution={}x{} refresh={}/{} monitor_scale={} v_sync_divider={}",
        path.Resolution.cx,
        path.Resolution.cy,
        path.RefreshRate.Numerator,
        path.RefreshRate.Denominator,
        path.MonitorScaleFactor,
        path.VSyncFreqDivider,
    ));
    tracing::info!(
        source,
        session_id,
        connector_index,
        os_target_id,
        width = path.Resolution.cx,
        height = path.Resolution.cy,
        refresh_num = path.RefreshRate.Numerator,
        refresh_den = path.RefreshRate.Denominator,
        monitor_scale = path.MonitorScaleFactor,
        v_sync_divider = path.VSyncFreqDivider,
        "SESSION_PROOF_IDD_DISPLAY_CONFIG_UPDATE_REQUEST"
    );

    let status = crate::monitor::update_target_modes(monitor, size, previous);
    crate::debug_trace(&format!(
        "SESSION_PROOF_IDD_TARGET_MODES_UPDATE session_id={session_id} status=0x{:08X}",
        ntstatus_to_u32(status)
    ));
    let status = if status < 0 {
        status
    } else {
        // SAFETY: The monitor-parented timer keeps both objects ahead of cleanup until this callback returns.
        unsafe { crate::iddcx::adapter_display_config_update(adapter, &mut in_args) }
    };

    {
        let mut state = match adapter_runtime_state().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        if state.timer_raw == timer as usize && status < 0 && status != crate::ntstatus_from_u32(0xC000_000D) {
            state.display_config_requested = false;
            state.display_config_request_session_id = None;
        }
    }

    crate::debug_trace(&format!(
        "SESSION_PROOF_IDD_DISPLAY_CONFIG_UPDATE_RESULT source={source} session_id={session_id} status=0x{:08X}",
        ntstatus_to_u32(status)
    ));
    if status < 0 {
        tracing::warn!(
            source,
            session_id,
            status,
            status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
            "SESSION_PROOF_IDD_DISPLAY_CONFIG_UPDATE_RESULT"
        );
    } else {
        tracing::info!(
            source,
            session_id,
            status,
            status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
            "SESSION_PROOF_IDD_DISPLAY_CONFIG_UPDATE_RESULT"
        );
    }
}

pub(crate) fn requested_display_size() -> Option<DisplaySize> {
    adapter_runtime_state()
        .lock()
        .expect("adapter runtime state poisoned")
        .display_config_request_size
}

#[cfg(ironrdp_idd_link)]
pub(crate) fn note_adapter_init_finished(adapter: IDDCX_ADAPTER) -> NTSTATUS {
    {
        let mut state = match adapter_runtime_state().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !DEVICE_ACTIVE.load(Ordering::Acquire) || state.adapter_raw != adapter as usize {
            tracing::warn!("Discarded adapter completion after power exit");
            return crate::ntstatus_from_u32(0xC000_02B6);
        }
        // SAFETY: The framework completion callback owns the adapter and its newly arrived monitor.
        let (status, timer) =
            unsafe { crate::wdf::timer_create(state.monitor_raw as IDDCX_MONITOR, display_timer_tick) };
        crate::debug_trace(&format!(
            "WdfTimerCreate: display update timer status=0x{:08X}",
            ntstatus_to_u32(status),
        ));
        if status < 0 {
            tracing::error!(status, "Failed to create display update timer");
            return status;
        }
        state.adapter_initialized = true;
        state.timer_raw = timer as usize;
        // SAFETY: The state lock prevents power exit from retiring this newly created timer.
        unsafe { crate::wdf::timer_start(timer) };
    }

    let config = load_runtime_config();
    crate::debug_trace(&format!(
        "SESSION_PROOF_IDD_ADAPTER_INIT_READY session_id={} wddm_enabled={} driver_loaded={}",
        optional_u32_text(config.session_id),
        config.wddm_idd_enabled,
        config.driver_loaded,
    ));
    tracing::info!(
        session_id = config.session_id,
        wddm_enabled = config.wddm_idd_enabled,
        driver_loaded = config.driver_loaded,
        "SESSION_PROOF_IDD_ADAPTER_INIT_READY"
    );

    STATUS_SUCCESS
}

#[cfg(ironrdp_idd_link)]
pub(crate) fn note_monitor_arrival(
    adapter: IDDCX_ADAPTER,
    monitor: IDDCX_MONITOR,
    connector_index: u32,
    os_target_id: u32,
) -> NTSTATUS {
    let config = load_runtime_config();
    {
        let mut state = match adapter_runtime_state().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !DEVICE_ACTIVE.load(Ordering::Acquire) || state.adapter_raw != adapter as usize {
            tracing::warn!("Discarded monitor arrival after power exit");
            return crate::ntstatus_from_u32(0xC000_02B6);
        }
        state.monitor_raw = monitor as usize;
        state.monitor_arrived = true;
        state.last_monitor_connector_index = Some(connector_index);
        state.last_monitor_os_target_id = Some(os_target_id);
        state.display_config_requested = false;
        state.display_config_request_session_id = None;
        state.last_path_count = 0;
        state.last_changed_paths = 0;
        state.last_active_paths = 0;
        state.swapchain_assigned = false;
        state.first_frame_logged = false;
        state.session_ready_logged = false;
    }

    crate::debug_trace(&format!(
        "SESSION_PROOF_IDD_MONITOR_ARRIVED connector_index={connector_index} os_target_id={os_target_id} session_id={} wddm_enabled={} driver_loaded={} hardware_id={}",
        optional_u32_text(config.session_id),
        config.wddm_idd_enabled,
        config.driver_loaded,
        optional_string_text(config.hardware_id.as_deref())
    ));
    tracing::info!(
        connector_index,
        os_target_id,
        session_id = config.session_id,
        wddm_enabled = config.wddm_idd_enabled,
        driver_loaded = config.driver_loaded,
        hardware_id = ?config.hardware_id,
        "SESSION_PROOF_IDD_MONITOR_ARRIVED"
    );

    STATUS_SUCCESS
}

#[cfg(ironrdp_idd_link)]
pub(crate) fn note_monitor_departure(monitor: IDDCX_MONITOR) {
    let mut state = adapter_runtime_state()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if state.monitor_raw == monitor as usize {
        if state.timer_raw != 0 {
            // SAFETY: The caller has not yet reported monitor departure, so its timer is still live.
            unsafe { crate::wdf::timer_stop(state.timer_raw as crate::wdf::WDFTIMER) };
        }
        state.timer_raw = 0;
        state.monitor_raw = 0;
        state.monitor_arrived = false;
        state.adapter_initialized = false;
    }
}

pub(crate) fn note_swapchain_assignment(session_id: Option<u32>) {
    let config = load_runtime_config();
    let mut state = match adapter_runtime_state().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    state.swapchain_assigned = true;
    maybe_log_session_ready(&config, &mut state);

    crate::debug_trace(&format!(
        "SESSION_PROOF_IDD_SWAPCHAIN_ASSIGNED session_id={} active_paths={}",
        optional_u32_text(session_id),
        state.last_active_paths
    ));
    tracing::info!(
        session_id,
        active_paths = state.last_active_paths,
        "SESSION_PROOF_IDD_SWAPCHAIN_ASSIGNED"
    );
}

pub(crate) fn note_swapchain_unassignment(session_id: Option<u32>) {
    let config = load_runtime_config();
    let mut state = match adapter_runtime_state().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    state.swapchain_assigned = false;
    maybe_log_session_ready(&config, &mut state);

    crate::debug_trace(&format!(
        "SESSION_PROOF_IDD_SWAPCHAIN_UNASSIGNED session_id={} active_paths={}",
        optional_u32_text(session_id),
        state.last_active_paths
    ));
    tracing::info!(
        session_id,
        active_paths = state.last_active_paths,
        "SESSION_PROOF_IDD_SWAPCHAIN_UNASSIGNED"
    );
}

pub(crate) fn note_first_frame(
    session_id: u32,
    presentation_frame_number: u32,
    width: usize,
    height: usize,
    path: &std::path::Path,
) {
    let config = load_runtime_config();
    let mut state = match adapter_runtime_state().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    state.first_frame_logged = true;
    maybe_log_session_ready(&config, &mut state);

    crate::debug_trace(&format!(
        "SESSION_PROOF_IDD_FIRST_FRAME session_id={session_id} presentation_frame_number={presentation_frame_number} width={width} height={height} path={}",
        path.display()
    ));
    tracing::info!(
        session_id,
        presentation_frame_number,
        width,
        height,
        path = %path.display(),
        "SESSION_PROOF_IDD_FIRST_FRAME"
    );
}

pub fn set_display_config(_adapter: IDDCX_ADAPTER, paths: &[crate::adapter::IDDCX_PATH]) -> NTSTATUS {
    let mut changed_paths = 0u32;
    let mut active_paths = 0u32;

    for path in paths {
        if (path.Flags & 1) != 0 {
            changed_paths = changed_paths.saturating_add(1);
        }

        if (path.Flags & 2) != 0 {
            active_paths = active_paths.saturating_add(1);
        }
    }

    let config = load_runtime_config();
    let mut state = match adapter_runtime_state().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    state.last_path_count = paths.len();
    state.last_changed_paths = changed_paths;
    state.last_active_paths = active_paths;

    crate::debug_trace(&format!(
        "SESSION_PROOF_IDD_COMMIT_MODES path_count={} changed_paths={} active_paths={} session_id={} wddm_enabled={} driver_loaded={} active_video_source={}",
        paths.len(),
        changed_paths,
        active_paths,
        optional_u32_text(config.session_id),
        config.wddm_idd_enabled,
        config.driver_loaded,
        optional_string_text(config.active_video_source.as_deref())
    ));
    tracing::info!(
        path_count = paths.len(),
        changed_paths,
        active_paths,
        session_id = config.session_id,
        wddm_enabled = config.wddm_idd_enabled,
        driver_loaded = config.driver_loaded,
        active_video_source = ?config.active_video_source,
        "SESSION_PROOF_IDD_COMMIT_MODES"
    );

    crate::debug_trace(&format!(
        "SESSION_PROOF_IDD_DISPLAY_CONFIG_APPLIED path_count={} changed_paths={} active_paths={} session_id={} wddm_enabled={} driver_loaded={} active_video_source={}",
        paths.len(),
        changed_paths,
        active_paths,
        optional_u32_text(config.session_id),
        config.wddm_idd_enabled,
        config.driver_loaded,
        optional_string_text(config.active_video_source.as_deref())
    ));
    tracing::info!(
        path_count = paths.len(),
        changed_paths,
        active_paths,
        session_id = config.session_id,
        wddm_enabled = config.wddm_idd_enabled,
        driver_loaded = config.driver_loaded,
        active_video_source = ?config.active_video_source,
        "SESSION_PROOF_IDD_DISPLAY_CONFIG_APPLIED"
    );

    maybe_log_session_ready(&config, &mut state);
    STATUS_SUCCESS
}

pub fn handle_session_transition(_adapter: IDDCX_ADAPTER, is_remote: bool) {
    let config = load_runtime_config();

    {
        let mut state = match adapter_runtime_state().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        if state.last_is_remote != is_remote || state.last_session_id != config.session_id {
            crate::debug_trace(&format!(
                "SESSION_PROOF_IDD_REMOTE_SESSION_TRANSITION is_remote={} session_id={} wddm_enabled={} driver_loaded={}",
                is_remote,
                optional_u32_text(config.session_id),
                config.wddm_idd_enabled,
                config.driver_loaded
            ));
            tracing::info!(
                is_remote,
                session_id = config.session_id,
                wddm_enabled = config.wddm_idd_enabled,
                driver_loaded = config.driver_loaded,
                "SESSION_PROOF_IDD_REMOTE_SESSION_TRANSITION"
            );
            state.last_is_remote = is_remote;
            state.last_session_id = config.session_id;
        }

        if !is_remote {
            reset_display_config_state(&config, &mut state, "session_inactive");
        }
    }
}
