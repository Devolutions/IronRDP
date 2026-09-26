//! `CLIPRDR` backend for Linux desktops, over the X11 and Wayland clipboards.
//!
//! Built on [`arboard`]. On Wayland it uses the data-control protocol when the
//! compositor offers it (Sway, KDE Plasma, GNOME 48 and later) and otherwise
//! the X11 clipboard through XWayland, which GNOME and KDE keep in sync with
//! the Wayland selection. Plain text (`CF_UNICODETEXT`) and images
//! (`CF_DIB`/`CF_DIBV5`) travel in both directions; files and HTML do not.
//!
//! Remote content is fetched as soon as the remote announces a copy and written
//! to the OS clipboard, because neither clipboard protocol used here lets a
//! client offer content it will only produce on demand. Local changes are
//! found by polling; see [`worker::POLL_INTERVAL`].

mod image;
mod os;
mod worker;

use core::fmt;
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

use ironrdp_cliprdr::backend::{ClipboardMessageProxy, CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse, FormatDataRequest,
    FormatDataResponse, LockDataId,
};
use ironrdp_core::impl_as_any;
use tracing::{debug, warn};

use self::worker::{Command, Worker};

#[derive(Debug)]
pub enum LinuxCliprdrError {
    /// No usable clipboard: neither a Wayland data-control clipboard nor an X11
    /// display could be opened.
    Clipboard(arboard::Error),
    Spawn(std::io::Error),
    /// The clipboard thread stopped before reporting whether it could start.
    WorkerStopped,
}

impl fmt::Display for LinuxCliprdrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Clipboard(error) => write!(f, "cannot open the OS clipboard: {error}"),
            Self::Spawn(error) => write!(f, "cannot start the clipboard thread: {error}"),
            Self::WorkerStopped => f.write_str("the clipboard thread stopped unexpectedly"),
        }
    }
}

impl core::error::Error for LinuxCliprdrError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Clipboard(error) => Some(error),
            Self::Spawn(error) => Some(error),
            Self::WorkerStopped => None,
        }
    }
}

/// The Linux clipboard bridge. Owns the thread that talks to the OS clipboard;
/// keep it alive for as long as the connection it serves, and build one
/// [`CliprdrBackend`] per channel initialization through
/// [`backend_factory`](Self::backend_factory).
pub struct LinuxClipboard {
    commands: Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl fmt::Debug for LinuxClipboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinuxClipboard").finish_non_exhaustive()
    }
}

impl LinuxClipboard {
    /// Opens the OS clipboard and starts the clipboard thread.
    ///
    /// `message_proxy` delivers the resulting `CLIPRDR` messages to the RDP
    /// session; it is called from the clipboard thread.
    pub fn new(message_proxy: impl ClipboardMessageProxy + 'static) -> Result<Self, LinuxCliprdrError> {
        let (commands, receiver) = mpsc::channel();
        let (ready_sender, ready) = mpsc::channel();

        let thread = std::thread::Builder::new()
            .name("cliprdr-linux".to_owned())
            .spawn(move || {
                let os = match os::ArboardClipboard::open() {
                    Ok(os) => os,
                    Err(error) => {
                        let _ = ready_sender.send(Err(error));
                        return;
                    }
                };
                let _ = ready_sender.send(Ok(()));
                Worker::new(os, message_proxy).run(&receiver);
                debug!("Clipboard thread stopped");
            })
            .map_err(LinuxCliprdrError::Spawn)?;

        match ready.recv() {
            Ok(Ok(())) => Ok(Self {
                commands,
                thread: Some(thread),
            }),
            Ok(Err(error)) => Err(LinuxCliprdrError::Clipboard(error)),
            Err(_) => Err(LinuxCliprdrError::WorkerStopped),
        }
    }

    pub fn backend_factory(&self) -> Box<dyn CliprdrBackendFactory + Send> {
        Box::new(LinuxCliprdrBackendFactory {
            commands: self.commands.clone(),
        })
    }
}

impl Drop for LinuxClipboard {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct LinuxCliprdrBackendFactory {
    commands: Sender<Command>,
}

impl CliprdrBackendFactory for LinuxCliprdrBackendFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(LinuxCliprdrBackend {
            commands: self.commands.clone(),
        })
    }
}

/// Forwards the `CLIPRDR` callbacks to the clipboard thread.
#[derive(Debug)]
pub struct LinuxCliprdrBackend {
    commands: Sender<Command>,
}

impl_as_any!(LinuxCliprdrBackend);

impl LinuxCliprdrBackend {
    fn send(&self, command: Command) {
        if self.commands.send(command).is_err() {
            warn!("The clipboard thread is gone; dropping a CLIPRDR event");
        }
    }
}

impl CliprdrBackend for LinuxCliprdrBackend {
    fn temporary_directory(&self) -> &str {
        ".cliprdr"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        // No file transfer: no STREAM_FILECLIP_ENABLED, no CAN_LOCK_CLIPDATA.
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_ready(&mut self) {}

    fn on_request_format_list(&mut self) {
        self.send(Command::AdvertiseLocal);
    }

    fn on_process_negotiated_capabilities(&mut self, capabilities: ClipboardGeneralCapabilityFlags) {
        debug!(?capabilities, "CLIPRDR capabilities negotiated");
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        self.send(Command::RemoteCopy(available_formats.to_vec()));
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.send(Command::RenderLocal(request.format));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        let data = (!response.is_error()).then(|| response.data().to_vec());
        self.send(Command::RemoteData(data));
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        debug!(?request, "File transfer is not supported");
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        debug!(?response, "File transfer is not supported");
    }

    fn on_lock(&mut self, data_id: LockDataId) {
        debug!(?data_id);
    }

    fn on_unlock(&mut self, data_id: LockDataId) {
        debug!(?data_id);
    }

    fn now_ms(&self) -> u64 {
        crate::native_now_ms()
    }
}
