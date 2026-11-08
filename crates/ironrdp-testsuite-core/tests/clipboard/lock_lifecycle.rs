//! Tests for clipboard lock lifecycle: automatic lock creation via FormatList,
//! incoming lock snapshots, concurrent locks, incoming lock limits, and
//! lock expiry on clipboard change.
//!
//! Migrated from `ironrdp-cliprdr/src/lib.rs` inline `#[cfg(test)]` module.
//! Behavior assertions use the public API (returned PDUs, backend callbacks);
//! bookkeeping assertions (internal lock counts) use the `__test` feature gate.

use core::time::Duration;
use std::sync::{Arc, Mutex};

use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::FileContentsResponse;
use ironrdp_cliprdr::pdu::{
    Capabilities, ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags,
    ClipboardPdu, ClipboardProtocolVersion, FileContentsFlags, FileContentsRequest, FileDescriptor, FormatList,
    LockDataId, PackedFileList,
};
use ironrdp_cliprdr::{Cliprdr, CliprdrClient, CliprdrState};
use ironrdp_core::Encode as _;
use ironrdp_svc::{SvcMessage, SvcProcessor as _};

use ironrdp_testsuite_core::clipboard::{
    CallbackTrackingBackend, LockingBackend, TestBackend, TimedRecordingBackend, init_ready_locking_client,
    set_remote_file_list,
};

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
    into_ready_locking_client(CliprdrClient::new(Box::new(LockingBackend::new())))
}

/// Helper: like [`ready_locking_client`], with the given backend and lock timeouts.
fn ready_locking_client_with_timeouts(
    backend: Box<dyn CliprdrBackend>,
    inactivity_timeout: Duration,
    max_lifetime: Duration,
) -> CliprdrClient {
    into_ready_locking_client(Cliprdr::with_lock_timeouts(backend, inactivity_timeout, max_lifetime))
}

/// Helper: put `cliprdr` in Ready state with locking and file streams negotiated.
fn into_ready_locking_client(mut cliprdr: CliprdrClient) -> CliprdrClient {
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

    let mut cliprdr: CliprdrClient =
        Cliprdr::with_lock_timeouts(Box::new(backend), Duration::from_secs(60), Duration::from_secs(600));
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

// oracle: mutant evicting the newest lock, dropping the new lock at capacity, or skipping on_unlock for the evicted lock
#[test]
fn incoming_lock_limit_exceeded() {
    let responses = Arc::new(Mutex::new(Vec::new()));
    let unlocks = Arc::new(Mutex::new(Vec::new()));
    let backend = TimedRecordingBackend::new(Arc::clone(&responses), Arc::clone(&unlocks));
    let mut cliprdr = CliprdrClient::new(Box::new(backend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;
    *cliprdr.__test_local_file_list_mut() = Some(sized_file_list(&[("test.txt", 100)]));

    assert_eq!(cliprdr.__test_locked_file_lists().len(), 0);

    // Process 100 incoming Lock PDUs (should all succeed).
    for i in 1..=100 {
        let lock_pdu = ClipboardPdu::LockData(LockDataId(i));
        let mut buf = vec![0u8; lock_pdu.size()];
        let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
        lock_pdu.encode(&mut cursor).unwrap();
        let result = cliprdr.process(&buf);
        assert!(result.is_ok());
    }

    assert_eq!(cliprdr.__test_locked_file_lists().len(), 100);
    assert!(unlocks.lock().unwrap().is_empty());

    // 101st Lock PDU evicts the oldest lock and notifies the backend via on_unlock.
    let lock_pdu = ClipboardPdu::LockData(LockDataId(101));
    let mut buf = vec![0u8; lock_pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    lock_pdu.encode(&mut cursor).unwrap();

    let result = cliprdr.process(&buf);
    assert!(result.is_ok());
    assert_eq!(cliprdr.__test_locked_file_lists().len(), 100);
    assert!(!cliprdr.__test_locked_file_lists().contains_key(&1));
    assert_eq!(*unlocks.lock().unwrap(), [1]);

    assert!(cliprdr.__test_locked_file_lists().contains_key(&2));
    assert!(cliprdr.__test_locked_file_lists().contains_key(&50));
    assert!(cliprdr.__test_locked_file_lists().contains_key(&100));
    assert!(cliprdr.__test_locked_file_lists().contains_key(&101));
}

// oracle: mutant leaving a relocked clipDataId at its old FIFO position so it is evicted before older locks
#[test]
fn relocking_an_existing_id_refreshes_its_eviction_order() {
    let responses = Arc::new(Mutex::new(Vec::new()));
    let unlocks = Arc::new(Mutex::new(Vec::new()));
    let backend = TimedRecordingBackend::new(Arc::clone(&responses), Arc::clone(&unlocks));
    let mut cliprdr = CliprdrClient::new(Box::new(backend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;
    *cliprdr.__test_local_file_list_mut() = Some(sized_file_list(&[("test.txt", 100)]));

    for i in 1..=100 {
        let lock_pdu = ClipboardPdu::LockData(LockDataId(i));
        let mut buf = vec![0u8; lock_pdu.size()];
        let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
        lock_pdu.encode(&mut cursor).unwrap();
        cliprdr.process(&buf).unwrap();
    }

    // Re-locking id 1 refreshes its position so id 2 becomes the oldest lock.
    let relock_pdu = ClipboardPdu::LockData(LockDataId(1));
    let mut buf = vec![0u8; relock_pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    relock_pdu.encode(&mut cursor).unwrap();
    cliprdr.process(&buf).unwrap();
    assert!(unlocks.lock().unwrap().is_empty());

    let lock_pdu = ClipboardPdu::LockData(LockDataId(101));
    let mut buf = vec![0u8; lock_pdu.size()];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    lock_pdu.encode(&mut cursor).unwrap();
    cliprdr.process(&buf).unwrap();

    assert_eq!(cliprdr.__test_locked_file_lists().len(), 100);
    assert!(cliprdr.__test_locked_file_lists().contains_key(&1));
    assert!(!cliprdr.__test_locked_file_lists().contains_key(&2));
    assert!(cliprdr.__test_locked_file_lists().contains_key(&101));
    assert_eq!(*unlocks.lock().unwrap(), [2]);
}

// oracle: mutant desynchronizing locked_file_list_order from locked_file_lists across lock, relock, unlock, or eviction
#[test]
fn locked_file_list_order_matches_active_snapshots_across_operations() {
    use std::collections::VecDeque;

    use ironrdp_cliprdr::MAX_LOCKED_FILE_LISTS;
    use proptest::prelude::*;

    proptest!(ProptestConfig::with_cases(32), |(
        ops in proptest::collection::vec((any::<bool>(), 1u32..140), 1..250),
    )| {
        let responses = Arc::new(Mutex::new(Vec::new()));
        let unlocks = Arc::new(Mutex::new(Vec::new()));
        let backend = TimedRecordingBackend::new(Arc::clone(&responses), Arc::clone(&unlocks));
        let mut cliprdr = CliprdrClient::new(Box::new(backend));
        *cliprdr.__test_state_mut() = CliprdrState::Ready;
        *cliprdr.__test_local_file_list_mut() = Some(sized_file_list(&[("test.txt", 100)]));

        let mut expected_order: VecDeque<u32> = VecDeque::new();

        for (is_lock, id) in ops {
            let pdu = if is_lock {
                if let Some(pos) = expected_order.iter().position(|&x| x == id) {
                    expected_order.remove(pos);
                } else if expected_order.len() == MAX_LOCKED_FILE_LISTS {
                    expected_order.pop_front();
                }
                expected_order.push_back(id);
                ClipboardPdu::LockData(LockDataId(id))
            } else {
                if let Some(pos) = expected_order.iter().position(|&x| x == id) {
                    expected_order.remove(pos);
                }
                ClipboardPdu::UnlockData(LockDataId(id))
            };

            let mut buf = vec![0u8; pdu.size()];
            let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
            pdu.encode(&mut cursor).unwrap();
            cliprdr.process(&buf).unwrap();

            prop_assert_eq!(cliprdr.__test_locked_file_list_order(), &expected_order);
            prop_assert_eq!(cliprdr.__test_locked_file_lists().len(), expected_order.len());
            for &locked_id in &expected_order {
                prop_assert!(cliprdr.__test_locked_file_lists().contains_key(&locked_id));
            }
        }
    });
}

#[test]
fn all_locks_expired_on_text_format_list() {
    let cleared_ids = Arc::new(Mutex::new(Vec::new()));
    let expired_ids = Arc::new(Mutex::new(Vec::new()));
    let backend = CallbackTrackingBackend::with_expired_tracking(Arc::clone(&cleared_ids), Arc::clone(&expired_ids));

    let mut cliprdr: CliprdrClient =
        Cliprdr::with_lock_timeouts(Box::new(backend), Duration::from_secs(60), Duration::from_secs(600));
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
        Duration::from_millis(20), // inactivity: 20ms
        Duration::from_secs(10),   // max: 10s
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

    let mut cliprdr: CliprdrClient =
        Cliprdr::with_lock_timeouts(Box::new(backend), Duration::from_millis(20), Duration::from_secs(10));
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

// -- Taking clipboard ownership releases stale download locks --------

/// When the local side initiates a file copy (an upload), it takes clipboard
/// ownership — so the outgoing locks placed for downloads from the previous owner are
/// now stale. They must be released with `Unlock` PDUs that PRECEDE our `FormatList` on
/// the wire; otherwise the server keeps tracking a lock for a download that will never
/// complete, which desyncs its clipboard state.
#[test]
fn initiate_file_copy_releases_outgoing_download_locks() {
    let mut cliprdr = ready_locking_client();

    // Two remote file lists => two outgoing download locks (e.g. two in-flight downloads).
    let lock1 = process_file_format_list(&mut cliprdr);
    let lock2 = process_file_format_list(&mut cliprdr);
    assert_eq!(cliprdr.__test_outgoing_locks().len(), 2);

    let messages: Vec<SvcMessage> = cliprdr
        .initiate_file_copy(vec![FileDescriptor::new("upload.txt")])
        .unwrap()
        .into();

    // Every outgoing lock is released and the current lock id is cleared.
    assert!(
        cliprdr.__test_outgoing_locks().is_empty(),
        "outgoing locks must be released when taking clipboard ownership"
    );
    assert_eq!(cliprdr.__test_current_lock_id(), None);

    // Wire order: an Unlock for each held lock, THEN our FormatList last.
    assert_eq!(messages.len(), 3, "expected 2 Unlock PDUs + 1 FormatList");

    decode_pdu!(messages[0] => _b0, pdu0);
    let id0 = match pdu0 {
        ClipboardPdu::UnlockData(id) => id.0,
        other => panic!("expected UnlockData first, got {other:?}"),
    };
    decode_pdu!(messages[1] => _b1, pdu1);
    let id1 = match pdu1 {
        ClipboardPdu::UnlockData(id) => id.0,
        other => panic!("expected UnlockData second, got {other:?}"),
    };
    let mut unlocked = [id0, id1];
    unlocked.sort_unstable();
    let mut expected = [lock1, lock2];
    expected.sort_unstable();
    assert_eq!(unlocked, expected, "both download locks must be unlocked");

    decode_pdu!(messages[2] => _b2, last_pdu);
    assert!(
        matches!(last_pdu, ClipboardPdu::FormatList(_)),
        "FormatList must come after the Unlock PDUs, got {last_pdu:?}"
    );
}

/// A client downloading a one-file remote list, with a SIZE request (stream 1) outstanding
/// under the list's lock, and a mock clock at 0.
struct Download {
    cliprdr: CliprdrClient,
    lock: u32,
    cleared: Arc<Mutex<Vec<Vec<LockDataId>>>>,
    expired: Arc<Mutex<Vec<Vec<LockDataId>>>>,
}

impl Download {
    /// Uses the default 60 s inactivity timeout, so a release sooner comes from the short
    /// timeout for locks kept past a local copy.
    fn start() -> Self {
        let cleared = Arc::new(Mutex::new(Vec::new()));
        let expired = Arc::new(Mutex::new(Vec::new()));
        let backend = CallbackTrackingBackend::with_expired_tracking(Arc::clone(&cleared), Arc::clone(&expired));
        let mut cliprdr = ready_locking_client_with_timeouts(
            Box::new(backend),
            Duration::from_secs(60),
            Duration::from_secs(2 * 3600),
        );
        set_remote_file_list(&mut cliprdr, vec![FileDescriptor::new("a.txt")]);
        let lock = cliprdr
            .__test_current_lock_id()
            .expect("a file Format List must create a lock when CAN_LOCK_CLIPDATA is negotiated");
        let request = FileContentsRequest {
            stream_id: 1,
            index: 0,
            flags: FileContentsFlags::SIZE,
            position: 0,
            requested_size: 8,
            data_id: Some(lock),
        };
        cliprdr.request_file_contents(request).unwrap();
        Self {
            cliprdr,
            lock,
            cleared,
            expired,
        }
    }

    fn advance_ms(&mut self, ms: u64) {
        self.cliprdr
            .downcast_backend::<CallbackTrackingBackend>()
            .unwrap()
            .advance_ms(ms);
    }

    /// The remote answers the outstanding SIZE request.
    fn respond(&mut self) {
        let pdu = ClipboardPdu::FileContentsResponse(FileContentsResponse::new_size_response(1, 1024));
        self.cliprdr.process(&ironrdp_core::encode_vec(&pdu).unwrap()).unwrap();
    }

    /// The local side copies a file, taking clipboard ownership.
    fn copy_local_file(&mut self) -> Vec<SvcMessage> {
        self.cliprdr
            .initiate_file_copy(vec![FileDescriptor::new("upload.txt")])
            .unwrap()
            .into()
    }

    fn drive_timeouts(&mut self) -> Vec<SvcMessage> {
        self.cliprdr.drive_timeouts().unwrap().into()
    }
}

/// The clipDataIds of the Unlock PDUs among `messages`, in order.
fn unlocked_ids(messages: &[SvcMessage]) -> Vec<u32> {
    messages
        .iter()
        .filter_map(|message| {
            decode_pdu!(message => _bytes, pdu);
            match pdu {
                ClipboardPdu::UnlockData(id) => Some(id.0),
                _ => None,
            }
        })
        .collect()
}

/// A lock with a FileContentsRequest still outstanding survives a local file copy, however
/// long the request has been waiting, because releasing it would make the remote drop the
/// File Stream data the request is reading.
/// The backend is not told the lock expired, because a backend aborts the download on that
/// callback.
#[test]
fn initiate_file_copy_keeps_a_lock_with_an_outstanding_request() {
    let mut download = Download::start();
    download.advance_ms(30_000);

    let messages = download.copy_local_file();

    // oracle: spec MS-RDPECLIP §2.2.4.1
    assert_eq!(
        unlocked_ids(&messages),
        Vec::<u32>::new(),
        "the lock in use is not unlocked"
    );
    assert_eq!(messages.len(), 1, "only the FormatList goes out");
    // oracle: mutant reporting a kept lock expired, which makes the daemon backend abort the download
    assert!(download.expired.lock().unwrap().is_empty());

    // Quiet well past the kept-lock timeout, but the request is still outstanding.
    download.advance_ms(20_000);
    let messages = download.drive_timeouts();

    // oracle: spec MS-RDPECLIP §2.2.4.1
    assert_eq!(
        unlocked_ids(&messages),
        Vec::<u32>::new(),
        "the sweep keeps a kept lock while a request under it is outstanding"
    );
    assert!(download.cleared.lock().unwrap().is_empty());
}

/// A lock whose download just received a response, with nothing outstanding, survives a
/// local file copy, because the download's next request may follow.
/// It is released once it has gone quiet for longer than that gap, measured from the
/// response.
// oracle: mutant measuring quiet from the request instead of the response, keeping the lock for the full inactivity timeout, or not reporting it cleared
#[test]
fn a_kept_lock_is_released_once_its_download_goes_quiet() {
    let mut download = Download::start();
    download.advance_ms(8_000);
    download.respond();
    download.advance_ms(1_000);

    let messages = download.copy_local_file();

    // oracle: spec MS-RDPECLIP §2.2.4.1
    assert_eq!(
        unlocked_ids(&messages),
        Vec::<u32>::new(),
        "the lock in use is not unlocked"
    );
    assert!(download.expired.lock().unwrap().is_empty());

    // 6 s after the response (14 s after the request): still within the gap.
    download.advance_ms(5_000);
    let messages = download.drive_timeouts();
    assert_eq!(unlocked_ids(&messages), Vec::<u32>::new());
    assert!(download.cleared.lock().unwrap().is_empty());

    // 12 s after the response: the download has gone quiet.
    download.advance_ms(6_000);
    let messages = download.drive_timeouts();
    assert_eq!(unlocked_ids(&messages), vec![download.lock]);
    assert_eq!(*download.cleared.lock().unwrap(), vec![vec![LockDataId(download.lock)]]);
}

/// A lock whose download finished longer ago than the gap between two requests is not in
/// use, so a local file copy unlocks it before the FormatList.
// oracle: mutant keeping every lock that was ever used, or keeping a lock past a finished download
#[test]
fn initiate_file_copy_unlocks_a_lock_whose_download_finished() {
    let mut download = Download::start();
    download.respond();
    download.advance_ms(11_000);

    let messages = download.copy_local_file();

    assert_eq!(messages.len(), 2, "expected 1 Unlock PDU + 1 FormatList");
    assert_eq!(unlocked_ids(&messages[..1]), vec![download.lock]);
    decode_pdu!(messages[1] => _bytes, last_pdu);
    assert!(
        matches!(last_pdu, ClipboardPdu::FormatList(_)),
        "FormatList must come after the Unlock PDU, got {last_pdu:?}"
    );
    assert_eq!(*download.cleared.lock().unwrap(), vec![vec![LockDataId(download.lock)]]);
    assert!(download.expired.lock().unwrap().is_empty());
}

/// With no outgoing locks held, `initiate_file_copy` sends only the `FormatList`
/// (no spurious `Unlock`).
#[test]
fn initiate_file_copy_without_locks_sends_only_format_list() {
    let mut cliprdr = ready_locking_client();
    assert!(cliprdr.__test_outgoing_locks().is_empty());

    let messages: Vec<SvcMessage> = cliprdr
        .initiate_file_copy(vec![FileDescriptor::new("upload.txt")])
        .unwrap()
        .into();

    assert_eq!(messages.len(), 1, "expected only a FormatList when no locks are held");
    decode_pdu!(messages[0] => _b0, pdu);
    assert!(
        matches!(pdu, ClipboardPdu::FormatList(_)),
        "expected FormatList, got {pdu:?}"
    );
}

// ── Receiver-side locked file list snapshots ────────────────────────

/// [MS-RDPECLIP] 3.1.5.4.6 - a request carrying a `clipDataId` is validated
/// against the list that lock covers.
///
/// A Lock PDU asks the remote to keep its File Stream data alive across a
/// clipboard change, and 3.1.5.4.6 requires that data to service a request
/// carrying the id, while 3.1.5.4.5 has the index come from a File List.
/// Validating such a request against the *current* remote file list defeats
/// that: replacing a two-file selection with a one-file one made an
/// already-issued request for index 1 fail, even though the locked data still
/// had the file.
#[test]
fn a_locked_request_validates_against_the_list_its_lock_covers() {
    let mut cliprdr = init_ready_locking_client();

    // Selection A: two files, locked.
    set_remote_file_list(
        &mut cliprdr,
        vec![FileDescriptor::new("a.txt"), FileDescriptor::new("b.txt")],
    );
    let lock_for_a = cliprdr
        .__test_current_lock_id()
        .expect("a file Format List must create a lock when CAN_LOCK_CLIPDATA is negotiated");

    // Selection B replaces it: one file, under a new lock. A's lock expires but
    // is deliberately kept, because transfers from A may still be in flight.
    set_remote_file_list(&mut cliprdr, vec![FileDescriptor::new("c.txt")]);
    assert_ne!(
        cliprdr.__test_current_lock_id(),
        Some(lock_for_a),
        "selection B must be covered by its own lock"
    );

    // A's second file, requested under A's lock.
    let under_a = FileContentsRequest {
        stream_id: 1,
        index: 1,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: Some(lock_for_a),
    };
    assert!(
        cliprdr.request_file_contents(under_a).is_ok(),
        "index 1 is in bounds for the two-file list this clipDataId locked; \
         rejecting it strands a transfer the remote is still holding data for"
    );

    // Selection B is still bounded by its own list.
    let past_b = FileContentsRequest {
        stream_id: 2,
        index: 1,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: None,
    };
    assert!(
        cliprdr.request_file_contents(past_b).is_err(),
        "index 1 does not exist in the current one-file selection"
    );
}

/// A snapshot lives and dies with its lock.
#[test]
fn a_snapshot_is_released_with_the_lock_it_belongs_to() {
    let mut cliprdr = init_ready_locking_client();

    set_remote_file_list(
        &mut cliprdr,
        vec![FileDescriptor::new("a.txt"), FileDescriptor::new("b.txt")],
    );
    let lock_for_a = cliprdr.__test_current_lock_id().unwrap();

    // Replace the selection, then let the sweep release the expired lock.
    set_remote_file_list(&mut cliprdr, vec![FileDescriptor::new("c.txt")]);
    cliprdr
        .downcast_backend::<LockingBackend>()
        .unwrap()
        .advance_ms(10 * 60 * 1000);
    let _ = cliprdr.drive_timeouts().unwrap();

    assert!(
        !cliprdr.__test_outgoing_locks().contains_key(&lock_for_a),
        "the sweep must have released the expired lock"
    );

    let under_a = FileContentsRequest {
        stream_id: 3,
        index: 1,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: Some(lock_for_a),
    };
    assert!(
        cliprdr.request_file_contents(under_a).is_err(),
        "once the Unlock went out the remote released the data, so the snapshot \
         must not keep validating requests against it"
    );
}
