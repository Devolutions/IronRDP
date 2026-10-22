mod clipboard_data_ref;
mod clipboard_impl;
mod cliprdr_backend;
mod data_object;
#[cfg(feature = "__test")]
pub mod file_list;
#[cfg(not(feature = "__test"))]
#[expect(
    unreachable_pub,
    reason = "the __test feature exposes internals to the shared test suite"
)]
pub(crate) mod file_list;
mod local_files;
mod os_clipboard;
mod remote_format_registry;
mod shared;
mod transport;
mod utils;

use std::rc::Rc;
use std::sync::{Arc, mpsc as mpsc_sync};

use ironrdp_cliprdr::backend::{ClipboardMessageProxy, CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileDescriptor,
    FormatDataRequest, FormatDataResponse, LockDataId,
};
use tracing::{debug, warn};
pub use windows::Win32::Foundation::HWND;
use windows::Win32::Foundation::{CLIPBRD_E_CANT_OPEN, E_ACCESSDENIED, FALSE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{APTTYPE, APTTYPE_MAINSTA, APTTYPE_STA, APTTYPEQUALIFIER, CoGetApartmentType};
use windows::Win32::System::DataExchange::AddClipboardFormatListener;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Shell::SetWindowSubclass;
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExA, DefWindowProcA, GetClassInfoA, PostMessageW, RegisterClassA, WINDOW_EX_STYLE,
    WM_USER, WNDCLASSA, WS_POPUP,
};
use windows::core::{Error, s};

use self::clipboard_impl::{ClipboardWindowState, WinClipboardImpl, clipboard_subproc, release};
use self::cliprdr_backend::WinCliprdrBackend;
use self::shared::ClipboardShared;

const BACKEND_CHANNEL_SIZE: usize = 8;
const WM_CLIPRDR_BACKEND_EVENT: u32 = WM_USER;
/// Posted by a [`WinClipboard`] dropped on another thread than its own, with its reference to the
/// window state as `lparam`.
const WM_CLIPRDR_RELEASE: u32 = WM_USER + 1;

pub type WinCliprdrResult<T> = Result<T, WinCliprdrError>;

#[derive(Debug)]
pub enum WinCliprdrError {
    AddClipboardFormatListener,
    FormatsEnumeration,
    ClipboardAccessDenied,
    ClipboardOpen,
    ClipboardEmpty,
    Utf16Conversion,
    ClipboardData,
    SetClipboardData,
    WindowSubclass,
    Alloc,
    DataReceiveTimeout,
    RenderFormat,
    WinAPI(Error),
}

impl core::fmt::Display for WinCliprdrError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WinCliprdrError::AddClipboardFormatListener => write!(f, "failed to register clipboard format listener"),
            WinCliprdrError::FormatsEnumeration => {
                write!(f, "failed to enumerate formats available in the current clipboard")
            }
            WinCliprdrError::ClipboardAccessDenied => write!(f, "clipboard is busy or denied"),
            WinCliprdrError::ClipboardOpen => write!(f, "failed to open the clipboard"),
            WinCliprdrError::ClipboardEmpty => write!(f, "failed to empty the clipboard"),
            WinCliprdrError::Utf16Conversion => write!(f, "failed to convert UTF-16 string to UTF-8"),
            WinCliprdrError::ClipboardData => write!(f, "failed to get current clipboard data"),
            WinCliprdrError::SetClipboardData => write!(f, "failed to set clipboard data"),
            WinCliprdrError::WindowSubclass => write!(f, "failed to subclass window"),
            WinCliprdrError::Alloc => write!(f, "failed to allocate global memory"),
            WinCliprdrError::DataReceiveTimeout => write!(f, "failed to receive data from remote clipboard"),
            WinCliprdrError::RenderFormat => write!(f, "failed to render clipboard format"),
            WinCliprdrError::WinAPI(_error) => write!(f, "WinAPI error"),
        }
    }
}

impl core::error::Error for WinCliprdrError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            WinCliprdrError::AddClipboardFormatListener => None,
            WinCliprdrError::FormatsEnumeration => None,
            WinCliprdrError::ClipboardAccessDenied => None,
            WinCliprdrError::ClipboardOpen => None,
            WinCliprdrError::ClipboardEmpty => None,
            WinCliprdrError::Utf16Conversion => None,
            WinCliprdrError::ClipboardData => None,
            WinCliprdrError::SetClipboardData => None,
            WinCliprdrError::WindowSubclass => None,
            WinCliprdrError::Alloc => None,
            WinCliprdrError::DataReceiveTimeout => None,
            WinCliprdrError::RenderFormat => None,
            WinCliprdrError::WinAPI(error) => Some(error),
        }
    }
}

impl From<Error> for WinCliprdrError {
    fn from(err: Error) -> Self {
        // `CLIPBRD_E_CANT_OPEN` is what OLE reports when another application holds the clipboard.
        if err.code() == E_ACCESSDENIED || err.code() == CLIPBRD_E_CANT_OPEN {
            WinCliprdrError::ClipboardAccessDenied
        } else {
            WinCliprdrError::WinAPI(err)
        }
    }
}

/// Sent from the clipboard backend shim to the actual WinAPI subproc event loop
#[derive(Debug)]
pub(crate) enum BackendEvent {
    // Events generated by OS event loop
    ClipboardUpdated,
    RenderFormat(ClipboardFormatId),
    RenderAllFormats,

    // PDU processing events
    /// A backend was built for a new `CLIPRDR` channel.
    SessionStarted,
    DowngradedCapabilities(ClipboardGeneralCapabilityFlags),
    Ready,
    /// The remote format list, with the generation it starts.
    RemoteFormatList(Vec<ClipboardFormat>, u64),
    FormatDataRequest(FormatDataRequest),
    FormatDataResponse(FormatDataResponse<'static>),
    RemoteRequestsFormatList,
    /// The remote answered a format list this side sent: `true` if it accepted it.
    FormatListResponse(bool),
    RemoteFileList {
        files: Vec<FileDescriptor>,
        clip_data_id: Option<u32>,
    },
    FileContentsRequest(FileContentsRequest),
    Lock(LockDataId),
    Unlock(LockDataId),
}

/// Windows RDP client clipboard implementation.
///
/// IronRDP client implementation should provide raw window handle and message proxy to send
/// messages from the backend to `CLIPRDR` SVC.
///
/// [`WinClipboard`] instance holds ownership of the actual clipboard backend processing logic
/// and should be kept alive during the whole lifetime of the application.
///
/// This type is not thread safe, it should be used only in the main thread with a windows event
/// loop. (GUI thread). However, backend factory returned by [`WinClipboard::backend_factory`]
/// can be safely used in other threads.
///
/// # File transfer
///
/// Files are copied in both directions ([MS-RDPECLIP] file stream protocol): local files
/// (`CF_HDROP`, or `FileGroupDescriptorW` from another application) are offered to the
/// remote, and remote files are put on the local clipboard as an OLE data object, so that
/// pasting into Explorer works. The latter needs a single-threaded apartment: it is only
/// enabled when [`WinClipboard`] is created on an STA thread (a GUI thread normally is).
///
/// File transfer is off unless enabled with [`WinClipboard::with_file_transfer`]. The embedder
/// must then route [`ClipboardMessage::SendInitiateFileCopy`],
/// [`ClipboardMessage::SendFileContentsRequest`] and
/// [`ClipboardMessage::SendFileContentsResponse`] to `Cliprdr`, and drive
/// `Cliprdr::drive_timeouts`.
///
/// Remote files are read without the clipboard thread: their streams live in the process's
/// multithreaded apartment, so the pasting application's reads run on COM worker threads, with
/// several chunks requested ahead. Only fetching the file list waits on the clipboard thread,
/// once per paste. This also initializes the multithreaded apartment for the clipboard's
/// lifetime.
///
/// [MS-RDPECLIP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeclip/fb9b7e0b-6db4-41c2-b83c-f889c1ee7688
/// [`ClipboardMessage::SendInitiateFileCopy`]: ironrdp_cliprdr::backend::ClipboardMessage::SendInitiateFileCopy
/// [`ClipboardMessage::SendFileContentsRequest`]: ironrdp_cliprdr::backend::ClipboardMessage::SendFileContentsRequest
/// [`ClipboardMessage::SendFileContentsResponse`]: ironrdp_cliprdr::backend::ClipboardMessage::SendFileContentsResponse
pub struct WinClipboard {
    window: HWND,
    backend_tx: mpsc_sync::SyncSender<BackendEvent>,
    file_transfer: bool,
    shared: Arc<ClipboardShared>,
    /// Shared with the window subclass; `None` once dropped.
    state: Option<Rc<ClipboardWindowState>>,
    /// The thread that created the window, which owns it, its subclass and OLE.
    thread_id: u32,

    /// From MS docs:
    /// ```text
    /// You cannot use the subclassing helper functions to subclass a window across threads
    /// ```
    ///
    /// Therefore this type should be non-Send and non-Sync to prevent incorrect use.
    _thread_marker: core::marker::PhantomData<*const ()>,
}

impl WinClipboard {
    /// Creates new clipboard instance, without file transfer.
    ///
    /// Under the hood, a hidden window is created for capturing the clipboard events.
    pub fn new(message_proxy: impl ClipboardMessageProxy + 'static) -> WinCliprdrResult<Self> {
        Self::with_file_transfer(message_proxy, false)
    }

    /// Creates new clipboard instance, with file transfer enabled or not.
    ///
    /// With `file_transfer` on, the embedder must handle the file stream messages (see
    /// [`WinClipboard`]). With it off, the backend neither negotiates nor produces any, and file
    /// lists are not advertised in either direction.
    pub fn with_file_transfer(
        message_proxy: impl ClipboardMessageProxy + 'static,
        file_transfer: bool,
    ) -> WinCliprdrResult<Self> {
        extern "system" fn wndproc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
            // SAFETY: default handler
            unsafe { DefWindowProcA(window, message, wparam, lparam) }
        }

        // SAFETY: low-level WinAPI call
        let instance = unsafe { GetModuleHandleA(None)? };
        let window_class = s!("IronRDPClipboardMonitor");

        let mut existing_wc = WNDCLASSA::default();
        // SAFETY: `instance` is a valid module handle, `window_class` is a valid null-terminated string,
        // and `existing_wc` is a valid mutable reference to a WNDCLASSA structure.
        let class_exists = unsafe { GetClassInfoA(Some(instance.into()), window_class, &mut existing_wc).is_ok() };

        if !class_exists {
            let wc = WNDCLASSA {
                hInstance: instance.into(),
                lpszClassName: window_class,
                lpfnWndProc: Some(wndproc),
                ..Default::default()
            };

            // SAFETY: low-level WinAPI call
            let atom = unsafe { RegisterClassA(&wc) };
            if atom == 0 {
                return Err(WinCliprdrError::from(Error::from_thread()));
            }
        }

        // SAFETY: low-level WinAPI call
        let window = unsafe {
            CreateWindowExA(
                WINDOW_EX_STYLE::default(),
                window_class,
                None,
                WS_POPUP,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                None,
                None,
                Some(instance.into()),
                None,
            )?
        };

        if window.is_invalid() {
            return Err(WinCliprdrError::from(Error::from_thread()));
        }
        // Init clipboard processing for WinAPI event loop
        //
        // SAFETY: `window` is a valid window handle
        unsafe { AddClipboardFormatListener(window)? };

        let (backend_tx, backend_rx) = mpsc_sync::sync_channel(BACKEND_CHANNEL_SIZE);

        let ole_initialized = file_transfer && init_ole();

        let shared = Arc::new(ClipboardShared::new(Box::new(message_proxy)));

        let state = Rc::new(ClipboardWindowState::new(WinClipboardImpl::new(
            window,
            Arc::clone(&shared),
            backend_rx,
            file_transfer,
            ole_initialized,
        )));

        let subclass_reference = Rc::into_raw(Rc::clone(&state)).expose_provenance();

        // We need to receive winapi messages in the main thread, so we need to add a subclass to
        // the window.
        //
        // SAFETY: `window` is a valid window handle, `clipboard_subproc` is in the static memory,
        // and the subclass gets its own reference to the state, from `Rc::into_raw`.
        let winapi_result = unsafe { SetWindowSubclass(window, Some(clipboard_subproc), 0, subclass_reference) };

        if winapi_result == FALSE {
            // SAFETY: The reference comes from `Rc::into_raw` above and the subclass never got it.
            drop(unsafe {
                Rc::from_raw(core::ptr::with_exposed_provenance::<ClipboardWindowState>(
                    subclass_reference,
                ))
            });
            if ole_initialized {
                // SAFETY: Balances the successful `OleInitialize` above, on the same thread.
                unsafe { OleUninitialize() };
            }
            return Err(WinCliprdrError::WindowSubclass);
        }

        Ok(Self {
            window,
            backend_tx,
            file_transfer,
            shared,
            state: Some(state),
            // SAFETY: Always safe to call.
            thread_id: unsafe { GetCurrentThreadId() },
            _thread_marker: Default::default(),
        })
    }

    /// Returns clipboard backend factory suitable for making backend instances for `CLIPRDR` SVC.
    pub fn backend_factory(&self) -> Box<dyn CliprdrBackendFactory + Send> {
        Box::new(WinCliprdrBackendFactory {
            tx: self.backend_tx.clone(),
            window: self.window,
            file_transfer: self.file_transfer,
            shared: Arc::clone(&self.shared),
        })
    }
}

impl Drop for WinClipboard {
    fn drop(&mut self) {
        let Some(state) = self.state.take() else {
            return;
        };

        // SAFETY: Always safe to call.
        if unsafe { GetCurrentThreadId() } == self.thread_id {
            release(self.window, state);
            return;
        }

        // Dropped on another thread, e.g. by a .NET finalizer: the window, its subclass and OLE
        // belong to the thread that created them, so the reference is handed over to it rather
        // than used, or waited on, from here.
        let address = Rc::into_raw(state).expose_provenance();

        // SAFETY: Posting is safe from any thread; the window's thread takes over the reference.
        let posted = unsafe {
            PostMessageW(
                Some(self.window),
                WM_CLIPRDR_RELEASE,
                WPARAM(0),
                LPARAM(address.cast_signed()),
            )
        };

        if let Err(error) = posted {
            // The window's thread is gone: leaking the state is all that is safe from here.
            warn!(%error, "Clipboard dropped on another thread and its own is gone; leaking it");
        }
    }
}

/// Initializes OLE on the current thread if it is a single-threaded apartment.
///
/// OLE needs an STA. Threads without COM are left alone, rather than turned into STAs behind
/// the embedder's back.
fn init_ole() -> bool {
    let mut apartment_type = APTTYPE::default();
    let mut qualifier = APTTYPEQUALIFIER::default();
    // SAFETY: Both pointers are valid for writes.
    let apartment = unsafe { CoGetApartmentType(&mut apartment_type, &mut qualifier) };

    if apartment.is_err() || !(apartment_type == APTTYPE_STA || apartment_type == APTTYPE_MAINSTA) {
        debug!(
            ?apartment_type,
            "Clipboard thread is not a single-threaded apartment; remote files can't be pasted"
        );
        return false;
    }

    // SAFETY: Called on an STA thread; balanced by `OleUninitialize` when the clipboard is torn down.
    match unsafe { OleInitialize(None) } {
        Ok(()) => true,
        Err(error) => {
            debug!(%error, "Failed to initialize OLE; remote files can't be pasted");
            false
        }
    }
}

/// Windows-specific clipboard backend factory
struct WinCliprdrBackendFactory {
    tx: mpsc_sync::SyncSender<BackendEvent>,
    window: HWND,
    file_transfer: bool,
    shared: Arc<ClipboardShared>,
}

// SAFETY: window handle is thread safe for PostMessageW usage
unsafe impl Send for WinCliprdrBackendFactory {}

impl CliprdrBackendFactory for WinCliprdrBackendFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(WinCliprdrBackend::new(
            self.window,
            self.tx.clone(),
            self.file_transfer,
            Arc::clone(&self.shared),
        ))
    }
}
