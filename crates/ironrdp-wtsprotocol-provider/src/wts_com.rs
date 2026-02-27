#![expect(
    clippy::as_pointer_underscore,
    clippy::inline_always,
    clippy::multiple_unsafe_ops_per_block
)]

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::fs::OpenOptions;
use std::io::{Read as _, Write};
use std::os::windows::io::AsRawHandle as _;
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::thread;

use ironrdp_pdu::nego;
use ironrdp_wtsprotocol_ipc::{
    default_pipe_name, pipe_path, read_json_message, resolve_pipe_name_from_env, write_json_message, ProviderCommand,
    ServiceEvent, DEFAULT_MAX_FRAME_SIZE,
};
use parking_lot::Mutex;
use tracing::{debug, info, warn};
use windows::core::AgileReference;
use windows::Win32::Foundation::{
    LocalFree, CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, ERROR_BROKEN_PIPE, ERROR_INSUFFICIENT_BUFFER,
    ERROR_IO_INCOMPLETE, ERROR_NO_DATA, ERROR_SEM_TIMEOUT, E_NOINTERFACE, E_NOTIMPL,
    E_POINTER, E_UNEXPECTED,
    HANDLE,
    HANDLE_PTR, HLOCAL,
};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{
    IsValidSecurityDescriptor, LookupAccountNameW, PSECURITY_DESCRIPTOR, PSID, SID_NAME_USE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemAlloc, CoUninitialize, IAgileObject, IAgileObject_Impl, IClassFactory, IClassFactory_Impl,
    COINIT_MULTITHREADED,
};
use windows::Win32::System::Pipes::PeekNamedPipe;
use windows::Win32::System::RemoteDesktop::{
    IWRdsProtocolConnection, IWRdsProtocolConnectionCallback, IWRdsProtocolConnectionSettings_Impl,
    IWRdsProtocolConnection_Impl, IWRdsProtocolLicenseConnection, IWRdsProtocolLicenseConnection_Impl,
    IWRdsProtocolListener, IWRdsProtocolListenerCallback, IWRdsProtocolListener_Impl,
    IWRdsProtocolLogonErrorRedirector, IWRdsProtocolLogonErrorRedirector_Impl, IWRdsProtocolManager,
    IWRdsProtocolManager_Impl, IWRdsProtocolSettings,
    IWRdsProtocolShadowConnection, IWRdsWddmIddProps, IWRdsWddmIddProps_Impl, WTSVirtualChannelClose,
    WTSVirtualChannelOpenEx, WTSVirtualChannelRead, WTSVirtualChannelWrite, WTSEnumerateProcessesW,
    WTSEnumerateSessionsW, WTSFreeMemory, WTSGetActiveConsoleSessionId, WTSQueryUserToken, WTS_PROCESS_INFOW,
    WTS_SESSION_INFOW,
    WRDS_CONNECTION_SETTINGS,
    WRDS_CONNECTION_SETTING_LEVEL, WRDS_LISTENER_SETTINGS, WRDS_LISTENER_SETTING_LEVEL,
    WRDS_LISTENER_SETTINGS_1,
    WRDS_SETTINGS,
    WTS_CERT_TYPE_INVALID, WTS_CHANNEL_OPTION_DYNAMIC, WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH,
    WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW, WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED, WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL,
    WTS_CLIENT_DATA, WTS_KEY_EXCHANGE_ALG_RSA, WTS_LICENSE_CAPABILITIES, WTS_LICENSE_PREAMBLE_VERSION,
    WTS_LOGON_ERROR_REDIRECTOR_RESPONSE, WTS_LOGON_ERR_NOT_HANDLED,
    WTS_PROPERTY_VALUE, WTS_PROTOCOL_STATUS,
    WTS_SERVICE_STATE, WTS_SESSION_ID, WTS_USER_CREDENTIAL,
};
use windows_core::{implement, Interface as _, BOOL, GUID, PCSTR, PCWSTR, PWSTR};
use windows_core::{IUnknown, HRESULT};

use crate::auth_bridge::{CredsspPolicy, CredsspServerBridge};
use crate::connection::ProtocolConnection;
use crate::listener::ProtocolListener;
use crate::manager::ProtocolManager;

const S_OK: HRESULT = HRESULT(0);
const S_FALSE: HRESULT = HRESULT(1);
// HRESULT_FROM_WIN32(ERROR_OUTOFMEMORY=14) == 0x8007000E
const E_OUTOFMEMORY: HRESULT = HRESULT(-2147024882);

const IRONRDP_IDD_HARDWARE_ID: &str = "RdpIdd_IndirectDisplay";
const WTS_PROTOCOL_TYPE_NON_RDP: u16 = 2;

const WTS_VALUE_TYPE_ULONG: u16 = 1;
const WTS_VALUE_TYPE_STRING: u16 = 2;
const WTS_VALUE_TYPE_GUID: u16 = 4;

// From Windows SDK `wtsdefs.h`.
const PROPERTY_TYPE_GET_FAST_RECONNECT: GUID = GUID::from_u128(0x6212_d757_0043_4862_99c3_9f30_59ac_2a3b);
const PROPERTY_TYPE_GET_FAST_RECONNECT_USER_SID: GUID = GUID::from_u128(0x197c_427a_0135_4b6d_9c5e_e657_9a0a_b625);
const PROPERTY_TYPE_CONNECTION_GUID: GUID = GUID::from_u128(0x9eaa_04f6_5b9d_4ba5_be9d_3748_ad6d_8af7);
const PROPERTY_TYPE_SUPPRESS_LOGON_UI: GUID = GUID::from_u128(0x846b_20bb_6254_430e_952f_b0c7_ca08_1915);
const PROPERTY_TYPE_CAPTURE_PROTECTED_CONTENT: GUID = GUID::from_u128(0x2918_db60_6cae_42a8_9945_8128_d7dd_8e71);
const PROPERTY_TYPE_LICENSE_GUID: GUID = GUID::from_u128(0x4daa_5ab8_8b6a_49cf_9c85_8add_504c_d1f7);

// From Windows SDK `wtsdefs.h`.
const WTS_QUERY_AUDIOENUM_DLL: GUID = GUID::from_u128(0x9bf4_fa97_c883_4c2a_80ab_5a39_c9af_00db);
const PROPERTY_TYPE_ENABLE_UNIVERSAL_APPS_FOR_CUSTOM_SHELL: GUID =
    GUID::from_u128(0xed2c_3fda_338d_4d3f_81a3_e767_310d_908e);

const FAST_RECONNECT_ENHANCED: u32 = 2;

fn deterministic_license_guid(connection_id: u32) -> GUID {
    const BASE: u128 = 0x7d5e31f3_0ff8_4a25_9fcb_7b7e2f634000;
    GUID::from_u128(BASE | u128::from(connection_id))
}

fn active_console_session_id() -> Option<u32> {
    // SAFETY: FFI call has no input parameters and always returns a plain value.
    let session_id = unsafe { WTSGetActiveConsoleSessionId() };
    if session_id == u32::MAX {
        None
    } else {
        Some(session_id)
    }
}

fn session_has_user_token(session_id: u32) -> bool {
    let mut token = HANDLE::default();
    // SAFETY: `WTSQueryUserToken` writes a token handle into `token` on success.
    let result = unsafe { WTSQueryUserToken(session_id, &mut token) };
    if result.is_ok() {
        // SAFETY: close token handle returned by `WTSQueryUserToken`.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(token);
        }
        true
    } else {
        false
    }
}

fn session_has_process(session_id: u32, process_name: &str) -> bool {
    let mut process_info_ptr: *mut WTS_PROCESS_INFOW = core::ptr::null_mut();
    let mut process_count = 0u32;

    // SAFETY: WTSEnumerateProcessesW writes a buffer pointer/count pair on success.
    let enumerate_result = unsafe { WTSEnumerateProcessesW(None, 0, 1, &mut process_info_ptr, &mut process_count) };
    if enumerate_result.is_err() || process_info_ptr.is_null() || process_count == 0 {
        return false;
    }

    let mut found = false;

    if let Ok(process_count_usize) = usize::try_from(process_count) {
        // SAFETY: `process_info_ptr` references `process_count_usize` entries on success.
        let processes = unsafe { core::slice::from_raw_parts(process_info_ptr, process_count_usize) };

        for entry in processes {
            if entry.SessionId != session_id {
                continue;
            }

            // SAFETY: pProcessName is a NUL-terminated string owned by the WTS buffer.
            let name = unsafe { PCWSTR(entry.pProcessName.0).to_string() }.unwrap_or_default();
            if name.eq_ignore_ascii_case(process_name) {
                found = true;
                break;
            }
        }
    }

    // SAFETY: free memory allocated by WTSEnumerateProcessesW.
    unsafe {
        WTSFreeMemory(process_info_ptr.cast());
    }

    found
}

fn session_is_interactive(session_id: u32) -> bool {
    session_has_user_token(session_id) && session_has_process(session_id, "explorer.exe")
}

fn canonicalize_enumerated_session_id(session_id: u32) -> u32 {
    // Some hosts expose high-bit encoded values via WTSEnumerateSessionsW (for example 0x0001_0002),
    // while token/process APIs expect the canonical low-word session id (2).
    if session_id > u32::from(u16::MAX) {
        let low_word = session_id & u32::from(u16::MAX);
        if low_word != 0 && low_word != u32::from(u16::MAX) {
            return low_word;
        }

        // Values with an empty or all-ones low word (for example 0x0001_0000) are not
        // valid user sessions for token/process APIs.
        return u32::MAX;
    }

    session_id
}

fn session_selection_snapshot() -> String {
    let console = active_console_session_id()
        .map(|session_id| session_id.to_string())
        .unwrap_or_else(|| "none".to_owned());

    let mut sessions_ptr: *mut WTS_SESSION_INFOW = core::ptr::null_mut();
    let mut session_count = 0u32;

    // SAFETY: WTSEnumerateSessionsW writes a pointer/count pair on success.
    let enumerate_result = unsafe { WTSEnumerateSessionsW(None, 0, 1, &mut sessions_ptr, &mut session_count) };
    if enumerate_result.is_err() || sessions_ptr.is_null() || session_count == 0 {
        return format!("console={console} sessions=none");
    }

    let mut rows = Vec::new();

    if let Ok(session_count_usize) = usize::try_from(session_count) {
        // SAFETY: `sessions_ptr` references `session_count_usize` entries on success.
        let sessions = unsafe { core::slice::from_raw_parts(sessions_ptr, session_count_usize) };

        let mut seen = HashSet::new();
        for entry in sessions {
            let raw = entry.SessionId;
            if raw == u32::MAX {
                continue;
            }

            let canonical = canonicalize_enumerated_session_id(raw);
            if !seen.insert((raw, canonical)) {
                continue;
            }

            let has_token = session_has_user_token(canonical);
            let has_explorer = session_has_process(canonical, "explorer.exe");
            let has_winlogon = session_has_process(canonical, "winlogon.exe");
            let has_logonui = session_has_process(canonical, "LogonUI.exe");

            rows.push(format!(
                "{raw}->{canonical}:token={has_token},explorer={has_explorer},winlogon={has_winlogon},logonui={has_logonui}",
            ));
        }
    }

    // SAFETY: free memory allocated by WTSEnumerateSessionsW.
    unsafe {
        WTSFreeMemory(sessions_ptr.cast());
    }

    if rows.is_empty() {
        format!("console={console} sessions=empty")
    } else {
        format!("console={console} sessions=[{}]", rows.join("; "))
    }
}

fn preferred_interactive_capture_session_id() -> Option<(u32, &'static str)> {
    let console_session = active_console_session_id();

    if let Some(session_id) = console_session {
        if session_has_user_token(session_id) && session_has_process(session_id, "explorer.exe") {
            return Some((session_id, "accept_connection_interactive_console"));
        }
    }

    let mut sessions_ptr: *mut WTS_SESSION_INFOW = core::ptr::null_mut();
    let mut session_count = 0u32;

    // SAFETY: WTSEnumerateSessionsW writes a pointer/count pair on success.
    let enumerate_result = unsafe { WTSEnumerateSessionsW(None, 0, 1, &mut sessions_ptr, &mut session_count) };

    if enumerate_result.is_ok() && !sessions_ptr.is_null() && session_count > 0 {
        if let Ok(session_count_usize) = usize::try_from(session_count) {
            // SAFETY: `sessions_ptr` references `session_count_usize` entries on success.
            let sessions = unsafe { core::slice::from_raw_parts(sessions_ptr, session_count_usize) };
            let mut candidates: Vec<u32> = sessions
                .iter()
                .map(|session| canonicalize_enumerated_session_id(session.SessionId))
                .collect();
            candidates.sort_unstable();
            candidates.dedup();

            for &session_id in &candidates {
                if session_id == u32::MAX {
                    continue;
                }

                if session_has_user_token(session_id) && session_has_process(session_id, "explorer.exe") {
                    // SAFETY: free memory allocated by WTSEnumerateSessionsW.
                    unsafe {
                        WTSFreeMemory(sessions_ptr.cast());
                    }
                    return Some((session_id, "accept_connection_interactive_enumerated"));
                }
            }
        }

        // SAFETY: free memory allocated by WTSEnumerateSessionsW.
        unsafe {
            WTSFreeMemory(sessions_ptr.cast());
        }
    }

    None
}

fn preferred_capture_session_id() -> Option<(u32, &'static str)> {
    let console_session = active_console_session_id();

    if let Some(session_id) = console_session {
        if session_has_user_token(session_id) {
            let source = if session_has_process(session_id, "explorer.exe") {
                "accept_connection_console_interactive"
            } else {
                "accept_connection_console_token"
            };

            return Some((session_id, source));
        }
    }

    let mut sessions_ptr: *mut WTS_SESSION_INFOW = core::ptr::null_mut();
    let mut session_count = 0u32;

    // SAFETY: WTSEnumerateSessionsW writes a pointer/count pair on success.
    let enumerate_result = unsafe { WTSEnumerateSessionsW(None, 0, 1, &mut sessions_ptr, &mut session_count) };

    if enumerate_result.is_ok() && !sessions_ptr.is_null() && session_count > 0 {
        if let Ok(session_count_usize) = usize::try_from(session_count) {
            // SAFETY: `sessions_ptr` references `session_count_usize` entries on success.
            let sessions = unsafe { core::slice::from_raw_parts(sessions_ptr, session_count_usize) };
            let mut candidates: Vec<u32> = sessions
                .iter()
                .map(|session| canonicalize_enumerated_session_id(session.SessionId))
                .collect();
            candidates.sort_unstable();
            candidates.dedup();

            for &session_id in &candidates {
                if session_id == u32::MAX || !session_has_user_token(session_id) {
                    continue;
                }

                let source = if session_has_process(session_id, "explorer.exe") {
                    "accept_connection_enumerated_interactive"
                } else {
                    "accept_connection_enumerated_token"
                };

                // SAFETY: free memory allocated by WTSEnumerateSessionsW.
                unsafe {
                    WTSFreeMemory(sessions_ptr.cast());
                }
                return Some((session_id, source));
            }
        }

        // SAFETY: free memory allocated by WTSEnumerateSessionsW.
        unsafe {
            WTSFreeMemory(sessions_ptr.cast());
        }
    }

    if let Some(session_id) = console_session {
        if session_id != 0 && session_id != u32::MAX {
            return Some((session_id, "accept_connection_console_fallback"));
        }
    }

    let mut sessions_ptr: *mut WTS_SESSION_INFOW = core::ptr::null_mut();
    let mut session_count = 0u32;

    // SAFETY: WTSEnumerateSessionsW writes a pointer/count pair on success.
    let enumerate_result = unsafe { WTSEnumerateSessionsW(None, 0, 1, &mut sessions_ptr, &mut session_count) };
    if enumerate_result.is_ok() && !sessions_ptr.is_null() && session_count > 0 {
        if let Ok(session_count_usize) = usize::try_from(session_count) {
            // SAFETY: `sessions_ptr` references `session_count_usize` entries on success.
            let sessions = unsafe { core::slice::from_raw_parts(sessions_ptr, session_count_usize) };
            let mut candidates: Vec<u32> = sessions
                .iter()
                .map(|session| canonicalize_enumerated_session_id(session.SessionId))
                .filter(|session_id| *session_id != 0 && *session_id != u32::MAX)
                .collect();
            candidates.sort_unstable();
            candidates.dedup();

            if let Some(session_id) = candidates.first().copied() {
                // SAFETY: free memory allocated by WTSEnumerateSessionsW.
                unsafe {
                    WTSFreeMemory(sessions_ptr.cast());
                }
                return Some((session_id, "accept_connection_enumerated_fallback"));
            }
        }

        // SAFETY: free memory allocated by WTSEnumerateSessionsW.
        unsafe {
            WTSFreeMemory(sessions_ptr.cast());
        }
    }

    None
}

fn lookup_account_sid_string(username: &str, domain: &str) -> windows_core::Result<String> {
    let username = username.trim();
    let domain = domain.trim();

    let account = if domain.is_empty() {
        username.to_owned()
    } else if username.contains('@') {
        // Already a UPN.
        username.to_owned()
    } else if domain.contains('.') {
        // AD often doesn't resolve DNS-style domains as down-level `DOMAIN\\user`.
        // Use UPN to make LookupAccountNameW succeed without relying on NetBIOS mapping.
        format!("{username}@{domain}")
    } else {
        format!("{domain}\\{username}")
    };

    let account_w = windows_core::HSTRING::from(account);

    let mut sid_size = 0u32;
    let mut domain_size = 0u32;
    let mut use_type = SID_NAME_USE(0);

    // First call to obtain required buffer sizes.
    // SAFETY: sizes/use_type are valid out-params.
    unsafe {
        let _ = LookupAccountNameW(
            None,
            PCWSTR(account_w.as_ptr()),
            None,
            &mut sid_size,
            None,
            &mut domain_size,
            &mut use_type,
        );
    }

    if sid_size == 0 {
        return Err(windows_core::Error::new(
            E_UNEXPECTED,
            "LookupAccountNameW returned zero SID size",
        ));
    }

    let sid_len = usize::try_from(sid_size)
        .map_err(|_| windows_core::Error::new(E_UNEXPECTED, "SID size does not fit in usize"))?;
    let domain_len = usize::try_from(domain_size)
        .map_err(|_| windows_core::Error::new(E_UNEXPECTED, "domain size does not fit in usize"))?;

    let mut sid_buf = vec![0u8; sid_len];
    let mut domain_buf = vec![0u16; domain_len];

    // SAFETY: buffers are allocated according to `sid_size` and `domain_size` returned above.
    unsafe {
        LookupAccountNameW(
            None,
            PCWSTR(account_w.as_ptr()),
            Some(PSID(sid_buf.as_mut_ptr().cast())),
            &mut sid_size,
            Some(PWSTR(domain_buf.as_mut_ptr())),
            &mut domain_size,
            &mut use_type,
        )?;
    }

    let mut sid_string = PWSTR::null();
    // SAFETY: `sid_buf` contains a valid SID and `sid_string` is a valid out-param.
    unsafe { ConvertSidToStringSidW(PSID(sid_buf.as_mut_ptr().cast()), &mut sid_string)? };

    // SAFETY: `sid_string` is NUL-terminated on success.
    let sid = unsafe { sid_string.to_string() }.map_err(|error| {
        windows_core::Error::new(
            E_UNEXPECTED,
            format!("failed to decode SID string returned by ConvertSidToStringSidW: {error}"),
        )
    })?;

    // SAFETY: ConvertSidToStringSidW allocates with LocalAlloc; free with LocalFree.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(sid_string.0.cast())));
    }

    Ok(sid)
}

const DEBUG_LOG_PATH_ENV: &str = "IRONRDP_WTS_PROVIDER_DEBUG_LOG";

fn normalize_winlogon_credentials(username: &str, domain: &str) -> (String, String) {
    let username = username.trim();
    let domain = domain.trim();

    if domain.is_empty() {
        if let Some((dom, user)) = username.split_once('\\') {
            let user = user.trim();
            let dom = dom.trim();
            if !user.is_empty() {
                return (user.to_owned(), dom.to_owned());
            }
        }

        if let Some((user, upn_domain)) = username.split_once('@') {
            let user = user.trim();
            let upn_domain = upn_domain.trim();
            if !user.is_empty() && !upn_domain.is_empty() {
                return (user.to_owned(), upn_domain.to_owned());
            }
        }
    }

    if let Some((user, upn_domain)) = username.split_once('@') {
        let user = user.trim();
        let upn_domain = upn_domain.trim();
        if !user.is_empty() {
            let domain_to_use = if domain.is_empty() { upn_domain } else { domain };
            if !domain_to_use.is_empty() {
                return (user.to_owned(), domain_to_use.to_owned());
            }
        }
    }

    (username.to_owned(), domain.to_owned())
}

fn debug_log_line(message: &str) {
    let Some(path) = std::env::var(DEBUG_LOG_PATH_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return;
    };

    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();

    let line = format!("{timestamp_ms} {message}\n");

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}

fn command_kind(command: &ProviderCommand) -> &'static str {
    match command {
        ProviderCommand::StartListen { .. } => "start_listen",
        ProviderCommand::StopListen { .. } => "stop_listen",
        ProviderCommand::WaitForIncoming { .. } => "wait_for_incoming",
        ProviderCommand::AcceptConnection { .. } => "accept_connection",
        ProviderCommand::CloseConnection { .. } => "close_connection",
        ProviderCommand::GetConnectionCredentials { .. } => "get_connection_credentials",
        ProviderCommand::SetCaptureSessionId { .. } => "set_capture_session_id",
        ProviderCommand::NotifyIddDriverLoaded { .. } => "notify_idd_driver_loaded",
    }
}

fn event_kind(event: &ServiceEvent) -> &'static str {
    match event {
        ServiceEvent::Ack => "ack",
        ServiceEvent::ListenerStarted { .. } => "listener_started",
        ServiceEvent::ListenerStopped { .. } => "listener_stopped",
        ServiceEvent::IncomingConnection { .. } => "incoming_connection",
        ServiceEvent::NoIncoming => "no_incoming",
        ServiceEvent::ConnectionReady { .. } => "connection_ready",
        ServiceEvent::ConnectionBroken { .. } => "connection_broken",
        ServiceEvent::ConnectionCredentials { .. } => "connection_credentials",
        ServiceEvent::NoCredentials { .. } => "no_credentials",
        ServiceEvent::Error { .. } => "error",
    }
}

pub const IRONRDP_PROTOCOL_MANAGER_CLSID: GUID = GUID::from_u128(0x89c7ed1e_25e5_4b15_8f52_ae6df4a5ceaf);
pub const IRONRDP_PROTOCOL_MANAGER_CLSID_STR: &str = "{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}";

static SERVER_LOCK_COUNT: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_OBJECT_COUNT: AtomicUsize = AtomicUsize::new(0);

const IRONRDP_CLIPRDR_CHANNEL_NAME: &str = "cliprdr";
const IRONRDP_RDPSND_CHANNEL_NAME: &str = "rdpsnd";
const IRONRDP_DRDYNVC_CHANNEL_NAME: &str = "drdynvc";
const IRONRDP_DISPLAYCONTROL_CHANNEL_NAME: &str = "Microsoft::Windows::RDS::DisplayControl";
const IRONRDP_GRAPHICS_CHANNEL_NAME: &str = "Microsoft::Windows::RDS::Graphics";
const IRONRDP_AINPUT_CHANNEL_NAME: &str = "FreeRDP::Advanced::Input";
const IRONRDP_ECHO_CHANNEL_NAME: &str = "ECHO";
const VIRTUAL_CHANNEL_FORWARDER_READ_TIMEOUT_MS: u32 = 100;
const VIRTUAL_CHANNEL_FORWARDER_BUFFER_SIZE: usize = 64 * 1024;
const VIRTUAL_CHANNEL_FORWARDER_OUTBOUND_QUEUE_SIZE: usize = 100;
const VIRTUAL_CHANNEL_PIPE_BRIDGE_ENV: &str = "IRONRDP_WTS_VC_BRIDGE_PIPE_PREFIX";
const VIRTUAL_CHANNEL_PIPE_BRIDGE_QUEUE_SIZE: usize = 200;
const VIRTUAL_CHANNEL_PIPE_BRIDGE_RECONNECT_DELAY: Duration = Duration::from_millis(500);
const VIRTUAL_CHANNEL_PIPE_BRIDGE_SEND_TIMEOUT: Duration = Duration::from_millis(100);
const VIRTUAL_CHANNEL_PIPE_BRIDGE_MAX_FRAME_SIZE: usize = 1024 * 1024;

type SharedVirtualChannelBridgeHandler = Arc<dyn VirtualChannelBridgeHandler>;

static VIRTUAL_CHANNEL_BRIDGE_HANDLER: OnceLock<Mutex<Option<SharedVirtualChannelBridgeHandler>>> = OnceLock::new();

struct ComInitGuard {
    initialized: bool,
}

impl ComInitGuard {
    fn new() -> Self {
        // SAFETY: COM initialization is per-thread.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        match hr.ok() {
            Ok(()) => Self { initialized: true },
            Err(error) => {
                debug_log_line(&format!("CoInitializeEx failed in StartListen path: {error}"));
                Self { initialized: false }
            }
        }
    }
}

impl Drop for ComInitGuard {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: paired with a successful CoInitializeEx in this guard.
            unsafe { CoUninitialize() };
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualChannelRouteKind {
    Unknown,
    IronRdpStatic,
    IronRdpDynamicBackbone,
    IronRdpDynamicEndpoint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualChannelBridgeEndpoint {
    pub endpoint_name: String,
    pub static_channel: bool,
    pub route_kind: VirtualChannelRouteKind,
}

#[derive(Clone)]
pub struct VirtualChannelBridgeTx {
    endpoint: VirtualChannelBridgeEndpoint,
    outbound_tx: mpsc::SyncSender<Vec<u8>>,
}

impl VirtualChannelBridgeTx {
    pub fn endpoint(&self) -> &VirtualChannelBridgeEndpoint {
        &self.endpoint
    }

    pub fn send(&self, payload: Vec<u8>) -> windows_core::Result<()> {
        self.outbound_tx
            .send(payload)
            .map_err(|_| windows_core::Error::new(E_UNEXPECTED, "virtual channel bridge sender is closed"))
    }
}

pub trait VirtualChannelBridgeHandler: Send + Sync {
    fn on_channel_opened(&self, endpoint: &VirtualChannelBridgeEndpoint, tx: VirtualChannelBridgeTx);

    fn on_channel_data(&self, endpoint: &VirtualChannelBridgeEndpoint, data: &[u8]);

    fn on_channel_closed(&self, endpoint: &VirtualChannelBridgeEndpoint);
}

pub fn set_virtual_channel_bridge_handler(handler: Option<Arc<dyn VirtualChannelBridgeHandler>>) {
    *virtual_channel_bridge_handler_slot().lock() = handler;
}

fn virtual_channel_bridge_handler_slot() -> &'static Mutex<Option<SharedVirtualChannelBridgeHandler>> {
    VIRTUAL_CHANNEL_BRIDGE_HANDLER.get_or_init(|| Mutex::new(None))
}

fn get_virtual_channel_bridge_handler() -> Option<SharedVirtualChannelBridgeHandler> {
    virtual_channel_bridge_handler_slot().lock().clone()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IronRdpVirtualChannelServer {
    Cliprdr,
    Rdpsnd,
    Drdynvc,
    DisplayControl,
    Graphics,
    AdvancedInput,
    Echo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VirtualChannelBridgePlan {
    route_kind: VirtualChannelRouteKind,
    hook_target: Option<IronRdpVirtualChannelServer>,
    preferred_dynamic_priority: Option<u32>,
}

impl VirtualChannelBridgePlan {
    fn for_endpoint(is_static: bool, hook_target: Option<IronRdpVirtualChannelServer>) -> Self {
        let route_kind = match hook_target {
            Some(IronRdpVirtualChannelServer::Cliprdr) | Some(IronRdpVirtualChannelServer::Rdpsnd) => {
                VirtualChannelRouteKind::IronRdpStatic
            }
            Some(IronRdpVirtualChannelServer::Drdynvc) => VirtualChannelRouteKind::IronRdpDynamicBackbone,
            Some(IronRdpVirtualChannelServer::DisplayControl)
            | Some(IronRdpVirtualChannelServer::Graphics)
            | Some(IronRdpVirtualChannelServer::AdvancedInput)
            | Some(IronRdpVirtualChannelServer::Echo) => VirtualChannelRouteKind::IronRdpDynamicEndpoint,
            None => VirtualChannelRouteKind::Unknown,
        };

        let preferred_dynamic_priority = if is_static {
            None
        } else {
            hook_target.map(IronRdpVirtualChannelServer::default_dynamic_priority)
        };

        Self {
            route_kind,
            hook_target,
            preferred_dynamic_priority,
        }
    }

    fn should_prepare_forwarding(self) -> bool {
        self.route_kind != VirtualChannelRouteKind::Unknown
    }
}

impl IronRdpVirtualChannelServer {
    fn name(self) -> &'static str {
        match self {
            Self::Cliprdr => IRONRDP_CLIPRDR_CHANNEL_NAME,
            Self::Rdpsnd => IRONRDP_RDPSND_CHANNEL_NAME,
            Self::Drdynvc => IRONRDP_DRDYNVC_CHANNEL_NAME,
            Self::DisplayControl => IRONRDP_DISPLAYCONTROL_CHANNEL_NAME,
            Self::Graphics => IRONRDP_GRAPHICS_CHANNEL_NAME,
            Self::AdvancedInput => IRONRDP_AINPUT_CHANNEL_NAME,
            Self::Echo => IRONRDP_ECHO_CHANNEL_NAME,
        }
    }

    fn requires_drdynvc_backbone(self) -> bool {
        matches!(
            self,
            Self::DisplayControl | Self::Graphics | Self::AdvancedInput | Self::Echo
        )
    }

    fn default_dynamic_priority(self) -> u32 {
        match self {
            Self::AdvancedInput => WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL,
            Self::Graphics => WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH,
            Self::DisplayControl => WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED,
            Self::Echo => WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW,
            Self::Cliprdr | Self::Rdpsnd | Self::Drdynvc => WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW,
        }
    }
}

fn endpoint_name_eq(lhs: &str, rhs: &str) -> bool {
    lhs.eq_ignore_ascii_case(rhs)
}

fn ironrdp_virtual_channel_server(endpoint_name: &str, is_static: bool) -> Option<IronRdpVirtualChannelServer> {
    if is_static {
        if endpoint_name_eq(endpoint_name, IRONRDP_CLIPRDR_CHANNEL_NAME) {
            return Some(IronRdpVirtualChannelServer::Cliprdr);
        }
        if endpoint_name_eq(endpoint_name, IRONRDP_RDPSND_CHANNEL_NAME) {
            return Some(IronRdpVirtualChannelServer::Rdpsnd);
        }
        if endpoint_name_eq(endpoint_name, IRONRDP_DRDYNVC_CHANNEL_NAME) {
            return Some(IronRdpVirtualChannelServer::Drdynvc);
        }

        return None;
    }

    if endpoint_name_eq(endpoint_name, IRONRDP_DISPLAYCONTROL_CHANNEL_NAME) {
        return Some(IronRdpVirtualChannelServer::DisplayControl);
    }
    if endpoint_name_eq(endpoint_name, IRONRDP_GRAPHICS_CHANNEL_NAME) {
        return Some(IronRdpVirtualChannelServer::Graphics);
    }
    if endpoint_name_eq(endpoint_name, IRONRDP_AINPUT_CHANNEL_NAME) {
        return Some(IronRdpVirtualChannelServer::AdvancedInput);
    }
    if endpoint_name_eq(endpoint_name, IRONRDP_ECHO_CHANNEL_NAME) {
        return Some(IronRdpVirtualChannelServer::Echo);
    }

    None
}

fn is_dynamic_channel_priority(value: u32) -> bool {
    matches!(
        value,
        WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
            | WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED
            | WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH
            | WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL
    )
}

fn virtual_channel_requested_priority(
    is_static: bool,
    requested_priority: u32,
    hook_target: Option<IronRdpVirtualChannelServer>,
) -> u32 {
    if is_static {
        return requested_priority;
    }

    if requested_priority == WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW {
        return hook_target
            .map(IronRdpVirtualChannelServer::default_dynamic_priority)
            .unwrap_or(requested_priority);
    }

    if is_dynamic_channel_priority(requested_priority) {
        return requested_priority;
    }

    hook_target
        .map(IronRdpVirtualChannelServer::default_dynamic_priority)
        .unwrap_or(requested_priority)
}

#[derive(Debug)]
struct ListenerWorker {
    stop_tx: mpsc::Sender<()>,
    join_handle: thread::JoinHandle<()>,
}

#[derive(Debug, Clone)]
struct ProviderControlBridge {
    pipe_name: Option<String>,
    optional_connection: bool,
}

#[derive(Debug, Clone)]
struct IncomingConnection {
    connection_id: u32,
    peer_addr: Option<String>,
}

enum WaitForIncomingEvent {
    Incoming(IncomingConnection),
    ConnectionBroken { connection_id: u32, reason: String },
}

impl ProviderControlBridge {
    fn from_env() -> Self {
        if let Some(pipe_name) = resolve_pipe_name_from_env() {
            return Self {
                pipe_name: Some(pipe_name),
                optional_connection: false,
            };
        }

        Self {
            pipe_name: Some(default_pipe_name()),
            optional_connection: true,
        }
    }

    fn start_listen(&self, listener_name: &str) -> windows_core::Result<bool> {
        // Use retries: if the pipe is momentarily busy (e.g., occupied by an orphaned worker
        // thread from a previous TermService session still polling WaitForIncoming), we want
        // to keep retrying rather than returning an error that causes a "Catastrophic failure".
        let Some(event) = self.send_command_retried(&ProviderCommand::StartListen {
            listener_name: listener_name.to_owned(),
        })?
        else {
            // When running under TermService, the companion service is typically started via a
            // Scheduled Task. On boot, TermService can call StartListen before that task has
            // created the named pipe. If we return "disabled" here, we never enter the
            // WaitForIncoming polling loop, so the companion never auto-starts its TCP listener.
            //
            // Treat the bridge as enabled and let the worker thread keep polling; once the pipe
            // becomes available, WaitForIncoming will auto-start the TCP listener.
            debug_log_line(&format!(
                "ProviderControlBridge::start_listen control pipe not available yet; enabling deferred StartListen listener_name={listener_name}",
            ));
            return Ok(true);
        };

        match event {
            ServiceEvent::ListenerStarted {
                listener_name: started_listener,
            } if started_listener == listener_name => Ok(true),
            ServiceEvent::Ack => Ok(true),
            ServiceEvent::Error { message } => Err(windows_core::Error::new(E_UNEXPECTED, message)),
            other => Err(windows_core::Error::new(
                E_UNEXPECTED,
                format!("unexpected service event on start listen: {other:?}"),
            )),
        }
    }

    fn stop_listen(&self, listener_name: &str) -> windows_core::Result<()> {
        let Some(event) = self.send_command(&ProviderCommand::StopListen {
            listener_name: listener_name.to_owned(),
        })?
        else {
            return Ok(());
        };

        match event {
            ServiceEvent::ListenerStopped {
                listener_name: stopped_listener,
            } if stopped_listener == listener_name => Ok(()),
            ServiceEvent::Ack => Ok(()),
            ServiceEvent::Error { message } => Err(windows_core::Error::new(E_UNEXPECTED, message)),
            other => Err(windows_core::Error::new(
                E_UNEXPECTED,
                format!("unexpected service event on stop listen: {other:?}"),
            )),
        }
    }

    fn set_capture_session_id_retried(
        &self,
        connection_id: u32,
        session_id: u32,
        source: &'static str,
    ) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_SEND source={source} connection_id={connection_id} session_id={session_id}",
        ));

        let Some(event) = self.send_command_retried(&ProviderCommand::SetCaptureSessionId {
            connection_id,
            session_id,
        })?
        else {
            debug_log_line(&format!(
                "SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_PIPE_UNAVAILABLE source={source} connection_id={connection_id} session_id={session_id}",
            ));
            return Ok(());
        };

        match event {
            ServiceEvent::Ack => {
                debug_log_line(&format!(
                    "SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_ACK source={source} connection_id={connection_id} session_id={session_id}",
                ));
                Ok(())
            }
            ServiceEvent::Error { message } => {
                debug_log_line(&format!(
                    "SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_ERROR source={source} connection_id={connection_id} session_id={session_id} message={message}",
                ));
                Err(windows_core::Error::new(E_UNEXPECTED, message))
            }
            other => {
                debug_log_line(&format!(
                    "SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_UNEXPECTED source={source} connection_id={connection_id} session_id={session_id} event={other:?}",
                ));
                Err(windows_core::Error::new(
                    E_UNEXPECTED,
                    format!("unexpected service event on set capture session id (retried): {other:?}"),
                ))
            }
        }
    }

    fn notify_idd_driver_loaded(&self, session_id: u32) -> windows_core::Result<()> {
        let Some(event) = self.send_command(&ProviderCommand::NotifyIddDriverLoaded { session_id })? else {
            return Ok(());
        };

        match event {
            ServiceEvent::Ack => Ok(()),
            ServiceEvent::Error { message } => Err(windows_core::Error::new(E_UNEXPECTED, message)),
            _ => Ok(()),
        }
    }

    fn accept_connection(&self, connection_id: u32) -> windows_core::Result<()> {
        let Some(event) = self.send_command_retried(&ProviderCommand::AcceptConnection { connection_id })? else {
            return Ok(());
        };

        match event {
            ServiceEvent::ConnectionReady {
                connection_id: ready_connection_id,
            } if ready_connection_id == connection_id => Ok(()),
            ServiceEvent::Ack => Ok(()),
            ServiceEvent::Error { message } => {
                debug_log_line(&format!(
                    "accept_connection error connection_id={connection_id} message={message}"
                ));
                Err(windows_core::Error::new(E_UNEXPECTED, message))
            }
            other => {
                debug_log_line(&format!(
                    "accept_connection unexpected_event connection_id={connection_id} event={other:?}"
                ));
                Err(windows_core::Error::new(
                    E_UNEXPECTED,
                    format!("unexpected service event on accept connection: {other:?}"),
                ))
            }
        }
    }

    fn close_connection(&self, connection_id: u32) -> windows_core::Result<()> {
        let Some(event) = self.send_command_retried(&ProviderCommand::CloseConnection { connection_id })? else {
            return Ok(());
        };

        match event {
            ServiceEvent::Ack => Ok(()),
            ServiceEvent::Error { message } => Err(windows_core::Error::new(E_UNEXPECTED, message)),
            _ => Ok(()),
        }
    }

    /// Retrieve the CredSSP-derived plaintext credentials for a connection from the companion service.
    ///
    /// Polls up to ~5 seconds waiting for the CredSSP handshake to complete.
    /// Tolerates transient pipe I/O errors (the companion service may restart its pipe server).
    /// Returns `(username, domain, password)` on success, or `None` when no credentials are
    /// available after the timeout.
    fn get_connection_credentials(&self, connection_id: u32) -> windows_core::Result<Option<(String, String, String)>> {
        let mut last_pipe_error: Option<windows_core::Error> = None;

        for poll in 0..20 {
            if poll > 0 {
                thread::sleep(Duration::from_millis(250));
            }

            let event = match self.send_command_retried(&ProviderCommand::GetConnectionCredentials { connection_id }) {
                Ok(Some(event)) => event,
                Ok(None) => {
                    // Pipe transiently unavailable (companion pipe server between iterations).
                    // Treat this like NoCredentials: continue polling rather than giving up.
                    continue;
                }
                Err(error) => {
                    if poll == 0 {
                        debug_log_line(&format!(
                            "get_connection_credentials pipe error connection_id={connection_id}; retrying... error={error}"
                        ));
                    }
                    last_pipe_error = Some(error);
                    continue;
                }
            };

            match event {
                ServiceEvent::ConnectionCredentials {
                    connection_id: cid,
                    username,
                    domain,
                    password,
                } if cid == connection_id => {
                    debug_log_line(&format!(
                        "get_connection_credentials ok connection_id={connection_id} poll={poll} username={username} domain={domain}"
                    ));
                    return Ok(Some((username, domain, password)));
                }
                ServiceEvent::NoCredentials { .. } => {
                    if poll == 0 {
                        debug_log_line(&format!(
                            "get_connection_credentials no_credentials yet connection_id={connection_id}; polling..."
                        ));
                    }
                    continue;
                }
                ServiceEvent::Error { message } => {
                    debug_log_line(&format!(
                        "get_connection_credentials service_error connection_id={connection_id} message={message}"
                    ));
                    last_pipe_error = Some(windows_core::Error::new(E_UNEXPECTED, message));
                    continue;
                }
                other => {
                    warn!(
                        connection_id,
                        ?other,
                        "Unexpected service event on get_connection_credentials"
                    );
                    return Ok(None);
                }
            }
        }

        debug_log_line(&format!(
            "get_connection_credentials timed out connection_id={connection_id} last_error={last_pipe_error:?}"
        ));
        Ok(None)
    }

    fn wait_for_incoming(
        &self,
        listener_name: &str,
        timeout_ms: u32,
    ) -> windows_core::Result<Option<WaitForIncomingEvent>> {
        let Some(event) = self.send_command_retried(&ProviderCommand::WaitForIncoming {
            listener_name: listener_name.to_owned(),
            timeout_ms,
        })?
        else {
            return Ok(None);
        };

        match event {
            ServiceEvent::IncomingConnection {
                listener_name: service_listener_name,
                connection_id,
                peer_addr,
            } => {
                if service_listener_name != listener_name {
                    return Err(windows_core::Error::new(
                        E_UNEXPECTED,
                        format!(
                            "incoming connection listener mismatch: expected {listener_name} got {service_listener_name}"
                        ),
                    ));
                }

                Ok(Some(WaitForIncomingEvent::Incoming(IncomingConnection {
                    connection_id,
                    peer_addr,
                })))
            }
            ServiceEvent::ConnectionBroken { connection_id, reason } => {
                Ok(Some(WaitForIncomingEvent::ConnectionBroken { connection_id, reason }))
            }
            ServiceEvent::NoIncoming | ServiceEvent::Ack => Ok(None),
            ServiceEvent::Error { message } => Err(windows_core::Error::new(E_UNEXPECTED, message)),
            other => Err(windows_core::Error::new(
                E_UNEXPECTED,
                format!("unexpected service event on wait incoming: {other:?}"),
            )),
        }
    }

    fn send_command(&self, command: &ProviderCommand) -> windows_core::Result<Option<ServiceEvent>> {
        self.send_command_with_retries(command, 1)
    }

    fn send_command_retried(&self, command: &ProviderCommand) -> windows_core::Result<Option<ServiceEvent>> {
        self.send_command_with_retries(command, 10)
    }

    fn send_command_with_retries(
        &self,
        command: &ProviderCommand,
        max_attempts: u32,
    ) -> windows_core::Result<Option<ServiceEvent>> {
        let Some(pipe_name) = self.pipe_name.as_ref() else {
            return Ok(None);
        };

        let full_pipe_name = pipe_path(pipe_name);

        let mut last_error = None;
        for attempt in 0..max_attempts {
            if attempt > 0 {
                thread::sleep(Duration::from_millis(50 + u64::from(attempt) * 30));
            }

            debug_log_line(&format!(
                "ProviderControlBridge::send_command open pipe={} command={}{}",
                full_pipe_name,
                command_kind(command),
                if attempt > 0 {
                    format!(" retry={attempt}")
                } else {
                    String::new()
                }
            ));

            let pipe_result = OpenOptions::new().read(true).write(true).open(&full_pipe_name);

            let mut pipe = match pipe_result {
                Ok(pipe) => pipe,
                Err(error)
                    if is_optional_control_pipe_error(&error)
                        && (self.optional_connection
                            || matches!(
                                command,
                                ProviderCommand::StartListen { .. }
                                    | ProviderCommand::WaitForIncoming { .. }
                                    | ProviderCommand::NotifyIddDriverLoaded { .. }
                            )) =>
                {
                    debug!(
                        %error,
                        pipe = %full_pipe_name,
                        command = command_kind(command),
                        "Companion control pipe not available yet; deferring provider IPC"
                    );
                    return Ok(None);
                }
                Err(error) if error.raw_os_error() == Some(231) && attempt + 1 < max_attempts => {
                    debug_log_line(&format!(
                        "ProviderControlBridge::send_command pipe_busy pipe={} command={} attempt={}",
                        full_pipe_name,
                        command_kind(command),
                        attempt
                    ));
                    last_error = Some(error);
                    continue;
                }
                Err(error) => {
                    debug_log_line(&format!(
                        "ProviderControlBridge::send_command open_failed pipe={} command={} error={}",
                        full_pipe_name,
                        command_kind(command),
                        error
                    ));
                    return Err(io_error_to_windows_error(error, "failed to connect to control pipe"));
                }
            };

            write_json_message(&mut pipe, command)
                .map_err(|error| io_error_to_windows_error(error, "failed to send control command"))?;

            let event = read_json_message::<ServiceEvent>(&mut pipe, DEFAULT_MAX_FRAME_SIZE)
                .map_err(|error| io_error_to_windows_error(error, "failed to read control response"))?;

            debug_log_line(&format!(
                "ProviderControlBridge::send_command ok command={} event={}",
                command_kind(command),
                event_kind(&event)
            ));

            return Ok(Some(event));
        }

        let error = last_error.unwrap_or_else(|| std::io::Error::other("max retries exceeded"));
        Err(io_error_to_windows_error(
            error,
            "failed to connect to control pipe after retries",
        ))
    }
}

fn is_optional_control_pipe_error(error: &std::io::Error) -> bool {
    use std::io::ErrorKind;

    matches!(
        error.kind(),
        ErrorKind::NotFound
            | ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionReset
            | ErrorKind::TimedOut
            | ErrorKind::BrokenPipe
            | ErrorKind::WouldBlock
    )
}

fn default_connection_settings(listener_name: &str) -> WRDS_CONNECTION_SETTINGS {
    // SAFETY: `WRDS_CONNECTION_SETTINGS` is a plain C struct; a zeroed value is a valid baseline.
    let mut settings: WRDS_CONNECTION_SETTINGS = unsafe { core::mem::zeroed() };
    settings.WRdsConnectionSettingLevel = WRDS_CONNECTION_SETTING_LEVEL(1);
    // SAFETY: WRDS_CONNECTION_SETTINGS contains a union; accessing the level-1 view is valid because we set the level.
    unsafe {
        // A/B mode: advertise a non-RDP protocol type.
        // WTS_PROTOCOL_TYPE_* values in the Windows SDK (WtsApi32.h):
        //   CONSOLE = 0, ICA = 1, RDP = 2
        let s1 = &mut settings.WRdsConnectionSetting.WRdsConnectionSettings1;
        s1.ProtocolType = WTS_PROTOCOL_TYPE_NON_RDP;
        copy_wide(&mut s1.ProtocolName, "RDP");

        // Do not force auto-logon here: TermService may query GetClientData/GetUserCredentials
        // before CredSSP credentials are available, and advertising "saved creds" with empty
        // fields can lead to immediate logon failure/disconnect.
        s1.fInheritAutoLogon = false;
        s1.fUsingSavedCreds = false;
        s1.fPromptForPassword = false;
        s1.fEnableWindowsKey = true;
        s1.fDisableCtrlAltDel = true;
        s1.fMouse = true;

        let ls = &mut s1.WRdsListenerSettings;
        ls.WRdsListenerSettingLevel = WRDS_LISTENER_SETTING_LEVEL(1);
        ls.WRdsListenerSetting.WRdsListenerSettings1.pSecurityDescriptor = core::ptr::null_mut();
    }

    debug_log_line(&format!(
        "default_connection_settings listener_name={listener_name} (matching MS sample: NULL SD)",
    ));

    settings
}

pub fn create_protocol_manager_com() -> IWRdsProtocolManager {
    install_default_virtual_channel_bridge_handler_from_env();
    ComProtocolManager::new().into()
}

fn install_default_virtual_channel_bridge_handler_from_env() {
    if get_virtual_channel_bridge_handler().is_some() {
        return;
    }

    let Some(pipe_prefix) = std::env::var(VIRTUAL_CHANNEL_PIPE_BRIDGE_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return;
    };

    info!(
        pipe_prefix = %pipe_prefix,
        "Installing default virtual channel named-pipe bridge handler"
    );
    set_virtual_channel_bridge_handler(Some(Arc::new(NamedPipeBridgeHandler::new(pipe_prefix))));
}

struct NamedPipeBridgeHandler {
    pipe_prefix: String,
    workers: Mutex<HashMap<String, NamedPipeBridgeWorker>>,
    bridge_txs: Mutex<HashMap<String, VirtualChannelBridgeTx>>,
}

impl NamedPipeBridgeHandler {
    fn new(pipe_prefix: String) -> Self {
        Self {
            pipe_prefix,
            workers: Mutex::new(HashMap::new()),
            bridge_txs: Mutex::new(HashMap::new()),
        }
    }

    fn restart_worker(
        &self,
        endpoint: &VirtualChannelBridgeEndpoint,
        tx: VirtualChannelBridgeTx,
    ) -> mpsc::SyncSender<Vec<u8>> {
        let endpoint_key = bridge_endpoint_key(endpoint);
        let pipe_path = bridge_pipe_path(&self.pipe_prefix, endpoint);
        let endpoint_for_worker = endpoint.clone();

        let (to_pipe_tx, to_pipe_rx) = mpsc::sync_channel(VIRTUAL_CHANNEL_PIPE_BRIDGE_QUEUE_SIZE);
        let (stop_tx, stop_rx) = mpsc::channel();

        let join_handle = thread::spawn(move || {
            run_named_pipe_bridge_worker(endpoint_for_worker, pipe_path, tx, to_pipe_rx, stop_rx)
        });

        let mut workers = self.workers.lock();
        if let Some(previous) = workers.insert(
            endpoint_key,
            NamedPipeBridgeWorker {
                stop_tx,
                to_pipe_tx: to_pipe_tx.clone(),
                join_handle,
            },
        ) {
            previous.stop_and_join();
        }

        to_pipe_tx
    }

    fn stop_worker(&self, endpoint: &VirtualChannelBridgeEndpoint) {
        let endpoint_key = bridge_endpoint_key(endpoint);
        if let Some(worker) = self.workers.lock().remove(&endpoint_key) {
            worker.stop_and_join();
        }
    }

    fn get_bridge_tx(&self, endpoint: &VirtualChannelBridgeEndpoint) -> Option<VirtualChannelBridgeTx> {
        self.bridge_txs.lock().get(&bridge_endpoint_key(endpoint)).cloned()
    }
}

impl VirtualChannelBridgeHandler for NamedPipeBridgeHandler {
    fn on_channel_opened(&self, endpoint: &VirtualChannelBridgeEndpoint, tx: VirtualChannelBridgeTx) {
        self.bridge_txs.lock().insert(bridge_endpoint_key(endpoint), tx.clone());
        let _ = self.restart_worker(endpoint, tx);
    }

    fn on_channel_data(&self, endpoint: &VirtualChannelBridgeEndpoint, data: &[u8]) {
        let endpoint_key = bridge_endpoint_key(endpoint);
        let worker_tx = {
            let workers = self.workers.lock();
            workers.get(&endpoint_key).map(|worker| worker.to_pipe_tx.clone())
        };

        let tx = if let Some(tx) = worker_tx {
            tx
        } else {
            let Some(bridge_tx) = self.get_bridge_tx(endpoint) else {
                warn!(
                    endpoint = %endpoint.endpoint_name,
                    "Named-pipe bridge worker unavailable and bridge tx is not registered"
                );
                return;
            };

            self.restart_worker(endpoint, bridge_tx)
        };

        let result = tx.send(data.to_vec());

        if result.is_err() {
            warn!(
                endpoint = %endpoint.endpoint_name,
                "Failed to queue payload into named-pipe bridge worker"
            );
        }
    }

    fn on_channel_closed(&self, endpoint: &VirtualChannelBridgeEndpoint) {
        self.bridge_txs.lock().remove(&bridge_endpoint_key(endpoint));
        self.stop_worker(endpoint);
    }
}

struct NamedPipeBridgeWorker {
    stop_tx: mpsc::Sender<()>,
    to_pipe_tx: mpsc::SyncSender<Vec<u8>>,
    join_handle: thread::JoinHandle<()>,
}

impl NamedPipeBridgeWorker {
    fn stop_and_join(self) {
        if let Err(error) = self.stop_tx.send(()) {
            warn!(%error, "Failed to stop named-pipe bridge worker");
        }

        if let Err(error) = self.join_handle.join() {
            warn!(?error, "Named-pipe bridge worker thread panicked");
        }
    }
}

fn run_named_pipe_bridge_worker(
    endpoint: VirtualChannelBridgeEndpoint,
    pipe_path: String,
    to_channel_tx: VirtualChannelBridgeTx,
    to_pipe_rx: mpsc::Receiver<Vec<u8>>,
    stop_rx: mpsc::Receiver<()>,
) {
    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        let open_result = OpenOptions::new().read(true).write(true).open(&pipe_path);
        let mut pipe = match open_result {
            Ok(pipe) => {
                info!(endpoint = %endpoint.endpoint_name, pipe = %pipe_path, "Connected named-pipe bridge worker");
                pipe
            }
            Err(error) => {
                debug!(
                    endpoint = %endpoint.endpoint_name,
                    pipe = %pipe_path,
                    %error,
                    "Named-pipe bridge worker waiting for server"
                );
                thread::sleep(VIRTUAL_CHANNEL_PIPE_BRIDGE_RECONNECT_DELAY);
                continue;
            }
        };

        let mut from_pipe_buffer = Vec::with_capacity(4096);

        loop {
            if stop_rx.try_recv().is_ok() {
                return;
            }

            let mut write_failed = false;

            match to_pipe_rx.recv_timeout(VIRTUAL_CHANNEL_PIPE_BRIDGE_SEND_TIMEOUT) {
                Ok(payload) => {
                    if let Err(error) = write_length_prefixed(&mut pipe, &payload) {
                        warn!(
                            endpoint = %endpoint.endpoint_name,
                            pipe = %pipe_path,
                            %error,
                            "Named-pipe bridge write failed; reconnecting"
                        );
                        write_failed = true;
                    }

                    if write_failed {
                        break;
                    }

                    while let Ok(queued_payload) = to_pipe_rx.try_recv() {
                        if let Err(error) = write_length_prefixed(&mut pipe, &queued_payload) {
                            warn!(
                                endpoint = %endpoint.endpoint_name,
                                pipe = %pipe_path,
                                %error,
                                "Named-pipe bridge write failed while draining queue; reconnecting"
                            );
                            write_failed = true;
                            break;
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }

            if write_failed {
                break;
            }

            if let Err(error) =
                pump_named_pipe_inbound_frames(&mut pipe, &endpoint, &to_channel_tx, &mut from_pipe_buffer)
            {
                warn!(
                    endpoint = %endpoint.endpoint_name,
                    pipe = %pipe_path,
                    %error,
                    "Named-pipe bridge read failed; reconnecting"
                );
                break;
            }
        }
    }
}

fn pump_named_pipe_inbound_frames(
    pipe: &mut std::fs::File,
    endpoint: &VirtualChannelBridgeEndpoint,
    to_channel_tx: &VirtualChannelBridgeTx,
    read_buffer: &mut Vec<u8>,
) -> std::io::Result<()> {
    let mut chunk = [0u8; 8192];

    loop {
        let available = named_pipe_available_bytes(pipe)?;
        if available == 0 {
            break;
        }

        let read_len = usize::try_from(available).unwrap_or(usize::MAX).min(chunk.len());

        let read_count = match pipe.read(&mut chunk[..read_len]) {
            Ok(0) => {
                return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "named pipe closed"));
            }
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };

        read_buffer.extend_from_slice(&chunk[..read_count]);
        drain_length_prefixed_pipe_frames(endpoint, to_channel_tx, read_buffer)?;
    }

    Ok(())
}

fn named_pipe_available_bytes(pipe: &std::fs::File) -> std::io::Result<u32> {
    let mut total_bytes_available = 0u32;

    // SAFETY: `pipe.as_raw_handle()` returns a live OS handle for this file. We only ask
    // for the available byte count and provide a valid out-pointer.
    unsafe {
        PeekNamedPipe(
            HANDLE(pipe.as_raw_handle()),
            None,
            0,
            None,
            Some(&mut total_bytes_available),
            None,
        )
    }
    .map_err(|error| {
        let kind = if error.code() == HRESULT::from_win32(ERROR_BROKEN_PIPE.0)
            || error.code() == HRESULT::from_win32(ERROR_NO_DATA.0)
        {
            std::io::ErrorKind::BrokenPipe
        } else {
            std::io::ErrorKind::Other
        };

        std::io::Error::new(kind, format!("failed to peek named pipe: {error}"))
    })?;

    Ok(total_bytes_available)
}

fn drain_length_prefixed_pipe_frames(
    endpoint: &VirtualChannelBridgeEndpoint,
    to_channel_tx: &VirtualChannelBridgeTx,
    read_buffer: &mut Vec<u8>,
) -> std::io::Result<()> {
    let mut frame_offset = 0usize;

    while read_buffer.len().saturating_sub(frame_offset) >= 4 {
        let frame_len_u32 = u32::from_le_bytes([
            read_buffer[frame_offset],
            read_buffer[frame_offset + 1],
            read_buffer[frame_offset + 2],
            read_buffer[frame_offset + 3],
        ]);
        let frame_len = usize::try_from(frame_len_u32).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "named-pipe bridge frame length does not fit in usize",
            )
        })?;

        if frame_len > VIRTUAL_CHANNEL_PIPE_BRIDGE_MAX_FRAME_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "named-pipe bridge frame length exceeds limit (len={frame_len}, max={VIRTUAL_CHANNEL_PIPE_BRIDGE_MAX_FRAME_SIZE})"
                ),
            ));
        }

        let frame_total_len = 4usize + frame_len;
        if read_buffer.len().saturating_sub(frame_offset) < frame_total_len {
            break;
        }

        let payload_start = frame_offset + 4;
        let payload_end = payload_start + frame_len;
        let payload = read_buffer[payload_start..payload_end].to_vec();

        match to_channel_tx.outbound_tx.try_send(payload) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                warn!(
                    endpoint = %endpoint.endpoint_name,
                    "Dropped named-pipe inbound frame because virtual channel outbound queue is full"
                );
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "virtual channel bridge sender is closed",
                ));
            }
        }

        frame_offset = payload_end;
    }

    if frame_offset > 0 {
        read_buffer.drain(..frame_offset);
    }

    Ok(())
}

fn write_length_prefixed(mut writer: impl Write, payload: &[u8]) -> std::io::Result<()> {
    let len = u32::try_from(payload.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "payload too large"))?;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(payload)
}

fn bridge_endpoint_key(endpoint: &VirtualChannelBridgeEndpoint) -> String {
    let kind = if endpoint.static_channel { "svc" } else { "dvc" };
    format!("{kind}:{}", endpoint.endpoint_name.to_ascii_lowercase())
}

fn bridge_pipe_path(pipe_prefix: &str, endpoint: &VirtualChannelBridgeEndpoint) -> String {
    let normalized_prefix = if pipe_prefix.starts_with(r"\\.\pipe\") {
        pipe_prefix.to_owned()
    } else {
        format!(r"\\.\pipe\{pipe_prefix}")
    };

    let kind = if endpoint.static_channel { "svc" } else { "dvc" };
    let channel = sanitize_pipe_segment(&endpoint.endpoint_name);

    format!("{normalized_prefix}.{kind}.{channel}")
}

fn sanitize_pipe_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }

    if out.is_empty() {
        "channel".to_owned()
    } else {
        out
    }
}

#[implement(IClassFactory)]
struct ProtocolManagerClassFactory;

impl IClassFactory_Impl for ProtocolManagerClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: windows_core::Ref<'_, IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut core::ffi::c_void,
    ) -> windows_core::Result<()> {
        if ppvobject.is_null() || riid.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null COM output pointer"));
        }

        if punkouter.is_some() {
            return Err(windows_core::Error::new(
                CLASS_E_NOAGGREGATION,
                "aggregation is not supported",
            ));
        }

        // SAFETY: `ppvobject` is non-null (checked above) and COM expects us to
        // initialize out-pointers on all paths.
        unsafe { *ppvobject = core::ptr::null_mut() };

        // SAFETY: `riid` is non-null (checked above) and points to a valid GUID per COM contract.
        let requested_iid = unsafe { *riid };

        if requested_iid == IWRdsProtocolManager::IID {
            let manager = create_protocol_manager_com();
            // SAFETY: `ppvobject` is non-null and this branch returns a valid COM interface pointer.
            unsafe { *ppvobject = manager.into_raw() };

            return Ok(());
        }

        if requested_iid == IUnknown::IID {
            let manager = create_protocol_manager_com();
            let unknown: IUnknown = manager.cast()?;
            // SAFETY: `ppvobject` is non-null and this branch returns a valid COM interface pointer.
            unsafe { *ppvobject = unknown.into_raw() };

            return Ok(());
        }

        Err(windows_core::Error::new(
            E_NOINTERFACE,
            "requested interface is not supported",
        ))
    }

    fn LockServer(&self, flock: BOOL) -> windows_core::Result<()> {
        if flock.as_bool() {
            SERVER_LOCK_COUNT.fetch_add(1, Ordering::SeqCst);
        } else {
            let _ =
                SERVER_LOCK_COUNT.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| current.checked_sub(1));
        }

        Ok(())
    }
}

#[expect(unreachable_pub)]
#[unsafe(no_mangle)]
pub extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut core::ffi::c_void,
) -> HRESULT {
    let result = dll_get_class_object_impl(rclsid, riid, ppv);

    match result {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

#[expect(unreachable_pub)]
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if SERVER_LOCK_COUNT.load(Ordering::SeqCst) == 0 && ACTIVE_OBJECT_COUNT.load(Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

fn dll_get_class_object_impl(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut core::ffi::c_void,
) -> windows_core::Result<()> {
    if ppv.is_null() || riid.is_null() || rclsid.is_null() {
        return Err(windows_core::Error::new(E_POINTER, "null class object pointer"));
    }

    // SAFETY: `ppv` is non-null (checked above) and COM expects out-pointers to be initialized.
    unsafe { *ppv = core::ptr::null_mut() };

    // SAFETY: `rclsid` is non-null (checked above) and points to a valid GUID per COM contract.
    let requested_clsid = unsafe { *rclsid };
    if requested_clsid != IRONRDP_PROTOCOL_MANAGER_CLSID {
        return Err(windows_core::Error::new(
            CLASS_E_CLASSNOTAVAILABLE,
            "unknown protocol manager CLSID",
        ));
    }

    let factory: IClassFactory = ProtocolManagerClassFactory.into();
    // SAFETY: `riid` is non-null (checked above) and points to a valid GUID per COM contract.
    let requested_iid = unsafe { *riid };

    if requested_iid == IClassFactory::IID {
        // SAFETY: `ppv` is non-null and this branch returns a valid COM interface pointer.
        unsafe { *ppv = factory.into_raw() };

        return Ok(());
    }

    if requested_iid == IUnknown::IID {
        let unknown: IUnknown = factory.cast()?;
        // SAFETY: `ppv` is non-null and this branch returns a valid COM interface pointer.
        unsafe { *ppv = unknown.into_raw() };

        return Ok(());
    }

    Err(windows_core::Error::new(
        E_NOINTERFACE,
        "requested class factory interface is not supported",
    ))
}

#[implement(IWRdsProtocolManager)]
struct ComProtocolManager {
    _lifetime: ComObjectLifetime,
    inner: ProtocolManager,
}

impl ComProtocolManager {
    fn new() -> Self {
        Self {
            _lifetime: ComObjectLifetime::new(),
            inner: ProtocolManager::new(),
        }
    }
}

#[derive(Debug)]
struct ComObjectLifetime;

impl ComObjectLifetime {
    fn new() -> Self {
        ACTIVE_OBJECT_COUNT.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for ComObjectLifetime {
    fn drop(&mut self) {
        ACTIVE_OBJECT_COUNT.fetch_sub(1, Ordering::SeqCst);
    }
}

impl IWRdsProtocolManager_Impl for ComProtocolManager_Impl {
    fn Initialize(
        &self,
        _piwrdssettings: windows_core::Ref<'_, IWRdsProtocolSettings>,
        _pwrdssettings: *const WRDS_SETTINGS,
    ) -> windows_core::Result<()> {
        debug_log_line("IWRdsProtocolManager::Initialize");
        info!("Initialized protocol manager");
        Ok(())
    }

    fn CreateListener(&self, wszlistenername: &PCWSTR) -> windows_core::Result<IWRdsProtocolListener> {
        let listener_name = if wszlistenername.is_null() {
            "IRDP-Tcp".to_owned()
        } else {
            // SAFETY: listener name is provided by termservice and expected to be a valid
            // NUL-terminated wide string.
            unsafe { wszlistenername.to_string() }.map_err(|error| {
                windows_core::Error::new(E_UNEXPECTED, format!("failed to decode listener name: {error}"))
            })?
        };

        debug_log_line(&format!(
            "IWRdsProtocolManager::CreateListener listener_name={listener_name}"
        ));
        info!(listener_name = %listener_name, "Created protocol listener");
        Ok(ComProtocolListener::new(self.inner.create_listener(), listener_name).into())
    }

    fn NotifyServiceStateChange(&self, _ptsservicestatechange: *const WTS_SERVICE_STATE) -> windows_core::Result<()> {
        info!("Received service state change notification");
        Ok(())
    }

    fn NotifySessionOfServiceStart(&self, _sessionid: *const WTS_SESSION_ID) -> windows_core::Result<()> {
        debug!("Received session service start notification");
        Ok(())
    }

    fn NotifySessionOfServiceStop(&self, _sessionid: *const WTS_SESSION_ID) -> windows_core::Result<()> {
        debug!("Received session service stop notification");
        Ok(())
    }

    fn NotifySessionStateChange(&self, _sessionid: *const WTS_SESSION_ID, eventid: u32) -> windows_core::Result<()> {
        debug!(eventid, "Received session state change notification");
        Ok(())
    }

    fn NotifySettingsChange(&self, _pwrdssettings: *const WRDS_SETTINGS) -> windows_core::Result<()> {
        info!("Received protocol settings change notification");
        Ok(())
    }

    fn Uninitialize(&self) -> windows_core::Result<()> {
        info!("Uninitialized protocol manager");
        Ok(())
    }
}

#[implement(IWRdsProtocolListener)]
struct ComProtocolListener {
    inner: Arc<ProtocolListener>,
    listener_name: String,
    control_bridge: ProviderControlBridge,
    callback: Mutex<Option<IWRdsProtocolListenerCallback>>,
    worker: Mutex<Option<ListenerWorker>>,
}

impl ComProtocolListener {
    fn new(inner: ProtocolListener, listener_name: String) -> Self {
        Self {
            inner: Arc::new(inner),
            listener_name,
            control_bridge: ProviderControlBridge::from_env(),
            callback: Mutex::new(None),
            worker: Mutex::new(None),
        }
    }
}

impl IWRdsProtocolListener_Impl for ComProtocolListener_Impl {
    fn GetSettings(
        &self,
        wrdslistenersettinglevel: WRDS_LISTENER_SETTING_LEVEL,
    ) -> windows_core::Result<WRDS_LISTENER_SETTINGS> {
        let setting_level_1 = WRDS_LISTENER_SETTING_LEVEL(1);

        if wrdslistenersettinglevel != setting_level_1 {
            debug_log_line(&format!(
                "IWRdsProtocolListener::GetSettings listener_name={} level={} -> E_NOTIMPL (unsupported level)",
                self.listener_name, wrdslistenersettinglevel.0
            ));

            return Err(windows_core::Error::new(E_NOTIMPL, "unsupported listener setting level"));
        }

        let mut settings = WRDS_LISTENER_SETTINGS::default();
        settings.WRdsListenerSettingLevel = setting_level_1;

        settings.WRdsListenerSetting.WRdsListenerSettings1 = WRDS_LISTENER_SETTINGS_1 {
            MaxProtocolListenerConnectionCount: 0,
            SecurityDescriptorSize: 0,
            pSecurityDescriptor: core::ptr::null_mut(),
        };

        debug_log_line(&format!(
            "IWRdsProtocolListener::GetSettings listener_name={} level={} -> ok (max_conn=0 sd_size=0 sd_ptr=0x0)",
            self.listener_name, wrdslistenersettinglevel.0
        ));

        Ok(settings)
    }

    fn StartListen(&self, pcallback: windows_core::Ref<'_, IWRdsProtocolListenerCallback>) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsProtocolListener::StartListen listener_name={}",
            self.listener_name
        ));

        let _com_init_guard = ComInitGuard::new();

        let callback = pcallback
            .ok()
            .map_err(|_| windows_core::Error::new(E_POINTER, "null listener callback"))?
            .clone();

        if self.worker.lock().is_some() {
            debug_log_line(&format!(
                "IWRdsProtocolListener::StartListen already_started listener_name={}",
                self.listener_name
            ));
            info!("Protocol listener already started");
            return Ok(());
        }

        debug_log_line(&format!(
            "ProviderControlBridge::start_listen begin listener_name={}",
            self.listener_name
        ));
        let control_bridge_enabled = self.control_bridge.start_listen(&self.listener_name)?;
        debug_log_line(&format!(
            "ProviderControlBridge::start_listen ok listener_name={} enabled={control_bridge_enabled}",
            self.listener_name
        ));

        let (stop_tx, stop_rx) = mpsc::channel();

        let callback_agile = callback.cast::<IAgileObject>().is_ok();
        debug_log_line(&format!(
            "listener callback agile={callback_agile} listener_name={}",
            self.listener_name
        ));

        // TermService's IWRdsProtocolListenerCallback does not have a registered proxy/stub IID on
        // Windows Server 2025 (REGDB_E_IIDNOTREG), so CoMarshalInterThreadInterfaceInStream fails.
        // In practice, this callback is in-proc and appears safe to invoke from a worker thread.
        // We keep ownership correct by AddRef-ing via clone + into_raw and reconstructing on the worker.
        #[expect(
            clippy::as_conversions,
            reason = "store a raw COM interface pointer as usize so it can cross a thread boundary"
        )]
        let callback_token = callback.clone().into_raw() as usize;
        let listener = Arc::clone(&self.inner);
        let control_bridge = self.control_bridge.clone();
        let listener_name = self.listener_name.clone();

        let join_handle = thread::spawn(move || {
            // SAFETY: each worker thread initializes and uninitializes COM exactly once.
            let co_initialize = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if let Err(error) = co_initialize.ok() {
                warn!(%error, "Failed to initialize COM on listener worker thread");
                return;
            }

            #[expect(
                clippy::as_conversions,
                reason = "reconstruct raw COM interface pointer from the usize token"
            )]
            let callback_for_worker = {
                // SAFETY: token was produced by `IWRdsProtocolListenerCallback::into_raw` in this process.
                unsafe { IWRdsProtocolListenerCallback::from_raw(callback_token as *mut core::ffi::c_void) }
            };

            let mut connection_callbacks: HashMap<u32, IWRdsProtocolConnectionCallback> = HashMap::new();

            if control_bridge_enabled {
                loop {
                    if stop_rx.try_recv().is_ok() {
                        break;
                    }

                    let event = match control_bridge.wait_for_incoming(&listener_name, 250) {
                        Ok(event) => event,
                        Err(error) => {
                            warn!(%error, listener_name = %listener_name, "Failed to poll incoming connection from companion service");
                            thread::sleep(Duration::from_millis(200));
                            continue;
                        }
                    };

                    let Some(event) = event else {
                        // wait_for_incoming returned Ok(None): no new connection arrived in the
                        // polling window.  Sleep briefly before re-polling so that orphaned worker
                        // threads (left running in svchost after TermService stops) do not
                        // monopolise the companion's control pipe and starve the StartListen IPC
                        // call that a freshly loaded DLL instance must send.
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    };

                    let incoming = match event {
                        WaitForIncomingEvent::Incoming(incoming) => Some(incoming),
                        WaitForIncomingEvent::ConnectionBroken { connection_id, reason } => {
                            debug_log_line(&format!(
                                "Received connection_broken from companion connection_id={connection_id} reason={reason}",
                            ));

                            if let Some(callback) = connection_callbacks.remove(&connection_id) {
                                // SAFETY: callback is a valid COM interface returned by TermService.
                                let broken_result = unsafe { callback.BrokenConnection(0, 0) };
                                debug_log_line(&format!(
                                    "IWRdsProtocolConnectionCallback::BrokenConnection result={broken_result:?} connection_id={connection_id}",
                                ));
                            } else {
                                debug_log_line(&format!(
                                    "No TermService callback registered for broken connection_id={connection_id}",
                                ));
                            }

                            None
                        }
                    };

                    let Some(incoming) = incoming else {
                        continue;
                    };

                    let connection_entry = listener.create_connection_with_id(incoming.connection_id);
                    let connection_callback_slot = Arc::new(Mutex::new(None));
                    let connection: IWRdsProtocolConnection = ComProtocolConnection::new(
                        connection_entry,
                        Arc::clone(&connection_callback_slot),
                        control_bridge.clone(),
                    )
                    .into();

                    // Start the RDP server immediately so the client's X224 handshake is processed.
                    // RDS may call AcceptConnection later; the companion returns ConnectionReady
                    // if the session is already running.
                    if let Err(error) = control_bridge.accept_connection(incoming.connection_id) {
                        debug_log_line(&format!(
                            "Early accept_connection failed connection_id={} error={}; continuing",
                            incoming.connection_id, error
                        ));
                        warn!(
                            %error,
                            connection_id = incoming.connection_id,
                            "Early accept_connection failed; RDS may call AcceptConnection later"
                        );
                    }

                    let mut settings = default_connection_settings(&listener_name);

                    let sd_probe: Option<&'static mut [u8]> =
                        if std::env::var("IRONRDP_WTS_SD_PROBE").as_deref() == Ok("1") {
                            // DIAGNOSTIC: TermService may treat WRDS_LISTENER_SETTINGS_1.pSecurityDescriptor as an
                            // output buffer. Provide a large, process-lifetime buffer and detect whether it is written.
                            let probe: &'static mut [u8] = Box::leak(vec![0xCCu8; 4096].into_boxed_slice());
                            let probe_len = u32::try_from(probe.len()).unwrap_or(0);
                            // SAFETY: WRDS_CONNECTION_SETTINGS contains a union; `default_connection_settings` sets level=1.
                            unsafe {
                                let ls1 = &mut settings
                                    .WRdsConnectionSetting
                                    .WRdsConnectionSettings1
                                    .WRdsListenerSettings
                                    .WRdsListenerSetting
                                    .WRdsListenerSettings1;
                                ls1.SecurityDescriptorSize = probe_len;
                                ls1.pSecurityDescriptor = probe.as_mut_ptr();
                            }
                            Some(probe)
                        } else {
                            None
                        };

                    let (listener_level, protocol_type) = {
                        // SAFETY: union view is valid because `default_connection_settings` sets setting_level=1.
                        unsafe {
                            let s1 = &settings.WRdsConnectionSetting.WRdsConnectionSettings1;
                            (s1.WRdsListenerSettings.WRdsListenerSettingLevel.0, s1.ProtocolType)
                        }
                    };

                    debug_log_line(&format!(
                        "OnConnected calling listener_name={} connection_id={} conn_level={} listener_level={} protocol_type={}",
                        listener_name,
                        incoming.connection_id,
                        settings.WRdsConnectionSettingLevel.0,
                        listener_level,
                        protocol_type,
                    ));

                    // SAFETY: WRDS_CONNECTION_SETTINGS contains a union; `default_connection_settings` sets level=1.
                    unsafe {
                        let ls1 = &settings
                            .WRdsConnectionSetting
                            .WRdsConnectionSettings1
                            .WRdsListenerSettings
                            .WRdsListenerSetting
                            .WRdsListenerSettings1;
                        let sd_ptr = ls1.pSecurityDescriptor;
                        let sd_size = ls1.SecurityDescriptorSize;
                        let max_conn = ls1.MaxProtocolListenerConnectionCount;
                        if !sd_ptr.is_null() && sd_size > 0 {
                            let sd_size_usize = usize::try_from(sd_size).unwrap_or(0);
                            let sd_bytes = core::slice::from_raw_parts(sd_ptr, core::cmp::min(sd_size_usize, 40));
                            let hex: String = sd_bytes
                                .iter()
                                .map(|b| format!("{b:02x}"))
                                .collect::<Vec<_>>()
                                .join(" ");
                            debug_log_line(&format!(
                                "OnConnected SD dump: max_conn={max_conn} sd_size={sd_size} sd_ptr={sd_ptr:?} first_bytes=[{hex}]"
                            ));
                            let sd_psec = PSECURITY_DESCRIPTOR(sd_ptr.cast());
                            let valid = IsValidSecurityDescriptor(sd_psec);
                            debug_log_line(&format!("OnConnected SD valid={} right_before_call", valid.as_bool()));
                        } else {
                            debug_log_line(&format!(
                                "OnConnected SD: NULL or empty max_conn={max_conn} sd_size={sd_size} sd_ptr={sd_ptr:?}"
                            ));
                        }
                    }

                    // SAFETY: COM callback and connection object are valid for call duration.
                    let result = unsafe { callback_for_worker.OnConnected(&connection, &settings) };
                    if let Some(sd_probe) = sd_probe.as_deref() {
                        let sd_written = sd_probe.iter().any(|byte| *byte != 0xCC);
                        let b0 = sd_probe.first().copied().unwrap_or(0);
                        let b1 = sd_probe.get(1).copied().unwrap_or(0);
                        let b2 = sd_probe.get(2).copied().unwrap_or(0);
                        let b3 = sd_probe.get(3).copied().unwrap_or(0);
                        debug_log_line(&format!(
                            "OnConnected sd_probe written={sd_written} first4={b0:02X}{b1:02X}{b2:02X}{b3:02X}",
                        ));
                    }

                    match result {
                        Ok(connection_callback) => {
                            debug_log_line(&format!(
                                "IWRdsProtocolListenerCallback::OnConnected ok listener_name={} connection_id={}",
                                listener_name, incoming.connection_id
                            ));

                            // SAFETY: connection_callback is a valid COM interface returned by TermService.
                            let conn_id_result = unsafe { connection_callback.GetConnectionId() };
                            debug_log_line(&format!(
                                "GetConnectionId result={:?} connection_id={}",
                                conn_id_result, incoming.connection_id
                            ));

                            match AgileReference::new(&connection_callback) {
                                Ok(callback_agile) => {
                                    *connection_callback_slot.lock() = Some(callback_agile);
                                }
                                Err(error) => {
                                    debug_log_line(&format!(
                                        "Failed to create AgileReference for connection callback connection_id={} error={error}",
                                        incoming.connection_id,
                                    ));
                                    warn!(
                                        %error,
                                        connection_id = incoming.connection_id,
                                        "Failed to create AgileReference for TermService connection callback"
                                    );
                                }
                            }

                            connection_callbacks.insert(incoming.connection_id, connection_callback.clone());

                            // SAFETY: connection_callback is a valid COM interface returned by TermService.
                            let ready_result = unsafe { connection_callback.OnReady() };
                            debug_log_line(&format!(
                                "OnReady result={:?} connection_id={}",
                                ready_result, incoming.connection_id
                            ));

                            debug!(
                                connection_id = incoming.connection_id,
                                peer_addr = ?incoming.peer_addr,
                                "Dispatched incoming connection from companion service"
                            );
                        }
                        Err(error) => {
                            debug_log_line(&format!(
                                "IWRdsProtocolListenerCallback::OnConnected failed listener_name={} connection_id={} error={error}",
                                listener_name,
                                incoming.connection_id
                            ));
                            warn!(
                                %error,
                                connection_id = incoming.connection_id,
                                "Failed to dispatch OnConnected callback for incoming connection"
                            );
                        }
                    }
                }
            } else {
                let bootstrap_connection = listener.create_connection();
                let connection_callback_slot = Arc::new(Mutex::new(None));
                let connection: IWRdsProtocolConnection = ComProtocolConnection::new(
                    bootstrap_connection,
                    Arc::clone(&connection_callback_slot),
                    control_bridge,
                )
                .into();

                let mut settings = default_connection_settings(&listener_name);

                let sd_probe: Option<&'static mut [u8]> = if std::env::var("IRONRDP_WTS_SD_PROBE").as_deref() == Ok("1")
                {
                    let probe: &'static mut [u8] = Box::leak(vec![0xCCu8; 4096].into_boxed_slice());
                    let probe_len = u32::try_from(probe.len()).unwrap_or(0);
                    // SAFETY: WRDS_CONNECTION_SETTINGS contains a union; `default_connection_settings` sets level=1.
                    unsafe {
                        let ls1 = &mut settings
                            .WRdsConnectionSetting
                            .WRdsConnectionSettings1
                            .WRdsListenerSettings
                            .WRdsListenerSetting
                            .WRdsListenerSettings1;
                        ls1.SecurityDescriptorSize = probe_len;
                        ls1.pSecurityDescriptor = probe.as_mut_ptr();
                    }
                    Some(probe)
                } else {
                    None
                };

                let (listener_level, protocol_type) = {
                    // SAFETY: union view is valid because `default_connection_settings` sets setting_level=1.
                    unsafe {
                        let s1 = &settings.WRdsConnectionSetting.WRdsConnectionSettings1;
                        (s1.WRdsListenerSettings.WRdsListenerSettingLevel.0, s1.ProtocolType)
                    }
                };

                debug_log_line(&format!(
                    "OnConnected calling listener_name={} connection_id={} conn_level={} listener_level={} protocol_type={}",
                    listener_name,
                    0,
                    settings.WRdsConnectionSettingLevel.0,
                    listener_level,
                    protocol_type,
                ));

                // SAFETY: COM callback and connection object are valid for call duration.
                let result = unsafe { callback_for_worker.OnConnected(&connection, &settings) };
                if let Some(sd_probe) = sd_probe.as_deref() {
                    let sd_written = sd_probe.iter().any(|byte| *byte != 0xCC);
                    let b0 = sd_probe.first().copied().unwrap_or(0);
                    let b1 = sd_probe.get(1).copied().unwrap_or(0);
                    let b2 = sd_probe.get(2).copied().unwrap_or(0);
                    let b3 = sd_probe.get(3).copied().unwrap_or(0);
                    debug_log_line(&format!(
                        "OnConnected sd_probe written={sd_written} first4={b0:02X}{b1:02X}{b2:02X}{b3:02X}",
                    ));
                }

                match result {
                    Ok(connection_callback) => match AgileReference::new(&connection_callback) {
                        Ok(callback_agile) => {
                            *connection_callback_slot.lock() = Some(callback_agile);
                        }
                        Err(error) => {
                            debug_log_line(&format!(
                                "Failed to create AgileReference for connection callback (standalone) error={error}",
                            ));
                            warn!(
                                %error,
                                "Failed to create AgileReference for TermService connection callback (standalone)"
                            );
                        }
                    },
                    Err(error) => {
                        warn!(%error, "Failed to dispatch OnConnected callback");
                    }
                }

                let _ = stop_rx.recv();
            }

            // SAFETY: paired with successful `CoInitializeEx` above.
            unsafe { CoUninitialize() };
        });

        *self.worker.lock() = Some(ListenerWorker { stop_tx, join_handle });
        *self.callback.lock() = Some(callback);
        debug_log_line(&format!(
            "IWRdsProtocolListener::StartListen worker_started listener_name={}",
            self.listener_name
        ));
        info!("Started protocol listener worker");

        Ok(())
    }

    fn StopListen(&self) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsProtocolListener::StopListen listener_name={}",
            self.listener_name
        ));
        if let Some(worker) = self.worker.lock().take() {
            if let Err(error) = worker.stop_tx.send(()) {
                warn!(%error, "Failed to signal listener worker stop");
            }

            if let Err(error) = worker.join_handle.join() {
                warn!(?error, "Listener worker thread panicked");
            }
        }

        *self.callback.lock() = None;

        if let Err(error) = self.control_bridge.stop_listen(&self.listener_name) {
            warn!(%error, listener_name = %self.listener_name, "Failed to stop companion service listener");
        }

        info!("Stopped protocol listener worker");
        Ok(())
    }
}

// Minimal IWRdsProtocolLicenseConnection implementation that allows TermService to proceed past
// licensing.  Returning E_NOTIMPL from GetLicenseConnection causes TermService to close the
// connection immediately; providing this stub object allows the connection to advance to
// IsUserAllowedToLogon so that the user token can be passed back and auto-logon can occur.
#[implement(IWRdsProtocolLicenseConnection)]
struct WrdsLicenseConnection;

impl IWRdsProtocolLicenseConnection_Impl for WrdsLicenseConnection_Impl {
    fn RequestLicensingCapabilities(
        &self,
        pplicensecapabilities: *mut WTS_LICENSE_CAPABILITIES,
        pcblicensecapabilities: *mut u32,
    ) -> windows_core::Result<()> {
        debug_log_line("WrdsLicenseConnection::RequestLicensingCapabilities");
        if pplicensecapabilities.is_null() || pcblicensecapabilities.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null pointer"));
        }
        // The SDK signature is PWRDS_LICENSE_CAPABILITIES* (double pointer): TermService passes a
        // pointer-to-pointer and expects us to allocate and write the struct address into it.
        // The windows-rs binding exposes this as *mut WTS_LICENSE_CAPABILITIES, so we treat it
        // as a raw byte pointer and write the pointer value unaligned.
        // SAFETY: TermService provides valid output slots for the duration of this call.
        unsafe {
            let caps = CoTaskMemAlloc(size_of::<WTS_LICENSE_CAPABILITIES>()).cast::<WTS_LICENSE_CAPABILITIES>();
            if caps.is_null() {
                return Err(windows_core::Error::new(E_OUTOFMEMORY, "CoTaskMemAlloc failed"));
            }
            // SAFETY: `caps` points to a writable allocation sized for WTS_LICENSE_CAPABILITIES.
            core::ptr::write(
                caps,
                WTS_LICENSE_CAPABILITIES {
                    KeyExchangeAlg: WTS_KEY_EXCHANGE_ALG_RSA,
                    ProtocolVer: WTS_LICENSE_PREAMBLE_VERSION,
                    fAuthenticateServer: BOOL(0),
                    CertType: WTS_CERT_TYPE_INVALID,
                    cbClientName: 0,
                    rgbClientName: [0; 42],
                },
            );
            // Write the allocated pointer into the caller's output slot (double-pointer semantics).
            let dst = pplicensecapabilities.cast::<u8>();
            let src = core::ptr::addr_of!(caps).cast::<u8>();
            core::ptr::copy_nonoverlapping(src, dst, size_of::<*mut WTS_LICENSE_CAPABILITIES>());
            // SAFETY: pcblicensecapabilities is a valid out-pointer.
            *pcblicensecapabilities = u32::try_from(size_of::<WTS_LICENSE_CAPABILITIES>()).unwrap_or(0);
        }
        Ok(())
    }

    fn SendClientLicense(&self, _pclientlicense: *const u8, _cbclientlicense: u32) -> windows_core::Result<()> {
        debug_log_line("WrdsLicenseConnection::SendClientLicense");
        Ok(())
    }

    fn RequestClientLicense(
        &self,
        _reserve1: *const u8,
        _reserve2: u32,
        _ppclientlicense: *mut u8,
        pcbclientlicense: *mut u32,
    ) -> windows_core::Result<()> {
        debug_log_line("WrdsLicenseConnection::RequestClientLicense");
        // Return an empty license (0 bytes) — sufficient for the RDS grace period and
        // "Remote Desktop for Administration" (2-admin-connection) mode on a non-CAL server.
        if !pcbclientlicense.is_null() {
            // SAFETY: pcbclientlicense is a valid out-pointer when non-null.
            unsafe { *pcbclientlicense = 0 };
        }
        Ok(())
    }

    fn ProtocolComplete(&self, _ulcomplete: u32) -> windows_core::Result<()> {
        debug_log_line("WrdsLicenseConnection::ProtocolComplete");
        Ok(())
    }
}

#[implement(IWRdsProtocolLogonErrorRedirector)]
struct WrdsLogonErrorRedirector;

impl IWRdsProtocolLogonErrorRedirector_Impl for WrdsLogonErrorRedirector_Impl {
    fn OnBeginPainting(&self) -> windows_core::Result<()> {
        debug_log_line("WrdsLogonErrorRedirector::OnBeginPainting");
        Ok(())
    }

    fn RedirectStatus(&self, pszmessage: &PCWSTR) -> windows_core::Result<WTS_LOGON_ERROR_REDIRECTOR_RESPONSE> {
        let message = if pszmessage.0.is_null() {
            String::new()
        } else {
            // SAFETY: TermService provides a valid NUL-terminated PCWSTR for the duration of this call.
            unsafe { pszmessage.to_string() }.unwrap_or_default()
        };

        debug_log_line(&format!(
            "WrdsLogonErrorRedirector::RedirectStatus message={message}",
        ));
        Ok(WTS_LOGON_ERR_NOT_HANDLED)
    }

    fn RedirectMessage(
        &self,
        pszcaption: &PCWSTR,
        pszmessage: &PCWSTR,
        utype: u32,
    ) -> windows_core::Result<WTS_LOGON_ERROR_REDIRECTOR_RESPONSE> {
        let caption = if pszcaption.0.is_null() {
            String::new()
        } else {
            // SAFETY: TermService provides a valid NUL-terminated PCWSTR for the duration of this call.
            unsafe { pszcaption.to_string() }.unwrap_or_default()
        };
        let message = if pszmessage.0.is_null() {
            String::new()
        } else {
            // SAFETY: TermService provides a valid NUL-terminated PCWSTR for the duration of this call.
            unsafe { pszmessage.to_string() }.unwrap_or_default()
        };

        debug_log_line(&format!(
            "WrdsLogonErrorRedirector::RedirectMessage utype={utype} caption={caption} message={message}",
        ));
        Ok(WTS_LOGON_ERR_NOT_HANDLED)
    }

    fn RedirectLogonError(
        &self,
        ntsstatus: i32,
        ntssubstatus: i32,
        pszcaption: &PCWSTR,
        pszmessage: &PCWSTR,
        utype: u32,
    ) -> windows_core::Result<WTS_LOGON_ERROR_REDIRECTOR_RESPONSE> {
        let caption = if pszcaption.0.is_null() {
            String::new()
        } else {
            // SAFETY: TermService provides a valid NUL-terminated PCWSTR for the duration of this call.
            unsafe { pszcaption.to_string() }.unwrap_or_default()
        };
        let message = if pszmessage.0.is_null() {
            String::new()
        } else {
            // SAFETY: TermService provides a valid NUL-terminated PCWSTR for the duration of this call.
            unsafe { pszmessage.to_string() }.unwrap_or_default()
        };

        debug_log_line(&format!(
            "WrdsLogonErrorRedirector::RedirectLogonError ntsstatus={ntsstatus} ntssubstatus={ntssubstatus} utype={utype} caption={caption} message={message}",
        ));
        Ok(WTS_LOGON_ERR_NOT_HANDLED)
    }
}

#[implement(IWRdsProtocolConnection, IWRdsWddmIddProps, IAgileObject)]
struct ComProtocolConnection {
    inner: Arc<ProtocolConnection>,
    auth_bridge: CredsspServerBridge,
    connection_callback: Arc<Mutex<Option<AgileReference<IWRdsProtocolConnectionCallback>>>>,
    control_bridge: ProviderControlBridge,
    wddm_idd_enabled: AtomicBool,
    driver_handle_raw: AtomicUsize,
    ready_notified: Mutex<bool>,
    cached_credentials: Mutex<Option<(String, String, String)>>,
    last_input_time: Mutex<u64>,
    virtual_channels: Mutex<Vec<VirtualChannelHandle>>,
    virtual_channel_forwarders: Mutex<Vec<VirtualChannelForwarderWorker>>,
    keyboard_handle: Mutex<Option<HANDLE>>,
    mouse_handle: Mutex<Option<HANDLE>>,
    video_handle: Mutex<Option<HANDLE>>,
}

impl IAgileObject_Impl for ComProtocolConnection_Impl {}

impl ComProtocolConnection {
    fn new(
        inner: Arc<ProtocolConnection>,
        connection_callback: Arc<Mutex<Option<AgileReference<IWRdsProtocolConnectionCallback>>>>,
        control_bridge: ProviderControlBridge,
    ) -> Self {
        Self {
            inner,
            auth_bridge: CredsspServerBridge::default(),
            connection_callback,
            control_bridge,
            wddm_idd_enabled: AtomicBool::new(false),
            driver_handle_raw: AtomicUsize::new(0),
            ready_notified: Mutex::new(false),
            cached_credentials: Mutex::new(None),
            last_input_time: Mutex::new(0),
            virtual_channels: Mutex::new(Vec::new()),
            virtual_channel_forwarders: Mutex::new(Vec::new()),
            keyboard_handle: Mutex::new(None),
            mouse_handle: Mutex::new(None),
            video_handle: Mutex::new(None),
        }
    }

    fn close_device_handles(&self) {
        // Best-effort cleanup: TermService might also close these, but we treat them as owned
        // by this connection object to avoid leaks across repeated connections.
        for slot in [&self.keyboard_handle, &self.mouse_handle, &self.video_handle] {
            if let Some(handle) = slot.lock().take() {
                // SAFETY: handle came from CreateFileW.
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(handle);
                }
            }
        }
    }

    fn open_device_handle_with_access(path: &str, desired_access: u32) -> windows_core::Result<HANDLE> {
        let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();

        // SAFETY: `wide` is NUL-terminated and lives for the duration of the call.
        unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                desired_access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }
    }

    fn open_device_handle(path: &str) -> windows_core::Result<HANDLE> {
        // Best-effort: request read/write; these are device objects.
        // If access is denied, TermService will likely fail to advance the connection sequence.
        let access = (windows::Win32::Storage::FileSystem::FILE_GENERIC_READ
            | windows::Win32::Storage::FileSystem::FILE_GENERIC_WRITE)
            .0;
        Self::open_device_handle_with_access(path, access)
    }

    fn open_first_device_handle(candidates: &[(&'static str, u32)]) -> windows_core::Result<HANDLE> {
        let mut last_error: Option<windows_core::Error> = None;
        for (path, access) in candidates {
            match Self::open_device_handle_with_access(path, *access) {
                Ok(handle) => return Ok(handle),
                Err(error) => last_error = Some(error),
            }
        }

        Err(last_error.unwrap_or_else(|| windows_core::Error::new(E_UNEXPECTED, "no device handle candidates")))
    }

    fn open_keyboard_device_handle() -> windows_core::Result<HANDLE> {
        // On some Windows Server builds, the Win32 \\.\KeyboardClassN links are missing,
        // but GLOBALROOT device paths are present and can be opened with 0 desired access.
        let rw = (windows::Win32::Storage::FileSystem::FILE_GENERIC_READ
            | windows::Win32::Storage::FileSystem::FILE_GENERIC_WRITE)
            .0;

        Self::open_first_device_handle(&[
            (r"\\.\KeyboardClass0", rw),
            (r"\\?\GLOBALROOT\Device\KeyboardClass0", 0),
            (r"\\?\GLOBALROOT\Device\KeyboardClass1", 0),
        ])
    }

    fn open_pointer_device_handle() -> windows_core::Result<HANDLE> {
        let rw = (windows::Win32::Storage::FileSystem::FILE_GENERIC_READ
            | windows::Win32::Storage::FileSystem::FILE_GENERIC_WRITE)
            .0;

        Self::open_first_device_handle(&[
            (r"\\.\PointerClass0", rw),
            (r"\\?\GLOBALROOT\Device\PointerClass0", 0),
            (r"\\?\GLOBALROOT\Device\PointerClass1", 0),
        ])
    }

    fn notify_ready(&self) -> windows_core::Result<()> {
        let mut ready_notified = self.ready_notified.lock();
        if *ready_notified {
            return Ok(());
        }

        // Best-effort: notify the companion that this connection is being accepted.  If the IPC
        // fails (e.g. the pipe is busy because the listener worker's early-accept already started
        // the session and the pipe is occupied with wait_for_incoming polling), log and continue
        // anyway.  The early accept from the listener worker has already started the IronRDP
        // session, so the companion is ready.  Returning an error here would cause TermService to
        // close the connection before ever calling GetClientData / IsUserAllowedToLogon.
        match self.control_bridge.accept_connection(self.inner.connection_id()) {
            Ok(()) => {
                debug_log_line(&format!(
                    "notify_ready: accept_connection ok connection_id={}",
                    self.inner.connection_id()
                ));
            }
            Err(error) => {
                debug_log_line(&format!(
                    "notify_ready: accept_connection IPC failed (early accept already active); continuing connection_id={} error={}",
                    self.inner.connection_id(),
                    error
                ));
            }
        }

        *ready_notified = true;
        Ok(())
    }

    fn fetch_and_cache_connection_credentials(
        &self,
        connection_id: u32,
        source: &'static str,
    ) -> windows_core::Result<Option<(String, String, String)>> {
        let Some((username, domain, password)) = self.control_bridge.get_connection_credentials(connection_id)? else {
            debug_log_line(&format!(
                "fetch_and_cache_connection_credentials none connection_id={connection_id} source={source}",
            ));
            return Ok(None);
        };

        let (winlogon_username, winlogon_domain) = normalize_winlogon_credentials(&username, &domain);
        let credentials = (winlogon_username, winlogon_domain, password);
        *self.cached_credentials.lock() = Some(credentials.clone());

        debug_log_line(&format!(
            "fetch_and_cache_connection_credentials cached connection_id={connection_id} source={source} user={} domain={}",
            credentials.0, credentials.1,
        ));

        Ok(Some(credentials))
    }

    fn cached_connection_credentials(&self) -> Option<(String, String, String)> {
        self.cached_credentials.lock().clone()
    }

    fn release_connection_callback(&self) {
        *self.connection_callback.lock() = None;
    }

    fn release_virtual_channels(&self) {
        let mut channels = self.virtual_channels.lock();
        channels.clear();
    }

    fn release_virtual_channel_forwarders(&self) {
        let mut workers = self.virtual_channel_forwarders.lock();

        for worker in workers.drain(..) {
            let endpoint_name = worker.endpoint.endpoint_name.clone();

            if let Err(error) = worker.stop_tx.send(()) {
                warn!(
                    endpoint = %endpoint_name,
                    %error,
                    "Failed to signal virtual channel forwarder stop"
                );
            }

            if let Err(error) = worker.join_handle.join() {
                warn!(
                    endpoint = %endpoint_name,
                    ?error,
                    "Virtual channel forwarder thread panicked"
                );
            }
        }
    }

    fn find_virtual_channel(&self, endpoint_name: &str, is_static: bool) -> Option<(HANDLE, VirtualChannelBridgePlan)> {
        self.virtual_channels
            .lock()
            .iter()
            .find(|channel| channel.matches(endpoint_name, is_static))
            .map(|channel| (channel.raw(), channel.bridge_plan))
    }

    fn register_virtual_channel(
        &self,
        handle: HANDLE,
        endpoint_name: Option<String>,
        is_static: bool,
        bridge_plan: VirtualChannelBridgePlan,
    ) -> windows_core::Result<HANDLE> {
        let endpoint_name_for_worker = endpoint_name.clone();
        let mut channels = self.virtual_channels.lock();
        channels.push(VirtualChannelHandle::new(handle, endpoint_name, is_static, bridge_plan));

        let channel_handle = channels
            .last()
            .map(VirtualChannelHandle::raw)
            .ok_or_else(|| windows_core::Error::new(E_UNEXPECTED, "virtual channel storage failure"))?;

        drop(channels);

        if let Some(endpoint_name) = endpoint_name_for_worker {
            self.maybe_start_virtual_channel_forwarder(channel_handle, endpoint_name, is_static, bridge_plan);
        }

        Ok(channel_handle)
    }

    fn maybe_start_virtual_channel_forwarder(
        &self,
        channel_handle: HANDLE,
        endpoint_name: String,
        is_static: bool,
        bridge_plan: VirtualChannelBridgePlan,
    ) {
        if !bridge_plan.should_prepare_forwarding() {
            return;
        }

        let Some(handler) = get_virtual_channel_bridge_handler() else {
            return;
        };

        let endpoint = VirtualChannelBridgeEndpoint {
            endpoint_name,
            static_channel: is_static,
            route_kind: bridge_plan.route_kind,
        };

        let (stop_tx, stop_rx) = mpsc::channel();
        let (outbound_tx, outbound_rx) = mpsc::sync_channel(VIRTUAL_CHANNEL_FORWARDER_OUTBOUND_QUEUE_SIZE);

        let tx = VirtualChannelBridgeTx {
            endpoint: endpoint.clone(),
            outbound_tx,
        };

        handler.on_channel_opened(&endpoint, tx);

        let endpoint_for_worker = endpoint.clone();
        let handler_for_worker = Arc::clone(&handler);
        let channel_handle_raw = handle_to_raw_usize(channel_handle);
        let join_handle = thread::spawn(move || {
            run_virtual_channel_forwarder(
                channel_handle_raw,
                endpoint_for_worker,
                handler_for_worker,
                outbound_rx,
                stop_rx,
            )
        });

        self.virtual_channel_forwarders
            .lock()
            .push(VirtualChannelForwarderWorker {
                endpoint,
                stop_tx,
                join_handle,
            });

        info!("Started virtual channel forwarder");
    }

    fn open_virtual_channel_by_name(
        &self,
        session_id: u32,
        endpoint_name: &str,
        is_static: bool,
        requested_priority: u32,
        bridge_plan: VirtualChannelBridgePlan,
    ) -> windows_core::Result<HANDLE> {
        if let Some((existing, _existing_bridge_plan)) = self.find_virtual_channel(endpoint_name, is_static) {
            return Ok(existing);
        }

        let endpoint_name_cstring = CString::new(endpoint_name)
            .map_err(|_| windows_core::Error::new(E_UNEXPECTED, "virtual channel endpoint contains NUL byte"))?;
        let endpoint = PCSTR::from_raw(endpoint_name_cstring.as_ptr().cast::<u8>());
        let flags = virtual_channel_open_flags(is_static, requested_priority);

        // SAFETY: `endpoint` points to a valid NUL-terminated string for the duration of the call.
        let channel = unsafe { WTSVirtualChannelOpenEx(session_id, endpoint, flags) }?;

        if let Some((existing, _existing_bridge_plan)) = self.find_virtual_channel(endpoint_name, is_static) {
            // SAFETY: `channel` is a handle returned by `WTSVirtualChannelOpenEx`.
            if let Err(error) = unsafe { WTSVirtualChannelClose(channel) } {
                warn!(%error, "Failed to close duplicate virtual channel handle");
            }

            return Ok(existing);
        }

        self.register_virtual_channel(channel, Some(endpoint_name.to_owned()), is_static, bridge_plan)
    }

    fn ensure_ironrdp_drdynvc_channel(&self, session_id: u32) -> windows_core::Result<HANDLE> {
        self.open_virtual_channel_by_name(
            session_id,
            IRONRDP_DRDYNVC_CHANNEL_NAME,
            true,
            0,
            VirtualChannelBridgePlan::for_endpoint(true, Some(IronRdpVirtualChannelServer::Drdynvc)),
        )
    }
}

impl IWRdsProtocolConnectionSettings_Impl for ComProtocolConnection_Impl {
    fn SetConnectionSetting(
        &self,
        _propertyid: &GUID,
        _ppropertyentriesin: *const WTS_PROPERTY_VALUE,
    ) -> windows_core::Result<()> {
        debug_log_line("IWRdsProtocolConnectionSettings::SetConnectionSetting called");
        Ok(())
    }

    fn GetConnectionSetting(
        &self,
        _propertyid: &GUID,
        _ppropertyentriesout: *mut WTS_PROPERTY_VALUE,
    ) -> windows_core::Result<()> {
        debug_log_line("IWRdsProtocolConnectionSettings::GetConnectionSetting called");
        Err(windows_core::Error::new(
            E_NOTIMPL,
            "GetConnectionSetting not implemented",
        ))
    }
}

impl IWRdsWddmIddProps_Impl for ComProtocolConnection_Impl {
    fn GetHardwareId(&self, pdisplaydriverhardwareid: &PCWSTR, count: u32) -> windows_core::Result<()> {
        // TermService uses this to locate the WDDM IDD display driver to load for a session.
        let hardware_id = IRONRDP_IDD_HARDWARE_ID;

        let wide: Vec<u16> = hardware_id.encode_utf16().chain(Some(0)).collect();

        let required = wide.len();
        let capacity = usize::try_from(count).unwrap_or(0);

        if pdisplaydriverhardwareid.0.is_null() {
            debug_log_line("IWRdsWddmIddProps::GetHardwareId null out-buffer");
            return Err(windows_core::Error::new(E_POINTER, "hardware id buffer is null"));
        }

        if capacity < required {
            debug_log_line(&format!(
                "IWRdsWddmIddProps::GetHardwareId insufficient buffer capacity={capacity} required={required}",
            ));
            return Err(windows_core::Error::new(
                HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0),
                "hardware id buffer is too small",
            ));
        }

        // SAFETY: TermService provides a writable buffer with `count` characters.
        unsafe {
            core::ptr::copy_nonoverlapping(wide.as_ptr(), pdisplaydriverhardwareid.0.cast_mut(), required);
        }

        debug_log_line(&format!(
            "IWRdsWddmIddProps::GetHardwareId wrote '{hardware_id}' chars={}",
            required.saturating_sub(1)
        ));

        Ok(())
    }

    fn OnDriverLoad(&self, sessionid: u32, driverhandle: HANDLE_PTR) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsWddmIddProps::OnDriverLoad session_id={sessionid} handle={driverhandle:?}",
        ));

        self.driver_handle_raw.store(driverhandle.0, Ordering::SeqCst);

        if let Err(error) = self.control_bridge.notify_idd_driver_loaded(sessionid) {
            debug_log_line(&format!(
                "IWRdsWddmIddProps::OnDriverLoad notify_idd_driver_loaded failed: {error}"
            ));
        }
        Ok(())
    }

    fn OnDriverUnload(&self, sessionid: u32) -> windows_core::Result<()> {
        debug_log_line(&format!("IWRdsWddmIddProps::OnDriverUnload session_id={sessionid}"));

        self.driver_handle_raw.store(0, Ordering::SeqCst);
        Ok(())
    }

    fn EnableWddmIdd(&self, enabled: BOOL) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsWddmIddProps::EnableWddmIdd enabled={}",
            enabled.as_bool()
        ));

        self.wddm_idd_enabled.store(enabled.as_bool(), Ordering::SeqCst);
        Ok(())
    }
}

impl IWRdsProtocolConnection_Impl for ComProtocolConnection_Impl {
    fn GetLogonErrorRedirector(&self) -> windows_core::Result<IWRdsProtocolLogonErrorRedirector> {
        Ok(WrdsLogonErrorRedirector.into())
    }

    fn AcceptConnection(&self) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        debug_log_line(&format!(
            "IWRdsProtocolConnection::AcceptConnection called connection_id={}",
            connection_id
        ));

        self.auth_bridge.validate_security_protocol(
            CredsspPolicy::default(),
            nego::SecurityProtocol::HYBRID | nego::SecurityProtocol::HYBRID_EX,
        )?;

        self.inner.accept_connection().map_err(transition_error)?;
        self.notify_ready()?;

        if self.cached_connection_credentials().is_none() {
            let _ = self.fetch_and_cache_connection_credentials(connection_id, "accept_connection_prefetch")?;
        }

        debug_log_line(&format!(
            "IWRdsProtocolConnection::AcceptConnection ok connection_id={connection_id}",
        ));
        Ok(())
    }

    fn GetClientData(&self, pclientdata: *mut WTS_CLIENT_DATA) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetClientData called connection_id={}",
            self.inner.connection_id()
        ));

        if pclientdata.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null client data pointer"));
        }

        // SAFETY: pclientdata is non-null and points to writable memory owned by TermService.
        let client_data = unsafe { &mut *pclientdata };
        *client_data = WTS_CLIENT_DATA::default();
        // Match typical RDP defaults so Winlogon doesn't block on secure-attention requirements.
        client_data.fDisableCtrlAltDel = true;
        client_data.fMouse = true;
        client_data.fMaximizeShell = true;
        client_data.fEnableWindowsKey = true;
        // Only advertise auto-logon once we actually have CredSSP-derived credentials.
        // Otherwise, some TermService/Winlogon paths will attempt to use empty fields and fail.
        client_data.fInheritAutoLogon = BOOL(0);
        client_data.fUsingSavedCreds = false;
        client_data.fPromptForPassword = false;
        client_data.fNoAudioPlayback = true;
        // Advertise an explicit custom protocol name.
        client_data.ProtocolType = WTS_PROTOCOL_TYPE_NON_RDP;
        copy_wide(&mut client_data.ProtocolName, "RDP");

        // Some TermService/Winlogon code paths rely on the auto-logon credentials embedded in
        // WTS_CLIENT_DATA when fInheritAutoLogon is set. Populate these from the CredSSP-derived
        // credentials when available.
        //
        // The username/password are plaintext per Microsoft docs.
        let connection_id = self.inner.connection_id();
        let credentials = self
            .cached_connection_credentials()
            .or(self.fetch_and_cache_connection_credentials(connection_id, "get_client_data")?);

        match credentials {
            Some((winlogon_username, winlogon_domain, password)) => {

                client_data.fInheritAutoLogon = BOOL(1);
                client_data.fUsingSavedCreds = true;
                client_data.fPromptForPassword = false;

                copy_wide(&mut client_data.UserName, &winlogon_username);
                copy_wide(&mut client_data.Domain, &winlogon_domain);
                copy_wide(&mut client_data.Password, &password);

                debug_log_line(&format!(
                    "IWRdsProtocolConnection::GetClientData filled_autologon_creds connection_id={connection_id} user={winlogon_username} domain={winlogon_domain}"
                ));
            }
            None => {
                debug_log_line(&format!(
                    "IWRdsProtocolConnection::GetClientData no_autologon_creds_yet connection_id={connection_id}"
                ));
            }
        }

        debug_log_line("IWRdsProtocolConnection::GetClientData ok");
        Ok(())
    }

    fn GetClientMonitorData(&self, pnummonitors: *mut u32, pprimarymonitor: *mut u32) -> windows_core::Result<()> {
        if !pnummonitors.is_null() {
            // SAFETY: `pnummonitors` is non-null and points to a writable buffer provided by the caller.
            unsafe { *pnummonitors = 1 };
        }

        if !pprimarymonitor.is_null() {
            // SAFETY: `pprimarymonitor` is non-null and points to a writable buffer provided by the caller.
            unsafe { *pprimarymonitor = 0 };
        }

        Ok(())
    }

    fn GetUserCredentials(&self, pusercreds: *mut WTS_USER_CREDENTIAL) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetUserCredentials called connection_id={connection_id}"
        ));

        if pusercreds.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null user credentials pointer"));
        }

        let credentials = self
            .cached_connection_credentials()
            .or(self.fetch_and_cache_connection_credentials(connection_id, "get_user_credentials")?);

        let Some((winlogon_username, winlogon_domain, password)) = credentials else {
            debug_log_line(&format!(
                "IWRdsProtocolConnection::GetUserCredentials no_credentials connection_id={connection_id}"
            ));
            return Err(windows_core::Error::new(
                E_NOTIMPL,
                "no CredSSP credentials available yet",
            ));
        };

        // SAFETY: pusercreds is non-null and points to writable memory owned by TermService.
        let creds = unsafe { &mut *pusercreds };
        *creds = WTS_USER_CREDENTIAL::default();
        copy_wide(&mut creds.UserName, &winlogon_username);
        copy_wide(&mut creds.Domain, &winlogon_domain);
        copy_wide(&mut creds.Password, &password);

        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetUserCredentials ok connection_id={connection_id} user={winlogon_username} domain={winlogon_domain}",
        ));

        Ok(())
    }

    fn GetLicenseConnection(&self) -> windows_core::Result<IWRdsProtocolLicenseConnection> {
        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetLicenseConnection called connection_id={}",
            self.inner.connection_id()
        ));
        // Return a stub license connection object.  Returning E_NOTIMPL here causes TermService
        // to close the connection immediately (it cannot proceed to IsUserAllowedToLogon without
        // completing the licensing phase).  The stub allows the licensing phase to succeed so that
        // the connection can advance to IsUserAllowedToLogon and session creation.
        let stub: IWRdsProtocolLicenseConnection = WrdsLicenseConnection.into();
        Ok(stub)
    }

    fn AuthenticateClientToSession(&self, sessionid: *mut WTS_SESSION_ID) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        debug_log_line(&format!(
            "IWRdsProtocolConnection::AuthenticateClientToSession called connection_id={connection_id}",
        ));

        // Microsoft docs: this is an `[out]` parameter (WRDS_SESSION_ID*), not an in-out hint.
        // We currently do not implement fast reconnect / session reattachment.
        if sessionid.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null session id pointer"));
        }

        // SAFETY: `sessionid` is non-null and points to a writable out-parameter buffer.
        // Initialize it defensively even when returning E_NOTIMPL to avoid leaking stale data.
        unsafe {
            *sessionid = WTS_SESSION_ID::default();
        }

        debug_log_line(&format!(
            "IWRdsProtocolConnection::AuthenticateClientToSession returning E_NOTIMPL connection_id={connection_id}",
        ));

        Err(windows_core::Error::new(
            E_NOTIMPL,
            "AuthenticateClientToSession is not implemented",
        ))
    }

    fn NotifySessionId(
        &self,
        sessionid: *const WTS_SESSION_ID,
        _sessionhandle: HANDLE_PTR,
    ) -> windows_core::Result<()> {
        if sessionid.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null session id pointer"));
        }

        // SAFETY: sessionid is non-null and points to a valid WTS_SESSION_ID provided by TermService.
        let wts_session_id = unsafe { (*sessionid).SessionId };
        debug_log_line(&format!(
            "IWRdsProtocolConnection::NotifySessionId called connection_id={} session_id={}",
            self.inner.connection_id(),
            wts_session_id
        ));
        self.inner.notify_session_id(wts_session_id).map_err(transition_error)?;

        if let Err(error) = self
            .control_bridge
            .set_capture_session_id_retried(self.inner.connection_id(), wts_session_id, "notify_session_id")
        {
            debug_log_line(&format!(
                "IWRdsProtocolConnection::NotifySessionId set_capture_session_id failed connection_id={} session_id={} error={error}",
                self.inner.connection_id(),
                wts_session_id,
            ));
        } else {
            debug_log_line(&format!(
                "IWRdsProtocolConnection::NotifySessionId set_capture_session_id ok connection_id={} session_id={}",
                self.inner.connection_id(),
                wts_session_id,
            ));
        }

        let wddm_enabled = self.wddm_idd_enabled.load(Ordering::SeqCst);
        let driver_handle_seen = self.driver_handle_raw.load(Ordering::SeqCst) != 0;
        if wddm_enabled && !driver_handle_seen {
            debug_log_line(&format!(
                "IWRdsProtocolConnection::NotifySessionId fallback notify_idd_driver_loaded session_id={wts_session_id}",
            ));
            if let Err(error) = self.control_bridge.notify_idd_driver_loaded(wts_session_id) {
                debug_log_line(&format!(
                    "IWRdsProtocolConnection::NotifySessionId fallback notify_idd_driver_loaded failed: {error}",
                ));
            }
        }

        Ok(())
    }

    fn GetInputHandles(
        &self,
        pkeyboardhandle: *mut HANDLE_PTR,
        pmousehandle: *mut HANDLE_PTR,
        pbeephandle: *mut HANDLE_PTR,
    ) -> windows_core::Result<()> {
        let keyboard_null = pkeyboardhandle.is_null();
        let mouse_null = pmousehandle.is_null();
        let beep_null = pbeephandle.is_null();

        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetInputHandles connection_id={} keyboard_null={} mouse_null={} beep_null={}",
            self.inner.connection_id(),
            keyboard_null,
            mouse_null,
            beep_null
        ));

        // Per docs, beep handle is unused and must be NULL.
        if !pbeephandle.is_null() {
            // SAFETY: non-null (checked above), valid TermService-provided pointer.
            unsafe { *pbeephandle = HANDLE_PTR::default() };
        }

        // Best-effort: provide real device handles so TermService can complete session setup.
        if !pkeyboardhandle.is_null() {
            let mut slot = self.keyboard_handle.lock();
            if slot.is_none() {
                match ComProtocolConnection::open_keyboard_device_handle() {
                    Ok(handle) => {
                        debug_log_line("GetInputHandles: opened keyboard device handle");
                        *slot = Some(handle);
                    }
                    Err(error) => {
                        debug_log_line(&format!(
                            "GetInputHandles: failed to open keyboard device handle: {error}"
                        ));

                        // Some environments deny opening keyboard device objects from the TermService process.
                        // Provide a non-null fallback handle so TermService can still advance the session.
                        match ComProtocolConnection::open_pointer_device_handle() {
                            Ok(handle) => {
                                debug_log_line("GetInputHandles: using pointer device handle as keyboard fallback");
                                *slot = Some(handle);
                            }
                            Err(fallback_error) => {
                                debug_log_line(&format!(
                                    "GetInputHandles: keyboard fallback (pointer device) failed: {fallback_error}"
                                ));
                            }
                        }
                    }
                }
            }
            let raw = slot.map(handle_to_raw_usize).unwrap_or_default();
            // SAFETY: non-null pointer, TermService-provided output.
            unsafe { *pkeyboardhandle = HANDLE_PTR(raw) };
        }

        if !pmousehandle.is_null() {
            let mut slot = self.mouse_handle.lock();
            if slot.is_none() {
                match ComProtocolConnection::open_pointer_device_handle() {
                    Ok(handle) => {
                        debug_log_line("GetInputHandles: opened pointer device handle");
                        *slot = Some(handle);
                    }
                    Err(error) => {
                        debug_log_line(&format!(
                            "GetInputHandles: failed to open pointer device handle: {error}"
                        ));
                    }
                }
            }
            let raw = slot.map(handle_to_raw_usize).unwrap_or_default();
            // SAFETY: non-null pointer, TermService-provided output.
            unsafe { *pmousehandle = HANDLE_PTR(raw) };
        }

        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetInputHandles ok connection_id={}",
            self.inner.connection_id()
        ));

        Ok(())
    }

    fn GetVideoHandle(&self) -> windows_core::Result<HANDLE_PTR> {
        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetVideoHandle called connection_id={}",
            self.inner.connection_id()
        ));

        let mut slot = self.video_handle.lock();
        if slot.is_none() {
            match ComProtocolConnection::open_device_handle(r"\\.\IronRdpIddVideo") {
                Ok(handle) => {
                    debug_log_line("GetVideoHandle: opened \\.\\IronRdpIddVideo");
                    *slot = Some(handle);
                }
                Err(error) => {
                    debug_log_line(&format!(
                        "GetVideoHandle: failed to open custom IDD: {error}, falling back to RdpVideoMiniport"
                    ));

                    match ComProtocolConnection::open_device_handle(r"\\.\RdpVideoMiniport") {
                        Ok(handle) => {
                            debug_log_line("GetVideoHandle: opened \\.\\RdpVideoMiniport (fallback)");
                            *slot = Some(handle);
                        }
                        Err(error) => {
                            debug_log_line(&format!("GetVideoHandle: failed to open fallback: {error}"));
                        }
                    }
                }
            }
        }

        Ok(HANDLE_PTR(slot.map(handle_to_raw_usize).unwrap_or_default()))
    }

    fn ConnectNotify(&self, sessionid: u32) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        debug_log_line(&format!(
            "IWRdsProtocolConnection::ConnectNotify called connection_id={connection_id} session_id={sessionid}"
        ));
        self.inner.connect_notify().map_err(transition_error)?;

        if let Err(error) = self
            .control_bridge
            .set_capture_session_id_retried(connection_id, sessionid, "connect_notify")
        {
            debug_log_line(&format!(
                "IWRdsProtocolConnection::ConnectNotify set_capture_session_id failed connection_id={connection_id} session_id={sessionid} error={error}"
            ));
        } else {
            debug_log_line(&format!(
                "IWRdsProtocolConnection::ConnectNotify set_capture_session_id ok connection_id={connection_id} session_id={sessionid}"
            ));
        }

        Ok(())
    }

    fn IsUserAllowedToLogon(
        &self,
        sessionid: u32,
        usertoken: HANDLE_PTR,
        _pdomainname: &PCWSTR,
        _pusername: &PCWSTR,
    ) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        let domain_name = if _pdomainname.is_null() {
            "<null>".to_owned()
        } else {
            // SAFETY: TermService provides a valid NUL-terminated PCWSTR for the duration of this call.
            unsafe { _pdomainname.to_string() }.unwrap_or_else(|_| "<invalid>".to_owned())
        };

        let user_name = if _pusername.is_null() {
            "<null>".to_owned()
        } else {
            // SAFETY: TermService provides a valid NUL-terminated PCWSTR for the duration of this call.
            unsafe { _pusername.to_string() }.unwrap_or_else(|_| "<invalid>".to_owned())
        };

        // Per docs, UserToken is an input token handle created by TermService/Winlogon.
        // We can allow/deny the logon here, but we must NOT treat it as an out-parameter.
        debug_log_line(&format!(
            "IWRdsProtocolConnection::IsUserAllowedToLogon called connection_id={connection_id} session_id={sessionid} usertoken=0x{:X} user={domain_name}\\{user_name}",
            usertoken.0
        ));

        // Backup path: some hosts delay or skip NotifySessionId.  IsUserAllowedToLogon already
        // carries the target session ID, so forward it to the companion now to minimize time spent
        // on guessed-session capture.
        let bridge = self.control_bridge.clone();
        thread::spawn(move || {
            match bridge.set_capture_session_id_retried(connection_id, sessionid, "is_user_allowed_to_logon") {
                Ok(()) => {
                    debug_log_line(&format!(
                        "IWRdsProtocolConnection::IsUserAllowedToLogon set_capture_session_id ok connection_id={connection_id} session_id={sessionid}",
                    ));
                }
                Err(error) => {
                    debug_log_line(&format!(
                        "IWRdsProtocolConnection::IsUserAllowedToLogon set_capture_session_id failed connection_id={connection_id} session_id={sessionid} error={error}",
                    ));
                }
            }
        });

        Ok(())
    }

    fn SessionArbitrationEnumeration(
        &self,
        _husertoken: HANDLE_PTR,
        bsinglesessionperuserenabled: BOOL,
        _psessionidarray: *mut u32,
        _pdwsessionidentifiercount: *mut u32,
    ) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsProtocolConnection::SessionArbitrationEnumeration called connection_id={} single_session={}",
            self.inner.connection_id(),
            bsinglesessionperuserenabled.as_bool()
        ));
        // Return E_NOTIMPL so TermService uses its own default session-arbitration logic.
        Err(windows_core::Error::new(
            E_NOTIMPL,
            "session arbitration uses default behavior",
        ))
    }

    fn LogonNotify(
        &self,
        _hclienttoken: HANDLE_PTR,
        _wszusername: &PCWSTR,
        _wszdomainname: &PCWSTR,
        _sessionid: *const WTS_SESSION_ID,
        _pwrdsconnectionsettings: *mut WRDS_CONNECTION_SETTINGS,
    ) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        debug_log_line(&format!(
            "IWRdsProtocolConnection::LogonNotify called connection_id={}",
            connection_id
        ));

        if !_sessionid.is_null() {
            // SAFETY: `_sessionid` is checked non-null and points to a TermService-provided
            // WTS_SESSION_ID valid for this callback invocation.
            let session_id = unsafe { (*_sessionid).SessionId };

            let bridge = self.control_bridge.clone();
            thread::spawn(move || {
                match bridge.set_capture_session_id_retried(connection_id, session_id, "logon_notify") {
                    Ok(()) => {
                        debug_log_line(&format!(
                            "IWRdsProtocolConnection::LogonNotify set_capture_session_id ok connection_id={connection_id} session_id={session_id}",
                        ));
                    }
                    Err(error) => {
                        debug_log_line(&format!(
                            "IWRdsProtocolConnection::LogonNotify set_capture_session_id failed connection_id={connection_id} session_id={session_id} error={error}",
                        ));
                    }
                }
            });
        }

        self.inner.logon_notify().map_err(transition_error)?;
        Ok(())
    }

    fn PreDisconnect(&self, disconnectreason: u32) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsProtocolConnection::PreDisconnect called connection_id={} reason={}",
            self.inner.connection_id(),
            disconnectreason
        ));
        Ok(())
    }

    fn DisconnectNotify(&self) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsProtocolConnection::DisconnectNotify called connection_id={}",
            self.inner.connection_id()
        ));
        self.inner.disconnect_notify().map_err(transition_error)?;

        if let Err(error) = self.control_bridge.close_connection(self.inner.connection_id()) {
            warn!(%error, connection_id = self.inner.connection_id(), "Failed to notify companion service on disconnect");
        }

        *self.ready_notified.lock() = false;
        self.close_device_handles();
        self.release_virtual_channel_forwarders();
        self.release_virtual_channels();
        self.release_connection_callback();
        Ok(())
    }

    fn Close(&self) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsProtocolConnection::Close called connection_id={}",
            self.inner.connection_id()
        ));

        if let Err(error) = self.control_bridge.close_connection(self.inner.connection_id()) {
            warn!(%error, connection_id = self.inner.connection_id(), "Failed to notify companion service on close");
        }

        *self.ready_notified.lock() = false;
        self.close_device_handles();
        self.release_virtual_channel_forwarders();
        self.release_virtual_channels();
        self.release_connection_callback();
        self.inner.close().map_err(transition_error)?;
        Ok(())
    }

    fn GetProtocolStatus(&self, pprotocolstatus: *mut WTS_PROTOCOL_STATUS) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetProtocolStatus called connection_id={}",
            self.inner.connection_id()
        ));

        if pprotocolstatus.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null protocol status pointer"));
        }

        // SAFETY: `pprotocolstatus` is non-null (checked above) and points to a writable buffer.
        let status = unsafe { &mut *pprotocolstatus };
        *status = WTS_PROTOCOL_STATUS::default();

        // Keep ProtocolType/Length aligned with non-RDP provider mode and use
        // a conservative baseline for the opaque `Specific` counters.
        status.Output.ProtocolType = WTS_PROTOCOL_TYPE_NON_RDP;
        status.Output.Length =
            u16::try_from(size_of::<windows::Win32::System::RemoteDesktop::WTS_PROTOCOL_COUNTERS>()).unwrap_or(0);
        status.Output.Specific = 0;
        status.Input.ProtocolType = WTS_PROTOCOL_TYPE_NON_RDP;
        status.Input.Length = status.Output.Length;
        status.Input.Specific = status.Output.Specific;

        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetProtocolStatus returned connection_id={} output={{type={},len={},specific={}}} input={{type={},len={},specific={}}}",
            self.inner.connection_id(),
            status.Output.ProtocolType,
            status.Output.Length,
            status.Output.Specific,
            status.Input.ProtocolType,
            status.Input.Length,
            status.Input.Specific,
        ));

        Ok(())
    }

    fn GetLastInputTime(&self) -> windows_core::Result<u64> {
        Ok(*self.last_input_time.lock())
    }

    fn SetErrorInfo(&self, ulerror: u32) -> windows_core::Result<()> {
        warn!(ulerror, "Received protocol error info");
        Ok(())
    }

    fn CreateVirtualChannel(
        &self,
        szendpointname: &PCSTR,
        bstatic: BOOL,
        requestedpriority: u32,
    ) -> windows_core::Result<usize> {
        let session_id = self
            .inner
            .session_id()
            .ok_or_else(|| windows_core::Error::new(E_UNEXPECTED, "session id is not available for virtual channel"))?;

        let endpoint = *szendpointname;
        if endpoint.is_null() {
            return Err(windows_core::Error::new(
                E_POINTER,
                "null virtual channel endpoint pointer",
            ));
        }

        let is_static = bstatic.as_bool();
        // SAFETY: `endpoint` is non-null (checked above) and points to a NUL-terminated
        // string provided by the caller.
        let endpoint_name = match unsafe { endpoint.to_string() } {
            Ok(name) => Some(name),
            Err(error) => {
                warn!(%error, "Failed to decode virtual channel endpoint name");
                None
            }
        };

        let hook_target = endpoint_name
            .as_deref()
            .and_then(|name| ironrdp_virtual_channel_server(name, is_static));
        let bridge_plan = VirtualChannelBridgePlan::for_endpoint(is_static, hook_target);

        let effective_priority = virtual_channel_requested_priority(is_static, requestedpriority, hook_target);

        if let Some(target) = hook_target {
            if target.requires_drdynvc_backbone() {
                if let Err(error) = self.ensure_ironrdp_drdynvc_channel(session_id) {
                    warn!(
                        %error,
                        endpoint = target.name(),
                        "Failed to pre-open DRDYNVC backbone for IronRDP dynamic channel"
                    );
                }
            }
        }

        if let Some(name) = endpoint_name.as_deref() {
            if let Some((existing_channel, existing_bridge_plan)) = self.find_virtual_channel(name, is_static) {
                debug!(
                    session_id,
                    endpoint = name,
                    static_channel = is_static,
                    route_kind = ?existing_bridge_plan.route_kind,
                    "Reusing virtual channel handle"
                );
                return Ok(handle_to_raw_usize(existing_channel));
            }
        }

        let flags = virtual_channel_open_flags(is_static, effective_priority);

        let channel = if let Some(name) = endpoint_name.as_deref() {
            self.open_virtual_channel_by_name(session_id, name, is_static, effective_priority, bridge_plan)?
        } else {
            // SAFETY: `endpoint` points to a valid NUL-terminated string for the duration of the call.
            let channel = unsafe { WTSVirtualChannelOpenEx(session_id, endpoint, flags) }?;
            self.register_virtual_channel(channel, None, is_static, bridge_plan)?
        };

        if bridge_plan.should_prepare_forwarding() {
            info!(
                session_id,
                static_channel = is_static,
                route_kind = ?bridge_plan.route_kind,
                preferred_dynamic_priority = bridge_plan.preferred_dynamic_priority,
                "Prepared virtual channel forwarding metadata"
            );
        }

        if let Some(target) = hook_target {
            info!(
                session_id,
                endpoint = target.name(),
                static_channel = is_static,
                requestedpriority,
                effective_priority,
                flags,
                "Hooked IronRDP virtual channel server endpoint"
            );
        } else {
            debug!(
                session_id,
                requestedpriority,
                effective_priority,
                static_channel = is_static,
                flags,
                "Created virtual channel"
            );
        }

        Ok(handle_to_raw_usize(channel))
    }

    fn QueryProperty(
        &self,
        _querytype: &GUID,
        _ulnumentriesin: u32,
        _ulnumentriesout: u32,
        _ppropertyentriesin: *const WTS_PROPERTY_VALUE,
        _ppropertyentriesout: *mut WTS_PROPERTY_VALUE,
    ) -> windows_core::Result<()> {
        if _ulnumentriesout == 0 {
            debug_log_line(&format!(
                "IWRdsProtocolConnection::QueryProperty called connection_id={} querytype={:?} in={} out=0",
                self.inner.connection_id(),
                _querytype,
                _ulnumentriesin,
            ));
            return Ok(());
        }

        if _ppropertyentriesout.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null property output pointer"));
        }

        let out_len = usize::try_from(_ulnumentriesout).unwrap_or(0);
        if out_len == 0 {
            return Ok(());
        }

        // SAFETY: pointer is non-null (checked above) and points to at least one output entry.
        let requested_type = unsafe { (*_ppropertyentriesout).Type };

        let is_known_querytype = *_querytype == PROPERTY_TYPE_GET_FAST_RECONNECT
            || *_querytype == PROPERTY_TYPE_GET_FAST_RECONNECT_USER_SID
            || *_querytype == PROPERTY_TYPE_ENABLE_UNIVERSAL_APPS_FOR_CUSTOM_SHELL
            || *_querytype == PROPERTY_TYPE_CONNECTION_GUID
            || *_querytype == PROPERTY_TYPE_SUPPRESS_LOGON_UI
            || *_querytype == PROPERTY_TYPE_CAPTURE_PROTECTED_CONTENT
            || *_querytype == PROPERTY_TYPE_LICENSE_GUID
            || *_querytype == WTS_QUERY_AUDIOENUM_DLL;

        debug_log_line(&format!(
            "IWRdsProtocolConnection::QueryProperty called connection_id={} querytype={:?} in={} out={} out0_type_in={requested_type}",
            self.inner.connection_id(),
            _querytype,
            _ulnumentriesin,
            _ulnumentriesout,
        ));

        // SAFETY: TermService provides `_ulnumentriesout` writable entries.
        let out = unsafe { core::slice::from_raw_parts_mut(_ppropertyentriesout, out_len) };
        for entry in out.iter_mut() {
            *entry = WTS_PROPERTY_VALUE::default();
        }

        // `wtsdefs.h` documents QueryProperty GUIDs and the expected output types.
        // TermService appears to treat invalid `Type` values as a hard failure.
        let first = &mut out[0];

        if *_querytype == PROPERTY_TYPE_GET_FAST_RECONNECT {
            first.Type = WTS_VALUE_TYPE_ULONG;
            first.u.ulVal = FAST_RECONNECT_ENHANCED;
            debug_log_line(&format!(
                "IWRdsProtocolConnection::QueryProperty PROPERTY_TYPE_GET_FAST_RECONNECT -> {FAST_RECONNECT_ENHANCED}",
            ));
            return Ok(());
        }

        if *_querytype == PROPERTY_TYPE_CONNECTION_GUID {
            debug_log_line("IWRdsProtocolConnection::QueryProperty PROPERTY_TYPE_CONNECTION_GUID -> E_NOTIMPL");
            return Err(E_NOTIMPL.into());
        }

        if *_querytype == PROPERTY_TYPE_SUPPRESS_LOGON_UI {
            first.Type = WTS_VALUE_TYPE_ULONG;
            first.u.ulVal = 0;
            debug_log_line("IWRdsProtocolConnection::QueryProperty PROPERTY_TYPE_SUPPRESS_LOGON_UI -> 0");
            return Ok(());
        }

        if *_querytype == PROPERTY_TYPE_CAPTURE_PROTECTED_CONTENT {
            debug_log_line("IWRdsProtocolConnection::QueryProperty PROPERTY_TYPE_CAPTURE_PROTECTED_CONTENT -> E_NOTIMPL");
            return Err(E_NOTIMPL.into());
        }

        if *_querytype == PROPERTY_TYPE_LICENSE_GUID {
            let license_guid = deterministic_license_guid(self.inner.connection_id());
            first.Type = WTS_VALUE_TYPE_GUID;
            first.u.guidVal = license_guid;
            debug_log_line(&format!(
                "IWRdsProtocolConnection::QueryProperty PROPERTY_TYPE_LICENSE_GUID -> {license_guid:?}",
            ));
            return Ok(());
        }

        if *_querytype == PROPERTY_TYPE_GET_FAST_RECONNECT_USER_SID {
            let connection_id = self.inner.connection_id();
            let sid = self
                .control_bridge
                .get_connection_credentials(connection_id)?
                .and_then(|(username, domain, _password)| {
                    let (winlogon_username, winlogon_domain) = normalize_winlogon_credentials(&username, &domain);
                    lookup_account_sid_string(&winlogon_username, &winlogon_domain).ok()
                })
                .unwrap_or_default();

            let sid_w: Vec<u16> = sid.encode_utf16().chain(core::iter::once(0)).collect();
            let bytes = sid_w.len().saturating_mul(size_of::<u16>());

            // SAFETY: CoTaskMemAlloc returns a valid pointer or null; TermService is responsible for freeing.
            let mem = unsafe { CoTaskMemAlloc(bytes) };
            if mem.is_null() {
                return Err(windows_core::Error::new(E_OUTOFMEMORY, "CoTaskMemAlloc failed"));
            }

            // SAFETY: `mem` is at least `bytes` bytes, and `sid_w` has the same length in u16s.
            unsafe {
                core::ptr::copy_nonoverlapping(sid_w.as_ptr(), mem.cast::<u16>(), sid_w.len());
            }

            first.Type = WTS_VALUE_TYPE_STRING;
            first.u.strVal.size = u32::try_from(sid_w.len()).unwrap_or(0);
            first.u.strVal.pstrVal = PWSTR(mem.cast());

            debug_log_line(&format!(
                "IWRdsProtocolConnection::QueryProperty PROPERTY_TYPE_GET_FAST_RECONNECT_USER_SID -> {sid}",
            ));
            return Ok(());
        }

        if *_querytype == PROPERTY_TYPE_ENABLE_UNIVERSAL_APPS_FOR_CUSTOM_SHELL {
            // `wtsdefs.h`: 0 = don't enable, 1 = enable.
            first.Type = WTS_VALUE_TYPE_ULONG;
            first.u.ulVal = 0;
            debug_log_line(
                "IWRdsProtocolConnection::QueryProperty PROPERTY_TYPE_ENABLE_UNIVERSAL_APPS_FOR_CUSTOM_SHELL -> 0",
            );
            return Ok(());
        }

        if *_querytype == WTS_QUERY_AUDIOENUM_DLL {
            // `wtsdefs.h`: used by audio enumeration / Media Foundation components.
            // We don't provide an audio-enum helper DLL; return an empty string.
            let empty_w: [u16; 1] = [0];
            let bytes = size_of::<u16>();

            // SAFETY: CoTaskMemAlloc returns a valid pointer or null; TermService is responsible for freeing.
            let mem = unsafe { CoTaskMemAlloc(bytes) };
            if mem.is_null() {
                return Err(windows_core::Error::new(E_OUTOFMEMORY, "CoTaskMemAlloc failed"));
            }

            // SAFETY: `mem` is at least `bytes` bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(empty_w.as_ptr(), mem.cast::<u16>(), empty_w.len());
            }

            first.Type = WTS_VALUE_TYPE_STRING;
            first.u.strVal.size = 1;
            first.u.strVal.pstrVal = PWSTR(mem.cast());

            debug_log_line("IWRdsProtocolConnection::QueryProperty WTS_QUERY_AUDIOENUM_DLL -> ''");
            return Ok(());
        }

        if !is_known_querytype {
            debug_log_line(&format!(
                "IWRdsProtocolConnection::QueryProperty unknown querytype={_querytype:?} requested_type={requested_type} -> E_NOTIMPL",
            ));
        }

        Err(windows_core::Error::new(
            E_NOTIMPL,
            "QueryProperty querytype is not implemented",
        ))
    }

    fn GetShadowConnection(&self) -> windows_core::Result<IWRdsProtocolShadowConnection> {
        Err(windows_core::Error::new(
            E_NOTIMPL,
            "shadow connection is not implemented",
        ))
    }

    fn NotifyCommandProcessCreated(&self, sessionid: u32) -> windows_core::Result<()> {
        let has_userinit = session_has_process(sessionid, "userinit.exe");
        let has_explorer = session_has_process(sessionid, "explorer.exe");
        let has_logonui = session_has_process(sessionid, "LogonUI.exe");
        let has_winlogon = session_has_process(sessionid, "winlogon.exe");

        debug_log_line(&format!(
            "IWRdsProtocolConnection::NotifyCommandProcessCreated called connection_id={} session_id={sessionid} userinit={} explorer={} logonui={} winlogon={}",
            self.inner.connection_id(),
            has_userinit,
            has_explorer,
            has_logonui,
            has_winlogon,
        ));
        Ok(())
    }
}

fn copy_wide<const N: usize>(target: &mut [u16; N], value: &str) {
    let mut utf16 = value.encode_utf16().take(N.saturating_sub(1));

    for (index, code_unit) in utf16.by_ref().enumerate() {
        target[index] = code_unit;
    }
}

fn transition_error(message: &'static str) -> windows_core::Error {
    windows_core::Error::new(E_UNEXPECTED, message)
}

fn io_error_to_windows_error(error: std::io::Error, context: &'static str) -> windows_core::Error {
    windows_core::Error::new(E_UNEXPECTED, format!("{context}: {error}"))
}

#[expect(clippy::as_conversions)]
fn handle_to_raw_usize(handle: HANDLE) -> usize {
    handle.0 as usize
}

#[expect(clippy::as_conversions)]
fn raw_usize_to_handle(raw: usize) -> HANDLE {
    HANDLE(raw as *mut core::ffi::c_void)
}

#[derive(Debug)]
struct VirtualChannelHandle {
    handle: HANDLE,
    endpoint_name: Option<String>,
    static_channel: bool,
    bridge_plan: VirtualChannelBridgePlan,
}

struct VirtualChannelForwarderWorker {
    endpoint: VirtualChannelBridgeEndpoint,
    stop_tx: mpsc::Sender<()>,
    join_handle: thread::JoinHandle<()>,
}

impl VirtualChannelHandle {
    fn new(
        handle: HANDLE,
        endpoint_name: Option<String>,
        static_channel: bool,
        bridge_plan: VirtualChannelBridgePlan,
    ) -> Self {
        Self {
            handle,
            endpoint_name,
            static_channel,
            bridge_plan,
        }
    }

    fn raw(&self) -> HANDLE {
        self.handle
    }

    fn matches(&self, endpoint_name: &str, is_static: bool) -> bool {
        self.static_channel == is_static
            && self
                .endpoint_name
                .as_deref()
                .is_some_and(|name| endpoint_name_eq(name, endpoint_name))
    }
}

impl Drop for VirtualChannelHandle {
    fn drop(&mut self) {
        // SAFETY: `self.handle` is a handle returned by `WTSVirtualChannelOpenEx`.
        if let Err(error) = unsafe { WTSVirtualChannelClose(self.handle) } {
            warn!(%error, "Failed to close virtual channel handle");
        }
    }
}

fn virtual_channel_open_flags(is_static: bool, requested_priority: u32) -> u32 {
    if is_static {
        return 0;
    }

    let dynamic_priority = match requested_priority {
        WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        | WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED
        | WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH
        | WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL => requested_priority,
        _ => WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW,
    };

    WTS_CHANNEL_OPTION_DYNAMIC | dynamic_priority
}

fn run_virtual_channel_forwarder(
    channel_handle_raw: usize,
    endpoint: VirtualChannelBridgeEndpoint,
    bridge_handler: SharedVirtualChannelBridgeHandler,
    outbound_rx: mpsc::Receiver<Vec<u8>>,
    stop_rx: mpsc::Receiver<()>,
) {
    let channel_handle = raw_usize_to_handle(channel_handle_raw);
    let mut read_buffer = vec![0u8; VIRTUAL_CHANNEL_FORWARDER_BUFFER_SIZE];

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        while let Ok(payload) = outbound_rx.try_recv() {
            let mut bytes_written = 0;
            // SAFETY: `channel_handle` is a live virtual channel handle, and `payload` points to
            // a valid buffer for the duration of the call.
            if let Err(error) = unsafe { WTSVirtualChannelWrite(channel_handle, &payload, &mut bytes_written) } {
                warn!(
                    endpoint = %endpoint.endpoint_name,
                    ?error,
                    "Failed to write virtual channel payload"
                );
                break;
            }
        }

        let mut bytes_read = 0;
        // SAFETY: `channel_handle` is a live virtual channel handle. `read_buffer` and `bytes_read`
        // are valid out-buffers for the duration of the call.
        match unsafe {
            WTSVirtualChannelRead(
                channel_handle,
                VIRTUAL_CHANNEL_FORWARDER_READ_TIMEOUT_MS,
                &mut read_buffer,
                &mut bytes_read,
            )
        } {
            Ok(()) => {
                if bytes_read == 0 {
                    continue;
                }

                let Ok(read_len) = usize::try_from(bytes_read) else {
                    warn!(
                        endpoint = %endpoint.endpoint_name,
                        bytes_read,
                        "Virtual channel forwarder read length does not fit in usize"
                    );
                    break;
                };
                bridge_handler.on_channel_data(&endpoint, &read_buffer[..read_len]);
            }
            Err(error) => {
                if is_virtual_channel_read_timeout(&error) {
                    continue;
                }

                warn!(
                    endpoint = %endpoint.endpoint_name,
                    ?error,
                    "Virtual channel forwarder read failed"
                );
                break;
            }
        }
    }

    bridge_handler.on_channel_closed(&endpoint);
}

fn is_virtual_channel_read_timeout(error: &windows_core::Error) -> bool {
    let code = error.code();

    code == HRESULT::from_win32(ERROR_SEM_TIMEOUT.0)
        || code == HRESULT::from_win32(ERROR_IO_INCOMPLETE.0)
        || code == HRESULT::from_win32(ERROR_NO_DATA.0)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::{
        bridge_pipe_path, drain_length_prefixed_pipe_frames, ironrdp_virtual_channel_server, sanitize_pipe_segment,
        virtual_channel_open_flags, virtual_channel_requested_priority, IronRdpVirtualChannelServer,
        VirtualChannelBridgeEndpoint, VirtualChannelBridgePlan, VirtualChannelBridgeTx, VirtualChannelRouteKind,
        VIRTUAL_CHANNEL_PIPE_BRIDGE_MAX_FRAME_SIZE,
    };
    use windows::Win32::System::RemoteDesktop::{
        WTS_CHANNEL_OPTION_DYNAMIC, WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH, WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW,
        WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED, WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL,
    };

    #[test]
    fn static_channels_use_zero_flags() {
        assert_eq!(virtual_channel_open_flags(true, 123), 0);
    }

    #[test]
    fn dynamic_channels_map_priority_flags() {
        assert_eq!(
            virtual_channel_open_flags(false, WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        );
        assert_eq!(
            virtual_channel_open_flags(false, WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED
        );
        assert_eq!(
            virtual_channel_open_flags(false, WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH
        );
        assert_eq!(
            virtual_channel_open_flags(false, WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL
        );
    }

    #[test]
    fn dynamic_channels_fallback_to_low_for_unknown_priority() {
        assert_eq!(
            virtual_channel_open_flags(false, 1),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        );
        assert_eq!(
            virtual_channel_open_flags(false, 123),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        );
        assert_eq!(
            virtual_channel_open_flags(false, u32::MAX),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        );
    }

    #[test]
    fn recognizes_ironrdp_static_virtual_channel_servers() {
        assert_eq!(
            ironrdp_virtual_channel_server("cliprdr", true),
            Some(IronRdpVirtualChannelServer::Cliprdr)
        );
        assert_eq!(
            ironrdp_virtual_channel_server("RDPSND", true),
            Some(IronRdpVirtualChannelServer::Rdpsnd)
        );
        assert_eq!(
            ironrdp_virtual_channel_server("DrDyNvC", true),
            Some(IronRdpVirtualChannelServer::Drdynvc)
        );
        assert_eq!(ironrdp_virtual_channel_server("cliprdr", false), None);
    }

    #[test]
    fn recognizes_ironrdp_dynamic_virtual_channel_servers() {
        assert_eq!(
            ironrdp_virtual_channel_server("Microsoft::Windows::RDS::DisplayControl", false),
            Some(IronRdpVirtualChannelServer::DisplayControl)
        );
        assert_eq!(
            ironrdp_virtual_channel_server("microsoft::windows::rds::graphics", false),
            Some(IronRdpVirtualChannelServer::Graphics)
        );
        assert_eq!(
            ironrdp_virtual_channel_server("FreeRDP::Advanced::Input", false),
            Some(IronRdpVirtualChannelServer::AdvancedInput)
        );
        assert_eq!(
            ironrdp_virtual_channel_server("echo", false),
            Some(IronRdpVirtualChannelServer::Echo)
        );
    }

    #[test]
    fn ironrdp_dynamic_channels_use_recommended_priorities_when_unknown() {
        assert_eq!(
            virtual_channel_requested_priority(false, 0, Some(IronRdpVirtualChannelServer::DisplayControl)),
            WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED
        );
        assert_eq!(
            virtual_channel_requested_priority(false, 0, Some(IronRdpVirtualChannelServer::Graphics)),
            WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH
        );
        assert_eq!(
            virtual_channel_requested_priority(false, 0, Some(IronRdpVirtualChannelServer::AdvancedInput)),
            WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL
        );
        assert_eq!(
            virtual_channel_requested_priority(false, 0, Some(IronRdpVirtualChannelServer::Echo)),
            WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        );
    }

    #[test]
    fn explicit_dynamic_priority_is_preserved() {
        assert_eq!(
            virtual_channel_requested_priority(
                false,
                WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED,
                Some(IronRdpVirtualChannelServer::AdvancedInput)
            ),
            WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED
        );
    }

    #[test]
    fn dynamic_server_backbone_requirements_are_exposed() {
        assert!(IronRdpVirtualChannelServer::DisplayControl.requires_drdynvc_backbone());
        assert!(IronRdpVirtualChannelServer::Graphics.requires_drdynvc_backbone());
        assert!(IronRdpVirtualChannelServer::AdvancedInput.requires_drdynvc_backbone());
        assert!(IronRdpVirtualChannelServer::Echo.requires_drdynvc_backbone());
        assert!(!IronRdpVirtualChannelServer::Cliprdr.requires_drdynvc_backbone());
        assert!(!IronRdpVirtualChannelServer::Rdpsnd.requires_drdynvc_backbone());
        assert!(!IronRdpVirtualChannelServer::Drdynvc.requires_drdynvc_backbone());
    }

    #[test]
    fn bridge_plan_classifies_known_ironrdp_routes() {
        assert_eq!(
            VirtualChannelBridgePlan::for_endpoint(true, Some(IronRdpVirtualChannelServer::Cliprdr)).route_kind,
            VirtualChannelRouteKind::IronRdpStatic
        );
        assert_eq!(
            VirtualChannelBridgePlan::for_endpoint(true, Some(IronRdpVirtualChannelServer::Drdynvc)).route_kind,
            VirtualChannelRouteKind::IronRdpDynamicBackbone
        );
        assert_eq!(
            VirtualChannelBridgePlan::for_endpoint(false, Some(IronRdpVirtualChannelServer::DisplayControl)).route_kind,
            VirtualChannelRouteKind::IronRdpDynamicEndpoint
        );
        assert_eq!(
            VirtualChannelBridgePlan::for_endpoint(false, None).route_kind,
            VirtualChannelRouteKind::Unknown
        );
    }

    #[test]
    fn bridge_plan_exposes_forwarding_preparation_and_priority() {
        let unknown_plan = VirtualChannelBridgePlan::for_endpoint(false, None);
        assert!(!unknown_plan.should_prepare_forwarding());
        assert_eq!(unknown_plan.preferred_dynamic_priority, None);

        let display_plan =
            VirtualChannelBridgePlan::for_endpoint(false, Some(IronRdpVirtualChannelServer::DisplayControl));
        assert!(display_plan.should_prepare_forwarding());
        assert_eq!(
            display_plan.preferred_dynamic_priority,
            Some(WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED)
        );

        let static_plan = VirtualChannelBridgePlan::for_endpoint(true, Some(IronRdpVirtualChannelServer::Rdpsnd));
        assert!(static_plan.should_prepare_forwarding());
        assert_eq!(static_plan.preferred_dynamic_priority, None);
    }

    #[test]
    fn sanitize_pipe_segment_normalizes_name() {
        assert_eq!(sanitize_pipe_segment("ClipRdr"), "cliprdr");
        assert_eq!(
            sanitize_pipe_segment("Microsoft::Windows::RDS::Graphics"),
            "microsoft__windows__rds__graphics"
        );
        assert_eq!(sanitize_pipe_segment(""), "channel");
    }

    #[test]
    fn bridge_pipe_path_uses_svc_and_dvc_suffixes() {
        let static_endpoint = VirtualChannelBridgeEndpoint {
            endpoint_name: "cliprdr".to_owned(),
            static_channel: true,
            route_kind: VirtualChannelRouteKind::IronRdpStatic,
        };

        let dynamic_endpoint = VirtualChannelBridgeEndpoint {
            endpoint_name: "Microsoft::Windows::RDS::Graphics".to_owned(),
            static_channel: false,
            route_kind: VirtualChannelRouteKind::IronRdpDynamicEndpoint,
        };

        assert_eq!(
            bridge_pipe_path("IronRdpVcBridge", &static_endpoint),
            r"\\.\pipe\IronRdpVcBridge.svc.cliprdr"
        );
        assert_eq!(
            bridge_pipe_path(r"\\.\pipe\Bridge", &dynamic_endpoint),
            r"\\.\pipe\Bridge.dvc.microsoft__windows__rds__graphics"
        );
    }

    #[test]
    fn drain_length_prefixed_frames_forwards_payloads() {
        let endpoint = VirtualChannelBridgeEndpoint {
            endpoint_name: "ECHO".to_owned(),
            static_channel: false,
            route_kind: VirtualChannelRouteKind::IronRdpDynamicEndpoint,
        };
        let (outbound_tx, outbound_rx) = mpsc::sync_channel(4);
        let bridge_tx = VirtualChannelBridgeTx {
            endpoint: endpoint.clone(),
            outbound_tx,
        };

        let mut framed = Vec::new();
        framed.extend_from_slice(&(3u32).to_le_bytes());
        framed.extend_from_slice(b"abc");
        framed.extend_from_slice(&(2u32).to_le_bytes());
        framed.extend_from_slice(b"de");

        drain_length_prefixed_pipe_frames(&endpoint, &bridge_tx, &mut framed).expect("framed payload should parse");

        assert!(framed.is_empty());
        assert_eq!(outbound_rx.try_recv().expect("first payload"), b"abc");
        assert_eq!(outbound_rx.try_recv().expect("second payload"), b"de");
    }

    #[test]
    fn drain_length_prefixed_frames_rejects_oversized_frame() {
        let endpoint = VirtualChannelBridgeEndpoint {
            endpoint_name: "ECHO".to_owned(),
            static_channel: false,
            route_kind: VirtualChannelRouteKind::IronRdpDynamicEndpoint,
        };
        let (outbound_tx, _outbound_rx) = mpsc::sync_channel(1);
        let bridge_tx = VirtualChannelBridgeTx {
            endpoint: endpoint.clone(),
            outbound_tx,
        };

        let mut framed = Vec::new();
        let oversized_len =
            u32::try_from(VIRTUAL_CHANNEL_PIPE_BRIDGE_MAX_FRAME_SIZE).expect("max frame size should fit in u32") + 1;
        framed.extend_from_slice(&oversized_len.to_le_bytes());

        let error = drain_length_prefixed_pipe_frames(&endpoint, &bridge_tx, &mut framed)
            .expect_err("oversized frame should fail");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
}
