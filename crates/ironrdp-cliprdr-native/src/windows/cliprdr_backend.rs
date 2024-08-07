use std::sync::mpsc as mpsc_sync;

use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse, FormatDataRequest,
    FormatDataResponse, LockDataId,
};
use ironrdp_svc::impl_as_any;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::windows::{BackendEvent, WM_CLIPRDR_BACKEND_EVENT};

#[derive(Debug)]
pub(crate) struct WinCliprdrBackend {
    backend_event_tx: mpsc_sync::SyncSender<BackendEvent>,
    window: HWND,
}

// SAFETY: window handle is thread safe for PostMessageW usage
unsafe impl Send for WinCliprdrBackend {}

impl_as_any!(WinCliprdrBackend);

impl WinCliprdrBackend {
    pub(crate) fn new(window: HWND, backend_event_tx: mpsc_sync::SyncSender<BackendEvent>) -> Self {
        Self {
            window,
            backend_event_tx,
        }
    }

    fn send_event(&self, event: BackendEvent) {
        if self.backend_event_tx.send(event).is_err() {
            // Channel is closed, backend is dead
            return;
        }
        // Wake up subproc event loop; Dont wait for result
        //
        // SAFETY: it is safe to call PostMessageW from any thread with a valid window handle
        if let Err(err) = unsafe { PostMessageW(self.window, WM_CLIPRDR_BACKEND_EVENT, WPARAM(0), LPARAM(0)) } {
            tracing::error!("Failed to post message to wake up subproc event loop: {}", err);
        }
    }
}

impl CliprdrBackend for WinCliprdrBackend {
    fn temporary_directory(&self) -> &str {
        ".cliprdr"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        // No additional capabilities yet
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_process_negotiated_capabilities(&mut self, capabilities: ClipboardGeneralCapabilityFlags) {
        self.send_event(BackendEvent::DowngradedCapabilities(capabilities))
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        self.send_event(BackendEvent::RemoteFormatList(available_formats.to_vec()));
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.send_event(BackendEvent::FormatDataRequest(request));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        self.send_event(BackendEvent::FormatDataResponse(response.into_owned()));
    }

    fn on_file_contents_request(&mut self, _request: FileContentsRequest) {
        // File transfer not implemented yet
    }

    fn on_file_contents_response(&mut self, _response: FileContentsResponse<'_>) {
        // File transfer not implemented yet
    }

    fn on_lock(&mut self, _data_id: LockDataId) {
        // File transfer not implemented yet
    }

    fn on_unlock(&mut self, _data_id: LockDataId) {
        // File transfer not implemented yet
    }

    fn on_request_format_list(&mut self) {
        self.send_event(BackendEvent::RemoteRequestsFormatList);
    }
}
