use core::cell::{Cell, RefCell, UnsafeCell};
use core::ffi::c_void;
use core::ptr::with_exposed_provenance;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, mpsc};

use ironrdp_cliprdr::backend::ClipboardMessage;
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags, FormatDataRequest,
    FormatDataResponse,
};
use tracing::{debug, error, warn};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, S_OK, WPARAM};
use windows::Win32::System::Com::{CO_MTA_USAGE_COOKIE, CoDecrementMTAUsage, CoIncrementMTAUsage, IDataObject};
use windows::Win32::System::DataExchange::{
    GetClipboardOwner, IsClipboardFormatAvailable, RemoveClipboardFormatListener,
};
use windows::Win32::System::Ole::{OleGetClipboard, OleSetClipboard, OleUninitialize};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, SetTimer, WA_INACTIVE, WM_ACTIVATE, WM_ACTIVATEAPP, WM_CLIPBOARDUPDATE, WM_NCDESTROY,
    WM_RENDERALLFORMATS, WM_RENDERFORMAT, WM_TIMER,
};
use windows::core::{HRESULT, Interface as _};

use crate::windows::clipboard_data_ref::ClipboardDataRef;
use crate::windows::data_object::{FileFormatIds, RemoteDataObject};
use crate::windows::file_list::{
    build_local_file_list, decode_file_group_descriptor, fits_without_huge_file_support, is_file_list_format,
    is_portable_format, is_sendable_descriptor,
};
use crate::windows::local_files::{LocalFileSource, LocalFiles, VirtualFiles, hglobal_data};
use crate::windows::os_clipboard::OwnedOsClipboard;
use crate::windows::remote_format_registry::{RemoteClipboardFormatRegistry, register_format_name};
use crate::windows::shared::ClipboardShared;
use crate::windows::transport::RemoteTransport;
use crate::windows::utils::render_format;
use crate::windows::{BackendEvent, WM_CLIPRDR_BACKEND_EVENT, WM_CLIPRDR_RELEASE, WinCliprdrError, WinCliprdrResult};

const IDT_CLIPBOARD_RETRY: usize = 1;

/// Files found on the local clipboard, before they are turned into a [`LocalFileSource`].
enum LocalClipboardFiles {
    Paths(Vec<PathBuf>),
    /// `FileGroupDescriptorW` and `FileContents` from another application.
    Virtual,
}

/// Internal implementation of the clipboard processing logic.
pub(crate) struct WinClipboardImpl {
    window: HWND,
    transport: Rc<RemoteTransport>,
    file_transfer: bool,
    /// OLE was initialized on this thread for this clipboard, and is uninitialized on teardown.
    ole_available: bool,
    /// Local ids of the formats used for files; `None` if they couldn't be registered.
    file_format_ids: Option<FileFormatIds>,
    // Number of attempts spent to process current clipboard message
    attempt: u32,
    // Message to retry
    retry_message: Option<BackendEvent>,
    // Formats available on the remote (represented as LOCAL format ids)
    available_formats_on_remote: Vec<ClipboardFormatId>,
    remote_format_registry: RemoteClipboardFormatRegistry,
    /// `Cliprdr` reached the Ready state, which `initiate_file_copy` requires.
    channel_ready: bool,
    local_files: LocalFiles,
    /// Data object put on the clipboard for remote files.
    remote_data_object: Option<IDataObject>,
    /// Keeps the multithreaded apartment, where remote file streams live, while OLE is in use.
    mta_usage: Option<CO_MTA_USAGE_COOKIE>,
}

impl WinClipboardImpl {
    pub(crate) fn new(
        window: HWND,
        shared: Arc<ClipboardShared>,
        backend_rx: mpsc::Receiver<BackendEvent>,
        file_transfer: bool,
        ole_available: bool,
    ) -> Self {
        let file_format_ids = file_format_ids();

        let mta_usage = if ole_available {
            // SAFETY: Plain call; balanced by `CoDecrementMTAUsage` in `shutdown`.
            match unsafe { CoIncrementMTAUsage() } {
                Ok(cookie) => Some(cookie),
                Err(error) => {
                    warn!(%error, "Failed to initialize the multithreaded apartment");
                    None
                }
            }
        } else {
            None
        };

        Self {
            window,
            transport: Rc::new(RemoteTransport::new(shared, backend_rx)),
            file_transfer,
            ole_available,
            file_format_ids,
            attempt: 0,
            retry_message: None,
            available_formats_on_remote: Vec::new(),
            remote_format_registry: Default::default(),
            channel_ready: false,
            local_files: LocalFiles::default(),
            remote_data_object: None,
            mta_usage,
        }
    }

    /// Whether local files can be offered to the remote.
    fn can_send_files(&self) -> bool {
        self.file_transfer_negotiated() && self.channel_ready
    }

    /// Whether remote files can be put on the local clipboard.
    fn can_receive_files(&self) -> bool {
        self.file_transfer_negotiated() && self.ole_available
    }

    /// Whether file transfer is enabled here and was negotiated with the remote.
    fn file_transfer_negotiated(&self) -> bool {
        self.file_transfer
            && self
                .transport
                .shared()
                .capabilities()
                .contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED)
    }

    /// Whether the clipboard holds what this instance put there for the remote.
    fn owns_clipboard(&self) -> bool {
        // SAFETY: `GetClipboardOwner` is always safe to call.
        let owner = unsafe { GetClipboardOwner() };
        owner == Ok(self.window) || self.owns_ole_clipboard()
    }

    fn owns_ole_clipboard(&self) -> bool {
        self.remote_data_object.as_ref().is_some_and(is_current_ole_clipboard)
    }

    fn on_format_data_request(&mut self, request: &FormatDataRequest) -> WinCliprdrResult<Option<ClipboardMessage>> {
        // Get data from the clipboard and send event to the main event loop
        let clipboard = OwnedOsClipboard::new(self.window)?;

        let buffer = ClipboardDataRef::get(&clipboard, request.format).map(|borrowed| borrowed.data().to_vec());

        let response = match buffer {
            Some(data) => FormatDataResponse::new_data(data),
            None => {
                // No data available for this format
                FormatDataResponse::new_error()
            }
        };

        Ok(Some(ClipboardMessage::SendFormatData(response)))
    }

    fn on_format_data_response(
        requested_local_format: ClipboardFormatId,
        response: &FormatDataResponse<'_>,
    ) -> WinCliprdrResult<()> {
        if response.is_error() {
            // No data available for this format anymore
            return Ok(());
        }

        // SAFETY: `on_format_data_response` is only called in a context of processing
        // `WM_RENDERFORMAT` and `WM_RENDERALLFORMATS` messages, or with the clipboard opened by its
        // owner, so we can safely assume that calling `ClipboardData::render` is safe.
        unsafe {
            render_format(requested_local_format, response.data())?;
        }

        Ok(())
    }

    fn on_clipboard_update(&mut self) -> WinCliprdrResult<Option<ClipboardMessage>> {
        if self.owns_clipboard() {
            // The clipboard holds remote data (e.g. while reconnecting): it can't be offered back.
            self.local_files.offer(None);
            return Ok(Some(ClipboardMessage::SendInitiateCopy(Vec::new())));
        }

        let mut clipboard = OwnedOsClipboard::new(self.window)?;
        let formats = clipboard.enum_available_formats()?;

        let files = if self.can_send_files() {
            self.find_local_files(&mut clipboard, &formats)
        } else {
            None
        };
        drop(clipboard);

        if let Some(source) = files.and_then(|files| self.local_file_source(files)) {
            let descriptors = source.descriptors();
            debug!(file_count = descriptors.len(), "Offering local files to the remote");
            self.local_files.offer(Some(Rc::new(source)));
            return Ok(Some(ClipboardMessage::SendInitiateFileCopy(descriptors)));
        }

        self.local_files.offer(None);

        let mut formats = formats;
        formats.retain(is_portable_format);

        let mut filtered_format_ids = formats.iter().map(|f| f.id()).collect::<HashSet<_, _>>();
        filter_format_ids(&mut filtered_format_ids);
        formats.retain(|format| filtered_format_ids.contains(&format.id()));

        Ok(Some(ClipboardMessage::SendInitiateCopy(formats)))
    }

    /// Looks for files on the open clipboard: `CF_HDROP` paths first, then descriptors of
    /// files provided by another application through `FileContents`.
    fn find_local_files(
        &self,
        clipboard: &mut OwnedOsClipboard,
        formats: &[ClipboardFormat],
    ) -> Option<LocalClipboardFiles> {
        let has_format = |id: ClipboardFormatId| formats.iter().any(|format| format.id() == id);

        if has_format(ClipboardFormatId::CF_HDROP) {
            if let Some(paths) = clipboard.dropped_files() {
                return Some(LocalClipboardFiles::Paths(paths));
            }
        }

        let ids = self.file_format_ids?;
        let file_descriptor = ClipboardFormatId::new(u32::from(ids.file_descriptor));
        let file_contents = ClipboardFormatId::new(u32::from(ids.file_contents));

        if self.ole_available && has_format(file_descriptor) && has_format(file_contents) {
            return Some(LocalClipboardFiles::Virtual);
        }

        None
    }

    fn local_file_source(&self, files: LocalClipboardFiles) -> Option<LocalFileSource> {
        // Without it, the remote can't read past 4 GiB: larger files are left out of the offer.
        let huge_files = self
            .transport
            .shared()
            .capabilities()
            .contains(ClipboardGeneralCapabilityFlags::HUGE_FILE_SUPPORT_ENABLED);

        match files {
            LocalClipboardFiles::Paths(paths) => {
                let mut entries = build_local_file_list(&paths);
                if !huge_files {
                    entries.retain(|entry| fits_without_huge_file_support(&entry.descriptor));
                }
                if entries.is_empty() {
                    return None;
                }
                Some(LocalFileSource::Paths(entries))
            }
            LocalClipboardFiles::Virtual => {
                let ids = self.file_format_ids?;

                // SAFETY: OLE was initialized on this thread (`ole_available`).
                let data_object = match unsafe { OleGetClipboard() } {
                    Ok(data_object) => data_object,
                    Err(error) => {
                        debug!(%error, "Failed to get the local clipboard data object");
                        return None;
                    }
                };

                // The list comes from the object that provides the contents, so the two match
                // even if the clipboard changes meanwhile.
                let descriptors = hglobal_data(&data_object, ids.file_descriptor)
                    .and_then(|data| decode_file_group_descriptor(&data))?;

                let files = descriptors
                    .into_iter()
                    .zip(0i32..)
                    .filter(|(descriptor, _)| is_sendable_descriptor(descriptor))
                    .filter(|(descriptor, _)| huge_files || fits_without_huge_file_support(descriptor))
                    .collect::<Vec<_>>();
                if files.is_empty() {
                    return None;
                }

                Some(LocalFileSource::Virtual(VirtualFiles::new(
                    data_object,
                    ids.file_contents,
                    files,
                )))
            }
        }
    }

    fn get_remote_format_data(
        &mut self,
        format: ClipboardFormatId,
    ) -> WinCliprdrResult<Option<FormatDataResponse<'static>>> {
        let mapped_format = self.remote_format_registry.local_to_remote(format);

        let remote_format = if let Some(format) = mapped_format {
            format
        } else {
            // Format is unknown or not supported on the local machine
            return Ok(None);
        };

        // We need to receive data from the remote clipboard immediately, because Windows
        // expects us to set clipboard data before returning from `WM_RENDERFORMAT` handler.
        //
        // Sadly, this will block the GUI thread, while data is being received
        self.transport.fetch_format_data(remote_format)
    }

    fn on_render_format(&mut self, format: ClipboardFormatId) -> WinCliprdrResult<Option<ClipboardMessage>> {
        // Owning clipboard is not required when processing `WM_RENDERFORMAT` message
        if let Some(response) = self.get_remote_format_data(format)? {
            Self::on_format_data_response(format, &response)?;
        }

        Ok(None)
    }

    /// Renders a format whose `WM_RENDERFORMAT` arrived while a handler was busy, so that later
    /// reads get the data even though that one went without.
    fn render_late(&mut self, format: ClipboardFormatId) -> WinCliprdrResult<()> {
        // Outside `WM_RENDERFORMAT`, data can only be set by the owner, with the clipboard open.
        let _clipboard = OwnedOsClipboard::new(self.window)?;
        // SAFETY: `GetClipboardOwner` is always safe to call
        if self.window != unsafe { GetClipboardOwner()? } {
            return Ok(());
        }

        self.on_render_format(format).map(|_| ())
    }

    fn on_render_all_formats(&mut self) -> WinCliprdrResult<Option<ClipboardMessage>> {
        // We need to be clipboard owner to be able to set all clipboard formats
        let _clipboard = match OwnedOsClipboard::new(self.window) {
            Ok(clipboard) => {
                // SAFETY: `GetClipboardOwner` is always safe to call
                if self.window != unsafe { GetClipboardOwner()? } {
                    // As per MSDN, we need to validate clipboard owner after opening clipboard
                    return Ok(None);
                }

                clipboard
            }
            Err(WinCliprdrError::ClipboardAccessDenied) => {
                // We don't own the clipboard anymore, we don't need to do anything
                return Ok(None);
            }
            Err(err) => {
                return Err(err);
            }
        };

        let formats = core::mem::take(&mut self.available_formats_on_remote);

        // Clearing clipboard is not required, just render all available formats

        for format in formats {
            if let Some(response) = self.get_remote_format_data(format)? {
                Self::on_format_data_response(format, &response)?;
            }
        }

        Ok(None)
    }

    fn on_remote_format_list(
        &mut self,
        formats: &[ClipboardFormat],
        generation: u64,
    ) -> WinCliprdrResult<Option<ClipboardMessage>> {
        self.available_formats_on_remote.clear();

        // Clear previous format mapping
        self.remote_format_registry.clear();

        let mut local_format_ids = formats
            .iter()
            .filter(|format| is_portable_format(format))
            .filter_map(|format| {
                // `register` will return None if format is unknown/unsupported on the local machine,
                // so we need to filter them out.
                self.remote_format_registry.register(format)
            })
            .collect::<HashSet<_>>();

        filter_format_ids(&mut local_format_ids);

        let remote_file_list_format = formats.iter().find(|format| is_file_list_format(format));

        if let (Some(remote_file_list_format), Some(ids)) = (remote_file_list_format, self.file_format_ids) {
            if self.can_receive_files() {
                return self.offer_remote_files(remote_file_list_format.id(), ids, &local_format_ids, generation);
            }
        }

        if local_format_ids.is_empty() {
            return Ok(None);
        }

        // We need to be clipboard owner to be able to set clipboard data
        let mut clipboard = OwnedOsClipboard::new(self.window)?;
        // Before sending delayed-rendered data, we need to clear clipboard first.
        clipboard.clear()?;
        self.remote_data_object = None;

        for format in local_format_ids.into_iter() {
            clipboard.delay_render(format)?;
            // Store available format for `WM_RENDERALLFORMATS` processing.
            self.available_formats_on_remote.push(format);
        }

        Ok(None)
    }

    /// Puts a data object for the remote clipboard, files included, on the local clipboard.
    fn offer_remote_files(
        &mut self,
        remote_file_list_format: ClipboardFormatId,
        ids: FileFormatIds,
        local_format_ids: &HashSet<ClipboardFormatId>,
        generation: u64,
    ) -> WinCliprdrResult<Option<ClipboardMessage>> {
        let passthrough = local_format_ids
            .iter()
            .filter_map(|local_format| {
                let local = u16::try_from(local_format.value()).ok()?;
                let remote = self.remote_format_registry.local_to_remote(*local_format)?;
                Some((local, remote))
            })
            .collect();

        let data_object: IDataObject = RemoteDataObject::new(
            Rc::clone(&self.transport),
            generation,
            ids,
            remote_file_list_format,
            passthrough,
            self.mta_usage.is_some(),
        )
        .into();

        // SAFETY: OLE was initialized on this thread (`can_receive_files`).
        unsafe { OleSetClipboard(&data_object)? };

        debug!("Remote files are available on the local clipboard");
        self.remote_data_object = Some(data_object);

        Ok(None)
    }

    fn on_session_started(&mut self) {
        self.channel_ready = false;
        self.transport.start_session();
        self.local_files.clear();
        // Its files belong to the previous channel and can't be fetched anymore.
        self.remove_remote_data_object();
    }

    fn on_ready(&mut self) -> WinCliprdrResult<Option<ClipboardMessage>> {
        self.channel_ready = true;

        // The format list sent while initializing could not include files: `Cliprdr` only
        // accepts a file list once the channel is ready, so offer them now.
        if !self.can_send_files() || self.owns_clipboard() || !self.local_clipboard_has_files() {
            return Ok(None);
        }

        self.on_clipboard_update()
    }

    fn local_clipboard_has_files(&self) -> bool {
        let is_available = |format: u32| {
            // SAFETY: `IsClipboardFormatAvailable` is always safe to call.
            unsafe { IsClipboardFormatAvailable(format) }.is_ok()
        };

        is_available(ClipboardFormatId::CF_HDROP.value())
            || self
                .file_format_ids
                .is_some_and(|ids| is_available(u32::from(ids.file_descriptor)))
    }

    /// Takes the remote data object off the clipboard, as it can't be served anymore, and lets go
    /// of the COM objects held, before OLE is uninitialized.
    fn shutdown(&mut self) {
        self.local_files.clear();
        self.remove_remote_data_object();
        // Applications may still hold the data object or its streams.
        self.transport.close();

        if let Some(cookie) = self.mta_usage.take() {
            // SAFETY: Balances `CoIncrementMTAUsage` in `new`.
            if let Err(error) = unsafe { CoDecrementMTAUsage(cookie) } {
                debug!(%error, "Failed to release the multithreaded apartment");
            }
        }
    }

    /// Takes the data object for remote files off the clipboard, if it is still there.
    fn remove_remote_data_object(&mut self) {
        if self.owns_ole_clipboard() {
            // SAFETY: OLE was initialized on this thread, as the data object was set.
            if let Err(error) = unsafe { OleSetClipboard(None) } {
                warn!(%error, "Failed to clear the remote files from the clipboard");
            }
        }

        self.remote_data_object = None;
    }

    fn handle_event(&mut self, event: BackendEvent) {
        let result = match &event {
            BackendEvent::FormatDataRequest(request) => self.on_format_data_request(request),
            BackendEvent::FormatDataResponse(_) | BackendEvent::RemoteFileList { .. } => {
                // The answer to a paste whose wait gave up.
                self.transport.drop_late_answer();
                Ok(None)
            }
            BackendEvent::RemoteFormatList(formats, generation) => self.on_remote_format_list(formats, *generation),

            BackendEvent::ClipboardUpdated | BackendEvent::RemoteRequestsFormatList => self.on_clipboard_update(),
            BackendEvent::RenderFormat(format) => self.on_render_format(*format),
            BackendEvent::RenderAllFormats => self.on_render_all_formats(),

            BackendEvent::SessionStarted => {
                self.on_session_started();
                Ok(None)
            }
            BackendEvent::DowngradedCapabilities(flags) => {
                debug!(?flags, "Clipboard capabilities negotiated");
                Ok(None)
            }
            BackendEvent::Ready => self.on_ready(),

            BackendEvent::FileContentsRequest(request) => Ok(Some(ClipboardMessage::SendFileContentsResponse(
                self.local_files.file_contents(request),
            ))),
            BackendEvent::FormatListResponse(accepted) => {
                self.local_files.acknowledge(*accepted);
                Ok(None)
            }
            BackendEvent::Lock(data_id) => {
                self.local_files.lock(data_id.0);
                Ok(None)
            }
            BackendEvent::Unlock(data_id) => {
                self.local_files.unlock(data_id.0);
                Ok(None)
            }
        };

        let retry_err = match result {
            Ok(Some(message)) => {
                self.transport.send(message);
                None
            }
            Ok(None) => {
                // No message to send
                None
            }
            Err(err) => {
                // Some errors are retryable (e.g. access to clipboard is temporarily denied)
                if let WinCliprdrError::ClipboardAccessDenied = &err {
                    Some(err)
                } else {
                    self.transport.send(ClipboardMessage::Error(Box::new(err)));
                    None
                }
            }
        };

        match retry_err {
            None => {
                self.attempt = 0;
            }
            Some(err) => {
                const MAX_PROCESSING_ATTEMPTS: u32 = 10;
                const PROCESSING_TIMEOUT_MS: u32 = 100;

                #[expect(clippy::arithmetic_side_effects)]
                // self.attempt can’t be greater than MAX_PROCESSING_ATTEMPTS, so the arithmetic is safe here
                if self.attempt < MAX_PROCESSING_ATTEMPTS {
                    self.attempt += 1;

                    self.retry_message = Some(event);

                    // SAFETY: `SetTimer` is always safe to call when `hwnd` is a valid window handle
                    unsafe {
                        SetTimer(
                            Some(self.window),
                            IDT_CLIPBOARD_RETRY,
                            self.attempt * PROCESSING_TIMEOUT_MS,
                            None,
                        )
                    };
                } else {
                    // Send error, retries limit exceeded
                    self.transport.send(ClipboardMessage::Error(Box::new(err)));
                }
            }
        }
    }

    fn handle_backend_event(&mut self) {
        if let Some(event) = self.transport.next_event() {
            self.handle_event(event);
        }
        // Otherwise no message has been received, spurious event
    }

    fn handle_retry_timer(&mut self) {
        // Timer is one-shot, we need to stop it immediately

        // SAFETY: `KillTimer` is always safe to call when `hwnd` is a valid window handle.
        if let Err(err) = unsafe { KillTimer(Some(self.window), IDT_CLIPBOARD_RETRY) } {
            error!("Failed to kill timer: {}", err);
        }

        if let Some(event) = self.retry_message.take() {
            self.handle_event(event);
        }
    }
}

fn filter_format_ids(formats: &mut HashSet<ClipboardFormatId>) {
    let has_non_utf16_formats =
        formats.contains(&ClipboardFormatId::CF_TEXT) || formats.contains(&ClipboardFormatId::CF_OEMTEXT);

    let has_utf16_formats = formats.contains(&ClipboardFormatId::CF_UNICODETEXT);

    // Windows could implicitly convert `CF_UNICODETEXT` to `CF_TEXT`, so to avoid
    // character encoding issues with application which prefer non-unicode text, we need to remove
    // `CF_TEXT` and `CF_OEMTEXT` from the list of formats.
    if has_utf16_formats && has_non_utf16_formats {
        formats.remove(&ClipboardFormatId::CF_TEXT);
        formats.remove(&ClipboardFormatId::CF_OEMTEXT);
    }
}

fn file_format_ids() -> Option<FileFormatIds> {
    let register = |name: &str| register_format_name(name).and_then(|id| u16::try_from(id.value()).ok());

    Some(FileFormatIds {
        file_descriptor: register(ClipboardFormatName::FILE_LIST.value())?,
        file_contents: register("FileContents")?,
        preferred_drop_effect: register(ClipboardFormatName::PREFERRED_DROP_EFFECT.value())?,
    })
}

fn is_current_ole_clipboard(data_object: &IDataObject) -> bool {
    // The `windows` binding folds S_FALSE ("not current") into `Ok`.
    windows::core::link!("ole32.dll" "system" fn OleIsCurrentClipboard(pdataobj: *mut c_void) -> HRESULT);

    // SAFETY: `data_object` is a valid interface pointer for the duration of the call.
    let result = unsafe { OleIsCurrentClipboard(data_object.as_raw()) };
    result == S_OK
}

/// State attached to the clipboard window through its subclass.
///
/// Some calls made while handling a message dispatch other messages to the same window before
/// returning: OLE calls into another process pump messages while they wait, and `EmptyClipboard`
/// lets other processes' `WM_RENDERFORMAT` requests through. A message arriving that way must
/// not touch the [`WinClipboardImpl`] already borrowed by the outer handler, so it is put off
/// until that handler returns.
///
/// The state is reference counted: [`crate::WinClipboard`] holds one reference, the subclass
/// another, and every running handler its own, so that nothing a handler on the stack uses is
/// freed under it, even when the clipboard is dropped from within one of its calls.
pub(crate) struct ClipboardWindowState {
    inner: UnsafeCell<WinClipboardImpl>,
    busy: Cell<bool>,
    window_is_active: Cell<bool>,
    missed_backend_events: Cell<u32>,
    missed_clipboard_update: Cell<bool>,
    missed_retry_timer: Cell<bool>,
    missed_render_formats: RefCell<Vec<ClipboardFormatId>>,
    /// The clipboard was dropped while a handler was busy: the outermost one tears it down.
    teardown_requested: Cell<bool>,
    torn_down: Cell<bool>,
}

impl ClipboardWindowState {
    pub(crate) fn new(inner: WinClipboardImpl) -> Self {
        Self {
            inner: UnsafeCell::new(inner),
            busy: Cell::new(false),
            window_is_active: Cell::new(true), // We assume that we start with current window active,
            missed_backend_events: Cell::new(0),
            missed_clipboard_update: Cell::new(false),
            missed_retry_timer: Cell::new(false),
            missed_render_formats: RefCell::new(Vec::new()),
            teardown_requested: Cell::new(false),
            torn_down: Cell::new(false),
        }
    }

    /// Runs `f` with exclusive access to the implementation, then whatever was put off meanwhile.
    ///
    /// Returns `None` without running `f` when called re-entrantly.
    fn with_impl<R>(&self, f: impl FnOnce(&mut WinClipboardImpl, &Self) -> R) -> Option<R> {
        if self.busy.replace(true) {
            return None;
        }

        // SAFETY: `busy` was false, so no other reference to `inner` exists: every access goes
        // through this method, on the window's thread.
        let ctx = unsafe { &mut *self.inner.get() };

        let result = f(ctx, self);

        loop {
            if self.missed_backend_events.get() != 0 {
                self.missed_backend_events.set(self.missed_backend_events.get() - 1);
                ctx.handle_backend_event();
            } else if self.missed_clipboard_update.replace(false) {
                self.on_clipboard_update(ctx);
            } else if self.missed_retry_timer.replace(false) {
                ctx.handle_retry_timer();
            } else {
                let Some(format) = self.missed_render_formats.borrow_mut().pop() else {
                    break;
                };
                if let Err(error) = ctx.render_late(format) {
                    debug!(%error, ?format, "Failed to render clipboard data late");
                }
            }
        }

        self.busy.set(false);
        Some(result)
    }

    fn on_clipboard_update(&self, ctx: &mut WinClipboardImpl) {
        // We need to send copy message from remote only when window is NOT active, because if
        // it is active, then user wants to perform copy from remote instead. Also, we need to
        // check that we are not the source of the clipboard change, because if we are,
        // then we don't need to send copy message to remote.
        if !(self.window_is_active.get() || ctx.owns_clipboard()) {
            ctx.handle_event(BackendEvent::ClipboardUpdated);
        }
    }
}

/// Gives up the [`crate::WinClipboard`] reference to `state`, tearing the clipboard down.
///
/// Must be called on the window's thread. If a handler is running further up the stack, the
/// teardown happens when it returns instead.
pub(crate) fn release(window: HWND, state: Rc<ClipboardWindowState>) {
    if state.busy.get() {
        state.teardown_requested.set(true);
        return;
    }

    teardown(window, &state);
}

/// Takes the remote data object off the clipboard, detaches the window and uninitializes OLE.
///
/// The caller holds a reference to `state` besides the subclass's, which this releases.
fn teardown(window: HWND, state: &ClipboardWindowState) {
    if state.torn_down.replace(true) {
        return;
    }

    // Not busy here: callers check, so this always runs.
    let ole_initialized = state
        .with_impl(|ctx, _| {
            ctx.shutdown();
            ctx.ole_available
        })
        .unwrap_or(false);

    // SAFETY: The format listener was registered when the clipboard was created.
    if let Err(error) = unsafe { RemoveClipboardFormatListener(window) } {
        debug!(%error, "Failed to remove clipboard listener");
    }

    // SAFETY: Removing our own subclass, on the window's thread.
    if unsafe { RemoveWindowSubclass(window, Some(clipboard_subproc), 0) }.as_bool() {
        // SAFETY: The subclass held a reference from `Rc::into_raw`, and can't use it anymore. The
        // caller's reference keeps `state` alive past this.
        drop(unsafe { Rc::from_raw(core::ptr::from_ref(state)) });
    } else {
        // Already removed on `WM_NCDESTROY`, with its reference.
        debug!("Clipboard window subclass already removed");
    }

    if ole_initialized {
        // SAFETY: Balances the successful `OleInitialize` made when the clipboard was created, on
        // the same thread, now that no call into OLE is on the stack.
        unsafe { OleUninitialize() };
    }
}

/// WinAPI event loop for clipboard processing
///
/// # Safety
///
/// This function should only be used for windows subclassing api via `SetWindowSubclass`, with
/// a reference to a [`ClipboardWindowState`] from `Rc::into_raw` as `data`.
pub(crate) unsafe extern "system" fn clipboard_subproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    let subclass_reference = with_exposed_provenance::<ClipboardWindowState>(data);

    // This handler's own reference, so that the state outlives it even if the clipboard is torn
    // down by a call it makes.
    //
    // SAFETY: The subclass holds a reference from `Rc::into_raw`, valid while it is installed.
    unsafe { Rc::increment_strong_count(subclass_reference) };
    // SAFETY: Same as above; this takes over the count just added.
    let state = unsafe { Rc::from_raw(subclass_reference) };

    let result = match msg {
        // Last message of the window: the subclass goes, with its reference.
        WM_NCDESTROY => {
            // SAFETY: Removing our own subclass from within its procedure is allowed.
            if unsafe { RemoveWindowSubclass(hwnd, Some(clipboard_subproc), id) }.as_bool() {
                // SAFETY: The subclass can't use its reference anymore.
                drop(unsafe { Rc::from_raw(subclass_reference) });
            } else {
                error!("Failed to remove window subclass");
            }

            // SAFETY: `DefSubclassProc` is always safe to call in context of subclass event loop
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        // We need to keep track of window state to distinguish between local and remote copy
        WM_ACTIVATE | WM_ACTIVATEAPP => {
            state
                .window_is_active
                .set(wparam.0 != usize::try_from(WA_INACTIVE).expect("WA_INACTIVE fits into usize"));
            LRESULT(0)
        }
        // Sent by the OS when OS clipboard content is changed
        WM_CLIPBOARDUPDATE => {
            if state.with_impl(|ctx, state| state.on_clipboard_update(ctx)).is_none() {
                state.missed_clipboard_update.set(true);
            }
            LRESULT(0)
        }
        // Sent by the OS when delay-rendered data is requested for rendering.
        WM_RENDERFORMAT => {
            let format = ClipboardFormatId::new(u32::try_from(wparam.0).expect("should never truncate in practice"));
            if state
                .with_impl(|ctx, _| ctx.handle_event(BackendEvent::RenderFormat(format)))
                .is_none()
            {
                // The data has to be set before returning, and it can't be fetched right now: this
                // request goes without, but it is rendered once the busy handler returns.
                debug!(?format, "Clipboard data requested while busy; rendering it later");
                state.missed_render_formats.borrow_mut().push(format);
            }
            LRESULT(0)
        }
        // Sent by the OS when all delay-rendered data is requested for rendering.
        WM_RENDERALLFORMATS => {
            if state
                .with_impl(|ctx, _| ctx.handle_event(BackendEvent::RenderAllFormats))
                .is_none()
            {
                warn!("All clipboard data requested while busy; not rendered");
            }
            LRESULT(0)
        }
        // User event, a message was sent by the `Cliprdr` backend shim
        WM_CLIPRDR_BACKEND_EVENT => {
            if state.with_impl(|ctx, _| ctx.handle_backend_event()).is_none() {
                state
                    .missed_backend_events
                    .set(state.missed_backend_events.get().saturating_add(1));
            }
            LRESULT(0)
        }
        // A `WinClipboard` dropped on another thread handed its reference over.
        WM_CLIPRDR_RELEASE => {
            let address = lparam.0.cast_unsigned();
            // SAFETY: `WinClipboard::drop` posts a reference from `Rc::into_raw` as `lparam`.
            let reference = unsafe { Rc::from_raw(with_exposed_provenance::<ClipboardWindowState>(address)) };
            release(hwnd, reference);
            LRESULT(0)
        }
        // Retry timer. Some operations clipboard operations such as `OpenClipboard` are
        // fallible. We need to retry them a few times before giving up.
        WM_TIMER if wparam.0 == IDT_CLIPBOARD_RETRY => {
            if state.with_impl(|ctx, _| ctx.handle_retry_timer()).is_none() {
                state.missed_retry_timer.set(true);
            }
            LRESULT(0)
        }
        // Call next event handler in the subclass chain
        //
        // SAFETY: `DefSubclassProc` is always safe to call in context of subclass event loop
        _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    };

    // The clipboard was dropped by a call made from a handler: the outermost one tears it down.
    if state.teardown_requested.get() && !state.busy.get() {
        teardown(hwnd, &state);
    }

    result
}
