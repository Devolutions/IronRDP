use std::sync::{Arc, mpsc as mpsc_sync};

use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse, FileDescriptor,
    FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp_core::{IntoOwned as _, impl_as_any};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::windows::shared::ClipboardShared;
use crate::windows::{BackendEvent, WM_CLIPRDR_BACKEND_EVENT};

#[derive(Debug)]
pub(crate) struct WinCliprdrBackend {
    backend_event_tx: mpsc_sync::SyncSender<BackendEvent>,
    window: HWND,
    file_transfer: bool,
    shared: Arc<ClipboardShared>,
}

// SAFETY: window handle is thread safe for PostMessageW usage
unsafe impl Send for WinCliprdrBackend {}

impl_as_any!(WinCliprdrBackend);

impl WinCliprdrBackend {
    pub(crate) fn new(
        window: HWND,
        backend_event_tx: mpsc_sync::SyncSender<BackendEvent>,
        file_transfer: bool,
        shared: Arc<ClipboardShared>,
    ) -> Self {
        // A backend is built for every new channel: per-channel state starts over.
        shared.start_session();

        let backend = Self {
            window,
            backend_event_tx,
            file_transfer,
            shared,
        };
        backend.send_event(BackendEvent::SessionStarted);
        backend
    }

    fn send_event(&self, event: BackendEvent) {
        if self.backend_event_tx.send(event).is_err() {
            // Channel is closed, backend is dead
            return;
        }
        // Wake up subproc event loop; Dont wait for result
        //
        // SAFETY: it is safe to call PostMessageW from any thread with a valid window handle
        if let Err(err) = unsafe { PostMessageW(Some(self.window), WM_CLIPRDR_BACKEND_EVENT, WPARAM(0), LPARAM(0)) } {
            tracing::error!("Failed to post message to wake up subproc event loop: {}", err);
        }
    }
}

impl CliprdrBackend for WinCliprdrBackend {
    fn temporary_directory(&self) -> &str {
        ".cliprdr"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        if self.file_transfer {
            // File lists only carry names relative to the copied items, never local paths.
            ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
                | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
                | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
                | ClipboardGeneralCapabilityFlags::HUGE_FILE_SUPPORT_ENABLED
        } else {
            ClipboardGeneralCapabilityFlags::empty()
        }
    }

    fn on_ready(&mut self) {
        self.send_event(BackendEvent::Ready);
    }

    fn on_format_list_response(&mut self, ok: bool) {
        self.send_event(BackendEvent::FormatListResponse(ok));
    }

    fn on_process_negotiated_capabilities(&mut self, capabilities: ClipboardGeneralCapabilityFlags) {
        self.shared.set_capabilities(capabilities);
        self.send_event(BackendEvent::DowngradedCapabilities(capabilities))
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        // Data of the previous remote clipboard stops being fetched right away.
        let generation = self.shared.next_generation();
        self.send_event(BackendEvent::RemoteFormatList(available_formats.to_vec(), generation));
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.send_event(BackendEvent::FormatDataRequest(request));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        self.send_event(BackendEvent::FormatDataResponse(response.into_owned()));
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        self.send_event(BackendEvent::FileContentsRequest(request));
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        // Straight to the stream reading the file, without the clipboard thread.
        self.shared.deliver(response.into_owned());
    }

    fn on_lock(&mut self, data_id: LockDataId) {
        self.send_event(BackendEvent::Lock(data_id));
    }

    fn on_unlock(&mut self, data_id: LockDataId) {
        self.send_event(BackendEvent::Unlock(data_id));
    }

    fn on_remote_file_list(&mut self, files: &[FileDescriptor], clip_data_id: Option<u32>) {
        self.send_event(BackendEvent::RemoteFileList {
            files: files.to_vec(),
            clip_data_id,
        });
    }

    fn on_outgoing_locks_cleared(&mut self, clip_data_ids: &[LockDataId]) {
        // Streams stop sending requests under these locks right away.
        self.shared.release_locks(clip_data_ids.iter().map(|data_id| data_id.0));
    }

    fn on_request_format_list(&mut self) {
        self.send_event(BackendEvent::RemoteRequestsFormatList);
    }

    fn now_ms(&self) -> u64 {
        crate::native_now_ms()
    }

    fn elapsed_ms(&self, since: u64) -> u64 {
        self.now_ms().saturating_sub(since)
    }
}
