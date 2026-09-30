//! The worker thread that owns the Wayland connection.
//!
//! Wayland objects live on this one thread. Callers talk to it through a
//! command channel plus a wake socket, so a request is served immediately
//! instead of at the next poll timeout.

use core::sync::atomic::{AtomicBool, Ordering};
use std::{
    io::Read as _,
    os::unix::{io::AsFd as _, net::UnixStream},
    sync::{Arc, Mutex, mpsc},
};

use nix::{
    errno::Errno,
    poll::{PollFd, PollFlags, PollTimeout, poll},
};
use wayland_client::{
    Connection, EventQueue,
    backend::WaylandError,
    globals::{GlobalList, registry_queue_init},
    protocol::wl_seat::WlSeat,
};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_manager_v1::ExtDataControlManagerV1;
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_manager_v1::ZwlrDataControlManagerV1;

use super::{
    dispatch::Client,
    error::{Error, Result},
    options::{Options, Protocol},
    state::{Command, DataControlManager, Shared, State},
};

/// A connected worker, ready to run.
pub(crate) struct Worker {
    connection: Connection,
    queue: EventQueue<Client>,
    client: Client,
    /// Kept alive for the device's lifetime.
    _seat: WlSeat,
    commands: mpsc::Receiver<Command>,
    wake: UnixStream,
    stop: Arc<AtomicBool>,
}

/// What the caller learns once the worker is connected.
pub(crate) struct Connected {
    pub(crate) protocol: Protocol,
    pub(crate) shared: Arc<Mutex<Shared>>,
}

/// Connect to the compositor, bind a data-control manager and the seat, and
/// create the device.
///
/// Runs on the worker thread, so any Wayland object created here stays there.
pub(crate) fn connect(
    options: &Options,
    commands: mpsc::Receiver<Command>,
    wake: UnixStream,
    stop: Arc<AtomicBool>,
) -> Result<(Worker, Connected)> {
    let connection = Connection::connect_to_env().map_err(|error| Error::Connect(error.to_string()))?;
    let (globals, mut queue) =
        registry_queue_init::<Client>(&connection).map_err(|error| Error::Wayland(error.to_string()))?;
    let queue_handle = queue.handle();

    let mut client = Client {
        data_control: State::default(),
    };

    let (manager, protocol) = bind_manager(&globals, &queue_handle, options)?;
    client.data_control.manager = Some(manager);

    let seat = globals
        .bind::<WlSeat, _, _>(&queue_handle, 1..=1, ())
        .map_err(|_| Error::NoSeat)?;
    client.data_control.create_device(&seat, &queue_handle);

    // The compositor sends the current selection right after the device is
    // created; wait for it so `selection_mime_types` is right from the start.
    queue
        .roundtrip(&mut client)
        .map_err(|error| Error::Wayland(error.to_string()))?;

    let shared = Arc::clone(&client.data_control.shared_state);
    tracing::info!(%protocol, "Data-control clipboard connected");

    Ok((
        Worker {
            connection,
            queue,
            client,
            _seat: seat,
            commands,
            wake,
            stop,
        },
        Connected { protocol, shared },
    ))
}

/// Bind the first protocol in the option's candidate order that the
/// compositor offers.
fn bind_manager(
    globals: &GlobalList,
    queue_handle: &wayland_client::QueueHandle<Client>,
    options: &Options,
) -> Result<(DataControlManager, Protocol)> {
    let candidates = options.candidates();
    for (index, protocol) in candidates.iter().enumerate() {
        let bound = match protocol {
            Protocol::Ext => globals
                .bind::<ExtDataControlManagerV1, _, _>(queue_handle, 1..=1, ())
                .ok()
                .map(DataControlManager::Ext),
            Protocol::Wlr => globals
                .bind::<ZwlrDataControlManagerV1, _, _>(queue_handle, 1..=2, ())
                .ok()
                .map(DataControlManager::Wlr),
        };
        if let Some(manager) = bound {
            if index > 0 {
                tracing::info!(
                    preferred = %candidates[0],
                    used = %protocol,
                    "Preferred data-control protocol unavailable, using the other"
                );
            }
            return Ok((manager, *protocol));
        }
    }

    if options.allow_fallback {
        Err(Error::Unsupported)
    } else {
        Err(Error::ProtocolUnavailable)
    }
}

impl Worker {
    /// Serve events and commands until asked to stop or the connection fails.
    pub(crate) fn run(mut self) {
        tracing::debug!("Data-control worker started");

        while !self.stop.load(Ordering::Relaxed) {
            if let Err(error) = self.queue.dispatch_pending(&mut self.client) {
                tracing::error!(%error, "Wayland dispatch failed");
                break;
            }

            self.process_commands();

            let flush_blocked = match self.connection.flush() {
                Ok(()) => false,
                Err(WaylandError::Io(ref error)) if error.kind() == std::io::ErrorKind::WouldBlock => true,
                Err(error) => {
                    tracing::error!(%error, "Wayland flush failed");
                    break;
                }
            };

            // `None` means events arrived after the dispatch above; loop to
            // handle them before waiting.
            let Some(guard) = self.queue.prepare_read() else {
                continue;
            };

            let wayland_events = if flush_blocked {
                PollFlags::POLLIN | PollFlags::POLLOUT
            } else {
                PollFlags::POLLIN
            };
            let mut fds = [
                PollFd::new(guard.connection_fd(), wayland_events),
                PollFd::new(self.wake.as_fd(), PollFlags::POLLIN),
            ];
            match poll(&mut fds, PollTimeout::NONE) {
                Ok(_) => {}
                Err(Errno::EINTR) => continue,
                Err(error) => {
                    tracing::error!(%error, "Wayland poll failed");
                    break;
                }
            }
            let wayland_readable = readable(&fds[0]);
            let woken = readable(&fds[1]);

            if wayland_readable {
                match guard.read() {
                    Ok(_) => {}
                    // Another reader took the data between poll and read.
                    Err(WaylandError::Io(ref error)) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => {
                        tracing::error!(%error, "Wayland read failed");
                        break;
                    }
                }
            } else {
                drop(guard);
            }

            if woken {
                self.drain_wake();
            }
        }

        tracing::debug!("Data-control worker stopped");
    }

    fn process_commands(&mut self) {
        let queue_handle = self.queue.handle();
        while let Ok(command) = self.commands.try_recv() {
            match command {
                Command::SetSelection { mime_types, data } => {
                    self.client.data_control.set_selection(&mime_types, data, &queue_handle);
                }
                Command::UpdateSourceData { mime_type, data } => {
                    self.client.data_control.update_source_data(mime_type, data);
                }
                Command::ReceiveFromOffer { mime_type, fd } => {
                    self.client.data_control.receive_from_offer(&mime_type, fd);
                }
                Command::CompleteTransfer { serial, data } => {
                    self.client.data_control.complete_transfer(serial, data);
                }
                Command::ClearSelection => {
                    self.client.data_control.clear_selection();
                }
            }
        }
    }

    /// Empty the wake socket so the next poll blocks again.
    fn drain_wake(&mut self) {
        let mut buffer = [0u8; 64];
        loop {
            match self.wake.read(&mut buffer) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    }
}

fn readable(fd: &PollFd<'_>) -> bool {
    fd.revents()
        .is_some_and(|events| events.intersects(PollFlags::POLLIN | PollFlags::POLLHUP | PollFlags::POLLERR))
}
