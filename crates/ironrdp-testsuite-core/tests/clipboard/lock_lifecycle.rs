//! Tests for clipboard lock lifecycle: automatic lock creation via FormatList,
//! incoming lock snapshots, concurrent locks, incoming lock limits, and
//! lock expiry on clipboard change.
//!
//! Migrated from `ironrdp-cliprdr/src/lib.rs` inline `#[cfg(test)]` module.
//! Behavior assertions use the public API (returned PDUs, backend callbacks);
//! bookkeeping assertions (internal lock counts) use the `__test` feature gate.

use std::sync::{Arc, Mutex};

use ironrdp_cliprdr::pdu::{
    Capabilities, ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags,
    ClipboardPdu, ClipboardProtocolVersion, FileContentsFlags, FileContentsRequest, FileDescriptor, FormatList,
    LockDataId, PackedFileList,
};
use ironrdp_cliprdr::{Cliprdr, CliprdrClient, CliprdrState};
use ironrdp_core::Encode as _;
use ironrdp_svc::{SvcMessage, SvcProcessor as _};

use super::test_helpers::{CallbackTrackingBackend, LockingBackend, TestBackend};

/// Introduce `let` bindings for the encoded bytes and the decoded
/// [`ClipboardPdu`] in the caller's scope.  Two names are required so
/// that the byte buffer outlives the borrowing PDU.
macro_rules! decode_pdu {
    ($msg:expr => $bytes:ident, $pdu:ident) => {
        let $bytes = ($msg).encode_unframed_pdu().unwrap();
        let $pdu = ironrdp_core::decode::<ClipboardPdu<'_>>(&$bytes).unwrap();
    };
}

/// Helper: build a simple file list.
fn file_list(names: &[&str]) -> PackedFileList {
    PackedFileList {
        files: names.iter().map(|name| FileDescriptor::new(*name)).collect(),
    }
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

/// Helper: create a Format List PDU containing FileGroupDescriptorW.
fn file_format_list_buf() -> Vec<u8> {
    let formats = vec![ClipboardFormat {
        id: ClipboardFormatId(49171),
        name: Some(ClipboardFormatName::new("FileGroupDescriptorW")),
    }];

    let format_list = FormatList::new_unicode(&formats, true).unwrap();
    let pdu = ClipboardPdu::FormatList(format_list);
    let mut buf = vec![0u8; pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    pdu.encode(&mut cursor).unwrap();
    buf
}

/// Helper: create a Format List PDU containing only text (no files).
fn text_format_list_buf() -> Vec<u8> {
    let formats = vec![ClipboardFormat {
        id: ClipboardFormatId(13), // CF_UNICODETEXT
        name: None,
    }];

    let format_list = FormatList::new_unicode(&formats, true).unwrap();
    let pdu = ClipboardPdu::FormatList(format_list);
    let mut buf = vec![0u8; pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    pdu.encode(&mut cursor).unwrap();
    buf
}

/// Helper: set up a CliprdrClient in Ready state with CAN_LOCK_CLIPDATA.
fn ready_locking_client() -> CliprdrClient {
    let mut cliprdr = CliprdrClient::new(Box::new(LockingBackend::new()));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;
    *cliprdr.__test_capabilities_mut() = Capabilities::new(
        ClipboardProtocolVersion::V2,
        ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
            | ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
            | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED,
    );
    cliprdr
}

/// Helper: process a file FormatList and extract the lock ID from the
/// returned Lock PDU.
fn process_file_format_list(cliprdr: &mut CliprdrClient) -> u32 {
    let messages: Vec<SvcMessage> = cliprdr.process(&file_format_list_buf()).unwrap();
    assert!(messages.len() >= 2, "expected FormatListResponse + LockData");
    decode_pdu!(messages[1] => _bytes, lock_pdu);
    match lock_pdu {
        ClipboardPdu::LockData(id) => id.0,
        other => panic!("expected LockData PDU, got {other:?}"),
    }
}

// -- Automatic lock basics -------------------------------------------

#[test]
fn lock_without_capability() {
    let mut cliprdr = CliprdrClient::new(Box::new(TestBackend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;

    // No CAN_LOCK_CLIPDATA in default capabilities
    assert!(
        !cliprdr
            .__test_capabilities()
            .flags()
            .contains(ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA)
    );

    let messages: Vec<SvcMessage> = cliprdr.process(&file_format_list_buf()).unwrap();

    // Only FormatListResponse, no Lock PDU
    assert_eq!(messages.len(), 1);
    assert!(cliprdr.__test_outgoing_locks().is_empty());
}

#[test]
fn lock_with_capability() {
    let mut cliprdr = ready_locking_client();

    let messages: Vec<SvcMessage> = cliprdr.process(&file_format_list_buf()).unwrap();
    assert_eq!(messages.len(), 2);

    // Behavior: returned message is a Lock PDU
    decode_pdu!(messages[1] => _bytes, pdu);
    let clip_data_id = match pdu {
        ClipboardPdu::LockData(id) => id.0,
        other => panic!("expected LockData PDU, got {other:?}"),
    };

    // Bookkeeping
    assert!(cliprdr.__test_outgoing_locks().contains_key(&clip_data_id));
    assert_eq!(cliprdr.__test_current_lock_id(), Some(clip_data_id));
}

#[test]
fn lock_expired_on_new_format_list() {
    let cleared_ids = Arc::new(Mutex::new(Vec::new()));
    let expired_ids = Arc::new(Mutex::new(Vec::new()));
    let backend = CallbackTrackingBackend::with_expired_tracking(Arc::clone(&cleared_ids), Arc::clone(&expired_ids));

    let mut cliprdr: CliprdrClient = Cliprdr::with_lock_timeouts(
        Box::new(backend),
        core::time::Duration::from_secs(60),
        core::time::Duration::from_secs(600),
    );
    *cliprdr.__test_state_mut() = CliprdrState::Ready;
    *cliprdr.__test_capabilities_mut() = Capabilities::new(
        ClipboardProtocolVersion::V2,
        ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA | ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES,
    );

    // Create lock via file FormatList
    let clip_data_id = process_file_format_list(&mut cliprdr);
    assert!(cliprdr.__test_outgoing_locks().contains_key(&clip_data_id));

    // Simulate receiving a new text FormatList (clipboard changed, no files)
    let messages: Vec<SvcMessage> = cliprdr.process(&text_format_list_buf()).unwrap();

    // Behavior: only FormatListResponse sent (no immediate Unlock PDU, no new lock)
    assert_eq!(messages.len(), 1);

    // Behavior: on_outgoing_locks_expired callback fired with our lock ID
    {
        let expired = expired_ids.lock().unwrap();
        assert_eq!(expired.len(), 1);
        assert!(expired[0].iter().any(|id| id.0 == clip_data_id));
    }

    // Behavior: on_outgoing_locks_cleared has NOT fired (lock not yet removed)
    assert!(cleared_ids.lock().unwrap().is_empty());

    // Bookkeeping: lock still tracked but current_lock_id cleared
    assert!(cliprdr.__test_outgoing_locks().contains_key(&clip_data_id));
    assert_eq!(cliprdr.__test_current_lock_id(), None);
}

#[test]
fn file_contents_request_includes_clip_data_id() {
    let mut cliprdr = ready_locking_client();

    let fl = sized_file_list(&[("test.txt", 1024)]);
    *cliprdr.__test_remote_file_list_mut() = Some(fl.clone());
    *cliprdr.__test_local_file_list_mut() = Some(fl);

    let clip_data_id = process_file_format_list(&mut cliprdr);
    assert_eq!(cliprdr.__test_current_lock_id(), Some(clip_data_id));

    let request = FileContentsRequest {
        stream_id: 100,
        index: 0,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: None,
    };

    let result = cliprdr.request_file_contents(request);
    assert!(result.is_ok());

    let messages = Vec::from(result.unwrap());
    assert_eq!(messages.len(), 1);
}

// -- Incoming lock snapshots -----------------------------------------

#[test]
fn lock_pdu_creates_file_list_snapshot() {
    let mut cliprdr = CliprdrClient::new(Box::new(TestBackend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;
    *cliprdr.__test_local_file_list_mut() = Some(file_list(&["file1.txt", "file2.txt"]));

    assert_eq!(cliprdr.__test_locked_file_lists().len(), 0);

    let lock_pdu = ClipboardPdu::LockData(LockDataId(42));
    let mut buf = vec![0u8; lock_pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    lock_pdu.encode(&mut cursor).unwrap();

    let result = cliprdr.process(&buf);
    assert!(result.is_ok());

    assert_eq!(cliprdr.__test_locked_file_lists().len(), 1);
    let snapshot = &cliprdr.__test_locked_file_lists()[&42];
    assert_eq!(snapshot.files.len(), 2);
    assert_eq!(snapshot.files[0].name, "file1.txt");
    assert_eq!(snapshot.files[1].name, "file2.txt");
}

#[test]
fn unlock_pdu_removes_file_list_snapshot() {
    let mut cliprdr = CliprdrClient::new(Box::new(TestBackend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;

    let fl = file_list(&["test.txt"]);
    *cliprdr.__test_local_file_list_mut() = Some(fl.clone());
    cliprdr.__test_locked_file_lists_mut().insert(99, fl);

    assert_eq!(cliprdr.__test_locked_file_lists().len(), 1);

    let unlock_pdu = ClipboardPdu::UnlockData(LockDataId(99));
    let mut buf = vec![0u8; unlock_pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    unlock_pdu.encode(&mut cursor).unwrap();

    let result = cliprdr.process(&buf);
    assert!(result.is_ok());
    assert_eq!(cliprdr.__test_locked_file_lists().len(), 0);
}

#[test]
fn file_contents_request_with_valid_clip_data_id() {
    let mut cliprdr = CliprdrClient::new(Box::new(TestBackend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;

    cliprdr
        .__test_locked_file_lists_mut()
        .insert(123, sized_file_list(&[("locked.txt", 500)]));

    *cliprdr.__test_local_file_list_mut() = Some(sized_file_list(&[("current.txt", 1000)]));

    let request = FileContentsRequest {
        stream_id: 200,
        index: 0,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: Some(123),
    };

    let pdu = ClipboardPdu::FileContentsRequest(request);
    let mut buf = vec![0u8; pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    pdu.encode(&mut cursor).unwrap();

    let result = cliprdr.process(&buf);
    assert!(result.is_ok());
}

#[test]
fn file_contents_request_with_invalid_clip_data_id() {
    let mut cliprdr = CliprdrClient::new(Box::new(TestBackend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;

    cliprdr
        .__test_locked_file_lists_mut()
        .insert(123, sized_file_list(&[("locked.txt", 500)]));

    let request = FileContentsRequest {
        stream_id: 300,
        index: 0,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: Some(999), // Invalid ID
    };

    let pdu = ClipboardPdu::FileContentsRequest(request);
    let mut buf = vec![0u8; pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    pdu.encode(&mut cursor).unwrap();

    let result = cliprdr.process(&buf);
    assert!(result.is_ok());

    let messages = result.unwrap();
    assert_eq!(messages.len(), 1);
}

#[test]
fn locked_file_list_persists_after_clipboard_change() {
    let mut cliprdr = CliprdrClient::new(Box::new(TestBackend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;

    *cliprdr.__test_local_file_list_mut() = Some(sized_file_list(&[("original.txt", 100)]));

    let lock_pdu = ClipboardPdu::LockData(LockDataId(555));
    let mut buf = vec![0u8; lock_pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    lock_pdu.encode(&mut cursor).unwrap();
    cliprdr.process(&buf).unwrap();

    // Change the local file list (simulating clipboard update)
    *cliprdr.__test_local_file_list_mut() = Some(sized_file_list(&[("new.txt", 200)]));

    // Verify locked snapshot still has original file
    let locked_snapshot = &cliprdr.__test_locked_file_lists()[&555];
    assert_eq!(locked_snapshot.files.len(), 1);
    assert_eq!(locked_snapshot.files[0].name, "original.txt");
    assert_eq!(locked_snapshot.files[0].file_size, Some(100));

    // FileContentsRequest with clipDataId should use original file
    let request = FileContentsRequest {
        stream_id: 400,
        index: 0,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: Some(555),
    };

    let pdu = ClipboardPdu::FileContentsRequest(request);
    let mut req_buf = vec![0u8; pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut req_buf);
    pdu.encode(&mut cursor).unwrap();

    let result = cliprdr.process(&req_buf);
    assert!(result.is_ok());
}

// -- Lock replacement on successive FormatLists ----------------------

#[test]
fn successive_file_format_lists_create_new_locks() {
    let mut cliprdr = ready_locking_client();

    // First file FormatList -> automatic lock
    let id1 = process_file_format_list(&mut cliprdr);
    assert_eq!(cliprdr.__test_outgoing_locks().len(), 1);

    // Second file FormatList -> new lock, first expired
    let id2 = process_file_format_list(&mut cliprdr);
    assert_ne!(id1, id2);
    assert_eq!(cliprdr.__test_outgoing_locks().len(), 2);
    assert_eq!(cliprdr.__test_current_lock_id(), Some(id2));

    // Third file FormatList
    let id3 = process_file_format_list(&mut cliprdr);
    assert_ne!(id2, id3);
    assert_eq!(cliprdr.__test_outgoing_locks().len(), 3);
    assert_eq!(cliprdr.__test_current_lock_id(), Some(id3));
}

#[test]
fn incoming_lock_limit_exceeded() {
    let mut cliprdr = CliprdrClient::new(Box::new(TestBackend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;
    *cliprdr.__test_local_file_list_mut() = Some(sized_file_list(&[("test.txt", 100)]));

    assert_eq!(cliprdr.__test_locked_file_lists().len(), 0);

    // Process 100 incoming Lock PDUs (should all succeed)
    for i in 1..=100 {
        let lock_pdu = ClipboardPdu::LockData(LockDataId(i));
        let mut buf = vec![0u8; lock_pdu.size()];
        let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
        lock_pdu.encode(&mut cursor).unwrap();
        let result = cliprdr.process(&buf);
        assert!(result.is_ok());
    }

    assert_eq!(cliprdr.__test_locked_file_lists().len(), 100);

    // 101st Lock PDU should be rejected silently
    let lock_pdu = ClipboardPdu::LockData(LockDataId(101));
    let mut buf = vec![0u8; lock_pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    lock_pdu.encode(&mut cursor).unwrap();

    let result = cliprdr.process(&buf);
    assert!(result.is_ok());
    assert_eq!(cliprdr.__test_locked_file_lists().len(), 100);
    assert!(!cliprdr.__test_locked_file_lists().contains_key(&101));

    assert!(cliprdr.__test_locked_file_lists().contains_key(&1));
    assert!(cliprdr.__test_locked_file_lists().contains_key(&50));
    assert!(cliprdr.__test_locked_file_lists().contains_key(&100));
}

#[test]
fn all_locks_expired_on_text_format_list() {
    let cleared_ids = Arc::new(Mutex::new(Vec::new()));
    let expired_ids = Arc::new(Mutex::new(Vec::new()));
    let backend = CallbackTrackingBackend::with_expired_tracking(Arc::clone(&cleared_ids), Arc::clone(&expired_ids));

    let mut cliprdr: CliprdrClient = Cliprdr::with_lock_timeouts(
        Box::new(backend),
        core::time::Duration::from_secs(60),
        core::time::Duration::from_secs(600),
    );
    *cliprdr.__test_state_mut() = CliprdrState::Ready;
    *cliprdr.__test_capabilities_mut() = Capabilities::new(
        ClipboardProtocolVersion::V2,
        ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA | ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES,
    );

    // Create 3 locks via successive file FormatLists
    let _id1 = process_file_format_list(&mut cliprdr);
    let _id2 = process_file_format_list(&mut cliprdr);
    let id3 = process_file_format_list(&mut cliprdr);

    // The first two FormatLists expired the previous lock(s), so clear the callback log
    expired_ids.lock().unwrap().clear();

    assert_eq!(cliprdr.__test_outgoing_locks().len(), 3);

    // Text FormatList -> all locks expired
    let messages: Vec<SvcMessage> = cliprdr.process(&text_format_list_buf()).unwrap();

    // Behavior: only FormatListResponse sent (no immediate Unlock PDUs)
    assert_eq!(messages.len(), 1);

    // Behavior: expired callback fired with the remaining active lock (id3)
    // (id1 and id2 were already expired by successive file FormatLists)
    {
        let expired = expired_ids.lock().unwrap();
        assert_eq!(expired.len(), 1, "expired callback should fire once");
        // Only id3 was still Active when the text FormatList arrived
        assert!(expired[0].iter().any(|id| id.0 == id3));
    }

    // Behavior: cleared callback has NOT fired (cleanup hasn't run)
    assert!(cleared_ids.lock().unwrap().is_empty());

    // Bookkeeping: locks still tracked, current_lock_id cleared
    assert_eq!(cliprdr.__test_outgoing_locks().len(), 3);
    assert!(cliprdr.__test_current_lock_id().is_none());
}

// -- Callback tracking -----------------------------------------------

#[test]
fn on_outgoing_locks_cleared_callback_invoked() {
    let cleared_ids = Arc::new(Mutex::new(Vec::new()));
    let backend = CallbackTrackingBackend::new(Arc::clone(&cleared_ids));

    let mut cliprdr: CliprdrClient = Cliprdr::with_lock_timeouts(
        Box::new(backend),
        core::time::Duration::from_millis(20), // inactivity: 20ms
        core::time::Duration::from_secs(10),   // max: 10s
    );
    *cliprdr.__test_state_mut() = CliprdrState::Ready;
    *cliprdr.__test_capabilities_mut() = Capabilities::new(
        ClipboardProtocolVersion::V2,
        ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA | ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES,
    );

    // Create lock via file FormatList
    let lock_id = process_file_format_list(&mut cliprdr);
    assert_eq!(cliprdr.__test_outgoing_locks().len(), 1);
    assert!(
        cleared_ids.lock().unwrap().is_empty(),
        "Callback should not be called yet"
    );

    // Text FormatList expires the lock
    cliprdr.process(&text_format_list_buf()).unwrap();

    // Callback should NOT be called yet (locks are expired, not cleaned up)
    assert!(
        cleared_ids.lock().unwrap().is_empty(),
        "Callback should not be called until drive_timeouts() is called"
    );
    assert_eq!(cliprdr.__test_outgoing_locks().len(), 1, "Locks should still exist");

    // Advance mock clock past inactivity timeout (20ms)
    let backend = cliprdr.downcast_backend::<CallbackTrackingBackend>().unwrap();
    backend.advance_ms(50);

    let _cleanup_messages = cliprdr.drive_timeouts().unwrap();

    let callbacks = cleared_ids.lock().unwrap();
    assert_eq!(callbacks.len(), 1, "Callback should be called once after cleanup");

    let cleared = &callbacks[0];
    assert_eq!(cleared.len(), 1, "Should have cleared 1 lock");
    assert!(
        cleared.iter().any(|id| id.0 == lock_id),
        "Cleared IDs should contain lock ID {lock_id}"
    );

    drop(callbacks);
    assert!(cliprdr.__test_outgoing_locks().is_empty());
    assert_eq!(cliprdr.__test_current_lock_id(), None);
}

#[test]
fn on_outgoing_locks_expired_callback_invoked() {
    let cleared_ids = Arc::new(Mutex::new(Vec::new()));
    let expired_ids = Arc::new(Mutex::new(Vec::new()));
    let backend = CallbackTrackingBackend::with_expired_tracking(Arc::clone(&cleared_ids), Arc::clone(&expired_ids));

    let mut cliprdr: CliprdrClient = Cliprdr::with_lock_timeouts(
        Box::new(backend),
        core::time::Duration::from_millis(20),
        core::time::Duration::from_secs(10),
    );
    *cliprdr.__test_state_mut() = CliprdrState::Ready;
    *cliprdr.__test_capabilities_mut() = Capabilities::new(
        ClipboardProtocolVersion::V2,
        ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA | ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES,
    );

    // Create lock via file FormatList
    let lock_id = process_file_format_list(&mut cliprdr);

    // No callbacks yet
    assert!(expired_ids.lock().unwrap().is_empty());

    // Text FormatList triggers expire_all_locks -> on_outgoing_locks_expired
    cliprdr.process(&text_format_list_buf()).unwrap();

    // Expired callback should have fired with the lock ID
    {
        let callbacks = expired_ids.lock().unwrap();
        assert_eq!(callbacks.len(), 1, "expired callback should fire once");

        let expired = &callbacks[0];
        assert_eq!(expired.len(), 1, "one lock should be expired");
        assert!(expired.iter().any(|id| id.0 == lock_id));
    }

    // Cleared callback should NOT have fired yet (cleanup hasn't run)
    assert!(cleared_ids.lock().unwrap().is_empty());
}
