//! One-shot DVC COM plugin request helper.
//!
//! Used by the pure-Rust RDPEWA backend when a host omits `clientDataJSON` (hash-only MS-RDPEWA).
//! Public WebAuthN* APIs cannot complete those ceremonies; `webauthn.dll`'s IWTS plugin path can.

use core::cell::RefCell;
use core::time::Duration;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc as std_mpsc;
use std::thread;

use tracing::{debug, info, warn};
use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG, MAX_PATH};
use windows::Win32::Networking::WindowsWebServices::WebAuthNCancelCurrentOperation;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::RemoteDesktop::{IWTSVirtualChannel, IWTSVirtualChannel_Impl, IWTSVirtualChannelCallback};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::core::{BSTR, Error, GUID, IUnknown, Ref, Result as WinResult};
use windows_core::{BOOL, implement};

use crate::channel::initialize_plugin_on_thread;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Structured oneshot failure so callers can avoid unsafe public-API fallback after timeout/cancel.
#[derive(Debug, Clone)]
pub struct PluginRequestError {
    pub hresult: u32,
    pub message: &'static str,
    /// `true` only when the plugin never started a ceremony (load/init failures).
    pub allow_public_fallback: bool,
}

impl core::fmt::Display for PluginRequestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} (HRESULT 0x{:08X})", self.message, self.hresult)
    }
}

impl core::error::Error for PluginRequestError {}

impl PluginRequestError {
    fn load(message: &'static str) -> Self {
        Self {
            hresult: hresult_to_u32(E_FAIL),
            message,
            allow_public_fallback: true,
        }
    }

    fn fail(hresult: u32, message: &'static str) -> Self {
        Self {
            hresult,
            message,
            allow_public_fallback: false,
        }
    }

    fn from_dynamic(message: String, allow_public_fallback: bool) -> Self {
        // Keep a static message for the wire/handler path; full detail goes to logs.
        warn!(%message, allow_public_fallback, "DVC COM oneshot error");
        Self {
            hresult: hresult_to_u32(E_FAIL),
            message: if allow_public_fallback {
                "COM plugin request failed before ceremony"
            } else {
                "COM plugin request failed"
            },
            allow_public_fallback,
        }
    }
}

/// Run a single request through a DVC COM plugin channel and return the plugin's `Write` payload.
///
/// This mirrors MSTSC's `webauthn.dll` path: open the named channel, deliver `request` via
/// `OnDataReceived`, and capture the bytes the plugin writes back on `IWTSVirtualChannel::Write`.
///
/// On timeout, best-effort cancels the in-flight WebAuthn operation using `cancellation_id` (16-byte
/// Windows GUID layout) before returning so callers do not start a second prompt.
///
/// # Errors
///
/// Returns a structured error when the plugin cannot be loaded, rejects the channel, fails the
/// request, or does not produce a response before `timeout`.
pub fn process_plugin_request(
    dll_path: &Path,
    channel_name: &str,
    request: &[u8],
    cancellation_id: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>, PluginRequestError> {
    let dll_path = dll_path.to_path_buf();
    let channel_name = channel_name.to_owned();
    let request = request.to_vec();
    let cancel_guid = guid_from_bytes(cancellation_id);

    let (tx, rx) = std_mpsc::sync_channel(1);
    let handle = thread::Builder::new()
        .name("dvc-com-oneshot".into())
        .spawn(move || {
            let result = run_oneshot_on_com_thread(&dll_path, &channel_name, &request);
            let _ = tx.send(result);
        })
        .map_err(|e| {
            warn!(error = %e, "failed to spawn COM oneshot thread");
            PluginRequestError::load("failed to spawn COM oneshot thread")
        })?;

    match rx.recv_timeout(timeout) {
        Ok(Ok(bytes)) => {
            let _ = handle.join();
            Ok(bytes)
        }
        Ok(Err(err)) => {
            let _ = handle.join();
            Err(err)
        }
        Err(_) => {
            warn!(?timeout, "COM plugin request timed out; cancelling in-flight operation");
            if let Some(guid) = cancel_guid {
                // SAFETY: cancel GUID is a plain value; WebAuthNCancelCurrentOperation is process-wide.
                let _ = unsafe { WebAuthNCancelCurrentOperation(&guid) };
            }
            // Do not join indefinitely: OnDataReceived may still be inside modal UI until cancel
            // lands. Detach the worker and refuse public-API fallback.
            drop(handle);
            Err(PluginRequestError::fail(
                0x8000_4004, /* E_ABORT */
                "COM plugin request timed out",
            ))
        }
    }
}

/// Convenience wrapper for System32 `webauthn.dll` / `WebAuthN_Channel`.
pub fn process_webauthn_dll_request(request: &[u8], cancellation_id: &[u8]) -> Result<Vec<u8>, PluginRequestError> {
    let dll = system_webauthn_dll_path().ok_or_else(|| PluginRequestError::load("failed to resolve System32 path"))?;
    process_plugin_request(&dll, "WebAuthN_Channel", request, cancellation_id, DEFAULT_TIMEOUT)
}

/// Resolve `%SystemRoot%\System32\webauthn.dll` via `GetSystemDirectoryW`.
pub fn system_webauthn_dll_path() -> Option<PathBuf> {
    let capacity = usize::try_from(MAX_PATH).ok()?;
    let mut buf = vec![0u16; capacity];
    // SAFETY: buffer length is in wide chars; API writes a NUL-terminated path.
    let len = unsafe { GetSystemDirectoryW(Some(&mut buf)) };
    let len = usize::try_from(len).ok()?;
    if len == 0 || len >= buf.len() {
        return None;
    }
    buf.truncate(len);
    let mut path = PathBuf::from(String::from_utf16_lossy(&buf));
    path.push("webauthn.dll");
    Some(path)
}

fn run_oneshot_on_com_thread(
    dll_path: &Path,
    channel_name: &str,
    request: &[u8],
) -> Result<Vec<u8>, PluginRequestError> {
    // SAFETY: initialize a STA apartment for the duration of this oneshot thread.
    let should_uninit = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();

    let result = run_oneshot_body(dll_path, channel_name, request);

    // SAFETY: balance a successful CoInitializeEx.
    if should_uninit {
        unsafe { CoUninitialize() };
    }
    result
}

fn run_oneshot_body(dll_path: &Path, channel_name: &str, request: &[u8]) -> Result<Vec<u8>, PluginRequestError> {
    let (plugin, _manager, listeners) = initialize_plugin_on_thread(dll_path).map_err(|e| {
        warn!(error = %e, "initialize_plugin_on_thread failed");
        PluginRequestError::from_dynamic(e, true)
    })?;

    // SAFETY: COM objects live on this thread for the duration of the oneshot.
    let _ = unsafe { plugin.Connected() };

    let listener = listeners
        .get(channel_name)
        .ok_or_else(|| PluginRequestError::load("plugin did not register expected channel listener"))?
        .clone();

    let capture = Rc::new(RefCell::new(Vec::<u8>::new()));
    let virtual_channel: IWTSVirtualChannel = CaptureVirtualChannel {
        capture: Rc::clone(&capture),
        closed: RefCell::new(false),
    }
    .into();

    let mut accept = BOOL::default();
    let mut channel_callback: Option<IWTSVirtualChannelCallback> = None;

    // SAFETY: COM call on the owning thread; out-params are stack locals valid for the call.
    unsafe {
        listener
            .OnNewChannelConnection(&virtual_channel, &BSTR::default(), &mut accept, &mut channel_callback)
            .map_err(|e| {
                warn!(error = %e, "OnNewChannelConnection failed");
                PluginRequestError::from_dynamic(format!("OnNewChannelConnection failed: {e}"), true)
            })?;
    }

    if !accept.as_bool() {
        return Err(PluginRequestError::load("plugin rejected oneshot channel"));
    }

    let callback =
        channel_callback.ok_or_else(|| PluginRequestError::load("plugin accepted channel without callback"))?;

    debug!(
        channel_name,
        request_len = request.len(),
        "Delivering oneshot request to COM plugin"
    );

    // SAFETY: buffer lives for the duration of OnDataReceived; webauthn.dll processes
    // WebAuthn ceremonies synchronously and calls Write before returning.
    unsafe {
        callback.OnDataReceived(request).map_err(|e| {
            warn!(error = %e, "OnDataReceived failed");
            PluginRequestError::from_dynamic(format!("OnDataReceived failed: {e}"), false)
        })?;
    }

    let response = capture.borrow().clone();
    if response.is_empty() {
        warn!(channel_name, "COM plugin produced empty Write payload");
        // SAFETY: channel teardown on owning thread.
        let _ = unsafe { callback.OnClose() };
        let _ = unsafe { plugin.Disconnected(0) };
        let _ = unsafe { plugin.Terminated() };
        return Err(PluginRequestError::fail(
            hresult_to_u32(E_FAIL),
            "COM plugin produced empty response",
        ));
    }

    let hresult = response
        .get(..4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
        .unwrap_or(0);
    info!(
        channel_name,
        hresult = format!("0x{hresult:08X}"),
        response_len = response.len(),
        "COM oneshot Write captured"
    );

    // SAFETY: channel teardown on owning thread.
    let _ = unsafe { callback.OnClose() };
    let _ = unsafe { plugin.Disconnected(0) };
    let _ = unsafe { plugin.Terminated() };

    Ok(response)
}

fn hresult_to_u32(hr: windows::core::HRESULT) -> u32 {
    u32::from_ne_bytes(hr.0.to_ne_bytes())
}

fn guid_from_bytes(bytes: &[u8]) -> Option<GUID> {
    if bytes.len() != 16 {
        return None;
    }
    Some(GUID {
        data1: u32::from_le_bytes(bytes[0..4].try_into().ok()?),
        data2: u16::from_le_bytes(bytes[4..6].try_into().ok()?),
        data3: u16::from_le_bytes(bytes[6..8].try_into().ok()?),
        data4: bytes[8..16].try_into().ok()?,
    })
}

/// `IWTSVirtualChannel` that captures raw `Write` bytes instead of framing DVC PDUs.
#[implement(IWTSVirtualChannel)]
struct CaptureVirtualChannel {
    capture: Rc<RefCell<Vec<u8>>>,
    closed: RefCell<bool>,
}

impl IWTSVirtualChannel_Impl for CaptureVirtualChannel_Impl {
    fn Write(&self, cbsize: u32, pbuffer: *const u8, _preserved: Ref<'_, IUnknown>) -> WinResult<()> {
        if *self.closed.borrow() {
            return Err(Error::new(E_FAIL, "channel is closed"));
        }

        let size = usize::try_from(cbsize).expect("u32 fits in usize");
        if pbuffer.is_null() && size > 0 {
            return Err(Error::new(E_INVALIDARG, "null buffer"));
        }

        // SAFETY: plugin guarantees buffer validity for the Write call.
        let data = if size > 0 {
            unsafe { core::slice::from_raw_parts(pbuffer, size) }.to_vec()
        } else {
            Vec::new()
        };

        debug!(size = data.len(), "CaptureVirtualChannel::Write");
        *self.capture.borrow_mut() = data;
        Ok(())
    }

    fn Close(&self) -> WinResult<()> {
        *self.closed.borrow_mut() = true;
        Ok(())
    }
}
