//! In-memory `CLIPRDR` backend for the headless agent daemon.
//!
//! Bridges `CLIPRDR` to the `clipboard-get` / `clipboard-set` IPC operations. Plain Unicode text
//! only (`CF_UNICODETEXT`); no file transfer, no HTML or image formats. A headless daemon has no
//! host clipboard of its own, so this backend holds the last text pushed by `clipboard-set` as
//! its local clipboard content and the last text received from the remote as its remote
//! clipboard content, both behind a lock shared with the daemon's IPC handlers.

use std::sync::{Arc, Mutex};

use ironrdp_client::rdp::RdpInputSender;
use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy, CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse,
    FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp_pdu::ironrdp_core::{IntoOwned as _, impl_as_any};
use tracing::debug;

/// Clipboard text shared between the `CLIPRDR` backend and the daemon's IPC handlers.
#[derive(Debug, Default)]
pub(crate) struct ClipboardState {
    /// Set by `clipboard-set`; advertised to the remote as `CF_UNICODETEXT` and served on request.
    pub(crate) local: Option<String>,
    /// Set from the remote's last `CF_UNICODETEXT` response; read by `clipboard-get`.
    pub(crate) remote: Option<String>,
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
/// but the clipboard text itself is daemon-lifetime state that survives a reconnect.
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
        })
    }
}

#[derive(Debug)]
struct AgentCliprdrBackend {
    state: Arc<Mutex<ClipboardState>>,
    proxy: AgentClipboardMessageProxy,
}

impl_as_any!(AgentCliprdrBackend);

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
        let formats = if self.state.lock().expect("clipboard state poisoned").local.is_some() {
            vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]
        } else {
            Vec::new()
        };
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendInitiateCopy(formats));
    }

    fn on_process_negotiated_capabilities(&mut self, capabilities: ClipboardGeneralCapabilityFlags) {
        debug!(?capabilities, "CLIPRDR capabilities negotiated");
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        // The remote clipboard changed: whatever text was cached no longer reflects it.
        self.state.lock().expect("clipboard state poisoned").remote = None;
        if available_formats
            .iter()
            .any(|format| format.id() == ClipboardFormatId::CF_UNICODETEXT)
        {
            self.proxy
                .send_clipboard_message(ClipboardMessage::SendInitiatePaste(ClipboardFormatId::CF_UNICODETEXT));
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let response = if request.format == ClipboardFormatId::CF_UNICODETEXT {
            match self.state.lock().expect("clipboard state poisoned").local.clone() {
                Some(text) => FormatDataResponse::new_unicode_string(&text).into_owned(),
                None => FormatDataResponse::new_error(),
            }
        } else {
            FormatDataResponse::new_error()
        };
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendFormatData(response));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if response.is_error() {
            return;
        }
        if let Ok(text) = response.to_unicode_string() {
            self.state.lock().expect("clipboard state poisoned").remote = Some(text);
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
