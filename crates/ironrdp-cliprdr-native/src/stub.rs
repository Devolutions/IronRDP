use ironrdp_cliprdr::backend::{CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse, FormatDataRequest,
    FormatDataResponse, LockDataId,
};
use ironrdp_core::impl_as_any;
use tracing::debug;

pub struct StubClipboard;

impl StubClipboard {
    pub fn new() -> Self {
        Self
    }

    pub fn backend_factory(&self) -> Box<dyn CliprdrBackendFactory + Send> {
        Box::new(StubCliprdrBackendFactory {})
    }
}

impl Default for StubClipboard {
    fn default() -> Self {
        Self::new()
    }
}

struct StubCliprdrBackendFactory {}

impl CliprdrBackendFactory for StubCliprdrBackendFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(StubCliprdrBackend::new())
    }
}

#[derive(Debug)]
pub struct StubCliprdrBackend;

impl_as_any!(StubCliprdrBackend);

impl StubCliprdrBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StubCliprdrBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl CliprdrBackend for StubCliprdrBackend {
    fn temporary_directory(&self) -> &str {
        ".cliprdr"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        // No additional capabilities yet
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_process_negotiated_capabilities(&mut self, capabilities: ClipboardGeneralCapabilityFlags) {
        debug!(?capabilities);
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        debug!(?available_formats);
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        debug!(?request);
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        debug!(?response);
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        debug!(?request);
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        debug!(?response);
    }

    fn on_lock(&mut self, data_id: LockDataId) {
        debug!(?data_id);
    }

    fn on_unlock(&mut self, data_id: LockDataId) {
        debug!(?data_id);
    }

    fn on_request_format_list(&mut self) {
        debug!("on_request_format_list");
    }
}
