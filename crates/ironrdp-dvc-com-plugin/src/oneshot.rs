//! One-shot DVC COM plugin request helper.
//!
//! Used by the pure-Rust RDPEWA backend when a host omits `clientDataJSON` (hash-only MS-RDPEWA).
//! Public WebAuthN* APIs cannot complete those ceremonies; `webauthn.dll`'s IWTS plugin path can.

use core::cell::RefCell;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc as std_mpsc;
use std::thread;

use tracing::{debug, info, warn};
use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::RemoteDesktop::{IWTSVirtualChannel, IWTSVirtualChannel_Impl, IWTSVirtualChannelCallback};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::core::{BSTR, Error, IUnknown, Ref, Result as WinResult};
use windows_core::{BOOL, implement};

use crate::channel::initialize_plugin_on_thread;

/// Run a single request through a DVC COM plugin channel and return the plugin's `Write` payload.
///
/// This mirrors MSTSC's `webauthn.dll` path: open the named channel, deliver `request` via `OnDataReceived`, and capture the bytes the plugin writes back on `IWTSVirtualChannel::Write`.
///
/// # Errors
///
/// Returns a human-readable error when the plugin cannot be loaded, rejects the channel, or fails the request.
///
/// This waits for the COM call to complete so the plugin is torn down on its owning apartment before the caller can try another WebAuthn implementation.
pub fn process_plugin_request(dll_path: &Path, channel_name: &str, request: &[u8]) -> Result<Vec<u8>, String> {
    let dll_path = dll_path.to_path_buf();
    let channel_name = channel_name.to_owned();
    let request = request.to_vec();

    let (tx, rx) = std_mpsc::sync_channel(1);
    thread::Builder::new()
        .name("dvc-com-oneshot".into())
        .spawn(move || {
            let result = run_oneshot_on_com_thread(&dll_path, &channel_name, &request);
            let _ = tx.send(result);
        })
        .map_err(|e| format!("failed to spawn COM oneshot thread: {e}"))?;

    rx.recv()
        .map_err(|_| "COM plugin thread terminated before producing a response".to_owned())?
}

/// Convenience wrapper for System32 `webauthn.dll` / `WebAuthN_Channel`.
pub fn process_webauthn_dll_request(request: &[u8]) -> Result<Vec<u8>, String> {
    process_plugin_request(&webauthn_dll_path()?, "WebAuthN_Channel", request)
}

/// Resolve the native `webauthn.dll` from the active Windows system directory.
pub fn webauthn_dll_path() -> Result<PathBuf, String> {
    let mut buffer = vec![0u16; 260];

    loop {
        // SAFETY: buffer is writable UTF-16 storage passed directly to the Windows API.
        let len = unsafe { GetSystemDirectoryW(Some(&mut buffer)) };
        if len == 0 {
            return Err(format!("GetSystemDirectoryW failed: {}", Error::from_thread()));
        }

        let len = usize::try_from(len).map_err(|_| "system directory path is too long".to_owned())?;
        if len < buffer.len() {
            let directory = String::from_utf16(&buffer[..len])
                .map_err(|_| "GetSystemDirectoryW returned invalid UTF-16".to_owned())?;
            return Ok(PathBuf::from(directory).join("webauthn.dll"));
        }

        buffer.resize(len + 1, 0);
    }
}

fn run_oneshot_on_com_thread(dll_path: &Path, channel_name: &str, request: &[u8]) -> Result<Vec<u8>, String> {
    // SAFETY: initialize a STA apartment for the duration of this oneshot thread.
    let should_uninit = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();

    let result = run_oneshot_body(dll_path, channel_name, request);

    // SAFETY: balance a successful CoInitializeEx.
    if should_uninit {
        unsafe { CoUninitialize() };
    }
    result
}

fn run_oneshot_body(dll_path: &Path, channel_name: &str, request: &[u8]) -> Result<Vec<u8>, String> {
    let (plugin, _manager, listeners) = initialize_plugin_on_thread(dll_path)?;

    // SAFETY: COM objects live on this thread for the duration of the oneshot.
    let _ = unsafe { plugin.Connected() };

    let listener = listeners
        .get(channel_name)
        .ok_or_else(|| format!("plugin did not register listener for {channel_name}"))?
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
            .map_err(|e| format!("OnNewChannelConnection failed: {e}"))?;
    }

    if !accept.as_bool() {
        return Err("plugin rejected oneshot channel".to_owned());
    }

    let callback = channel_callback.ok_or_else(|| "plugin accepted channel without callback".to_owned())?;

    debug!(
        channel_name,
        request_len = request.len(),
        "Delivering oneshot request to COM plugin"
    );

    // SAFETY: buffer lives for the duration of OnDataReceived; webauthn.dll processes
    // WebAuthn ceremonies synchronously and calls Write before returning.
    let result = unsafe {
        callback
            .OnDataReceived(request)
            .map_err(|e| format!("OnDataReceived failed: {e}"))
    }
    .and_then(|()| {
        let response = capture.borrow().clone();
        if response.is_empty() {
            warn!(channel_name, "COM plugin produced empty Write payload");
            return Err("COM plugin produced empty response".to_owned());
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

        Ok(response)
    });

    // SAFETY: channel teardown on owning thread.
    let _ = unsafe { callback.OnClose() };
    let _ = unsafe { plugin.Disconnected(0) };
    let _ = unsafe { plugin.Terminated() };

    result
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
