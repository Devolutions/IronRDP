//! This module provides infrastructure for implementing OS-specific clipboard backend.

use ironrdp_core::AsAny;

use crate::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse,
    FileDescriptor, FormatDataRequest, FormatDataResponse, LockDataId, OwnedFormatDataResponse,
};

pub trait ClipboardError: core::error::Error + Send + Sync + 'static {}

impl<T> ClipboardError for T where T: core::error::Error + Send + Sync + 'static {}

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

    /// Sent by clipboard backend when file contents are needed from the remote.
    ///
    /// Implementation should send file contents request on `CLIPRDR` SVC when received.
    SendFileContentsRequest(FileContentsRequest),

    /// Sent by clipboard backend when file contents data is ready to be sent to the remote.
    ///
    /// Implementation should send file contents response on `CLIPRDR` SVC when received.
    SendFileContentsResponse(FileContentsResponse<'static>),

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
    fn on_format_data_request(&mut self, request: FormatDataRequest);

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

    /// Processes incoming Lock PDU from the server.
    ///
    /// Called by [crate::Cliprdr] when server requests to lock **client clipboard data**.
    /// This is an incoming lock request - the server wants to prevent the client's clipboard
    /// from changing during file upload operations.
    ///
    fn on_lock(&mut self, data_id: LockDataId);

    /// Processes incoming Unlock PDU from the server.
    ///
    /// Called by [crate::Cliprdr] when server requests to unlock **client clipboard data**.
    /// This is an incoming unlock request - the server is done with the locked clipboard snapshot.
    ///
    fn on_unlock(&mut self, data_id: LockDataId);

    /// [2.2.5.2] Processes remote file list metadata
    ///
    /// Called by [crate::Cliprdr] when file list metadata is received from the remote
    /// in response to a paste request for the FileGroupDescriptorW format (delayed
    /// rendering). The file list is not fetched automatically when a FormatList arrives;
    /// the backend must call [`crate::Cliprdr::initiate_paste`] to request it.
    /// The backend receives file metadata (names, sizes, timestamps) to decide whether
    /// to download files.
    ///
    /// ## Parameters
    ///
    /// - `files`: File metadata received from the remote
    /// - `clip_data_id`: The clipDataId for the lock that was automatically
    ///   created when the Format List was received. `None` if locking was not
    ///   negotiated (CAN_LOCK_CLIPDATA capability absent). Use this ID in
    ///   [`crate::Cliprdr::request_file_contents`] calls to download files against
    ///   the locked clipboard snapshot.
    ///
    /// ## Security Considerations
    ///
    /// **Windows reserved device names**: File names like `CON`, `PRN`, `AUX`, `NUL`,
    /// `COM1`-`COM9`, and `LPT1`-`LPT9` are reserved on Windows. Creating a file with
    /// one of these names opens the corresponding device instead of a regular file,
    /// which can cause hangs or unexpected behavior. Backends writing files to disk
    /// on Windows should use [`crate::is_windows_device_name`] to detect and reject
    /// these names before creating files.
    ///
    /// **File size validation**: The `file_size` field in [`FileDescriptor`] is provided
    /// by the remote peer and should not be trusted for memory allocation decisions.
    /// A malicious remote could advertise an arbitrarily large file size (up to `u64::MAX`)
    /// to cause out-of-memory conditions. Backends should:
    /// - Validate file sizes against available disk space before downloading
    /// - Use streaming writes or chunked downloads rather than pre-allocating buffers
    /// - Consider enforcing maximum file size limits appropriate for their use case
    ///
    /// [2.2.5.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeclip/9c01c966-e09b-438d-9391-ce31f3caddc3
    fn on_remote_file_list(&mut self, files: &[FileDescriptor], clip_data_id: Option<u32>) {
        let _ = (files, clip_data_id);
    }

    /// Called when expired outgoing clipboard locks are cleaned up.
    ///
    /// This is triggered by [`crate::Cliprdr::drive_timeouts`] after a lock has been
    /// in the Expired state and either the inactivity timeout or max lifetime has elapsed.
    /// Locks transition to Expired when a new FormatList PDU is received (clipboard change);
    /// this callback fires later when the cleanup actually removes them.
    ///
    /// **Use case**: Backends can use this to clean up any associated lock state, cancel
    /// ongoing downloads, or update UI to reflect that the lock is no longer valid.
    ///
    /// ## Parameters
    ///
    /// - `clip_data_ids`: List of clipDataIds for locks that were cleaned up.
    ///
    /// **Note**: Unlock PDUs are automatically sent for each cleared lock.
    fn on_outgoing_locks_cleared(&mut self, clip_data_ids: &[LockDataId]) {
        let _ = clip_data_ids;
        // Default implementation does nothing - backends can override to handle lock cleanup
    }

    /// Called when outgoing locks transition from Active to Expired due to a
    /// clipboard change (new FormatList received from remote).
    ///
    /// Expired locks are not yet removed -- they remain in the outgoing locks
    /// map to protect in-flight file downloads. The locks will be cleaned up
    /// later by [`crate::Cliprdr::drive_timeouts`], which triggers
    /// [`CliprdrBackend::on_outgoing_locks_cleared`].
    ///
    /// This callback fires once per clipboard change, with only the lock IDs
    /// that transitioned from Active to Expired during that event.
    fn on_outgoing_locks_expired(&mut self, clip_data_ids: &[LockDataId]) {
        let _ = clip_data_ids;
    }

    /// Returns the current monotonic time in milliseconds.
    ///
    /// Used by [`crate::Cliprdr`] for lock inactivity tracking and cleanup scheduling.
    /// Implementations should return a monotonically non-decreasing value.
    ///
    /// The default implementation uses `std::time::Instant` with a process-local
    /// epoch. Override this for WASM (use `Performance.now()`) or tests (use a
    /// controllable counter for deterministic behavior).
    fn now_ms(&self) -> u64 {
        use std::sync::OnceLock;
        use std::time::Instant;

        static EPOCH: OnceLock<Instant> = OnceLock::new();
        let epoch = EPOCH.get_or_init(Instant::now);

        u64::try_from(epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Returns the elapsed time in milliseconds since the given timestamp.
    ///
    /// `since` is a value previously returned by [`now_ms`](Self::now_ms).
    /// If `since` is in the future (clock skew), implementations should return 0.
    fn elapsed_ms(&self, since: u64) -> u64 {
        self.now_ms().saturating_sub(since)
    }
}

/// Required to build backend for the OS clipboard implementation.
///
/// Factory is required because RDP connection could be re-established multiple times, and `CLIPRDR`
/// channel will be re-initialized each time.
pub trait CliprdrBackendFactory {
    /// Builds new backend instance.
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend>;
}
