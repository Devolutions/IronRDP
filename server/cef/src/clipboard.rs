//! CLIPRDR bridge for the browser's process-private clipboard.
//!
//! [`Clipboard::new`] provides a receiver for client copies; browser copies go through [`Clipboard::offer`].
//! Clipboard data never passes through the host clipboard.

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr::chunked_fetch::{ChunkedFetch, ChunkedFetchProgress};
use ironrdp_cliprdr::is_windows_device_name;
use ironrdp_cliprdr::pdu::{
    ClipboardFileAttributes, ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags,
    FileContentsRequest, FileContentsResponse, FileDescriptor, FormatDataRequest, FormatDataResponse, LockDataId,
    OwnedFormatDataResponse,
};
use ironrdp_cliprdr_format::{bitmap, html};
use ironrdp_core::{IntoOwned as _, impl_as_any};
use ironrdp_server::{CliprdrServerFactory, ServerEvent, ServerEventSender};
use tokio::sync::{mpsc::UnboundedSender, oneshot, watch};

/// Largest accepted representation, in bytes, in either direction.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_FILE_COUNT: usize = 32;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SESSION_FILE_BYTES: u64 = 64 * 1024 * 1024;
const FILE_CHUNK_BYTES: u32 = 64 * 1024;
const MAX_PENDING_PAYLOAD_RESPONSES: usize = 32;

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

/// One client clipboard item, including its staged files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipboardItem {
    pub snapshot: Snapshot,
    pub files: Vec<PathBuf>,
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
    connection: Option<Arc<AtomicBool>>,
    retired: bool,
    /// INVARIANT: At most one browser offer waits in the event queue; newer offers coalesce into `offer_dirty`.
    offer_in_flight: Option<oneshot::Receiver<()>>,
    offer_token: Option<Arc<AtomicBool>>,
    offer_dirty: bool,
    /// INVARIANT: Tracks uncompleted payload-bearing responses so queued clipboard bytes stay bounded.
    completions: VecDeque<(oneshot::Receiver<()>, usize)>,
    staged_directories: Vec<PathBuf>,
    staged_bytes: u64,
    staged_file_count: usize,
    file_copy_generation: u64,
}

impl Shared {
    fn prune_completed(&mut self) -> usize {
        if self
            .offer_in_flight
            .as_mut()
            .is_some_and(|rx| !matches!(rx.try_recv(), Err(oneshot::error::TryRecvError::Empty)))
        {
            self.offer_in_flight = None;
        }
        self.completions
            .retain_mut(|(completion, _)| matches!(completion.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
        self.completions.iter().map(|(_, bytes)| *bytes).sum()
    }

    fn cancel_offers(&mut self) {
        if let Some(token) = self.offer_token.take() {
            token.store(false, Ordering::Release);
        }
        self.offer_in_flight = None;
        self.offer_dirty = false;
    }

    fn send_offer(&mut self) {
        let Some(connection) = self.connection.clone() else {
            return;
        };
        if !connection.load(Ordering::Acquire) {
            return;
        }
        let _ = self.prune_completed();
        if self.offer_in_flight.is_some() {
            self.offer_dirty = true;
            return;
        }
        let Some(events) = &self.events else {
            return;
        };
        self.offer_dirty = false;
        let formats = self.local.formats();
        let token = Arc::new(AtomicBool::new(true));
        let event_token = Arc::clone(&token);
        let (completion, receiver) = oneshot::channel();
        if events
            .send(ServerEvent::ClipboardWithValidity {
                message: ClipboardMessage::SendInitiateCopy(formats),
                completion,
                is_current: Box::new(move || event_token.load(Ordering::Acquire) && connection.load(Ordering::Acquire)),
            })
            .is_err()
        {
            tracing::debug!("RDP server stopped before a clipboard offer was sent");
        } else {
            self.offer_token = Some(token);
            self.offer_in_flight = Some(receiver);
        }
    }

    fn send(&mut self, message: ClipboardMessage, connection: &Arc<AtomicBool>) {
        if !connection.load(Ordering::Acquire) {
            return;
        }
        let _ = self.prune_completed();
        let payload_bytes = match &message {
            ClipboardMessage::SendFormatData(response) => response.data().len(),
            ClipboardMessage::SendFileContentsResponse(response) => response.data().len(),
            _ => 0,
        };
        let Some(events) = &self.events else {
            return;
        };
        if 0 < payload_bytes {
            let (completion, receiver) = oneshot::channel();
            if events
                .send(ServerEvent::ClipboardWithCompletion(message, completion))
                .is_err()
            {
                tracing::debug!("RDP server stopped before a clipboard message was sent");
            } else {
                self.completions.push_back((receiver, payload_bytes));
            }
        } else if events.send(ServerEvent::Clipboard(message)).is_err() {
            tracing::debug!("RDP server stopped before a clipboard message was sent");
        }
    }
}

/// Connects the browser clipboard to the RDP server's `CLIPRDR` channel.
#[derive(Clone, Debug)]
pub struct Clipboard {
    shared: Arc<Mutex<Shared>>,
    incoming: watch::Sender<Option<ClipboardItem>>,
    staging_dir: Option<PathBuf>,
}

impl Clipboard {
    /// Returns the bridge and a receiver for the latest complete client clipboard item.
    ///
    /// The receiver starts at `None`; each new update replaces any unread item.
    pub fn new() -> (Self, watch::Receiver<Option<ClipboardItem>>) {
        let (incoming, receiver) = watch::channel(None);
        let clipboard = Self {
            shared: Arc::new(Mutex::new(Shared::default())),
            incoming,
            staging_dir: None,
        };
        (clipboard, receiver)
    }

    /// Sets the private profile directory used to stage client clipboard files.
    pub fn with_staging_dir(mut self, staging_dir: PathBuf) -> Self {
        self.staging_dir = Some(staging_dir);
        self
    }

    /// Advertises new browser clipboard content to the client.
    pub fn offer(&self, snapshot: Snapshot) {
        let mut shared = self.shared.lock().expect("clipboard state poisoned");
        if shared.retired {
            return;
        }
        shared.local = snapshot.bounded();
        shared.send_offer();
    }

    /// Flushes a coalesced browser clipboard offer once an earlier offer finishes draining.
    pub fn tick(&self) {
        let mut shared = self.shared.lock().expect("clipboard state poisoned");
        if shared.retired {
            return;
        }
        let _ = shared.prune_completed();
        if shared.offer_dirty && shared.offer_in_flight.is_none() {
            shared.send_offer();
        }
    }

    pub(crate) fn disconnected(&self) -> anyhow::Result<()> {
        let staged_directories = {
            let mut shared = self.shared.lock().expect("clipboard state poisoned");
            if let Some(connection) = shared.connection.take() {
                connection.store(false, Ordering::Release);
            }
            shared.local = Snapshot::default();
            shared.retired = true;
            shared.cancel_offers();
            shared.completions.clear();
            shared.staged_bytes = 0;
            shared.staged_file_count = 0;
            std::mem::take(&mut shared.staged_directories)
        };
        self.incoming.send_replace(None);
        for directory in staged_directories {
            match fs::remove_dir_all(&directory) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to remove clipboard staging directory {}", directory.display())
                    });
                }
            }
        }
        Ok(())
    }

    pub fn factory(&self) -> Box<dyn CliprdrServerFactory> {
        Box::new(self.clone())
    }
}

impl CliprdrBackendFactory for Clipboard {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        let connection = Arc::new(AtomicBool::new(true));
        let mut shared = self.shared.lock().expect("clipboard state poisoned");
        if let Some(previous) = shared.connection.replace(Arc::clone(&connection)) {
            previous.store(false, Ordering::Release);
            shared.local = Snapshot::default();
            shared.cancel_offers();
            shared.completions.clear();
            self.incoming.send_replace(None);
        }
        shared.retired = false;
        drop(shared);
        Box::new(Backend {
            shared: Arc::clone(&self.shared),
            connection,
            incoming: self.incoming.clone(),
            staging_dir: self.staging_dir.clone(),
            queue: VecDeque::new(),
            in_flight: None,
            stale: false,
            pending: false,
            formats_complete: false,
            received: Snapshot::default(),
            received_files: None,
            file_transfer_enabled: false,
            file_transfer: None,
            next_stream_id: 1,
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
    Files(ClipboardFormatId),
}

impl Fetch {
    fn format(self) -> ClipboardFormatId {
        match self {
            Self::Text => ClipboardFormatId::CF_UNICODETEXT,
            Self::Html(id) => id,
            Self::Image { dibv5: true } => ClipboardFormatId::CF_DIBV5,
            Self::Image { dibv5: false } => ClipboardFormatId::CF_DIB,
            Self::Files(format) => format,
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
    connection: Arc<AtomicBool>,
    incoming: watch::Sender<Option<ClipboardItem>>,
    staging_dir: Option<PathBuf>,
    queue: VecDeque<Fetch>,
    in_flight: Option<Fetch>,
    /// The response to `in_flight` belongs to a superseded client copy.
    stale: bool,
    /// INVARIANT: Publish one complete item for the latest list, even if no formats are usable.
    pending: bool,
    formats_complete: bool,
    received: Snapshot,
    received_files: Option<Vec<PathBuf>>,
    file_transfer_enabled: bool,
    file_transfer: Option<FileTransfer>,
    next_stream_id: u32,
}

impl_as_any!(Backend);

#[derive(Debug)]
struct PendingFile {
    index: i32,
    name: String,
}

#[derive(Debug)]
struct ActiveFile {
    fetch: ChunkedFetch,
    path: PathBuf,
}

#[derive(Debug)]
struct FileTransfer {
    queue: VecDeque<PendingFile>,
    active: Option<ActiveFile>,
    clip_data_id: Option<u32>,
    staging_dir: PathBuf,
    completed_paths: Vec<PathBuf>,
}

impl Backend {
    fn send(&self, message: ClipboardMessage) {
        self.shared
            .lock()
            .expect("clipboard state poisoned")
            .send(message, &self.connection);
    }

    fn request_next(&mut self) {
        if self.in_flight.is_some() {
            return;
        }
        if let Some(fetch) = self.queue.pop_front() {
            self.in_flight = Some(fetch);
            self.send(ClipboardMessage::SendInitiatePaste(fetch.format()));
        } else if self.pending {
            self.formats_complete = true;
            self.publish_if_ready();
        }
    }

    fn publish_if_ready(&mut self) {
        if !self.connection.load(Ordering::Acquire) {
            return;
        }
        if !self.pending || !self.formats_complete {
            return;
        }
        let Some(files) = self.received_files.take() else {
            return;
        };
        self.pending = false;
        let snapshot = std::mem::take(&mut self.received);
        self.incoming.send_replace(Some(ClipboardItem { snapshot, files }));
    }

    fn start_file_transfer(&mut self, files: &[FileDescriptor], clip_data_id: Option<u32>) {
        let Some(staging_root) = self.staging_dir.as_ref() else {
            self.publish_file_transfer();
            return;
        };
        let mut queue = VecDeque::new();
        let remaining_files = {
            let shared = self.shared.lock().expect("clipboard state poisoned");
            MAX_FILE_COUNT.saturating_sub(shared.staged_file_count)
        };
        for (index, file) in files.iter().enumerate() {
            if queue.len() == remaining_files {
                tracing::debug!(max = MAX_FILE_COUNT, "Ignoring excess client clipboard files");
                break;
            }
            if file
                .attributes
                .is_some_and(|attributes| attributes.contains(ClipboardFileAttributes::DIRECTORY))
                || !is_safe_file_name(&file.name)
            {
                tracing::debug!(name = %file.name, "Ignoring unsafe client clipboard file");
                continue;
            }
            let Ok(index) = i32::try_from(index) else {
                tracing::debug!("Ignoring client clipboard file with an invalid index");
                continue;
            };
            queue.push_back(PendingFile {
                index,
                name: file.name.clone(),
            });
        }
        if queue.is_empty() {
            self.publish_file_transfer();
            return;
        }

        let generation = {
            let mut shared = self.shared.lock().expect("clipboard state poisoned");
            shared.file_copy_generation = shared.file_copy_generation.wrapping_add(1);
            shared.file_copy_generation
        };
        let staging_dir = staging_root.join(format!("copy-{generation}"));
        if let Err(error) = fs::create_dir_all(staging_root).and_then(|()| fs::create_dir(&staging_dir)) {
            tracing::warn!(%error, "Failed to create client clipboard staging directory");
            self.publish_file_transfer();
            return;
        }
        self.shared
            .lock()
            .expect("clipboard state poisoned")
            .staged_directories
            .push(staging_dir.clone());
        self.file_transfer = Some(FileTransfer {
            queue,
            active: None,
            clip_data_id,
            staging_dir,
            completed_paths: Vec::new(),
        });
        self.request_next_file();
    }

    fn request_next_file(&mut self) {
        let Some(transfer) = self.file_transfer.as_mut() else {
            return;
        };
        if transfer.active.is_some() {
            return;
        }
        let Some(pending) = transfer.queue.pop_front() else {
            self.publish_file_transfer();
            return;
        };

        let remaining_budget = {
            let shared = self.shared.lock().expect("clipboard state poisoned");
            MAX_SESSION_FILE_BYTES.saturating_sub(shared.staged_bytes)
        };
        if remaining_budget == 0 {
            tracing::warn!(max = MAX_SESSION_FILE_BYTES, "Client clipboard file budget exhausted");
            self.publish_file_transfer();
            return;
        }
        let stream_id = self.next_stream_id;
        self.next_stream_id = self.next_stream_id.wrapping_add(1);
        let Some(transfer) = self.file_transfer.as_mut() else {
            return;
        };
        let path = transfer
            .staging_dir
            .join(format!("{:04}", pending.index))
            .join(pending.name);
        let mut fetch = ChunkedFetch::new_with_size_query(
            stream_id,
            pending.index,
            FILE_CHUNK_BYTES,
            transfer.clip_data_id,
            remaining_budget.min(MAX_FILE_BYTES),
        );
        let Some(request) = fetch.next_request() else {
            tracing::error!("Failed to create client clipboard file-size request");
            self.request_next_file();
            return;
        };
        transfer.active = Some(ActiveFile { fetch, path });
        self.send(ClipboardMessage::SendFileContentsRequest(request));
    }

    fn publish_file_transfer(&mut self) {
        let paths = self
            .file_transfer
            .take()
            .map(|transfer| transfer.completed_paths)
            .unwrap_or_default();
        self.received_files = Some(paths);
        self.publish_if_ready();
    }

    fn finish_file(&mut self, path: PathBuf, data: &[u8]) -> anyhow::Result<()> {
        let bytes = u64::try_from(data.len()).unwrap_or(u64::MAX);
        {
            let mut shared = self.shared.lock().expect("clipboard state poisoned");
            anyhow::ensure!(
                bytes <= MAX_SESSION_FILE_BYTES.saturating_sub(shared.staged_bytes),
                "client clipboard file budget exceeded"
            );
            anyhow::ensure!(
                shared.staged_file_count < MAX_FILE_COUNT,
                "client clipboard file count exceeded"
            );
            shared.staged_bytes += bytes;
            shared.staged_file_count += 1;
        }
        let result = (|| {
            let parent = path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("client clipboard staging path has no parent"))?;
            fs::create_dir_all(parent)?;
            let mut file = OpenOptions::new().write(true).create_new(true).open(&path)?;
            if let Err(error) = file.write_all(data) {
                drop(file);
                if let Err(remove_error) = fs::remove_file(&path) {
                    tracing::warn!(error = %remove_error, "Failed to remove incomplete clipboard file");
                }
                return Err(error.into());
            }
            Ok(())
        })();
        if let Err(error) = result {
            let mut shared = self.shared.lock().expect("clipboard state poisoned");
            shared.staged_bytes -= bytes;
            shared.staged_file_count -= 1;
            return Err(error);
        }
        Ok(())
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
            Fetch::Files(_) => {}
        }
    }

    fn process_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        let Some(transfer) = self.file_transfer.as_mut() else {
            tracing::debug!(
                stream_id = response.stream_id(),
                "Dropping stale clipboard file response"
            );
            return;
        };
        let Some(mut active) = transfer.active.take() else {
            tracing::debug!(
                stream_id = response.stream_id(),
                "Dropping unexpected clipboard file response"
            );
            return;
        };
        if active.fetch.stream_id() != response.stream_id() {
            transfer.active = Some(active);
            tracing::debug!(
                stream_id = response.stream_id(),
                "Dropping mismatched clipboard file response"
            );
            return;
        }
        match active.fetch.on_response(&response) {
            ChunkedFetchProgress::InProgress => {
                let Some(request) = active.fetch.next_request() else {
                    tracing::error!("Clipboard file fetch lost its next request");
                    self.request_next_file();
                    return;
                };
                self.file_transfer
                    .as_mut()
                    .expect("file transfer remains active")
                    .active = Some(active);
                self.send(ClipboardMessage::SendFileContentsRequest(request));
            }
            ChunkedFetchProgress::Complete => {
                let data = active.fetch.into_data();
                match self.finish_file(active.path.clone(), &data) {
                    Ok(()) => {
                        if let Some(transfer) = &mut self.file_transfer {
                            transfer.completed_paths.push(active.path);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Failed to stage client clipboard file");
                    }
                }
                self.request_next_file();
            }
            ChunkedFetchProgress::Failed => {
                tracing::warn!(
                    stream_id = response.stream_id(),
                    "Client clipboard file transfer failed"
                );
                self.request_next_file();
            }
        }
    }
}

impl CliprdrBackend for Backend {
    fn temporary_directory(&self) -> &str {
        ""
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
            | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
    }

    fn on_ready(&mut self) {}

    fn on_request_format_list(&mut self) {
        if !self.connection.load(Ordering::Acquire) {
            return;
        }
        let mut shared = self.shared.lock().expect("clipboard state poisoned");
        shared.send_offer();
    }

    fn on_process_negotiated_capabilities(&mut self, capabilities: ClipboardGeneralCapabilityFlags) {
        self.file_transfer_enabled = capabilities.contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED);
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        if !self.connection.load(Ordering::Acquire) {
            return;
        }
        self.shared.lock().expect("clipboard state poisoned").cancel_offers();
        let has = |id| available_formats.iter().any(|format| format.id() == id);
        let file_format = if self.file_transfer_enabled && self.staging_dir.is_some() {
            available_formats
                .iter()
                .find(|format| format.name() == Some(&ClipboardFormatName::FILE_LIST))
                .map(ClipboardFormat::id)
        } else {
            None
        };
        self.queue.clear();
        self.received = Snapshot::default();
        self.file_transfer = None;
        self.received_files = file_format.is_none().then(Vec::new);
        self.stale = self.in_flight.is_some();
        self.pending = true;
        self.formats_complete = false;
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
        if let Some(format) = file_format {
            self.queue.push_back(Fetch::Files(format));
        }
        if has(ClipboardFormatId::CF_DIBV5) {
            self.queue.push_back(Fetch::Image { dibv5: true });
        } else if has(ClipboardFormatId::CF_DIB) {
            self.queue.push_back(Fetch::Image { dibv5: false });
        }
        self.request_next();
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        if !self.connection.load(Ordering::Acquire) {
            return;
        }
        let mut shared = self.shared.lock().expect("clipboard state poisoned");
        let queued_bytes = shared.prune_completed();
        let remaining_bytes = if MAX_PENDING_PAYLOAD_RESPONSES <= shared.completions.len() {
            0
        } else {
            MAX_BYTES.saturating_sub(queued_bytes)
        };
        let response = if remaining_bytes == 0 {
            None
        } else {
            let local = &shared.local;
            let encoded = match request.format {
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
            };
            encoded.filter(|response| response.data().len() <= remaining_bytes)
        };
        let response: OwnedFormatDataResponse = response.unwrap_or_else(FormatDataResponse::new_error);
        shared.send(ClipboardMessage::SendFormatData(response), &self.connection);
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if !self.connection.load(Ordering::Acquire) {
            return;
        }
        let Some(fetch) = self.in_flight.take() else {
            tracing::debug!("Dropped unexpected clipboard format data");
            return;
        };
        if std::mem::take(&mut self.stale) {
            // Responses arrive in request order, so this answers the superseded copy.
        } else if matches!(fetch, Fetch::Files(_)) {
            tracing::warn!("Failed to obtain client clipboard file list");
            self.received_files = Some(Vec::new());
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

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        if !self.connection.load(Ordering::Acquire) {
            return;
        }
        self.process_file_contents_response(response);
    }

    fn on_remote_file_list(&mut self, files: &[FileDescriptor], clip_data_id: Option<u32>) {
        if !self.connection.load(Ordering::Acquire) {
            return;
        }
        let Some(fetch) = self.in_flight.take() else {
            tracing::debug!("Dropping clipboard file list without a matching request");
            return;
        };
        if !matches!(fetch, Fetch::Files(_)) {
            self.in_flight = Some(fetch);
            tracing::warn!("Received a clipboard file list for an unexpected format request");
            return;
        }
        if std::mem::take(&mut self.stale) {
            self.request_next();
            return;
        }
        self.start_file_transfer(files, clip_data_id);
        self.request_next();
    }

    fn on_lock(&mut self, _data_id: LockDataId) {}

    fn on_unlock(&mut self, _data_id: LockDataId) {}
}

fn is_safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\' | ':'))
        && !is_windows_device_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_cliprdr::pdu::{ClipboardFormat, FormatDataResponse};
    use tokio::sync::mpsc::unbounded_channel;

    // oracle: mutant clipboard state from one RDP client is available to the next client.
    #[test]
    fn clipboard_state_does_not_cross_connection_backend_lifetimes() {
        let (clipboard, incoming) = Clipboard::new();
        let (sender, mut events) = unbounded_channel();
        let mut factory = clipboard.clone();
        factory.set_sender(sender);

        let mut first = factory.build_cliprdr_backend();
        first.on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]);
        let _request = events.try_recv().expect("clipboard request");
        first.on_format_data_response(FormatDataResponse::new_unicode_string("previous client"));
        clipboard.offer(Snapshot {
            text: Some("previous browser".to_owned()),
            ..Snapshot::default()
        });
        let old_offer = events.try_recv().expect("browser clipboard offer");
        clipboard.disconnected().expect("disconnect cleanup");
        clipboard.offer(Snapshot {
            text: Some("clipboard changed after disconnect".to_owned()),
            ..Snapshot::default()
        });
        drop(first);

        let mut second = factory.build_cliprdr_backend();
        second.on_request_format_list();
        let new_client_offer = events.try_recv().expect("new client clipboard offer");

        assert!(incoming.borrow().is_none());
        let ServerEvent::ClipboardWithValidity {
            message: ClipboardMessage::SendInitiateCopy(formats),
            completion,
            is_current,
        } = new_client_offer
        else {
            panic!("expected a clipboard format list with validity tracking");
        };
        assert!(formats.is_empty());
        assert!(!completion.is_closed());
        assert!(is_current());
        let ServerEvent::ClipboardWithValidity {
            completion, is_current, ..
        } = old_offer
        else {
            panic!("expected a clipboard offer with validity tracking");
        };
        assert!(completion.is_closed());
        assert!(!is_current());
    }

    // oracle: mutant staged client clipboard files remain available after disconnect.
    #[test]
    fn disconnect_removes_staged_client_files() {
        let root = tempfile::tempdir().expect("staging root");
        let directory = root.path().join("copy-1");
        std::fs::create_dir(&directory).expect("staging directory");
        std::fs::write(directory.join("client.bin"), b"secret").expect("staged file");
        let (clipboard, _) = Clipboard::new();
        clipboard
            .shared
            .lock()
            .expect("clipboard state poisoned")
            .staged_directories
            .push(directory.clone());

        clipboard.disconnected().expect("disconnect cleanup");

        assert!(!directory.exists());
    }
}
