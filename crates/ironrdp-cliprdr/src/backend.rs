//! This module provides infrastructure for implementing OS-specific clipboard backend.

use ironrdp_core::AsAny;

use crate::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse,
    FormatDataRequest, FormatDataResponse, LockDataId, OwnedFormatDataResponse,
};

pub trait ClipboardError: std::error::Error + Send + Sync + 'static {}

impl<T> ClipboardError for T where T: std::error::Error + Send + Sync + 'static {}

/// Message sent by the OS clipboard backend event loop.
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
    SendFormatData(OwnedFormatDataResponse),

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
pub trait ClipboardMessageProxy: core::fmt::Debug + Send {
    fn send_clipboard_message(&self, message: ClipboardMessage);
}

/// OS-specific clipboard backend interface.
pub trait CliprdrBackend: AsAny + core::fmt::Debug + Send {
    /// Returns path to local temporary directory where clipboard-transferred files should be
    /// stored.
    fn temporary_directory(&self) -> &str;

    /// Returns capabilities of the client.
    ///
    /// This method is called by [crate::Cliprdr] when it is
    /// ready to send capabilities to the server. Note that this method by itself does not
    /// trigger any network activity and values are only used during negotiation phase later. Client
    /// should wait for `on_process_negotiated_capabilities` to be called before using any additional
    /// [crate::Cliprdr] capabilities.
    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags;

    /// Called by [crate::Cliprdr] when it is ready to process clipboard data (channel initialized)
    fn on_ready(&mut self);

    /// Processes signal to start clipboard copy sequence.
    ///
    /// Trait implementer is responsible for gathering its list of available [`ClipboardFormat`]
    /// and passing them into [`crate::Cliprdr`]'s `initiate_copy` method.
    ///
    /// Called by [crate::Cliprdr] during initialization phase as a request to start copy
    /// sequence on the client. This is needed to advertise available formats on the
    /// client's clipboard prior to `CLIPRDR` SVC initialization.
    fn on_request_format_list(&mut self);

    /// Called by [crate::Cliprdr] when copy sequence is finished.
    /// This method is called after remote returns format list response.
    ///
    /// Useful for the backend implementations which need to know when remote is ready to paste
    /// previously advertised formats from the client. E.g. Web client uses this for
    /// Firefox-specific logic to delay sending keyboard key events to prevent pasting the old
    /// data from the clipboard.
    ///
    /// This method has default implementation which does nothing because it is not required for
    /// most of the backends.
    fn on_format_list_received(&mut self) {}

    /// Adjusts [crate::Cliprdr] backend capabilities based on capabilities negotiated with a server.
    ///
    /// Called by [crate::Cliprdr] when capability negotiation is finished and server capabilities are
    /// received. This method should be used to decide which capabilities should be used by the client.
    fn on_process_negotiated_capabilities(&mut self, capabilities: ClipboardGeneralCapabilityFlags);

    /// Processes remote clipboard format list.
    ///
    /// Called by [crate::Cliprdr] when server sends list of clipboard formats available in remote's
    /// clipboard (whenever a cut/copy is executed on remote).
    ///
    /// Trait implementer should keep track of the latest available formats sent to it through
    /// this method. These are needed to be passed in to [`crate::Cliprdr::initiate_paste`]
    /// when a user initiates a paste operation of remote data to the local machine.
    ///
    /// Clipboard endpoint implementation should keep track of available formats prior
    /// to requesting data from the server.
    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]);

    /// Processes remote's request to send format data.
    ///
    /// Called by [crate::Cliprdr] when server requests data to be copied from the client clipboard.
    ///
    /// This method only signals the client that server requests data in the given format.
    /// Implementors should respond by compiling a [`FormatDataResponse`] and calling
    /// [`crate::Cliprdr::submit_format_data`]
    fn on_format_data_request(&mut self, format: FormatDataRequest);

    /// Called by [`crate::Cliprdr`] when server sends data to the client clipboard as a response to
    /// previously sent format data request.
    ///
    /// If data is not available anymore, [`FormatDataResponse`] will have its `is_error` field
    /// set to `true`.
    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>);

    /// Processes remote's request to send file contents.
    ///
    /// Called by [crate::Cliprdr] when server requests file contents to be copied from the client
    /// clipboard.
    ///
    /// This method only signals the client that server requests specific file contents, and
    /// client should respond by calling `submit_file_contents` on [crate::Cliprdr]
    fn on_file_contents_request(&mut self, request: FileContentsRequest);

    /// Processes remote's response to previously sent file contents request.
    ///
    /// Called by [crate::Cliprdr] when server sends file contents to the client clipboard as a response to
    /// previously sent file contents request.
    ///
    /// If data is not available anymore, then server will send error response instead.
    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>);

    /// Locks specific data stream in the client clipboard.
    ///
    /// Called by [crate::Cliprdr] when server requests to lock client clipboard.
    fn on_lock(&mut self, data_id: LockDataId);

    /// Unlocks specific data stream in the client clipboard.
    ///
    /// Called by [crate::Cliprdr] when server requests to unlock client clipboard.
    fn on_unlock(&mut self, data_id: LockDataId);
}

/// Required to build backend for the OS clipboard implementation.
///
/// Factory is required because RDP connection could be re-established multiple times, and `CLIPRDR`
/// channel will be re-initialized each time.
pub trait CliprdrBackendFactory {
    /// Builds new backend instance.
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend>;
}
