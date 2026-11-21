use ironrdp_client::config::{Config, ConfigBuilder, Destination};
use ironrdp_client::{
    __test_apply_server_redirection, __test_build_connector, __test_check_target_certificate,
    __test_reconnect_through_broker,
};
use ironrdp_connector::{ClientConnector, ClientConnectorState, Credentials, Sequence as _};
use ironrdp_core::{WriteBuf, decode, encode_vec};
use ironrdp_pdu::mcs::SendDataRequest;
use ironrdp_pdu::nego::{
    ConnectionConfirm, ConnectionRequest, ConnectionRequestWithOpaqueRoutingToken, ResponseFlags, SecurityProtocol,
};
use ironrdp_pdu::rdp::ClientInfoPdu;
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_pdu::rdp::client_info::ClientInfoFlags;
use ironrdp_pdu::rdp::rdstls::{RDSTLS_VERSION_1, RdstlsCapabilities, RdstlsPasswordAuthenticationRequest};
use ironrdp_pdu::rdp::server_redirection::{ServerRedirectionFlags, ServerRedirectionPdu, TargetCertificateContainer};
use ironrdp_pdu::x224::X224;

const REDIRECTION_GUID: &[u8] = b"{\0G\0}\0";
/// Opaque blob; it ends in a lone UTF-16 surrogate so that no decoder can take it for text.
const ENCRYPTED_PASSWORD: &[u8] = &[0x30, 0x82, 0x01, 0x0A, 0x00, 0xD8];

fn broker_config() -> Config {
    ConfigBuilder::new()
        .with_destination(Destination::from_parts("broker.invalid", 3389))
        .with_username("broker_user")
        .with_password("secret".to_owned())
        .with_domain("BROKER".to_owned())
        .with_client_build(1)
        .with_client_dir(r"C:\")
        .with_client_name("client")
        .with_platform(MajorPlatformType::WINDOWS)
        .with_load_balance_info("tsv://MS Terminal Services Plugin.1.Desktops".to_owned())
        .build()
        .expect("broker test configuration")
}

fn rdstls_redirection() -> ServerRedirectionPdu {
    ServerRedirectionPdu {
        session_id: 7,
        flags: ServerRedirectionFlags::PASSWORD_IS_PK_ENCRYPTED,
        target_fqdn: Some("target.invalid".to_owned()),
        username: Some("target_user".to_owned()),
        domain: Some("TARGET".to_owned()),
        password: Some(ENCRYPTED_PASSWORD.to_vec()),
        redirection_guid: Some(REDIRECTION_GUID.to_vec()),
        target_certificate: Some(TargetCertificateContainer::from_certificate_der(
            REPORTED_CERTIFICATE.to_vec(),
        )),
        ..ServerRedirectionPdu::default()
    }
}

fn redirected_config(redirection: &ServerRedirectionPdu) -> Config {
    let mut config = broker_config();
    __test_apply_server_redirection(&mut config, redirection).expect("apply server redirection");
    config
}

fn requested_protocol(connector: &mut ClientConnector) -> SecurityProtocol {
    let mut output = WriteBuf::new();
    connector.step_no_input(&mut output).unwrap();
    decode::<X224<ConnectionRequest>>(output.filled()).unwrap().0.protocol
}

// oracle: spec MS-RDPBCGR §2.2.17.2
// The GUID and the encrypted password are the ones of the Server Redirection Packet, and so are the user name and
// domain it carries.
#[test]
fn encrypted_redirection_password_is_presented_through_rdstls() {
    let config = redirected_config(&rdstls_redirection());
    let mut connector = __test_build_connector(&config, None).expect("redirected connector");
    let mut output = WriteBuf::new();

    assert_eq!(
        requested_protocol(&mut connector),
        SecurityProtocol::RDSTLS | SecurityProtocol::SSL
    );
    let confirmation = encode_vec(&X224(ConnectionConfirm::Response {
        flags: ResponseFlags::empty(),
        protocol: SecurityProtocol::RDSTLS,
    }))
    .unwrap();
    connector.step(&confirmation, None, &mut output).unwrap();
    connector.mark_security_upgrade_as_done();
    let capabilities = encode_vec(&RdstlsCapabilities {
        supported_versions: RDSTLS_VERSION_1,
    })
    .unwrap();
    output.clear();
    connector.step(&capabilities, None, &mut output).unwrap();

    let request = decode::<RdstlsPasswordAuthenticationRequest>(output.filled()).unwrap();
    assert_eq!(
        request,
        RdstlsPasswordAuthenticationRequest {
            redirection_guid: REDIRECTION_GUID.to_vec(),
            username: "target_user".to_owned(),
            domain: "TARGET".to_owned(),
            password: ENCRYPTED_PASSWORD.to_vec(),
        }
    );
}

// oracle: reference FreeRDP/FreeRDP@12d8399cf44c:libfreerdp/core/connection.c
// `rdp_client_connect` turns on auto-logon when a user name and a redirection password are present.
#[test]
fn rdstls_redirection_logs_on_automatically() {
    let config = redirected_config(&rdstls_redirection());
    let mut connector = __test_build_connector(&config, None).expect("redirected connector");
    connector.state = ClientConnectorState::SecureSettingsExchange {
        io_channel_id: 1003,
        user_channel_id: 1002,
    };
    let mut output = WriteBuf::new();

    connector.step_no_input(&mut output).unwrap();

    let data = decode::<X224<SendDataRequest<'_>>>(output.filled()).unwrap().0;
    let client_info = decode::<ClientInfoPdu>(data.user_data.as_ref()).unwrap().client_info;
    assert!(client_info.flags.contains(ClientInfoFlags::AUTOLOGON));
}

// oracle: reference FreeRDP/FreeRDP@12d8399cf44c:libfreerdp/core/connection.c
// `rdp_client_reconnect_via_broker`: the target refuses a second use of the encrypted password, so a reconnection
// starts again from the server the connection was first made to, which redirects with a fresh one.
#[test]
fn reconnection_after_an_rdstls_redirection_goes_through_the_broker() {
    let mut config = redirected_config(&rdstls_redirection());

    assert!(__test_reconnect_through_broker(&mut config));

    assert_eq!(config.destination().name(), "broker.invalid");
    assert_eq!(
        config.load_balance_info(),
        Some("tsv://MS Terminal Services Plugin.1.Desktops")
    );
    assert!(matches!(
        &config.connector().credentials,
        Credentials::UsernamePassword { username, .. } if username == "broker_user"
    ));
    assert_eq!(config.connector().domain.as_deref(), Some("BROKER"));
    let mut connector = __test_build_connector(&config, None).expect("broker connector");
    let mut output = WriteBuf::new();
    connector.step_no_input(&mut output).unwrap();
    let request = decode::<X224<ConnectionRequestWithOpaqueRoutingToken>>(output.filled())
        .unwrap()
        .0;
    assert_eq!(request.routing_token.0, "tsv://MS Terminal Services Plugin.1.Desktops");
    assert!(!request.request.protocol.contains(SecurityProtocol::RDSTLS));
}

// oracle: reference FreeRDP/FreeRDP@12d8399cf44c:libfreerdp/core/connection.c
// The broker is the first server, not the last redirecting one: a second redirection keeps the original origin.
#[test]
fn reconnection_after_two_redirections_goes_to_the_first_server() {
    let mut config = broker_config();
    let first = ServerRedirectionPdu {
        target_fqdn: Some("second-broker.invalid".to_owned()),
        ..ServerRedirectionPdu::default()
    };
    __test_apply_server_redirection(&mut config, &first).expect("apply first redirection");
    __test_apply_server_redirection(&mut config, &rdstls_redirection()).expect("apply second redirection");

    assert!(__test_reconnect_through_broker(&mut config));

    assert_eq!(config.destination().name(), "broker.invalid");
}

// oracle: reference FreeRDP/FreeRDP@12d8399cf44c:libfreerdp/core/connection.c
// `rdp_client_reconnect_via_broker` returns to the first server only when LB_PASSWORD_IS_PK_ENCRYPTED is set, so a
// plain password cookie reconnects straight to the target.
#[test]
fn reconnection_after_a_plain_cookie_redirection_stays_on_the_target() {
    let redirection = ServerRedirectionPdu {
        flags: ServerRedirectionFlags::empty(),
        ..rdstls_redirection()
    };
    let mut config = redirected_config(&redirection);

    assert!(!__test_reconnect_through_broker(&mut config));

    assert_eq!(config.destination().name(), "target.invalid");
}

/// Stand-ins for DER certificates; the check compares bytes only.
const REPORTED_CERTIFICATE: &[u8] = &[0x30, 0x03, 0x02, 0x01, 0x01];
const OTHER_CERTIFICATE: &[u8] = &[0x30, 0x03, 0x02, 0x01, 0x02];

// oracle: spec MS-RDPBCGR §5.4.5.3.1
// The TargetCertificate "MUST be identical to the certificate that the server presents"; RDSTLS would hand the
// password to whichever server completed the handshake.
#[test]
fn rdstls_redirection_target_with_another_certificate_is_refused() {
    let config = redirected_config(&rdstls_redirection());

    assert!(__test_check_target_certificate(&config, OTHER_CERTIFICATE).is_err());
    assert!(__test_check_target_certificate(&config, REPORTED_CERTIFICATE).is_ok());
}

// oracle: reference FreeRDP/FreeRDP@12d8399cf44c:libfreerdp/core/redirection.c
// FreeRDP stores the TargetCertificate but never compares it with the one the target presents. Without RDSTLS,
// certificate validation and CredSSP authenticate the target, so a mismatch is only reported.
#[test]
fn plain_redirection_target_with_another_certificate_is_accepted() {
    let redirection = ServerRedirectionPdu {
        flags: ServerRedirectionFlags::empty(),
        ..rdstls_redirection()
    };
    let config = redirected_config(&redirection);

    assert!(__test_check_target_certificate(&config, OTHER_CERTIFICATE).is_ok());
}

// oracle: spec MS-RDPBCGR §2.2.17.2
// The RDSTLS request carries the RedirectionGuid of the Server Redirection Packet; without one the target cannot
// accept the encrypted password.
#[rstest::rstest]
#[case::absent(None)]
#[case::empty(Some(Vec::new()))]
fn rdstls_redirection_without_a_guid_is_refused(#[case] redirection_guid: Option<Vec<u8>>) {
    let redirection = ServerRedirectionPdu {
        redirection_guid,
        ..rdstls_redirection()
    };
    let mut config = broker_config();

    assert!(__test_apply_server_redirection(&mut config, &redirection).is_err());
}

// oracle: spec MS-RDPBCGR §5.4.5.3.1
// Only the reported TargetCertificate authenticates an RDSTLS target, so without one the password is not presented.
#[test]
fn rdstls_redirection_without_a_target_certificate_is_refused() {
    let redirection = ServerRedirectionPdu {
        target_certificate: None,
        ..rdstls_redirection()
    };
    let mut config = broker_config();

    assert!(__test_apply_server_redirection(&mut config, &redirection).is_err());
}
