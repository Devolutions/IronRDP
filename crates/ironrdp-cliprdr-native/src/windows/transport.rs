use core::cell::{Cell, RefCell};
use core::time::Duration;
use std::collections::VecDeque;
use std::sync::{Arc, mpsc};
use std::time::Instant;

use ironrdp_cliprdr::backend::ClipboardMessage;
use ironrdp_cliprdr::pdu::{ClipboardFormatId, FileDescriptor, FormatDataResponse};
use tracing::{debug, warn};

use crate::windows::shared::ClipboardShared;
use crate::windows::{BackendEvent, WinCliprdrError, WinCliprdrResult};

/// How long the clipboard thread waits for the remote to answer a format data request.
const FORMAT_DATA_TIMEOUT: Duration = Duration::from_secs(10);

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

/// The clipboard thread's end of the channel to `CLIPRDR`, shared by the window subclass
/// procedure and the OLE data object it hands out.
///
/// Everything here runs on the thread owning the clipboard window: blocking waits are the
/// same pattern as `WM_RENDERFORMAT` handling, which must produce data before returning.
/// Events that arrive during a wait and are not what the wait is for are queued and handled
/// from the window procedure afterwards, rather than dropped.
#[derive(Debug)]
pub(crate) struct RemoteTransport {
    shared: Arc<ClipboardShared>,
    /// `None` once closed: data objects handed out may outlive the clipboard, and must not keep
    /// the backend's channel open after nothing drains it anymore.
    backend_rx: RefCell<Option<mpsc::Receiver<BackendEvent>>>,
    deferred: RefCell<VecDeque<BackendEvent>>,
    /// Answers still due for pastes whose wait gave up.
    late_answers: Cell<u32>,
}

impl RemoteTransport {
    pub(crate) fn new(shared: Arc<ClipboardShared>, backend_rx: mpsc::Receiver<BackendEvent>) -> Self {
        Self {
            shared,
            backend_rx: RefCell::new(Some(backend_rx)),
            deferred: RefCell::new(VecDeque::new()),
            late_answers: Cell::new(0),
        }
    }

    pub(crate) fn shared(&self) -> &Arc<ClipboardShared> {
        &self.shared
    }

    pub(crate) fn send(&self, message: ClipboardMessage) {
        self.shared.send(message);
    }

    /// Next event to handle from the window procedure: queued ones first, in arrival order.
    pub(crate) fn next_event(&self) -> Option<BackendEvent> {
        let deferred = self.deferred.borrow_mut().pop_front();
        deferred.or_else(|| self.backend_rx.borrow().as_ref()?.try_recv().ok())
    }

    /// Disconnects the backend's channel, so that the backend never waits for room in it once
    /// the clipboard is gone, and invalidates whatever the clipboard handed out.
    pub(crate) fn close(&self) {
        self.backend_rx.borrow_mut().take();
        self.deferred.borrow_mut().clear();
        self.shared.close();
    }

    /// Forgets the answers due in the previous channel.
    pub(crate) fn start_session(&self) {
        self.late_answers.set(0);
    }

    /// Fetches data in `remote_format` from the remote clipboard.
    ///
    /// Returns `None` if the remote answered with something else than format data.
    pub(crate) fn fetch_format_data(
        &self,
        remote_format: ClipboardFormatId,
    ) -> WinCliprdrResult<Option<FormatDataResponse<'static>>> {
        self.send(ClipboardMessage::SendInitiatePaste(remote_format));

        self.wait_for_answer(|answer| match answer {
            BackendEvent::FormatDataResponse(response) => Some(response),
            _ => None,
        })
    }

    /// Fetches the remote file list advertised as `remote_format`.
    ///
    /// `Cliprdr` decodes a successful answer itself and reports it through
    /// `on_remote_file_list`; failures still come back as format data.
    pub(crate) fn fetch_file_list(&self, remote_format: ClipboardFormatId) -> WinCliprdrResult<RemoteFileList> {
        let generation = self.shared.generation();
        let session = self.shared.session();

        self.send(ClipboardMessage::SendInitiatePaste(remote_format));

        self.wait_for_answer(|answer| match answer {
            // `Cliprdr` attaches the lock current when the answer arrives: after a newer remote
            // format list, that lock is the newer clipboard's, not this list's.
            BackendEvent::RemoteFileList { files, clip_data_id }
                if self.shared.generation() == generation && self.shared.session() == session =>
            {
                Ok(RemoteFileList {
                    files,
                    clip_data_id,
                    generation,
                    session,
                })
            }
            _ => Err(WinCliprdrError::ClipboardData),
        })?
    }

    /// Waits for the answer to the paste just initiated: a `FormatDataResponse`, or the
    /// `RemoteFileList` `Cliprdr` decodes a file list response into.
    ///
    /// Answers carry no request id, but come in request order: after a wait gives up, the next
    /// answer is the late one, and is dropped by any wait or by [`Self::drop_late_answer`].
    fn wait_for_answer<T>(&self, mut accept: impl FnMut(BackendEvent) -> T) -> WinCliprdrResult<T> {
        let deadline = Instant::now() + FORMAT_DATA_TIMEOUT;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());

            let received = match self.backend_rx.borrow().as_ref() {
                Some(backend_rx) => backend_rx.recv_timeout(remaining),
                None => return Err(WinCliprdrError::ClipboardData),
            };

            let event = match received {
                Ok(event) => event,
                Err(error) => {
                    warn!(%error, "No answer from the remote clipboard");
                    self.late_answers.set(self.late_answers.get().saturating_add(1));
                    return Err(WinCliprdrError::DataReceiveTimeout);
                }
            };

            match event {
                BackendEvent::FormatDataResponse(_) | BackendEvent::RemoteFileList { .. } => {
                    if self.late_answers.get() == 0 {
                        return Ok(accept(event));
                    }
                    self.late_answers.set(self.late_answers.get() - 1);
                    debug!("Dropping a late clipboard data answer");
                }
                event => self.deferred.borrow_mut().push_back(event),
            }
        }
    }

    /// Accounts for an answer that arrived while nothing was waiting for it.
    pub(crate) fn drop_late_answer(&self) {
        self.late_answers.set(self.late_answers.get().saturating_sub(1));
    }
}
