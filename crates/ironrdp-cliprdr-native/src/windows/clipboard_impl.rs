use core::time::Duration;
use std::collections::HashSet;
use std::sync::mpsc;

use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy};
use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId, FormatDataRequest, FormatDataResponse};
use tracing::warn;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::GetClipboardOwner;
use windows::Win32::UI::Shell::DefSubclassProc;
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, SetTimer, WA_INACTIVE, WM_ACTIVATE, WM_CLIPBOARDUPDATE, WM_DESTROY, WM_RENDERALLFORMATS,
    WM_RENDERFORMAT, WM_TIMER,
};

use crate::windows::clipboard_data_ref::ClipboardDataRef;
use crate::windows::os_clipboard::OwnedOsClipboard;
use crate::windows::remote_format_registry::RemoteClipboardFormatRegistry;
use crate::windows::utils::render_format;
use crate::windows::{BackendEvent, WinCliprdrError, WinCliprdrResult, WM_CLIPRDR_BACKEND_EVENT};

const RENDER_FORMAT_TIMEOUT_SECS: u64 = 10;
const IDT_CLIPBOARD_RETRY: usize = 1;

/// Internal implementation of the clipboard processing logic.
pub(crate) struct WinClipboardImpl {
    window: HWND,
    message_proxy: Box<dyn ClipboardMessageProxy>,
    window_is_active: bool,
    backend_rx: mpsc::Receiver<BackendEvent>,
    // Number of attempts spent to process current clipboard message
    attempt: u32,
    // Message to retry
    retry_message: Option<BackendEvent>,
    // Formats available on the remote (represented as LOCAL format ids)
    available_formats_on_remote: Vec<ClipboardFormatId>,
    remote_format_registry: RemoteClipboardFormatRegistry,
}

impl WinClipboardImpl {
    pub(crate) fn new(
        window: HWND,
        message_proxy: impl ClipboardMessageProxy + 'static,
        backend_rx: mpsc::Receiver<BackendEvent>,
    ) -> Self {
        Self {
            window,
            message_proxy: Box::new(message_proxy),
            window_is_active: true, // We assume that we start with current window active,
            backend_rx,
            attempt: 0,
            retry_message: None,
            available_formats_on_remote: Vec::new(),
            remote_format_registry: Default::default(),
        }
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
        // `WM_RENDERFORMAT` and `WM_RENDERALLFORMATS` messages, so we can safely assume that
        // calling `ClipboardData::render` is safe.
        unsafe {
            render_format(requested_local_format, response.data())?;
        }

        Ok(())
    }

    fn on_clipboard_update(&mut self) -> WinCliprdrResult<Option<ClipboardMessage>> {
        let mut formats = OwnedOsClipboard::new(self.window)?.enum_available_formats()?;

        let mut filtered_format_ids = formats.iter().map(|f| f.id()).collect::<HashSet<_, _>>();
        filter_format_ids(&mut filtered_format_ids);
        formats.retain(|format| filtered_format_ids.contains(&format.id()));

        Ok(Some(ClipboardMessage::SendInitiateCopy(formats)))
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
        self.message_proxy
            .send_clipboard_message(ClipboardMessage::SendInitiatePaste(remote_format));
        let data = self
            .backend_rx
            .recv_timeout(Duration::from_secs(RENDER_FORMAT_TIMEOUT_SECS))
            .map_err(|_| WinCliprdrError::DataReceiveTimeout)?;

        match data {
            BackendEvent::FormatDataResponse(response) => Ok(Some(response)),
            _ => {
                warn!("Unexpected FormatData response: {:?}", data);
                // Unexpected message, ignore it
                Ok(None)
            }
        }
    }

    fn on_render_format(&mut self, format: ClipboardFormatId) -> WinCliprdrResult<Option<ClipboardMessage>> {
        // Owning clipboard is not required when processing `WM_RENDERFORMAT` message
        if let Some(response) = self.get_remote_format_data(format)? {
            Self::on_format_data_response(format, &response)?;
        }

        Ok(None)
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

    fn on_remote_format_list(&mut self, formats: &[ClipboardFormat]) -> WinCliprdrResult<Option<ClipboardMessage>> {
        self.available_formats_on_remote.clear();

        // Clear previous format mapping
        self.remote_format_registry.clear();

        let mut local_format_ids = formats
            .iter()
            .filter_map(|format| {
                // `register` will return None if format is unknown/unsupported on the local machine,
                // so we need to filter them out.
                self.remote_format_registry.register(format)
            })
            .collect::<HashSet<_>>();

        filter_format_ids(&mut local_format_ids);

        if local_format_ids.is_empty() {
            return Ok(None);
        }

        // We need to be clipboard owner to be able to set clipboard data
        let mut clipboard = OwnedOsClipboard::new(self.window)?;
        // Before sending delayed-rendered data, we need to clear clipboard first.
        clipboard.clear()?;

        for format in local_format_ids.into_iter() {
            clipboard.delay_render(format)?;
            // Store available format for `WM_RENDERALLFORMATS` processing.
            self.available_formats_on_remote.push(format);
        }

        Ok(None)
    }

    fn handle_event(&mut self, event: BackendEvent) {
        let result = match &event {
            BackendEvent::FormatDataRequest(request) => self.on_format_data_request(request),
            BackendEvent::FormatDataResponse(_) => {
                // Out-of-order message, ignore it.
                Ok(None)
            }
            BackendEvent::RemoteFormatList(formats) => self.on_remote_format_list(formats),

            BackendEvent::ClipboardUpdated | BackendEvent::RemoteRequestsFormatList => self.on_clipboard_update(),
            BackendEvent::RenderFormat(format) => self.on_render_format(*format),
            BackendEvent::RenderAllFormats => self.on_render_all_formats(),

            BackendEvent::DowngradedCapabilities(flags) => {
                warn!(?flags, "Unhandled downgraded capabilities event");
                Ok(None)
            }
        };

        let retry_err = match result {
            Ok(Some(message)) => {
                self.message_proxy.send_clipboard_message(message);
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
                    self.message_proxy
                        .send_clipboard_message(ClipboardMessage::Error(Box::new(err)));
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

                #[allow(clippy::arithmetic_side_effects)]
                // self.attempt can’t be greater than MAX_PROCESSING_ATTEMPTS, so the arithmetic is safe here
                if self.attempt < MAX_PROCESSING_ATTEMPTS {
                    self.attempt += 1;

                    self.retry_message = Some(event);

                    // SAFETY: `SetTimer` is always safe to call when `hwnd` is a valid window handle
                    unsafe {
                        SetTimer(
                            self.window,
                            IDT_CLIPBOARD_RETRY,
                            self.attempt * PROCESSING_TIMEOUT_MS,
                            None,
                        )
                    };
                } else {
                    // Send error, retries limit exceeded
                    self.message_proxy
                        .send_clipboard_message(ClipboardMessage::Error(Box::new(err)));
                }
            }
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

/// WinAPI event loop for clipboard processing
///
/// SAFETY: This function should only be used for windows subclassing api via `SetWindowSubclass`.
pub(crate) unsafe extern "system" fn clipboard_subproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    if msg == WM_DESTROY {
        // Transfer ownership and drop previously allocated context

        // SAFETY: `data` is a valid pointer, returned by `Box::into_raw`, transferred to OS earlier
        // via `SetWindowSubclass` call.
        let _ = unsafe { Box::from_raw(data as *mut WinClipboardImpl) };
        return LRESULT(0);
    }

    // SAFETY: `data` is a valid pointer, returned by `Box::into_raw`, transferred to OS earlier
    // via `SetWindowSubclass` call.
    let ctx = unsafe { &mut *(data as *mut WinClipboardImpl) };

    match msg {
        // We need to keep track of window state to distinguish between local and remote copy
        WM_ACTIVATE => ctx.window_is_active = wparam.0 != WA_INACTIVE as usize, // `as` conversion is fine for constants
        // Sent by the OS when OS clipboard content is changed
        WM_CLIPBOARDUPDATE => {
            // SAFETY: `GetClipboardOwner` is always safe to call.
            let clipboard_owner = unsafe { GetClipboardOwner() };
            let spurious_event = clipboard_owner == Ok(hwnd);

            // We need to send copy message from remote only when window is NOT active, because if
            // it is active, then user wants to perform copy from remote instead. Also, we need to
            // check that we are not the source of the clipboard change, because if we are,
            // then we don't need to send copy message to remote.
            if !(ctx.window_is_active || spurious_event) {
                ctx.handle_event(BackendEvent::ClipboardUpdated);
            }
        }
        // Sent by the OS when delay-rendered data is requested for rendering.
        WM_RENDERFORMAT => {
            #[allow(clippy::cast_possible_truncation)] // should never truncate in practice
            ctx.handle_event(BackendEvent::RenderFormat(ClipboardFormatId::new(wparam.0 as u32)));
        }
        // Sent by the OS when all delay-rendered data is requested for rendering.
        WM_RENDERALLFORMATS => {
            ctx.handle_event(BackendEvent::RenderAllFormats);
        }
        // User event, a message was sent by the `Cliprdr` backend shim
        WM_CLIPRDR_BACKEND_EVENT => {
            let message = if let Ok(message) = ctx.backend_rx.try_recv() {
                message
            } else {
                // No message has been received, spurious event
                return LRESULT(0);
            };

            ctx.handle_event(message);
        }
        // Retry timer. Some operations clipboard operations such as `OpenClipboard` are
        // fallible. We need to retry them a few times before giving up.
        WM_TIMER => {
            if wparam.0 == IDT_CLIPBOARD_RETRY {
                // Timer is one-shot, we need to stop it immediately

                // SAFETY: `KillTimer` is always safe to call when `hwnd` is a valid window handle.
                if let Err(err) = unsafe { KillTimer(hwnd, IDT_CLIPBOARD_RETRY) } {
                    tracing::error!("Failed to kill timer: {}", err);
                }

                if let Some(event) = ctx.retry_message.take() {
                    ctx.handle_event(event);
                }
            }
        }
        _ => {
            // Call next event handler in the subclass chain

            // SAFETY: `DefSubclassProc` is always safe to call in context of subclass event loop
            return unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
        }
    };

    LRESULT(0) // SUCCESS
}
