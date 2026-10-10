//! The public clipboard handle.

use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::{
    collections::HashMap,
    io::{Read as _, Write as _},
    os::{
        fd::OwnedFd,
        unix::{io::AsFd as _, net::UnixStream},
    },
    sync::{Arc, Mutex, MutexGuard, mpsc},
    thread::JoinHandle,
};

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

use super::{
    error::{Error, Result},
    mime::find_mime_match,
    options::{Options, Protocol},
    state::{Command, Shared, lock_shared},
    worker,
};

/// How long a read waits for the source to deliver more data.
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(5);

/// What to offer as the clipboard selection.
///
/// Each MIME type is either given data up front with [`data`](Self::data), or
/// only advertised with [`advertise`](Self::advertise). A paste of an
/// advertised type with no data raises a [`TransferRequest`], which the owner
/// answers later. That is delayed rendering: the data is produced only if
/// something pastes it.
#[derive(Debug, Clone, Default)]
pub struct Content {
    mime_types: Vec<String>,
    data: HashMap<String, Vec<u8>>,
}

impl Content {
    /// Empty content.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Offer `mime_type` with its data.
    #[must_use]
    pub fn data(mut self, mime_type: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        let mime_type = mime_type.into();
        self.push_type(&mime_type);
        self.data.insert(mime_type, data.into());
        self
    }

    /// Offer `mime_type` without data; pastes of it raise a [`TransferRequest`].
    #[must_use]
    pub fn advertise(mut self, mime_type: impl Into<String>) -> Self {
        self.push_type(&mime_type.into());
        self
    }

    fn push_type(&mut self, mime_type: &str) {
        if !self.mime_types.iter().any(|m| m == mime_type) {
            self.mime_types.push(mime_type.to_owned());
        }
    }
}

/// Channel to the worker thread: a command queue and a socket that wakes its
/// poll loop.
struct Link {
    commands: mpsc::Sender<Command>,
    wake: UnixStream,
}

impl Link {
    fn send(&self, command: Command) -> Result<()> {
        self.commands.send(command).map_err(|_| Error::Stopped)?;
        self.notify();
        Ok(())
    }

    fn notify(&self) {
        // A full socket buffer means a wake-up is already pending.
        let _ = (&self.wake).write(&[1]);
    }
}

/// A paste waiting for data that was advertised without any.
///
/// Answer it with [`complete`](Self::complete), or [`fail`](Self::fail) if the
/// data cannot be produced. Dropping the request without answering counts as
/// failing it, so the paste is closed at once and the next paste raises a new
/// request. A paste that waits more than five seconds for an answer is closed
/// with no data, but an answer that comes later is still kept and serves the
/// next paste of that type.
#[derive(Debug)]
#[must_use = "a paste gets no data until the request is answered"]
pub struct TransferRequest {
    serial: u32,
    mime_type: String,
    link: Arc<Link>,
    answered: bool,
}

impl core::fmt::Debug for Link {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Link").finish_non_exhaustive()
    }
}

impl TransferRequest {
    /// The MIME type being pasted.
    #[must_use]
    pub fn mime_type(&self) -> &str {
        &self.mime_type
    }

    /// Supply the data. It is written to every paste of this type that is
    /// waiting, and cached for later pastes.
    ///
    /// # Errors
    ///
    /// [`Error::Stopped`] if the clipboard worker has shut down.
    pub fn complete(mut self, data: impl Into<Vec<u8>>) -> Result<()> {
        self.answered = true;
        self.link.send(Command::CompleteTransfer {
            serial: self.serial,
            data: Some(data.into()),
        })
    }

    /// Close the waiting pastes with no data.
    ///
    /// # Errors
    ///
    /// [`Error::Stopped`] if the clipboard worker has shut down.
    pub fn fail(mut self) -> Result<()> {
        self.answered = true;
        self.link.send(Command::CompleteTransfer {
            serial: self.serial,
            data: None,
        })
    }
}

impl Drop for TransferRequest {
    fn drop(&mut self) {
        if self.answered {
            return;
        }
        // A worker that has already stopped has no paste left to close.
        let _ = self.link.send(Command::CompleteTransfer {
            serial: self.serial,
            data: None,
        });
    }
}

/// A connection to the Wayland compositor's clipboard.
///
/// Connecting spawns one thread that owns the Wayland connection. Dropping the
/// handle stops and joins it.
///
/// ```no_run
/// use ironrdp_cliprdr_native::data_control::{Content, DataControl};
///
/// # fn main() -> ironrdp_cliprdr_native::data_control::Result<()> {
/// let clipboard = DataControl::connect()?;
/// clipboard.set_selection(Content::new().data("text/plain;charset=utf-8", "hello"))?;
/// # Ok(())
/// # }
/// ```
pub struct DataControl {
    link: Arc<Link>,
    shared: Arc<Mutex<Shared>>,
    protocol: Protocol,
    max_read_bytes: usize,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl core::fmt::Debug for DataControl {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DataControl")
            .field("protocol", &self.protocol)
            .finish_non_exhaustive()
    }
}

impl DataControl {
    /// Connect with the default [`Options`].
    ///
    /// Blocks until the compositor has answered, so call it from a thread that may block.
    ///
    /// # Errors
    ///
    /// See [`Error`]: no compositor, no data-control protocol, or no seat.
    pub fn connect() -> Result<Self> {
        Self::connect_with(&Options::new())
    }

    /// Connect with explicit options.
    ///
    /// Blocks until the compositor has answered, so call it from a thread that may block.
    ///
    /// # Errors
    ///
    /// See [`Error`]: no compositor, no data-control protocol, or no seat.
    pub fn connect_with(options: &Options) -> Result<Self> {
        let (command_tx, command_rx) = mpsc::channel();
        let (wake_tx, wake_rx) = UnixStream::pair()?;
        wake_tx.set_nonblocking(true)?;
        wake_rx.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);

        let thread_options = options.clone();
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("ironrdp-cliprdr-data-control".into())
            .spawn(
                move || match worker::connect(&thread_options, command_rx, wake_rx, thread_stop) {
                    Ok((worker, connected)) => {
                        if ready_tx.send(Ok(connected)).is_ok() {
                            worker.run();
                        }
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                    }
                },
            )?;

        let connected = match ready_rx.recv() {
            Ok(Ok(connected)) => connected,
            Ok(Err(error)) => {
                let _ = thread.join();
                return Err(error);
            }
            Err(_) => {
                let _ = thread.join();
                return Err(Error::Stopped);
            }
        };

        Ok(Self {
            link: Arc::new(Link {
                commands: command_tx,
                wake: wake_tx,
            }),
            shared: connected.shared,
            protocol: connected.protocol,
            max_read_bytes: options.max_read_bytes,
            stop,
            thread: Some(thread),
        })
    }

    /// Wait until the compositor has processed the requests sent so far.
    ///
    /// Afterwards [`owns_selection`](Self::owns_selection) reflects the outcome of an earlier
    /// [`set_selection`](Self::set_selection), including a copy by another client that won.
    pub(crate) fn synchronize(&self) -> Result<()> {
        let (sender, receiver) = mpsc::channel();
        self.link.send(Command::Synchronize(sender))?;
        receiver.recv_timeout(READ_IDLE_TIMEOUT).map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => Error::Timeout,
            mpsc::RecvTimeoutError::Disconnected => Error::Stopped,
        })
    }

    /// The protocol the compositor is speaking.
    #[must_use]
    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    fn shared(&self) -> MutexGuard<'_, Shared> {
        lock_shared(&self.shared)
    }

    /// MIME types the current selection offers; empty if there is none.
    #[must_use]
    pub fn selection_mime_types(&self) -> Vec<String> {
        self.shared().mime_types.clone()
    }

    /// A counter that increases on every selection change.
    #[must_use]
    pub fn serial(&self) -> u32 {
        self.shared().serial
    }

    /// Whether this handle owns the clipboard selection.
    ///
    /// It turns `true` when the worker processes [`set_selection`](Self::set_selection). It turns
    /// `false` when another client takes the selection, when the selection is cleared, and when
    /// this handle gives it up. Like [`serial`](Self::serial), it reports what the worker has seen
    /// so far, so it can trail a request that is still queued.
    #[must_use]
    pub fn owns_selection(&self) -> bool {
        self.shared().own_source_live
    }

    /// Call `callback` with the offered MIME types whenever another client
    /// changes the selection.
    ///
    /// A selection this handle set itself is not reported; it still advances
    /// [`serial`](Self::serial) and updates
    /// [`selection_mime_types`](Self::selection_mime_types). Clearing the
    /// selection is reported as an empty list, even when this handle cleared
    /// it with [`clear_selection`](Self::clear_selection).
    ///
    /// The callback runs on the worker thread and must not block.
    pub fn on_change(&self, callback: impl Fn(Vec<String>) + Send + Sync + 'static) {
        self.shared().on_change = Some(Arc::new(callback));
    }

    /// Call `callback` when something pastes a type that was advertised
    /// without data.
    ///
    /// The callback runs on the worker thread and must not block; hand the
    /// request to another thread and answer it there.
    pub fn on_transfer(&self, callback: impl Fn(TransferRequest) + Send + Sync + 'static) {
        let link = Arc::clone(&self.link);
        self.shared().on_transfer = Some(Arc::new(move |serial, mime_type| {
            callback(TransferRequest {
                serial,
                mime_type,
                link: Arc::clone(&link),
                answered: false,
            });
        }));
    }

    /// Become the clipboard owner and offer `content`.
    ///
    /// The selection advertises one private type next to the types of `content`, which marks it as
    /// this client's own. Other clients see that type among the types they are offered, and a request
    /// for it gets no data. This client leaves it out of the types it reports to the callbacks and
    /// lists in [`selection_mime_types`](Self::selection_mime_types).
    ///
    /// # Errors
    ///
    /// [`Error::Stopped`] if the worker has shut down.
    pub fn set_selection(&self, content: Content) -> Result<()> {
        self.link.send(Command::SetSelection {
            mime_types: content.mime_types,
            data: content.data,
        })
    }

    /// Add data for a type after [`set_selection`](Self::set_selection),
    /// without replacing the selection.
    ///
    /// # Errors
    ///
    /// [`Error::Stopped`] if the worker has shut down.
    pub fn update_data(&self, mime_type: impl Into<String>, data: impl Into<Vec<u8>>) -> Result<()> {
        self.link.send(Command::UpdateSourceData {
            mime_type: mime_type.into(),
            data: data.into(),
        })
    }

    /// Give up the selection if this handle still owns it.
    ///
    /// # Errors
    ///
    /// [`Error::Stopped`] if the worker has shut down.
    pub fn clear_selection(&self) -> Result<()> {
        self.link.send(Command::ClearSelection)
    }

    /// Read the current selection as `mime_type`.
    ///
    /// Charset differences are tolerated, see [`find_mime_match`]. Returns
    /// `Ok(None)` when there is no selection or it does not offer the type.
    ///
    /// The bytes are in the charset of the type the source offered, which can differ from
    /// `mime_type`. Match `mime_type` against [`selection_mime_types`](Self::selection_mime_types)
    /// with [`find_mime_match`] to learn which type will be read.
    ///
    /// Blocks until the source has delivered everything, so call it from a thread that may block.
    /// The timeout measures silence, not the total time, so a source that keeps delivering data is
    /// only stopped by the size limit.
    ///
    /// # Errors
    ///
    /// [`Error::SelectionChanged`] if the selection changed before the data
    /// arrived, [`Error::TooLarge`] past the size limit, [`Error::Timeout`] if
    /// the source stalls for five seconds, [`Error::Stopped`] if the worker has
    /// shut down, [`Error::Io`] for pipe failures.
    pub fn read(&self, mime_type: &str) -> Result<Option<Vec<u8>>> {
        let (available, serial) = {
            let shared = self.shared();
            (shared.mime_types.clone(), shared.serial)
        };
        let Some(matched) = find_mime_match(mime_type, &available) else {
            return Ok(None);
        };

        let (mut reader, writer) = std::io::pipe()?;
        self.link.send(Command::ReceiveFromOffer {
            mime_type: matched.to_owned(),
            fd: OwnedFd::from(writer),
        })?;

        let mut data = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let mut fds = [PollFd::new(reader.as_fd(), PollFlags::POLLIN)];
            let timeout = PollTimeout::try_from(READ_IDLE_TIMEOUT).unwrap_or(PollTimeout::NONE);
            match poll(&mut fds, timeout) {
                Ok(0) => return Err(Error::Timeout),
                Ok(_) => {}
                Err(nix::errno::Errno::EINTR) => continue,
                Err(errno) => return Err(std::io::Error::from(errno).into()),
            }
            match reader.read(&mut chunk) {
                // The writer closed. The source has sent everything, unless the selection changed
                // under the request, which leaves the pipe empty or holding another selection's data.
                Ok(0) if self.serial() != serial => return Err(Error::SelectionChanged),
                // Nothing arrived and the worker is gone, so the request never left the process.
                // Data that did arrive is complete either way, because the source writes it directly.
                Ok(0) if data.is_empty() && self.shared().stopped => return Err(Error::Stopped),
                Ok(0) => return Ok(Some(data)),
                Ok(n) => {
                    if self.max_read_bytes < data.len() + n {
                        return Err(Error::TooLarge {
                            size: data.len() + n,
                            limit: self.max_read_bytes,
                        });
                    }
                    data.extend_from_slice(&chunk[..n]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// Test-only: a handle with no worker thread. The caller receives the commands the handle
    /// sends and plays the worker.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the wake-up socket cannot be created.
    #[cfg(feature = "__test")]
    #[doc(hidden)]
    pub fn without_worker(shared: Arc<Mutex<Shared>>) -> Result<(Self, mpsc::Receiver<Command>)> {
        let (commands, receiver) = mpsc::channel();
        // The other end is dropped, so a wake-up write fails, and `Link::notify` ignores that.
        let (wake, _peer) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        let handle = Self {
            link: Arc::new(Link { commands, wake }),
            shared,
            protocol: Protocol::Ext,
            max_read_bytes: Options::new().max_read_bytes,
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        Ok((handle, receiver))
    }
}

impl Drop for DataControl {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.link.notify();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
