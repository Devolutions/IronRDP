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
mod transport;
mod utils;

use std::sync::mpsc as mpsc_sync;

use ironrdp_cliprdr::backend::{ClipboardMessageProxy, CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse,
    FileDescriptor, FormatDataRequest, FormatDataResponse, LockDataId,
};
use tracing::{debug, error};
pub use windows::Win32::Foundation::HWND;
use windows::Win32::Foundation::{CLIPBRD_E_CANT_OPEN, E_ACCESSDENIED, FALSE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{APTTYPE, APTTYPE_MAINSTA, APTTYPE_STA, APTTYPEQUALIFIER, CoGetApartmentType};
use windows::Win32::System::DataExchange::{AddClipboardFormatListener, RemoveClipboardFormatListener};
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows::Win32::UI::Shell::{RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExA, DefWindowProcA, GetClassInfoA, RegisterClassA, SendMessageW, WINDOW_EX_STYLE,
    WM_USER, WNDCLASSA, WS_POPUP,
};
use windows::core::{Error, s};

use self::clipboard_impl::{ClipboardWindowState, WinClipboardImpl, clipboard_subproc};
use self::cliprdr_backend::WinCliprdrBackend;

const BACKEND_CHANNEL_SIZE: usize = 8;
const WM_CLIPRDR_BACKEND_EVENT: u32 = WM_USER;
/// Sent by [`WinClipboard`] when it is dropped, while the subclass is still installed.
const WM_CLIPRDR_SHUTDOWN: u32 = WM_USER + 1;

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
    RemoteFormatList(Vec<ClipboardFormat>),
    FormatDataRequest(FormatDataRequest),
    FormatDataResponse(FormatDataResponse<'static>),
    RemoteRequestsFormatList,
    RemoteFileList {
        files: Vec<FileDescriptor>,
        clip_data_id: Option<u32>,
    },
    FileContentsRequest(FileContentsRequest),
    FileContentsResponse(FileContentsResponse<'static>),
    Lock(LockDataId),
    Unlock(LockDataId),
    OutgoingLocksCleared(Vec<LockDataId>),
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
/// The embedder must route [`ClipboardMessage::SendInitiateFileCopy`],
/// [`ClipboardMessage::SendFileContentsRequest`] and
/// [`ClipboardMessage::SendFileContentsResponse`] to `Cliprdr`, and drive
/// `Cliprdr::drive_timeouts`. Embedders that don't should use
/// [`WinClipboard::with_file_transfer`] to turn file transfer off.
///
/// [MS-RDPECLIP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeclip/fb9b7e0b-6db4-41c2-b83c-f889c1ee7688
/// [`ClipboardMessage::SendInitiateFileCopy`]: ironrdp_cliprdr::backend::ClipboardMessage::SendInitiateFileCopy
/// [`ClipboardMessage::SendFileContentsRequest`]: ironrdp_cliprdr::backend::ClipboardMessage::SendFileContentsRequest
/// [`ClipboardMessage::SendFileContentsResponse`]: ironrdp_cliprdr::backend::ClipboardMessage::SendFileContentsResponse
pub struct WinClipboard {
    window: HWND,
    backend_tx: mpsc_sync::SyncSender<BackendEvent>,
    file_transfer: bool,
    ole_initialized: bool,
    /// Address of the subclass state, from `Box::into_raw`.
    state: usize,

    /// From MS docs:
    /// ```text
    /// You cannot use the subclassing helper functions to subclass a window across threads
    /// ```
    ///
    /// Therefore this type should be non-Send and non-Sync to prevent incorrect use.
    _thread_marker: core::marker::PhantomData<*const ()>,
}

impl WinClipboard {
    /// Creates new clipboard instance, with file transfer enabled.
    ///
    /// Under the hood, a hidden window is created for capturing the clipboard events.
    pub fn new(message_proxy: impl ClipboardMessageProxy + 'static) -> WinCliprdrResult<Self> {
        Self::with_file_transfer(message_proxy, true)
    }

    /// Creates new clipboard instance, with file transfer enabled or not.
    ///
    /// With `file_transfer` off, the backend neither negotiates nor produces any file stream
    /// message, and file lists are not advertised in either direction.
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

        let ctx = Box::new(ClipboardWindowState::new(WinClipboardImpl::new(
            window,
            message_proxy,
            backend_rx,
            file_transfer,
            ole_initialized,
        )));

        let state = Box::into_raw(ctx).expose_provenance();

        // We need to receive winapi messages in the main thread, so we need to add a subclass to
        // the window.
        //
        // SAFETY: `window` is a valid window handle, `clipboard_subproc` is in the static memory,
        // `ctx` is valid and its ownership is transferred to the subclass via `into_raw`.
        let winapi_result = unsafe { SetWindowSubclass(window, Some(clipboard_subproc), 0, state) };

        if winapi_result == FALSE {
            // SAFETY: `state` comes from `Box::into_raw` above and the subclass procedure never got it.
            drop(unsafe { Box::from_raw(core::ptr::with_exposed_provenance_mut::<ClipboardWindowState>(state)) });
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
            ole_initialized,
            state,
            _thread_marker: Default::default(),
        })
    }

    /// Returns clipboard backend factory suitable for making backend instances for `CLIPRDR` SVC.
    pub fn backend_factory(&self) -> Box<dyn CliprdrBackendFactory + Send> {
        Box::new(WinCliprdrBackendFactory {
            tx: self.backend_tx.clone(),
            window: self.window,
            file_transfer: self.file_transfer,
        })
    }
}

impl Drop for WinClipboard {
    fn drop(&mut self) {
        // Let the clipboard implementation take its OLE data object off the clipboard: it can't
        // serve anything once this instance is gone.
        //
        // SAFETY: `window` is a valid window handle owned by this thread.
        unsafe { SendMessageW(self.window, WM_CLIPRDR_SHUTDOWN, None, None) };

        // Remove clipboard processing from WinAPI event loop

        // SAFETY: Format listener was registered in the `new` method previously.
        if let Err(err) = unsafe { RemoveClipboardFormatListener(self.window) } {
            error!("Failed to remove clipboard listener: {}", err)
        }

        // SAFETY: Subclass was registered in the `new` method previously.
        if unsafe { RemoveWindowSubclass(self.window, Some(clipboard_subproc), 0) }.as_bool() {
            // The subclass procedure can't run anymore, so its state can go.
            //
            // SAFETY: `state` is the pointer returned by `Box::into_raw` in `new`. It was not freed
            // yet: that only happens on `WM_NCDESTROY`, after which the subclass is already gone and
            // can't be removed here.
            drop(unsafe {
                Box::from_raw(core::ptr::with_exposed_provenance_mut::<ClipboardWindowState>(
                    self.state,
                ))
            });
        } else {
            error!("Failed to remove window subclass")
        }

        if self.ole_initialized {
            // SAFETY: Balances the successful `OleInitialize` in `new`, on the same thread
            // (`WinClipboard` is neither `Send` nor `Sync`).
            unsafe { OleUninitialize() };
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

    // SAFETY: Called on an STA thread; balanced by `OleUninitialize` in `WinClipboard::drop`.
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
}

// SAFETY: window handle is thread safe for PostMessageW usage
unsafe impl Send for WinCliprdrBackendFactory {}

impl CliprdrBackendFactory for WinCliprdrBackendFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(WinCliprdrBackend::new(self.window, self.tx.clone(), self.file_transfer))
    }
}
