use ironrdp_connector::{
    ClientConnector, ClientConnectorState, ConnectorErrorKind, Credentials, Sequence as _, Written,
};
use ironrdp_core::{WriteBuf, decode, encode_vec};
use ironrdp_pdu::mcs::SendDataRequest;
use ironrdp_pdu::nego::{ConnectionConfirm, ConnectionRequest, FailureCode, ResponseFlags, SecurityProtocol};
use ironrdp_pdu::rdp::ClientInfoPdu;
use ironrdp_pdu::rdp::client_info::ClientInfoFlags;
use ironrdp_pdu::x224::{X224, X224Data};

const ASSERTION: &str = "e30.e30.c2ln";

fn connector() -> ClientConnector {
    ClientConnector::new(super::test_config(), "127.0.0.1:3389".parse().unwrap()).with_entra_auth()
}

fn confirmation(protocol: SecurityProtocol) -> Vec<u8> {
    encode_vec(&X224(ConnectionConfirm::Response {
        flags: ResponseFlags::empty(),
        protocol,
    }))
    .unwrap()
}

fn after_tls() -> ClientConnector {
    let mut connector = connector();
    let mut output = WriteBuf::new();
    connector.step_no_input(&mut output).unwrap();
    connector
        .step(&confirmation(SecurityProtocol::RDSAAD), None, &mut output)
        .unwrap();
    assert!(connector.should_perform_security_upgrade());
    assert!(connector.entra_server_nonce().is_none());
    connector.mark_security_upgrade_as_done();
    assert!(matches!(connector.state, ClientConnectorState::EntraAuthWaitNonce));
    assert!(!connector.should_perform_credssp());
    connector
}

fn awaiting_result() -> ClientConnector {
    let mut connector = after_tls();
    let mut output = WriteBuf::new();
    connector.step(br#"{"ts_nonce":"nonce"}"#, None, &mut output).unwrap();
    connector.submit_entra_assertion(ASSERTION, &mut output).unwrap();
    connector
}

#[test]
fn entra_negotiation_requires_rds_aad_without_fallback() {
    let mut config = super::test_config();
    config.enable_credssp = true;
    config.enable_standard_rdp_security = true;
    let mut connector = ClientConnector::new(config, "127.0.0.1:3389".parse().unwrap()).with_entra_auth();
    let mut output = WriteBuf::new();
    for protocol in [
        SecurityProtocol::empty(),
        SecurityProtocol::SSL,
        SecurityProtocol::HYBRID,
    ] {
        assert!(
            connector
                .initiate_with_security_protocol(protocol, &mut output)
                .is_err()
        );
        assert_eq!(output.filled_len(), 0);
    }
    connector.step_no_input(&mut output).unwrap();
    let request = decode::<X224<ConnectionRequest>>(output.filled()).unwrap().0;
    assert_eq!(request.protocol, SecurityProtocol::RDSAAD);
}

#[test]
fn entra_rejects_server_downgrade_and_mixed_protocol_selection() {
    for protocol in [
        SecurityProtocol::empty(),
        SecurityProtocol::SSL,
        SecurityProtocol::HYBRID,
        SecurityProtocol::RDSAAD | SecurityProtocol::HYBRID,
        SecurityProtocol::RDSAAD | SecurityProtocol::SSL,
    ] {
        let mut connector = connector();
        let mut output = WriteBuf::new();
        connector.step_no_input(&mut output).unwrap();
        output.clear();
        assert!(connector.step(&confirmation(protocol), None, &mut output).is_err());
        assert!(matches!(connector.state, ClientConnectorState::Consumed));
        assert_eq!(output.filled_len(), 0);
    }
}

#[test]
fn entra_nonce_assertion_and_success_gate_basic_settings() {
    let mut connector = after_tls();
    let mut output = WriteBuf::new();
    assert!(connector.submit_entra_assertion(ASSERTION, &mut output).is_err());
    assert_eq!(output.filled_len(), 0);
    assert_eq!(
        connector
            .step(br#"{"ts_nonce":"server\"nonce\u00e9"}"#, None, &mut output)
            .unwrap(),
        Written::Nothing
    );
    assert_eq!(connector.entra_server_nonce(), Some("server\"nonce\u{e9}"));
    assert!(connector.next_pdu_hint().is_none());
    assert!(connector.step_no_input(&mut output).is_err());
    assert_eq!(connector.entra_server_nonce(), Some("server\"nonce\u{e9}"));

    let written = connector.submit_entra_assertion(ASSERTION, &mut output).unwrap();
    assert_eq!(written.size(), Some(output.filled_len()));
    assert_eq!(output.filled(), br#"{"rdp_assertion":"e30.e30.c2ln"}"#);
    assert!(matches!(connector.state, ClientConnectorState::EntraAuthWaitResult));
    assert!(connector.entra_server_nonce().is_none());
    assert!(connector.submit_entra_assertion(ASSERTION, &mut output).is_err());
    output.clear();

    connector
        .step(br#"{"authentication_result":"0"}"#, None, &mut output)
        .unwrap();
    assert_eq!(output.filled_len(), 0);
    assert!(matches!(
        connector.state,
        ClientConnectorState::BasicSettingsExchangeSendInitial {
            selected_protocol: SecurityProtocol::RDSAAD
        }
    ));
    connector.step_no_input(&mut output).unwrap();
    assert!(matches!(
        connector.state,
        ClientConnectorState::BasicSettingsExchangeWaitResponse { .. }
    ));
    assert!(!output.filled().is_empty());
}

#[test]
fn entra_json_hint_handles_all_fragments_and_coalesced_messages() {
    let connector = after_tls();
    let hint = connector.next_pdu_hint().unwrap();
    let nonce = br#"{"ts_nonce":"a\\\"b\u1234"}"#;
    for end in 0..nonce.len() {
        assert_eq!(hint.find_size(&nonce[..end]).unwrap(), None);
    }
    assert_eq!(hint.find_size(nonce).unwrap(), Some((true, nonce.len())));
    let mut combined = nonce.to_vec();
    combined.extend_from_slice(br#"{"authentication_result":0}"#);
    assert_eq!(hint.find_size(&combined).unwrap(), Some((true, nonce.len())));
    assert!(hint.find_size(b"[]").is_err());
    assert!(hint.find_size(b"{!}").is_err());
    assert!(hint.find_size(&[b' '; 64 * 1024]).is_err());

    let connector = awaiting_result();
    let hint = connector.next_pdu_hint().unwrap();
    let result = br#"{"authentication_result":0}"#;
    for end in 0..result.len() {
        assert_eq!(hint.find_size(&result[..end]).unwrap(), None);
    }
    let mut combined = result.to_vec();
    combined.extend_from_slice(&[3, 0, 0, 4]);
    assert_eq!(hint.find_size(&combined).unwrap(), Some((true, result.len())));
}

// oracle: spec MS-RDPBCGR §2.2.18.3; RFC 8259 §2
#[test]
fn entra_auth_result_trailing_json_whitespace_does_not_corrupt_x224_framing() {
    let mut connector = awaiting_result();
    connector
        .step(br#"{"authentication_result":0}"#, None, &mut WriteBuf::new())
        .unwrap();

    let mut output = WriteBuf::new();
    connector.step_no_input(&mut output).unwrap();
    let hint = connector.next_pdu_hint().expect("X.224 response hint");
    let x224 = encode_vec(&X224(X224Data {
        data: std::borrow::Cow::Borrowed(&ironrdp_testsuite_core::mcs::CONNECT_RESPONSE_BUFFER),
    }))
    .unwrap();
    let mut input = b"\r\n \t".to_vec();
    input.extend_from_slice(&x224);

    for end in 0..4 {
        assert_eq!(hint.find_size(&input[..end]).unwrap(), None);
    }
    assert_eq!(hint.find_size(&input).unwrap(), Some((true, input.len())));
    let error = connector.step(&input, None, &mut output).unwrap_err();
    assert!(matches!(error.kind(), ConnectorErrorKind::General));
}

#[test]
fn entra_json_size_limit_accepts_complete_boundary_and_rejects_overflow() {
    let prefix = "{\"ts_nonce\":\"";
    let suffix = "\"}";
    let nonce = format!(
        "{prefix}{}{suffix}",
        "a".repeat(64 * 1024 - prefix.len() - suffix.len())
    );
    let mut connector = after_tls();
    let hint = connector.next_pdu_hint().unwrap();
    assert_eq!(hint.find_size(nonce.as_bytes()).unwrap(), Some((true, 64 * 1024)));
    connector.step(nonce.as_bytes(), None, &mut WriteBuf::new()).unwrap();
    assert_eq!(
        connector.entra_server_nonce().unwrap().len(),
        64 * 1024 - prefix.len() - suffix.len()
    );

    let oversized = format!("{prefix}{}{suffix}", "a".repeat(64 * 1024));
    let mut connector = after_tls();
    assert!(
        connector
            .next_pdu_hint()
            .unwrap()
            .find_size(oversized.as_bytes())
            .is_err()
    );
    assert!(
        connector
            .step(oversized.as_bytes(), None, &mut WriteBuf::new())
            .is_err()
    );
    assert!(matches!(connector.state, ClientConnectorState::Consumed));
}

#[test]
fn entra_rejects_malformed_nonce_messages() {
    for input in [
        &br#"{"ts_nonce":""}"#[..],
        br#"{"ts_nonce":1}"#,
        br#"{"wrong":"nonce"}"#,
        br#"{"authentication_result":0}"#,
        br#"{"ts_nonce":"nonce"}{}"#,
        b"{",
        b"{\"ts_nonce\":\"\xff\"}",
    ] {
        let mut connector = after_tls();
        let mut output = WriteBuf::new();
        assert!(connector.step(input, None, &mut output).is_err());
        assert!(matches!(connector.state, ClientConnectorState::Consumed));
        assert_eq!(output.filled_len(), 0);
    }
}

#[test]
fn entra_rejects_invalid_assertions_without_advancing_or_writing() {
    let mut connector = after_tls();
    let mut output = WriteBuf::new();
    connector.step(br#"{"ts_nonce":"nonce"}"#, None, &mut output).unwrap();
    for assertion in ["", "bearer", "a.b.c", "e30.e30.", "e30.e30.c2ln.extra", "e30.e30.c2ln="] {
        assert!(connector.submit_entra_assertion(assertion, &mut output).is_err());
        assert_eq!(connector.entra_server_nonce(), Some("nonce"));
        assert_eq!(output.filled_len(), 0);
    }
    let oversized = format!("e30.e30.{}", "a".repeat(64 * 1024));
    assert!(connector.submit_entra_assertion(&oversized, &mut output).is_err());
    assert_eq!(connector.entra_server_nonce(), Some("nonce"));
    assert_eq!(output.filled_len(), 0);
}

#[test]
fn entra_assertion_message_size_limit_includes_json_envelope() {
    let envelope_len = br#"{"rdp_assertion":""}"#.len();
    let signature_len = 64 * 1024 - envelope_len - "e30.e30.".len();
    let mut connector = after_tls();
    let mut output = WriteBuf::new();
    connector.step(br#"{"ts_nonce":"nonce"}"#, None, &mut output).unwrap();
    let too_large = format!("e30.e30.{}", "a".repeat(signature_len + 2));
    assert!(connector.submit_entra_assertion(&too_large, &mut output).is_err());
    assert_eq!(output.filled_len(), 0);
    assert_eq!(connector.entra_server_nonce(), Some("nonce"));
    let boundary = format!("e30.e30.{}", "a".repeat(signature_len));
    let written = connector.submit_entra_assertion(&boundary, &mut output).unwrap();
    assert_eq!(output.filled_len(), 64 * 1024);
    assert_eq!(written.size(), Some(64 * 1024));
    assert!(matches!(connector.state, ClientConnectorState::EntraAuthWaitResult));
}

#[test]
fn entra_authentication_failure_preserves_hresult_and_aborts() {
    for input in [
        &br#"{"authentication_result":"2147942405"}"#[..],
        br#"{"authentication_result":2147942405}"#,
        br#"{"authentication_result":"-2147024891"}"#,
        br#"{"authentication_result":-2147024891}"#,
    ] {
        let mut connector = awaiting_result();
        let mut output = WriteBuf::new();
        let error = connector.step(input, None, &mut output).unwrap_err();
        assert!(matches!(error.kind(), ConnectorErrorKind::EntraAuth(0x8007_0005)));
        assert!(matches!(connector.state, ClientConnectorState::Consumed));
        assert_eq!(output.filled_len(), 0);
    }
    let mut connector = awaiting_result();
    connector
        .step(br#"{"authentication_result":0}"#, None, &mut WriteBuf::new())
        .unwrap();
    assert!(matches!(
        connector.state,
        ClientConnectorState::BasicSettingsExchangeSendInitial { .. }
    ));
}

#[test]
fn entra_rejects_invalid_authentication_results() {
    for input in [
        &br#"{"authentication_result":null}"#[..],
        br#"{"authentication_result":false}"#,
        br#"{"authentication_result":0.0}"#,
        br#"{"authentication_result":"success"}"#,
        br#"{"authentication_result":4294967296}"#,
        br#"{"authentication_result":-2147483649}"#,
        br#"{"ts_nonce":"nonce"}"#,
    ] {
        let mut connector = awaiting_result();
        assert!(connector.step(input, None, &mut WriteBuf::new()).is_err());
        assert!(matches!(connector.state, ClientConnectorState::Consumed));
    }
    let mut connector = awaiting_result();
    let error = connector
        .step(br#"{"authentication_result":1}"#, None, &mut WriteBuf::new())
        .unwrap_err();
    assert!(matches!(error.kind(), ConnectorErrorKind::EntraAuth(1)));
}

#[test]
fn entra_client_info_never_sends_password_or_pin() {
    for smart_card in [false, true] {
        for entra in [false, true] {
            let mut config = super::test_config();
            config.autologon = true;
            if smart_card {
                config.credentials = Credentials::SmartCard {
                    pin: "test".into(),
                    config: None,
                };
            }
            let mut connector = ClientConnector::new(config, "127.0.0.1:3389".parse().unwrap());
            if entra {
                connector = connector.with_entra_auth();
            }
            connector.state = ClientConnectorState::SecureSettingsExchange {
                io_channel_id: 1003,
                user_channel_id: 1002,
            };
            let mut output = WriteBuf::new();
            connector.step_no_input(&mut output).unwrap();
            let data = decode::<X224<SendDataRequest<'_>>>(output.filled()).unwrap().0;
            let info = decode::<ClientInfoPdu>(data.user_data.as_ref()).unwrap().client_info;
            assert_eq!(info.credentials.password, if entra { "" } else { "test" });
            assert_eq!(info.flags.contains(ClientInfoFlags::AUTOLOGON), !entra);
            assert_eq!(
                info.flags.contains(ClientInfoFlags::PASSWORD_IS_SC_PIN),
                smart_card && !entra
            );
        }
    }
}

#[test]
fn credssp_and_tls_negotiation_are_unchanged_without_entra_opt_in() {
    for protocol in [
        SecurityProtocol::SSL,
        SecurityProtocol::HYBRID,
        SecurityProtocol::HYBRID_EX,
    ] {
        let mut config = super::test_config();
        config.enable_credssp = true;
        let mut connector = ClientConnector::new(config, "127.0.0.1:3389".parse().unwrap());
        let mut output = WriteBuf::new();
        connector.step_no_input(&mut output).unwrap();
        let request = decode::<X224<ConnectionRequest>>(output.filled()).unwrap().0;
        assert_eq!(
            request.protocol,
            SecurityProtocol::SSL | SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX
        );
        connector.step(&confirmation(protocol), None, &mut output).unwrap();
        connector.mark_security_upgrade_as_done();
        assert_eq!(connector.should_perform_credssp(), protocol != SecurityProtocol::SSL);
        assert!(connector.entra_server_nonce().is_none());
    }
}

#[test]
fn entra_required_negotiation_failure_has_specific_diagnostic() {
    let mut connector = connector();
    let mut output = WriteBuf::new();
    connector.step_no_input(&mut output).unwrap();
    let failure = encode_vec(&X224(ConnectionConfirm::Failure {
        code: FailureCode::ENTRA_AUTH_REQUIRED_BY_SERVER,
    }))
    .unwrap();
    let error = connector.step(&failure, None, &mut output).unwrap_err();
    let ConnectorErrorKind::Negotiation(failure) = error.kind() else {
        panic!("expected a negotiation failure");
    };
    assert_eq!(u32::from(failure.code()), 7);
    assert!(failure.to_string().contains("Microsoft Entra"));
}
