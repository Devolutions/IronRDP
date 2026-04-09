#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

pub mod backend;
pub mod pdu;

use std::collections::HashMap;

use backend::CliprdrBackend;
use ironrdp_core::{AsAny, EncodeResult, IntoOwned as _, decode};
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::{PduResult, decode_err, encode_err};
use ironrdp_svc::{
    ChannelFlags, CompressionCondition, SvcClientProcessor, SvcMessage, SvcProcessor, SvcProcessorMessages,
    SvcServerProcessor,
};
use pdu::{
    Capabilities, ClientTemporaryDirectory, ClipboardFormat, ClipboardFormatId, ClipboardFormatName,
    ClipboardGeneralCapabilityFlags, ClipboardPdu, ClipboardProtocolVersion, FileContentsFlags, FileContentsRequest,
    FileContentsResponse, FileDescriptor, FormatDataRequest, FormatListResponse, LockDataId, OwnedFormatDataResponse,
    PackedFileList,
};
use tracing::{debug, error, info, trace, warn};

#[rustfmt::skip] // do not reorder
use crate::pdu::FormatList;

/// PDUs for sending to the server on the CLIPRDR channel.
pub type CliprdrSvcMessages<R> = SvcProcessorMessages<Cliprdr<R>>;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "__test", visibility::make(pub))]
pub(crate) enum CliprdrState {
    Initialization,
    Ready,
}

/// [MS-RDPECLIP] 2.2.5.3 / 2.2.5.4 - Tracks state of a file contents transfer
///
/// Used to validate FileContentsResponse matches the corresponding FileContentsRequest
/// and to support concurrent transfers identified by streamId.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "__test", visibility::make(pub))]
pub(crate) struct FileTransferState {
    /// File index (lindex) from the file list.
    /// Validated non-negative and in-bounds at parse time (Parse Don't Validate).
    /// Wire format is i32 per [MS-RDPECLIP] 2.2.5.3, but stored as usize after validation.
    pub file_index: usize,
    /// Flags from the request (SIZE or RANGE)
    /// Used for SIZE/RANGE response validation
    pub flags: FileContentsFlags,
    /// When this request was sent (milliseconds from backend clock).
    /// Used for stale request cleanup.
    pub sent_at_ms: u64,
}

/// State of a clipboard lock
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "__test", visibility::make(pub))]
pub(crate) enum LockState {
    /// Lock is active and protecting clipboard data
    Active,

    /// Lock has expired (clipboard changed) but may still be in use
    /// Will be cleaned up based on activity and time rules
    Expired {
        /// When this lock expired (clipboard changed), in milliseconds
        /// from the backend clock.
        expired_at_ms: u64,
    },
}

/// [MS-RDPECLIP] 2.2.4 - Outgoing clipboard lock state tracking
///
/// Tracks state of a clipboard lock with activity-based timeout.
/// Used to manage multiple concurrent file download operations.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "__test", visibility::make(pub))]
pub(crate) struct OutgoingLock {
    /// Current state of this lock
    pub state: LockState,

    /// When this lock was created (milliseconds from backend clock),
    /// used for max_lifetime enforcement.
    pub created_at_ms: u64,

    /// Last time this lock was used for a FileContentsRequest
    /// (milliseconds from backend clock).
    /// Updated to prevent timeout during active transfers.
    pub last_used_at_ms: u64,
}

/// Detects if a filename contains an absolute path.
///
/// Per [MS-RDPECLIP] 3.1.1.3, when CB_FILECLIP_NO_FILE_PATHS is set, filenames
/// MUST NOT include source paths. This function detects absolute paths across
/// different OS path conventions:
///
/// - Unix absolute: `/path/to/file`
/// - Windows absolute: `C:\path\to\file` or `C:/path/to/file`
/// - Windows drive-relative: `C:relative` (references specific drive)
/// - UNC paths: `\\server\share\file`
/// - Long UNC paths: `\\?\UNC\server\share\file`
/// - Long path prefix: `\\?\C:\very\long\path`
///
/// Returns false for relative paths like `file.txt` or `subfolder/file.txt`.
#[cfg_attr(feature = "__test", visibility::make(pub))]
pub(crate) fn is_absolute_path(filename: &str) -> bool {
    // Unix absolute path (including root)
    if filename.starts_with('/') {
        return true;
    }

    // Windows long path prefix: \\?\ or \\.\
    // Catches: \\?\C:\path, \\?\UNC\server\share, \\.\device
    if filename.starts_with("\\\\?\\") || filename.starts_with("\\\\.\\") {
        return true;
    }

    // UNC path: \\server\share or //server/share
    if filename.starts_with("\\\\") || filename.starts_with("//") {
        return true;
    }

    // Windows absolute path: C:\ or C:/
    // Also Windows drive-relative: C:relative (no separator after colon)
    // Both reveal drive information and should be blocked
    if filename.len() >= 2 {
        let mut chars = filename.chars();
        if let (Some(first), Some(second)) = (chars.next(), chars.next()) {
            if first.is_ascii_alphabetic() && second == ':' {
                return true;
            }
        }
    }

    false
}

/// Checks whether a filename is a Windows reserved device name.
///
/// On Windows, creating a file named `CON`, `PRN`, `AUX`, `NUL`,
/// `COM1`-`COM9`, or `LPT1`-`LPT9` opens the corresponding device
/// instead of a regular file. This is true even with an extension
/// (e.g. `CON.txt` opens the console device).
///
/// The check is case-insensitive and also matches names with extensions
/// (the stem before the first `.` is checked).
///
/// Backends that write received files to disk on Windows should call
/// this function and reject matching names in their
/// [`CliprdrBackend::on_remote_file_list`](crate::backend::CliprdrBackend::on_remote_file_list)
/// implementation.
///
/// # Example
///
/// ```
/// use ironrdp_cliprdr::is_windows_device_name;
///
/// assert!(is_windows_device_name("CON"));
/// assert!(is_windows_device_name("con.txt"));
/// assert!(is_windows_device_name("NUL"));
/// assert!(is_windows_device_name("LPT1.doc"));
/// assert!(!is_windows_device_name("document.txt"));
/// assert!(!is_windows_device_name("console.log"));
/// ```
pub fn is_windows_device_name(filename: &str) -> bool {
    // Extract the stem (part before the first dot) for comparison.
    // "CON.txt" -> "CON", "NUL" -> "NUL"
    let stem = filename.split('.').next().unwrap_or("");

    const DEVICE_NAMES: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
        "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    DEVICE_NAMES.iter().any(|name| stem.eq_ignore_ascii_case(name))
}

/// Result of sanitizing a file path from a remote peer.
#[cfg_attr(feature = "__test", visibility::make(pub))]
pub(crate) struct SanitizedPath {
    /// The file basename (e.g., `"file.txt"`).
    pub name: String,
    /// The relative directory path (e.g., `"temp\\subdir"`), or `None` for
    /// root-level files. Uses `\` as the separator to match the Windows wire
    /// convention.
    pub relative_path: Option<String>,
}

/// Sanitizes a file path received from a remote peer.
///
/// A malicious remote could send paths containing traversal sequences
/// (e.g. `../../../etc/cron.d/backdoor`) or absolute paths. This function:
///
/// 1. Strips absolute path prefixes (drive letters, UNC, `/`-rooted)
/// 2. Removes `.` and `..` traversal components
/// 3. Preserves safe relative directory components
/// 4. Extracts the basename
///
/// Returns `None` if the path is empty or consists entirely of path
/// separators, traversal components, or null bytes.
///
/// # Examples
///
/// - `"temp\\file.txt"` -> `SanitizedPath { name: "file.txt", relative_path: Some("temp") }`
/// - `"folder\\sub\\file.txt"` -> `SanitizedPath { name: "file.txt", relative_path: Some("folder\\sub") }`
/// - `"C:\\Users\\victim\\Desktop\\file.txt"` -> `SanitizedPath { name: "file.txt", relative_path: Some("Users\\victim\\Desktop") }`
/// - `"../../../etc/passwd"` -> `SanitizedPath { name: "passwd", relative_path: Some("etc") }`
/// - `"file.txt"` -> `SanitizedPath { name: "file.txt", relative_path: None }`
///
/// # Limitations
///
/// This function only splits on ASCII path separators (`/` U+002F and `\`
/// U+005C). Unicode look-alikes such as fullwidth solidus (U+FF0F),
/// fullwidth reverse solidus (U+FF3C), or division slash (U+2215) are
/// **not** treated as separators. Some operating systems may normalize
/// these characters to their ASCII equivalents when creating files.
///
/// Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`,
/// `COM1`-`COM9`, `LPT1`-`LPT9`) are **not** rejected by this function.
/// On Windows, creating a file with one of these names accesses the
/// corresponding device rather than a regular file. Backends that write
/// files to disk on Windows **must** check for and reject these names
/// before creating files. The [`is_windows_device_name`] helper can be
/// used for this check.
#[cfg_attr(feature = "__test", visibility::make(pub))]
pub(crate) fn sanitize_file_path(filename: &str) -> Option<SanitizedPath> {
    // Strip trailing null bytes: CLIPRDR file descriptors use null-terminated
    // strings padded with nulls, so the filename may contain trailing \0 chars.
    let filename = filename.trim_end_matches('\0');

    // Reject filenames with embedded null bytes. These have no legitimate use
    // and could cause truncation when passed to C-based filesystem APIs (the
    // OS would treat \0 as a string terminator, silently shortening the name).
    if filename.contains('\0') {
        return None;
    }

    // Fast path: no separators (the common case for flat file lists).
    // Avoids Vec allocation and join when the name has no path components.
    if !filename.contains('/') && !filename.contains('\\') {
        if filename.is_empty() || filename == "." || filename == ".." {
            return None;
        }
        return Some(SanitizedPath {
            name: filename.to_owned(),
            relative_path: None,
        });
    }

    // Split on both Windows and Unix path separators, filter out empty
    // components and traversal sequences, keeping only safe components.
    let safe_components: Vec<&str> = filename
        .split(['/', '\\'])
        .filter(|c| !c.is_empty() && *c != "." && *c != "..")
        .collect();

    if safe_components.is_empty() {
        return None;
    }

    // Strip absolute path prefix: if the first component looks like a drive
    // letter (e.g., "C:"), a UNC host, or a Windows long-path prefix, discard
    // all leading components that are part of the absolute prefix.
    let components = strip_absolute_prefix(&safe_components);

    if components.is_empty() {
        return None;
    }

    // Last component is the basename; everything before is the relative path.
    let (dir_parts, basename) = components.split_at(components.len() - 1);
    let name = (*basename.first()?).to_owned();

    // Reject if the basename is empty or a traversal component (should not
    // happen after filtering, but defense in depth)
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }

    let relative_path = if dir_parts.is_empty() {
        None
    } else {
        Some(dir_parts.join("\\"))
    };

    Some(SanitizedPath { name, relative_path })
}

/// Strips absolute path prefixes from the component list.
///
/// Handles:
/// - Windows drive letters: `["C:", "Users", ...]` -> `["Users", ...]`
/// - UNC paths: `["server", "share", ...]` after split -> keep from share onward
/// - Long path prefixes: `["?", "C:", ...]` or `[".", "device", ...]`
///
/// If the original path was absolute, we strip the prefix and return the
/// remaining relative portion. If it was already relative, returns as-is.
///
/// Returns a sub-slice of the input to avoid allocation.
fn strip_absolute_prefix<'a>(components: &'a [&str]) -> &'a [&'a str] {
    if components.is_empty() {
        return &[];
    }

    let first = components[0];

    // Check for Windows drive letter prefix (e.g., "C:")
    if first.len() == 2 && first.as_bytes()[0].is_ascii_alphabetic() && first.as_bytes()[1] == b':' {
        // Absolute path like C:\Users\...; drop the drive letter.
        return &components[1..];
    }

    // Detect "?" or "." as first component which indicates \\?\ or \\.\
    // long path prefixes.
    if first == "?" || (first == "." && components.len() > 1) {
        // Long path prefix: \\?\C:\path or \\.\device\path
        // Skip the prefix marker and any following drive letter.
        let rest = &components[1..];
        if let Some(second) = rest.first() {
            if second.len() == 2 && second.as_bytes()[0].is_ascii_alphabetic() && second.as_bytes()[1] == b':' {
                return &rest[1..];
            }
        }
        return rest;
    }

    // No absolute prefix detected; return as-is.
    components
}

/// Marker trait distinguishing client and server roles for the CLIPRDR channel.
///
/// The clipboard channel has symmetric message processing with role-specific
/// differences during initialization: the server transitions to Ready on
/// receiving the first Format List, while the client transitions on receiving
/// Format List Response (per [MS-RDPECLIP] 1.3.2.1).
pub trait Role: core::fmt::Debug + Send + 'static {
    /// Returns `true` if this role is the server side of the CLIPRDR channel.
    fn is_server() -> bool;
}

/// Maximum number of incoming clipboard locks (remote lock requests) to track.
///
/// This limit prevents malicious remotes from exhausting memory by sending
/// unlimited Lock PDUs without corresponding Unlock PDUs.
///
/// Per MS-RDPECLIP 3.1.5.3.2, clipboard changes should auto-unlock, but
/// malicious or buggy implementations may not comply. This limit provides
/// defense-in-depth protection.
const MAX_LOCKED_FILE_LISTS: usize = 100;

/// Maximum number of outgoing clipboard locks we will create.
///
/// Unlike incoming locks, outgoing locks are created by the local side
/// (one per file download). This limit prevents unbounded
/// growth of `outgoing_locks` if the application repeatedly creates locks
/// without unlocking them.
const MAX_OUTGOING_LOCKS: usize = 100;

/// Maximum number of pending (unanswered) file contents requests.
///
/// Each call to [`Cliprdr::request_file_contents`] inserts an entry that is
/// removed when the corresponding [`FileContentsResponse`] arrives. This limit
/// prevents unbounded growth if responses are never received.
const MAX_PENDING_FILE_REQUESTS: usize = 1000;

/// CLIPRDR static virtual channel endpoint implementation
#[derive(Debug)]
pub struct Cliprdr<R: Role> {
    backend: Box<dyn CliprdrBackend>,
    capabilities: Capabilities,
    state: CliprdrState,

    /// Tracks the format ID of the most recently sent FormatDataRequest.
    /// Used to correlate FormatDataResponse with the request that produced it,
    /// so we only intercept responses for the file list format and forward all
    /// others to the backend.
    pending_format_data_request: Option<ClipboardFormatId>,

    /// Stores the local file list when initiating a file copy operation.
    /// Set by initiate_file_copy(), used to respond to FormatDataRequest.
    local_file_list: Option<PackedFileList>,

    /// Format ID used for local FileGroupDescriptorW in the FormatList we sent.
    /// Tracked so we can recognize FormatDataRequest for our file list.
    local_file_list_format_id: Option<ClipboardFormatId>,

    /// Stores the remote file list after receiving it via FormatDataResponse.
    /// Used for validating FileContentsRequest.lindex bounds.
    remote_file_list: Option<PackedFileList>,

    /// Format ID used by remote for FileGroupDescriptorW in FormatList they sent.
    /// Detected by finding format with name "FileGroupDescriptorW".
    remote_file_list_format_id: Option<ClipboardFormatId>,

    /// [MS-RDPECLIP] 2.2.5.3 - Tracks FileContentsRequest PDUs we've sent (client → server downloads)
    /// Maps streamId → FileTransferState to validate incoming FileContentsResponse PDUs.
    /// Supports concurrent transfers with different streamIds.
    sent_file_contents_requests: HashMap<u32, FileTransferState>,

    /// [MS-RDPECLIP] 2.2.4 - Outgoing clipboard lock tracking
    /// Maps clipDataId → OutgoingLock for all active locks we've sent to remote.
    /// When we lock clipboard for file download, we generate a clipDataId and store lock state here.
    /// This enables multiple concurrent file downloads, each with independent clipDataId.
    outgoing_locks: HashMap<u32, OutgoingLock>,

    /// The most recently created lock's clipDataId.
    /// Used as the default clipDataId for new FileContentsRequest calls.
    /// Set when a lock is created on FormatList, cleared when last lock is removed.
    current_lock_id: Option<u32>,

    /// Counter for generating unique clipDataId values for lock operations.
    /// Incremented each time a new lock is needed. Zero is avoided per convention.
    next_clip_data_id: u32,

    /// Timeout for inactive expired locks (no FileContentsRequest)
    /// Default: 60 seconds
    lock_inactivity_timeout: core::time::Duration,

    /// Maximum lifetime for expired locks regardless of activity
    /// Prevents resource leaks from stalled transfers
    /// Default: 2 hours
    lock_max_lifetime: core::time::Duration,

    /// [MS-RDPECLIP] 3.1.5.3.2 - Locked file list snapshots (incoming from remote)
    /// Maps clipDataId -> PackedFileList for servicing FileContentsRequest with clipDataId.
    /// When a Lock PDU is received, we snapshot the current local_file_list and store it here.
    /// This allows us to serve file requests even after the clipboard changes.
    locked_file_lists: HashMap<u32, PackedFileList>,

    /// Tracks last FileContentsRequest activity per locked file list.
    /// Maps clipDataId -> last activity timestamp (ms from backend clock).
    /// Initialized when a Lock PDU is received; updated on each incoming
    /// FileContentsRequest that references the clipDataId. Used by the
    /// cleanup sweep to detect abandoned uploads.
    locked_file_list_activity: HashMap<u32, u64>,

    /// Timeout for individual file contents requests.
    /// Requests older than this are removed and the backend receives a
    /// synthetic error response. Default: 60 seconds.
    transfer_timeout: core::time::Duration,

    _marker: core::marker::PhantomData<R>,
}

pub type CliprdrClient = Cliprdr<Client>;
pub type CliprdrServer = Cliprdr<Server>;

impl SvcClientProcessor for CliprdrClient {}
impl SvcServerProcessor for CliprdrServer {}

impl<R: Role> AsAny for Cliprdr<R> {
    #[inline]
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    #[inline]
    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

impl<R: Role> Cliprdr<R> {
    const CHANNEL_NAME: ChannelName = ChannelName::from_static(b"cliprdr\0");

    /// Creates new CLIPRDR processor with default timeout policy
    ///
    /// Defaults:
    /// - Inactivity timeout: 60 seconds (cleanup expired locks with no FileContentsRequest activity)
    /// - Max lifetime: 2 hours (force cleanup regardless of activity)
    /// - Transfer timeout: 60 seconds (individual file contents request timeout)
    ///
    /// Callers must drive [`Self::drive_timeouts()`] from a periodic timer (e.g., every 5 seconds)
    /// to process expired locks and stale transfers.
    pub fn new(backend: Box<dyn CliprdrBackend>) -> Self {
        Self::with_all_config(
            backend,
            core::time::Duration::from_secs(60),       // inactivity_timeout
            core::time::Duration::from_secs(2 * 3600), // max_lifetime (2 hours)
            core::time::Duration::from_secs(60),       // transfer_timeout
        )
    }

    /// Creates new CLIPRDR processor with custom lock timeout policy
    ///
    /// ## Parameters
    /// - `inactivity_timeout`: Cleanup expired locks with no FileContentsRequest for this duration
    /// - `max_lifetime`: Force cleanup after this duration regardless of activity
    ///
    /// Callers must drive [`Self::drive_timeouts()`] from a periodic timer (e.g., every 5 seconds).
    pub fn with_lock_timeouts(
        backend: Box<dyn CliprdrBackend>,
        inactivity_timeout: core::time::Duration,
        max_lifetime: core::time::Duration,
    ) -> Self {
        Self::with_all_config(
            backend,
            inactivity_timeout,
            max_lifetime,
            core::time::Duration::from_secs(60), // transfer_timeout
        )
    }

    /// Creates new CLIPRDR processor with full configuration control
    ///
    /// ## Parameters
    /// - `inactivity_timeout`: Cleanup expired locks with no FileContentsRequest for this duration
    /// - `max_lifetime`: Force cleanup after this duration regardless of activity
    /// - `transfer_timeout`: Timeout for individual file contents requests and upload inactivity
    ///
    /// Callers must drive [`Self::drive_timeouts()`] from a periodic timer (e.g., every 5 seconds).
    pub fn with_all_config(
        backend: Box<dyn CliprdrBackend>,
        inactivity_timeout: core::time::Duration,
        max_lifetime: core::time::Duration,
        transfer_timeout: core::time::Duration,
    ) -> Self {
        // This CLIPRDR implementation supports long format names by default
        let flags = ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES | backend.client_capabilities();

        Self {
            backend,
            state: CliprdrState::Initialization,
            capabilities: Capabilities::new(ClipboardProtocolVersion::V2, flags),
            pending_format_data_request: None,
            local_file_list: None,
            local_file_list_format_id: None,
            remote_file_list: None,
            remote_file_list_format_id: None,
            sent_file_contents_requests: HashMap::new(),
            outgoing_locks: HashMap::new(),
            current_lock_id: None,
            next_clip_data_id: 1, // Start at 1, avoiding 0
            lock_inactivity_timeout: inactivity_timeout,
            lock_max_lifetime: max_lifetime,
            locked_file_lists: HashMap::new(),
            locked_file_list_activity: HashMap::new(),
            transfer_timeout,
            _marker: core::marker::PhantomData,
        }
    }

    pub fn downcast_backend<T: CliprdrBackend>(&self) -> Option<&T> {
        self.backend.as_any().downcast_ref::<T>()
    }

    pub fn downcast_backend_mut<T: CliprdrBackend>(&mut self) -> Option<&mut T> {
        self.backend.as_any_mut().downcast_mut::<T>()
    }

    fn are_long_format_names_enabled(&self) -> bool {
        self.capabilities
            .flags()
            .contains(ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES)
    }

    fn build_format_list(&self, formats: &[ClipboardFormat]) -> EncodeResult<FormatList<'static>> {
        FormatList::new_unicode(formats, self.are_long_format_names_enabled())
    }

    /// Returns an error if the clipboard channel is not in Ready state.
    fn require_ready(&self, method: &'static str) -> PduResult<()> {
        if self.state != CliprdrState::Ready {
            return Err(ironrdp_pdu::PduError::new(
                method,
                ironrdp_pdu::PduErrorKind::Other {
                    description: "clipboard channel is not in Ready state",
                },
            ));
        }
        Ok(())
    }

    fn handle_server_capabilities(&mut self, server_capabilities: Capabilities) -> PduResult<Vec<SvcMessage>> {
        self.capabilities.downgrade(&server_capabilities);
        self.backend
            .on_process_negotiated_capabilities(self.capabilities.flags());

        // Do not send anything, wait for monitor ready pdu
        Ok(Vec::new())
    }

    fn handle_monitor_ready(&mut self) -> PduResult<Vec<SvcMessage>> {
        // [MS-RDPECLIP] 3.2.5.1 - Initialization Sequence (client side)
        //
        // The spec requires the client to send its Capabilities PDU after
        // receiving Monitor Ready. We achieve this by asking the backend
        // for its initial format list here. The backend responds
        // asynchronously (e.g. via ClipboardMessage::SendInitiateCopy),
        // which eventually calls `initiate_copy()`. In Initialization
        // state, `initiate_copy()` bundles Capabilities + Temporary
        // Directory + Format List into a single batch.
        //
        // INVARIANT (ordering): `handle_server_capabilities()` runs
        // synchronously during `process()` for the Capabilities PDU,
        // which always precedes the Monitor Ready PDU in the server's
        // initialization sequence. By the time any backend callback
        // triggers `initiate_copy()`, `self.capabilities` has already
        // been downgraded against the server's capabilities. This holds
        // regardless of whether the backend responds synchronously or
        // asynchronously, because `process()` for the Capabilities PDU
        // completes before `process()` for Monitor Ready begins.
        self.backend.on_request_format_list();
        Ok(Vec::new())
    }

    fn handle_format_list_response(&mut self, response: FormatListResponse) -> PduResult<Vec<SvcMessage>> {
        match response {
            FormatListResponse::Ok => {
                if !R::is_server() {
                    if self.state == CliprdrState::Initialization {
                        info!("Clipboard virtual channel initialized");
                        self.state = CliprdrState::Ready;
                        self.backend.on_ready();
                    } else {
                        info!("Remote accepted format list");
                    }
                }
            }
            FormatListResponse::Fail => {
                // [MS-RDPECLIP] 3.1.5.2.4 - The remote rejected our FormatList but the
                // channel remains operational. Clear local state so we don't serve stale
                // data, but stay in Ready state to allow subsequent clipboard operations.
                warn!("Remote rejected our format list, clearing local clipboard state");

                self.local_file_list = None;
                self.local_file_list_format_id = None;

                if !self.sent_file_contents_requests.is_empty() {
                    info!(
                        count = self.sent_file_contents_requests.len(),
                        "Clearing pending file contents requests due to FormatListResponse::Fail"
                    );

                    // Notify backend for each pending request so it can clean up
                    // (e.g. reject pending download promises in WASM).
                    let stream_ids: Vec<u32> = self.sent_file_contents_requests.keys().copied().collect();
                    for stream_id in stream_ids {
                        self.backend
                            .on_file_contents_response(FileContentsResponse::new_error(stream_id));
                    }

                    self.sent_file_contents_requests.clear();
                }
            }
        }

        Ok(Vec::new())
    }

    fn handle_format_list(&mut self, format_list: FormatList<'_>) -> PduResult<Vec<SvcMessage>> {
        if R::is_server() && self.state == CliprdrState::Initialization {
            info!("Clipboard virtual channel initialized");
            self.state = CliprdrState::Ready;
            self.backend.on_ready();
        }

        // Clear any previous remote clipboard state since new content is available
        self.remote_file_list = None;
        self.remote_file_list_format_id = None;
        self.pending_format_data_request = None;

        // [MS-RDPECLIP] 2.2.4.2 - Expire locks when clipboard changes
        // Locks enter grace period with activity-based timeout
        if !self.outgoing_locks.is_empty() {
            let active_locks_count = self
                .outgoing_locks
                .values()
                .filter(|lock| matches!(lock.state, LockState::Active))
                .count();
            if active_locks_count > 0 {
                debug!(
                    count = active_locks_count,
                    "Expiring active locks due to clipboard change (new FormatList received)"
                );
            }
        }
        // Transition active locks to Expired — but do NOT send Unlock PDUs yet.
        // Active downloads from the previous clipboard may still be using these locks.
        // The cleanup timer will send Unlock only after inactivity timeout.
        self.expire_all_locks();
        let mut messages = Vec::new();

        // Do NOT clear sent_file_contents_requests here.
        //
        // Per [MS-RDPECLIP] 2.2.4.1 and 3.1.5.3.2, clipboard locks ensure that
        // file stream data is retained by the server even after the clipboard
        // changes. The spec says: "The purpose of this PDU is to request that
        // the Shared Clipboard Owner retain all File Stream data [...] even when
        // the Shared Owner clipboard has changed and the File Stream data is no
        // longer available."
        //
        // expire_all_locks() above correctly transitions locks to Expired with a
        // grace period so in-flight downloads can finish. But if we cleared the
        // request tracking here, valid FileContentsResponse PDUs from the server
        // (serviced from locked data) would be dropped as "unknown streamId",
        // silently breaking downloads that should succeed.
        //
        // Each entry is removed individually when its response arrives (line that
        // calls sent_file_contents_requests.remove(&stream_id) in the
        // FileContentsResponse handler).
        //
        // Note: FormatListResponse::Fail also clears sent_file_contents_requests
        // because the remote rejected our clipboard and no further responses for
        // those requests will arrive.

        let formats = format_list.get_formats(self.are_long_format_names_enabled())?;

        // Notify backend of available formats
        self.backend.on_remote_copy(&formats);

        // [MS-RDPECLIP] 1.3.1.2 - Detect if FormatList contains FileGroupDescriptorW by name.
        // [MS-RDPECLIP] 1.3.2.2.3 - "The Local Clipboard Owner first requests the list of files
        // available from the clipboard." The word "first" refers to ordering within the paste
        // sequence itself (file list before file contents), NOT immediately after FormatList receipt.
        // Per spec section 1.3.1.4 "Delayed Rendering", the file list is requested only when the
        // user initiates a paste operation. The format ID is stored and the file list is requested
        // only when the paste operation occurs.
        let file_list_format = formats.iter().find(|fmt| {
            fmt.name
                .as_ref()
                .map(|n| n.value() == ClipboardFormatName::FILE_LIST.value())
                .unwrap_or(false)
        });

        if let Some(format) = file_list_format {
            // Store the format ID for later use when user initiates paste
            self.remote_file_list_format_id = Some(format.id);
            info!(format_id = ?format.id, "FileGroupDescriptorW format available in FormatList");
        }

        // [MS-RDPECLIP] 3.1.5.2.2 - Acknowledge the FormatList before any
        // further PDUs. The FormatListResponse logically completes the copy
        // sequence; sending Lock before it is technically permitted by the
        // spec but unusual. FreeRDP sends FormatListResponse first as well.
        messages.push(into_cliprdr_message(ClipboardPdu::FormatListResponse(
            FormatListResponse::Ok,
        )));

        // [MS-RDPECLIP] 2.2.4.1 / Figure 3 - Automatically lock remote clipboard
        // when file data is detected. Sent after FormatListResponse to complete
        // the copy sequence first.
        if file_list_format.is_some() {
            if let Some(lock_messages) = self.send_lock() {
                messages.extend(lock_messages);
            }
        }

        Ok(messages)
    }

    /// Submits the format data response, returning a [`CliprdrSvcMessages`] to send on the channel.
    ///
    /// Should be called by the clipboard implementation when it receives data from the OS clipboard
    /// and is ready to send it to the server. This should happen after
    /// [`CliprdrBackend::on_format_data_request`] is called by [`Cliprdr`].
    ///
    /// If data is not available anymore, an error response should be sent instead.
    pub fn submit_format_data(&self, response: OwnedFormatDataResponse) -> PduResult<CliprdrSvcMessages<R>> {
        self.require_ready("submit_format_data")?;

        let pdu = ClipboardPdu::FormatDataResponse(response);

        Ok(vec![into_cliprdr_message(pdu)].into())
    }

    /// Submits the file contents response, returning a [`CliprdrSvcMessages`] to send on the channel.
    ///
    /// Should be called by the clipboard implementation when file data is ready to send it to the
    /// server. This should happen after [`CliprdrBackend::on_file_contents_request`] is called
    /// by [`Cliprdr`].
    ///
    /// If data is not available anymore, an error response should be sent instead.
    pub fn submit_file_contents(&self, response: FileContentsResponse<'static>) -> PduResult<CliprdrSvcMessages<R>> {
        self.require_ready("submit_file_contents")?;

        let pdu = ClipboardPdu::FileContentsResponse(response);

        Ok(vec![into_cliprdr_message(pdu)].into())
    }

    pub fn capabilities(&self) -> PduResult<SvcMessage> {
        let pdu = ClipboardPdu::Capabilities(self.capabilities.clone());

        Ok(into_cliprdr_message(pdu))
    }

    pub fn monitor_ready(&self) -> PduResult<SvcMessage> {
        let pdu = ClipboardPdu::MonitorReady;

        Ok(into_cliprdr_message(pdu))
    }

    /// Starts processing of `CLIPRDR` copy command. Should be called by the clipboard
    /// implementation when user performs OS-specific copy command (e.g. `Ctrl+C` shortcut on
    /// keyboard)
    ///
    /// Note: For file copies, use `initiate_file_copy()` instead.
    ///
    /// Takes `&mut self` because it manages `local_file_list` state on each copy,
    /// not just PDU encoding.
    pub fn initiate_copy(&mut self, available_formats: &[ClipboardFormat]) -> PduResult<CliprdrSvcMessages<R>> {
        // Per [MS-RDPECLIP] 3.1.1.1, each FormatList completely replaces the previous.
        // A text/image copy ends file visibility to the remote, which may interrupt an
        // in-progress file download - acceptable since the user explicitly chose new content.
        self.local_file_list = None;
        self.local_file_list_format_id = None;

        let mut pdus = Vec::new();

        if R::is_server() {
            pdus.push(ClipboardPdu::FormatList(
                self.build_format_list(available_formats).map_err(|e| encode_err!(e))?,
            ));
        } else {
            match self.state {
                CliprdrState::Ready => {
                    info!("User initiated copy, sending format list");
                    pdus.push(ClipboardPdu::FormatList(
                        self.build_format_list(available_formats).map_err(|e| encode_err!(e))?,
                    ));
                }
                CliprdrState::Initialization => {
                    // During initialization state, first copy action is synthetic and should be sent along with
                    // capabilities and temporary directory PDUs.
                    pdus.push(ClipboardPdu::Capabilities(self.capabilities.clone()));
                    pdus.push(ClipboardPdu::TemporaryDirectory(
                        ClientTemporaryDirectory::new(self.backend.temporary_directory())
                            .map_err(|e| encode_err!(e))?,
                    ));
                    pdus.push(ClipboardPdu::FormatList(
                        self.build_format_list(available_formats).map_err(|e| encode_err!(e))?,
                    ));
                }
            }
        }

        Ok(pdus.into_iter().map(into_cliprdr_message).collect::<Vec<_>>().into())
    }

    /// Takes `&mut self` because it tracks `pending_format_data_request` for response correlation.
    pub fn initiate_paste(&mut self, requested_format: ClipboardFormatId) -> PduResult<CliprdrSvcMessages<R>> {
        self.require_ready("initiate_paste")?;

        // When user initiates paste, send format data request to server, and expect to
        // receive response with contents via `FormatDataResponse` PDU.
        // Track the format so we can correlate the response correctly.
        self.pending_format_data_request = Some(requested_format);

        if Some(requested_format) == self.remote_file_list_format_id {
            info!(format_id = ?requested_format, "User initiated paste for FileGroupDescriptorW");
        }

        let pdu = ClipboardPdu::FormatDataRequest(FormatDataRequest {
            format: requested_format,
        });

        Ok(vec![into_cliprdr_message(pdu)].into())
    }

    /// Generates the next unique clip_data_id for lock operations.
    ///
    /// Per [MS-RDPECLIP] 3.1.5.3.1, the clipDataId must uniquely identify
    /// File Stream data on the clipboard. This method ensures unique IDs by
    /// incrementing a counter, skipping 0 and any IDs that collide with
    /// still-active outgoing locks (only possible after u32 wraparound).
    ///
    /// INVARIANT: this loop terminates because `outgoing_locks.len() <=
    /// MAX_OUTGOING_LOCKS` (100), enforced by the caller `send_lock`.
    /// At most 101 iterations are needed to find
    /// an unused non-zero ID.
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    fn generate_clip_data_id(&mut self) -> u32 {
        debug_assert!(
            self.outgoing_locks.len() <= MAX_OUTGOING_LOCKS,
            "outgoing_locks exceeds MAX_OUTGOING_LOCKS; loop may not terminate"
        );

        // Bounded loop converts a potential infinite hang into a visible panic
        // if the caller invariant (outgoing_locks.len() <= MAX_OUTGOING_LOCKS)
        // is ever violated in release builds.
        for _ in 0..MAX_OUTGOING_LOCKS + 2 {
            let id = self.next_clip_data_id;
            self.next_clip_data_id = self.next_clip_data_id.wrapping_add(1);
            if self.next_clip_data_id == 0 {
                self.next_clip_data_id = 1;
            }
            if id != 0 && !self.outgoing_locks.contains_key(&id) {
                return id;
            }
        }

        unreachable!(
            "no free clip_data_id within {MAX_OUTGOING_LOCKS} + 2 iterations; \
             caller failed to enforce MAX_OUTGOING_LOCKS"
        )
    }

    /// Sends a Lock PDU when file data is detected in a Format List.
    ///
    /// Called internally when files are detected in a Format List from the remote.
    /// Returns None if locking capability is not negotiated or channel is not in Ready state.
    /// Follows the sequence shown in [MS-RDPECLIP] section 1.3.2.3 Figure 3.
    fn send_lock(&mut self) -> Option<Vec<SvcMessage>> {
        // Must be in Ready state
        if self.state != CliprdrState::Ready {
            return None;
        }

        // Check if locking capability is supported
        if !self
            .capabilities
            .flags()
            .contains(ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA)
        {
            return None;
        }

        // Defense-in-depth: limit outgoing locks
        if MAX_OUTGOING_LOCKS <= self.outgoing_locks.len() {
            warn!(
                current_locks = self.outgoing_locks.len(),
                max = MAX_OUTGOING_LOCKS,
                "Too many outgoing locks, skipping automatic lock"
            );
            return None;
        }

        // Detach the previous lock but keep it in outgoing_locks.
        // It may still be protecting active file downloads. expire_all_locks()
        // (called from handle_format_list) will transition it to Expired, and
        // cleanup_expired_locks will send the Unlock PDU once the lock becomes
        // inactive. We must NOT send Unlock immediately — that could abort
        // concurrent downloads from the previous clipboard.
        if let Some(prev_lock_id) = self.current_lock_id.take() {
            // The lock remains in outgoing_locks; expire_all_locks() handles it
            debug!(
                clip_data_id = prev_lock_id,
                "Detached previous lock (kept for active transfers)"
            );
        }

        // Generate unique ID for this lock
        let clip_data_id = self.generate_clip_data_id();

        let now = self.backend.now_ms();

        // Create a minimal lock entry (no file_list needed for remote clipboard)
        // The file list is on the remote side; we're just reserving it
        let lock = OutgoingLock {
            state: LockState::Active,
            created_at_ms: now,
            last_used_at_ms: now,
        };

        // Store in outgoing locks map
        self.outgoing_locks.insert(clip_data_id, lock);
        self.current_lock_id = Some(clip_data_id);

        info!(clip_data_id, "Sent clipboard lock");

        let pdu = ClipboardPdu::LockData(LockDataId(clip_data_id));
        Some(vec![into_cliprdr_message(pdu)])
    }

    /// Transitions all active locks to expired state when clipboard changes.
    ///
    /// Called when FormatList arrives (clipboard content changed).
    /// Locks enter a grace period and will be cleaned up based on activity.
    ///
    /// # Concurrent lock safety
    ///
    /// Multiple locks may be active simultaneously — each protecting a separate
    /// set of file downloads. When the remote clipboard changes, we must NOT
    /// immediately send Unlock PDUs because that would abort in-flight downloads
    /// from the previous clipboard. Instead, locks transition to `Expired` state
    /// and remain in `outgoing_locks`. The two-tier cleanup
    /// ([`cleanup_expired_locks`]) sends Unlock PDUs only when a lock has been
    /// inactive for `lock_inactivity_timeout` (no `FileContentsRequest` activity)
    /// or exceeds `lock_max_lifetime`.
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    fn expire_all_locks(&mut self) {
        if self.outgoing_locks.is_empty() {
            return;
        }

        let now = self.backend.now_ms();
        let mut newly_expired = Vec::new();

        // Transition all Active locks to Expired
        for (&id, lock) in self.outgoing_locks.iter_mut() {
            if matches!(lock.state, LockState::Active) {
                lock.state = LockState::Expired { expired_at_ms: now };
                newly_expired.push(LockDataId(id));
            }
        }

        // Clear lock IDs when clipboard changes so new requests don't
        // attach an expired lock's clipDataId to new clipboard content.
        self.current_lock_id = None;

        if !newly_expired.is_empty() {
            info!(
                count = newly_expired.len(),
                inactivity_timeout_secs = self.lock_inactivity_timeout.as_secs(),
                max_lifetime_secs = self.lock_max_lifetime.as_secs(),
                "Expiring locks due to clipboard change"
            );
            self.backend.on_outgoing_locks_expired(&newly_expired);
        }

        // Backend notification deferred until actual cleanup
    }

    /// Lazily runs periodic cleanup during normal API activity.
    ///
    /// Cleans up expired locks, stale file contents requests, and inactive
    /// locked file list snapshots in a single throttled sweep.
    ///
    /// Fast paths:
    /// Processes expired locks, stale file transfers, and abandoned uploads.
    ///
    /// This method performs three cleanup sweeps:
    /// 1. **Outgoing locks**: Sends Unlock PDUs for locks that have exceeded their inactivity
    ///    timeout (default 60s) or max lifetime (default 2h)
    /// 2. **Stale requests**: Removes file contents requests older than `transfer_timeout` and
    ///    sends synthetic error responses to the backend
    /// 3. **Abandoned uploads**: Cleans up locked file list snapshots with no recent activity
    ///
    /// Callers must drive this method from a periodic timer in their event loop
    /// (e.g., every 5 seconds). The returned PDUs must be sent on the CLIPRDR channel.
    ///
    /// ## Example
    /// ```no_run
    /// # use ironrdp_cliprdr::{Cliprdr, CliprdrBackend};
    /// # fn example(cliprdr: &mut Cliprdr<ironrdp_svc::client_processor::Client>) -> Result<(), Box<dyn std::error::Error>> {
    /// // Called from event loop timer arm (e.g., every 5 seconds)
    /// let messages = cliprdr.drive_timeouts()?;
    /// for msg in messages.into_iter() {
    ///     // send_on_channel(msg)?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn drive_timeouts(&mut self) -> PduResult<CliprdrSvcMessages<R>> {
        self.drive_timeouts_impl()
    }

    /// Internal implementation of timeout processing.
    fn drive_timeouts_impl(&mut self) -> PduResult<CliprdrSvcMessages<R>> {
        let now = self.backend.now_ms();
        let inactivity_timeout_ms = u64::try_from(self.lock_inactivity_timeout.as_millis()).unwrap_or(u64::MAX);
        let max_lifetime_ms = u64::try_from(self.lock_max_lifetime.as_millis()).unwrap_or(u64::MAX);
        let mut messages = Vec::new();
        let mut expired_ids = Vec::new();

        // Collect locks that should be cleaned up
        for (clip_data_id, lock) in &self.outgoing_locks {
            if matches!(lock.state, LockState::Expired { .. }) {
                let total_lifetime_ms = now.saturating_sub(lock.created_at_ms);
                let time_since_activity_ms = now.saturating_sub(lock.last_used_at_ms);

                // Rule 1: Cleanup if inactive for inactivity_timeout (60s)
                if inactivity_timeout_ms <= time_since_activity_ms {
                    debug!(
                        clip_data_id,
                        inactive_secs = time_since_activity_ms / 1000,
                        "Lock inactive, sending Unlock PDU"
                    );
                    expired_ids.push(*clip_data_id);
                    continue;
                }

                // Rule 2: Force cleanup after max_lifetime since creation (2h)
                if max_lifetime_ms <= total_lifetime_ms {
                    warn!(
                        clip_data_id,
                        lifetime_secs = total_lifetime_ms / 1000,
                        "Lock exceeded maximum lifetime, forcing cleanup"
                    );
                    expired_ids.push(*clip_data_id);
                    continue;
                }

                // Lock is expired but still within timeout window
                debug!(
                    clip_data_id,
                    time_since_activity_secs = time_since_activity_ms / 1000,
                    total_lifetime_secs = total_lifetime_ms / 1000,
                    "Expired lock still within timeout window"
                );
            }
        }

        // Remove and send Unlock for each
        for clip_data_id in &expired_ids {
            if let Some(_lock) = self.outgoing_locks.remove(clip_data_id) {
                debug!(clip_data_id, "Removed expired lock from tracking");
                let pdu = ClipboardPdu::UnlockData(LockDataId(*clip_data_id));
                messages.push(into_cliprdr_message(pdu));
            }
        }

        // Log cleanup summary
        if !expired_ids.is_empty() {
            info!(
                count = expired_ids.len(),
                clip_data_ids = ?expired_ids,
                "Automatic lock cleanup completed"
            );
        }

        // Clear current_lock_id if it was expired
        if let Some(current_id) = self.current_lock_id {
            if expired_ids.contains(&current_id) {
                self.current_lock_id = None;
            }
        }

        // Notify backend of timeout-expired locks
        if !expired_ids.is_empty() {
            let lock_ids: Vec<LockDataId> = expired_ids.iter().map(|id| LockDataId(*id)).collect();
            self.backend.on_outgoing_locks_cleared(&lock_ids);
        }

        // Cleanup stale file contents requests that have been pending too long.
        // Sends a synthetic error response to the backend for each timed-out
        // request so callers can clean up rather than waiting forever.
        let transfer_timeout_ms = u64::try_from(self.transfer_timeout.as_millis()).unwrap_or(u64::MAX);
        let stale_stream_ids: Vec<u32> = self
            .sent_file_contents_requests
            .iter()
            .filter(|(_, state)| transfer_timeout_ms <= now.saturating_sub(state.sent_at_ms))
            .map(|(stream_id, _)| *stream_id)
            .collect();

        for stream_id in &stale_stream_ids {
            self.sent_file_contents_requests.remove(stream_id);
            warn!(
                stream_id,
                timeout_secs = transfer_timeout_ms / 1000,
                "File contents request timed out, sending synthetic error to backend"
            );
            self.backend
                .on_file_contents_response(FileContentsResponse::new_error(*stream_id));
        }

        if !stale_stream_ids.is_empty() {
            info!(
                count = stale_stream_ids.len(),
                "Stale file contents request cleanup completed"
            );
        }

        // Cleanup locked file list snapshots with no recent FileContentsRequest
        // activity. Notifies backend via on_unlock() for each removed entry so
        // it can release associated file handles and resources.
        let stale_lock_ids: Vec<u32> = self
            .locked_file_list_activity
            .iter()
            .filter(|(_, last_activity)| transfer_timeout_ms <= now.saturating_sub(**last_activity))
            .map(|(clip_data_id, _)| *clip_data_id)
            .collect();

        for clip_data_id in &stale_lock_ids {
            self.locked_file_lists.remove(clip_data_id);
            self.locked_file_list_activity.remove(clip_data_id);
            warn!(
                clip_data_id,
                timeout_secs = transfer_timeout_ms / 1000,
                "Locked file list timed out due to upload inactivity, sending unlock to backend"
            );
            self.backend.on_unlock(LockDataId(*clip_data_id));
        }

        if !stale_lock_ids.is_empty() {
            info!(count = stale_lock_ids.len(), "Upload inactivity cleanup completed");
        }

        Ok(messages.into())
    }

    /// [2.2.5.3] File Contents Request PDU (CLIPRDR_FILECONTENTS_REQUEST)
    ///
    /// Requests file contents from the Shared Clipboard Owner. Should be called when
    /// the Local Clipboard Owner needs file data after receiving a file list format.
    /// The remote will respond via [`CliprdrBackend::on_file_contents_response`].
    ///
    /// ## Validation
    ///
    /// Per [MS-RDPECLIP] 3.1.5.4.5:
    /// - The file index (lindex) must be obtained from a prior file list exchange
    /// - For SIZE requests: cbRequested must be 8, position must be 0
    /// - For RANGE requests: the specified range must be within file bounds
    ///
    /// The streamId is tracked to validate the corresponding FileContentsResponse.
    ///
    /// [2.2.5.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeclip/cbc851d3-4e68-45f4-9292-26872a9209f2
    pub fn request_file_contents(&mut self, mut request: FileContentsRequest) -> PduResult<CliprdrSvcMessages<R>> {
        self.require_ready("request_file_contents")?;

        // [MS-RDPECLIP] 2.2.2.1.1.1 - CB_STREAM_FILECLIP_ENABLED must be negotiated
        if !self
            .capabilities
            .flags()
            .contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED)
        {
            return Err(ironrdp_pdu::PduError::new(
                "request_file_contents",
                ironrdp_pdu::PduErrorKind::Other {
                    description: "CB_STREAM_FILECLIP_ENABLED not negotiated",
                },
            ));
        }

        // [MS-RDPECLIP] 2.2.5.3 - Include clipDataId if we have an active lock
        // Use the most recently created lock (current_lock_id) by default.
        // Caller can override by setting request.data_id explicitly before calling this method.
        // This allows the receiver to correlate the request with locked File Stream data
        // even if the clipboard has changed since the lock was sent.
        if request.data_id.is_none() {
            if let Some(clip_data_id) = self.current_lock_id {
                request.data_id = Some(clip_data_id);
            } else if self
                .capabilities
                .flags()
                .contains(ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA)
            {
                // Locking was negotiated but no lock is active (e.g. all locks
                // expired). The request will proceed without a clipDataId, which
                // means the server may serve it from the current (possibly changed)
                // clipboard rather than a locked snapshot.
                debug!(
                    stream_id = request.stream_id,
                    "File contents request proceeding without lock despite CAN_LOCK_CLIPDATA being negotiated"
                );
            }
        }

        // Update last_used_at to track activity and prevent timeout
        if let Some(clip_data_id) = request.data_id {
            if let Some(lock) = self.outgoing_locks.get_mut(&clip_data_id) {
                lock.last_used_at_ms = self.backend.now_ms();
                trace!(
                    clip_data_id,
                    stream_id = request.stream_id,
                    "Updated lock activity timestamp"
                );
            }
        }

        // [MS-RDPECLIP] 2.2.5.3 - Validate flags are spec-compliant
        if let Err(e) = request.flags.validate() {
            return Err(ironrdp_pdu::PduError::new(
                "request_file_contents",
                ironrdp_pdu::PduErrorKind::Other { description: e },
            ));
        }

        // [MS-RDPECLIP] 2.2.5.3 - Validate SIZE request constraints
        if request.flags.contains(FileContentsFlags::SIZE) {
            if request.requested_size != 8 {
                return Err(ironrdp_pdu::PduError::new(
                    "request_file_contents",
                    ironrdp_pdu::PduErrorKind::Other {
                        description: "SIZE request must have requested_size=8",
                    },
                ));
            }
            if request.position != 0 {
                return Err(ironrdp_pdu::PduError::new(
                    "request_file_contents",
                    ironrdp_pdu::PduErrorKind::Other {
                        description: "SIZE request must have position=0",
                    },
                ));
            }
        }

        // [MS-RDPECLIP] 3.1.5.4.5 - Validate file index is from known file list
        let validated_file_index = usize::try_from(request.index).map_err(|_| {
            ironrdp_pdu::PduError::new(
                "request_file_contents",
                ironrdp_pdu::PduErrorKind::Other {
                    description: "file index is negative",
                },
            )
        })?;

        if let Some(ref file_list) = self.remote_file_list {
            if file_list.files.len() <= validated_file_index {
                return Err(ironrdp_pdu::PduError::new(
                    "request_file_contents",
                    ironrdp_pdu::PduErrorKind::Other {
                        description: "file index out of bounds for remote file list",
                    },
                ));
            }

            // [MS-RDPECLIP] 3.1.5.4.5 - Validate RANGE request is within file bounds
            if request.flags.contains(FileContentsFlags::RANGE) {
                // Validate requested_size > 0 for RANGE requests
                if request.requested_size == 0 {
                    return Err(ironrdp_pdu::PduError::new(
                        "request_file_contents",
                        ironrdp_pdu::PduErrorKind::Other {
                            description: "RANGE request must have requested_size > 0",
                        },
                    ));
                }

                if let Some(file_desc) = file_list.files.get(validated_file_index) {
                    if let Some(file_size) = file_desc.file_size {
                        let end_position = request.position.saturating_add(u64::from(request.requested_size));
                        if file_size < end_position {
                            return Err(ironrdp_pdu::PduError::new(
                                "request_file_contents",
                                ironrdp_pdu::PduErrorKind::Other {
                                    description: "RANGE request exceeds file bounds",
                                },
                            ));
                        }
                    }
                }
            }

            // [MS-RDPECLIP] 2.2.5.3 - Validate huge file position constraints
            let supports_huge_files = self
                .capabilities
                .flags()
                .contains(ClipboardGeneralCapabilityFlags::HUGE_FILE_SUPPORT_ENABLED);

            if !supports_huge_files && 0x8000_0000 <= request.position {
                // 2^31
                return Err(ironrdp_pdu::PduError::new(
                    "request_file_contents",
                    ironrdp_pdu::PduErrorKind::Other {
                        description: "large file position requires CB_HUGE_FILE_SUPPORT_ENABLED capability",
                    },
                ));
            }
        } else {
            warn!("FileContentsRequest sent without remote file list");
            // Proceeding anyway - remote may have file list we don't know about
        }

        // Reject if too many requests are already pending.
        if MAX_PENDING_FILE_REQUESTS <= self.sent_file_contents_requests.len() {
            return Err(ironrdp_pdu::PduError::new(
                "request_file_contents",
                ironrdp_pdu::PduErrorKind::Other {
                    description: "too many pending file contents requests",
                },
            ));
        }

        // Track this request so we can validate the response.
        if self.sent_file_contents_requests.contains_key(&request.stream_id) {
            warn!(
                stream_id = request.stream_id,
                "Overwriting pending request with same stream_id"
            );
        }
        self.sent_file_contents_requests.insert(
            request.stream_id,
            FileTransferState {
                file_index: validated_file_index,
                flags: request.flags,
                sent_at_ms: self.backend.now_ms(),
            },
        );

        debug!(
            stream_id = request.stream_id,
            index = request.index,
            flags = ?request.flags,
            "Sending FileContentsRequest"
        );

        let pdu = ClipboardPdu::FileContentsRequest(request);
        Ok(vec![into_cliprdr_message(pdu)].into())
    }

    /// [2.2.5.2] CLIPRDR_FILELIST - Initiates file copy operation
    ///
    /// Starts processing of file copy command with the given file descriptors.
    /// Should be called by the clipboard implementation when user performs a file copy
    /// operation. This method stores the file list and sends a FormatList PDU containing
    /// the FileGroupDescriptorW format.
    ///
    /// Per [MS-RDPECLIP] 1.3.1.4 "Delayed Rendering", the file list data will only be sent
    /// when the remote requests it via FormatDataRequest (after user initiates paste remotely).
    ///
    /// ## Validation
    ///
    /// Per [MS-RDPECLIP] 2.2.5.2.3.1, file names are validated against the following constraints:
    /// - Maximum length: 259 characters (leaving room for null terminator in 260-character field)
    /// - File name must not be empty
    ///
    /// Invalid file descriptors are logged and skipped; if all descriptors are invalid,
    /// an empty file list is sent.
    ///
    /// [2.2.5.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeclip/9c01c966-e09b-438d-9391-ce31f3caddc3
    pub fn initiate_file_copy(&mut self, files: Vec<FileDescriptor>) -> PduResult<CliprdrSvcMessages<R>> {
        self.require_ready("initiate_file_copy")?;

        if !self
            .capabilities
            .flags()
            .contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED)
        {
            return Err(ironrdp_pdu::PduError::new(
                "initiate_file_copy",
                ironrdp_pdu::PduErrorKind::Other {
                    description: "CB_STREAM_FILECLIP_ENABLED not negotiated - server does not support file transfer",
                },
            ));
        }

        // [MS-RDPECLIP] 2.2.5.2.3.1 - Validate file descriptors per spec requirements
        // fileName field is 520 bytes = 260 Unicode characters (including null terminator)
        const MAX_FILENAME_LEN: usize = 259;

        let file_clip_no_file_paths = self
            .capabilities
            .flags()
            .contains(ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS);

        let original_count = files.len();
        let validated_files: Vec<FileDescriptor> = files
            .into_iter()
            .filter(|file| {
                // Compute wire name length without allocating: the wire name is
                // "relative_path\name" (or just "name" when there is no path).
                let wire_len = match &file.relative_path {
                    Some(path) if !path.is_empty() => {
                        path.chars().count() + 1 /* backslash */ + file.name.chars().count()
                    }
                    _ => file.name.chars().count(),
                };

                // The wire name's absolute-path prefix comes from the first
                // component, so check the relative_path (if present) or the
                // name directly - no allocation needed.
                let wire_is_absolute = match &file.relative_path {
                    Some(path) if !path.is_empty() => is_absolute_path(path),
                    _ => is_absolute_path(&file.name),
                };

                if file.name.is_empty() {
                    warn!(name = %file.name, "Skipping file with empty name");
                    false
                } else if MAX_FILENAME_LEN < wire_len {
                    warn!(
                        name = %file.name,
                        path = ?file.relative_path,
                        wire_length = wire_len,
                        max_length = MAX_FILENAME_LEN,
                        "Skipping file with wire name exceeding maximum length"
                    );
                    false
                } else if file_clip_no_file_paths && wire_is_absolute {
                    warn!(
                        name = %file.name,
                        path = ?file.relative_path,
                        "Skipping file with absolute path (CB_FILECLIP_NO_FILE_PATHS is set)"
                    );
                    false
                } else {
                    true
                }
            })
            .collect();

        if validated_files.len() < original_count {
            info!(
                total = original_count,
                valid = validated_files.len(),
                "File list validation completed with warnings"
            );
        }

        // Store the validated file list so we can send it when requested
        self.local_file_list = Some(PackedFileList { files: validated_files });

        // Build a format list with FileGroupDescriptorW format.
        // Per MS-RDPECLIP 1.3.1.2, the format name "FileGroupDescriptorW" is constant across all
        // implementations, but the format ID is arbitrary and OS-specific.
        //
        // Format ID ranges:
        // - 0x0000-0x00FF: Standard Windows clipboard formats (CF_TEXT, CF_UNICODETEXT, etc.)
        // - 0xC000-0xFFFF: Private/application-specific formats (registered via RegisterClipboardFormat)
        //
        // We use 0xC0FE in the private range. This ID is only used locally to identify our file list
        // in the format list we send. The remote endpoint will map this to their own format ID based
        // on the format name "FileGroupDescriptorW". When the remote requests this format via
        // FormatDataRequest, they will use our ID (0xC0FE), which we use to recognize the request
        // in handle_format_data_request.
        const FILE_LIST_FORMAT_ID: u32 = 0xC0FE;
        let format_id = ClipboardFormatId::new(FILE_LIST_FORMAT_ID);
        let formats = vec![ClipboardFormat::new(format_id).with_name(ClipboardFormatName::FILE_LIST)];

        // Track the format ID we're using for this file list
        self.local_file_list_format_id = Some(format_id);

        let format_list = self.build_format_list(&formats).map_err(|e| encode_err!(e))?;
        let pdu = ClipboardPdu::FormatList(format_list);

        Ok(vec![into_cliprdr_message(pdu)].into())
    }
}

impl<R: Role> SvcProcessor for Cliprdr<R> {
    fn channel_name(&self) -> ChannelName {
        Self::CHANNEL_NAME
    }

    fn start(&mut self) -> PduResult<Vec<SvcMessage>> {
        if self.state != CliprdrState::Initialization {
            error!("Attempted to start clipboard static virtual channel in invalid state");
        }

        if R::is_server() {
            Ok(vec![self.capabilities()?, self.monitor_ready()?])
        } else {
            Ok(Vec::new())
        }
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let pdu = decode::<ClipboardPdu<'_>>(payload).map_err(|e| decode_err!(e))?;

        match pdu {
            ClipboardPdu::Capabilities(caps) => self.handle_server_capabilities(caps),
            ClipboardPdu::FormatList(format_list) => self.handle_format_list(format_list),
            ClipboardPdu::FormatListResponse(response) => self.handle_format_list_response(response),
            ClipboardPdu::MonitorReady => self.handle_monitor_ready(),
            ClipboardPdu::LockData(id) => {
                // [MS-RDPECLIP] 3.1.5.3.2 - Processing a Lock Clipboard Data PDU
                // Store a snapshot of the current file list so we can service
                // FileContentsRequest PDUs with this clipDataId even after clipboard changes.

                // Defense-in-depth: Limit number of locked file lists to prevent memory exhaustion
                // from malicious remotes sending unlimited Lock PDUs
                if MAX_LOCKED_FILE_LISTS <= self.locked_file_lists.len() {
                    warn!(
                        clip_data_id = id.0,
                        current_locks = self.locked_file_lists.len(),
                        "Too many locked file lists, rejecting new lock request"
                    );
                    return Ok(Vec::new());
                }

                if let Some(ref file_list) = self.local_file_list {
                    info!(clip_data_id = id.0, "Locking clipboard with file list snapshot");
                    self.locked_file_lists.insert(id.0, file_list.clone());
                    self.locked_file_list_activity.insert(id.0, self.backend.now_ms());
                } else {
                    // This is expected when the local side has no file list (e.g., browser
                    // client with no pending upload). Per [MS-RDPECLIP] 3.1.5.3.2, the
                    // storage action is conditional on File Stream data being present.
                    debug!(
                        clip_data_id = id.0,
                        "Received Lock PDU but no local file list available"
                    );
                }
                self.backend.on_lock(id);
                Ok(Vec::new())
            }
            ClipboardPdu::UnlockData(id) => {
                // [MS-RDPECLIP] 3.1.5.3.4 - Processing an Unlock Clipboard Data PDU
                // Release the file list snapshot associated with this clipDataId.
                if self.locked_file_lists.remove(&id.0).is_some() {
                    self.locked_file_list_activity.remove(&id.0);
                    info!(
                        clip_data_id = id.0,
                        "Unlocking clipboard and releasing file list snapshot"
                    );
                } else {
                    // Per [MS-RDPECLIP] 3.1.5.3.4, an Unlock for a nonexistent Lock
                    // "MUST be ignored." This is normal when the Lock had no file data.
                    debug!(clip_data_id = id.0, "Received Unlock PDU but no locked file list found");
                }
                self.backend.on_unlock(id);
                Ok(Vec::new())
            }
            ClipboardPdu::FormatDataRequest(request) => {
                // Check if this is a request for our stored file list by comparing format IDs
                if Some(request.format) == self.local_file_list_format_id {
                    if let Some(ref file_list) = self.local_file_list {
                        // Respond with the stored file list
                        info!(
                            format_id = ?request.format,
                            file_count = file_list.files.len(),
                            "Responding to FileGroupDescriptorW request with stored file list"
                        );
                        let response = OwnedFormatDataResponse::new_file_list(file_list).map_err(|e| encode_err!(e))?;
                        let pdu = ClipboardPdu::FormatDataResponse(response);
                        return Ok(vec![into_cliprdr_message(pdu)]);
                    } else {
                        // Format ID matches but we don't have a file list - this shouldn't happen
                        warn!("Received FormatDataRequest for file list format but no file list stored");
                    }
                }

                // Forward to backend for other format requests
                self.backend.on_format_data_request(request);

                // NOTE: Actual data should be sent later via `submit_format_data` method,
                // therefore we do not send anything immediately.
                Ok(Vec::new())
            }
            ClipboardPdu::FormatDataResponse(response) => {
                // Correlate this response with the most recently sent FormatDataRequest.
                // Only intercept as a file list if the request was for the file list format;
                // forward all other responses (text, images, etc.) to the backend.
                let requested_format = self.pending_format_data_request.take();
                let is_file_list_response =
                    requested_format.is_some() && requested_format == self.remote_file_list_format_id;

                if is_file_list_response {
                    if response.is_error() {
                        warn!(?requested_format, "FileGroupDescriptorW request failed");
                        self.backend.on_format_data_response(response);
                        Ok(Vec::new())
                    } else {
                        // Parse the file list and store it in our abstract data model
                        match response.to_file_list() {
                            Ok(mut file_list) => {
                                // Sanitize file paths to prevent path traversal attacks while
                                // preserving safe relative directory structure.
                                // A malicious remote could send names like "../../../etc/passwd".
                                for file in &mut file_list.files {
                                    if let Some(sanitized) = sanitize_file_path(&file.name) {
                                        // Compare components directly to detect changes
                                        // without allocating a combined wire name string.
                                        let changed = file.name != sanitized.name
                                            || match (&file.relative_path, &sanitized.relative_path) {
                                                (None, None) => false,
                                                (Some(a), Some(b)) => a != b,
                                                _ => true,
                                            };
                                        if changed {
                                            warn!(
                                                original = %file.name,
                                                sanitized_name = %sanitized.name,
                                                sanitized_path = ?sanitized.relative_path,
                                                "Sanitized potentially dangerous file path from remote"
                                            );
                                        }
                                        file.name = sanitized.name;
                                        file.relative_path = sanitized.relative_path;
                                    } else {
                                        warn!(
                                            original = %file.name,
                                            "Rejecting file with invalid name from remote"
                                        );
                                        file.name = String::from("unnamed_file");
                                        file.relative_path = None;
                                    }
                                }

                                info!(
                                    file_count = file_list.files.len(),
                                    "Received FileGroupDescriptorW from remote"
                                );
                                // Notify backend with file metadata and the current lock ID
                                // (if locking was negotiated). The lock is already held at this point.
                                self.backend.on_remote_file_list(&file_list.files, self.current_lock_id);

                                // Store the remote file list for FileContentsRequest validation.
                                self.remote_file_list = Some(file_list);

                                Ok(Vec::new())
                            }
                            Err(err) => {
                                error!(?err, "Failed to parse FileGroupDescriptorW from FormatDataResponse");
                                // Notify backend of the failure so it can handle the error
                                self.backend.on_format_data_response(response);
                                Ok(Vec::new())
                            }
                        }
                    }
                } else {
                    // Forward other format data responses to backend
                    self.backend.on_format_data_response(response);
                    Ok(Vec::new())
                }
            }
            ClipboardPdu::FileContentsRequest(request) => {
                // [MS-RDPECLIP] 2.2.2.1.1.1 - CB_STREAM_FILECLIP_ENABLED must be negotiated
                if !self
                    .capabilities
                    .flags()
                    .contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED)
                {
                    warn!(
                        stream_id = request.stream_id,
                        "Received FileContentsRequest but CB_STREAM_FILECLIP_ENABLED not negotiated"
                    );
                    let error_response = FileContentsResponse::new_error(request.stream_id);
                    let pdu = ClipboardPdu::FileContentsResponse(error_response.into_owned());
                    return Ok(vec![into_cliprdr_message(pdu)]);
                }

                // [MS-RDPECLIP] 3.1.5.4.6 - Processing a File Contents Request PDU
                // Per MS-RDPECLIP 3.1.5.4.6: "If the clipDataId field is present, then the locked
                // File Stream data associated with the ID MUST be used to service the request."

                // Determine which file list to use based on clipDataId presence
                let file_list_to_use = if let Some(clip_data_id) = request.data_id {
                    // Use locked file list snapshot if clipDataId is present
                    match self.locked_file_lists.get(&clip_data_id) {
                        Some(locked_list) => {
                            // Track activity to prevent inactivity timeout
                            self.locked_file_list_activity
                                .insert(clip_data_id, self.backend.now_ms());
                            debug!(
                                stream_id = request.stream_id,
                                clip_data_id, "Using locked file list snapshot for FileContentsRequest"
                            );
                            Some(locked_list)
                        }
                        None => {
                            // Lock snapshot was cleaned up (inactivity timeout or Unlock PDU),
                            // but the file list itself is still valid. Fall back to local_file_list
                            // to support repaste scenarios where the server retries after a delay.
                            debug!(
                                stream_id = request.stream_id,
                                clip_data_id, "Locked file list snapshot expired, falling back to local file list"
                            );
                            self.local_file_list.as_ref()
                        }
                    }
                } else {
                    // Use current local file list if no clipDataId
                    self.local_file_list.as_ref()
                };

                // Validate the file index against the chosen file list bounds.
                if let Some(file_list) = file_list_to_use {
                    // INVARIANT: request.index >= 0 (validated during decode), so usize
                    // conversion is safe. Use usize comparison to avoid u32 truncation
                    // on the file list length.
                    let file_index = usize::try_from(request.index).unwrap_or(usize::MAX);
                    if file_list.files.len() <= file_index {
                        warn!(
                            stream_id = request.stream_id,
                            index = request.index,
                            file_count = file_list.files.len(),
                            clip_data_id = ?request.data_id,
                            "Received FileContentsRequest with index out of bounds"
                        );
                        // [MS-RDPECLIP] 3.1.5.4.7 - Send error response if request cannot be satisfied
                        let error_response = FileContentsResponse::new_error(request.stream_id);
                        let pdu = ClipboardPdu::FileContentsResponse(error_response.into_owned());
                        return Ok(vec![into_cliprdr_message(pdu)]);
                    }
                } else {
                    warn!(
                        stream_id = request.stream_id,
                        "Received FileContentsRequest but no file list available"
                    );
                    // Send error response - we have no files to serve
                    let error_response = FileContentsResponse::new_error(request.stream_id);
                    let pdu = ClipboardPdu::FileContentsResponse(error_response.into_owned());
                    return Ok(vec![into_cliprdr_message(pdu)]);
                }

                debug!(
                    stream_id = request.stream_id,
                    index = request.index,
                    flags = ?request.flags,
                    "Processing FileContentsRequest"
                );

                // Forward to backend - it will call submit_file_contents() with response
                self.backend.on_file_contents_request(request);
                Ok(Vec::new())
            }
            ClipboardPdu::FileContentsResponse(response) => {
                // [MS-RDPECLIP] 3.1.5.4.8 - Processing a File Contents Response PDU
                let stream_id = response.stream_id();

                // Validate this response matches a request we sent
                if let Some(transfer_state) = self.sent_file_contents_requests.remove(&stream_id) {
                    debug!(
                        stream_id,
                        file_index = transfer_state.file_index,
                        is_error = response.is_error(),
                        data_len = response.data().len(),
                        "Received FileContentsResponse"
                    );

                    if response.is_error() {
                        warn!(stream_id, "FileContentsResponse indicates failure (CB_RESPONSE_FAIL)");

                        // [MS-RDPECLIP] 2.2.5.4 - FAIL responses MUST have zero-length data.
                        // Sanitize non-conforming error responses to prevent backends from
                        // misinterpreting stale data bytes as valid content.
                        if !response.data().is_empty() {
                            warn!(
                                stream_id,
                                data_len = response.data().len(),
                                "Sanitizing error response: clearing non-empty data per MS-RDPECLIP 2.2.5.4"
                            );
                            self.backend
                                .on_file_contents_response(FileContentsResponse::new_error(stream_id));
                        } else {
                            self.backend.on_file_contents_response(response);
                        }
                    } else if transfer_state.flags.contains(FileContentsFlags::SIZE) && response.data().len() != 8 {
                        // [MS-RDPECLIP] 2.2.5.4 - SIZE responses MUST be exactly 8 bytes
                        // (a 64-bit unsigned integer). A malformed SIZE response would cause
                        // backends to either fail in data_as_size() or misinterpret the bytes.
                        // Convert to an error response so backends handle it uniformly.
                        warn!(
                            stream_id,
                            data_len = response.data().len(),
                            "Converting malformed SIZE response to error: expected 8 bytes per MS-RDPECLIP 2.2.5.4"
                        );
                        self.backend
                            .on_file_contents_response(FileContentsResponse::new_error(stream_id));
                    } else {
                        // Forward valid response to backend
                        self.backend.on_file_contents_response(response);
                    }
                } else {
                    warn!(
                        stream_id,
                        "Received FileContentsResponse for unknown streamId (no matching request sent); dropping"
                    );
                }

                Ok(Vec::new())
            }
            ClipboardPdu::TemporaryDirectory(_) => {
                // do nothing
                Ok(Vec::new())
            }
        }
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::WhenRdpDataIsCompressed
    }
}

fn into_cliprdr_message(pdu: ClipboardPdu<'static>) -> SvcMessage {
    // Adding [`CHANNEL_FLAG_SHOW_PROTOCOL`] is a must for clipboard svc messages, because they
    // contain chunked data. This is the requirement from `MS-RDPBCGR` specification.
    SvcMessage::from(pdu).with_flags(ChannelFlags::SHOW_PROTOCOL)
}

/// Client-side role marker for the CLIPRDR channel.
#[derive(Debug)]
pub struct Client {}

impl Role for Client {
    fn is_server() -> bool {
        false
    }
}

/// Server-side role marker for the CLIPRDR channel.
#[derive(Debug)]
pub struct Server {}

impl Role for Server {
    fn is_server() -> bool {
        true
    }
}

/// Test-only accessors for `Cliprdr` internal state.
///
/// These methods are gated behind the `__test` feature and exist solely
/// so that tests in `ironrdp-testsuite-core` can set up / inspect internal
/// fields without making them part of the public API.
#[cfg(feature = "__test")]
#[doc(hidden)]
impl<R: Role> Cliprdr<R> {
    pub fn __test_state(&self) -> &CliprdrState {
        &self.state
    }

    pub fn __test_state_mut(&mut self) -> &mut CliprdrState {
        &mut self.state
    }

    pub fn __test_capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn __test_capabilities_mut(&mut self) -> &mut Capabilities {
        &mut self.capabilities
    }

    pub fn __test_outgoing_locks(&self) -> &HashMap<u32, OutgoingLock> {
        &self.outgoing_locks
    }

    pub fn __test_outgoing_locks_mut(&mut self) -> &mut HashMap<u32, OutgoingLock> {
        &mut self.outgoing_locks
    }

    pub fn __test_current_lock_id(&self) -> Option<u32> {
        self.current_lock_id
    }

    pub fn __test_sent_file_contents_requests(&self) -> &HashMap<u32, FileTransferState> {
        &self.sent_file_contents_requests
    }

    pub fn __test_sent_file_contents_requests_mut(&mut self) -> &mut HashMap<u32, FileTransferState> {
        &mut self.sent_file_contents_requests
    }

    pub fn __test_locked_file_lists(&self) -> &HashMap<u32, PackedFileList> {
        &self.locked_file_lists
    }

    pub fn __test_locked_file_lists_mut(&mut self) -> &mut HashMap<u32, PackedFileList> {
        &mut self.locked_file_lists
    }

    pub fn __test_locked_file_list_activity(&self) -> &HashMap<u32, u64> {
        &self.locked_file_list_activity
    }

    pub fn __test_local_file_list(&self) -> &Option<PackedFileList> {
        &self.local_file_list
    }

    pub fn __test_local_file_list_mut(&mut self) -> &mut Option<PackedFileList> {
        &mut self.local_file_list
    }

    pub fn __test_local_file_list_format_id(&self) -> Option<ClipboardFormatId> {
        self.local_file_list_format_id
    }

    pub fn __test_local_file_list_format_id_mut(&mut self) -> &mut Option<ClipboardFormatId> {
        &mut self.local_file_list_format_id
    }

    pub fn __test_remote_file_list_mut(&mut self) -> &mut Option<PackedFileList> {
        &mut self.remote_file_list
    }

    pub fn __test_remote_file_list_format_id(&self) -> Option<ClipboardFormatId> {
        self.remote_file_list_format_id
    }
}
