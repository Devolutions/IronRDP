//! [`DvcComChannel`] — the `DvcProcessor` implementation that bridges IronRDP ↔ COM plugin.
//!
//! Also contains the public [`load_dvc_plugin`] function which loads a plugin DLL,
//! initializes its COM objects, and returns a set of `DvcComChannel`s to register
//! with IronRDP's `DrdynvcClient`.

use core::cell::Cell;
use core::ffi::c_void;
use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt as _;
use std::path::Path;
use std::sync::{mpsc as std_mpsc, Arc};
use std::thread;

use ironrdp_core::impl_as_any;
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::{pdu_other_err, PduResult};
use ironrdp_svc::SvcMessage;
use tracing::{debug, error, info, warn};
use windows::core::{HRESULT, PCSTR, PCWSTR};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::RemoteDesktop::{IWTSListenerCallback, IWTSPlugin, IWTSVirtualChannelManager};
use windows_core::{Interface as _, GUID};

use crate::com::{ChannelManager, OnWriteDvc};
use crate::worker::{run_com_worker, ComCommand};

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

/// A DVC channel backed by a native COM plugin DLL.
///
/// Each instance represents one listener (channel name) registered by the plugin
/// during `IWTSPlugin::Initialize`. It implements [`DvcProcessor`] + [`DvcClientProcessor`]
/// so it can be registered with IronRDP's `DrdynvcClient`.
///
/// Communication with the COM worker thread happens via `std::sync::mpsc` channels.
pub struct DvcComChannel {
    channel_name: String,
    command_tx: std_mpsc::Sender<ComCommand>,
    on_write_dvc_tx: std_mpsc::Sender<OnWriteDvc>,
    on_write_dvc_factory: Arc<dyn Fn() -> OnWriteDvcMessage + Send + Sync>,
    /// Set to false after the first `start()` call sends `Connected`
    needs_connected: bool,
    _worker_handle: Option<thread::JoinHandle<()>>,
}

impl_as_any!(DvcComChannel);

impl DvcProcessor for DvcComChannel {
    fn channel_name(&self) -> &str {
        &self.channel_name
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        info!(
            channel_name = %self.channel_name,
            channel_id,
            "DVC COM channel start"
        );

        // Notify the plugin that the RDP connection is established (only once per plugin)
        if self.needs_connected {
            self.needs_connected = false;
            let _ = self.command_tx.send(ComCommand::Connected);
        }

        // Create a fresh write callback for this channel opening
        let write_cb = (self.on_write_dvc_factory)();
        let _ = self.on_write_dvc_tx.send(write_cb);

        let (accept_tx, accept_rx) = std_mpsc::sync_channel(1);

        self.command_tx
            .send(ComCommand::ChannelOpened {
                channel_name: self.channel_name.clone(),
                channel_id,
                accept_tx,
            })
            .map_err(|_| pdu_other_err!("COM worker thread is gone"))?;

        // Block until the COM thread processes the channel open
        let accepted = accept_rx.recv().unwrap_or(false);

        if accepted {
            info!(
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
        self.command_tx
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

        let _ = self.command_tx.send(ComCommand::ChannelClosed { channel_id });
    }
}

impl DvcClientProcessor for DvcComChannel {}

impl Drop for DvcComChannel {
    fn drop(&mut self) {
        // Send shutdown to the COM worker thread
        let _ = self.command_tx.send(ComCommand::Shutdown);
        // Don't join — the worker will clean up asynchronously
    }
}

/// Callback type matching the pipe proxy pattern: called when the plugin writes outbound DVC data.
pub(crate) type OnWriteDvcMessage = Box<dyn Fn(u32, Vec<SvcMessage>) -> PduResult<()> + Send + 'static>;

/// Load a DVC client plugin DLL and return channels for each listener the plugin registers.
///
/// # Arguments
///
/// * `dll_path` — Path to the DVC plugin DLL (e.g. `C:\Windows\System32\webauthn.dll`)
/// * `on_write_dvc` — Factory function that creates a write callback for each channel.
///   The callback is invoked when the plugin calls `IWTSVirtualChannel::Write()`,
///   sending the encoded DVC messages back into IronRDP's session event loop.
///
/// # Returns
///
/// A `Vec<DvcComChannel>`, one per listener the plugin registered during `Initialize`.
/// These should be added to a `DrdynvcClient` via `with_dynamic_channel()`.
///
/// # Panics
///
/// Panics if the COM worker thread cannot be spawned.
pub fn load_dvc_plugin<F>(dll_path: &Path, on_write_dvc_factory: F) -> PduResult<Vec<DvcComChannel>>
where
    F: Fn() -> OnWriteDvcMessage + Send + Sync + 'static,
{
    info!(dll = %dll_path.display(), "Loading DVC COM plugin");

    // Channel for sending commands to the COM worker thread
    let (command_tx, command_rx) = std_mpsc::channel();

    // Channel for sending write callbacks to the COM worker thread
    let (on_write_dvc_tx, on_write_dvc_rx) = std_mpsc::channel();

    // Channel for receiving the list of registered listeners back from the COM thread
    let (init_tx, init_rx) = std_mpsc::sync_channel::<Result<Vec<String>, String>>(1);

    let dll_path_owned = dll_path.to_path_buf();
    let _on_write_dvc_tx_clone = on_write_dvc_tx.clone();

    let _worker_handle = thread::Builder::new()
        .name("dvc-com-worker".into())
        .spawn(move || {
            // Load and initialize on the COM thread
            match initialize_plugin_on_thread(&dll_path_owned) {
                Ok((plugin, manager, listeners)) => {
                    let channel_names: Vec<String> = listeners.keys().cloned().collect();
                    info!(
                        channels = ?channel_names,
                        "Plugin initialized, registered {} listener(s)",
                        channel_names.len()
                    );
                    let _ = init_tx.send(Ok(channel_names));

                    // Enter the command loop
                    run_com_worker(plugin, manager, listeners, command_rx, on_write_dvc_rx);
                }
                Err(e) => {
                    error!(error = %e, "Failed to initialize DVC COM plugin");
                    let _ = init_tx.send(Err(e));
                }
            }
        })
        .expect("spawn COM worker thread");

    // Wait for initialization to complete
    let channel_names = init_rx
        .recv()
        .map_err(|_| pdu_other_err!("COM worker thread died during initialization"))?
        .map_err(|e| pdu_other_err!("plugin initialization failed").with_source(std::io::Error::other(e)))?;

    if channel_names.is_empty() {
        warn!(dll = %dll_path.display(), "Plugin registered no listeners");
    }

    // Create a DvcComChannel for each registered listener
    let mut channels = Vec::with_capacity(channel_names.len());
    let is_first = Cell::new(true);
    let factory: Arc<dyn Fn() -> OnWriteDvcMessage + Send + Sync> = Arc::new(on_write_dvc_factory);

    for name in channel_names {
        debug!(channel_name = %name, "Creating DvcComChannel");

        channels.push(DvcComChannel {
            channel_name: name,
            command_tx: command_tx.clone(),
            on_write_dvc_tx: on_write_dvc_tx.clone(),
            on_write_dvc_factory: Arc::clone(&factory),
            needs_connected: is_first.get(),
            _worker_handle: None,
        });
        is_first.set(false);
    }

    Ok(channels)
}

/// Load the plugin DLL and call VirtualChannelGetInstance + Initialize on the COM thread.
///
/// Returns the plugin COM object, the channel manager interface, and
/// the map of listener names → callbacks.
fn initialize_plugin_on_thread(
    dll_path: &Path,
) -> Result<
    (
        IWTSPlugin,
        IWTSVirtualChannelManager,
        HashMap<String, IWTSListenerCallback>,
    ),
    String,
> {
    // Load the DLL
    let dll_path_wide: Vec<u16> = dll_path.as_os_str().encode_wide().chain(core::iter::once(0)).collect();
    let dll_path_pcwstr = PCWSTR(dll_path_wide.as_ptr());

    // SAFETY: loading the DLL into this process
    let hmodule = unsafe { LoadLibraryW(dll_path_pcwstr) }.map_err(|e| format!("LoadLibraryW failed: {e}"))?;

    info!(dll = %dll_path.display(), "DLL loaded successfully");

    // Get the VirtualChannelGetInstance export
    let proc_name = PCSTR::from_raw(c"VirtualChannelGetInstance".as_ptr().cast::<u8>());

    // SAFETY: hmodule is valid, proc_name is a null-terminated ASCII string
    let proc_addr = unsafe { GetProcAddress(hmodule, proc_name) }
        .ok_or_else(|| "VirtualChannelGetInstance export not found in DLL".to_owned())?;

    // SAFETY: transmuting the function pointer; we trust the DLL follows the documented API
    let get_instance: VirtualChannelGetInstanceFn = unsafe { core::mem::transmute(proc_addr) };

    info!("VirtualChannelGetInstance export found");

    // Phase 1: query the number of plugin objects
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

    info!(count = num_objs, "Plugin reports {} object(s)", num_objs);

    if num_objs == 0 {
        return Err("plugin returned 0 objects".to_owned());
    }

    // Phase 2: get the actual plugin objects
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

    // Use the first plugin object
    let plugin_ptr = obj_array[0];
    if plugin_ptr.is_null() {
        return Err("VirtualChannelGetInstance returned null plugin pointer".to_owned());
    }

    // SAFETY: the plugin pointer is a valid IWTSPlugin COM interface pointer
    let plugin: IWTSPlugin = unsafe { IWTSPlugin::from_raw(plugin_ptr) };

    info!("Got IWTSPlugin COM object");

    // Create shared state for listeners: we keep an Rc clone so we can read the
    // map after Initialize() without needing an unsafe cast from the COM pointer.
    let listeners_rc = std::rc::Rc::new(core::cell::RefCell::new(HashMap::new()));
    let channel_manager_impl = ChannelManager::new(std::rc::Rc::clone(&listeners_rc));
    let manager: IWTSVirtualChannelManager = channel_manager_impl.into();

    // SAFETY: calling IWTSPlugin::Initialize with our channel manager
    unsafe { plugin.Initialize(&manager) }.map_err(|e| format!("IWTSPlugin::Initialize failed: {e}"))?;

    info!("IWTSPlugin::Initialize succeeded");

    // Read the listener map that the plugin populated during Initialize.
    let listeners: HashMap<String, IWTSListenerCallback> = listeners_rc.borrow().clone();

    Ok((plugin, manager, listeners))
}
