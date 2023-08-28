use crate::backend::windows::clipboard_data::ClipboardData;
use crate::backend::windows::clipboard_data_ref::ClipboardDataRef;
use crate::backend::windows::os_clipboard::OwnedOsClipboard;
use crate::backend::windows::remote_format_registry::RemoteClipboardFormatRegistry;
use crate::backend::windows::{BackendEvent, WinCliprdrError, WinCliprdrResult, WM_USER_CLIPBOARD};
use crate::backend::{ClipboardMessage, ClipboardMessageProxy};
use crate::pdu::{ClipboardFormat, ClipboardFormatId, FormatDataRequest, FormatDataResponse};

use winapi::shared::windef::HWND;
use winapi::um::commctrl::DefSubclassProc;
use winapi::um::winuser::{
    GetClipboardOwner, KillTimer, SetTimer, WA_INACTIVE, WM_ACTIVATE, WM_CLIPBOARDUPDATE, WM_DESTROY,
    WM_RENDERALLFORMATS, WM_RENDERFORMAT, WM_TIMER,
};

use std::collections::HashSet;
use std::sync::mpsc as mpsc_sync;
use std::time::Duration;

const RENDER_FORMAT_TIMEOUT_SECS: u64 = 10;
const IDT_CLIPBOARD_RETRY: usize = 1;

/// Internal implementation of the clipboard processing logic.
pub(crate) struct WinClipboardImpl {
    pub window: HWND,
    pub message_proxy: Box<dyn ClipboardMessageProxy>,
    pub window_is_active: bool,
    pub backend_rx: mpsc_sync::Receiver<BackendEvent>,
    // Number of attempts spent to process current clipboard message
    pub attempt: usize,
    // Message to retry
    pub retry_message: Option<BackendEvent>,
    // Formats avaialble on the remote (represented as LOCAL format ids)
    pub available_formats_on_remote: Vec<ClipboardFormatId>,
    pub remote_format_registry: RemoteClipboardFormatRegistry,
}

impl WinClipboardImpl {
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
        &mut self,
        requested_local_format: ClipboardFormatId,
        response: &FormatDataResponse,
    ) -> WinCliprdrResult<()> {
        if response.is_error() {
            // No data available for this format anymore
            return Ok(());
        }

        ClipboardData::render(requested_local_format, response.data())?;
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

        // We need to receive data from the remote clipboard immediately, becasuse Windows
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
                // Unexpected message, ignore it
                Ok(None)
            }
        }
    }

    fn on_render_format(&mut self, format: ClipboardFormatId) -> WinCliprdrResult<Option<ClipboardMessage>> {
        // Owning clipboard is not required when processing `WM_RENDERFORMAT` message
        if let Some(response) = self.get_remote_format_data(format)? {
            self.on_format_data_response(format, &response)?;
        }

        Ok(None)
    }

    fn on_render_all_formats(&mut self) -> WinCliprdrResult<Option<ClipboardMessage>> {
        // We need to be clipboard owner to be able to set all clipboard formats
        let _clipboard = match OwnedOsClipboard::new(self.window) {
            Ok(clipboard) => {
                if self.window != unsafe { GetClipboardOwner() } {
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

        let formats = std::mem::take(&mut self.available_formats_on_remote);

        // Clearing clipboard is not required, just render all available formats

        for format in formats {
            if let Some(response) = self.get_remote_format_data(format)? {
                self.on_format_data_response(format, &response)?;
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

            _ => Ok(None),
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
                const MAX_PROCESSING_ATTEMPTS: usize = 10;
                const PROCESSING_TIMEOUT_MS: u32 = 100;

                if self.attempt < MAX_PROCESSING_ATTEMPTS {
                    self.attempt += 1;
                    self.retry_message = Some(event);
                    unsafe {
                        SetTimer(
                            self.window,
                            IDT_CLIPBOARD_RETRY,
                            self.attempt as u32 * PROCESSING_TIMEOUT_MS,
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

// WinAPI event loop for clipboard processing
pub(crate) unsafe extern "system" fn clipboard_subproc(
    hwnd: winapi::shared::windef::HWND,
    msg: winapi::shared::minwindef::UINT,
    wparam: winapi::shared::minwindef::WPARAM,
    lparam: winapi::shared::minwindef::LPARAM,
    _id: winapi::shared::basetsd::UINT_PTR,
    data: winapi::shared::basetsd::DWORD_PTR,
) -> winapi::shared::minwindef::LRESULT {
    if msg == WM_DESTROY {
        // Remove previously allocated context
        std::mem::drop(Box::from_raw(data as *mut WinClipboardImpl));
        return 0;
    }

    let ctx = &mut *(data as *mut WinClipboardImpl);

    match msg {
        // We need to keep track of window state to distinguish between local and remote copy
        WM_ACTIVATE => {
            if wparam == WA_INACTIVE as _ {
                ctx.window_is_active = false;
            } else {
                ctx.window_is_active = true;
            }
        }
        // Sent by the OS when OS clipboard content is changed
        WM_CLIPBOARDUPDATE => {
            let clipboard_owner = GetClipboardOwner();
            let spurious_event = clipboard_owner == hwnd;

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
            ctx.handle_event(BackendEvent::RenderFormat(ClipboardFormatId::new(wparam as _)));
        }
        // Sent by the OS when all delay-rendered data is requested for rendering.
        WM_RENDERALLFORMATS => {
            ctx.handle_event(BackendEvent::RenderAllFormats);
        }
        // Manually sent messages from `Cliprdr` backend shim
        WM_USER_CLIPBOARD => {
            let message = if let Ok(message) = ctx.backend_rx.try_recv() {
                message
            } else {
                // No message has been received, spurious event
                return 0;
            };

            ctx.handle_event(message);
        }
        // Retry timer. Some operations clipboard operations such as `OpenClipboard` are
        // fallible. We need to retry them a few times before giving up.
        WM_TIMER => {
            if wparam == IDT_CLIPBOARD_RETRY {
                // Timer is one-shot, we need to stop it immediately
                KillTimer(hwnd, IDT_CLIPBOARD_RETRY);

                if let Some(event) = ctx.retry_message.take() {
                    ctx.handle_event(event);
                }
            }
        }
        _ => {
            // Call next event handler in the subclass chain
            return DefSubclassProc(hwnd, msg, wparam, lparam);
        }
    };

    0 // SUCCESS
}
