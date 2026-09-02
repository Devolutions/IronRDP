//! In-memory `CLIPRDR` backend for the headless agent daemon.
//!
//! Bridges `CLIPRDR` to the `clipboard-get`/`clipboard-set` IPC operations. Plain Unicode text
//! (`CF_UNICODETEXT`) and images (`CF_DIB`/`CF_DIBV5`, stored as PNG); no file transfer, no HTML.
//! A headless daemon has no host clipboard of its own, so this backend holds the last content
//! pushed by a `clipboard-set*` operation as its local clipboard content and the last content
//! received from the remote as its remote clipboard content, both behind a lock shared with the
//! daemon's IPC handlers. Content is a single logical item at a time (text or image, not both at
//! once), matching how each `clipboard-set*` call replaces whatever was there before.

use std::sync::{Arc, Mutex};

use ironrdp_client::rdp::RdpInputSender;
use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy, CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse,
    FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp_cliprdr_format::bitmap;
use ironrdp_pdu::ironrdp_core::{IntoOwned as _, impl_as_any};
use ironrdp_rpc::ipc::MAX_CLIPBOARD_IMAGE_BYTES;
use tracing::debug;

/// One piece of clipboard content, in the daemon's own internal representation.
///
/// Images are stored as PNG bytes regardless of which `CF_DIB`/`CF_DIBV5` format the remote asks
/// for or offers; conversion to/from the wire's DIB byte layout happens at the point of use via
/// `ironrdp_cliprdr_format::bitmap`.
#[derive(Debug, Clone)]
pub(crate) enum ClipboardContent {
    Text(String),
    Image(Vec<u8>),
}

/// Clipboard content shared between the `CLIPRDR` backend and the daemon's IPC handlers.
#[derive(Debug, Default)]
pub(crate) struct ClipboardState {
    /// Set by a `clipboard-set*` operation; advertised to the remote and served on request.
    pub(crate) local: Option<ClipboardContent>,
    /// Set from the remote's last copy; read by a `clipboard-get*` operation.
    pub(crate) remote: Option<ClipboardContent>,
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
}

impl AgentCliprdrBackendFactory {
    pub(crate) fn new(state: Arc<Mutex<ClipboardState>>, input_tx: RdpInputSender) -> Self {
        Self { state, input_tx }
    }
}

impl CliprdrBackendFactory for AgentCliprdrBackendFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(AgentCliprdrBackend {
            state: Arc::clone(&self.state),
            proxy: AgentClipboardMessageProxy(self.input_tx.clone()),
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
pub(crate) fn advertised_formats(content: &ClipboardContent) -> Vec<ClipboardFormat> {
    match content {
        ClipboardContent::Text(_) => vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)],
        // Advertise both DIB variants: DIBV5 is the richer format (alpha channel), but not every
        // peer understands it, so DIB is offered too. Both convert from the same PNG.
        ClipboardContent::Image(_) => vec![
            ClipboardFormat::new(ClipboardFormatId::CF_DIB),
            ClipboardFormat::new(ClipboardFormatId::CF_DIBV5),
        ],
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
}

impl CliprdrBackend for AgentCliprdrBackend {
    fn temporary_directory(&self) -> &str {
        ".cliprdr"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        // No file transfer support: no STREAM_FILECLIP_ENABLED, no CAN_LOCK_CLIPDATA.
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_ready(&mut self) {}

    fn on_request_format_list(&mut self) {
        let formats = match self.state.lock().expect("clipboard state poisoned").local.as_ref() {
            Some(content) => advertised_formats(content),
            None => Vec::new(),
        };
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendInitiateCopy(formats));
    }

    fn on_process_negotiated_capabilities(&mut self, capabilities: ClipboardGeneralCapabilityFlags) {
        debug!(?capabilities, "CLIPRDR capabilities negotiated");
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        // The remote clipboard changed: whatever content was cached no longer reflects it.
        self.state.lock().expect("clipboard state poisoned").remote = None;

        // Prefer the richest representation offered: image over text, DIBV5 over DIB.
        let has_dibv5 = available_formats
            .iter()
            .any(|format| format.id() == ClipboardFormatId::CF_DIBV5);
        let has_dib = available_formats
            .iter()
            .any(|format| format.id() == ClipboardFormatId::CF_DIB);
        let has_text = available_formats
            .iter()
            .any(|format| format.id() == ClipboardFormatId::CF_UNICODETEXT);

        let target = if has_dibv5 {
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
        debug!(?request, "File contents request ignored: no file transfer support");
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        debug!(?response, "File contents response ignored: no file transfer support");
    }

    fn on_lock(&mut self, data_id: LockDataId) {
        debug!(?data_id, "Clipboard lock ignored: no file transfer support");
    }

    fn on_unlock(&mut self, data_id: LockDataId) {
        debug!(?data_id, "Clipboard unlock ignored: no file transfer support");
    }
}
