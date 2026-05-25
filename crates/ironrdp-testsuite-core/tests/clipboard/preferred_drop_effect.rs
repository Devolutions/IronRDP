//! Regression tests for the `Preferred DropEffect` companion format
//! advertised alongside `FileGroupDescriptorW` by [`Cliprdr::initiate_file_copy`].
//!
//! Follows up on [PR #1301]: the merge landed without test coverage and
//! the Copilot review flagged this. The two behaviors covered here:
//!
//! 1. `initiate_file_copy` advertises BOTH `FileGroupDescriptorW` and
//!    `Preferred DropEffect` in the outgoing `FormatList`.
//! 2. A subsequent `FormatDataRequest` for the drop-effect format id is
//!    answered inline with the 4-byte little-endian `DROPEFFECT_COPY`
//!    payload (`0x01 0x00 0x00 0x00`), not forwarded to the backend.
//!
//! [PR #1301]: https://github.com/Devolutions/IronRDP/pull/1301
//! [Copilot review comment]: https://github.com/Devolutions/IronRDP/pull/1301#discussion_r2169280717

use ironrdp_cliprdr::pdu::{ClipboardFormatName, ClipboardPdu, FileDescriptor, FormatDataRequest};
use ironrdp_svc::{SvcMessage, SvcProcessor as _};

use super::test_helpers::init_ready_client;

/// Decode an SvcMessage back into a ClipboardPdu for assertion.
/// Two `let` bindings are required so the byte buffer outlives the
/// borrowing PDU.
macro_rules! decode_pdu {
    ($msg:expr => $bytes:ident, $pdu:ident) => {
        let $bytes = ($msg).encode_unframed_pdu().unwrap();
        let $pdu = ironrdp_core::decode::<ClipboardPdu<'_>>(&$bytes).unwrap();
    };
}

/// `initiate_file_copy` must advertise BOTH `FileGroupDescriptorW`
/// (the file list itself) AND `Preferred DropEffect` (the companion
/// format Windows Explorer pairs with file lists to engage its shell
/// file-copy machinery + native progress dialog).
#[test]
fn initiate_file_copy_advertises_drop_effect_alongside_file_group_descriptor() {
    let mut cliprdr = init_ready_client();

    let files = vec![
        FileDescriptor::new("alpha.txt").with_file_size(100),
        FileDescriptor::new("beta.bin").with_file_size(200),
    ];
    let messages: Vec<SvcMessage> = cliprdr.initiate_file_copy(files).unwrap().into();

    assert_eq!(messages.len(), 1, "initiate_file_copy should send a single FormatList PDU");

    decode_pdu!(&messages[0] => bytes, pdu);
    let ClipboardPdu::FormatList(format_list) = pdu else {
        panic!("expected FormatList PDU, got {pdu:?}");
    };

    let formats = format_list
        .get_formats(true)
        .expect("FormatList should decode under long-format-names");

    let has_file_group_descriptor = formats
        .iter()
        .any(|f| f.name.as_ref().is_some_and(|n| n == &ClipboardFormatName::FILE_LIST));
    let has_drop_effect = formats.iter().any(|f| {
        f.name
            .as_ref()
            .is_some_and(|n| n == &ClipboardFormatName::PREFERRED_DROP_EFFECT)
    });

    assert!(
        has_file_group_descriptor,
        "FormatList must advertise FileGroupDescriptorW; got {formats:#?}"
    );
    assert!(
        has_drop_effect,
        "FormatList must advertise Preferred DropEffect; got {formats:#?}"
    );
}

/// A `FormatDataRequest` for the drop-effect format id is answered
/// inline by `Cliprdr` itself (not forwarded to the backend) with the
/// 4-byte little-endian `DROPEFFECT_COPY = 0x00000001` payload.
#[test]
fn format_data_request_for_drop_effect_returns_dropeffect_copy_inline() {
    let mut cliprdr = init_ready_client();

    // Drive `initiate_file_copy` so the server sets its
    // `local_drop_effect_format_id` and starts recognizing requests for
    // it. Discard the outbound FormatList — covered by the test above.
    let files = vec![FileDescriptor::new("doc.txt").with_file_size(42)];
    let _ = cliprdr.initiate_file_copy(files).unwrap();

    // Find the drop-effect format id from the advertised FormatList by
    // re-driving the call would be circular; instead, key off the format
    // *name* by walking the FormatList we just emitted. Easier and
    // wire-faithful since the remote keys off the name too.
    let initiate_msgs: Vec<SvcMessage> =
        cliprdr.initiate_file_copy(vec![FileDescriptor::new("d.txt").with_file_size(1)]).unwrap().into();
    decode_pdu!(&initiate_msgs[0] => initiate_bytes, initiate_pdu);
    let ClipboardPdu::FormatList(format_list) = initiate_pdu else {
        panic!("expected FormatList");
    };
    let drop_effect_format = format_list
        .get_formats(true)
        .unwrap()
        .into_iter()
        .find(|f| {
            f.name
                .as_ref()
                .is_some_and(|n| n == &ClipboardFormatName::PREFERRED_DROP_EFFECT)
        })
        .expect("Preferred DropEffect must be advertised");
    let drop_effect_id = drop_effect_format.id;

    // Simulate the remote asking for the drop-effect format.
    let request_pdu = ClipboardPdu::FormatDataRequest(FormatDataRequest {
        format: drop_effect_id,
    });
    let request_bytes = ironrdp_core::encode_vec(&request_pdu).unwrap();
    let responses: Vec<SvcMessage> = cliprdr.process(&request_bytes).unwrap();

    assert_eq!(
        responses.len(),
        1,
        "drop-effect FormatDataRequest should be answered inline with one FormatDataResponse"
    );

    decode_pdu!(&responses[0] => resp_bytes, resp_pdu);
    let ClipboardPdu::FormatDataResponse(response) = resp_pdu else {
        panic!("expected FormatDataResponse, got {resp_pdu:?}");
    };
    assert!(!response.is_error(), "response must not be an error");

    // [MS-RDPECLIP] Preferred DropEffect payload is a 4-byte u32 LE.
    // `DROPEFFECT_COPY = 0x00000001` is what we advertise as the only
    // value `initiate_file_copy` ever produces.
    assert_eq!(
        response.data(),
        &[0x01, 0x00, 0x00, 0x00],
        "Preferred DropEffect payload must be exactly 4 bytes DROPEFFECT_COPY (LE)"
    );
}
