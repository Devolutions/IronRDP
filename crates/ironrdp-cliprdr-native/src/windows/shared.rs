use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, mpsc};

use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy};
use ironrdp_cliprdr::pdu::{ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse};
use tracing::debug;

/// Clipboard state shared by the clipboard thread, the `CLIPRDR` backend (on the RDP thread) and
/// the streams serving remote files (on COM worker threads).
///
/// The backend updates it as soon as the channel reports a change, so that a stream reading a
/// file in chunks stops at once when the data it reads is no longer valid, and hands file
/// contents responses directly to the stream waiting for them: reading remote files never
/// involves the clipboard thread.
#[derive(Debug)]
pub(crate) struct ClipboardShared {
    message_proxy: Mutex<Box<dyn ClipboardMessageProxy>>,
    capabilities: AtomicU32,
    /// Bumped on every remote format list: data fetched by format id is only valid for the
    /// remote clipboard it was advertised with.
    generation: AtomicU64,
    /// Bumped on every new `CLIPRDR` channel (connection or reconnection).
    session: AtomicU64,
    /// Outgoing locks `Cliprdr` has released in the current session.
    released_locks: Mutex<HashSet<u32>>,
    next_stream_id: AtomicU32,
    /// Streams waiting for a file contents response, by stream id.
    waiters: Mutex<HashMap<u32, mpsc::Sender<FileContentsResponse<'static>>>>,
}

impl ClipboardShared {
    pub(crate) fn new(message_proxy: Box<dyn ClipboardMessageProxy>) -> Self {
        Self {
            message_proxy: Mutex::new(message_proxy),
            capabilities: AtomicU32::new(0),
            generation: AtomicU64::new(0),
            session: AtomicU64::new(0),
            released_locks: Mutex::new(HashSet::new()),
            next_stream_id: AtomicU32::new(1),
            waiters: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn send(&self, message: ClipboardMessage) {
        self.message_proxy
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .send_clipboard_message(message);
    }

    pub(crate) fn capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::from_bits_retain(self.capabilities.load(Ordering::Acquire))
    }

    pub(crate) fn set_capabilities(&self, capabilities: ClipboardGeneralCapabilityFlags) {
        self.capabilities.store(capabilities.bits(), Ordering::Release);
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Invalidates data of the previous remote clipboard, returning the new generation.
    pub(crate) fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
    }

    pub(crate) fn session(&self) -> u64 {
        self.session.load(Ordering::Acquire)
    }

    /// Invalidates everything fetched or requested in the previous channel.
    pub(crate) fn start_session(&self) {
        self.session.fetch_add(1, Ordering::AcqRel);
        self.next_generation();
        self.set_capabilities(ClipboardGeneralCapabilityFlags::empty());
        self.lock_released_locks().clear();
        // The previous channel answers nothing more: its waiters fail at once.
        self.lock_waiters().clear();
    }

    /// Invalidates everything handed out, for good: the clipboard is going away, and streams or
    /// data objects that outlive it fail instead of waiting for answers.
    pub(crate) fn close(&self) {
        self.start_session();
    }

    pub(crate) fn release_locks(&self, clip_data_ids: impl IntoIterator<Item = u32>) {
        self.lock_released_locks().extend(clip_data_ids);
    }

    pub(crate) fn is_lock_released(&self, clip_data_id: u32) -> bool {
        self.lock_released_locks().contains(&clip_data_id)
    }

    fn lock_released_locks(&self) -> std::sync::MutexGuard<'_, HashSet<u32>> {
        self.released_locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Sends `request` with a fresh stream id, returning where its response will arrive.
    ///
    /// `Cliprdr` answers every request it sent or rejected, synthesizing a failure on its own
    /// transfer timeout, so every waiter gets an answer. One nobody waits for anymore is dropped.
    pub(crate) fn request_file_contents(
        &self,
        mut request: FileContentsRequest,
    ) -> mpsc::Receiver<FileContentsResponse<'static>> {
        let stream_id = self.next_stream_id.fetch_add(1, Ordering::AcqRel);
        request.stream_id = stream_id;

        let (sender, receiver) = mpsc::channel();
        self.lock_waiters().insert(stream_id, sender);

        self.send(ClipboardMessage::SendFileContentsRequest(request));

        receiver
    }

    /// Hands a file contents response to the stream waiting for it.
    pub(crate) fn deliver(&self, response: FileContentsResponse<'static>) {
        let waiter = self.lock_waiters().remove(&response.stream_id());

        match waiter {
            Some(waiter) => {
                // The stream may have given up on it meanwhile.
                let _ = waiter.send(response);
            }
            None => debug!(
                stream_id = response.stream_id(),
                "Dropping unexpected file contents response"
            ),
        }
    }

    fn lock_waiters(&self) -> std::sync::MutexGuard<'_, HashMap<u32, mpsc::Sender<FileContentsResponse<'static>>>> {
        self.waiters.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
