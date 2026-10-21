use core::cell::{Cell, RefCell};
use core::time::Duration;
use std::collections::{HashSet, VecDeque};
use std::sync::mpsc;
use std::time::Instant;

use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy};
use ironrdp_cliprdr::pdu::{
    ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse, FileDescriptor,
    FormatDataResponse,
};
use tracing::{debug, warn};

use crate::windows::{BackendEvent, WinCliprdrError, WinCliprdrResult};

/// How long the clipboard thread waits for the remote to answer a format data request.
const FORMAT_DATA_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the clipboard thread waits for the remote to answer a file contents request.
///
/// Longer than [`FORMAT_DATA_TIMEOUT`]: a chunk can be up to
/// [`crate::windows::file_list::MAX_FILE_CONTENTS_CHUNK`] bytes.
const FILE_CONTENTS_TIMEOUT: Duration = Duration::from_secs(30);

/// Remote file list received for a paste, with the lock that keeps it valid.
#[derive(Debug)]
pub(crate) struct RemoteFileList {
    pub(crate) files: Vec<FileDescriptor>,
    /// `clipDataId` of the lock `Cliprdr` placed on the remote clipboard when it advertised
    /// these files; `None` when locking was not negotiated.
    pub(crate) clip_data_id: Option<u32>,
    pub(crate) generation: u64,
    pub(crate) session: u64,
}

/// Channel between the clipboard (GUI) thread and the `CLIPRDR` channel, shared by the window
/// subclass procedure and the OLE objects it hands out.
///
/// Everything here runs on the thread owning the clipboard window: blocking waits are the
/// same pattern as `WM_RENDERFORMAT` handling, which must produce data before returning.
/// Events that arrive during a wait and are not what the wait is for are queued and handled
/// from the window procedure afterwards, rather than dropped.
#[derive(Debug)]
pub(crate) struct RemoteTransport {
    message_proxy: Box<dyn ClipboardMessageProxy>,
    backend_rx: mpsc::Receiver<BackendEvent>,
    deferred: RefCell<VecDeque<BackendEvent>>,
    capabilities: Cell<ClipboardGeneralCapabilityFlags>,
    /// Bumped on every remote format list: data fetched by format id is only valid for the
    /// remote clipboard it was advertised with.
    generation: Cell<u64>,
    /// Bumped on every new `CLIPRDR` channel (connection or reconnection).
    session: Cell<u64>,
    next_stream_id: Cell<u32>,
    /// Outgoing locks `Cliprdr` has released in the current session.
    released_locks: RefCell<HashSet<u32>>,
}

impl RemoteTransport {
    pub(crate) fn new(message_proxy: Box<dyn ClipboardMessageProxy>, backend_rx: mpsc::Receiver<BackendEvent>) -> Self {
        Self {
            message_proxy,
            backend_rx,
            deferred: RefCell::new(VecDeque::new()),
            capabilities: Cell::new(ClipboardGeneralCapabilityFlags::empty()),
            generation: Cell::new(0),
            session: Cell::new(0),
            next_stream_id: Cell::new(1),
            released_locks: RefCell::new(HashSet::new()),
        }
    }

    pub(crate) fn send(&self, message: ClipboardMessage) {
        self.message_proxy.send_clipboard_message(message);
    }

    /// Next event to handle from the window procedure: queued ones first, in arrival order.
    pub(crate) fn next_event(&self) -> Option<BackendEvent> {
        let deferred = self.deferred.borrow_mut().pop_front();
        deferred.or_else(|| self.backend_rx.try_recv().ok())
    }

    pub(crate) fn capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        self.capabilities.get()
    }

    pub(crate) fn set_capabilities(&self, capabilities: ClipboardGeneralCapabilityFlags) {
        self.capabilities.set(capabilities);
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation.get()
    }

    pub(crate) fn next_generation(&self) -> u64 {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        generation
    }

    pub(crate) fn session(&self) -> u64 {
        self.session.get()
    }

    pub(crate) fn start_session(&self) {
        self.session.set(self.session.get().wrapping_add(1));
        self.next_generation();
        self.capabilities.set(ClipboardGeneralCapabilityFlags::empty());
        self.released_locks.borrow_mut().clear();
    }

    pub(crate) fn release_locks(&self, clip_data_ids: impl IntoIterator<Item = u32>) {
        self.released_locks.borrow_mut().extend(clip_data_ids);
    }

    pub(crate) fn is_lock_released(&self, clip_data_id: u32) -> bool {
        self.released_locks.borrow().contains(&clip_data_id)
    }

    fn next_stream_id(&self) -> u32 {
        let stream_id = self.next_stream_id.get();
        self.next_stream_id.set(stream_id.wrapping_add(1));
        stream_id
    }

    /// Fetches data in `remote_format` from the remote clipboard.
    ///
    /// Returns `None` if the remote answered with something else than format data.
    pub(crate) fn fetch_format_data(
        &self,
        remote_format: ClipboardFormatId,
    ) -> WinCliprdrResult<Option<FormatDataResponse<'static>>> {
        self.send(ClipboardMessage::SendInitiatePaste(remote_format));

        self.wait_for(FORMAT_DATA_TIMEOUT, |event| match event {
            BackendEvent::FormatDataResponse(response) => Wait::Done(Some(response)),
            event => Wait::Defer(event),
        })
    }

    /// Fetches the remote file list advertised as `remote_format`.
    ///
    /// `Cliprdr` decodes a successful answer itself and reports it through
    /// `on_remote_file_list`; failures still come back as format data.
    pub(crate) fn fetch_file_list(&self, remote_format: ClipboardFormatId) -> WinCliprdrResult<RemoteFileList> {
        let generation = self.generation();
        let session = self.session();

        self.send(ClipboardMessage::SendInitiatePaste(remote_format));

        self.wait_for(FORMAT_DATA_TIMEOUT, |event| match event {
            BackendEvent::RemoteFileList { files, clip_data_id } => Wait::Done(Ok(RemoteFileList {
                files,
                clip_data_id,
                generation,
                session,
            })),
            BackendEvent::FormatDataResponse(_) => Wait::Done(Err(WinCliprdrError::ClipboardData)),
            event => Wait::Defer(event),
        })?
    }

    /// Sends `request` and waits for the matching `FileContentsResponse`.
    ///
    /// The stream id is assigned here. `Cliprdr` answers every request it sent or rejected,
    /// synthesizing a failure on its own transfer timeout, so a response that shows up
    /// after this wait gave up is recognized by its stream id and dropped.
    pub(crate) fn fetch_file_contents(
        &self,
        mut request: FileContentsRequest,
    ) -> WinCliprdrResult<FileContentsResponse<'static>> {
        let stream_id = self.next_stream_id();
        request.stream_id = stream_id;

        self.send(ClipboardMessage::SendFileContentsRequest(request));

        let response = self.wait_for(FILE_CONTENTS_TIMEOUT, |event| match event {
            BackendEvent::FileContentsResponse(response) if response.stream_id() == stream_id => Wait::Done(response),
            BackendEvent::FileContentsResponse(response) => {
                debug!(
                    stream_id = response.stream_id(),
                    "Dropping stale file contents response"
                );
                Wait::Drop
            }
            event => Wait::Defer(event),
        })?;

        if response.is_error() {
            return Err(WinCliprdrError::ClipboardData);
        }

        Ok(response)
    }

    fn wait_for<T>(&self, timeout: Duration, mut accept: impl FnMut(BackendEvent) -> Wait<T>) -> WinCliprdrResult<T> {
        let deadline = Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());

            let event = self.backend_rx.recv_timeout(remaining).map_err(|error| {
                warn!(%error, "No answer from the remote clipboard");
                WinCliprdrError::DataReceiveTimeout
            })?;

            // Applied right away rather than when the event is handled: a stream reading in a
            // loop must not send another request under a lock `Cliprdr` already released.
            if let BackendEvent::OutgoingLocksCleared(data_ids) = &event {
                self.release_locks(data_ids.iter().map(|data_id| data_id.0));
            }

            match accept(event) {
                Wait::Done(value) => return Ok(value),
                Wait::Defer(event) => self.deferred.borrow_mut().push_back(event),
                Wait::Drop => {}
            }
        }
    }
}

enum Wait<T> {
    Done(T),
    Defer(BackendEvent),
    Drop,
}
