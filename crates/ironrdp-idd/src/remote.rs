use crate::{ntstatus_to_u32, IDDCX_ADAPTER, IDDCX_MONITOR, NTSTATUS, STATUS_SUCCESS};
#[cfg(ironrdp_idd_link)]
use core::mem::size_of;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
#[cfg(ironrdp_idd_link)]
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_2DREGION, DISPLAYCONFIG_RATIONAL, DISPLAYCONFIG_ROTATION_IDENTITY,
};
#[cfg(ironrdp_idd_link)]
use windows::Win32::Foundation::POINT;

#[derive(Debug, Clone, Default)]
pub struct RuntimeConfig {
    pub dump_dir: Option<PathBuf>,
    pub session_id: Option<u32>,
    pub wddm_idd_enabled: bool,
    pub driver_loaded: bool,
    pub hardware_id: Option<String>,
    pub active_video_source: Option<String>,
}

#[derive(Debug, Default)]
struct AdapterRuntimeState {
    last_is_remote: bool,
    last_session_id: Option<u32>,
    adapter_initialized: bool,
    monitor_arrived: bool,
    last_monitor_connector_index: Option<u32>,
    last_monitor_os_target_id: Option<u32>,
    display_config_requested: bool,
    display_config_request_session_id: Option<u32>,
    last_path_count: usize,
    last_changed_paths: u32,
    last_active_paths: u32,
    swapchain_assigned: bool,
    first_frame_logged: bool,
    session_ready_logged: bool,
}

static ADAPTER_RUNTIME_STATE: OnceLock<Mutex<AdapterRuntimeState>> = OnceLock::new();
static ACTIVE_ADAPTER_RAW: AtomicUsize = AtomicUsize::new(0);
static REMOTE_STATE_MONITOR_RUNNING: AtomicBool = AtomicBool::new(false);

fn adapter_runtime_state() -> &'static Mutex<AdapterRuntimeState> {
    ADAPTER_RUNTIME_STATE.get_or_init(|| Mutex::new(AdapterRuntimeState::default()))
}

fn optional_u32_text(value: Option<u32>) -> String {
    value.map(|value| value.to_string()).unwrap_or_else(|| "none".to_owned())
}

fn optional_string_text(value: Option<&str>) -> String {
    value.filter(|value| !value.is_empty())
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
                state.last_active_paths,
                state.swapchain_assigned,
                state.first_frame_logged,
                config.driver_loaded,
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

pub fn load_runtime_config() -> RuntimeConfig {
    let Ok(content) = std::fs::read_to_string(crate::IDD_RUNTIME_STATE_FILE) else {
        return RuntimeConfig::default();
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

    RuntimeConfig {
        dump_dir: values
            .get("dump_dir")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from),
        session_id: values
            .get("session_id")
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| *value != 0),
        wddm_idd_enabled: values
            .get("wddm_idd_enabled")
            .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "True")),
        driver_loaded: values
            .get("driver_loaded")
            .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "True")),
        hardware_id: values.get("hardware_id").cloned(),
        active_video_source: values.get("active_video_source").cloned(),
    }
}

pub fn runtime_dump_dir() -> Option<PathBuf> {
    load_runtime_config().dump_dir
}

pub fn runtime_session_id() -> Option<u32> {
    load_runtime_config().session_id
}

pub fn swapchain_dump_runtime() -> Result<RuntimeConfig, String> {
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

fn ensure_remote_state_monitor_thread() {
    if REMOTE_STATE_MONITOR_RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }

    if let Err(error) = std::thread::Builder::new()
        .name("irdp-idd-remote-state".to_owned())
        .spawn(|| loop {
            let adapter_raw = ACTIVE_ADAPTER_RAW.load(Ordering::Acquire);
            if adapter_raw != 0 {
                let config = load_runtime_config();
                let is_remote = config.wddm_idd_enabled && config.session_id.is_some();
                handle_session_transition(adapter_raw as IDDCX_ADAPTER, is_remote);
            }

            std::thread::sleep(Duration::from_millis(250));
        })
    {
        REMOTE_STATE_MONITOR_RUNNING.store(false, Ordering::Release);
        crate::debug_trace(&format!(
            "SESSION_PROOF_IDD_REMOTE_STATE_MONITOR_ERROR stage=spawn error={error}"
        ));
    }
}

#[cfg(not(ironrdp_idd_link))]
fn maybe_request_display_config_update(_adapter: IDDCX_ADAPTER, _source: &str) {}

#[cfg(ironrdp_idd_link)]
fn maybe_request_display_config_update(adapter: IDDCX_ADAPTER, source: &str) {
    ACTIVE_ADAPTER_RAW.store(adapter as usize, Ordering::Release);

    let config = load_runtime_config();
    let Some(session_id) = config.session_id else {
        return;
    };
    if !config.wddm_idd_enabled {
        return;
    }

    let Some((monitor, connector_index, os_target_id)) = crate::monitor::current_monitor() else {
        return;
    };

    {
        let mut state = match adapter_runtime_state().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        if !state.adapter_initialized || !state.monitor_arrived {
            return;
        }

        if state.display_config_request_session_id == Some(session_id) && state.last_active_paths > 0 {
            return;
        }

        state.display_config_requested = true;
        state.display_config_request_session_id = Some(session_id);
    }

    let mut path = crate::iddcx::IDDCX_DISPLAYCONFIGPATH {
        Size: size_of::<crate::iddcx::IDDCX_DISPLAYCONFIGPATH>() as u32,
        MonitorObject: monitor,
        Position: POINT { x: 0, y: 0 },
        Resolution: DISPLAYCONFIG_2DREGION { cx: 1024, cy: 768 },
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
        "SESSION_PROOF_IDD_DISPLAY_CONFIG_UPDATE_REQUEST source={source} session_id={session_id} connector_index={connector_index} os_target_id={os_target_id} resolution={}x{} refresh={}/{} monitor_scale={} v_sync_divider={}",
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

    let status = unsafe { crate::iddcx::adapter_display_config_update(adapter, &mut in_args) };

    {
        let mut state = match adapter_runtime_state().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        if status < 0 {
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

pub fn note_adapter_init_finished(adapter: IDDCX_ADAPTER) {
    ACTIVE_ADAPTER_RAW.store(adapter as usize, Ordering::Release);
    ensure_remote_state_monitor_thread();

    {
        let mut state = match adapter_runtime_state().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.adapter_initialized = true;
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

    maybe_request_display_config_update(adapter, "adapter_init_ready");
}

pub fn note_monitor_arrival(adapter: IDDCX_ADAPTER, monitor: IDDCX_MONITOR, connector_index: u32, os_target_id: u32) {
    let _ = monitor;
    ensure_remote_state_monitor_thread();

    let config = load_runtime_config();
    {
        let mut state = match adapter_runtime_state().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
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

    maybe_request_display_config_update(adapter, "monitor_arrival");
}

pub fn note_swapchain_assignment(session_id: Option<u32>) {
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
    tracing::info!(session_id, active_paths = state.last_active_paths, "SESSION_PROOF_IDD_SWAPCHAIN_ASSIGNED");
}

pub fn note_swapchain_unassignment(session_id: Option<u32>) {
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
    tracing::info!(session_id, active_paths = state.last_active_paths, "SESSION_PROOF_IDD_SWAPCHAIN_UNASSIGNED");
}

pub fn note_first_frame(session_id: u32, presentation_frame_number: u32, width: usize, height: usize, path: &std::path::Path) {
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

pub fn set_display_config(
    _adapter: IDDCX_ADAPTER,
    paths: &[crate::adapter::IDDCX_PATH],
) -> NTSTATUS {
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
    if active_paths == 0 {
        state.display_config_requested = false;
    }

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

pub fn handle_session_transition(adapter: IDDCX_ADAPTER, is_remote: bool) {
    ACTIVE_ADAPTER_RAW.store(adapter as usize, Ordering::Release);

    let config = load_runtime_config();
    let mut should_request_display_config = false;

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
        } else if state.adapter_initialized && state.monitor_arrived {
            should_request_display_config = true;
        }
    }

    if should_request_display_config {
        maybe_request_display_config_update(adapter, "remote_session_transition");
    }
}
