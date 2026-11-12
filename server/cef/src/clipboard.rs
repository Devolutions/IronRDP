//! CLIPRDR bridge for the browser's process-private clipboard.
//!
//! [`Clipboard::new`] provides a receiver for client copies; browser copies go through [`Clipboard::offer`].
//! Clipboard data never passes through the host clipboard.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags, FileContentsRequest,
    FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId, OwnedFormatDataResponse,
};
use ironrdp_cliprdr_format::{bitmap, html};
use ironrdp_core::{IntoOwned as _, impl_as_any};
use ironrdp_server::{CliprdrServerFactory, ServerEvent, ServerEventSender};
use tokio::sync::{mpsc::UnboundedSender, watch};

/// Largest accepted representation, in bytes, in either direction.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;

/// Format ID this server assigns to the registered `HTML Format` it offers.
const HTML_FORMAT_ID: ClipboardFormatId = ClipboardFormatId(0xC000);

/// Representations of one clipboard item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub text: Option<String>,
    /// HTML fragment markup, without the `CF_HTML` envelope.
    pub html: Option<String>,
    pub png: Option<Vec<u8>>,
}

impl Snapshot {
    /// Drops representations larger than [`MAX_BYTES`].
    pub fn bounded(mut self) -> Self {
        self.text = self.text.filter(|text| text.len() <= MAX_BYTES);
        self.html = self.html.filter(|html| html.len() <= MAX_BYTES);
        self.png = self.png.filter(|png| png.len() <= MAX_BYTES);
        self
    }

    fn formats(&self) -> Vec<ClipboardFormat> {
        let mut formats = Vec::new();
        if self.text.is_some() {
            formats.push(ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT));
        }
        if self.html.is_some() {
            formats.push(ClipboardFormat::new(HTML_FORMAT_ID).with_name(ClipboardFormatName::HTML));
        }
        if self.png.is_some() {
            formats.push(ClipboardFormat::new(ClipboardFormatId::CF_DIBV5));
            formats.push(ClipboardFormat::new(ClipboardFormatId::CF_DIB));
        }
        formats
    }
}

#[derive(Debug, Default)]
struct Shared {
    events: Option<UnboundedSender<ServerEvent>>,
    local: Snapshot,
}

impl Shared {
    fn send(&self, message: ClipboardMessage) {
        let Some(events) = &self.events else {
            return;
        };
        if events.send(ServerEvent::Clipboard(message)).is_err() {
            tracing::debug!("RDP server stopped before a clipboard message was sent");
        }
    }
}

/// Connects the browser clipboard to the RDP server's `CLIPRDR` channel.
#[derive(Clone, Debug)]
pub struct Clipboard {
    shared: Arc<Mutex<Shared>>,
    incoming: watch::Sender<Option<Snapshot>>,
}

impl Clipboard {
    /// Returns the bridge and a receiver for the latest complete client clipboard item.
    ///
    /// The receiver starts at `None`; each new update replaces any unread item.
    pub fn new() -> (Self, watch::Receiver<Option<Snapshot>>) {
        let (incoming, receiver) = watch::channel(None);
        let clipboard = Self {
            shared: Arc::new(Mutex::new(Shared::default())),
            incoming,
        };
        (clipboard, receiver)
    }

    /// Advertises new browser clipboard content to the client.
    pub fn offer(&self, snapshot: Snapshot) {
        let mut shared = self.shared.lock().expect("clipboard state poisoned");
        shared.local = snapshot.bounded();
        let formats = shared.local.formats();
        shared.send(ClipboardMessage::SendInitiateCopy(formats));
    }

    pub fn factory(&self) -> Box<dyn CliprdrServerFactory> {
        Box::new(self.clone())
    }
}

impl CliprdrBackendFactory for Clipboard {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(Backend {
            shared: Arc::clone(&self.shared),
            incoming: self.incoming.clone(),
            queue: VecDeque::new(),
            in_flight: None,
            stale: false,
            pending: false,
            received: Snapshot::default(),
        })
    }
}

impl ServerEventSender for Clipboard {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        self.shared.lock().expect("clipboard state poisoned").events = Some(sender);
    }
}

impl CliprdrServerFactory for Clipboard {}

#[derive(Clone, Copy, Debug)]
enum Fetch {
    Text,
    Html(ClipboardFormatId),
    Image { dibv5: bool },
}

impl Fetch {
    fn format(self) -> ClipboardFormatId {
        match self {
            Self::Text => ClipboardFormatId::CF_UNICODETEXT,
            Self::Html(id) => id,
            Self::Image { dibv5: true } => ClipboardFormatId::CF_DIBV5,
            Self::Image { dibv5: false } => ClipboardFormatId::CF_DIB,
        }
    }
}

/// Per-connection `CLIPRDR` backend.
///
/// Browser paste reads the private clipboard synchronously, so every supported representation of a
/// client copy is fetched eagerly, one request at a time, and delivered as one item.
#[derive(Debug)]
struct Backend {
    shared: Arc<Mutex<Shared>>,
    incoming: watch::Sender<Option<Snapshot>>,
    queue: VecDeque<Fetch>,
    in_flight: Option<Fetch>,
    /// The response to `in_flight` belongs to a superseded client copy.
    stale: bool,
    /// INVARIANT: After requested formats respond, publish one snapshot for the latest list, even if empty.
    pending: bool,
    received: Snapshot,
}

impl_as_any!(Backend);

impl Backend {
    fn send(&self, message: ClipboardMessage) {
        self.shared.lock().expect("clipboard state poisoned").send(message);
    }

    fn request_next(&mut self) {
        if self.in_flight.is_some() {
            return;
        }
        if let Some(fetch) = self.queue.pop_front() {
            self.in_flight = Some(fetch);
            self.send(ClipboardMessage::SendInitiatePaste(fetch.format()));
        } else if self.pending {
            self.pending = false;
            let snapshot = std::mem::take(&mut self.received);
            self.incoming.send_replace(Some(snapshot));
        }
    }

    fn accept(&mut self, fetch: Fetch, response: &FormatDataResponse<'_>) {
        match fetch {
            Fetch::Text => match response.to_unicode_string() {
                Ok(text) if text.len() <= MAX_BYTES => self.received.text = Some(text),
                Ok(_) => tracing::debug!("Dropped oversized client clipboard text"),
                Err(error) => tracing::debug!(%error, "Dropped invalid client clipboard text"),
            },
            Fetch::Html(_) => match html::cf_html_to_plain_html(response.data()) {
                Ok(fragment) if fragment.len() <= MAX_BYTES => self.received.html = Some(fragment.to_owned()),
                Ok(_) => tracing::debug!("Dropped oversized client clipboard HTML"),
                Err(error) => tracing::debug!(%error, "Dropped invalid client clipboard HTML"),
            },
            Fetch::Image { dibv5 } => {
                let png = if dibv5 {
                    bitmap::dibv5_to_png(response.data())
                } else {
                    bitmap::dib_to_png(response.data())
                };
                match png {
                    Ok(png) if png.len() <= MAX_BYTES => self.received.png = Some(png),
                    Ok(_) => tracing::debug!("Dropped oversized client clipboard image"),
                    Err(error) => tracing::debug!(%error, "Dropped invalid client clipboard image"),
                }
            }
        }
    }
}

impl CliprdrBackend for Backend {
    fn temporary_directory(&self) -> &str {
        ""
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_ready(&mut self) {}

    fn on_request_format_list(&mut self) {
        let shared = self.shared.lock().expect("clipboard state poisoned");
        let formats = shared.local.formats();
        shared.send(ClipboardMessage::SendInitiateCopy(formats));
    }

    fn on_process_negotiated_capabilities(&mut self, _capabilities: ClipboardGeneralCapabilityFlags) {}

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        let has = |id| available_formats.iter().any(|format| format.id() == id);
        self.queue.clear();
        self.received = Snapshot::default();
        self.stale = self.in_flight.is_some();
        self.pending = true;
        if has(ClipboardFormatId::CF_UNICODETEXT) {
            self.queue.push_back(Fetch::Text);
        }
        // The client assigns registered format IDs; match the name.
        if let Some(format) = available_formats
            .iter()
            .find(|format| format.name() == Some(&ClipboardFormatName::HTML))
        {
            self.queue.push_back(Fetch::Html(format.id()));
        }
        if has(ClipboardFormatId::CF_DIBV5) {
            self.queue.push_back(Fetch::Image { dibv5: true });
        } else if has(ClipboardFormatId::CF_DIB) {
            self.queue.push_back(Fetch::Image { dibv5: false });
        }
        self.request_next();
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let response = {
            let shared = self.shared.lock().expect("clipboard state poisoned");
            let local = &shared.local;
            match request.format {
                ClipboardFormatId::CF_UNICODETEXT => local
                    .text
                    .as_deref()
                    .map(|text| FormatDataResponse::new_unicode_string(text).into_owned()),
                id if id == HTML_FORMAT_ID => local
                    .html
                    .as_deref()
                    .map(|fragment| FormatDataResponse::new_data(html::plain_html_to_cf_html(fragment).into_bytes())),
                ClipboardFormatId::CF_DIBV5 => local
                    .png
                    .as_deref()
                    .and_then(|png| bitmap::png_to_cf_dibv5(png).ok())
                    .map(FormatDataResponse::new_data),
                ClipboardFormatId::CF_DIB => local
                    .png
                    .as_deref()
                    .and_then(|png| bitmap::png_to_cf_dib(png).ok())
                    .map(FormatDataResponse::new_data),
                _ => None,
            }
        };
        let response: OwnedFormatDataResponse = response.unwrap_or_else(FormatDataResponse::new_error);
        self.send(ClipboardMessage::SendFormatData(response));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        let Some(fetch) = self.in_flight.take() else {
            tracing::debug!("Dropped unexpected clipboard format data");
            return;
        };
        if std::mem::take(&mut self.stale) {
            // Responses arrive in request order, so this answers the superseded copy.
        } else if !response.is_error() {
            self.accept(fetch, &response);
        }
        self.request_next();
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        self.send(ClipboardMessage::SendFileContentsResponse(
            FileContentsResponse::new_error(request.stream_id),
        ));
    }

    fn on_file_contents_response(&mut self, _response: FileContentsResponse<'_>) {}

    fn on_lock(&mut self, _data_id: LockDataId) {}

    fn on_unlock(&mut self, _data_id: LockDataId) {}
}
