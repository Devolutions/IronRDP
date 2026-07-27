//! Front ordering of the client connector.
//!
//! A standard connection negotiates X.224 first and upgrades security afterwards. A Hyper-V console
//! sends a Preconnection Blob, upgrades security, authenticates, and only then negotiates X.224 —
//! and the host must select plain `HYBRID`, since NLA has already run by that point. Both orderings
//! share the X.224 initiation states, so the standard path is covered here too.

use ironrdp_connector::{ClientConnector, ClientConnectorState, Sequence as _};
use ironrdp_core::{WriteBuf, encode_vec};
use ironrdp_pdu::nego::{ConnectionConfirm, ResponseFlags, SecurityProtocol};
use ironrdp_pdu::x224::X224;

use super::test_config;

const VM_ID: &str = "efd1efab-c750-4262-b1bb-af0f7733bdd6";

fn vmconnect_connector() -> ClientConnector {
    ClientConnector::new_vmconnect(test_config(), "127.0.0.1:12345".parse().unwrap(), VM_ID.to_owned())
}

fn server_connection_confirm(protocol: SecurityProtocol) -> Vec<u8> {
    encode_vec(&X224(ConnectionConfirm::Response {
        flags: ResponseFlags::empty(),
        protocol,
    }))
    .unwrap()
}

/// Drives a vmconnect connector up to the point where it awaits the X.224 connection confirm.
fn vmconnect_connector_awaiting_confirm() -> ClientConnector {
    let mut connector = vmconnect_connector();
    let mut output = WriteBuf::new();

    connector.step_no_input(&mut output).unwrap();
    connector.mark_security_upgrade_as_done();
    connector.mark_credssp_as_done();
    output.clear();
    connector.step_no_input(&mut output).unwrap();

    connector
}

#[test]
fn vmconnect_sends_preconnection_blob_then_negotiates_x224_after_nla() {
    let mut connector = vmconnect_connector();
    let mut output = WriteBuf::new();

    assert!(matches!(
        connector.state,
        ClientConnectorState::PreconnectionBlobSendRequest
    ));

    let written = connector.step_no_input(&mut output).unwrap();
    assert!(written.size().is_some(), "the preconnection blob must be written");

    let vm_id_utf16: Vec<u8> = VM_ID.encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert!(
        output
            .filled()
            .windows(vm_id_utf16.len())
            .any(|window| window == vm_id_utf16),
        "the preconnection blob must carry the VM ID"
    );

    // TLS and NLA both run before the negotiation, exactly as for a standard connection.
    assert!(connector.should_perform_security_upgrade());
    connector.mark_security_upgrade_as_done();
    assert!(connector.should_perform_credssp());
    connector.mark_credssp_as_done();

    assert!(
        matches!(connector.state, ClientConnectorState::ConnectionInitiationSendRequest),
        "the X.224 negotiation happens last for a Hyper-V console"
    );

    output.clear();
    let written = connector.step_no_input(&mut output).unwrap();
    assert!(written.size().is_some(), "the connection request must be written");

    output.clear();
    connector
        .step(&server_connection_confirm(SecurityProtocol::HYBRID), &mut output)
        .unwrap();

    assert!(
        matches!(
            connector.state,
            ClientConnectorState::BasicSettingsExchangeSendInitial { .. }
        ),
        "a confirmed negotiation hands over to the shared tail"
    );
}

#[test]
fn vmconnect_rejects_a_host_that_does_not_select_plain_hybrid() {
    // NLA already ran with plain HYBRID, so anything else would be echoed into Client Core Data as
    // a protocol that was never performed.
    for protocol in [
        SecurityProtocol::HYBRID_EX,
        SecurityProtocol::SSL,
        SecurityProtocol::HYBRID | SecurityProtocol::SSL,
    ] {
        let mut connector = vmconnect_connector_awaiting_confirm();
        let mut output = WriteBuf::new();

        let result = connector.step(&server_connection_confirm(protocol), &mut output);

        assert!(result.is_err(), "{protocol} must be rejected for a Hyper-V console");
    }
}

/// The console is a separate service from the Remote Desktop host and registers its own SPN, so
/// asking for `TERMSRV` would fail Kerberos against a Hyper-V host.
#[test]
fn vmconnect_authenticates_against_the_virtual_console_service() {
    assert_eq!(
        vmconnect_connector().spn_service_class(),
        "Microsoft Virtual Console Service"
    );
    assert_eq!(
        ClientConnector::new(test_config(), "127.0.0.1:12345".parse().unwrap()).spn_service_class(),
        "TERMSRV"
    );
}

#[test]
fn standard_connection_still_negotiates_x224_first() {
    let mut connector = ClientConnector::new(test_config(), "127.0.0.1:12345".parse().unwrap());
    let mut output = WriteBuf::new();

    assert!(matches!(
        connector.state,
        ClientConnectorState::ConnectionInitiationSendRequest
    ));

    let written = connector.step_no_input(&mut output).unwrap();
    assert!(written.size().is_some(), "the connection request must be written");

    // `test_config` advertises TLS only, so the server selecting it is accepted.
    output.clear();
    connector
        .step(&server_connection_confirm(SecurityProtocol::SSL), &mut output)
        .unwrap();

    assert!(
        matches!(connector.state, ClientConnectorState::EnhancedSecurityUpgrade { .. }),
        "a standard connection upgrades security after the negotiation"
    );
}

#[test]
fn standard_connection_rejects_a_protocol_it_did_not_advertise() {
    let mut connector = ClientConnector::new(test_config(), "127.0.0.1:12345".parse().unwrap());
    let mut output = WriteBuf::new();

    connector.step_no_input(&mut output).unwrap();
    output.clear();

    let result = connector.step(&server_connection_confirm(SecurityProtocol::HYBRID_EX), &mut output);

    assert!(result.is_err(), "the server must not select an unadvertised protocol");
}
