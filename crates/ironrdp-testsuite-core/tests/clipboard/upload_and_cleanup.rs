//! Tests for initiate_copy file list preservation, FormatList interaction
//! with in-flight requests, stale request cleanup, and locked file list
//! cleanup.
//!
//! Migrated from `ironrdp-cliprdr/src/lib.rs` inline `#[cfg(test)]` module.
//! Behavior assertions verify returned PDUs and backend callbacks;
//! bookkeeping assertions (tracking maps, file lists) use the `__test`
//! feature gate.

use std::sync::{Arc, Mutex};

use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardPdu, FileContentsFlags, FileContentsRequest, FileContentsResponse,
    FileDescriptor, FormatList, FormatListResponse, LockDataId, PackedFileList,
};
use ironrdp_cliprdr::{Cliprdr, CliprdrClient, CliprdrState, FileTransferState};
use ironrdp_core::Encode as _;
use ironrdp_svc::{SvcMessage, SvcProcessor as _};

use super::test_helpers::{RecordingBackend, TestBackend, TimedRecordingBackend};

/// Introduce `let` bindings for the encoded bytes and the decoded
/// [`ClipboardPdu`] in the caller's scope.  Two names are required so
/// that the byte buffer outlives the borrowing PDU.
macro_rules! decode_pdu {
    ($msg:expr => $bytes:ident, $pdu:ident) => {
        let $bytes = ($msg).encode_unframed_pdu().unwrap();
        let $pdu = ironrdp_core::decode::<ClipboardPdu<'_>>(&$bytes).unwrap();
    };
}

/// Helper: build a file list with sizes.
fn sized_file_list(entries: &[(&str, u64)]) -> PackedFileList {
    PackedFileList {
        files: entries
            .iter()
            .map(|(name, size)| FileDescriptor::new(*name).with_file_size(*size))
            .collect(),
    }
}

// ── initiate_copy behavior ──────────────────────────────────────────

#[test]
fn initiate_copy_clears_file_list_even_during_upload() {
    // Per [MS-RDPECLIP] 3.1.1.1, each FormatList completely replaces the previous.
    // A text/image copy ends file visibility to the remote - acceptable since
    // the user explicitly chose new content.
    let mut cliprdr = CliprdrClient::new(Box::new(TestBackend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;

    let file_format_id = ClipboardFormatId::new(0xC0FE);
    *cliprdr.__test_local_file_list_mut() = Some(sized_file_list(&[("upload.txt", 1024)]));
    *cliprdr.__test_local_file_list_format_id_mut() = Some(file_format_id);

    // Text clipboard change triggers initiate_copy
    let text_format = ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT);
    let messages: Vec<SvcMessage> = cliprdr
        .initiate_copy(core::slice::from_ref(&text_format))
        .unwrap()
        .into();

    // Behavior: a FormatList PDU is returned
    assert!(!messages.is_empty());
    decode_pdu!(messages[0] => _bytes, pdu);
    assert!(
        matches!(pdu, ClipboardPdu::FormatList(_)),
        "initiate_copy should produce a FormatList PDU"
    );

    // Bookkeeping: file list cleared unconditionally
    assert!(cliprdr.__test_local_file_list().is_none());
    assert_eq!(cliprdr.__test_local_file_list_format_id(), None);
}

#[test]
fn initiate_copy_clears_file_list_when_no_upload() {
    let mut cliprdr = CliprdrClient::new(Box::new(TestBackend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;

    let text_format = ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT);
    let messages: Vec<SvcMessage> = cliprdr
        .initiate_copy(core::slice::from_ref(&text_format))
        .unwrap()
        .into();

    // Behavior: a FormatList PDU is returned
    assert!(!messages.is_empty());
    decode_pdu!(messages[0] => _bytes, pdu);
    assert!(matches!(pdu, ClipboardPdu::FormatList(_)));

    // Bookkeeping: no file list when not uploading
    assert!(cliprdr.__test_local_file_list().is_none());
    assert!(cliprdr.__test_local_file_list_format_id().is_none());
}

// ── FormatList interaction with in-flight requests ──────────────────

/// [MS-RDPECLIP] 2.2.4.1 / 3.1.5.3.2 - Clipboard locks ensure file data
/// survives clipboard changes. The client must NOT discard its request
/// tracking, or valid responses would be dropped as "unknown streamId".
#[test]
fn format_list_preserves_in_flight_file_contents_requests() {
    let responses = Arc::new(Mutex::new(Vec::new()));
    let backend = RecordingBackend {
        responses: Arc::clone(&responses),
    };

    let mut cliprdr = CliprdrClient::new(Box::new(backend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;

    // Simulate an in-flight download
    cliprdr.__test_sent_file_contents_requests_mut().insert(
        1,
        FileTransferState {
            file_index: 0,
            flags: FileContentsFlags::SIZE,
            sent_at_ms: 0,
        },
    );

    // New FormatList arrives (remote user copied a second file)
    let format_list = FormatList::new_unicode(&[], false).unwrap();
    let format_list_pdu = ClipboardPdu::FormatList(format_list);
    let mut buf = vec![0u8; format_list_pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    format_list_pdu.encode(&mut cursor).unwrap();

    let result = cliprdr.process(&buf);
    assert!(result.is_ok());

    assert!(
        cliprdr.__test_sent_file_contents_requests().contains_key(&1),
        "In-flight request for stream_id=1 must not be cleared by FormatList"
    );

    // Now the server delivers the FileContentsResponse for the locked data
    let response = FileContentsResponse::new_size_response(1, 4096);
    let response_pdu = ClipboardPdu::FileContentsResponse(response);
    let mut buf = vec![0u8; response_pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    response_pdu.encode(&mut cursor).unwrap();

    let result = cliprdr.process(&buf);
    assert!(result.is_ok());

    let received = responses.lock().unwrap();
    assert_eq!(received.len(), 1, "Response should be forwarded to backend");
    assert_eq!(received[0].stream_id, 1);
    assert!(!received[0].is_error, "Response should not be an error");
    assert_eq!(received[0].data_len, 8, "SIZE response should be 8 bytes");

    drop(received);
    assert!(
        !cliprdr.__test_sent_file_contents_requests().contains_key(&1),
        "Request tracking should be consumed after response arrives"
    );
}

#[test]
fn format_list_response_fail_notifies_backend_for_pending_requests() {
    let responses = Arc::new(Mutex::new(Vec::new()));
    let backend = RecordingBackend {
        responses: Arc::clone(&responses),
    };

    let mut cliprdr = CliprdrClient::new(Box::new(backend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;

    for stream_id in [10, 20, 30] {
        cliprdr.__test_sent_file_contents_requests_mut().insert(
            stream_id,
            FileTransferState {
                file_index: 0,
                flags: FileContentsFlags::RANGE,
                sent_at_ms: 0,
            },
        );
    }

    let fail_pdu = ClipboardPdu::FormatListResponse(FormatListResponse::Fail);
    let encoded = ironrdp_core::encode_vec(&fail_pdu).unwrap();
    let result = cliprdr.process(&encoded);
    assert!(result.is_ok());

    assert!(
        cliprdr.__test_sent_file_contents_requests().is_empty(),
        "Pending requests must be cleared after FormatListResponse::Fail"
    );

    let received = responses.lock().unwrap();
    assert_eq!(
        received.len(),
        3,
        "Backend should receive an error response for each pending request"
    );

    let mut received_ids: Vec<u32> = received.iter().map(|r| r.stream_id).collect();
    received_ids.sort();
    assert_eq!(received_ids, vec![10, 20, 30]);

    for r in received.iter() {
        assert!(
            r.is_error,
            "Each response should be an error for stream_id={}",
            r.stream_id
        );
        assert_eq!(r.data_len, 0, "Error responses should have no data");
    }
}

// ── Stale request cleanup ───────────────────────────────────────────

#[test]
fn stale_request_cleanup_after_timeout() {
    let responses = Arc::new(Mutex::new(Vec::new()));
    let unlocks = Arc::new(Mutex::new(Vec::new()));
    let backend = TimedRecordingBackend::new(Arc::clone(&responses), Arc::clone(&unlocks));

    let mut cliprdr: CliprdrClient = Cliprdr::with_all_config(
        Box::new(backend),
        core::time::Duration::from_secs(60),
        core::time::Duration::from_secs(3600),
        core::time::Duration::from_millis(200), // 200ms transfer timeout
    );
    *cliprdr.__test_state_mut() = CliprdrState::Ready;

    cliprdr.__test_sent_file_contents_requests_mut().insert(
        1,
        FileTransferState {
            file_index: 0,
            flags: FileContentsFlags::SIZE,
            sent_at_ms: 0,
        },
    );
    cliprdr.__test_sent_file_contents_requests_mut().insert(
        2,
        FileTransferState {
            file_index: 1,
            flags: FileContentsFlags::RANGE,
            sent_at_ms: 0,
        },
    );
    assert_eq!(cliprdr.__test_sent_file_contents_requests().len(), 2);

    cliprdr
        .downcast_backend::<TimedRecordingBackend>()
        .unwrap()
        .advance_ms(250);

    let _messages = cliprdr.drive_timeouts().unwrap();

    assert_eq!(cliprdr.__test_sent_file_contents_requests().len(), 0);

    let received = responses.lock().unwrap();
    assert_eq!(received.len(), 2);
    for r in received.iter() {
        assert!(r.is_error, "Timed-out request should produce error response");
    }
}

#[test]
fn stale_request_cleanup_spares_recent_requests() {
    let responses = Arc::new(Mutex::new(Vec::new()));
    let unlocks = Arc::new(Mutex::new(Vec::new()));
    let backend = TimedRecordingBackend::new(Arc::clone(&responses), Arc::clone(&unlocks));

    let mut cliprdr: CliprdrClient = Cliprdr::with_all_config(
        Box::new(backend),
        core::time::Duration::from_secs(60),
        core::time::Duration::from_secs(3600),
        core::time::Duration::from_millis(200), // 200ms transfer timeout
    );
    *cliprdr.__test_state_mut() = CliprdrState::Ready;

    // Insert an old request (clock=0)
    cliprdr.__test_sent_file_contents_requests_mut().insert(
        1,
        FileTransferState {
            file_index: 0,
            flags: FileContentsFlags::SIZE,
            sent_at_ms: 0,
        },
    );

    // Advance and insert a recent one (clock=150)
    cliprdr
        .downcast_backend::<TimedRecordingBackend>()
        .unwrap()
        .advance_ms(150);

    cliprdr.__test_sent_file_contents_requests_mut().insert(
        2,
        FileTransferState {
            file_index: 1,
            flags: FileContentsFlags::RANGE,
            sent_at_ms: 150,
        },
    );

    // Advance to 250ms total: request 1 = 250ms old (> 200ms), request 2 = 100ms (< 200ms)
    cliprdr
        .downcast_backend::<TimedRecordingBackend>()
        .unwrap()
        .advance_ms(100);

    let _messages = cliprdr.drive_timeouts().unwrap();

    assert_eq!(cliprdr.__test_sent_file_contents_requests().len(), 1);
    assert!(cliprdr.__test_sent_file_contents_requests().contains_key(&2));

    let received = responses.lock().unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].stream_id, 1);
    assert!(received[0].is_error);
}

// ── Locked file list cleanup ────────────────────────────────────────

#[test]
fn locked_file_list_cleaned_up_after_inactivity() {
    let responses = Arc::new(Mutex::new(Vec::new()));
    let unlocks = Arc::new(Mutex::new(Vec::new()));
    let backend = TimedRecordingBackend::new(Arc::clone(&responses), Arc::clone(&unlocks));

    let mut cliprdr: CliprdrClient = Cliprdr::with_all_config(
        Box::new(backend),
        core::time::Duration::from_secs(60),
        core::time::Duration::from_secs(3600),
        core::time::Duration::from_millis(200), // 200ms transfer timeout
    );
    *cliprdr.__test_state_mut() = CliprdrState::Ready;
    *cliprdr.__test_local_file_list_mut() = Some(sized_file_list(&[("upload.txt", 100)]));

    // Process incoming Lock PDU at clock=0
    let lock_pdu = ClipboardPdu::LockData(LockDataId(42));
    let mut buf = vec![0u8; lock_pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    lock_pdu.encode(&mut cursor).unwrap();
    cliprdr.process(&buf).unwrap();

    assert_eq!(cliprdr.__test_locked_file_lists().len(), 1);
    assert!(cliprdr.__test_locked_file_list_activity().contains_key(&42));

    // Advance past transfer timeout with no FileContentsRequest activity
    cliprdr
        .downcast_backend::<TimedRecordingBackend>()
        .unwrap()
        .advance_ms(250);

    let _messages = cliprdr.drive_timeouts().unwrap();

    assert_eq!(cliprdr.__test_locked_file_lists().len(), 0);
    assert!(!cliprdr.__test_locked_file_list_activity().contains_key(&42));

    let unlock_ids = unlocks.lock().unwrap();
    assert_eq!(unlock_ids.len(), 1);
    assert_eq!(unlock_ids[0], 42);
}

#[test]
fn locked_file_list_activity_prevents_cleanup() {
    let responses = Arc::new(Mutex::new(Vec::new()));
    let unlocks = Arc::new(Mutex::new(Vec::new()));
    let backend = TimedRecordingBackend::new(Arc::clone(&responses), Arc::clone(&unlocks));

    let mut cliprdr: CliprdrClient = Cliprdr::with_all_config(
        Box::new(backend),
        core::time::Duration::from_secs(60),
        core::time::Duration::from_secs(3600),
        core::time::Duration::from_millis(200), // 200ms transfer timeout
    );
    *cliprdr.__test_state_mut() = CliprdrState::Ready;
    *cliprdr.__test_local_file_list_mut() = Some(sized_file_list(&[("upload.txt", 100)]));

    // Process Lock PDU at clock=0
    let lock_pdu = ClipboardPdu::LockData(LockDataId(42));
    let mut buf = vec![0u8; lock_pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    lock_pdu.encode(&mut cursor).unwrap();
    cliprdr.process(&buf).unwrap();

    // Advance 150ms, then send an incoming FileContentsRequest (updates activity)
    cliprdr
        .downcast_backend::<TimedRecordingBackend>()
        .unwrap()
        .advance_ms(150);

    let fcr = FileContentsRequest {
        stream_id: 1,
        index: 0,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: Some(42),
    };
    let fcr_pdu = ClipboardPdu::FileContentsRequest(fcr);
    let mut fcr_buf = vec![0u8; fcr_pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut fcr_buf);
    fcr_pdu.encode(&mut cursor).unwrap();
    cliprdr.process(&fcr_buf).unwrap();

    // Advance another 100ms (250ms total, but only 100ms since last activity)
    cliprdr
        .downcast_backend::<TimedRecordingBackend>()
        .unwrap()
        .advance_ms(100);

    let _messages = cliprdr.drive_timeouts().unwrap();

    // Locked file list should NOT be cleaned up (100ms since activity < 200ms timeout)
    assert_eq!(cliprdr.__test_locked_file_lists().len(), 1);

    let unlock_ids = unlocks.lock().unwrap();
    assert_eq!(unlock_ids.len(), 0);
}

#[test]
fn locked_file_list_cleanup_only_inactive_entries() {
    let responses = Arc::new(Mutex::new(Vec::new()));
    let unlocks = Arc::new(Mutex::new(Vec::new()));
    let backend = TimedRecordingBackend::new(Arc::clone(&responses), Arc::clone(&unlocks));

    let mut cliprdr: CliprdrClient = Cliprdr::with_all_config(
        Box::new(backend),
        core::time::Duration::from_secs(60),
        core::time::Duration::from_secs(3600),
        core::time::Duration::from_millis(200), // 200ms transfer timeout
    );
    *cliprdr.__test_state_mut() = CliprdrState::Ready;
    *cliprdr.__test_local_file_list_mut() = Some(sized_file_list(&[("upload.txt", 100)]));

    // Process two Lock PDUs at clock=0
    for clip_data_id in [10, 20] {
        let lock_pdu = ClipboardPdu::LockData(LockDataId(clip_data_id));
        let mut buf = vec![0u8; lock_pdu.size()];
        let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
        lock_pdu.encode(&mut cursor).unwrap();
        cliprdr.process(&buf).unwrap();
    }
    assert_eq!(cliprdr.__test_locked_file_lists().len(), 2);

    // Advance 150ms and send activity only for lock 20
    cliprdr
        .downcast_backend::<TimedRecordingBackend>()
        .unwrap()
        .advance_ms(150);

    let fcr = FileContentsRequest {
        stream_id: 1,
        index: 0,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: Some(20),
    };
    let fcr_pdu = ClipboardPdu::FileContentsRequest(fcr);
    let mut fcr_buf = vec![0u8; fcr_pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut fcr_buf);
    fcr_pdu.encode(&mut cursor).unwrap();
    cliprdr.process(&fcr_buf).unwrap();

    // Advance another 100ms: lock 10 is 250ms inactive, lock 20 is 100ms since activity
    cliprdr
        .downcast_backend::<TimedRecordingBackend>()
        .unwrap()
        .advance_ms(100);

    let _messages = cliprdr.drive_timeouts().unwrap();

    // Only lock 10 should be cleaned up (inactive), lock 20 still active
    assert_eq!(cliprdr.__test_locked_file_lists().len(), 1);
    assert!(cliprdr.__test_locked_file_lists().contains_key(&20));
    assert!(!cliprdr.__test_locked_file_lists().contains_key(&10));

    let unlock_ids = unlocks.lock().unwrap();
    assert_eq!(unlock_ids.len(), 1);
    assert_eq!(unlock_ids[0], 10);
}

// ── Repaste after lock expiry ──────────────────────────────────────

#[test]
fn file_contents_request_falls_back_to_local_file_list_after_lock_expires() {
    let responses = Arc::new(Mutex::new(Vec::new()));
    let unlocks = Arc::new(Mutex::new(Vec::new()));
    let backend = TimedRecordingBackend::new(Arc::clone(&responses), Arc::clone(&unlocks));

    let mut cliprdr: CliprdrClient = Cliprdr::with_all_config(
        Box::new(backend),
        core::time::Duration::from_secs(60),
        core::time::Duration::from_secs(3600),
        core::time::Duration::from_millis(200), // 200ms transfer timeout
    );
    *cliprdr.__test_state_mut() = CliprdrState::Ready;
    *cliprdr.__test_local_file_list_mut() = Some(sized_file_list(&[("report.pdf", 4096)]));

    // Server sends Lock PDU at clock=0
    let lock_pdu = ClipboardPdu::LockData(LockDataId(99));
    let mut buf = vec![0u8; lock_pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    lock_pdu.encode(&mut cursor).unwrap();
    cliprdr.process(&buf).unwrap();
    assert_eq!(cliprdr.__test_locked_file_lists().len(), 1);

    // First paste attempt fails on the server (no valid target), so no
    // FileContentsRequest is sent. Time passes past the transfer timeout.
    cliprdr
        .downcast_backend::<TimedRecordingBackend>()
        .unwrap()
        .advance_ms(250);
    let _messages = cliprdr.drive_timeouts().unwrap();

    // Lock snapshot is gone
    assert_eq!(cliprdr.__test_locked_file_lists().len(), 0);

    // User opens a valid target and pastes again. The server sends a
    // FileContentsRequest with the original clipDataId. This must succeed
    // by falling back to local_file_list rather than returning an error.
    let fcr = FileContentsRequest {
        stream_id: 7,
        index: 0,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: Some(99),
    };
    let fcr_pdu = ClipboardPdu::FileContentsRequest(fcr);
    let mut fcr_buf = vec![0u8; fcr_pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut fcr_buf);
    fcr_pdu.encode(&mut cursor).unwrap();

    // process() returns Ok(empty) when the request is forwarded to the
    // backend. A non-empty return would mean an error response was sent.
    let result = cliprdr.process(&fcr_buf).unwrap();
    assert!(
        result.is_empty(),
        "Expected request to be forwarded to backend, but got error response PDU"
    );
}
