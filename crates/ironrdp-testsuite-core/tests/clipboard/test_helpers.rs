//! Shared test backends and initialization helpers for clipboard tests.
//!
//! This module consolidates the various mock backend implementations used
//! across clipboard test modules into a single location, and provides
//! convenience helpers for driving a [`CliprdrClient`] through the protocol
//! handshake to the `Ready` state via the public API.

// Items in this module are consumed by sibling test modules via
// `super::test_helpers::*`; the compiler cannot see that usage chain and
// warns about "unreachable pub items" on methods/fields inside
// `pub(super)` structs. `dead_code` is suppressed because not all items
// are used yet during the incremental migration from lib.rs inline tests.
#![allow(unreachable_pub, dead_code)]

use core::cell::Cell;
use std::sync::{Arc, Mutex};

use ironrdp_cliprdr::CliprdrClient;
use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::{
    Capabilities, ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags,
    ClipboardPdu, ClipboardProtocolVersion, FileContentsRequest, FileContentsResponse, FileDescriptor,
    FormatDataRequest, FormatDataResponse, FormatListResponse, LockDataId,
};
use ironrdp_core::AsAny;
use ironrdp_svc::SvcProcessor as _;

// ── Clock helper ────────────────────────────────────────────────────

/// Returns monotonic milliseconds using a process-wide epoch, for test
/// backends that don't need a controllable clock.
pub(super) fn real_now_ms() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    u64::try_from(EPOCH.get_or_init(Instant::now).elapsed().as_millis()).unwrap_or(u64::MAX)
}

// ── TestBackend ─────────────────────────────────────────────────────

/// Simplest possible backend: all callbacks are no-ops, no locking
/// capability, uses real wall-clock time.
#[derive(Debug)]
pub(super) struct TestBackend;

impl CliprdrBackend for TestBackend {
    fn temporary_directory(&self) -> &str {
        "/tmp"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
    }

    fn on_ready(&mut self) {}
    fn on_request_format_list(&mut self) {}
    fn on_process_negotiated_capabilities(&mut self, _capabilities: ClipboardGeneralCapabilityFlags) {}
    fn on_remote_copy(&mut self, _available_formats: &[ClipboardFormat]) {}
    fn on_format_data_request(&mut self, _request: FormatDataRequest) {}
    fn on_format_data_response(&mut self, _response: FormatDataResponse<'_>) {}
    fn on_file_contents_request(&mut self, _request: FileContentsRequest) {}
    fn on_file_contents_response(&mut self, _response: FileContentsResponse<'_>) {}
    fn on_lock(&mut self, _data_id: LockDataId) {}
    fn on_unlock(&mut self, _data_id: LockDataId) {}

    fn now_ms(&self) -> u64 {
        real_now_ms()
    }
    fn elapsed_ms(&self, since: u64) -> u64 {
        self.now_ms().saturating_sub(since)
    }
}

impl AsAny for TestBackend {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

// ── LockingBackend ──────────────────────────────────────────────────

/// Backend that advertises `CAN_LOCK_CLIPDATA` and provides a mock
/// clock via `Cell<u64>` for deterministic lock timeout tests.
#[derive(Debug)]
pub(super) struct LockingBackend {
    clock_ms: Cell<u64>,
}

impl LockingBackend {
    pub fn new() -> Self {
        Self { clock_ms: Cell::new(0) }
    }

    /// Advance the mock clock by `ms` milliseconds.
    pub fn advance_ms(&self, ms: u64) {
        self.clock_ms.set(self.clock_ms.get() + ms);
    }
}

impl CliprdrBackend for LockingBackend {
    fn temporary_directory(&self) -> &str {
        "/tmp"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
    }

    fn on_ready(&mut self) {}
    fn on_request_format_list(&mut self) {}
    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}
    fn on_remote_copy(&mut self, _: &[ClipboardFormat]) {}
    fn on_format_data_request(&mut self, _: FormatDataRequest) {}
    fn on_format_data_response(&mut self, _: FormatDataResponse<'_>) {}
    fn on_file_contents_request(&mut self, _: FileContentsRequest) {}
    fn on_file_contents_response(&mut self, _: FileContentsResponse<'_>) {}
    fn on_lock(&mut self, _: LockDataId) {}
    fn on_unlock(&mut self, _: LockDataId) {}

    fn now_ms(&self) -> u64 {
        self.clock_ms.get()
    }
    fn elapsed_ms(&self, since: u64) -> u64 {
        self.now_ms().saturating_sub(since)
    }
}

impl AsAny for LockingBackend {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

// ── ReceivedResponse + RecordingBackend ─────────────────────────────

/// Response recorded by [`RecordingBackend`] for assertion in tests.
#[derive(Debug, Clone)]
pub(super) struct ReceivedResponse {
    pub stream_id: u32,
    pub is_error: bool,
    pub data_len: usize,
}

/// Backend that records [`FileContentsResponse`] callbacks for later
/// assertion. Used by tests that verify response forwarding behavior
/// (e.g. malformed size responses, error sanitization).
#[derive(Debug)]
pub(super) struct RecordingBackend {
    pub responses: Arc<Mutex<Vec<ReceivedResponse>>>,
}

impl CliprdrBackend for RecordingBackend {
    fn temporary_directory(&self) -> &str {
        "/tmp"
    }
    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
    }
    fn on_ready(&mut self) {}
    fn on_request_format_list(&mut self) {}
    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}
    fn on_remote_copy(&mut self, _: &[ClipboardFormat]) {}
    fn on_format_data_request(&mut self, _: FormatDataRequest) {}
    fn on_format_data_response(&mut self, _: FormatDataResponse<'_>) {}
    fn on_file_contents_request(&mut self, _: FileContentsRequest) {}
    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        self.responses.lock().unwrap().push(ReceivedResponse {
            stream_id: response.stream_id(),
            is_error: response.is_error(),
            data_len: response.data().len(),
        });
    }
    fn on_lock(&mut self, _: LockDataId) {}
    fn on_unlock(&mut self, _: LockDataId) {}

    fn now_ms(&self) -> u64 {
        real_now_ms()
    }
    fn elapsed_ms(&self, since: u64) -> u64 {
        self.now_ms().saturating_sub(since)
    }
}

impl AsAny for RecordingBackend {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

// ── TimedRecordingBackend ───────────────────────────────────────────

/// Backend combining mock clock, response recording, and unlock
/// tracking. Used for tests that need deterministic time together with
/// callback verification.
#[derive(Debug)]
pub(super) struct TimedRecordingBackend {
    pub clock_ms: Cell<u64>,
    pub responses: Arc<Mutex<Vec<ReceivedResponse>>>,
    pub unlocks: Arc<Mutex<Vec<u32>>>,
}

impl TimedRecordingBackend {
    pub fn new(responses: Arc<Mutex<Vec<ReceivedResponse>>>, unlocks: Arc<Mutex<Vec<u32>>>) -> Self {
        Self {
            clock_ms: Cell::new(0),
            responses,
            unlocks,
        }
    }

    /// Advance the mock clock by `ms` milliseconds.
    pub fn advance_ms(&self, ms: u64) {
        self.clock_ms.set(self.clock_ms.get() + ms);
    }
}

impl CliprdrBackend for TimedRecordingBackend {
    fn temporary_directory(&self) -> &str {
        "/tmp"
    }
    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
    }
    fn on_ready(&mut self) {}
    fn on_request_format_list(&mut self) {}
    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}
    fn on_remote_copy(&mut self, _: &[ClipboardFormat]) {}
    fn on_format_data_request(&mut self, _: FormatDataRequest) {}
    fn on_format_data_response(&mut self, _: FormatDataResponse<'_>) {}
    fn on_file_contents_request(&mut self, _: FileContentsRequest) {}
    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        self.responses.lock().unwrap().push(ReceivedResponse {
            stream_id: response.stream_id(),
            is_error: response.is_error(),
            data_len: response.data().len(),
        });
    }
    fn on_lock(&mut self, _: LockDataId) {}
    fn on_unlock(&mut self, data_id: LockDataId) {
        self.unlocks.lock().unwrap().push(data_id.0);
    }

    fn now_ms(&self) -> u64 {
        self.clock_ms.get()
    }
    fn elapsed_ms(&self, since: u64) -> u64 {
        self.now_ms().saturating_sub(since)
    }
}

impl AsAny for TimedRecordingBackend {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

// ── CallbackTrackingBackend ─────────────────────────────────────────

/// Backend that tracks `on_outgoing_locks_cleared` and
/// `on_outgoing_locks_expired` callback invocations.
/// Has a mock clock for deterministic lock timeout tests.
#[derive(Debug)]
pub(super) struct CallbackTrackingBackend {
    pub cleared_ids: Arc<Mutex<Vec<Vec<LockDataId>>>>,
    pub expired_ids: Arc<Mutex<Vec<Vec<LockDataId>>>>,
    pub clock_ms: Cell<u64>,
}

impl CallbackTrackingBackend {
    pub fn new(cleared_ids: Arc<Mutex<Vec<Vec<LockDataId>>>>) -> Self {
        Self {
            cleared_ids,
            expired_ids: Arc::new(Mutex::new(Vec::new())),
            clock_ms: Cell::new(0),
        }
    }

    pub fn with_expired_tracking(
        cleared_ids: Arc<Mutex<Vec<Vec<LockDataId>>>>,
        expired_ids: Arc<Mutex<Vec<Vec<LockDataId>>>>,
    ) -> Self {
        Self {
            cleared_ids,
            expired_ids,
            clock_ms: Cell::new(0),
        }
    }

    /// Advance the mock clock by `ms` milliseconds.
    pub fn advance_ms(&self, ms: u64) {
        self.clock_ms.set(self.clock_ms.get() + ms);
    }
}

impl CliprdrBackend for CallbackTrackingBackend {
    fn temporary_directory(&self) -> &str {
        "/tmp"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
    }

    fn on_ready(&mut self) {}
    fn on_request_format_list(&mut self) {}
    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}
    fn on_remote_copy(&mut self, _: &[ClipboardFormat]) {}
    fn on_format_data_request(&mut self, _: FormatDataRequest) {}
    fn on_format_data_response(&mut self, _: FormatDataResponse<'_>) {}
    fn on_file_contents_request(&mut self, _: FileContentsRequest) {}
    fn on_file_contents_response(&mut self, _: FileContentsResponse<'_>) {}
    fn on_lock(&mut self, _: LockDataId) {}
    fn on_unlock(&mut self, _: LockDataId) {}

    fn on_outgoing_locks_cleared(&mut self, clip_data_ids: &[LockDataId]) {
        self.cleared_ids.lock().unwrap().push(clip_data_ids.to_vec());
    }

    fn on_outgoing_locks_expired(&mut self, clip_data_ids: &[LockDataId]) {
        self.expired_ids.lock().unwrap().push(clip_data_ids.to_vec());
    }

    fn now_ms(&self) -> u64 {
        self.clock_ms.get()
    }
    fn elapsed_ms(&self, since: u64) -> u64 {
        self.now_ms().saturating_sub(since)
    }
}

impl AsAny for CallbackTrackingBackend {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

// ── Initialization helpers ──────────────────────────────────────────

/// The capability flags used by [server_capabilities_pdu] in the
/// simulated handshake. Tests that need to match against negotiated
/// capabilities can reference this constant.
pub(super) const HANDSHAKE_SERVER_FLAGS: ClipboardGeneralCapabilityFlags =
    ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
        .union(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED)
        .union(ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS)
        .union(ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA);

/// Builds a server Capabilities PDU with file transfer + locking flags.
fn server_capabilities_pdu() -> Vec<u8> {
    ironrdp_core::encode_vec(&ClipboardPdu::Capabilities(Capabilities::new(
        ClipboardProtocolVersion::V2,
        HANDSHAKE_SERVER_FLAGS,
    )))
    .unwrap()
}

/// Builds a MonitorReady PDU.
fn monitor_ready_pdu() -> Vec<u8> {
    ironrdp_core::encode_vec(&ClipboardPdu::MonitorReady).unwrap()
}

/// Builds a FormatListResponse::Ok PDU.
fn format_list_response_ok_pdu() -> Vec<u8> {
    ironrdp_core::encode_vec(&ClipboardPdu::FormatListResponse(FormatListResponse::Ok)).unwrap()
}

/// Drive a [`CliprdrClient`] through the full initialization handshake
/// to `Ready` state using the public API.
///
/// Simulates:
/// 1. Server sends Capabilities
/// 2. Server sends MonitorReady
/// 3. Client calls `initiate_copy` (sends Caps + TempDir + FormatList)
/// 4. Server replies FormatListResponse::Ok -> client transitions to Ready
pub(super) fn drive_to_ready(cliprdr: &mut CliprdrClient) {
    let caps_bytes = server_capabilities_pdu();
    cliprdr.process(&caps_bytes).unwrap();

    let monitor_bytes = monitor_ready_pdu();
    cliprdr.process(&monitor_bytes).unwrap();

    let formats = vec![ClipboardFormat::new(ClipboardFormatId::new(13))];
    cliprdr.initiate_copy(&formats).unwrap();

    let resp_bytes = format_list_response_ok_pdu();
    cliprdr.process(&resp_bytes).unwrap();
}

/// Create a [`CliprdrClient`] with the given backend, driven to Ready state.
pub(super) fn init_ready_client_with_backend(backend: Box<dyn CliprdrBackend>) -> CliprdrClient {
    let mut cliprdr = CliprdrClient::new(backend);
    drive_to_ready(&mut cliprdr);
    cliprdr
}

/// Create a [`CliprdrClient`] with a [`TestBackend`], driven to Ready state.
pub(super) fn init_ready_client() -> CliprdrClient {
    init_ready_client_with_backend(Box::new(TestBackend))
}

/// Create a [`CliprdrClient`] with a [`LockingBackend`], driven to Ready state.
pub(super) fn init_ready_locking_client() -> CliprdrClient {
    init_ready_client_with_backend(Box::new(LockingBackend::new()))
}

/// Simulate the remote sending a FormatList containing FileGroupDescriptorW,
/// then the client requesting and receiving the file list through the public
/// protocol flow.
///
/// After this call, `cliprdr.request_file_contents(...)` will accept
/// indices into the provided `files` list.
pub(super) fn set_remote_file_list(cliprdr: &mut CliprdrClient, files: Vec<FileDescriptor>) {
    use ironrdp_cliprdr::pdu::{FormatList, PackedFileList};

    // 1. Remote sends FormatList with FileGroupDescriptorW
    let file_list_format_id = ClipboardFormatId::new(49534);
    let file_list_format = ClipboardFormat::new(file_list_format_id).with_name(ClipboardFormatName::FILE_LIST);
    let format_list = FormatList::new_unicode(core::slice::from_ref(&file_list_format), true).unwrap();
    let format_list_pdu = ClipboardPdu::FormatList(format_list);
    let format_list_bytes = ironrdp_core::encode_vec(&format_list_pdu).unwrap();
    cliprdr.process(&format_list_bytes).unwrap();

    // 2. Client initiates paste for the FileGroupDescriptorW format
    cliprdr.initiate_paste(file_list_format_id).unwrap();

    // 3. Build a FormatDataResponse containing the packed file list
    let packed = PackedFileList { files };
    let packed_bytes = ironrdp_core::encode_vec(&packed).unwrap();

    let response_pdu = ClipboardPdu::FormatDataResponse(FormatDataResponse::new_data(&packed_bytes));
    let response_bytes = ironrdp_core::encode_vec(&response_pdu).unwrap();
    cliprdr.process(&response_bytes).unwrap();
}
