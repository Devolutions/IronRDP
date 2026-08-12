//! [`DvcComChannel`] — the `DvcProcessor` implementation that bridges IronRDP ↔ COM plugin.
//!
//! Also contains the public [`load_dvc_plugin`] / [`load_dvc_plugin_listeners`] helpers which load a
//! plugin DLL, initialize its COM objects, and return processors or recreatable listeners.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt as _;
use std::path::Path;
use std::sync::{Arc, mpsc as std_mpsc};
use std::thread;

use ironrdp_core::impl_as_any;
use ironrdp_dvc::{DvcChannelListener, DvcClientProcessor, DvcMessage, DvcProcessor, DynamicChannelId};
use ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_svc::SvcMessage;
use tracing::{debug, error, trace, warn};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::RemoteDesktop::{IWTSListenerCallback, IWTSPlugin, IWTSVirtualChannelManager};
use windows::core::{HRESULT, PCSTR, PCWSTR};
use windows_core::{GUID, Interface as _};

use crate::com::{ChannelManager, OnWriteDvc};
use crate::worker::{ComCommand, run_com_worker};

/// Type signature for the `VirtualChannelGetInstance` export in a DVC plugin DLL.
///
/// ```c
/// HRESULT VCAPITYPE VirtualChannelGetInstance(
///     REFIID  refiid,
///     ULONG  *pNumObjs,
///     VOID  **ppObjArray
/// );
/// ```
type VirtualChannelGetInstanceFn =
    unsafe extern "system" fn(refiid: *const GUID, pnumobjs: *mut u32, ppobjarray: *mut *mut c_void) -> HRESULT;

/// Shared COM worker + plugin state.
///
/// Lifetime is independent of individual DVC open/close cycles. Shutdown is sent only when the last
/// [`Arc`] clone is dropped (session teardown).
struct ComPluginShared {
    command_tx: std_mpsc::Sender<ComCommand>,
    on_write_dvc_tx: std_mpsc::Sender<OnWriteDvc>,
    on_write_dvc_factory: Arc<dyn Fn() -> OnWriteDvcMessage + Send + Sync>,
    connected_sent: AtomicBool,
    _worker_handle: Option<thread::JoinHandle<()>>,
}

impl Drop for ComPluginShared {
    fn drop(&mut self) {
        let _ = self.command_tx.send(ComCommand::Shutdown);
    }
}

/// A DVC channel backed by a native COM plugin DLL.
///
/// Each instance represents one open of a listener (channel name) registered by the plugin during
/// `IWTSPlugin::Initialize`. Multiple instances may share the same plugin worker so hosts that
/// open and close the channel around individual RPCs (e.g. `WebAuthN_Channel`) keep working.
pub struct DvcComChannel {
    channel_name: String,
    shared: Arc<ComPluginShared>,
}

impl_as_any!(DvcComChannel);

impl DvcProcessor for DvcComChannel {
    fn channel_name(&self) -> &str {
        &self.channel_name
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        debug!(
            channel_name = %self.channel_name,
            channel_id,
            "DVC COM channel start"
        );

        // Notify the plugin that the RDP connection is established (only once per plugin).
        if !self.shared.connected_sent.swap(true, Ordering::SeqCst) {
            let _ = self.shared.command_tx.send(ComCommand::Connected);
        }

        // Fresh write callback for this channel open.
        let write_cb = (self.shared.on_write_dvc_factory)();
        let _ = self.shared.on_write_dvc_tx.send(write_cb);

        let (accept_tx, accept_rx) = std_mpsc::sync_channel(1);

        self.shared
            .command_tx
            .send(ComCommand::ChannelOpened {
                channel_name: self.channel_name.clone(),
                channel_id,
                accept_tx,
            })
            .map_err(|_| pdu_other_err!("COM worker thread is gone"))?;

        let accepted = accept_rx.recv().unwrap_or(false);

        if accepted {
            debug!(
                channel_name = %self.channel_name,
                channel_id,
                "COM plugin accepted DVC channel"
            );
        } else {
            warn!(
                channel_name = %self.channel_name,
                channel_id,
                "COM plugin rejected DVC channel"
            );
        }

        Ok(vec![])
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        self.shared
            .command_tx
            .send(ComCommand::DataReceived {
                channel_id,
                data: payload.to_vec(),
            })
            .map_err(|_| pdu_other_err!("COM worker thread is gone"))?;

        Ok(vec![])
    }

    fn close(&mut self, channel_id: u32) {
        debug!(
            channel_name = %self.channel_name,
            channel_id,
            "DVC COM channel close"
        );

        let _ = self.shared.command_tx.send(ComCommand::ChannelClosed { channel_id });
    }
}

impl DvcClientProcessor for DvcComChannel {}

/// Recreatable listener for a single COM plugin channel name.
///
/// Prefer this over a one-shot [`DvcComChannel`] when the host opens and closes the DVC around each
/// RPC (as Windows does for `WebAuthN_Channel`).
pub struct DvcComChannelListener {
    channel_name: String,
    shared: Arc<ComPluginShared>,
}

impl DvcComChannelListener {
    fn new(channel_name: String, shared: Arc<ComPluginShared>) -> Self {
        Self { channel_name, shared }
    }
}

impl DvcChannelListener for DvcComChannelListener {
    fn channel_name(&self) -> &str {
        &self.channel_name
    }

    fn create(&mut self, _channel_id: DynamicChannelId) -> Option<Box<dyn DvcClientProcessor>> {
        Some(Box::new(DvcComChannel {
            channel_name: self.channel_name.clone(),
            shared: Arc::clone(&self.shared),
        }))
    }
}

/// Callback type matching the pipe proxy pattern: called when the plugin writes outbound DVC data.
pub(crate) type OnWriteDvcMessage = Box<dyn Fn(u32, Vec<SvcMessage>) -> PduResult<()> + Send + 'static>;

/// Load a DVC client plugin DLL and return one-shot channels for each listener the plugin registers.
///
/// Prefer [`load_dvc_plugin_listeners`] when the host re-creates channels (WebAuthn).
///
/// # Panics
///
/// Panics if the COM worker thread cannot be spawned.
pub fn load_dvc_plugin<F>(dll_path: &Path, on_write_dvc_factory: F) -> PduResult<Vec<DvcComChannel>>
where
    F: Fn() -> OnWriteDvcMessage + Send + Sync + 'static,
{
    let (shared, channel_names) = load_plugin_shared(dll_path, on_write_dvc_factory)?;
    Ok(channel_names
        .into_iter()
        .map(|name| DvcComChannel {
            channel_name: name,
            shared: Arc::clone(&shared),
        })
        .collect())
}

/// Load a DVC client plugin DLL and return recreatable listeners for each registered channel name.
///
/// # Panics
///
/// Panics if the COM worker thread cannot be spawned.
pub fn load_dvc_plugin_listeners<F>(dll_path: &Path, on_write_dvc_factory: F) -> PduResult<Vec<DvcComChannelListener>>
where
    F: Fn() -> OnWriteDvcMessage + Send + Sync + 'static,
{
    let (shared, channel_names) = load_plugin_shared(dll_path, on_write_dvc_factory)?;
    Ok(channel_names
        .into_iter()
        .map(|name| DvcComChannelListener::new(name, Arc::clone(&shared)))
        .collect())
}

fn load_plugin_shared<F>(dll_path: &Path, on_write_dvc_factory: F) -> PduResult<(Arc<ComPluginShared>, Vec<String>)>
where
    F: Fn() -> OnWriteDvcMessage + Send + Sync + 'static,
{
    debug!(dll = %dll_path.display(), "Loading DVC COM plugin");

    let (command_tx, command_rx) = std_mpsc::channel();
    let (on_write_dvc_tx, on_write_dvc_rx) = std_mpsc::channel();
    let (init_tx, init_rx) = std_mpsc::sync_channel::<Result<Vec<String>, String>>(1);

    let dll_path_owned = dll_path.to_path_buf();

    let worker_handle = thread::Builder::new()
        .name("dvc-com-worker".into())
        .spawn(move || match initialize_plugin_on_thread(&dll_path_owned) {
            Ok((plugin, manager, listeners)) => {
                let channel_names: Vec<String> = listeners.keys().cloned().collect();
                debug!(
                    channels = ?channel_names,
                    "Plugin initialized, registered {} listener(s)",
                    channel_names.len()
                );
                let _ = init_tx.send(Ok(channel_names));
                run_com_worker(plugin, manager, listeners, command_rx, on_write_dvc_rx);
            }
            Err(e) => {
                error!(error = %e, "Failed to initialize DVC COM plugin");
                let _ = init_tx.send(Err(e));
            }
        })
        .expect("spawn COM worker thread");

    let channel_names = init_rx
        .recv()
        .map_err(|_| pdu_other_err!("COM worker thread died during initialization"))?
        .map_err(|e| pdu_other_err!("plugin initialization failed").with_source(std::io::Error::other(e)))?;

    if channel_names.is_empty() {
        warn!(dll = %dll_path.display(), "Plugin registered no listeners");
    }

    let shared = Arc::new(ComPluginShared {
        command_tx,
        on_write_dvc_tx,
        on_write_dvc_factory: Arc::new(on_write_dvc_factory),
        connected_sent: AtomicBool::new(false),
        _worker_handle: Some(worker_handle),
    });

    Ok((shared, channel_names))
}

/// Load the plugin DLL and call VirtualChannelGetInstance + Initialize on the COM thread.
pub(crate) fn initialize_plugin_on_thread(
    dll_path: &Path,
) -> Result<
    (
        IWTSPlugin,
        IWTSVirtualChannelManager,
        HashMap<String, IWTSListenerCallback>,
    ),
    String,
> {
    let dll_path_wide: Vec<u16> = dll_path.as_os_str().encode_wide().chain(core::iter::once(0)).collect();
    let dll_path_pcwstr = PCWSTR(dll_path_wide.as_ptr());

    // SAFETY: loading the DLL into this process
    let hmodule = unsafe { LoadLibraryW(dll_path_pcwstr) }.map_err(|e| format!("LoadLibraryW failed: {e}"))?;

    trace!(dll = %dll_path.display(), "DLL loaded successfully");

    let proc_name = PCSTR::from_raw(c"VirtualChannelGetInstance".as_ptr().cast::<u8>());

    // SAFETY: hmodule is valid, proc_name is a null-terminated ASCII string
    let proc_addr = unsafe { GetProcAddress(hmodule, proc_name) }
        .ok_or_else(|| "VirtualChannelGetInstance export not found in DLL".to_owned())?;

    // SAFETY: the export matches the documented VirtualChannelGetInstance signature
    let get_instance: VirtualChannelGetInstanceFn = unsafe { core::mem::transmute(proc_addr) };

    trace!("VirtualChannelGetInstance export found");

    let iid = IWTSPlugin::IID;
    let mut num_objs: u32 = 0;

    // SAFETY: first call with null array to get count
    let hr = unsafe { get_instance(&iid, &mut num_objs, core::ptr::null_mut()) };
    if hr.is_err() {
        return Err(format!(
            "VirtualChannelGetInstance phase 1 failed: HRESULT 0x{:08X}",
            hr.0
        ));
    }

    trace!(count = num_objs, "Plugin reports {} object(s)", num_objs);

    if num_objs == 0 {
        return Err("plugin returned 0 objects".to_owned());
    }

    let mut obj_array: Vec<*mut c_void> =
        vec![core::ptr::null_mut(); usize::try_from(num_objs).expect("u32 fits in usize")];

    // SAFETY: second call with allocated array
    let hr = unsafe { get_instance(&iid, &mut num_objs, obj_array.as_mut_ptr()) };
    if hr.is_err() {
        return Err(format!(
            "VirtualChannelGetInstance phase 2 failed: HRESULT 0x{:08X}",
            hr.0
        ));
    }

    let plugin_ptr = obj_array[0];
    if plugin_ptr.is_null() {
        return Err("VirtualChannelGetInstance returned null plugin pointer".to_owned());
    }

    // SAFETY: the plugin pointer is a valid IWTSPlugin COM interface pointer
    let plugin: IWTSPlugin = unsafe { IWTSPlugin::from_raw(plugin_ptr) };

    trace!("Got IWTSPlugin COM object");

    let listeners_rc = std::rc::Rc::new(core::cell::RefCell::new(HashMap::new()));
    let channel_manager_impl = ChannelManager::new(std::rc::Rc::clone(&listeners_rc));
    let manager: IWTSVirtualChannelManager = channel_manager_impl.into();

    // SAFETY: calling IWTSPlugin::Initialize with our channel manager
    unsafe { plugin.Initialize(&manager) }.map_err(|e| format!("IWTSPlugin::Initialize failed: {e}"))?;

    trace!("IWTSPlugin::Initialize succeeded");

    let listeners: HashMap<String, IWTSListenerCallback> = listeners_rc.borrow().clone();

    Ok((plugin, manager, listeners))
}
