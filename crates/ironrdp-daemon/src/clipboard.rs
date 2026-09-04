//! In-memory `CLIPRDR` backend for the headless agent daemon.
//!
//! Bridges `CLIPRDR` to the `clipboard-get`/`clipboard-set` IPC operations. Plain Unicode text
//! (`CF_UNICODETEXT`), images (`CF_DIB`/`CF_DIBV5`, stored as PNG), and files (the
//! `FileGroupDescriptorW` file-list mechanism). A headless daemon has no host clipboard of its
//! own, so this backend holds the last content pushed by a `clipboard-set*` operation as its
//! local clipboard content and the last content received from the remote as its remote clipboard
//! content, both behind a lock shared with the daemon's IPC handlers. Content is a single logical
//! item at a time (text, image, or files, not several at once), matching how each
//! `clipboard-set*` call replaces whatever was there before.
//!
//! Files are a structurally different mechanism from the other three: `CLIPRDR`'s delayed-render
//! file-list PDUs (`initiate_file_copy`/`on_file_contents_request`/`on_remote_file_list`), not
//! `FormatDataRequest`/`FormatDataResponse`. `ironrdp_cliprdr::Cliprdr` owns the whole lock
//! lifecycle (snapshotting the local file list on an incoming `LockData`, auto-locking when a
//! remote file list arrives, timeout-driven expiry); this backend's `on_lock`/`on_unlock` stay
//! informational. `on_outgoing_locks_expired` is not, though: it aborts any active
//! `clipboard-get-file` fetch bound to the expiring lock rather than let it continue against a
//! remote clipboard that has since changed underneath it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ironrdp_client::rdp::RdpInputSender;
use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy, CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr::chunked_fetch::{ChunkedFetch, ChunkedFetchProgress};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags, FileContentsFlags,
    FileContentsRequest, FileContentsResponse, FileDescriptor, FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp_cliprdr_format::bitmap;
use ironrdp_pdu::ironrdp_core::{IntoOwned as _, impl_as_any};
use ironrdp_rpc::ipc::MAX_CLIPBOARD_IMAGE_BYTES;
use tracing::debug;

/// `stream_id` used for every `FileContentsRequest` this backend issues.
///
/// Only one fetch (`ClipboardState::active_fetch`) is ever outstanding at a time, matching the
/// one-outstanding-paste model already used for text/image; with no concurrent fetch to
/// disambiguate against, a fixed id is sufficient and avoids a counter.
pub(crate) const FILE_FETCH_STREAM_ID: u32 = 1;

/// Chunk size `clipboard_get_file` drives `ChunkedFetch` with, in bytes.
///
/// A pragmatic middle ground: large enough that a multi-megabyte file does not need thousands of
/// round trips, small enough that one chunk comfortably fits an IPC frame's own overhead.
pub(crate) const FILE_FETCH_CHUNK_SIZE: u32 = 256 * 1024;

/// One piece of clipboard content, in the daemon's own internal representation.
///
/// Images are stored as PNG bytes regardless of which `CF_DIB`/`CF_DIBV5` format the remote asks
/// for or offers; conversion to/from the wire's DIB byte layout happens at the point of use via
/// `ironrdp_cliprdr_format::bitmap`. Files are the wire-shaped metadata list, used identically for
/// what we offer locally and what the remote last advertised: local disk paths backing an offered
/// list live separately in `ClipboardState::local_file_paths`, since `FileDescriptor` itself
/// carries no local filesystem path.
#[derive(Debug, Clone)]
pub(crate) enum ClipboardContent {
    Text(String),
    Image(Vec<u8>),
    Files(Vec<FileDescriptor>),
}

/// Clipboard content shared between the `CLIPRDR` backend and the daemon's IPC handlers.
#[derive(Debug)]
pub(crate) struct ClipboardState {
    /// Set by a `clipboard-set*` operation; advertised to the remote and served on request.
    pub(crate) local: Option<ClipboardContent>,
    /// Set from the remote's last copy; read by a `clipboard-get*` operation.
    pub(crate) remote: Option<ClipboardContent>,
    /// Local filesystem paths backing `local`'s entries when `local` is
    /// `Some(ClipboardContent::Files(_))`, parallel-indexed to that list. Needed to serve
    /// `on_file_contents_request` by reading the actual bytes; empty otherwise.
    pub(crate) local_file_paths: Vec<PathBuf>,
    /// The `clipDataId` of the lock automatically covering `remote`'s file list, when `remote` is
    /// `Some(ClipboardContent::Files(_))` and locking was negotiated. Passed to `ChunkedFetch` so
    /// a fetch stays bound to the snapshot it was listed against.
    pub(crate) remote_file_lock_id: Option<u32>,
    /// The capabilities the active session negotiated, updated by
    /// `CliprdrBackend::on_process_negotiated_capabilities`. `clipboard_set_files` and
    /// `clipboard_get_file` check this before sending a file-transfer message: `Cliprdr` itself
    /// hard-errors a file-transfer call when `STREAM_FILECLIP_ENABLED` was not negotiated, and
    /// that error is session-fatal by the time it reaches `ironrdp-client`'s dispatcher, so this
    /// backend must fail cleanly at the IPC layer instead of sending the message at all.
    pub(crate) negotiated_capabilities: ClipboardGeneralCapabilityFlags,
    /// The in-progress `clipboard-get-file` fetch, if any. Only one is ever driven at a time.
    pub(crate) active_fetch: Option<ChunkedFetch>,
    /// The `clipDataId` `active_fetch` is bound to, if locking was negotiated. Tracked
    /// separately from `active_fetch` itself (`ChunkedFetch` does not expose its own bound id)
    /// so `on_outgoing_locks_expired` can recognize and abort a fetch whose lock just expired.
    pub(crate) active_fetch_lock_id: Option<u32>,
    /// The last [`ChunkedFetchProgress`] `on_file_contents_response` observed for `active_fetch`,
    /// once it is `Complete` or `Failed`. `ChunkedFetch::is_finished` collapses both outcomes to
    /// one bool; `clipboard_get_file` needs to tell them apart to know whether to return the
    /// fetched bytes or an error, so this is tracked alongside rather than re-derived.
    pub(crate) active_fetch_result: Option<ChunkedFetchProgress>,
}

impl Default for ClipboardState {
    fn default() -> Self {
        Self {
            local: None,
            remote: None,
            local_file_paths: Vec::new(),
            remote_file_lock_id: None,
            negotiated_capabilities: ClipboardGeneralCapabilityFlags::empty(),
            active_fetch: None,
            active_fetch_result: None,
            active_fetch_lock_id: None,
        }
    }
}

/// Forwards `CLIPRDR` events into the session's bounded input channel.
///
/// Mirrors `ironrdp_client`'s own internal proxy, which is crate-private and so not reusable
/// from here.
#[derive(Clone, Debug)]
struct AgentClipboardMessageProxy(RdpInputSender);

impl ClipboardMessageProxy for AgentClipboardMessageProxy {
    fn send_clipboard_message(&self, message: ClipboardMessage) {
        if self.0.send_clipboard(message).is_err() {
            debug!("Dropped clipboard message: session input channel is closed");
        }
    }
}

/// Builds a fresh [`AgentCliprdrBackend`] for each `CLIPRDR` channel initialization.
///
/// A new backend is required per connection (the channel is re-initialized on every reconnect),
/// but the clipboard content itself is daemon-lifetime state that survives a reconnect.
pub(crate) struct AgentCliprdrBackendFactory {
    state: Arc<Mutex<ClipboardState>>,
    input_tx: RdpInputSender,
    file_fetch_notify: Arc<tokio::sync::Notify>,
}

impl AgentCliprdrBackendFactory {
    pub(crate) fn new(
        state: Arc<Mutex<ClipboardState>>,
        input_tx: RdpInputSender,
        file_fetch_notify: Arc<tokio::sync::Notify>,
    ) -> Self {
        Self {
            state,
            input_tx,
            file_fetch_notify,
        }
    }
}

impl CliprdrBackendFactory for AgentCliprdrBackendFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(AgentCliprdrBackend {
            state: Arc::clone(&self.state),
            proxy: AgentClipboardMessageProxy(self.input_tx.clone()),
            file_fetch_notify: Arc::clone(&self.file_fetch_notify),
            pending_paste: None,
            pending_since_ms: None,
            desired_paste: None,
        })
    }
}

/// Which format this backend most recently requested from the remote via
/// [`ClipboardMessage::SendInitiatePaste`], so [`AgentCliprdrBackend::on_format_data_response`]
/// knows how to interpret the raw bytes that come back. `FormatDataResponse` does not itself carry
/// the format it answers; MS-RDPECLIP allows only one outstanding format-data request at a time,
/// so the backend has to remember what it last asked for.
#[derive(Debug, Clone, Copy)]
enum PendingPaste {
    Text,
    /// Image paste, remembering which DIB variant was requested so the right conversion function
    /// is used on the response.
    Image {
        dibv5: bool,
    },
    /// File-list paste. The list itself arrives through the separate
    /// [`CliprdrBackend::on_remote_file_list`] callback, not `on_format_data_response`; this
    /// variant exists so `on_remote_copy`'s one-outstanding-request bookkeeping and timeout still
    /// cover the file-list case the same way as the others.
    Files,
}

/// How long an outstanding paste request is given to answer before a newer remote copy is allowed
/// to supersede it.
///
/// MS-RDPECLIP allows only one outstanding `FormatDataRequest` at a time, and `FormatDataResponse`
/// carries no correlation ID, so issuing a second request while the first is unanswered risks a
/// stale response being misinterpreted as the answer to the new one. Queuing the newer request
/// behind the outstanding one avoids that, but a peer that never responds would otherwise wedge the
/// queue forever; this timeout bounds that.
const PENDING_PASTE_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug)]
struct AgentCliprdrBackend {
    state: Arc<Mutex<ClipboardState>>,
    proxy: AgentClipboardMessageProxy,
    /// Woken whenever `on_file_contents_response` advances or finishes `active_fetch`, so
    /// `clipboard_get_file`'s wait loop knows to re-check state instead of polling.
    file_fetch_notify: Arc<tokio::sync::Notify>,
    /// The format currently awaiting a `FormatDataResponse`, if any.
    pending_paste: Option<PendingPaste>,
    /// When `pending_paste` was issued, per [`CliprdrBackend::now_ms`].
    pending_since_ms: Option<u64>,
    /// A newer paste target that arrived while `pending_paste` was still outstanding. Issued once
    /// the outstanding one resolves (or times out).
    desired_paste: Option<(PendingPaste, ClipboardFormatId)>,
}

impl_as_any!(AgentCliprdrBackend);

/// Builds the [`ClipboardFormat`] list to advertise for the given local content.
///
/// Shared by [`AgentCliprdrBackend::on_request_format_list`] and the daemon's `clipboard_set*`
/// handlers, which advertise immediately to an already-connected session.
///
/// Does not cover [`ClipboardContent::Files`]: files use the separate
/// `ClipboardMessage::SendInitiateFileCopy` path (`Cliprdr::initiate_file_copy`), not
/// `SendInitiateCopy`, since offering a file list is `CLIPRDR`'s delayed-render file-list
/// mechanism rather than a `FormatDataRequest`-served format. Both call sites branch on
/// `ClipboardContent::Files` before reaching this function.
pub(crate) fn advertised_formats(content: &ClipboardContent) -> Vec<ClipboardFormat> {
    match content {
        ClipboardContent::Text(_) => vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)],
        // Advertise both DIB variants: DIBV5 is the richer format (alpha channel), but not every
        // peer understands it, so DIB is offered too. Both convert from the same PNG.
        ClipboardContent::Image(_) => vec![
            ClipboardFormat::new(ClipboardFormatId::CF_DIB),
            ClipboardFormat::new(ClipboardFormatId::CF_DIBV5),
        ],
        ClipboardContent::Files(_) => Vec::new(),
    }
}

impl AgentCliprdrBackend {
    /// Marks `paste` as the outstanding request and sends it. Only ever call this when no other
    /// request is outstanding (`pending_paste` is `None`), which `on_remote_copy` and
    /// `on_format_data_response` are responsible for maintaining.
    fn issue_paste(&mut self, (paste, format): (PendingPaste, ClipboardFormatId)) {
        self.pending_paste = Some(paste);
        self.pending_since_ms = Some(self.now_ms());
        self.desired_paste = None;
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendInitiatePaste(format));
    }

    /// Builds the response to one `FileContentsRequest` for a file we offered, reading from the
    /// local disk path `clipboard_set_files` recorded alongside the offered `FileDescriptor`.
    fn build_file_contents_response(&self, request: &FileContentsRequest) -> FileContentsResponse<'static> {
        use std::io::{Read as _, Seek as _, SeekFrom};

        let state = self.state.lock().expect("clipboard state poisoned");
        let index = match usize::try_from(request.index) {
            Ok(index) => index,
            Err(_) => return FileContentsResponse::new_error(request.stream_id),
        };
        let Some(ClipboardContent::Files(files)) = state.local.as_ref() else {
            return FileContentsResponse::new_error(request.stream_id);
        };
        let (Some(descriptor), Some(path)) = (files.get(index), state.local_file_paths.get(index)) else {
            return FileContentsResponse::new_error(request.stream_id);
        };
        if descriptor
            .attributes
            .is_some_and(|attributes| attributes.contains(ironrdp_cliprdr::pdu::ClipboardFileAttributes::DIRECTORY))
        {
            debug!(index, "File contents requested for a directory entry; refusing");
            return FileContentsResponse::new_error(request.stream_id);
        }
        let path = path.clone();
        drop(state);

        if request.flags.contains(FileContentsFlags::SIZE) {
            return match std::fs::metadata(&path) {
                Ok(metadata) => FileContentsResponse::new_size_response(request.stream_id, metadata.len()),
                Err(error) => {
                    debug!(%error, index, "Failed to stat offered file for a SIZE request");
                    FileContentsResponse::new_error(request.stream_id)
                }
            };
        }

        // RANGE. Cap the read below the caller's own `requested_size`: MS-RDPECLIP defines
        // `cbRequested` as an upper bound on what a responder may return, not a guarantee of how
        // much it will, so a peer asking for an unreasonably large single chunk gets a shorter
        // response instead of forcing a matching allocation here; `ChunkedFetch` on the requesting
        // side already handles a response shorter than requested by issuing another RANGE request
        // for the remainder.
        const MAX_RESPONSE_CHUNK_BYTES: u32 = 4 * 1024 * 1024;
        let read_len = request.requested_size.min(MAX_RESPONSE_CHUNK_BYTES);

        let mut file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(error) => {
                debug!(%error, index, "Failed to open offered file for a RANGE request");
                return FileContentsResponse::new_error(request.stream_id);
            }
        };
        if let Err(error) = file.seek(SeekFrom::Start(request.position)) {
            debug!(%error, index, position = request.position, "Failed to seek offered file");
            return FileContentsResponse::new_error(request.stream_id);
        }
        let mut buf = Vec::new();
        match file.take(u64::from(read_len)).read_to_end(&mut buf) {
            Ok(_) => FileContentsResponse::new_data_response(request.stream_id, buf),
            Err(error) => {
                debug!(%error, index, "Failed to read offered file");
                FileContentsResponse::new_error(request.stream_id)
            }
        }
    }
}

impl CliprdrBackend for AgentCliprdrBackend {
    fn temporary_directory(&self) -> &str {
        ".cliprdr"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
    }

    fn on_ready(&mut self) {}

    fn on_request_format_list(&mut self) {
        let state = self.state.lock().expect("clipboard state poisoned");
        match state.local.as_ref() {
            Some(ClipboardContent::Files(files)) => {
                if state
                    .negotiated_capabilities
                    .contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED)
                {
                    let files = files.clone();
                    drop(state);
                    self.proxy
                        .send_clipboard_message(ClipboardMessage::SendInitiateFileCopy(files));
                } else {
                    debug!("Not re-advertising local file list: file transfer was not negotiated");
                    drop(state);
                    self.proxy
                        .send_clipboard_message(ClipboardMessage::SendInitiateCopy(Vec::new()));
                }
            }
            Some(content) => {
                let formats = advertised_formats(content);
                drop(state);
                self.proxy
                    .send_clipboard_message(ClipboardMessage::SendInitiateCopy(formats));
            }
            None => {
                drop(state);
                self.proxy
                    .send_clipboard_message(ClipboardMessage::SendInitiateCopy(Vec::new()));
            }
        }
    }

    fn on_process_negotiated_capabilities(&mut self, capabilities: ClipboardGeneralCapabilityFlags) {
        debug!(?capabilities, "CLIPRDR capabilities negotiated");
        self.state
            .lock()
            .expect("clipboard state poisoned")
            .negotiated_capabilities = capabilities;
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        // The remote clipboard changed: whatever content was cached no longer reflects it.
        {
            let mut state = self.state.lock().expect("clipboard state poisoned");
            state.remote = None;
            state.remote_file_lock_id = None;
            // Abandoning an in-progress fetch here without marking it failed would leave its
            // waiting `clipboard_get_file` caller unable to tell its fetch was interrupted from a
            // fresh one nobody has started yet: it would keep waiting until its own timeout, and
            // a second `clipboard_get_file` call in the meantime would see `active_fetch: None`
            // and start a new fetch, whose result the first caller's notified-but-stale wakeup
            // could then read as its own. Failing it explicitly and waking any waiter now closes
            // that window.
            if state.active_fetch.take().is_some() {
                state.active_fetch_lock_id = None;
                state.active_fetch_result = Some(ChunkedFetchProgress::Failed);
                drop(state);
                self.file_fetch_notify.notify_waiters();
            }
        }

        // Prefer the richest representation offered: files over image over text, DIBV5 over DIB.
        let has_dibv5 = available_formats
            .iter()
            .any(|format| format.id() == ClipboardFormatId::CF_DIBV5);
        let has_dib = available_formats
            .iter()
            .any(|format| format.id() == ClipboardFormatId::CF_DIB);
        let has_text = available_formats
            .iter()
            .any(|format| format.id() == ClipboardFormatId::CF_UNICODETEXT);
        // The remote assigns its own ID to the registered file-list format; match by name and use
        // whatever ID it picked, same reasoning as the HTML format elsewhere in this file.
        let remote_file_list_id = available_formats
            .iter()
            .find(|format| format.name() == Some(&ClipboardFormatName::FILE_LIST))
            .map(ClipboardFormat::id);

        let target = if let Some(file_list_id) = remote_file_list_id {
            Some((PendingPaste::Files, file_list_id))
        } else if has_dibv5 {
            Some((PendingPaste::Image { dibv5: true }, ClipboardFormatId::CF_DIBV5))
        } else if has_dib {
            Some((PendingPaste::Image { dibv5: false }, ClipboardFormatId::CF_DIB))
        } else if has_text {
            Some((PendingPaste::Text, ClipboardFormatId::CF_UNICODETEXT))
        } else {
            None
        };
        let Some(target) = target else {
            self.pending_paste = None;
            self.pending_since_ms = None;
            self.desired_paste = None;
            return;
        };

        // A paste is already outstanding: Queue this one behind it rather than issuing a second
        // request, unless the outstanding one has been waiting long enough that the peer has
        // likely abandoned it. See `PENDING_PASTE_TIMEOUT_MS` for why this matters.
        if let Some(since) = self.pending_since_ms {
            if self.elapsed_ms(since) < PENDING_PASTE_TIMEOUT_MS {
                self.desired_paste = Some(target);
                return;
            }
            debug!("Pending paste timed out; abandoning it for the newer copy");
        }

        self.issue_paste(target);
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let content = self.state.lock().expect("clipboard state poisoned").local.clone();
        let response = match (request.format, content) {
            (ClipboardFormatId::CF_UNICODETEXT, Some(ClipboardContent::Text(text))) => {
                FormatDataResponse::new_unicode_string(&text).into_owned()
            }
            (ClipboardFormatId::CF_DIB, Some(ClipboardContent::Image(png))) => match bitmap::png_to_cf_dib(&png) {
                Ok(dib) => FormatDataResponse::new_data(dib).into_owned(),
                Err(error) => {
                    debug!(%error, "PNG to CF_DIB conversion failed");
                    FormatDataResponse::new_error()
                }
            },
            (ClipboardFormatId::CF_DIBV5, Some(ClipboardContent::Image(png))) => match bitmap::png_to_cf_dibv5(&png) {
                Ok(dibv5) => FormatDataResponse::new_data(dibv5).into_owned(),
                Err(error) => {
                    debug!(%error, "PNG to CF_DIBV5 conversion failed");
                    FormatDataResponse::new_error()
                }
            },
            _ => FormatDataResponse::new_error(),
        };
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendFormatData(response));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        self.pending_since_ms = None;

        if response.is_error() {
            self.pending_paste = None;
        } else if let Some(pending) = self.pending_paste.take() {
            let content = match pending {
                PendingPaste::Text => response.to_unicode_string().ok().map(ClipboardContent::Text),
                PendingPaste::Image { dibv5 } => {
                    let converted = if dibv5 {
                        bitmap::dibv5_to_png(response.data())
                    } else {
                        bitmap::dib_to_png(response.data())
                    };
                    match converted {
                        Ok(png) if png.len() > MAX_CLIPBOARD_IMAGE_BYTES => {
                            debug!(
                                png_len = png.len(),
                                "Remote image exceeds the clipboard RPC transport limit; dropped"
                            );
                            None
                        }
                        Ok(png) => Some(ClipboardContent::Image(png)),
                        Err(error) => {
                            debug!(%error, "DIB to PNG conversion failed");
                            None
                        }
                    }
                }
                // A successful file-list response is intercepted by `Cliprdr` itself and
                // delivered via `on_remote_file_list`, never reaching here; this arm exists only
                // for the fallback case where the response failed to parse as a file list, which
                // `Cliprdr` forwards here with `is_error()` still false.
                PendingPaste::Files => {
                    debug!("File list response could not be parsed; dropped");
                    None
                }
            };
            if let Some(content) = content {
                self.state.lock().expect("clipboard state poisoned").remote = Some(content);
            }
        } else {
            debug!("Format data response received with no pending paste; dropped");
        }

        // A newer copy arrived while this response was outstanding: Issue it now.
        if let Some(target) = self.desired_paste.take() {
            self.issue_paste(target);
        }
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        let response = self.build_file_contents_response(&request);
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendFileContentsResponse(response));
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        let mut state = self.state.lock().expect("clipboard state poisoned");
        let Some(fetch) = state.active_fetch.as_mut() else {
            debug!("File contents response received with no active fetch; dropped");
            return;
        };
        if fetch.stream_id() != response.stream_id() {
            debug!(
                expected = fetch.stream_id(),
                got = response.stream_id(),
                "File contents response stream id mismatch; dropped"
            );
            return;
        }

        let progress = fetch.on_response(&response);
        let next_request = if matches!(progress, ChunkedFetchProgress::InProgress) {
            fetch.next_request()
        } else {
            None
        };
        if matches!(progress, ChunkedFetchProgress::Complete | ChunkedFetchProgress::Failed) {
            state.active_fetch_result = Some(progress);
        }
        drop(state);

        match next_request {
            Some(next) => self
                .proxy
                .send_clipboard_message(ClipboardMessage::SendFileContentsRequest(next)),
            None => {
                if matches!(progress, ChunkedFetchProgress::Complete | ChunkedFetchProgress::Failed) {
                    self.file_fetch_notify.notify_waiters();
                }
            }
        }
    }

    fn on_lock(&mut self, data_id: LockDataId) {
        // `Cliprdr` already snapshots `local_file_list` for this id and releases it on the
        // matching unlock; nothing for this backend to do beyond logging.
        debug!(?data_id, "Remote locked local clipboard file list");
    }

    fn on_unlock(&mut self, data_id: LockDataId) {
        debug!(?data_id, "Remote unlocked local clipboard file list");
    }

    fn on_remote_file_list(&mut self, files: &[FileDescriptor], clip_data_id: Option<u32>) {
        debug!(file_count = files.len(), ?clip_data_id, "Received remote file list");
        let mut state = self.state.lock().expect("clipboard state poisoned");
        state.remote = Some(ClipboardContent::Files(files.to_vec()));
        state.remote_file_lock_id = clip_data_id;

        // Success for a `PendingPaste::Files` request arrives here, not through
        // `on_format_data_response`, so this callback is what actually clears it.
        self.pending_paste = None;
        self.pending_since_ms = None;
        if let Some(target) = self.desired_paste.take() {
            drop(state);
            self.issue_paste(target);
        }
    }

    fn on_outgoing_locks_expired(&mut self, clip_data_ids: &[LockDataId]) {
        let mut state = self.state.lock().expect("clipboard state poisoned");
        let Some(active_id) = state.active_fetch_lock_id else {
            return;
        };
        if clip_data_ids.iter().any(|id| id.0 == active_id) {
            debug!(
                clip_data_id = active_id,
                "Active file fetch's lock expired; aborting rather than continue against a changed remote clipboard"
            );
            state.active_fetch = None;
            state.active_fetch_lock_id = None;
            state.active_fetch_result = Some(ChunkedFetchProgress::Failed);
            drop(state);
            self.file_fetch_notify.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ClipboardContent, advertised_formats};

    #[test]
    fn advertised_formats_is_empty_for_files() {
        // Files use `SendInitiateFileCopy`, not `SendInitiateCopy`; both call sites branch on
        // `ClipboardContent::Files` before reaching `advertised_formats`, so this is defensive
        // documentation of that invariant rather than a path either call site actually takes.
        let formats = advertised_formats(&ClipboardContent::Files(Vec::new()));
        assert!(formats.is_empty());
    }
}
