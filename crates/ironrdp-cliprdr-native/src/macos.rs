//! macOS CLIPRDR backend.
//!
//! Local text and images are delay-rendered when the remote requests them.
//! Remote text and images are requested when the remote copy is announced.
//! Local files are advertised with `FileGroupDescriptorW` and served on
//! request. Remote file lists are published to the macOS pasteboard as sparse
//! local placeholders; the bytes for each file are fetched only when a reader,
//! such as Finder, performs a coordinated read.
//!
//! `NSPasteboard` must be used from the main thread. [`MacClipboard`]
//! therefore owns the pasteboard state and the embedder must call
//! [`MacClipboard::poll`] from its main event loop and drop it on that same
//! thread. `CLIPRDR` callbacks from the session thread are forwarded to that
//! queue with a channel.
//!
//! Completed staging files are deleted through an `NSFileCoordinator` on a
//! dedicated cleanup thread. The coordinator waits for in-flight Finder copies
//! before removing their hidden source, so a completed Finder copy is not
//! interrupted by clipboard invalidation.

use core::cell::Cell;
use core::marker::PhantomData;
use core::panic::AssertUnwindSafe;
use core::ptr;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Seek as _, SeekFrom, Write as _};
use std::panic::catch_unwind;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use block2::{Block, StackBlock};
use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy, CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr::pdu::{
    ClipboardFileAttributes, ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags,
    FileContentsFlags, FileContentsRequest, FileContentsResponse, FileDescriptor, FormatDataRequest,
    FormatDataResponse, LockDataId,
};
use ironrdp_cliprdr_format::bitmap;
use ironrdp_core::{IntoOwned as _, impl_as_any};
use objc2::mutability::InteriorMutable;
use objc2::rc::{Allocated, Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{ClassType, DeclaredClass, declare_class, msg_send_id};
use objc2_app_kit::{
    NSBitmapImageRep, NSImage, NSPNGFileType, NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeString,
    NSPasteboardTypeTIFF, NSPasteboardWriting,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSData, NSDictionary, NSError, NSFileCoordinator, NSFileCoordinatorWritingOptions,
    NSFilePresenter, NSOperationQueue, NSString, NSURL,
};
use tracing::{debug, info, warn};

/// How long a lazy file fetch may go without a response before it is failed.
const LAZY_READ_TIMEOUT: Duration = Duration::from_secs(60);
const REMOTE_FORMAT_TIMEOUT: Duration = Duration::from_secs(60);
/// Largest `RANGE` request in the file-contents protocol.
const MAX_FILE_CHUNK: u32 = 1024 * 1024;
/// Largest text or image accepted from the remote clipboard.
const MAX_TRANSFER_BYTES: usize = 100 * 1024 * 1024;
/// Maximum number of entries in one local file list.
const MAX_LOCAL_FILES: usize = 1000;
/// How often byte progress is logged for a large file.
const PROGRESS_CHUNK: u64 = 64 * 1024 * 1024;
/// Staging directories older than this are removed when a backend is created.
const STAGING_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// How long a normal exit waits for coordinated Finder copies to finish.
///
/// If this expires, the staging tree is left for the startup prune. Waiting
/// only delays exiting the process; it never deletes the user's destination.
const EXIT_CLEANUP_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// UTF-16 pasteboard type without a byte-order mark.
const UTF16_PLAIN_TEXT: &str = "public.utf16-plain-text";
/// UTF-16 pasteboard type with an external byte-order mark.
const UTF16_EXTERNAL_PLAIN_TEXT: &str = "public.utf16-external-plain-text";

type LazyReadReply = Sender<Result<(), String>>;

/// Presenters run on an `NSOperationQueue`, which cannot receive a Rust
/// `Sender` as an Objective-C argument. One clipboard bridge owns this
/// process-wide endpoint; the id lets an old bridge drop without clearing a
/// newer one during reconnect.
static LAZY_EVENTS: Mutex<Option<(u64, Sender<BackendEvent>)>> = Mutex::new(None);
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub enum MacCliprdrError {
    /// The private staging directory could not be created.
    TemporaryDirectory(io::Error),
}

impl fmt::Display for MacCliprdrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TemporaryDirectory(error) => {
                write!(f, "cannot create the clipboard staging directory: {error}")
            }
        }
    }
}

impl core::error::Error for MacCliprdrError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::TemporaryDirectory(error) => Some(error),
        }
    }
}

/// macOS clipboard bridge for text, images, and files.
///
/// Create one instance per connected session and keep it alive while that
/// session is active. The embedder must call [`Self::poll`] from the main
/// thread regularly and drop the instance on that same thread; it processes
/// `CLIPRDR` callbacks, pasteboard changes, delayed format requests, and lazy
/// file reads. The backend itself is built per `CLIPRDR` channel through
/// [`Self::backend_factory`].
pub struct MacClipboard {
    proxy: Box<dyn ClipboardMessageProxy>,
    events: Receiver<BackendEvent>,
    events_tx: Sender<BackendEvent>,
    ready: bool,
    capabilities: ClipboardGeneralCapabilityFlags,
    last_change_count: Option<isize>,
    /// Set only while the pasteboard contains content this bridge published.
    owns_pasteboard: bool,
    download_root: PathBuf,
    temporary_directory: String,
    offer: Option<LazyOffer>,
    download: Option<LazyDownload>,
    queue: VecDeque<usize>,
    local_files: Vec<LocalFile>,
    local_file_snapshots: HashMap<u32, Vec<LocalFile>>,
    remote_queue: VecDeque<RemoteFormat>,
    remote_pending: Option<RemoteFormat>,
    remote_pending_since: Option<Instant>,
    session_id: u64,
    /// The pasteboard and presenter registration are main-thread only.
    _thread_marker: PhantomData<*const ()>,
}

impl fmt::Debug for MacClipboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MacClipboard")
            .field("ready", &self.ready)
            .field("capabilities", &self.capabilities)
            .field("offer", &self.offer)
            .field("download", &self.download)
            .finish_non_exhaustive()
    }
}

impl MacClipboard {
    /// Creates the main-thread clipboard bridge.
    ///
    /// `message_proxy` receives `CLIPRDR` messages generated by pasteboard
    /// activity, including file-contents requests made while Finder is reading
    /// a placeholder.
    pub fn new(message_proxy: impl ClipboardMessageProxy + 'static) -> Result<Self, MacCliprdrError> {
        let download_root = clipboard_root();
        create_private_dir_all(&download_root).map_err(MacCliprdrError::TemporaryDirectory)?;
        cleanup_old_staging(&download_root, STAGING_MAX_AGE);

        let (events_tx, events) = mpsc::channel();
        let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        match LAZY_EVENTS.lock() {
            Ok(mut events_slot) => *events_slot = Some((session_id, events_tx.clone())),
            Err(_) => warn!("the macOS clipboard event slot is poisoned"),
        }
        let temporary_directory = download_root.to_string_lossy().into_owned();

        Ok(Self {
            proxy: Box::new(message_proxy),
            events,
            events_tx,
            ready: false,
            capabilities: ClipboardGeneralCapabilityFlags::empty(),
            last_change_count: None,
            owns_pasteboard: false,
            download_root,
            temporary_directory,
            offer: None,
            download: None,
            queue: VecDeque::new(),
            local_files: Vec::new(),
            local_file_snapshots: HashMap::new(),
            remote_queue: VecDeque::new(),
            remote_pending: None,
            remote_pending_since: None,
            session_id,
            _thread_marker: PhantomData,
        })
    }

    /// Returns a factory suitable for the `CLIPRDR` channel.
    pub fn backend_factory(&self) -> Box<dyn CliprdrBackendFactory + Send> {
        Box::new(MacCliprdrBackendFactory {
            events: self.events_tx.clone(),
            temporary_directory: self.temporary_directory.clone(),
        })
    }

    /// Returns whether [`Self::poll`] has pending work or an active file offer.
    ///
    /// Embedders that otherwise sleep in their event loop can use this to keep
    /// a short timer alive while a Finder read or a remote format request is
    /// outstanding.
    #[must_use]
    pub fn needs_poll(&self) -> bool {
        self.offer.is_some()
            || self.download.is_some()
            || !self.remote_queue.is_empty()
            || self.remote_pending.is_some()
    }

    /// Processes pending work on the macOS main thread.
    ///
    /// This must be called from the thread that created the instance. It
    /// forwards queued `CLIPRDR` callbacks, notices local pasteboard changes,
    /// answers delayed format and file-content requests, and drives active
    /// lazy downloads. A Finder coordinated read blocks its own operation
    /// queue until this method completes that file's download or fails it.
    pub fn poll(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.handle_event(event);
        }

        self.poll_pasteboard();
        self.expire_remote_format();
        self.expire_download();
    }

    fn handle_event(&mut self, event: BackendEvent) {
        match event {
            BackendEvent::Ready => {
                self.ready = true;
                let pasteboard = unsafe {
                    // SAFETY: `poll` runs on the main thread; the general
                    // pasteboard is only touched from that thread.
                    NSPasteboard::generalPasteboard()
                };
                self.last_change_count = Some(unsafe {
                    // SAFETY: `pasteboard` is a valid general pasteboard on
                    // the main thread.
                    pasteboard.changeCount()
                });
            }
            BackendEvent::RequestFormatList => self.advertise_local(),
            BackendEvent::NegotiatedCapabilities(capabilities) => {
                self.capabilities = capabilities;
                debug!(?capabilities, "macOS CLIPRDR capabilities negotiated");
            }
            BackendEvent::RemoteCopy(formats) => self.handle_remote_copy(formats),
            BackendEvent::FormatDataRequest(request) => self.respond_local_format(request),
            BackendEvent::FormatDataResponse { data, is_error } => self.receive_remote_data(&data, is_error),
            BackendEvent::FileContentsRequest(request) => self.respond_local_file(request),
            BackendEvent::Lock(data_id) => {
                if !self.local_files.is_empty() {
                    self.local_file_snapshots.insert(data_id.0, self.local_files.clone());
                }
            }
            BackendEvent::Unlock(data_id) => {
                self.local_file_snapshots.remove(&data_id.0);
            }
            BackendEvent::RemoteFileList { files, clip_data_id } => {
                if !self
                    .capabilities
                    .contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED)
                {
                    warn!("remote offered files without negotiated file clipboard support");
                    return;
                }
                self.start_lazy_offer(files, clip_data_id);
            }
            BackendEvent::FileContentsResponse {
                stream_id,
                data,
                is_error,
            } => self.receive_lazy_contents(stream_id, &data, is_error),
            BackendEvent::LazyRead { path, reply } => self.request_lazy_read(path, reply),
        }
    }

    fn handle_remote_copy(&mut self, formats: Vec<ClipboardFormat>) {
        let file_list = formats
            .iter()
            .find(|format| is_file_list_format(format))
            .map(ClipboardFormat::id);

        self.clear_lazy_offer();
        self.remote_queue.clear();
        self.remote_pending = None;
        self.remote_pending_since = None;
        self.local_files.clear();
        // The remote clipboard replaced whatever was local. Clear the old
        // file URLs immediately; for a file list, the placeholders are
        // published again once the descriptors arrive. Text and images are
        // requested next and written into the empty pasteboard.
        self.clear_local_pasteboard();

        if let Some(format) = file_list {
            self.proxy
                .send_clipboard_message(ClipboardMessage::SendInitiatePaste(format));
            return;
        }

        self.remote_queue = remote_formats_to_request(&formats).into();
        self.request_next_remote_format();
    }

    fn poll_pasteboard(&mut self) {
        if !self.ready {
            return;
        }

        let pasteboard = unsafe {
            // SAFETY: `poll` runs on the main thread; the general pasteboard
            // is only touched from that thread.
            NSPasteboard::generalPasteboard()
        };
        let change_count = unsafe {
            // SAFETY: `pasteboard` is a valid general pasteboard on the main
            // thread.
            pasteboard.changeCount()
        };
        let Some(previous) = self.last_change_count else {
            self.last_change_count = Some(change_count);
            return;
        };
        if previous == change_count {
            return;
        }

        // The change was not one of our own writes: the user or another app
        // replaced the clipboard content. Never clear it on exit.
        self.owns_pasteboard = false;
        self.last_change_count = Some(change_count);
        self.advertise_local();
    }

    fn advertise_local(&mut self) {
        self.clear_lazy_offer();
        self.remote_queue.clear();
        self.remote_pending = None;
        self.remote_pending_since = None;
        self.local_files.clear();

        let mut roots = read_local_files()
            .into_iter()
            .filter(|path| !path.starts_with(&self.download_root))
            .collect::<Vec<_>>();
        roots.sort();
        roots.dedup();
        let entries = build_local_file_list(&roots);
        if !entries.is_empty()
            && self
                .capabilities
                .contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED)
        {
            self.local_files = entries;
            let descriptors = self.local_files.iter().map(|entry| entry.descriptor.clone()).collect();
            self.proxy
                .send_clipboard_message(ClipboardMessage::SendInitiateFileCopy(descriptors));
            return;
        }

        let mut formats = Vec::new();
        if read_local_text().is_some() {
            formats.push(ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT));
        }
        if local_image_available() {
            formats.push(ClipboardFormat::new(ClipboardFormatId::CF_DIB));
            formats.push(ClipboardFormat::new(ClipboardFormatId::CF_DIBV5));
        }
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendInitiateCopy(formats));
    }

    fn respond_local_format(&self, request: FormatDataRequest) {
        let data = match request.format {
            ClipboardFormatId::CF_UNICODETEXT => read_local_text().map(|text| encode_unicode_text(&text)),
            ClipboardFormatId::CF_DIB => read_local_png().and_then(|png| bitmap::png_to_cf_dib(&png).ok()),
            ClipboardFormatId::CF_DIBV5 => read_local_png().and_then(|png| bitmap::png_to_cf_dibv5(&png).ok()),
            _ => None,
        };

        let response = match data {
            Some(data) if !data.is_empty() && data.len() <= MAX_TRANSFER_BYTES => {
                FormatDataResponse::new_data(data).into_owned()
            }
            _ => FormatDataResponse::new_error().into_owned(),
        };
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendFormatData(response));
    }

    fn receive_remote_data(&mut self, data: &[u8], is_error: bool) {
        let Some(format) = self.remote_pending.take() else {
            debug!("ignoring clipboard data without a pending request");
            return;
        };
        self.remote_pending_since = None;

        if is_error {
            warn!(?format, "remote clipboard data is no longer available");
        } else if data.len() <= MAX_TRANSFER_BYTES {
            match format {
                RemoteFormat::Text => {
                    if let Some(text) = decode_unicode_text(data) {
                        self.write_pasteboard_text(&text);
                    }
                }
                RemoteFormat::Dib => match bitmap::dib_to_png(data) {
                    Ok(png) => {
                        self.write_pasteboard_png(&png);
                    }
                    Err(error) => warn!(%error, "cannot decode remote CF_DIB image"),
                },
                RemoteFormat::DibV5 => match bitmap::dibv5_to_png(data) {
                    Ok(png) => {
                        self.write_pasteboard_png(&png);
                    }
                    Err(error) => warn!(%error, "cannot decode remote CF_DIBV5 image"),
                },
            }
        } else {
            warn!(size = data.len(), "remote clipboard data is too large");
        }

        self.request_next_remote_format();
    }

    fn request_next_remote_format(&mut self) {
        if self.remote_pending.is_some() {
            return;
        }
        let Some(format) = self.remote_queue.pop_front() else {
            return;
        };
        self.remote_pending = Some(format);
        self.remote_pending_since = Some(Instant::now());
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendInitiatePaste(format.id()));
    }

    fn expire_remote_format(&mut self) {
        let Some(since) = self.remote_pending_since else {
            return;
        };
        if since.elapsed() < REMOTE_FORMAT_TIMEOUT {
            return;
        }
        warn!("remote clipboard data request timed out");
        self.remote_pending = None;
        self.remote_pending_since = None;
        self.request_next_remote_format();
    }

    fn respond_local_file(&self, request: FileContentsRequest) {
        let index = usize::try_from(request.index).ok();
        let files = request
            .data_id
            .and_then(|data_id| self.local_file_snapshots.get(&data_id))
            .unwrap_or(&self.local_files);
        let entry = index.and_then(|index| files.get(index));
        let response = match entry {
            Some(entry) => read_local_file_contents(&request, entry),
            None => FileContentsResponse::new_error(request.stream_id).into_owned(),
        };
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendFileContentsResponse(response));
    }

    fn write_pasteboard_text(&mut self, text: &str) {
        let pasteboard = unsafe {
            // SAFETY: called on the main thread.
            NSPasteboard::generalPasteboard()
        };
        let string = NSString::from_str(text);
        let written = unsafe {
            // SAFETY: `pasteboard` and `string` are valid for the call, and
            // the type is a standard pasteboard text type.
            pasteboard.setString_forType(&string, NSPasteboardTypeString)
        };
        if !written {
            warn!("the macOS pasteboard rejected remote text");
        }

        // Some applications still read the older UTF-16 pasteboard types.
        let utf16_type = NSString::from_str(UTF16_PLAIN_TEXT);
        let mut utf16 = Vec::with_capacity(text.len().saturating_mul(2));
        for unit in text.encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        let utf16 = NSData::with_bytes(&utf16);
        let utf16_written = unsafe {
            // SAFETY: `pasteboard`, `utf16_type`, and `utf16` are valid for
            // the call.
            pasteboard.setData_forType(Some(&utf16), &utf16_type)
        };
        self.owns_pasteboard = written || utf16_written;

        self.last_change_count = Some(unsafe {
            // SAFETY: called on the main thread.
            pasteboard.changeCount()
        });
    }

    fn write_pasteboard_png(&mut self, png: &[u8]) {
        let pasteboard = unsafe {
            // SAFETY: called on the main thread.
            NSPasteboard::generalPasteboard()
        };
        let data = NSData::with_bytes(png);
        let written = unsafe {
            // SAFETY: `pasteboard` and `data` are valid for the call, and the
            // type is a standard pasteboard image type.
            pasteboard.setData_forType(Some(&data), NSPasteboardTypePNG)
        };
        // Some older applications only ask for TIFF. Publishing both types
        // keeps the image pasteable without another round trip.
        let mut tiff_written = false;
        if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
            if let Some(tiff) = unsafe {
                // SAFETY: `image` is a valid image created from the PNG data.
                image.TIFFRepresentation()
            } {
                tiff_written = unsafe {
                    // SAFETY: `pasteboard` and `tiff` are valid for the call.
                    pasteboard.setData_forType(Some(&tiff), NSPasteboardTypeTIFF)
                };
            }
        }
        if !written {
            warn!("the macOS pasteboard rejected remote image data");
        }
        self.owns_pasteboard = written || tiff_written;
        self.last_change_count = Some(unsafe {
            // SAFETY: called on the main thread.
            pasteboard.changeCount()
        });
    }

    fn clear_local_pasteboard(&mut self) {
        let pasteboard = unsafe {
            // SAFETY: `poll` runs on the main thread; the general pasteboard
            // is only touched from that thread.
            NSPasteboard::generalPasteboard()
        };
        unsafe {
            // SAFETY: `pasteboard` is a valid general pasteboard on the main
            // thread.
            pasteboard.clearContents();
        }
        self.owns_pasteboard = false;
        self.last_change_count = Some(unsafe {
            // SAFETY: `pasteboard` is a valid general pasteboard on the main
            // thread.
            pasteboard.changeCount()
        });
    }

    /// Clears the pasteboard only when our publication is still current.
    fn clear_owned_pasteboard(&mut self) {
        if !self.owns_pasteboard {
            return;
        }

        let pasteboard = unsafe {
            // SAFETY: called on the main thread.
            NSPasteboard::generalPasteboard()
        };
        let current = unsafe {
            // SAFETY: `pasteboard` is a valid general pasteboard on the main
            // thread.
            pasteboard.changeCount()
        };
        if self.last_change_count != Some(current) {
            // Another application changed the pasteboard after our last poll.
            self.owns_pasteboard = false;
            return;
        }

        self.clear_local_pasteboard();
    }

    fn start_lazy_offer(&mut self, files: Vec<FileDescriptor>, clip_data_id: Option<u32>) {
        self.clear_lazy_offer();
        self.remote_queue.clear();
        self.remote_pending = None;
        self.remote_pending_since = None;
        self.local_files.clear();

        let root = match self.next_staging_root() {
            Ok(root) => root,
            Err(error) => {
                warn!(%error, "cannot create a lazy clipboard staging directory");
                return;
            }
        };

        let mut prepared = Vec::with_capacity(files.len());
        let mut roots = Vec::new();
        let mut missing_size = false;

        for (wire_index, descriptor) in files.into_iter().enumerate() {
            let Some(path) = safe_remote_path(&root, &descriptor) else {
                warn!(name = %descriptor.name, path = ?descriptor.relative_path, "skipping unsafe remote clipboard path");
                continue;
            };
            if !is_directory(&descriptor) && descriptor.file_size.is_none() {
                missing_size = true;
            }
            let top_level = descriptor.relative_path.as_deref().is_none_or(|path| path.is_empty());
            if top_level {
                roots.push(path.clone());
            }
            prepared.push((wire_index, descriptor, path));
        }

        if missing_size {
            warn!("remote file list is missing a file size; lazy placeholders need sizes");
            let _ = fs::remove_dir_all(&root);
            return;
        }
        if prepared.is_empty() {
            let _ = fs::remove_dir_all(&root);
            return;
        }
        if roots.is_empty() {
            roots = prepared.iter().map(|(_, _, path)| path.clone()).collect();
        }

        if let Err(error) = create_private_dir_all(&root) {
            warn!(%error, path = %root.display(), "cannot create lazy clipboard root");
            return;
        }

        let mut lazy_files = Vec::with_capacity(prepared.len());
        let mut presenters = Vec::with_capacity(prepared.len());

        for (wire_index, descriptor, path) in prepared {
            if is_directory(&descriptor) {
                if let Err(error) = create_private_dir_all(&path) {
                    warn!(%error, path = %path.display(), "cannot create lazy clipboard directory");
                    remove_presenters(&mut presenters);
                    let _ = fs::remove_dir_all(&root);
                    return;
                }
                lazy_files.push(LazyFile {
                    descriptor,
                    wire_index,
                    path,
                    state: LazyFileState::Complete,
                    waiters: Vec::new(),
                });
                presenters.push(None);
                continue;
            }

            let Some(size) = descriptor.file_size else {
                warn!(name = %descriptor.name, "remote file is missing a size");
                remove_presenters(&mut presenters);
                let _ = fs::remove_dir_all(&root);
                return;
            };

            if let Some(parent) = path.parent() {
                if let Err(error) = create_private_dir_all(parent) {
                    warn!(%error, path = %parent.display(), "cannot create lazy clipboard directory");
                    remove_presenters(&mut presenters);
                    let _ = fs::remove_dir_all(&root);
                    return;
                }
            }

            if let Err(error) = create_sparse_file(&path, size) {
                warn!(%error, path = %path.display(), "cannot create lazy clipboard placeholder");
                remove_presenters(&mut presenters);
                let _ = fs::remove_dir_all(&root);
                return;
            }

            let presenter = match LazyFilePresenter::new(&path) {
                Some(presenter) => presenter,
                None => {
                    warn!(path = %path.display(), "cannot create lazy clipboard file presenter");
                    remove_presenters(&mut presenters);
                    let _ = fs::remove_dir_all(&root);
                    return;
                }
            };
            let presenter = ProtocolObject::from_retained(presenter);
            unsafe {
                // SAFETY: called on the main thread, and the presenter stays
                // alive in `presenters` until it is removed.
                NSFileCoordinator::addFilePresenter(&presenter);
            }
            lazy_files.push(LazyFile {
                descriptor,
                wire_index,
                path,
                state: if size == 0 {
                    LazyFileState::Complete
                } else {
                    LazyFileState::Pending
                },
                waiters: Vec::new(),
            });
            presenters.push(Some(presenter));
        }

        info!(
            count = lazy_files.len(),
            path = %root.display(),
            "published lazy clipboard placeholder(s)"
        );

        self.offer = Some(LazyOffer {
            root,
            files: lazy_files,
            presenters,
            clip_data_id,
            next_stream_id: 1,
        });

        if let Err(error) = self.set_local_files(&roots) {
            warn!(%error, "cannot publish lazy clipboard file URLs");
            self.clear_lazy_offer();
        }
    }

    fn next_staging_root(&self) -> io::Result<PathBuf> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default();
        let pid = std::process::id();
        for suffix in 0..100 {
            let name = if suffix == 0 {
                format!("recv-{pid}-{timestamp}")
            } else {
                format!("recv-{pid}-{timestamp}-{suffix}")
            };
            let root = self.download_root.join(name);
            if !root.exists() {
                return Ok(root);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "cannot allocate a unique clipboard staging directory",
        ))
    }

    fn set_local_files(&mut self, paths: &[PathBuf]) -> Result<(), String> {
        let objects: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = paths
            .iter()
            .filter(|path| path.exists())
            .map(|path| {
                let path = NSString::from_str(&path.to_string_lossy());
                let url = unsafe {
                    // SAFETY: the path is a valid local file path and is only
                    // used on the main thread.
                    NSURL::fileURLWithPath(&path)
                };
                ProtocolObject::from_retained(url)
            })
            .collect();

        if objects.is_empty() {
            return Err("no clipboard files are available".to_owned());
        }

        let pasteboard = unsafe {
            // SAFETY: called on the main thread.
            NSPasteboard::generalPasteboard()
        };
        unsafe {
            // SAFETY: `pasteboard` is a valid general pasteboard on the main
            // thread.
            pasteboard.clearContents();
        }
        self.owns_pasteboard = false;
        let objects = NSArray::from_vec(objects);
        let written = unsafe {
            // SAFETY: every object is a file URL implementing
            // `NSPasteboardWriting`, and the array is valid for the call.
            pasteboard.writeObjects(&objects)
        };
        if !written {
            return Err("the macOS pasteboard rejected the file URLs".to_owned());
        }
        self.owns_pasteboard = true;
        self.last_change_count = Some(unsafe {
            // SAFETY: called on the main thread.
            pasteboard.changeCount()
        });
        Ok(())
    }

    fn request_lazy_read(&mut self, path: PathBuf, reply: LazyReadReply) {
        let Some(offer) = self.offer.as_mut() else {
            let _ = reply.send(Err("lazy clipboard offer is gone".to_owned()));
            return;
        };
        let Some(index) = offer.files.iter().position(|file| file.path == path) else {
            let _ = reply.send(Err("file is not part of the lazy clipboard offer".to_owned()));
            return;
        };

        match offer.files[index].state {
            LazyFileState::Complete => {
                let _ = reply.send(Ok(()));
            }
            LazyFileState::Failed => {
                let _ = reply.send(Err("lazy file transfer already failed".to_owned()));
            }
            LazyFileState::Downloading => {
                offer.files[index].waiters.push(reply);
            }
            LazyFileState::Pending => {
                offer.files[index].waiters.push(reply);
                self.queue.push_back(index);
                self.advance_lazy_download();
            }
        }
    }

    fn advance_lazy_download(&mut self) {
        if self.download.is_some() {
            return;
        }

        loop {
            let Some(index) = self.queue.pop_front() else {
                return;
            };
            let Some(offer) = self.offer.as_ref() else {
                return;
            };
            let Some(file) = offer.files.get(index) else {
                continue;
            };
            if file.state != LazyFileState::Pending {
                continue;
            }

            let Some(size) = file.descriptor.file_size else {
                self.fail_lazy_file(index, "file size is unavailable");
                continue;
            };
            let Ok(wire_index) = i32::try_from(file.wire_index) else {
                self.fail_lazy_file(index, "remote file index is too large");
                continue;
            };
            let clip_data_id = offer.clip_data_id;
            let stream_id = offer.next_stream_id;
            let path = file.path.clone();
            let name = file.descriptor.name.clone();

            let handle = match open_existing_private_file(&path, size) {
                Ok(handle) => handle,
                Err(error) => {
                    self.fail_lazy_file(index, &format!("cannot open placeholder: {error}"));
                    continue;
                }
            };

            let Some(offer) = self.offer.as_mut() else {
                return;
            };
            offer.next_stream_id = offer.next_stream_id.wrapping_add(1).max(1);
            offer.files[index].state = LazyFileState::Downloading;

            info!(name = %name, size, "downloading lazy clipboard file");
            self.download = Some(LazyDownload {
                file_index: index,
                stream_id,
                position: 0,
                size,
                file: handle,
                last_activity: Instant::now(),
                next_progress: progress_interval(size),
            });
            self.send_file_request(wire_index, stream_id, 0, size, clip_data_id);
            return;
        }
    }

    fn receive_lazy_contents(&mut self, stream_id: u32, data: &[u8], is_error: bool) {
        enum Action {
            Fail(String),
            Complete { file_index: usize, size: u64 },
            Continue { file_index: usize },
        }

        let action = {
            let Some(active) = self.download.as_mut() else {
                debug!(stream_id, "ignoring an unsolicited file-contents response");
                return;
            };
            if active.stream_id != stream_id {
                debug!(
                    stream_id,
                    expected = active.stream_id,
                    "ignoring a stale file-contents response"
                );
                return;
            }

            active.last_activity = Instant::now();

            if is_error {
                Action::Fail("remote file request failed".to_owned())
            } else {
                let data_len = u64::try_from(data.len()).unwrap_or(u64::MAX);
                if data.is_empty() || active.position.saturating_add(data_len) > active.size {
                    Action::Fail("remote file response has an invalid length".to_owned())
                } else if let Err(error) = active.file.write_all(data) {
                    Action::Fail(format!("cannot write placeholder: {error}"))
                } else {
                    active.position += data_len;

                    if active.position >= active.size {
                        if let Err(error) = active.file.flush() {
                            Action::Fail(format!("cannot flush placeholder: {error}"))
                        } else {
                            Action::Complete {
                                file_index: active.file_index,
                                size: active.size,
                            }
                        }
                    } else {
                        if active.position >= active.next_progress {
                            let percent = active.position.saturating_mul(100) / active.size;
                            info!(
                                file = %self
                                    .offer
                                    .as_ref()
                                    .and_then(|offer| offer.files.get(active.file_index))
                                    .map(|file| file.descriptor.name.as_str())
                                    .unwrap_or("<unknown>"),
                                percent,
                                position = active.position,
                                size = active.size,
                                "lazy clipboard progress"
                            );
                            active.next_progress = active
                                .position
                                .saturating_add(progress_interval(active.size))
                                .min(active.size);
                        }
                        Action::Continue {
                            file_index: active.file_index,
                        }
                    }
                }
            }
        };

        match action {
            Action::Fail(reason) => self.fail_lazy_download(&reason),
            Action::Complete { file_index, size } => {
                let name = self
                    .offer
                    .as_ref()
                    .and_then(|offer| offer.files.get(file_index))
                    .map(|file| file.descriptor.name.clone())
                    .unwrap_or_else(|| "<unknown>".to_owned());
                self.download = None;
                self.complete_lazy_file(file_index);
                info!(name = %name, size, "downloaded lazy clipboard file");
                self.advance_lazy_download();
            }
            Action::Continue { file_index } => {
                let Some(offer) = self.offer.as_ref() else {
                    return;
                };
                let Some(file) = offer.files.get(file_index) else {
                    return;
                };
                let Ok(wire_index) = i32::try_from(file.wire_index) else {
                    self.fail_lazy_download("remote file index is too large");
                    return;
                };
                let Some(active) = self.download.as_ref() else {
                    return;
                };
                let position = active.position;
                let remaining = active.size - position;
                let stream_id = active.stream_id;
                let clip_data_id = offer.clip_data_id;
                self.send_file_request(wire_index, stream_id, position, remaining, clip_data_id);
            }
        }
    }

    fn send_file_request(
        &self,
        wire_index: i32,
        stream_id: u32,
        position: u64,
        remaining: u64,
        clip_data_id: Option<u32>,
    ) {
        let requested_size = u32::try_from(remaining.min(u64::from(MAX_FILE_CHUNK))).unwrap_or(MAX_FILE_CHUNK);
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendFileContentsRequest(FileContentsRequest {
                stream_id,
                index: wire_index,
                flags: FileContentsFlags::RANGE,
                position,
                requested_size,
                data_id: clip_data_id,
            }));
    }

    fn complete_lazy_file(&mut self, file_index: usize) {
        let Some(offer) = self.offer.as_mut() else {
            return;
        };
        let Some(file) = offer.files.get_mut(file_index) else {
            return;
        };
        file.state = LazyFileState::Complete;
        for waiter in file.waiters.drain(..) {
            let _ = waiter.send(Ok(()));
        }
    }

    fn fail_lazy_download(&mut self, reason: &str) {
        let Some(active) = self.download.take() else {
            return;
        };
        self.fail_lazy_file(active.file_index, reason);
    }

    fn fail_lazy_file(&mut self, file_index: usize, reason: &str) {
        warn!(%reason, "lazy clipboard file failed");
        let Some(offer) = self.offer.as_mut() else {
            return;
        };
        let Some(file) = offer.files.get_mut(file_index) else {
            return;
        };
        file.state = LazyFileState::Failed;
        let path = file.path.clone();
        for waiter in file.waiters.drain(..) {
            let _ = waiter.send(Err(reason.to_owned()));
        }
        // Remove the placeholder before unregistering its presenter: a reader
        // that starts in between must fail, not read a sparse incomplete file.
        let _ = fs::remove_file(&path);
        if let Some(presenter) = offer.presenters.get_mut(file_index).and_then(Option::take) {
            unsafe {
                // SAFETY: presenters are removed on the main thread, and this
                // presenter is no longer registered.
                NSFileCoordinator::removeFilePresenter(&presenter);
            }
        }
        self.advance_lazy_download();
    }

    fn expire_download(&mut self) {
        let Some(active) = self.download.as_ref() else {
            return;
        };
        if active.last_activity.elapsed() < LAZY_READ_TIMEOUT {
            return;
        }
        self.fail_lazy_download("lazy clipboard file transfer timed out");
    }

    fn clear_lazy_offer(&mut self) {
        self.retire_lazy_offer(false);
    }

    fn retire_lazy_offer(&mut self, drain_events: bool) {
        self.queue.clear();
        self.download = None;

        let Some(mut offer) = self.offer.take() else {
            if drain_events {
                self.abort_pending_events();
            }
            return;
        };

        // Wake Finder reads before unregistering their presenters. A blocked
        // reader is waiting for this reply, and removing its presenter first
        // would make the reader and the cleanup wait on each other.
        for file in &mut offer.files {
            for waiter in file.waiters.drain(..) {
                let _ = waiter.send(Err("lazy clipboard offer changed".to_owned()));
            }
            // Incomplete placeholders must disappear before their presenter is
            // removed, otherwise a newly started read could copy sparse zeroes.
            if file.state != LazyFileState::Complete {
                if let Err(error) = fs::remove_file(&file.path) {
                    if error.kind() != io::ErrorKind::NotFound {
                        warn!(%error, path = %file.path.display(), "cannot remove an incomplete clipboard placeholder");
                    }
                }
            }
        }
        if drain_events {
            // A Finder read may already be queued in the event channel but not
            // yet turned into a waiter. Release it before removing presenters.
            self.abort_pending_events();
        }
        remove_presenters(&mut offer.presenters);

        let paths = offer.files.into_iter().map(|file| file.path).collect();
        info!(path = %offer.root.display(), "retiring lazy clipboard staging tree");
        cleanup_queue().enqueue(paths, offer.root);
    }

    fn abort_pending_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            if let BackendEvent::LazyRead { reply, .. } = event {
                let _ = reply.send(Err("clipboard session closed".to_owned()));
            }
        }
    }
}

impl Drop for MacClipboard {
    fn drop(&mut self) {
        if MainThreadMarker::new().is_some() {
            self.clear_owned_pasteboard();
            self.retire_lazy_offer(true);
            if !cleanup_queue().wait_for_idle(EXIT_CLEANUP_TIMEOUT) {
                warn!(
                    timeout = ?EXIT_CLEANUP_TIMEOUT,
                    "timed out waiting for macOS clipboard staging cleanup"
                );
            }
        } else {
            warn!("MacClipboard dropped off the main thread; skipping clipboard cleanup");
        }

        if let Ok(mut events_slot) = LAZY_EVENTS.lock() {
            if events_slot
                .as_ref()
                .is_some_and(|(session_id, _)| *session_id == self.session_id)
            {
                *events_slot = None;
            }
        }
    }
}

/// Forwards `CLIPRDR` callbacks from the session thread to [`MacClipboard`].
#[derive(Debug)]
pub struct MacCliprdrBackend {
    events: Sender<BackendEvent>,
    temporary_directory: String,
}

impl_as_any!(MacCliprdrBackend);

impl MacCliprdrBackend {
    fn send(&self, event: BackendEvent) {
        if self.events.send(event).is_err() {
            warn!("macOS clipboard event loop is gone; dropping a CLIPRDR event");
        }
    }
}

impl CliprdrBackend for MacCliprdrBackend {
    fn temporary_directory(&self) -> &str {
        &self.temporary_directory
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
            | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
            | ClipboardGeneralCapabilityFlags::HUGE_FILE_SUPPORT_ENABLED
    }

    fn on_ready(&mut self) {
        self.send(BackendEvent::Ready);
    }

    fn on_request_format_list(&mut self) {
        self.send(BackendEvent::RequestFormatList);
    }

    fn on_process_negotiated_capabilities(&mut self, capabilities: ClipboardGeneralCapabilityFlags) {
        self.send(BackendEvent::NegotiatedCapabilities(capabilities));
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        self.send(BackendEvent::RemoteCopy(available_formats.to_vec()));
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.send(BackendEvent::FormatDataRequest(request));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        let too_large = MAX_TRANSFER_BYTES < response.data().len();
        self.send(BackendEvent::FormatDataResponse {
            data: if too_large {
                Vec::new()
            } else {
                response.data().to_vec()
            },
            is_error: response.is_error() || too_large,
        });
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        self.send(BackendEvent::FileContentsRequest(request));
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        let data = response.data();
        let too_large = data.len() > usize::try_from(MAX_FILE_CHUNK).unwrap_or(usize::MAX);
        self.send(BackendEvent::FileContentsResponse {
            stream_id: response.stream_id(),
            data: if too_large { Vec::new() } else { data.to_vec() },
            is_error: response.is_error() || too_large,
        });
    }

    fn on_lock(&mut self, data_id: LockDataId) {
        self.send(BackendEvent::Lock(data_id));
    }

    fn on_unlock(&mut self, data_id: LockDataId) {
        self.send(BackendEvent::Unlock(data_id));
    }

    fn on_remote_file_list(&mut self, files: &[FileDescriptor], clip_data_id: Option<u32>) {
        self.send(BackendEvent::RemoteFileList {
            files: files.to_vec(),
            clip_data_id,
        });
    }

    fn now_ms(&self) -> u64 {
        crate::native_now_ms()
    }
}

#[derive(Debug)]
struct MacCliprdrBackendFactory {
    events: Sender<BackendEvent>,
    temporary_directory: String,
}

impl CliprdrBackendFactory for MacCliprdrBackendFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(MacCliprdrBackend {
            events: self.events.clone(),
            temporary_directory: self.temporary_directory.clone(),
        })
    }
}

#[derive(Debug)]
enum BackendEvent {
    Ready,
    RequestFormatList,
    NegotiatedCapabilities(ClipboardGeneralCapabilityFlags),
    RemoteCopy(Vec<ClipboardFormat>),
    FormatDataRequest(FormatDataRequest),
    FormatDataResponse {
        data: Vec<u8>,
        is_error: bool,
    },
    FileContentsRequest(FileContentsRequest),
    Lock(LockDataId),
    Unlock(LockDataId),
    RemoteFileList {
        files: Vec<FileDescriptor>,
        clip_data_id: Option<u32>,
    },
    FileContentsResponse {
        stream_id: u32,
        data: Vec<u8>,
        is_error: bool,
    },
    LazyRead {
        path: PathBuf,
        reply: LazyReadReply,
    },
}

#[derive(Debug, Clone)]
struct LocalFile {
    descriptor: FileDescriptor,
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteFormat {
    Text,
    Dib,
    DibV5,
}

impl RemoteFormat {
    fn id(self) -> ClipboardFormatId {
        match self {
            Self::Text => ClipboardFormatId::CF_UNICODETEXT,
            Self::Dib => ClipboardFormatId::CF_DIB,
            Self::DibV5 => ClipboardFormatId::CF_DIBV5,
        }
    }
}

#[derive(Debug)]
struct LazyOffer {
    root: PathBuf,
    files: Vec<LazyFile>,
    presenters: Vec<Option<Retained<ProtocolObject<dyn NSFilePresenter>>>>,
    clip_data_id: Option<u32>,
    next_stream_id: u32,
}

#[derive(Debug)]
struct LazyFile {
    descriptor: FileDescriptor,
    wire_index: usize,
    path: PathBuf,
    state: LazyFileState,
    waiters: Vec<LazyReadReply>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LazyFileState {
    Pending,
    Downloading,
    Complete,
    Failed,
}

#[derive(Debug)]
struct LazyDownload {
    file_index: usize,
    stream_id: u32,
    position: u64,
    size: u64,
    file: File,
    last_activity: Instant,
    next_progress: u64,
}

struct LazyPresenterIvars {
    path: PathBuf,
    url: Retained<NSURL>,
    queue: Retained<NSOperationQueue>,
}

declare_class!(
    struct LazyFilePresenter;

    // SAFETY: The ivars are owned by the class and only accessed on the
    // operation queue or from the owning `MacClipboard`.
    unsafe impl ClassType for LazyFilePresenter {
        type Super = NSObject;
        type Mutability = InteriorMutable;
        const NAME: &'static str = "IronRdpLazyFilePresenter";
    }

    impl DeclaredClass for LazyFilePresenter {
        type Ivars = LazyPresenterIvars;
    }

    unsafe impl LazyFilePresenter {
        #[method_id(initWithPath:)]
        fn init_with_path(this: Allocated<Self>, path: &NSString) -> Option<Retained<Self>> {
            let path_buf = PathBuf::from(path.to_string());
            let url = unsafe {
                // SAFETY: `path` is a valid local file path.
                NSURL::fileURLWithPath(path)
            };
            let queue = unsafe {
                // SAFETY: a new operation queue is returned retained.
                NSOperationQueue::new()
            };
            unsafe {
                // SAFETY: `queue` is a valid operation queue owned by this
                // object; one operation at a time keeps Finder's reads
                // serialized per file.
                queue.setMaxConcurrentOperationCount(1);
            }
            let this = this.set_ivars(LazyPresenterIvars {
                path: path_buf,
                url,
                queue,
            });
            unsafe {
                // SAFETY: `this` is an allocated instance of this class.
                msg_send_id![super(this), init]
            }
        }
    }

    // SAFETY: `NSObjectProtocol` has no required methods.
    unsafe impl NSObjectProtocol for LazyFilePresenter {}

    // SAFETY: `NSFilePresenter` is implemented on the operation queue; the
    // reader waits for the main-thread download before releasing Finder.
    unsafe impl NSFilePresenter for LazyFilePresenter {
        #[method_id(presentedItemURL)]
        fn presented_item_url(&self) -> Option<Retained<NSURL>> {
            Some(self.ivars().url.clone())
        }

        #[method_id(presentedItemOperationQueue)]
        fn presented_item_operation_queue(&self) -> Retained<NSOperationQueue> {
            self.ivars().queue.clone()
        }

        #[method(relinquishPresentedItemToReader:)]
        fn relinquish_to_reader(&self, reader: &Block<dyn Fn(*mut Block<dyn Fn()>)>) {
            let path = self.ivars().path.clone();

            let result = catch_unwind(AssertUnwindSafe(|| {
                let events = LAZY_EVENTS
                    .lock()
                    .map_err(|_| "lazy clipboard event slot is poisoned".to_owned())?
                    .as_ref()
                    .map(|(_, events)| events.clone())
                    .ok_or_else(|| "lazy clipboard event loop is unavailable".to_owned())?;
                let (reply, wait) = mpsc::channel();
                events
                    .send(BackendEvent::LazyRead { path, reply })
                    .map_err(|_| "lazy clipboard event loop is closed".to_owned())?;
                wait.recv()
                    .map_err(|_| "lazy clipboard read wait failed".to_owned())?
            }));

            match result {
                Ok(Err(error)) => warn!(%error, "lazy clipboard read failed"),
                Err(_) => warn!("lazy clipboard reader panicked"),
                Ok(Ok(())) => {}
            }

            let _ = catch_unwind(AssertUnwindSafe(|| {
                reader.call((ptr::null_mut(),));
            }));
        }
    }
);

impl LazyFilePresenter {
    fn new(path: &Path) -> Option<Retained<Self>> {
        let path = NSString::from_str(&path.to_string_lossy());
        let allocated = Self::alloc();
        unsafe {
            // SAFETY: `allocated` is an uninitialized instance of this class,
            // and the argument type matches `initWithPath:`.
            msg_send_id![allocated, initWithPath: &*path]
        }
    }
}

fn remove_presenters(presenters: &mut [Option<Retained<ProtocolObject<dyn NSFilePresenter>>>]) {
    for presenter in presenters.iter_mut().filter_map(Option::take) {
        unsafe {
            // SAFETY: called on the main thread while the presenter is still
            // retained by `presenter`.
            NSFileCoordinator::removeFilePresenter(&presenter);
        }
    }
}

fn remote_formats_to_request(formats: &[ClipboardFormat]) -> Vec<RemoteFormat> {
    let has = |id| formats.iter().any(|format| format.id() == id);
    let mut requested = Vec::new();
    if has(ClipboardFormatId::CF_UNICODETEXT) {
        requested.push(RemoteFormat::Text);
    }
    if has(ClipboardFormatId::CF_DIBV5) {
        requested.push(RemoteFormat::DibV5);
    } else if has(ClipboardFormatId::CF_DIB) {
        requested.push(RemoteFormat::Dib);
    }
    requested
}

fn encode_unicode_text(text: &str) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\r\n");
    let mut units: Vec<u16> = normalized.encode_utf16().collect();
    units.push(0);
    let mut bytes = Vec::with_capacity(units.len() * 2);
    for unit in units {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

fn decode_unicode_text(bytes: &[u8]) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut units: Vec<u16> = bytes
        .chunks(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    if units.first() == Some(&0xFEFF) {
        units.remove(0);
    }
    if units.last() == Some(&0) {
        units.pop();
    }
    let text = String::from_utf16_lossy(&units);
    Some(text.replace("\r\n", "\n").replace('\r', "\n"))
}

fn read_local_text() -> Option<String> {
    let pasteboard = unsafe {
        // SAFETY: clipboard access happens on the main thread.
        NSPasteboard::generalPasteboard()
    };
    if let Some(text) = unsafe {
        // SAFETY: `pasteboard` is a valid general pasteboard and the type is
        // a standard pasteboard text type.
        pasteboard.stringForType(NSPasteboardTypeString)
    } {
        return Some(text.to_string());
    }

    for type_name in [UTF16_EXTERNAL_PLAIN_TEXT, UTF16_PLAIN_TEXT] {
        let type_name = NSString::from_str(type_name);
        if let Some(data) = unsafe {
            // SAFETY: `pasteboard` and `type_name` are valid for the call.
            pasteboard.dataForType(&type_name)
        } {
            if let Some(text) = decode_utf16_pasteboard(data.bytes()) {
                return Some(text);
            }
        }
    }
    None
}

fn decode_utf16_pasteboard(bytes: &[u8]) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let (big_endian, bytes) = if bytes.starts_with(&[0xFE, 0xFF]) {
        (true, &bytes[2..])
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        (false, &bytes[2..])
    } else {
        (false, bytes)
    };
    let mut units = bytes
        .chunks_exact(2)
        .map(|chunk| {
            if big_endian {
                u16::from_be_bytes([chunk[0], chunk[1]])
            } else {
                u16::from_le_bytes([chunk[0], chunk[1]])
            }
        })
        .collect::<Vec<_>>();
    if units.last() == Some(&0) {
        units.pop();
    }
    Some(String::from_utf16_lossy(&units))
}

fn local_image_available() -> bool {
    let pasteboard = unsafe {
        // SAFETY: clipboard access happens on the main thread.
        NSPasteboard::generalPasteboard()
    };
    let png = unsafe {
        // SAFETY: `pasteboard` is valid and the type is a standard image type.
        pasteboard.dataForType(NSPasteboardTypePNG)
    };
    let tiff = unsafe {
        // SAFETY: `pasteboard` is valid and the type is a standard image type.
        pasteboard.dataForType(NSPasteboardTypeTIFF)
    };
    png.is_some() || tiff.is_some()
}

fn read_local_png() -> Option<Vec<u8>> {
    let pasteboard = unsafe {
        // SAFETY: clipboard access happens on the main thread.
        NSPasteboard::generalPasteboard()
    };
    if let Some(data) = unsafe {
        // SAFETY: `pasteboard` is valid and the type is a standard image type.
        pasteboard.dataForType(NSPasteboardTypePNG)
    } {
        if data.length() <= MAX_TRANSFER_BYTES {
            return Some(data.bytes().to_vec());
        }
        return None;
    }

    let tiff = unsafe {
        // SAFETY: `pasteboard` is valid and the type is a standard image type.
        pasteboard.dataForType(NSPasteboardTypeTIFF)
    }?;
    if MAX_TRANSFER_BYTES < tiff.length() {
        return None;
    }
    let image = NSImage::initWithData(NSImage::alloc(), &tiff)?;
    let tiff = unsafe {
        // SAFETY: `image` is a valid image created from the pasteboard data.
        image.TIFFRepresentation()
    }?;
    let representation = unsafe {
        // SAFETY: the TIFF data came from a valid `NSImage`.
        NSBitmapImageRep::imageRepWithData(&tiff)
    }?;
    let properties = NSDictionary::new();
    let png = unsafe {
        // SAFETY: `representation` is valid, and an empty properties
        // dictionary is accepted for PNG output.
        representation.representationUsingType_properties(NSPNGFileType, &properties)
    }?;
    if MAX_TRANSFER_BYTES < png.length() {
        return None;
    }
    Some(png.bytes().to_vec())
}

#[expect(
    clippy::as_conversions,
    clippy::multiple_unsafe_ops_per_block,
    reason = "Objective-C class and object pointers need one audited FFI conversion block"
)]
fn read_local_files() -> Vec<PathBuf> {
    unsafe {
        // SAFETY: clipboard access happens on the main thread, and the URL
        // class is a valid Objective-C class.
        let pasteboard = NSPasteboard::generalPasteboard();
        let url_class = {
            let class: *const AnyClass = NSURL::class();
            let class = class as *mut AnyObject;
            let Some(class) = Retained::retain(class) else {
                return Vec::new();
            };
            class
        };
        let classes = NSArray::from_vec(vec![url_class]);
        let Some(objects) = pasteboard.readObjectsForClasses_options(&classes, None) else {
            return Vec::new();
        };

        let mut paths = Vec::new();
        for object in objects.iter() {
            // SAFETY: the pasteboard was queried for `NSURL` objects only.
            let url = &*(object as *const AnyObject as *const NSURL);
            if url.isFileURL() {
                if let Some(path) = url.path() {
                    paths.push(PathBuf::from(path.to_string()));
                }
            }
        }
        paths
    }
}

fn build_local_file_list(roots: &[PathBuf]) -> Vec<LocalFile> {
    let mut entries = Vec::new();
    for root in roots {
        let Some(name) = root.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        collect_local_file(root, name, None, &mut entries);
    }
    entries
}

fn collect_local_file(path: &Path, name: &str, relative_path: Option<&str>, entries: &mut Vec<LocalFile>) {
    if entries.len() >= MAX_LOCAL_FILES {
        return;
    }
    if !is_safe_component(name) {
        return;
    }
    if let Some(relative_path) = relative_path {
        if !relative_path.is_empty() && !is_safe_relative_wire_path(relative_path) {
            return;
        }
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        return;
    }

    let attributes = if metadata.is_dir() {
        ClipboardFileAttributes::DIRECTORY
    } else {
        ClipboardFileAttributes::NORMAL
    };
    let mut descriptor = FileDescriptor::new(name).with_attributes(attributes);
    if !metadata.is_dir() {
        descriptor = descriptor.with_file_size(metadata.len());
    }
    if let Some(relative_path) = relative_path {
        descriptor = descriptor.with_relative_path(relative_path);
    }
    if !is_sendable_descriptor(&descriptor) {
        return;
    }

    entries.push(LocalFile {
        descriptor,
        path: path.to_path_buf(),
    });

    if !metadata.is_dir() {
        return;
    }

    let children_relative = match relative_path {
        Some(relative_path) => format!("{relative_path}\\{name}"),
        None => name.to_owned(),
    };
    let Ok(read_dir) = fs::read_dir(path) else {
        return;
    };
    let mut children = read_dir.filter_map(Result::ok).collect::<Vec<_>>();
    children.sort_by_key(fs::DirEntry::file_name);

    for child in children {
        let Ok(file_type) = child.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let file_name = child.file_name();
        let Some(child_name) = file_name.to_str() else {
            continue;
        };
        collect_local_file(&child.path(), child_name, Some(&children_relative), entries);
    }
}

fn is_sendable_descriptor(descriptor: &FileDescriptor) -> bool {
    if !is_safe_component(&descriptor.name) {
        return false;
    }
    if let Some(relative_path) = descriptor.relative_path.as_deref() {
        if !relative_path.is_empty() && !is_safe_relative_wire_path(relative_path) {
            return false;
        }
    }
    let wire_len = match descriptor.relative_path.as_deref() {
        Some(path) if !path.is_empty() => path.encode_utf16().count() + 1 + descriptor.name.encode_utf16().count(),
        _ => descriptor.name.encode_utf16().count(),
    };
    let first_component = descriptor
        .relative_path
        .as_deref()
        .filter(|path| !path.is_empty())
        .unwrap_or(&descriptor.name);
    wire_len <= 259 && !is_absolute_wire_name(first_component)
}

fn is_safe_relative_wire_path(path: &str) -> bool {
    !path.is_empty() && path.split(['\\', '/']).all(is_safe_component)
}

fn is_absolute_wire_name(name: &str) -> bool {
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some('/' | '\\'), _) => true,
        (Some(drive), Some(':')) => drive.is_ascii_alphabetic(),
        _ => false,
    }
}

fn read_local_file_contents(request: &FileContentsRequest, entry: &LocalFile) -> FileContentsResponse<'static> {
    if is_directory(&entry.descriptor) {
        return FileContentsResponse::new_error(request.stream_id).into_owned();
    }

    if request.flags.contains(FileContentsFlags::SIZE) {
        return match fs::metadata(&entry.path) {
            Ok(metadata) => FileContentsResponse::new_size_response(request.stream_id, metadata.len()).into_owned(),
            Err(error) => {
                warn!(path = %entry.path.display(), %error, "cannot read clipboard file size");
                FileContentsResponse::new_error(request.stream_id).into_owned()
            }
        };
    }

    if !request.flags.contains(FileContentsFlags::RANGE) || request.requested_size == 0 {
        return FileContentsResponse::new_error(request.stream_id).into_owned();
    }

    let read_len = u64::from(request.requested_size).min(u64::from(MAX_FILE_CHUNK));
    let result = File::open(&entry.path).and_then(|mut file| {
        file.seek(SeekFrom::Start(request.position))?;
        let mut data = Vec::new();
        file.take(read_len).read_to_end(&mut data)?;
        Ok(data)
    });

    match result {
        Ok(data) => FileContentsResponse::new_data_response(request.stream_id, data).into_owned(),
        Err(error) => {
            warn!(path = %entry.path.display(), %error, "cannot read clipboard file");
            FileContentsResponse::new_error(request.stream_id).into_owned()
        }
    }
}

fn is_file_list_format(format: &ClipboardFormat) -> bool {
    format
        .name()
        .is_some_and(|name| name.value() == ClipboardFormatName::FILE_LIST.value())
}

fn is_directory(descriptor: &FileDescriptor) -> bool {
    descriptor
        .attributes
        .is_some_and(|attributes| attributes.contains(ClipboardFileAttributes::DIRECTORY))
}

fn safe_remote_path(root: &Path, descriptor: &FileDescriptor) -> Option<PathBuf> {
    let mut path = root.to_path_buf();

    if let Some(relative_path) = descriptor.relative_path.as_deref() {
        if !relative_path.is_empty() {
            for component in relative_path.split('\\') {
                if !is_safe_component(component) {
                    return None;
                }
                path.push(component);
            }
        }
    }

    if !is_safe_component(&descriptor.name) {
        return None;
    }
    path.push(&descriptor.name);

    path.starts_with(root).then_some(path)
}

fn is_safe_component(component: &str) -> bool {
    !component.is_empty()
        && component != "."
        && component != ".."
        && !component.contains('/')
        && !component.contains('\\')
        && !component.contains('\0')
        && !component.contains(':')
}

fn progress_interval(size: u64) -> u64 {
    (size / 10).max(PROGRESS_CHUNK)
}

fn clipboard_root() -> PathBuf {
    std::env::temp_dir().join("ironrdp-cliprdr")
}

fn create_private_dir_all(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};

    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

fn open_private_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

fn open_existing_private_file(path: &Path, size: u64) -> io::Result<File> {
    use std::os::unix::fs::PermissionsExt as _;

    let file = OpenOptions::new().write(true).open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    // Keep the placeholder sparse. `set_len` is a no-op when the size already
    // matches, and it restores the expected size if a previous attempt was
    // interrupted.
    file.set_len(size)?;
    Ok(file)
}

fn create_sparse_file(path: &Path, size: u64) -> io::Result<()> {
    let file = open_private_file(path)?;
    file.set_len(size)
}

/// Serialized background cleanup for retired lazy clipboard offers.
///
/// `NSFileCoordinator` is safe to use off the main thread, but every delete
/// must be coordinated so an active Finder copy can finish first. A single
/// worker keeps Foundation calls serialized and lets `MacClipboard::drop`
/// wait for all outstanding cleanup.
struct CleanupQueue {
    sender: Option<Sender<CleanupJob>>,
    pending: Arc<(Mutex<usize>, Condvar)>,
}

struct CleanupJob {
    paths: Vec<PathBuf>,
    root: PathBuf,
    /// Decrements the queue's pending count when this job is dropped.
    _pending: PendingGuard,
}

struct PendingGuard {
    pending: Arc<(Mutex<usize>, Condvar)>,
}

impl PendingGuard {
    fn new(pending: Arc<(Mutex<usize>, Condvar)>) -> Self {
        let (lock, _) = &*pending;
        *lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) += 1;
        Self { pending }
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        let (lock, condvar) = &*self.pending;
        let mut pending = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *pending = pending.saturating_sub(1);
        if *pending == 0 {
            condvar.notify_all();
        }
    }
}

impl CleanupQueue {
    fn start() -> Self {
        let pending = Arc::new((Mutex::new(0), Condvar::new()));
        let (sender, receiver) = mpsc::channel::<CleanupJob>();
        let spawned = thread::Builder::new()
            .name("ironrdp-clipboard-cleanup".to_owned())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    let _ = catch_unwind(AssertUnwindSafe(|| cleanup_job(&job)));
                }
            });
        let sender = match spawned {
            Ok(_) => Some(sender),
            Err(error) => {
                warn!(%error, "cannot start the macOS clipboard cleanup thread; deleting synchronously");
                None
            }
        };
        Self { sender, pending }
    }

    fn enqueue(&self, paths: Vec<PathBuf>, root: PathBuf) {
        let job = CleanupJob {
            paths,
            root,
            _pending: PendingGuard::new(Arc::clone(&self.pending)),
        };

        match &self.sender {
            Some(sender) => {
                if let Err(error) = sender.send(job) {
                    let job = error.0;
                    let _ = catch_unwind(AssertUnwindSafe(|| cleanup_job(&job)));
                }
            }
            None => {
                let _ = catch_unwind(AssertUnwindSafe(|| cleanup_job(&job)));
            }
        }
    }

    fn wait_for_idle(&self, timeout: Duration) -> bool {
        let (lock, condvar) = &*self.pending;
        let guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (guard, _) = condvar
            .wait_timeout_while(guard, timeout, |pending| *pending > 0)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard == 0
    }
}

fn cleanup_queue() -> &'static CleanupQueue {
    static QUEUE: OnceLock<CleanupQueue> = OnceLock::new();
    QUEUE.get_or_init(CleanupQueue::start)
}

fn cleanup_job(job: &CleanupJob) {
    let mut paths = job.paths.clone();
    // Delete children before their parent directory.
    paths.sort_by_key(|path| core::cmp::Reverse(path.components().count()));
    paths.dedup();

    let mut all_removed = true;
    for path in &paths {
        if fs::symlink_metadata(path).is_err() {
            continue;
        }
        if !coordinate_remove(path) {
            all_removed = false;
        }
    }

    if all_removed {
        if let Err(error) = fs::remove_dir_all(&job.root) {
            if error.kind() != io::ErrorKind::NotFound {
                warn!(%error, path = %job.root.display(), "cannot remove the clipboard staging tree");
            }
        }
    } else {
        warn!(path = %job.root.display(), "leaving the clipboard staging tree for startup cleanup");
    }
}

fn coordinate_remove(path: &Path) -> bool {
    let is_dir = fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_dir())
        .unwrap_or(false);

    autoreleasepool(|_| {
        let path_string = NSString::from_str(&path.to_string_lossy());
        let url = unsafe {
            // SAFETY: `path_string` is a valid local path.
            NSURL::fileURLWithPath(&path_string)
        };
        let coordinator = unsafe {
            // SAFETY: a new coordinator is returned retained and has no
            // registered presenter of its own.
            NSFileCoordinator::initWithFilePresenter(NSFileCoordinator::alloc(), None)
        };
        let mut error: Option<Retained<NSError>> = None;
        let removed = Cell::new(false);
        let block = StackBlock::new(|_url: NonNull<NSURL>| {
            let result = if is_dir {
                fs::remove_dir_all(path)
            } else {
                fs::remove_file(path)
            };
            match result {
                Ok(()) => removed.set(true),
                Err(error) => {
                    warn!(%error, path = %path.display(), "cannot remove a clipboard staging path");
                }
            }
        });
        unsafe {
            // SAFETY: `coordinator` and `url` are valid, and `block` lives for
            // the duration of the synchronous coordination call.
            coordinator.coordinateWritingItemAtURL_options_error_byAccessor(
                &url,
                NSFileCoordinatorWritingOptions::NSFileCoordinatorWritingForDeleting,
                Some(&mut error),
                &block,
            );
        }
        if let Some(error) = error {
            warn!(
                code = error.code(),
                path = %path.display(),
                "coordinated clipboard delete failed"
            );
            false
        } else {
            removed.get()
        }
    })
}

fn cleanup_old_staging(root: &Path, max_age: Duration) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("recv-") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let Ok(age) = now.duration_since(modified) else {
            continue;
        };
        if max_age <= age {
            debug!(path = %path.display(), "removing old clipboard staging directory");
            let _ = fs::remove_dir_all(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_traversal_and_separators() {
        let root = Path::new("/tmp/root");
        for (relative, name) in [
            (None, "../escape"),
            (Some(".."), "file"),
            (Some("a/../../b"), "file"),
            (Some("a\\b"), "file"),
            (None, "a/b"),
            (None, "a\\b"),
            (None, "a:b"),
        ] {
            let mut descriptor = FileDescriptor::new(name).with_file_size(0);
            if let Some(relative) = relative {
                descriptor = descriptor.with_relative_path(relative);
            }
            assert!(
                safe_remote_path(root, &descriptor).is_none(),
                "accepted {relative:?} {name:?}"
            );
        }
    }

    #[test]
    fn preserves_nested_paths() {
        let root = Path::new("/tmp/root");
        let descriptor = FileDescriptor::new("file.txt")
            .with_file_size(1)
            .with_relative_path("nested\\dir");
        assert_eq!(
            safe_remote_path(root, &descriptor),
            Some(PathBuf::from("/tmp/root/nested/dir/file.txt"))
        );
    }
}
