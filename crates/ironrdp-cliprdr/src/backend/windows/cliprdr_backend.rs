use crate::backend::windows::{BackendEvent, WM_USER_CLIPBOARD};
use crate::backend::CliprdrBackend;
use crate::pdu::{
    ClipboardFormat, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse, FormatDataRequest,
    FormatDataResponse, LockDataId,
};

use winapi::shared::windef::HWND;
use winapi::um::winuser::PostMessageW;

use std::sync::mpsc as mpsc_sync;

#[derive(Debug)]
pub struct WinCliprdrBackend {
    backend_event_tx: mpsc_sync::SyncSender<BackendEvent>,

    /// Window handle.
    ///
    /// NOTE: HWND is non-Send, type, but for our purposes it's safe to send between futures/threads
    window: usize,
}

impl WinCliprdrBackend {
    pub(crate) fn new(window: HWND, backend_event_tx: mpsc_sync::SyncSender<BackendEvent>) -> Self {
        Self {
            window: window as usize,
            backend_event_tx,
        }
    }

    fn send_event(&self, event: BackendEvent) {
        if self.backend_event_tx.send(event).is_err() {
            // Channel is closed, backend is dead
            return;
        }
        // Wake up subproc event loop; Dont wait for result
        unsafe { PostMessageW(self.window as _, WM_USER_CLIPBOARD, 0, 0) };
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

    fn on_receive_downgraded_capabilities(&mut self, capabilities: crate::pdu::ClipboardGeneralCapabilityFlags) {
        self.send_event(BackendEvent::DowngradedCapabilities(capabilities))
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        self.send_event(BackendEvent::RemoteFormatList(available_formats.to_vec()));
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.send_event(BackendEvent::FormatDataRequest(request));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse) {
        self.send_event(BackendEvent::FormatDataResponse(response.into_owned()));
    }

    fn on_file_contents_request(&mut self, _request: FileContentsRequest) {
        // File transfer not implemented yet
    }

    fn on_file_contents_response(&mut self, _response: FileContentsResponse) {
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
