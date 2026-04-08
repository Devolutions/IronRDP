//! Tests for automatic lock behavior on incoming FormatList processing.
//!
//! Migrated from `ironrdp-cliprdr/src/lib.rs` inline `#[cfg(test)]` module.
//! Behavior assertions use the public API (returned PDUs, message counts);
//! bookkeeping assertions (internal lock counts) use the `__test` feature gate.

use ironrdp_cliprdr::pdu::{
    Capabilities, ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags,
    ClipboardPdu, ClipboardProtocolVersion, FormatList,
};
use ironrdp_cliprdr::{CliprdrClient, CliprdrState};
use ironrdp_core::Encode as _;
use ironrdp_svc::{SvcMessage, SvcProcessor as _};

use super::test_helpers::TestBackend;

/// Introduce `let` bindings for the encoded bytes and the decoded
/// [`ClipboardPdu`] in the caller's scope.  Two names are required so
/// that the byte buffer outlives the borrowing PDU.
macro_rules! decode_pdu {
    ($msg:expr => $bytes:ident, $pdu:ident) => {
        let $bytes = ($msg).encode_unframed_pdu().unwrap();
        let $pdu = ironrdp_core::decode::<ClipboardPdu<'_>>(&$bytes).unwrap();
    };
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

/// Helper: set up a CliprdrClient with locking capability in Ready state.
fn ready_client(can_lock: bool) -> CliprdrClient {
    let mut cliprdr = CliprdrClient::new(Box::new(TestBackend));
    *cliprdr.__test_state_mut() = CliprdrState::Ready;

    let mut flags = ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES;
    if can_lock {
        flags |= ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA;
    }
    *cliprdr.__test_capabilities_mut() = Capabilities::new(ClipboardProtocolVersion::V2, flags);

    cliprdr
}

#[test]
fn automatic_lock() {
    let mut cliprdr = ready_client(true);
    let buf = file_format_list_buf();

    let messages: Vec<SvcMessage> = cliprdr.process(&buf).unwrap();

    // Behavior: FormatListResponse + Lock PDU
    assert_eq!(messages.len(), 2, "should have FormatListResponse and Lock");
    decode_pdu!(messages[0] => _bytes0, pdu0);
    assert!(matches!(pdu0, ClipboardPdu::FormatListResponse(_)));

    decode_pdu!(messages[1] => _bytes1, lock_pdu);
    let lock_id = match lock_pdu {
        ClipboardPdu::LockData(id) => id,
        other => panic!("expected LockData PDU, got {other:?}"),
    };

    // Bookkeeping: one lock tracked, IDs consistent
    assert_eq!(cliprdr.__test_outgoing_locks().len(), 1);
    assert!(cliprdr.__test_outgoing_locks().contains_key(&lock_id.0));
}

#[test]
fn lock_without_capability() {
    let mut cliprdr = ready_client(false);
    let buf = file_format_list_buf();

    let messages: Vec<SvcMessage> = cliprdr.process(&buf).unwrap();

    // Behavior: no Lock PDU when CAN_LOCK_CLIPDATA not negotiated
    assert_eq!(messages.len(), 1, "should only have FormatListResponse");
    decode_pdu!(messages[0] => _bytes, pdu);
    assert!(matches!(pdu, ClipboardPdu::FormatListResponse(_)));

    // Bookkeeping
    assert_eq!(cliprdr.__test_outgoing_locks().len(), 0);
}

#[test]
fn new_format_list_replaces_old_lock() {
    let mut cliprdr = ready_client(true);

    // First FormatList with files -> gets a Lock PDU
    let msgs1: Vec<SvcMessage> = cliprdr.process(&file_format_list_buf()).unwrap();
    assert_eq!(msgs1.len(), 2);
    decode_pdu!(msgs1[1] => _bytes1, pdu1);
    let first_lock_id = match pdu1 {
        ClipboardPdu::LockData(id) => id,
        other => panic!("expected LockData, got {other:?}"),
    };

    // Second FormatList with files -> new Lock PDU, old lock expired
    let msgs2: Vec<SvcMessage> = cliprdr.process(&file_format_list_buf()).unwrap();
    assert_eq!(msgs2.len(), 2, "should have FormatListResponse + new Lock");
    decode_pdu!(msgs2[1] => _bytes2, pdu2);
    let second_lock_id = match pdu2 {
        ClipboardPdu::LockData(id) => id,
        other => panic!("expected LockData, got {other:?}"),
    };

    // Behavior: different lock IDs issued
    assert_ne!(
        first_lock_id, second_lock_id,
        "lock ID should change on clipboard change"
    );

    // Bookkeeping: both locks tracked (first expired, second active)
    assert_eq!(cliprdr.__test_outgoing_locks().len(), 2);
    assert!(cliprdr.__test_outgoing_locks().contains_key(&second_lock_id.0));
}

#[test]
fn no_lock_for_non_file_formats() {
    let mut cliprdr = ready_client(true);
    let buf = text_format_list_buf();

    let messages: Vec<SvcMessage> = cliprdr.process(&buf).unwrap();

    // Behavior: no Lock PDU for text-only clipboard
    assert_eq!(messages.len(), 1, "should only have FormatListResponse");
    decode_pdu!(messages[0] => _bytes, pdu);
    assert!(matches!(pdu, ClipboardPdu::FormatListResponse(_)));

    // Bookkeeping
    assert_eq!(cliprdr.__test_outgoing_locks().len(), 0);
}
