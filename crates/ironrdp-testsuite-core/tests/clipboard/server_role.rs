//! Server-role tests for the CLIPRDR channel.
//!
//! The server has different initialization behavior from the client:
//! it sends Capabilities + MonitorReady in `start()`, and transitions
//! to Ready on receiving a FormatList (not FormatListResponse).

use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId, ClipboardPdu, FormatList, FormatListResponse};
use ironrdp_cliprdr::{CliprdrServer, CliprdrState};
use ironrdp_svc::SvcProcessor as _;

use super::test_helpers::TestBackend;

/// Helper: decode a SvcMessage back into a ClipboardPdu for assertion.
macro_rules! decode_pdu {
    ($msg:expr => $bytes:ident, $pdu:ident) => {
        let $bytes = ($msg).encode_unframed_pdu().unwrap();
        let $pdu = ironrdp_core::decode::<ClipboardPdu<'_>>(&$bytes).unwrap();
    };
}

#[test]
fn server_start_sends_capabilities_and_monitor_ready() {
    let mut server = CliprdrServer::new(Box::new(TestBackend));

    let messages = server.start().unwrap();
    assert_eq!(messages.len(), 2, "start() should send Capabilities + MonitorReady");

    // First PDU: Capabilities
    decode_pdu!(&messages[0] => bytes0, pdu0);
    assert!(
        matches!(pdu0, ClipboardPdu::Capabilities(_)),
        "first PDU should be Capabilities, got {pdu0:?}"
    );

    // Second PDU: MonitorReady
    decode_pdu!(&messages[1] => bytes1, pdu1);
    assert!(
        matches!(pdu1, ClipboardPdu::MonitorReady),
        "second PDU should be MonitorReady, got {pdu1:?}"
    );
}

#[test]
fn server_transitions_to_ready_on_format_list() {
    let mut server = CliprdrServer::new(Box::new(TestBackend));
    let _ = server.start().unwrap();

    // Server should be in Initialization
    assert_eq!(*server.__test_state(), CliprdrState::Initialization);

    // Client sends a FormatList
    let format_list = FormatList::new_unicode(&[ClipboardFormat::new(ClipboardFormatId::new(13))], true).unwrap();
    let bytes = ironrdp_core::encode_vec(&ClipboardPdu::FormatList(format_list)).unwrap();
    let response = server.process(&bytes).unwrap();

    // Server should now be Ready
    assert_eq!(*server.__test_state(), CliprdrState::Ready);

    // Response should contain FormatListResponse::Ok
    assert!(!response.is_empty(), "server should respond to FormatList");
    decode_pdu!(&response[0] => resp_bytes, resp_pdu);
    assert!(
        matches!(resp_pdu, ClipboardPdu::FormatListResponse(FormatListResponse::Ok)),
        "server should respond with FormatListResponse::Ok, got {resp_pdu:?}"
    );
}

#[test]
fn server_rejects_operations_before_format_list() {
    let mut server = CliprdrServer::new(Box::new(TestBackend));
    let _ = server.start().unwrap();

    // Attempting initiate_paste should fail because server is not in Ready state
    let result = server.initiate_paste(ClipboardFormatId::new(13));
    assert!(result.is_err(), "initiate_paste should fail in Initialization state");
}
