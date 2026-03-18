use std::sync::{Arc, Mutex};

use ironrdp_cliprdr::CliprdrClient;
use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::{
    Capabilities, CapabilitySet, ClipboardFileAttributes, ClipboardFormat, ClipboardFormatId, ClipboardFormatName,
    ClipboardGeneralCapabilityFlags, ClipboardPdu, ClipboardProtocolVersion, FileDescriptor, FormatDataResponse,
    FormatList, FormatListResponse, GeneralCapabilitySet, PackedFileList,
};
use ironrdp_core::AsAny;
use ironrdp_svc::SvcProcessor as _;

/// Tracks callbacks invoked on the backend for verification in integration tests
#[derive(Debug, Default, Clone)]
struct IntegrationCallbackTracker {
    remote_copy_calls: Vec<Vec<ClipboardFormat>>,
    remote_file_list_calls: Vec<Vec<FileDescriptor>>,
}

/// Mock backend for integration testing that tracks all callbacks
#[derive(Debug)]
struct IntegrationMockBackend {
    temp_dir: String,
    tracker: Arc<Mutex<IntegrationCallbackTracker>>,
}

impl CliprdrBackend for IntegrationMockBackend {
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
    fn on_format_data_response(&mut self, _response: FormatDataResponse<'_>) {}
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

impl AsAny for IntegrationMockBackend {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

#[test]
fn integration_remote_file_list_delayed_rendering() {
    // [MS-RDPECLIP] 1.3.2.2.3 and 1.3.1.4 - Integration test for delayed rendering file list request
    //
    // This test verifies the spec-compliant flow when remote sends a FormatList with FileGroupDescriptorW:
    // 1. Remote sends FormatList containing FileGroupDescriptorW format
    // 2. Client stores the format but does NOT request file list immediately (delayed rendering)
    // 3. User initiates paste by calling initiate_paste() with the FileGroupDescriptorW format ID
    // 4. Client sends FormatDataRequest for the file list
    // 5. Remote sends FormatDataResponse with PackedFileList
    // 6. Client parses file list and calls backend.on_remote_file_list()
    //
    // Per MS-RDPECLIP section 1.3.2.2.3, "The Local Clipboard Owner first requests the list of
    // files available from the clipboard." The word "first" refers to ordering within the paste
    // sequence (file list before file contents), NOT immediately after FormatList receipt.
    // File lists are requested only when the user initiates a paste operation.

    let tracker = Arc::new(Mutex::new(IntegrationCallbackTracker::default()));
    let backend = Box::new(IntegrationMockBackend {
        temp_dir: "/tmp/test".to_owned(),
        tracker: Arc::clone(&tracker),
    });

    let mut cliprdr = CliprdrClient::new(backend);

    // Initialize the client to Ready state (simulating connection setup)
    let empty_formats: Vec<ClipboardFormat> = vec![];
    let _: Vec<_> = cliprdr.initiate_copy(&empty_formats).unwrap().into();
    let format_list_response_pdu = ClipboardPdu::FormatListResponse(FormatListResponse::Ok);
    let _: Vec<_> = cliprdr
        .process(&ironrdp_core::encode_vec(&format_list_response_pdu).unwrap())
        .unwrap();

    // Step 1: Simulate remote sending FormatList with FileGroupDescriptorW
    let formats = vec![
        ClipboardFormat::new(ClipboardFormatId::new(13)), // CF_UNICODETEXT
        ClipboardFormat::new(ClipboardFormatId::new(0xC0BC)).with_name(ClipboardFormatName::FILE_LIST), // FileGroupDescriptorW
    ];

    let format_list = FormatList::new_unicode(&formats, true).unwrap();
    let format_list_pdu = ClipboardPdu::FormatList(format_list);
    let encoded_format_list = ironrdp_core::encode_vec(&format_list_pdu).unwrap();

    // Step 2: Process the FormatList - should store format but NOT request file list yet
    let messages: Vec<_> = cliprdr.process(&encoded_format_list).unwrap();

    // Verify that backend.on_remote_copy() was called with the formats
    let copy_calls = tracker.lock().unwrap().remote_copy_calls.clone();
    assert_eq!(copy_calls.len(), 1, "on_remote_copy should be called once");
    assert_eq!(copy_calls[0].len(), 2, "Should receive both formats");

    // Verify that ONLY FormatListResponse was sent (no automatic FormatDataRequest)
    assert_eq!(
        messages.len(),
        1,
        "Should send only FormatListResponse (delayed rendering)"
    );

    // Step 3: User initiates paste for FileGroupDescriptorW format
    let file_list_format_id = ClipboardFormatId::new(0xC0BC);
    let paste_messages: Vec<_> = cliprdr.initiate_paste(file_list_format_id).unwrap().into();

    // Verify that FormatDataRequest is sent NOW (on user paste)
    assert_eq!(
        paste_messages.len(),
        1,
        "Should send FormatDataRequest on user-initiated paste"
    );

    // Step 4: Simulate remote sending FormatDataResponse with file list
    let file_list = PackedFileList {
        files: vec![
            FileDescriptor::new("document.pdf")
                .with_attributes(ClipboardFileAttributes::ARCHIVE)
                .with_last_write_time(129010042240261384)
                .with_file_size(1024),
            FileDescriptor::new("spreadsheet.xlsx")
                .with_attributes(ClipboardFileAttributes::ARCHIVE)
                .with_last_write_time(129010042240261384)
                .with_file_size(2048),
        ],
    };

    let response = FormatDataResponse::new_file_list(&file_list).unwrap();
    let response_pdu = ClipboardPdu::FormatDataResponse(response);
    let encoded_response = ironrdp_core::encode_vec(&response_pdu).unwrap();

    // Step 5: Process the FormatDataResponse
    let messages: Vec<_> = cliprdr.process(&encoded_response).unwrap();

    // Should not send any messages in response
    assert_eq!(messages.len(), 0, "No response expected for FormatDataResponse");

    // Step 6: Verify that backend.on_remote_file_list() was called with correct files
    let file_list_calls = tracker.lock().unwrap().remote_file_list_calls.clone();
    assert_eq!(file_list_calls.len(), 1, "on_remote_file_list should be called once");
    assert_eq!(file_list_calls[0].len(), 2, "Should receive 2 files");
    assert_eq!(file_list_calls[0][0].name, "document.pdf");
    assert_eq!(file_list_calls[0][0].file_size, Some(1024));
    assert_eq!(file_list_calls[0][1].name, "spreadsheet.xlsx");
    assert_eq!(file_list_calls[0][1].file_size, Some(2048));
}

#[test]
fn integration_local_file_copy_sends_format_list() {
    // [MS-RDPECLIP] 2.2.5.2 - Integration test for local file copy operation
    //
    // This test verifies the complete flow when client initiates a file copy:
    // 1. Client calls initiate_file_copy() with file metadata
    // 2. Client sends FormatList with FileGroupDescriptorW format
    // 3. Remote sends FormatDataRequest for the file list
    // 4. Client automatically responds with FormatDataResponse containing file list

    let tracker = Arc::new(Mutex::new(IntegrationCallbackTracker::default()));
    let backend = Box::new(IntegrationMockBackend {
        temp_dir: "/tmp/test".to_owned(),
        tracker: Arc::clone(&tracker),
    });

    let mut cliprdr = CliprdrClient::new(backend);

    // Transition to Ready state by simulating server initialization messages
    // Server sends Capabilities
    let capabilities = Capabilities {
        capabilities: vec![CapabilitySet::General(GeneralCapabilitySet {
            version: ClipboardProtocolVersion::V2,
            general_flags: ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
                | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
                | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS,
        })],
    };
    let capabilities_pdu = ClipboardPdu::Capabilities(capabilities);
    let _: Vec<_> = cliprdr
        .process(&ironrdp_core::encode_vec(&capabilities_pdu).unwrap())
        .unwrap();

    // Server sends MonitorReady
    let monitor_ready_pdu = ClipboardPdu::MonitorReady;
    let _: Vec<_> = cliprdr
        .process(&ironrdp_core::encode_vec(&monitor_ready_pdu).unwrap())
        .unwrap();

    // Client sends initial empty FormatList to complete initialization
    // initiate_copy works in Initialization state and sends Capabilities + TemporaryDirectory + FormatList
    let empty_formats: Vec<ClipboardFormat> = vec![];
    let _: Vec<_> = cliprdr.initiate_copy(&empty_formats).unwrap().into();

    // Server responds with FormatListResponse::Ok (transitions client to Ready state)
    let format_list_response_pdu = ClipboardPdu::FormatListResponse(FormatListResponse::Ok);
    let _: Vec<_> = cliprdr
        .process(&ironrdp_core::encode_vec(&format_list_response_pdu).unwrap())
        .unwrap();

    // Step 1: Client initiates file copy
    let files = vec![
        FileDescriptor::new("report.docx")
            .with_attributes(ClipboardFileAttributes::ARCHIVE)
            .with_last_write_time(132489216000000000)
            .with_file_size(5000),
        FileDescriptor::new("presentation.pptx")
            .with_attributes(ClipboardFileAttributes::ARCHIVE)
            .with_last_write_time(132489216000000000)
            .with_file_size(10000),
    ];

    let messages: Vec<_> = cliprdr.initiate_file_copy(files).unwrap().into();

    // Step 2: Verify FormatList was sent
    assert_eq!(messages.len(), 1, "Should send FormatList");

    // Step 3: Simulate remote sending FormatDataRequest for our file list
    let request_pdu = ClipboardPdu::FormatDataRequest(ironrdp_cliprdr::pdu::FormatDataRequest {
        format: ClipboardFormatId::new(0xC0FE), // Our format ID
    });
    let encoded_request = ironrdp_core::encode_vec(&request_pdu).unwrap();

    let messages: Vec<_> = cliprdr.process(&encoded_request).unwrap();

    // Step 4: Verify FormatDataResponse with file list was sent
    assert_eq!(messages.len(), 1, "Should send FormatDataResponse");
}

#[test]
fn integration_empty_file_list_handling() {
    // Edge case: Verify that empty file lists are handled gracefully throughout the flow

    let tracker = Arc::new(Mutex::new(IntegrationCallbackTracker::default()));
    let backend = Box::new(IntegrationMockBackend {
        temp_dir: "/tmp/test".to_owned(),
        tracker: Arc::clone(&tracker),
    });

    let mut cliprdr = CliprdrClient::new(backend);

    // Initialize the client to Ready state
    let empty_formats: Vec<ClipboardFormat> = vec![];
    let _: Vec<_> = cliprdr.initiate_copy(&empty_formats).unwrap().into();
    let format_list_response_pdu = ClipboardPdu::FormatListResponse(FormatListResponse::Ok);
    let _: Vec<_> = cliprdr
        .process(&ironrdp_core::encode_vec(&format_list_response_pdu).unwrap())
        .unwrap();

    // Simulate remote sending FormatList with FileGroupDescriptorW
    let formats = vec![ClipboardFormat::new(ClipboardFormatId::new(0xC0BC)).with_name(ClipboardFormatName::FILE_LIST)];

    let format_list = FormatList::new_unicode(&formats, true).unwrap();
    let format_list_pdu = ClipboardPdu::FormatList(format_list);
    let encoded_format_list = ironrdp_core::encode_vec(&format_list_pdu).unwrap();

    let messages: Vec<_> = cliprdr.process(&encoded_format_list).unwrap();
    assert_eq!(
        messages.len(),
        1,
        "Should send only FormatListResponse (delayed rendering)"
    );

    // User initiates paste
    let file_list_format_id = ClipboardFormatId::new(0xC0BC);
    let _: Vec<_> = cliprdr.initiate_paste(file_list_format_id).unwrap().into();

    // Simulate remote sending empty file list
    let empty_file_list = PackedFileList { files: vec![] };

    let response = FormatDataResponse::new_file_list(&empty_file_list).unwrap();
    let response_pdu = ClipboardPdu::FormatDataResponse(response);
    let encoded_response = ironrdp_core::encode_vec(&response_pdu).unwrap();

    let _: Vec<_> = cliprdr.process(&encoded_response).unwrap();

    // Verify backend was called with empty file list
    let file_list_calls = tracker.lock().unwrap().remote_file_list_calls.clone();
    assert_eq!(file_list_calls.len(), 1);
    assert_eq!(file_list_calls[0].len(), 0, "Should receive empty file list");
}

#[test]
fn integration_multiple_format_lists_in_sequence() {
    // Edge case: Multiple FormatLists with FileGroupDescriptorW in rapid succession
    // Verifies that state is properly cleared between clipboard updates

    let tracker = Arc::new(Mutex::new(IntegrationCallbackTracker::default()));
    let backend = Box::new(IntegrationMockBackend {
        temp_dir: "/tmp/test".to_owned(),
        tracker: Arc::clone(&tracker),
    });

    let mut cliprdr = CliprdrClient::new(backend);

    // Initialize the client to Ready state
    let empty_formats: Vec<ClipboardFormat> = vec![];
    let _: Vec<_> = cliprdr.initiate_copy(&empty_formats).unwrap().into();
    let format_list_response_pdu = ClipboardPdu::FormatListResponse(FormatListResponse::Ok);
    let _: Vec<_> = cliprdr
        .process(&ironrdp_core::encode_vec(&format_list_response_pdu).unwrap())
        .unwrap();

    // First FormatList with one file
    let formats1 = vec![ClipboardFormat::new(ClipboardFormatId::new(0xC0BC)).with_name(ClipboardFormatName::FILE_LIST)];
    let format_list1 = FormatList::new_unicode(&formats1, true).unwrap();
    let pdu1 = ClipboardPdu::FormatList(format_list1);
    let _: Vec<_> = cliprdr.process(&ironrdp_core::encode_vec(&pdu1).unwrap()).unwrap();

    // User initiates paste for first file list
    let file_list_format_id = ClipboardFormatId::new(0xC0BC);
    let _: Vec<_> = cliprdr.initiate_paste(file_list_format_id).unwrap().into();

    // Respond with file list
    let file_list1 = PackedFileList {
        files: vec![FileDescriptor::new("file1.txt").with_file_size(100)],
    };
    let response1 = FormatDataResponse::new_file_list(&file_list1).unwrap();
    let response_pdu1 = ClipboardPdu::FormatDataResponse(response1);
    let _: Vec<_> = cliprdr
        .process(&ironrdp_core::encode_vec(&response_pdu1).unwrap())
        .unwrap();

    // Second FormatList with different file (new clipboard content)
    let formats2 = vec![ClipboardFormat::new(ClipboardFormatId::new(0xC0BC)).with_name(ClipboardFormatName::FILE_LIST)];
    let format_list2 = FormatList::new_unicode(&formats2, true).unwrap();
    let pdu2 = ClipboardPdu::FormatList(format_list2);
    let _: Vec<_> = cliprdr.process(&ironrdp_core::encode_vec(&pdu2).unwrap()).unwrap();

    // User initiates paste for second file list
    let _: Vec<_> = cliprdr.initiate_paste(file_list_format_id).unwrap().into();

    // Respond with different file list
    let file_list2 = PackedFileList {
        files: vec![FileDescriptor::new("file2.txt").with_file_size(200)],
    };
    let response2 = FormatDataResponse::new_file_list(&file_list2).unwrap();
    let response_pdu2 = ClipboardPdu::FormatDataResponse(response2);
    let _: Vec<_> = cliprdr
        .process(&ironrdp_core::encode_vec(&response_pdu2).unwrap())
        .unwrap();

    // Verify both file lists were processed correctly
    let file_list_calls = tracker.lock().unwrap().remote_file_list_calls.clone();
    assert_eq!(file_list_calls.len(), 2, "Should process both file lists");
    assert_eq!(file_list_calls[0][0].name, "file1.txt");
    assert_eq!(file_list_calls[1][0].name, "file2.txt");
}

#[test]
fn integration_unexpected_file_list_response_ignored() {
    // Edge case: Receiving FormatDataResponse with file list when not expecting it
    // Should be safely ignored (no crash or state corruption)

    let tracker = Arc::new(Mutex::new(IntegrationCallbackTracker::default()));
    let backend = Box::new(IntegrationMockBackend {
        temp_dir: "/tmp/test".to_owned(),
        tracker: Arc::clone(&tracker),
    });

    let mut cliprdr = CliprdrClient::new(backend);

    // Send FormatDataResponse with file list WITHOUT first receiving a FormatList
    let file_list = PackedFileList {
        files: vec![FileDescriptor::new("unexpected.txt").with_file_size(100)],
    };

    let response = FormatDataResponse::new_file_list(&file_list).unwrap();
    let response_pdu = ClipboardPdu::FormatDataResponse(response);
    let encoded_response = ironrdp_core::encode_vec(&response_pdu).unwrap();

    // Should not crash or call backend
    let _: Vec<_> = cliprdr.process(&encoded_response).unwrap();

    // Verify backend was NOT called since we weren't expecting a file list
    let file_list_calls = tracker.lock().unwrap().remote_file_list_calls.clone();
    assert_eq!(
        file_list_calls.len(),
        0,
        "Unexpected file list should not trigger callback"
    );
}

#[test]
fn integration_format_list_response_fail_clears_local_state() {
    // Verify that FormatListResponse::Fail clears local file list state
    // Per MS-RDPECLIP 3.1.5.2.4, if the remote rejects our FormatList, we should
    // clear our local clipboard state since the remote cannot process it

    let tracker = Arc::new(Mutex::new(IntegrationCallbackTracker::default()));
    let backend = Box::new(IntegrationMockBackend {
        temp_dir: "/tmp/test".to_owned(),
        tracker: Arc::clone(&tracker),
    });

    let mut cliprdr = CliprdrClient::new(backend);

    // Transition to Ready state
    let capabilities = Capabilities {
        capabilities: vec![CapabilitySet::General(GeneralCapabilitySet {
            version: ClipboardProtocolVersion::V2,
            general_flags: ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
                | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
                | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS,
        })],
    };
    let capabilities_pdu = ClipboardPdu::Capabilities(capabilities);
    let _: Vec<_> = cliprdr
        .process(&ironrdp_core::encode_vec(&capabilities_pdu).unwrap())
        .unwrap();

    let monitor_ready_pdu = ClipboardPdu::MonitorReady;
    let _: Vec<_> = cliprdr
        .process(&ironrdp_core::encode_vec(&monitor_ready_pdu).unwrap())
        .unwrap();

    let empty_formats: Vec<ClipboardFormat> = vec![];
    let _: Vec<_> = cliprdr.initiate_copy(&empty_formats).unwrap().into();

    let format_list_response_pdu = ClipboardPdu::FormatListResponse(FormatListResponse::Ok);
    let _: Vec<_> = cliprdr
        .process(&ironrdp_core::encode_vec(&format_list_response_pdu).unwrap())
        .unwrap();

    // Now in Ready state - initiate file copy
    let files = vec![
        FileDescriptor::new("report.docx")
            .with_attributes(ClipboardFileAttributes::ARCHIVE)
            .with_last_write_time(132489216000000000)
            .with_file_size(5000),
    ];

    let messages: Vec<_> = cliprdr.initiate_file_copy(files).unwrap().into();
    assert_eq!(messages.len(), 1, "Should send FormatList");

    // Simulate remote rejecting our FormatList
    let fail_response_pdu = ClipboardPdu::FormatListResponse(FormatListResponse::Fail);
    let messages: Vec<_> = cliprdr
        .process(&ironrdp_core::encode_vec(&fail_response_pdu).unwrap())
        .unwrap();

    // Should not send any response
    assert_eq!(messages.len(), 0, "No messages expected after FormatListResponse::Fail");

    // Verify state was cleared by attempting to send a FormatDataRequest
    // If state wasn't cleared, the cliprdr would try to respond with the stored file list
    // Since state WAS cleared, it should forward the request to backend instead
    let request_pdu = ClipboardPdu::FormatDataRequest(ironrdp_cliprdr::pdu::FormatDataRequest {
        format: ClipboardFormatId::new(0xC0FE), // Our format ID from initiate_file_copy
    });
    let encoded_request = ironrdp_core::encode_vec(&request_pdu).unwrap();

    let messages: Vec<_> = cliprdr.process(&encoded_request).unwrap();

    // Should NOT send FormatDataResponse with file list (since state was cleared)
    // Instead, request should be forwarded to backend (no immediate response)
    assert_eq!(
        messages.len(),
        0,
        "Should not auto-respond with file list after state was cleared by Fail"
    );
}
