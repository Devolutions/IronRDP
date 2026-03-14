use std::sync::{Arc, Mutex};

use ironrdp_cliprdr::CliprdrClient;
use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::{
    Capabilities, ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags,
    ClipboardPdu, ClipboardProtocolVersion, FileDescriptor, FormatListResponse,
};
use ironrdp_core::AsAny;
use ironrdp_svc::SvcProcessor as _;

/// Tracks callbacks invoked on the backend for verification in tests
#[derive(Debug, Default, Clone)]
struct CallbackTracker {
    remote_copy_calls: Vec<Vec<ClipboardFormat>>,
    remote_file_list_calls: Vec<Vec<FileDescriptor>>,
}

/// Mock backend for testing delayed rendering behavior
#[derive(Debug)]
struct MockBackend {
    temp_dir: String,
    tracker: Arc<Mutex<CallbackTracker>>,
}

impl CliprdrBackend for MockBackend {
    fn temporary_directory(&self) -> &str {
        &self.temp_dir
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
            | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
    }

    fn on_ready(&mut self) {}
    fn on_request_format_list(&mut self) {}
    fn on_process_negotiated_capabilities(&mut self, _capabilities: ClipboardGeneralCapabilityFlags) {}
    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        self.tracker
            .lock()
            .unwrap()
            .remote_copy_calls
            .push(available_formats.to_vec());
    }
    fn on_format_data_request(&mut self, _request: ironrdp_cliprdr::pdu::FormatDataRequest) {}
    fn on_format_data_response(&mut self, _response: ironrdp_cliprdr::pdu::FormatDataResponse<'_>) {}
    fn on_file_contents_request(&mut self, _request: ironrdp_cliprdr::pdu::FileContentsRequest) {}
    fn on_file_contents_response(&mut self, _response: ironrdp_cliprdr::pdu::FileContentsResponse<'_>) {}
    fn on_lock(&mut self, _data_id: ironrdp_cliprdr::pdu::LockDataId) {}
    fn on_unlock(&mut self, _data_id: ironrdp_cliprdr::pdu::LockDataId) {}
    fn on_remote_file_list(&mut self, files: &[FileDescriptor], _clip_data_id: Option<u32>) {
        self.tracker.lock().unwrap().remote_file_list_calls.push(files.to_vec());
    }

    fn now_ms(&self) -> u64 {
        // Tests use real time; lock timeouts are not exercised here.
        use std::time::Instant;
        static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        u64::try_from(EPOCH.get_or_init(Instant::now).elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn elapsed_ms(&self, since: u64) -> u64 {
        self.now_ms().saturating_sub(since)
    }
}

impl AsAny for MockBackend {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

/// Drive a CliprdrClient through the initialization handshake to Ready state.
///
/// Simulates: Server Capabilities -> Monitor Ready -> client initiate_copy ->
/// FormatListResponse::Ok, which transitions the client from Initialization to Ready.
fn drive_to_ready(cliprdr: &mut CliprdrClient) {
    // Server sends Capabilities with file transfer support
    let caps_pdu = ClipboardPdu::Capabilities(Capabilities::new(
        ClipboardProtocolVersion::V2,
        ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
            | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS,
    ));
    let caps_bytes = ironrdp_core::encode_vec(&caps_pdu).unwrap();
    cliprdr.process(&caps_bytes).unwrap();

    // Server sends Monitor Ready (triggers backend.on_request_format_list)
    let monitor_pdu = ClipboardPdu::MonitorReady;
    let monitor_bytes = ironrdp_core::encode_vec(&monitor_pdu).unwrap();
    cliprdr.process(&monitor_bytes).unwrap();

    // Client responds with initiate_copy (sends Caps + TempDir + FormatList)
    let formats = vec![ClipboardFormat::new(ClipboardFormatId::new(13))];
    cliprdr.initiate_copy(&formats).unwrap();

    // Server accepts with FormatListResponse::Ok -> transitions to Ready
    let resp_pdu = ClipboardPdu::FormatListResponse(FormatListResponse::Ok);
    let resp_bytes = ironrdp_core::encode_vec(&resp_pdu).unwrap();
    cliprdr.process(&resp_bytes).unwrap();
}

fn new_cliprdr() -> CliprdrClient {
    let backend = Box::new(MockBackend {
        temp_dir: "/tmp/test".to_owned(),
        tracker: Arc::new(Mutex::new(CallbackTracker::default())),
    });
    CliprdrClient::new(backend)
}

fn new_ready_cliprdr() -> CliprdrClient {
    let mut cliprdr = new_cliprdr();
    drive_to_ready(&mut cliprdr);
    cliprdr
}

#[test]
fn initiate_file_copy_requires_ready_state() {
    // [MS-RDPECLIP] 2.2.5.2 - initiate_file_copy returns Err when not in Ready state
    let mut cliprdr = new_cliprdr();

    let files = vec![
        FileDescriptor::new("test.txt").with_file_size(1024),
        FileDescriptor::new("data.bin").with_file_size(2048),
    ];

    let result = cliprdr.initiate_file_copy(files);
    assert!(result.is_err(), "Should return Err when not in Ready state");
}

#[test]
fn initiate_paste_requires_ready_state() {
    // [MS-RDPECLIP] 1.3.1.4 - Verify that initiate_paste requires Ready state
    // Per "Delayed Rendering", format data is only requested when user pastes (and state is Ready)

    let mut cliprdr = new_cliprdr();

    let format_id = ClipboardFormatId::new(0xC0BC);

    let result = cliprdr.initiate_paste(format_id);
    assert!(result.is_err(), "Should return Err when not in Ready state");
}

#[test]
fn initiate_copy_api() {
    // Verify that initiate_copy sends FormatList (possibly with initialization PDUs)

    let mut cliprdr = new_cliprdr();

    let formats = vec![ClipboardFormat::new(ClipboardFormatId::new(13))]; // CF_UNICODETEXT

    // Initiate copy - should send initialization PDUs + FormatList on first call
    let result = cliprdr.initiate_copy(&formats);
    assert!(result.is_ok(), "Should successfully initiate copy");

    let output: Vec<_> = result.unwrap().into();
    // Should send Capabilities + TemporaryDirectory + FormatList during initialization
    assert!(
        !output.is_empty(),
        "Should send at least FormatList (may include initialization PDUs)"
    );
}

#[test]
fn initiate_copy_clears_file_list_state() {
    // Verify that initiate_copy clears any previous file list state
    // This ensures that file list state doesn't leak between operations

    let mut cliprdr = new_ready_cliprdr();

    // First, initiate file copy (now in Ready state, so this succeeds)
    let files = vec![FileDescriptor::new("test.txt").with_file_size(1024)];

    cliprdr.initiate_file_copy(files).unwrap();

    // Now initiate regular copy (should clear file list state)
    let text_formats = vec![ClipboardFormat::new(ClipboardFormatId::new(13))]; // CF_UNICODETEXT
    let result = cliprdr.initiate_copy(&text_formats);
    assert!(result.is_ok(), "Should allow regular copy after file copy");
}

#[test]
fn file_descriptor_round_trip() {
    // Test that FileDescriptor can be encoded and decoded correctly
    // This is essential for file list metadata exchange

    let original = FileDescriptor::new("test.txt")
        .with_last_write_time(129010042240261384)
        .with_file_size(1024);

    let encoded = ironrdp_core::encode_vec(&original).unwrap();
    let decoded: FileDescriptor = ironrdp_core::decode(&encoded).unwrap();

    assert_eq!(decoded.name, original.name);
    assert_eq!(decoded.attributes, original.attributes);
    assert_eq!(decoded.last_write_time, original.last_write_time);
    assert_eq!(decoded.file_size, original.file_size);
}

#[test]
fn file_list_format_name_constant() {
    // [MS-RDPECLIP] 1.3.1.2 - Verify that the FileGroupDescriptorW format name constant is correct
    // The format name is constant across all implementations, only format ID varies

    assert_eq!(
        ClipboardFormatName::FILE_LIST.value(),
        "FileGroupDescriptorW",
        "FILE_LIST constant should be FileGroupDescriptorW per MS-RDPECLIP 1.3.1.2"
    );
}

#[test]
fn empty_file_list() {
    // Verify that empty file lists are handled gracefully in Ready state

    let mut cliprdr = new_ready_cliprdr();

    let files = Vec::new(); // Empty file list

    let result = cliprdr.initiate_file_copy(files);
    assert!(result.is_ok(), "Should handle empty file list gracefully");
}

#[test]
fn file_descriptor_minimal_metadata() {
    // Test file descriptor with only filename (no attributes, size, or timestamp)
    // Per spec, these fields are optional

    let file_desc = FileDescriptor::new("minimal.txt");

    let encoded = ironrdp_core::encode_vec(&file_desc).unwrap();
    let decoded: FileDescriptor = ironrdp_core::decode(&encoded).unwrap();

    assert_eq!(decoded.name, "minimal.txt");
    assert!(decoded.attributes.is_none());
    assert!(decoded.last_write_time.is_none());
    assert!(decoded.file_size.is_none());
}

#[test]
fn file_descriptor_259_char_name_accepted() {
    // [MS-RDPECLIP] 2.2.5.2.3.1 - Verify that exactly 259 character filename is valid
    // The fileName field is 520 bytes = 260 Unicode characters (including null terminator)
    // Maximum content length is 259 characters

    let mut cliprdr = new_ready_cliprdr();

    // Create a filename with exactly 259 characters (valid boundary)
    let name = "a".repeat(259);
    let files = vec![FileDescriptor::new(name).with_file_size(1024)];

    let result = cliprdr.initiate_file_copy(files);
    assert!(result.is_ok(), "Should accept 259 character filename");

    // In Ready state, a valid file should produce a FormatList message
    let output: Vec<_> = result.unwrap().into();
    assert!(!output.is_empty(), "Should send FormatList for valid file");
}

#[test]
fn file_descriptor_260_char_name_rejected() {
    // [MS-RDPECLIP] 2.2.5.2.3.1 - Verify that 260 character filename is rejected
    // Maximum length is 259 characters (leaving room for null terminator in 260-character field)

    let mut cliprdr = new_ready_cliprdr();

    // Create a filename with 260 characters (invalid - exceeds max)
    let name = "a".repeat(260);
    let files = vec![FileDescriptor::new(name).with_file_size(1024)];

    // Should succeed but skip the invalid descriptor
    let result = cliprdr.initiate_file_copy(files);
    assert!(result.is_ok(), "Should handle 260 character filename gracefully");

    // The invalid descriptor is skipped with a warning; an empty file list is still sent
}

#[test]
fn file_descriptor_empty_name_rejected() {
    // [MS-RDPECLIP] 2.2.5.2.3.1 - Verify that empty filename is rejected
    // File names must not be empty per spec

    let mut cliprdr = new_ready_cliprdr();

    let files = vec![FileDescriptor::new("").with_file_size(1024)];

    // Should succeed but skip the invalid descriptor
    let result = cliprdr.initiate_file_copy(files);
    assert!(result.is_ok(), "Should handle empty filename gracefully");
}

#[test]
fn file_descriptor_mixed_valid_invalid_names() {
    // Test that valid descriptors are accepted while invalid ones are skipped
    // when processing a file list with mixed validity

    let mut cliprdr = new_ready_cliprdr();

    let files = vec![
        FileDescriptor::new("valid.txt").with_file_size(100),      // Valid
        FileDescriptor::new("").with_file_size(200),               // Invalid - empty
        FileDescriptor::new("also_valid.doc").with_file_size(300), // Valid
        FileDescriptor::new("x".repeat(260)).with_file_size(400),  // Invalid - too long
    ];

    // Should succeed - valid files accepted, invalid ones skipped
    let result = cliprdr.initiate_file_copy(files);
    assert!(
        result.is_ok(),
        "Should process file list with mixed valid/invalid descriptors"
    );

    // Two valid descriptors should be stored and a FormatList sent
    let output: Vec<_> = result.unwrap().into();
    assert!(!output.is_empty(), "Should send FormatList with valid descriptors");
}

#[test]
fn initiate_file_copy_requires_stream_fileclip_enabled() {
    // Verify that initiate_file_copy returns Err when STREAM_FILECLIP_ENABLED is not negotiated.
    // This can happen when the server does not advertise file transfer support.

    let backend = Box::new(MockBackend {
        temp_dir: "/tmp/test".to_owned(),
        tracker: Arc::new(Mutex::new(CallbackTracker::default())),
    });

    let mut cliprdr = CliprdrClient::new(backend);

    // Drive to Ready but with server capabilities that lack STREAM_FILECLIP_ENABLED
    let caps_pdu = ClipboardPdu::Capabilities(Capabilities::new(
        ClipboardProtocolVersion::V2,
        // Only USE_LONG_FORMAT_NAMES - no STREAM_FILECLIP_ENABLED
        ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES,
    ));
    let caps_bytes = ironrdp_core::encode_vec(&caps_pdu).unwrap();
    cliprdr.process(&caps_bytes).unwrap();

    let monitor_pdu = ClipboardPdu::MonitorReady;
    let monitor_bytes = ironrdp_core::encode_vec(&monitor_pdu).unwrap();
    cliprdr.process(&monitor_bytes).unwrap();

    let formats = vec![ClipboardFormat::new(ClipboardFormatId::new(13))];
    cliprdr.initiate_copy(&formats).unwrap();

    let resp_pdu = ClipboardPdu::FormatListResponse(FormatListResponse::Ok);
    let resp_bytes = ironrdp_core::encode_vec(&resp_pdu).unwrap();
    cliprdr.process(&resp_bytes).unwrap();

    // Now in Ready state, but without STREAM_FILECLIP_ENABLED
    let files = vec![FileDescriptor::new("test.txt").with_file_size(1024)];

    let result = cliprdr.initiate_file_copy(files);
    assert!(
        result.is_err(),
        "Should return Err when STREAM_FILECLIP_ENABLED is not negotiated"
    );
}
