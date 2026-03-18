//! Tests for FileContentsRequest/Response tracking, validation, and
//! concurrent transfer support.
//!
//! Migrated from `ironrdp-cliprdr/src/lib.rs` inline `#[cfg(test)]` module.
//! Behavior assertions verify returned PDUs and backend callbacks;
//! bookkeeping assertions (tracking map state) use the `__test` feature gate.

use std::sync::{Arc, Mutex};

use ironrdp_cliprdr::pdu::{
    ClipboardPdu, FileContentsFlags, FileContentsRequest, FileContentsResponse, FileDescriptor, PackedFileList,
};
use ironrdp_cliprdr::{CliprdrClient, CliprdrState, FileTransferState};
use ironrdp_core::Encode as _;
use ironrdp_svc::{SvcMessage, SvcProcessor as _};

use super::test_helpers::{RecordingBackend, TestBackend};

/// Introduce `let` bindings for the encoded bytes and the decoded
/// [`ClipboardPdu`] in the caller's scope.  Two names are required so
/// that the byte buffer outlives the borrowing PDU.
macro_rules! decode_pdu {
    ($msg:expr => $bytes:ident, $pdu:ident) => {
        let $bytes = ($msg).encode_unframed_pdu().unwrap();
        let $pdu = ironrdp_core::decode::<ClipboardPdu<'_>>(&$bytes).unwrap();
    };
}

/// Helper: create a CliprdrClient in Ready state using __test accessors.
fn ready_client() -> CliprdrClient {
    let mut cliprdr = CliprdrClient::new(Box::new(TestBackend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;
    cliprdr
}

/// Helper: build a simple file list with the given file names.
fn file_list(names: &[&str]) -> PackedFileList {
    PackedFileList {
        files: names.iter().map(|name| FileDescriptor::new(*name)).collect(),
    }
}

/// Helper: build a file list where each file has a specified size.
fn sized_file_list(entries: &[(&str, u64)]) -> PackedFileList {
    PackedFileList {
        files: entries
            .iter()
            .map(|(name, size)| FileDescriptor::new(*name).with_file_size(*size))
            .collect(),
    }
}

// ── Request tracking ────────────────────────────────────────────────

#[test]
fn request_tracking() {
    let mut cliprdr = ready_client();
    *cliprdr.__test_remote_file_list_mut() = Some(file_list(&["file1.txt", "file2.txt", "file3.txt"]));

    let request = FileContentsRequest {
        stream_id: 42,
        index: 1,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: None,
    };

    let messages: Vec<SvcMessage> = cliprdr.request_file_contents(request).unwrap().into();

    // Behavior: a FileContentsRequest PDU is returned with the correct fields
    assert_eq!(messages.len(), 1);
    decode_pdu!(messages[0] => _bytes, pdu);
    match pdu {
        ClipboardPdu::FileContentsRequest(req) => {
            assert_eq!(req.stream_id, 42);
            assert_eq!(req.index, 1);
            assert_eq!(req.flags, FileContentsFlags::SIZE);
        }
        other => panic!("expected FileContentsRequest PDU, got {other:?}"),
    }

    // Bookkeeping: tracking entry created
    assert!(cliprdr.__test_sent_file_contents_requests().contains_key(&42));
}

#[test]
fn request_index_validation() {
    let mut cliprdr = ready_client();
    *cliprdr.__test_remote_file_list_mut() = Some(file_list(&["file1.txt", "file2.txt"]));

    // Index 5 is out of bounds for a 2-file list
    let request = FileContentsRequest {
        stream_id: 99,
        index: 5,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: None,
    };

    let result = cliprdr.request_file_contents(request);
    assert!(result.is_err(), "out-of-bounds index should be rejected");
}

#[test]
fn request_bounds_validation() {
    let mut cliprdr = ready_client();
    *cliprdr.__test_remote_file_list_mut() = Some(sized_file_list(&[("test.txt", 1000)]));

    // Test 1: Overflow detection - position near u64::MAX
    let overflow_request = FileContentsRequest {
        stream_id: 100,
        index: 0,
        flags: FileContentsFlags::RANGE,
        position: u64::MAX - 1,
        requested_size: 100,
        data_id: None,
    };
    assert!(
        cliprdr.request_file_contents(overflow_request).is_err(),
        "overflow position should be rejected"
    );

    // Test 2: Out-of-bounds detection - position + size > file_size
    let out_of_bounds_request = FileContentsRequest {
        stream_id: 101,
        index: 0,
        flags: FileContentsFlags::RANGE,
        position: 900,
        requested_size: 200, // 900 + 200 = 1100 > 1000
        data_id: None,
    };
    assert!(
        cliprdr.request_file_contents(out_of_bounds_request).is_err(),
        "out-of-bounds range should be rejected"
    );

    // Test 3: Valid request within bounds
    let valid_request = FileContentsRequest {
        stream_id: 102,
        index: 0,
        flags: FileContentsFlags::RANGE,
        position: 500,
        requested_size: 400, // 500 + 400 = 900 <= 1000
        data_id: None,
    };
    assert!(cliprdr.request_file_contents(valid_request).is_ok());
    assert!(cliprdr.__test_sent_file_contents_requests().contains_key(&102));
}

// ── Response validation ─────────────────────────────────────────────

#[test]
fn response_validation() {
    let responses = Arc::new(Mutex::new(Vec::new()));
    let backend = RecordingBackend {
        responses: Arc::clone(&responses),
    };
    let mut cliprdr = CliprdrClient::new(Box::new(backend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;

    // Pre-populate tracking for stream_id 42 (bookkeeping setup)
    cliprdr.__test_sent_file_contents_requests_mut().insert(
        42,
        FileTransferState {
            file_index: 0,
            flags: FileContentsFlags::SIZE,
            sent_at_ms: 0,
        },
    );

    // Create and encode a valid SIZE response
    let response = FileContentsResponse::new_size_response(42, 1024);
    let pdu = ClipboardPdu::FileContentsResponse(response);
    let mut buf = vec![0u8; pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    pdu.encode(&mut cursor).unwrap();

    cliprdr.process(&buf).unwrap();

    // Behavior: backend received the response via callback
    let received = responses.lock().unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].stream_id, 42);
    assert!(!received[0].is_error);

    // Bookkeeping: tracking entry consumed
    drop(received);
    assert!(!cliprdr.__test_sent_file_contents_requests().contains_key(&42));
}

#[test]
fn concurrent_file_transfers() {
    let mut cliprdr = ready_client();
    *cliprdr.__test_remote_file_list_mut() = Some(file_list(&["file1.txt", "file2.txt"]));

    let request1 = FileContentsRequest {
        stream_id: 10,
        index: 0,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: None,
    };
    let request2 = FileContentsRequest {
        stream_id: 20,
        index: 1,
        flags: FileContentsFlags::RANGE,
        position: 0,
        requested_size: 1024,
        data_id: None,
    };

    cliprdr.request_file_contents(request1).unwrap();
    cliprdr.request_file_contents(request2).unwrap();

    assert!(cliprdr.__test_sent_file_contents_requests().contains_key(&10));
    assert!(cliprdr.__test_sent_file_contents_requests().contains_key(&20));
    assert_eq!(cliprdr.__test_sent_file_contents_requests().len(), 2);
}

#[test]
fn error_response_clears_tracking() {
    let mut cliprdr = ready_client();

    cliprdr.__test_sent_file_contents_requests_mut().insert(
        123,
        FileTransferState {
            file_index: 0,
            flags: FileContentsFlags::SIZE,
            sent_at_ms: 0,
        },
    );

    let response = FileContentsResponse::new_error(123);
    let pdu = ClipboardPdu::FileContentsResponse(response);
    let mut buf = vec![0u8; pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    pdu.encode(&mut cursor).unwrap();

    let result = cliprdr.process(&buf);
    assert!(result.is_ok());
    assert!(!cliprdr.__test_sent_file_contents_requests().contains_key(&123));
}

/// [MS-RDPECLIP] 2.2.5.4 - SIZE responses MUST be exactly 8 bytes.
/// A malformed SIZE response should be converted to an error response
/// before forwarding to the backend.
#[test]
fn malformed_size_response_converted_to_error() {
    let responses = Arc::new(Mutex::new(Vec::new()));
    let backend = RecordingBackend {
        responses: Arc::clone(&responses),
    };

    let mut cliprdr = CliprdrClient::new(Box::new(backend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;

    cliprdr.__test_sent_file_contents_requests_mut().insert(
        42,
        FileTransferState {
            file_index: 0,
            flags: FileContentsFlags::SIZE,
            sent_at_ms: 0,
        },
    );

    // Create a malformed SIZE response (4 bytes instead of required 8)
    let response = FileContentsResponse::new_data_response(42, vec![0u8; 4]);
    let pdu = ClipboardPdu::FileContentsResponse(response);
    let mut buf = vec![0u8; pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    pdu.encode(&mut cursor).unwrap();

    let result = cliprdr.process(&buf);
    assert!(result.is_ok());

    let received = responses.lock().unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].stream_id, 42);
    assert!(
        received[0].is_error,
        "malformed SIZE response should be converted to error"
    );
    assert_eq!(received[0].data_len, 0, "error response should have zero-length data");
}

/// [MS-RDPECLIP] 2.2.5.4 - FAIL responses MUST have zero-length data.
/// Non-empty error responses should be sanitized before forwarding.
#[test]
fn error_response_data_sanitized() {
    let responses = Arc::new(Mutex::new(Vec::new()));
    let backend = RecordingBackend {
        responses: Arc::clone(&responses),
    };

    let mut cliprdr = CliprdrClient::new(Box::new(backend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;

    cliprdr.__test_sent_file_contents_requests_mut().insert(
        99,
        FileTransferState {
            file_index: 0,
            flags: FileContentsFlags::RANGE,
            sent_at_ms: 0,
        },
    );

    // Manually construct a non-conforming error response with non-empty data.
    // Wire format: msgType(2) + msgFlags(2) + dataLen(4) + streamId(4) + data(N)
    // For an error response: msgFlags = CB_RESPONSE_FAIL (0x0002)
    let mut buf = Vec::new();
    buf.extend_from_slice(&0x0009u16.to_le_bytes()); // msgType = CB_FILECONTENTS_RESPONSE
    buf.extend_from_slice(&0x0002u16.to_le_bytes()); // msgFlags = CB_RESPONSE_FAIL
    buf.extend_from_slice(&8u32.to_le_bytes()); // dataLen = 4 (streamId) + 4 (stale data)
    buf.extend_from_slice(&99u32.to_le_bytes()); // streamId = 99
    buf.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]); // stale data (should be empty)

    let result = cliprdr.process(&buf);
    assert!(result.is_ok());

    let received = responses.lock().unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].stream_id, 99);
    assert!(received[0].is_error, "should still be an error");
    assert_eq!(
        received[0].data_len, 0,
        "stale data should be stripped from error response"
    );
}

// ── Incoming request validation ─────────────────────────────────────

#[test]
fn incoming_file_contents_request_validation() {
    let mut cliprdr = ready_client();
    *cliprdr.__test_local_file_list_mut() = Some(file_list(&["local1.txt", "local2.txt"]));

    // Create an incoming request with invalid index
    let request = FileContentsRequest {
        stream_id: 555,
        index: 10, // Out of bounds
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: None,
    };

    let pdu = ClipboardPdu::FileContentsRequest(request);
    let mut buf = vec![0u8; pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    pdu.encode(&mut cursor).unwrap();

    // Behavior: returns an error FileContentsResponse
    let messages: Vec<SvcMessage> = cliprdr.process(&buf).unwrap();
    assert_eq!(messages.len(), 1);
    decode_pdu!(messages[0] => _bytes, pdu);
    match pdu {
        ClipboardPdu::FileContentsResponse(resp) => {
            assert!(resp.is_error(), "out-of-bounds request should produce error response");
            assert_eq!(resp.stream_id(), 555);
        }
        other => panic!("expected FileContentsResponse PDU, got {other:?}"),
    }
}

// ── Unknown and duplicate streamId handling ─────────────────────────

#[test]
fn unknown_stream_id_response_dropped_silently() {
    let responses = Arc::new(Mutex::new(Vec::new()));
    let backend = RecordingBackend {
        responses: Arc::clone(&responses),
    };
    let mut cliprdr = super::test_helpers::init_ready_client_with_backend(Box::new(backend));

    // Send a FileContentsResponse for a stream_id that was never requested
    let response = FileContentsResponse::new_size_response(9999, 42);
    let pdu = ClipboardPdu::FileContentsResponse(response);
    let bytes = ironrdp_core::encode_vec(&pdu).unwrap();

    // Should succeed without error
    let messages: Vec<SvcMessage> = cliprdr.process(&bytes).unwrap();

    // No outbound PDUs generated
    assert!(messages.is_empty(), "no PDUs should be sent for unknown streamId");

    // Backend should NOT have received the response (it was dropped)
    let received = responses.lock().unwrap();
    assert!(
        received.is_empty(),
        "backend should not receive response for unknown streamId"
    );
}

#[test]
fn duplicate_stream_id_overwrites_tracking() {
    let mut cliprdr = ready_client();
    *cliprdr.__test_remote_file_list_mut() = Some(sized_file_list(&[("a.txt", 100), ("b.txt", 200)]));

    // First request with stream_id=1, file index 0
    let request1 = FileContentsRequest {
        stream_id: 1,
        index: 0,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: None,
    };
    let _: Vec<SvcMessage> = cliprdr.request_file_contents(request1).unwrap().into();

    // Second request reusing stream_id=1, but for file index 1
    let request2 = FileContentsRequest {
        stream_id: 1,
        index: 1,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: None,
    };
    let _: Vec<SvcMessage> = cliprdr.request_file_contents(request2).unwrap().into();

    // Only one tracking entry should exist (second overwrites first)
    let tracking = cliprdr.__test_sent_file_contents_requests();
    assert_eq!(
        tracking.len(),
        1,
        "duplicate stream_id should overwrite, not accumulate"
    );

    let state = tracking.get(&1).unwrap();
    assert_eq!(
        state.file_index, 1,
        "tracking should reflect the second request's file index"
    );
}
