#[cfg(windows)]
pub mod windows;

use crate::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse,
    FormatDataRequest, FormatDataResponse, LockDataId,
};

pub trait ClipboardError: std::error::Error + Send + Sync + 'static {}
impl<T> ClipboardError for T where T: std::error::Error + Send + Sync + 'static {}

/// Message received from the OS clipboard backend event loop.
#[derive(Debug)]
pub enum ClipboardMessage {
    /// Sent by clipboard backend when OS clipboard content is changed and ready to be
    /// delay-rendered when needed by the remote.
    ///
    /// Client implementation should initiate copy on `CLIPRDR` SVC when this message is received.
    SendInitiateCopy(Vec<ClipboardFormat>),

    /// Sent by clipboard backend when format data is ready to be sent to the remote.
    ///
    /// Client implementation should send format data to `CLIPRDR` SVC when this message is
    /// received.
    SendFormatData(FormatDataResponse<'static>),

    /// Sent by clipboard backend when format data in given format is need to be received from
    /// the remote.
    ///
    /// Client implementation should send initiate paste on `CLIPRDR` SVC when this message is
    /// received.
    SendInitiatePaste(ClipboardFormatId),

    /// Failure received from the OS clipboard event loop.
    ///
    /// Client implementation should log/display this error.
    Error(Box<dyn ClipboardError>),
}

/// Proxy to send messages from the os clipboard backend to the main application event loop
/// (e.g. winit event loop).
pub trait ClipboardMessageProxy: std::fmt::Debug + Send + Sync {
    fn send_clipboard_message(&self, message: ClipboardMessage);
    fn clone_box(&self) -> Box<dyn ClipboardMessageProxy>;
}

/// OS-specific clipboard backend inteface.
pub trait CliprdrBackend: std::fmt::Debug + Send + Sync + 'static {
    /// Should return path to local temporary directory where clipboard-transfered files should be
    /// stored.
    fn temporary_directory(&self) -> &str;

    /// Should return capabilities of the client. This method is called by `CLIPRDR` when it is
    /// ready to send capabilities to the server. Note that this method by itself does not
    /// trigger any network activity and values are only used during negotiation phase later. Client
    /// should wait for `on_receive_downgraded_capabilities` to be called before using any additional
    /// `CLIPRDR` capabilities.
    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags;

    /// Called by `CLIPRDR` when server requests to start copy sequence. This is usually triggered
    /// during cliprdr initialization to receive list of initially available clipbpard formats.
    fn on_request_format_list(&mut self);

    /// Called by `CLIPRDR` when capability negotiation is finished and server capabilities are
    /// received. This method should be used to decide which capabilities should be used.
    fn on_receive_downgraded_capabilities(&mut self, capabilities: ClipboardGeneralCapabilityFlags);

    /// Called by `CLIPRDR` when server sends list of clipboard formats available in remote's
    /// clipboard.
    ///
    /// Clipboard endpoint implementation should keep track of available formats prior
    /// to requesting data from the server.
    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]);

    /// Called by `CLIPRDR` when server requests data to be copied from the client clipboard.
    ///
    /// This method only signals the client that server requests data in the given format, and
    /// client should respond by calling `sumbit_format_data` on `CLIPRDR`
    fn on_format_data_request(&mut self, format: FormatDataRequest);

    /// Called by `CLIPRDR` when server sends coped to the client clipboard as a response to
    /// previously sent format data request.
    ///
    /// If data is not available anymore, then server will send error response instead.
    fn on_format_data_response(&mut self, response: FormatDataResponse);

    /// Called by `CLIPRDR` when server requests file contents to be copied from the client
    /// clipboard.
    ///
    /// This method only signals the client that server requests specific file contents, and
    /// client should respond by calling `sumbit_file_contents` on `CLIPRDR`
    fn on_file_contents_request(&mut self, request: FileContentsRequest);

    /// Called by `CLIPRDR` when server sends file contents to the client clipboard as a response to
    /// previously sent file contents request.
    ///
    /// If data is not available anymore, then server will send error response instead.
    fn on_file_contents_response(&mut self, response: FileContentsResponse);

    /// Called by `CLIPRDR` when server requests to lock client clipboard.
    fn on_lock(&mut self, data_id: LockDataId);

    /// Called by `CLIPRDR` when server requests to unlock client clipboard.
    fn on_unlock(&mut self, data_id: LockDataId);
}

/// Required to build backend for the OS clipboard implementation.
///
/// Factory is requried because RDP connection could be re-established multiple times, and `CLIPRDR`
/// channel will be re-initialized each time.
pub trait CliprdrBackendFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend>;
}
