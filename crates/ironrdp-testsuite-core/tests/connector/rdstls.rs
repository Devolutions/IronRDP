use ironrdp_connector::{ClientConnector, ClientConnectorState, ConnectorErrorKind, Sequence as _};
use ironrdp_core::{WriteBuf, decode, encode_vec};
use ironrdp_pdu::mcs::SendDataRequest;
use ironrdp_pdu::nego::{ConnectionConfirm, ConnectionRequest, ResponseFlags, SecurityProtocol};
use ironrdp_pdu::rdp::ClientInfoPdu;
use ironrdp_pdu::rdp::rdstls::{
    RDSTLS_VERSION_1, RdstlsAuthenticationResponse, RdstlsCapabilities, RdstlsPasswordAuthenticationRequest,
    RdstlsResultCode,
};
use ironrdp_pdu::x224::X224;

const REDIRECTION_GUID: &[u8] = b"{\0G\0}\0";
/// Opaque blob; it ends in a lone UTF-16 surrogate so that no decoder can take it for text.
const ENCRYPTED_PASSWORD: &[u8] = &[0x30, 0x82, 0x01, 0x0A, 0x00, 0xD8];

fn connector() -> ClientConnector {
    let mut config = super::test_config();
    config.domain = Some("CONTOSO".to_owned());
    ClientConnector::new(config, "127.0.0.1:3389".parse().unwrap())
        .with_rdstls_redirection(REDIRECTION_GUID.to_vec(), ENCRYPTED_PASSWORD.to_vec())
}

fn confirmation(protocol: SecurityProtocol) -> Vec<u8> {
    encode_vec(&X224(ConnectionConfirm::Response {
        flags: ResponseFlags::empty(),
        protocol,
    }))
    .unwrap()
}

fn capabilities(supported_versions: u16) -> Vec<u8> {
    encode_vec(&RdstlsCapabilities { supported_versions }).unwrap()
}

fn response(result_code: RdstlsResultCode) -> Vec<u8> {
    encode_vec(&RdstlsAuthenticationResponse { result_code }).unwrap()
}

fn after_tls(selected_protocol: SecurityProtocol) -> ClientConnector {
    let mut connector = connector();
    let mut output = WriteBuf::new();
    connector.step_no_input(&mut output).unwrap();
    connector
        .step(&confirmation(selected_protocol), None, &mut output)
        .unwrap();
    connector.mark_security_upgrade_as_done();
    connector
}

fn awaiting_response() -> ClientConnector {
    let mut connector = after_tls(SecurityProtocol::RDSTLS);
    connector
        .step(&capabilities(RDSTLS_VERSION_1), None, &mut WriteBuf::new())
        .unwrap();
    connector
}

// oracle: reference FreeRDP/FreeRDP@12d8399cf44c:libfreerdp/core/nego.c
// `nego_attempt_rdstls` requests RDSTLS and TLS only: the encrypted password is useless to CredSSP.
#[test]
fn rdstls_redirection_requests_rdstls_and_tls_only() {
    let mut config = super::test_config();
    config.enable_credssp = true;
    let mut connector = ClientConnector::new(config, "127.0.0.1:3389".parse().unwrap())
        .with_rdstls_redirection(REDIRECTION_GUID.to_vec(), ENCRYPTED_PASSWORD.to_vec());
    let mut output = WriteBuf::new();

    connector.step_no_input(&mut output).unwrap();

    let request = decode::<X224<ConnectionRequest>>(output.filled()).unwrap().0;
    assert_eq!(request.protocol, SecurityProtocol::RDSTLS | SecurityProtocol::SSL);
}

// oracle: spec MS-RDPBCGR §2.2.1.2.1
// The server selects a single protocol out of the requested ones.
#[rstest::rstest]
#[case::credssp(SecurityProtocol::HYBRID)]
#[case::standard_rdp_security(SecurityProtocol::empty())]
#[case::rdstls_and_tls_together(SecurityProtocol::RDSTLS | SecurityProtocol::SSL)]
fn rdstls_redirection_rejects_a_selection_it_did_not_request(#[case] selected_protocol: SecurityProtocol) {
    let mut connector = connector();
    let mut output = WriteBuf::new();
    connector.step_no_input(&mut output).unwrap();

    assert!(
        connector
            .step(&confirmation(selected_protocol), None, &mut output)
            .is_err()
    );
}

// oracle: spec MS-RDPBCGR §2.2.17.2
// The GUID and the password come from the Server Redirection Packet; user name and domain from the configuration
// the client updated with it.
#[test]
fn capabilities_are_answered_with_the_redirection_credentials() {
    let mut connector = after_tls(SecurityProtocol::RDSTLS);
    let mut output = WriteBuf::new();

    connector
        .step(&capabilities(RDSTLS_VERSION_1), None, &mut output)
        .unwrap();

    let request = decode::<RdstlsPasswordAuthenticationRequest>(output.filled()).unwrap();
    assert_eq!(
        request,
        RdstlsPasswordAuthenticationRequest {
            redirection_guid: REDIRECTION_GUID.to_vec(),
            username: "test".to_owned(),
            domain: "CONTOSO".to_owned(),
            password: ENCRYPTED_PASSWORD.to_vec(),
        }
    );
}

// oracle: reference FreeRDP/FreeRDP@12d8399cf44c:libfreerdp/core/rdstls.c
// A FreeRDP server advertises every version it supports, RDSTLS_VERSION_1 | RDSTLS_VERSION_2 (0x0003).
#[test]
fn capabilities_with_more_than_version_1_are_answered() {
    let mut connector = after_tls(SecurityProtocol::RDSTLS);
    let mut output = WriteBuf::new();

    connector.step(&capabilities(0x0003), None, &mut output).unwrap();

    assert!(decode::<RdstlsPasswordAuthenticationRequest>(output.filled()).is_ok());
}

// oracle: spec MS-RDPBCGR §2.2.17.1
// Version 1 is the only one the client can speak.
#[test]
fn capabilities_without_version_1_are_rejected() {
    let mut connector = after_tls(SecurityProtocol::RDSTLS);
    let mut output = WriteBuf::new();

    assert!(connector.step(&capabilities(0x0002), None, &mut output).is_err());
    assert_eq!(output.filled_len(), 0);
}

// oracle: spec MS-RDPBCGR §2.2.17.4
#[test]
fn successful_authentication_continues_with_basic_settings_exchange() {
    let mut connector = awaiting_response();

    connector
        .step(&response(RdstlsResultCode::SUCCESS), None, &mut WriteBuf::new())
        .unwrap();

    assert!(matches!(
        connector.state,
        ClientConnectorState::BasicSettingsExchangeSendInitial {
            selected_protocol: SecurityProtocol::RDSTLS
        }
    ));
}

// oracle: spec MS-RDPBCGR §2.2.17.4
#[test]
fn failed_authentication_reports_the_result_code() {
    let mut connector = awaiting_response();

    let error = connector
        .step(&response(RdstlsResultCode::LOGON_FAILURE), None, &mut WriteBuf::new())
        .unwrap_err();

    assert!(matches!(
        error.kind(),
        ConnectorErrorKind::Rdstls(RdstlsResultCode::LOGON_FAILURE)
    ));
}

// oracle: spec MS-RDPBCGR §2.2.17.1 and §2.2.17.4
// RDSTLS PDUs carry no length, so the hint must take exactly one fixed-size PDU and leave the next one buffered.
#[test]
fn pdu_hint_frames_one_rdstls_pdu() {
    let mut buffered = capabilities(RDSTLS_VERSION_1);
    buffered.extend_from_slice(&response(RdstlsResultCode::SUCCESS));
    let connector = after_tls(SecurityProtocol::RDSTLS);
    let hint = connector
        .next_pdu_hint()
        .expect("a hint while waiting for the capabilities");
    assert_eq!(hint.find_size(&buffered).unwrap(), Some((true, 8)));

    let connector = awaiting_response();
    let hint = connector
        .next_pdu_hint()
        .expect("a hint while waiting for the response");
    assert_eq!(hint.find_size(&buffered[8..]).unwrap(), Some((true, 10)));
}

// oracle: reference FreeRDP/FreeRDP@12d8399cf44c:libfreerdp/core/nego.c
// A server that picks TLS over RDSTLS gets a plain TLS connection, without RDSTLS PDUs.
#[test]
fn tls_selection_skips_the_rdstls_exchange() {
    let connector = after_tls(SecurityProtocol::SSL);

    assert!(matches!(
        connector.state,
        ClientConnectorState::BasicSettingsExchangeSendInitial {
            selected_protocol: SecurityProtocol::SSL
        }
    ));
}

// oracle: reference FreeRDP/FreeRDP@12d8399cf44c:libfreerdp/core/info.c
// `rdp_write_info_packet` sends the redirection password, encrypted or not, as the Client Info password cookie.
#[test]
fn client_info_carries_the_encrypted_password_as_the_cookie() {
    let mut connector = connector();
    connector.state = ClientConnectorState::SecureSettingsExchange {
        io_channel_id: 1003,
        user_channel_id: 1002,
    };
    let mut output = WriteBuf::new();

    connector.step_no_input(&mut output).unwrap();

    let data = decode::<X224<SendDataRequest<'_>>>(output.filled()).unwrap().0;
    let client_info = decode::<ClientInfoPdu>(data.user_data.as_ref()).unwrap().client_info;
    assert_eq!(
        client_info.password_cookie.map(|cookie| cookie.0),
        Some(ENCRYPTED_PASSWORD.to_vec())
    );
}
