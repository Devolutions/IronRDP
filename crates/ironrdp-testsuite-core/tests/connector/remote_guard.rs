use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use ironrdp_connector::credssp::{CredsspSequence, KerberosConfig};
use ironrdp_connector::remote_guard::{
    Backend, Context, KERBEROS, MAX_MESSAGE_SIZE, NTLM, PROFILE, STATUS_ACCESS_DENIED, STATUS_INVALID_HANDLE,
    STATUS_INVALID_PARAMETER, STATUS_NOT_SUPPORTED, WRAP_OVERHEAD, asn1_output, bytes, empty_ntlm_response, key,
    ntlm_credential as map_ntlm_credential, output, unicode, unicode_bytes,
};
use ironrdp_connector::sspi::credssp::{self, ClientState};
use ironrdp_connector::sspi::rcg::NtlmRedirectionNames;
use ironrdp_connector::sspi::{Error, ErrorKind, Secret};
use ironrdp_connector::{ClientConnector, ClientConnectorState, Credentials, Sequence as _};
use ironrdp_core::{Decode as _, ReadCursor, WriteBuf, decode, encode_vec};
use ironrdp_dvc::{DrdynvcClient, DvcProcessor as _};
use ironrdp_pdu::mcs::SendDataRequest;
use ironrdp_pdu::nego;
use ironrdp_pdu::rdp::ClientInfoPdu;
use ironrdp_pdu::rdp::client_info::ClientInfoFlags;
use ironrdp_pdu::rdp::session_info::ServerAutoReconnect;
use ironrdp_pdu::x224::X224;
use ironrdp_rdpear::ndr::{self, KerberosRequest as Request, KerberosResponseCall as Response};
use ironrdp_rdpear::{AuthRedirectionBackend as _, AuthRedirectionClient, RemoteGuardInnerPacket};
use ironrdp_svc::SvcProcessor as _;

fn connector() -> ClientConnector {
    let mut config = super::test_config();
    config.enable_credssp = true;
    config.autologon = true;
    config.credentials = Credentials::UsernamePassword {
        username: "test".into(),
        password: "secret-sentinel".into(),
    };
    config.domain = Some("example.test".into());
    config.client_name = "test-client".into();
    ClientConnector::new(config, "127.0.0.1:3389".parse().unwrap())
}

fn confirm(flags: nego::ResponseFlags, protocol: nego::SecurityProtocol) -> Vec<u8> {
    encode_vec(&X224(nego::ConnectionConfirm::Response { flags, protocol })).unwrap()
}

fn step_client_info(mut connector: ClientConnector) -> (ClientInfoPdu, Vec<u8>) {
    connector.state = ClientConnectorState::SecureSettingsExchange {
        io_channel_id: 1003,
        user_channel_id: 1002,
    };
    let mut output = WriteBuf::new();
    connector.step_no_input(&mut output).unwrap();
    let data = decode::<X224<SendDataRequest<'_>>>(output.filled()).unwrap().0;
    let bytes = data.user_data.into_owned();
    let pdu = decode::<ClientInfoPdu>(&bytes).unwrap();
    (pdu, bytes)
}

#[test]
fn rcg_negotiates_only_credssp_and_requires_server_support() {
    for protocol in [nego::SecurityProtocol::HYBRID, nego::SecurityProtocol::HYBRID_EX] {
        let mut connector = connector().with_remote_guard(DrdynvcClient::new()).unwrap();
        let mut output = WriteBuf::new();
        connector.step_no_input(&mut output).unwrap();
        let request = decode::<X224<nego::ConnectionRequest>>(output.filled()).unwrap().0;
        assert_eq!(
            request.flags,
            nego::RequestFlags::REDIRECTED_AUTHENTICATION_MODE_REQUIRED
        );
        assert_eq!(
            request.protocol,
            nego::SecurityProtocol::HYBRID | nego::SecurityProtocol::HYBRID_EX
        );
        assert_eq!(
            connector
                .get_static_channel_processor::<DrdynvcClient>()
                .unwrap()
                .max_message_size(),
            Some(MAX_MESSAGE_SIZE)
        );
        let response = confirm(nego::ResponseFlags::REDIRECTED_AUTHENTICATION_MODE_SUPPORTED, protocol);
        connector.step(&response, None, &mut WriteBuf::new()).unwrap();
        connector.mark_security_upgrade_as_done();
        assert!(connector.should_perform_credssp());
        assert!(
            connector.step_no_input(&mut WriteBuf::new()).is_err(),
            "cannot skip live-context handoff"
        );
    }
    for (flags, protocol) in [
        (nego::ResponseFlags::empty(), nego::SecurityProtocol::HYBRID),
        (
            nego::ResponseFlags::REDIRECTED_AUTHENTICATION_MODE_SUPPORTED,
            nego::SecurityProtocol::SSL,
        ),
        (
            nego::ResponseFlags::REDIRECTED_AUTHENTICATION_MODE_SUPPORTED,
            nego::SecurityProtocol::empty(),
        ),
        (
            nego::ResponseFlags::REDIRECTED_AUTHENTICATION_MODE_SUPPORTED,
            nego::SecurityProtocol::HYBRID | nego::SecurityProtocol::SSL,
        ),
    ] {
        let mut connector = connector().with_remote_guard(DrdynvcClient::new()).unwrap();
        connector.step_no_input(&mut WriteBuf::new()).unwrap();
        assert!(
            connector
                .step(&confirm(flags, protocol), None, &mut WriteBuf::new())
                .is_err()
        );
        assert!(matches!(connector.state, ClientConnectorState::Consumed));
    }
}

#[test]
fn rcg_ntlm_is_explicit_opt_in_and_keeps_kerberos_negotiation() {
    let kerberos_only = connector().with_remote_guard(DrdynvcClient::new()).unwrap();
    assert!(kerberos_only.__test_remote_guard_ntlm().is_none());
    for names in [
        None,
        Some(NtlmRedirectionNames {
            user: "SAMUser".into(),
            domain: "NETBIOS".into(),
        }),
    ] {
        let mut connector = connector()
            .with_remote_guard_ntlm(DrdynvcClient::new(), names.clone())
            .unwrap();
        let configured = connector.__test_remote_guard_ntlm().unwrap();
        assert_eq!(
            configured.as_ref().map(|names| &names.user),
            names.as_ref().map(|names| &names.user)
        );
        assert_eq!(
            configured.as_ref().map(|names| &names.domain),
            names.as_ref().map(|names| &names.domain)
        );
        let mut output = WriteBuf::new();
        connector.step_no_input(&mut output).unwrap();
        let request = decode::<X224<nego::ConnectionRequest>>(output.filled()).unwrap().0;
        assert_eq!(
            request.flags,
            nego::RequestFlags::REDIRECTED_AUTHENTICATION_MODE_REQUIRED
        );
        assert_eq!(
            request.protocol,
            nego::SecurityProtocol::HYBRID | nego::SecurityProtocol::HYBRID_EX
        );
        connector
            .step(
                &confirm(
                    nego::ResponseFlags::REDIRECTED_AUTHENTICATION_MODE_SUPPORTED,
                    nego::SecurityProtocol::HYBRID,
                ),
                None,
                &mut WriteBuf::new(),
            )
            .unwrap();
        connector.mark_security_upgrade_as_done();
        let (sequence, _) = CredsspSequence::init_for_connector(
            &connector,
            "server.example.test".into(),
            vec![1],
            Some(KerberosConfig::new_with_kdc_url(None, "test-client".into()).unwrap()),
        )
        .unwrap();
        assert!(sequence.remote_guard_enabled());
        assert!(sequence.__test_remote_guard_ntlm_enabled());
    }
    let mut stale = connector();
    stale.step_no_input(&mut WriteBuf::new()).unwrap();
    assert!(stale.with_remote_guard_ntlm(DrdynvcClient::new(), None).is_err());
    assert!(
        kerberos_only
            .with_remote_guard_ntlm(DrdynvcClient::new(), None)
            .is_err()
    );
}

#[test]
fn rcg_ntlm_only_keeps_ntlm_policy_and_disables_kerberos_redirection() {
    let ntlm_only = connector()
        .with_remote_guard_ntlm_only(DrdynvcClient::new(), None)
        .unwrap();
    assert!(ntlm_only.__test_remote_guard_ntlm().is_some());
    assert!(!ntlm_only.__test_remote_guard_kerberos());

    let kerberos = connector().with_remote_guard(DrdynvcClient::new()).unwrap();
    assert!(kerberos.__test_remote_guard_kerberos());
    let negotiate = connector().with_remote_guard_ntlm(DrdynvcClient::new(), None).unwrap();
    assert!(negotiate.__test_remote_guard_kerberos());

    let mut stale = connector();
    stale.step_no_input(&mut WriteBuf::new()).unwrap();
    assert!(stale.with_remote_guard_ntlm_only(DrdynvcClient::new(), None).is_err());
}

#[test]
fn rcg_cannot_handoff_a_different_ntlm_policy() {
    for enabled in [false, true] {
        let mut source = if enabled {
            connector().with_remote_guard_ntlm(DrdynvcClient::new(), None).unwrap()
        } else {
            connector().with_remote_guard(DrdynvcClient::new()).unwrap()
        };
        source.state = ClientConnectorState::Credssp {
            selected_protocol: nego::SecurityProtocol::HYBRID,
        };
        let (mut sequence, _) = CredsspSequence::init_for_connector(
            &source,
            "server.example.test".into(),
            vec![1],
            Some(KerberosConfig::new_with_kdc_url(None, "test-client".into()).unwrap()),
        )
        .unwrap();
        assert_eq!(sequence.__test_remote_guard_ntlm_enabled(), enabled);
        sequence
            .handle_process_result(ClientState::FinalMessage(Default::default()), &mut WriteBuf::new())
            .unwrap();
        assert!(sequence.__test_is_finished());
        let mut target = if enabled {
            connector().with_remote_guard(DrdynvcClient::new()).unwrap()
        } else {
            connector().with_remote_guard_ntlm(DrdynvcClient::new(), None).unwrap()
        };
        target.state = source.state;
        let error = target.complete_credssp(sequence).unwrap_err();
        assert!(error.to_string().contains("NTLM redirection policy does not match"));
    }
}

#[test]
fn rcg_rejects_disabled_credssp_duplicates_and_explicit_downgrade() {
    let mut disabled = connector();
    disabled.config.enable_credssp = false;
    assert!(disabled.with_remote_guard(DrdynvcClient::new()).is_err());
    assert!(
        connector()
            .with_entra_auth()
            .with_remote_guard(DrdynvcClient::new())
            .is_err()
    );
    assert!(
        connector()
            .with_static_channel(DrdynvcClient::new())
            .with_remote_guard(DrdynvcClient::new())
            .is_err()
    );
    let mut rcg = connector().with_remote_guard(DrdynvcClient::new()).unwrap();
    assert!(
        rcg.initiate_with_security_protocol(nego::SecurityProtocol::SSL, &mut WriteBuf::new())
            .is_err()
    );
    assert!(
        rcg.initiate_with_security_protocol(nego::SecurityProtocol::empty(), &mut WriteBuf::new())
            .is_err()
    );
}

#[test]
fn rcg_rejects_auto_reconnect_and_reserved_listeners() {
    let cookie = || ServerAutoReconnect {
        logon_id: 1,
        random_bits: [0; 16],
    };
    assert!(
        connector()
            .with_auto_reconnect_cookie(cookie())
            .with_remote_guard(DrdynvcClient::new())
            .is_err()
    );
    let mut late_cookie = connector()
        .with_remote_guard(DrdynvcClient::new())
        .unwrap()
        .with_auto_reconnect_cookie(cookie());
    let mut output = WriteBuf::new();
    assert!(late_cookie.step_no_input(&mut output).is_err());
    assert!(output.filled().is_empty());

    struct ReservedListener(Arc<AtomicUsize>);
    impl ironrdp_dvc::DvcChannelListener for ReservedListener {
        fn channel_name(&self) -> &str {
            ironrdp_rdpear::CHANNEL_NAME
        }

        fn create(&mut self, _: u32) -> Option<Box<dyn ironrdp_dvc::DvcClientProcessor>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            None
        }
    }
    let created = Arc::new(AtomicUsize::new(0));
    let carrier = DrdynvcClient::new().with_listener(ReservedListener(Arc::clone(&created)));
    assert!(carrier.has_channel_name(ironrdp_rdpear::CHANNEL_NAME));
    assert!(connector().with_remote_guard(carrier).is_err());
    assert_eq!(created.load(Ordering::SeqCst), 0);
}

#[test]
fn rcg_client_info_never_copies_password_smartcard_pin_or_redirection_cookie() {
    let cookie = b"redirect-cookie-sentinel".to_vec();
    let (ordinary, _) = step_client_info(connector());
    assert_eq!(ordinary.client_info.credentials.password, "secret-sentinel");
    let (_, ordinary_cookie_bytes) = step_client_info(connector().with_redirection_password_cookie(cookie.clone()));
    assert!(
        ordinary_cookie_bytes
            .windows(cookie.len())
            .any(|window| window == cookie.as_slice())
    );

    for credentials in [
        Credentials::UsernamePassword {
            username: "test".into(),
            password: "secret-sentinel".into(),
        },
        Credentials::SmartCard {
            pin: "pin-sentinel".into(),
            config: None,
        },
    ] {
        let mut rcg = connector()
            .with_remote_guard(DrdynvcClient::new())
            .unwrap()
            .with_redirection_password_cookie(cookie.clone());
        rcg.config.credentials = credentials;
        let (info, encoded) = step_client_info(rcg);
        assert!(info.client_info.credentials.password.is_empty());
        assert!(info.client_info.password_cookie.is_none());
        assert!(!info.client_info.flags.contains(ClientInfoFlags::PASSWORD_IS_SC_PIN));
        for secret in ["secret-sentinel", "pin-sentinel"] {
            let utf16: Vec<u8> = secret.encode_utf16().flat_map(u16::to_le_bytes).collect();
            assert!(!encoded.windows(utf16.len()).any(|window| window == utf16));
        }
        assert!(!encoded.windows(cookie.len()).any(|window| window == cookie.as_slice()));
    }
}

#[test]
fn rcg_requires_kerberos_config_and_cannot_complete_an_unfinished_exchange() {
    let mut connector = connector().with_remote_guard(DrdynvcClient::new()).unwrap();
    connector.state = ClientConnectorState::Credssp {
        selected_protocol: nego::SecurityProtocol::HYBRID_EX,
    };
    assert!(CredsspSequence::init_for_connector(&connector, "server.example.test".into(), vec![1], None).is_err());
    assert!(
        CredsspSequence::init_for_connector(
            &connector,
            "server.example.test".into(),
            vec![1],
            Some(KerberosConfig::new_with_iakerb("client".into())),
        )
        .is_err()
    );
    let (sequence, _) = CredsspSequence::init_for_connector(
        &connector,
        "server.example.test".into(),
        vec![1],
        Some(KerberosConfig::new_with_kdc_url(None, "client".into()).unwrap()),
    )
    .unwrap();
    assert!(sequence.remote_guard_enabled());
    assert!(connector.complete_credssp(sequence).is_err());
    assert!(connector.should_perform_credssp());
    assert!(
        !connector
            .get_static_channel_processor::<DrdynvcClient>()
            .unwrap()
            .has_channel_name(ironrdp_rdpear::CHANNEL_NAME)
    );
}

#[test]
fn ordinary_connection_keeps_its_negotiation_flags_and_mode() {
    let mut connector = connector();
    let mut output = WriteBuf::new();
    connector.step_no_input(&mut output).unwrap();
    let request = decode::<X224<nego::ConnectionRequest>>(output.filled()).unwrap().0;
    assert_eq!(request.flags, nego::RequestFlags::empty());
    assert!(request.protocol.contains(nego::SecurityProtocol::SSL));
    assert!(!connector.remote_guard_enabled());
}

fn rcg_sequence() -> CredsspSequence {
    let mut rcg = connector().with_remote_guard(DrdynvcClient::new()).unwrap();
    rcg.state = ClientConnectorState::Credssp {
        selected_protocol: nego::SecurityProtocol::HYBRID_EX,
    };
    CredsspSequence::init_for_connector(
        &rcg,
        "server.example.test".into(),
        vec![1],
        Some(KerberosConfig::new_with_kdc_url(None, "client".into()).unwrap()),
    )
    .unwrap()
    .0
}

#[test]
fn rcg_context_cannot_be_taken_before_authentication() {
    let mut sequence = rcg_sequence();
    assert!(sequence.__test_take_remote_guard_session().is_err());
    sequence
        .handle_process_result(
            ClientState::FinalMessage(credssp::TsRequest::default()),
            &mut WriteBuf::new(),
        )
        .unwrap();
    assert!(sequence.__test_take_remote_guard_session().is_err());
    assert!(
        sequence
            .handle_process_result(
                ClientState::ReplyNeeded(credssp::TsRequest::default()),
                &mut WriteBuf::new(),
            )
            .is_err(),
        "a client result cannot bypass server authorization"
    );
    assert!(!sequence.__test_is_finished());
    sequence.decode_server_message(&0u32.to_le_bytes()).unwrap();
    assert!(sequence.__test_is_finished());
    assert!(
        sequence.__test_take_remote_guard_session().is_err(),
        "the real SSPI context must also be authenticated"
    );
}

#[test]
fn rcg_early_auth_denial_or_malformed_result_is_terminal() {
    for input in [5u32.to_le_bytes().to_vec(), vec![0], 1u32.to_le_bytes().to_vec()] {
        let mut sequence = rcg_sequence();
        sequence
            .handle_process_result(
                ClientState::FinalMessage(credssp::TsRequest::default()),
                &mut WriteBuf::new(),
            )
            .unwrap();
        assert!(sequence.decode_server_message(&input).is_err());
        assert!(sequence.__test_is_failed());
        assert!(!sequence.__test_is_finished());
        assert!(sequence.__test_take_remote_guard_session().is_err());
        assert!(sequence.decode_server_message(&0u32.to_le_bytes()).is_err());
    }
}

struct ScriptedContext {
    plaintext: Vec<u8>,
    fail_unwrap: bool,
    fail_wrap: bool,
    ntlm_available: bool,
    ntlm_error: Option<ErrorKind>,
    executed: Arc<AtomicUsize>,
    ntlm_executed: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}

impl Drop for ScriptedContext {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

impl Context for ScriptedContext {
    fn unwrap(&mut self, token: &[u8]) -> ironrdp_connector::sspi::Result<Secret<Vec<u8>>> {
        assert_eq!(token, b"test-token");
        if self.fail_unwrap {
            return Err(Error::new(ErrorKind::MessageAltered, "test integrity failure"));
        }
        Ok(Secret::new(self.plaintext.clone()))
    }

    fn wrap(&mut self, plaintext: &[u8]) -> ironrdp_connector::sspi::Result<Vec<u8>> {
        if self.fail_wrap {
            return Err(Error::new(ErrorKind::ContextExpired, "test wrap failure"));
        }
        let mut token = vec![0xa5; WRAP_OVERHEAD];
        token.extend_from_slice(plaintext);
        Ok(token)
    }

    fn execute(&mut self, request: &Request) -> ironrdp_connector::sspi::Result<Response> {
        self.executed.fetch_add(1, Ordering::SeqCst);
        if let Request::ComputeTgsChecksum(request) = request {
            assert_eq!(request.checksum_type, 16);
            return Ok(Response::ComputeTgsChecksum(asn1_output(vec![0x30, 0])?));
        }
        Err(Error::new(ErrorKind::UnsupportedFunction, "test unsupported call"))
    }

    fn ntlm_available(&self) -> bool {
        self.ntlm_available
    }

    fn execute_ntlm(&mut self, request: &ndr::NtlmRequest) -> ironrdp_connector::sspi::Result<ndr::NtlmResponseCall> {
        self.ntlm_executed.fetch_add(1, Ordering::SeqCst);
        if let Some(kind) = self.ntlm_error {
            return Err(Error::new(kind, "test NTLM failure"));
        }
        match request {
            ndr::NtlmRequest::Lm20GetNtlm3ChallengeResponse(_) => {
                assert_eq!(request, &ntlm_v2_request());
                Ok(ndr::NtlmResponseCall::Lm20GetNtlm3ChallengeResponse(
                    ndr::Lm20GetNtlm3ChallengeResponseResponse {
                        ntlm3_response: ndr::NtlmResponseBuffer {
                            length: 512,
                            value: Some(vec![0x11; 512]),
                        },
                        lm3_response: ndr::Opaque([0x22; 24]),
                        user_session_key: ndr::Opaque([0x33; 16]),
                        lm_session_key: ndr::Opaque([0x44; 8]),
                    },
                ))
            }
            ndr::NtlmRequest::CompareCredentials(request) => {
                assert_eq!(request.lhs_credential, Some(ntlm_credential()));
                assert_eq!(request.rhs_credential, Some(ntlm_credential()));
                Ok(ndr::NtlmResponseCall::CompareCredentials(
                    ndr::CompareCredentialsResponse {
                        are_nt_owfs_equal: 1,
                        are_lm_owfs_equal: 0,
                        are_sha_owfs_equal: 0,
                    },
                ))
            }
            _ => Err(Error::new(ErrorKind::UnsupportedFunction, "unexpected NTLM dispatch")),
        }
    }
}

fn backend(request: &Request) -> Backend<ScriptedContext> {
    Backend {
        context: Some(ScriptedContext {
            plaintext: packet(
                KERBEROS,
                &request
                    .encode_with_profile(ndr::Endianness::Little, ndr::NdrLimits::default(), PROFILE)
                    .unwrap(),
            ),
            fail_unwrap: false,
            fail_wrap: false,
            ntlm_available: true,
            ntlm_error: None,
            executed: Arc::default(),
            ntlm_executed: Arc::default(),
            dropped: Arc::default(),
        }),
        negotiated: false,
        ntlm_negotiated: false,
        kerberos_allowed: true,
    }
}

fn packet(package_name: &[u8], call: &[u8]) -> Vec<u8> {
    encode_vec(&RemoteGuardInnerPacket {
        package_name,
        package_call: call,
    })
    .unwrap()
}

fn version(version: u32) -> Request {
    Request::NegotiateVersion(ndr::NegotiateVersion { version })
}

fn response(backend: &mut Backend<ScriptedContext>) -> ndr::KerberosResponse {
    let token = backend.process(b"test-token", MAX_MESSAGE_SIZE).unwrap();
    assert_eq!(&token[..WRAP_OVERHEAD], &[0xa5; WRAP_OVERHEAD]);
    let inner = RemoteGuardInnerPacket::decode(&mut ReadCursor::new(&token[WRAP_OVERHEAD..])).unwrap();
    assert_eq!(inner.package_name, KERBEROS);
    ndr::KerberosResponse::decode_with_profile(inner.package_call, ndr::NdrLimits::default(), PROFILE).unwrap()
}

fn ntlm_credential() -> ndr::EncryptedSecrets {
    ndr::EncryptedSecrets {
        reserved1: 1,
        reserved2: 0,
        reserved3: 0,
        reserved4: ndr::CredentialKeyType::DomainUser,
        reserved5: ndr::Opaque([0x55; 20]),
        reserved6: output(vec![0x66; 32]).unwrap(),
    }
}

fn ntlm_v2_request() -> ndr::NtlmRequest {
    return ndr::NtlmRequest::Lm20GetNtlm3ChallengeResponse(ndr::Lm20GetNtlm3ChallengeResponseRequest {
        credential: Some(ntlm_credential()),
        user_name: Some(name("User")),
        logon_domain_name: Some(name("DOMAIN")),
        server_name: Some(ndr::UnicodeString {
            length: 4,
            maximum_length: 4,
            buffer: Some(vec![0, 0]),
        }),
        challenge_to_client: ndr::Opaque([0x77; 8]),
    });

    fn name(value: &str) -> ndr::UnicodeString {
        let buffer = value.encode_utf16().collect::<Vec<_>>();
        let length = u16::try_from(buffer.len() * 2).unwrap();
        ndr::UnicodeString {
            length,
            maximum_length: length,
            buffer: Some(buffer),
        }
    }
}

fn set_ntlm_request(backend: &mut Backend<ScriptedContext>, request: &ndr::NtlmRequest) {
    backend.context.as_mut().unwrap().plaintext = packet(
        NTLM,
        &request
            .encode_with_profile(ndr::Endianness::Little, ndr::NdrLimits::default(), PROFILE)
            .unwrap(),
    );
}

fn ntlm_response(backend: &mut Backend<ScriptedContext>) -> ndr::NtlmResponse {
    let token = backend.process(b"test-token", MAX_MESSAGE_SIZE).unwrap();
    assert_eq!(&token[..WRAP_OVERHEAD], &[0xa5; WRAP_OVERHEAD]);
    let inner = RemoteGuardInnerPacket::decode(&mut ReadCursor::new(&token[WRAP_OVERHEAD..])).unwrap();
    assert_eq!(inner.package_name, NTLM);
    ndr::NtlmResponse::decode_with_profile(inner.package_call, ndr::NdrLimits::default(), PROFILE).unwrap()
}

#[test]
fn rcg_version_is_negotiated_before_operation_dispatch() {
    let request = Request::ComputeTgsChecksum(ndr::ComputeTgsChecksumRequest {
        checksum_type: 16,
        ..Default::default()
    });
    let mut backend = backend(&request);
    let calls = Arc::clone(&backend.context.as_ref().unwrap().executed);
    let rejected = response(&mut backend);
    assert_eq!(rejected.status, STATUS_INVALID_PARAMETER);
    assert_eq!(rejected.call.call_id(), request.call_id());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let operation = backend.context.as_ref().unwrap().plaintext.clone();
    backend.context.as_mut().unwrap().plaintext = packet(
        KERBEROS,
        &version(0)
            .encode_with_profile(ndr::Endianness::Little, ndr::NdrLimits::default(), PROFILE)
            .unwrap(),
    );
    assert_eq!(
        response(&mut backend),
        ndr::KerberosResponse {
            status: 0,
            call: Response::NegotiateVersion(ndr::NegotiateVersion { version: 0 })
        }
    );
    backend.context.as_mut().unwrap().plaintext = operation;
    assert_eq!(
        response(&mut backend),
        ndr::KerberosResponse {
            status: 0,
            call: Response::ComputeTgsChecksum(asn1_output(vec![0x30, 0]).unwrap()),
        }
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn rcg_unsupported_version_does_not_authorize_operations() {
    let mut backend = backend(&version(1));
    assert_eq!(response(&mut backend).status, STATUS_NOT_SUPPORTED);
    assert!(!backend.negotiated);
}

#[test]
fn rcg_unsupported_operation_has_matching_error_response() {
    let mut backend = backend(&Request::CreateDhKeyAgreement(ndr::Ignored { ignored: 0 }));
    backend.negotiated = true;
    let response = response(&mut backend);
    assert_eq!(response.status, STATUS_NOT_SUPPORTED);
    assert_eq!(response.call.call_id(), 0x115);
    assert!(backend.context.is_some());
}

#[test]
fn rcg_ntlm_only_policy_rejects_kerberos_but_not_ntlm() {
    let mut backend = backend(&version(0));
    backend.kerberos_allowed = false;
    assert_eq!(
        response(&mut backend),
        ndr::KerberosResponse {
            status: STATUS_NOT_SUPPORTED,
            call: Response::NegotiateVersion(ndr::NegotiateVersion { version: 0 }),
        }
    );
    assert!(!backend.negotiated);
    set_ntlm_request(
        &mut backend,
        &ndr::NtlmRequest::NegotiateVersion(ndr::NegotiateVersion { version: 0 }),
    );
    assert_eq!(ntlm_response(&mut backend).status, 0);
    assert!(backend.ntlm_negotiated);
}

#[test]
fn rcg_ntlm_version_is_independent_and_dispatches_v2_only_after_negotiation() {
    let mut backend = backend(&version(0));
    assert_eq!(response(&mut backend).status, 0);
    assert!(backend.negotiated);
    assert!(!backend.ntlm_negotiated);
    let calls = Arc::clone(&backend.context.as_ref().unwrap().ntlm_executed);
    set_ntlm_request(&mut backend, &ntlm_v2_request());
    let rejected = ntlm_response(&mut backend);
    assert_eq!(rejected.status, STATUS_INVALID_PARAMETER);
    assert_eq!(rejected.call.wire_call_id(PROFILE), 0x202);
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    set_ntlm_request(
        &mut backend,
        &ndr::NtlmRequest::NegotiateVersion(ndr::NegotiateVersion { version: 0 }),
    );
    assert_eq!(
        ntlm_response(&mut backend),
        ndr::NtlmResponse {
            status: 0,
            call: ndr::NtlmResponseCall::NegotiateVersion(ndr::NegotiateVersion { version: 0 }),
        }
    );
    set_ntlm_request(&mut backend, &ntlm_v2_request());
    assert_eq!(
        ntlm_response(&mut backend),
        ndr::NtlmResponse {
            status: 0,
            call: ndr::NtlmResponseCall::Lm20GetNtlm3ChallengeResponse(ndr::Lm20GetNtlm3ChallengeResponseResponse {
                ntlm3_response: ndr::NtlmResponseBuffer {
                    length: 512,
                    value: Some(vec![0x11; 512]),
                },
                lm3_response: ndr::Opaque([0x22; 24]),
                user_session_key: ndr::Opaque([0x33; 16]),
                lm_session_key: ndr::Opaque([0x44; 8]),
            }),
        }
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(backend.context.as_ref().unwrap().executed.load(Ordering::SeqCst), 0);
}

#[test]
fn rcg_ntlm_negotiation_does_not_authorize_kerberos() {
    let request = Request::ComputeTgsChecksum(Default::default());
    let mut backend = backend(&request);
    let kerberos_packet = backend.context.as_ref().unwrap().plaintext.clone();
    set_ntlm_request(
        &mut backend,
        &ndr::NtlmRequest::NegotiateVersion(ndr::NegotiateVersion { version: 0 }),
    );
    assert_eq!(ntlm_response(&mut backend).status, 0);
    assert!(!backend.negotiated);
    backend.context.as_mut().unwrap().plaintext = kerberos_packet;
    assert_eq!(response(&mut backend).status, STATUS_INVALID_PARAMETER);
    assert_eq!(backend.context.as_ref().unwrap().executed.load(Ordering::SeqCst), 0);
}

#[test]
fn rcg_ntlm_unavailable_credentials_and_versions_do_not_enable_dispatch() {
    for available in [false, true] {
        let mut backend = backend(&version(0));
        backend.context.as_mut().unwrap().ntlm_available = available;
        for version in if available { &[1][..] } else { &[0, 1][..] } {
            set_ntlm_request(
                &mut backend,
                &ndr::NtlmRequest::NegotiateVersion(ndr::NegotiateVersion { version: *version }),
            );
            assert_eq!(ntlm_response(&mut backend).status, STATUS_NOT_SUPPORTED);
            assert!(!backend.ntlm_negotiated);
        }
        set_ntlm_request(&mut backend, &ntlm_v2_request());
        assert_eq!(
            ntlm_response(&mut backend).status,
            if available {
                STATUS_INVALID_PARAMETER
            } else {
                STATUS_NOT_SUPPORTED
            }
        );
        assert_eq!(
            backend.context.as_ref().unwrap().ntlm_executed.load(Ordering::SeqCst),
            0
        );
    }
}

#[test]
fn rcg_ntlm_v1_operations_remain_disabled_after_negotiation() {
    let mut backend = backend(&version(0));
    backend.ntlm_negotiated = true;
    for request in [
        ndr::NtlmRequest::CalculateNtResponse(Default::default()),
        ndr::NtlmRequest::CalculateUserSessionKeyNt(Default::default()),
    ] {
        set_ntlm_request(&mut backend, &request);
        let response = ntlm_response(&mut backend);
        assert_eq!(response.status, STATUS_NOT_SUPPORTED);
        assert_eq!(response.call, empty_ntlm_response(&request));
    }
    assert_eq!(
        backend.context.as_ref().unwrap().ntlm_executed.load(Ordering::SeqCst),
        0
    );
    assert!(backend.context.is_some());
}

#[test]
fn rcg_ntlm_comparison_preserves_each_equality_result() {
    let mut backend = backend(&version(0));
    backend.ntlm_negotiated = true;
    set_ntlm_request(
        &mut backend,
        &ndr::NtlmRequest::CompareCredentials(ndr::CompareCredentialsRequest {
            lhs_credential: Some(ntlm_credential()),
            rhs_credential: Some(ntlm_credential()),
        }),
    );
    assert_eq!(
        ntlm_response(&mut backend),
        ndr::NtlmResponse {
            status: 0,
            call: ndr::NtlmResponseCall::CompareCredentials(ndr::CompareCredentialsResponse {
                are_nt_owfs_equal: 1,
                are_lm_owfs_equal: 0,
                are_sha_owfs_equal: 0,
            }),
        }
    );
    assert_eq!(
        backend.context.as_ref().unwrap().ntlm_executed.load(Ordering::SeqCst),
        1
    );
    assert_eq!(backend.context.as_ref().unwrap().executed.load(Ordering::SeqCst), 0);
}

#[test]
fn rcg_ntlm_operation_errors_are_protected_and_do_not_close_the_context() {
    let request = ntlm_v2_request();
    for (kind, status) in [
        (ErrorKind::InvalidParameter, STATUS_INVALID_PARAMETER),
        (ErrorKind::InvalidHandle, STATUS_INVALID_HANDLE),
        (ErrorKind::NoCredentials, STATUS_ACCESS_DENIED),
        (ErrorKind::LogonDenied, STATUS_ACCESS_DENIED),
        (ErrorKind::UnsupportedFunction, STATUS_NOT_SUPPORTED),
    ] {
        let mut backend = backend(&version(0));
        backend.ntlm_negotiated = true;
        backend.context.as_mut().unwrap().ntlm_error = Some(kind);
        set_ntlm_request(&mut backend, &request);
        assert_eq!(
            ntlm_response(&mut backend),
            ndr::NtlmResponse {
                status,
                call: empty_ntlm_response(&request),
            }
        );
        assert!(backend.context.is_some());
        assert_eq!(backend.context.as_ref().unwrap().dropped.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn rcg_ntlm_malformed_packets_or_protection_failures_discard_both_packages() {
    for failure in 0..3 {
        let mut backend = backend(&version(0));
        backend.negotiated = true;
        backend.ntlm_negotiated = true;
        set_ntlm_request(&mut backend, &ntlm_v2_request());
        let context = backend.context.as_mut().unwrap();
        let dropped = Arc::clone(&context.dropped);
        match failure {
            0 => context.plaintext = packet(NTLM, &[0]),
            1 => context.fail_unwrap = true,
            2 => context.fail_wrap = true,
            _ => unreachable!(),
        }
        assert!(backend.process(b"test-token", MAX_MESSAGE_SIZE).is_err());
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert!(!backend.negotiated);
        assert!(!backend.ntlm_negotiated);
        assert!(backend.context.is_none());
        assert!(backend.process(b"test-token", MAX_MESSAGE_SIZE).is_err());
    }
}

#[test]
fn rcg_ntlm_response_limit_is_exact_and_failure_discards_both_packages() {
    let mut backend = backend(&version(0));
    backend.negotiated = true;
    backend.ntlm_negotiated = true;
    set_ntlm_request(&mut backend, &ntlm_v2_request());
    let token = backend.process(b"test-token", MAX_MESSAGE_SIZE).unwrap();
    assert_eq!(backend.process(b"test-token", token.len()).unwrap().len(), token.len());
    assert!(backend.process(b"test-token", token.len() - 1).is_err());
    assert!(backend.context.is_none());
    assert!(!backend.negotiated);
    assert!(!backend.ntlm_negotiated);
}

#[test]
fn rcg_authentication_or_encoding_failure_drops_the_context() {
    for failure in 0..5 {
        let mut backend = backend(&version(0));
        let context = backend.context.as_mut().unwrap();
        let dropped = Arc::clone(&context.dropped);
        match failure {
            0 => context.fail_unwrap = true,
            1 => context.fail_wrap = true,
            2 => context.plaintext = vec![0xff],
            3 => context.plaintext = packet(b"unauthorized", &[0]),
            4 => context.plaintext = packet(KERBEROS, &[0]),
            _ => unreachable!(),
        }
        assert!(backend.process(b"test-token", MAX_MESSAGE_SIZE).is_err());
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert!(backend.process(b"test-token", MAX_MESSAGE_SIZE).is_err());
    }
}

#[test]
fn rcg_response_limit_is_checked_before_wrapping() {
    let mut backend = backend(&version(0));
    let token = backend.process(b"test-token", MAX_MESSAGE_SIZE).unwrap();
    let exact_limit = token.len();
    assert_eq!(backend.process(b"test-token", exact_limit).unwrap().len(), exact_limit);
    assert!(backend.process(b"test-token", exact_limit - 1).is_err());
    assert!(backend.context.is_none());
}

#[test]
fn rcg_request_limit_precedes_unwrap_and_close_is_terminal() {
    let mut backend = backend(&version(0));
    assert!(
        backend
            .process(&vec![0; MAX_MESSAGE_SIZE + 1], MAX_MESSAGE_SIZE)
            .is_err()
    );
    assert!(backend.context.is_none());
    let mut backend = self::backend(&version(0));
    backend.negotiated = true;
    backend.ntlm_negotiated = true;
    let dropped = Arc::clone(&backend.context.as_ref().unwrap().dropped);
    backend.close();
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(!backend.negotiated);
    assert!(!backend.ntlm_negotiated);
    assert!(backend.process(b"test-token", MAX_MESSAGE_SIZE).is_err());
}

#[test]
fn rcg_dvc_wraps_the_backend_response_and_releases_it_on_close() {
    let backend = backend(&version(0));
    let dropped = Arc::clone(&backend.context.as_ref().unwrap().dropped);
    let mut client = AuthRedirectionClient::new(Box::new(backend), 4096).unwrap();
    client.start(11).unwrap();
    let request = encode_vec(&ironrdp_rdpear::AuthRedirectionPdu {
        reserved: 0,
        encrypted_payload: b"test-token".to_vec(),
    })
    .unwrap();
    let messages = client.process(11, &request).unwrap();
    assert_eq!(messages.len(), 1);
    client.close(11);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(client.process(11, &request).is_err());
}

#[test]
fn rcg_key_and_unicode_adapters_preserve_values_and_reject_invalid_pointers() {
    let encryption_key = ndr::EncryptionKey {
        reserved2: 18,
        reserved3: output(vec![1, 2, 3]).unwrap(),
        ..Default::default()
    };
    let reference = key(&encryption_key).unwrap();
    assert_eq!(reference.encryption_type, 18);
    assert_eq!(reference.value, &[1, 2, 3]);
    assert!(key(&ndr::EncryptionKey::default()).is_err());
    assert!(
        bytes(&ndr::OctetString {
            length: 10,
            value: Some(vec![1])
        })
        .is_err()
    );
    assert!(
        unicode(&ndr::UnicodeString {
            length: 2,
            maximum_length: 2,
            buffer: Some(vec![0xd800])
        })
        .is_err()
    );
    let unicode_name = ndr::UnicodeString {
        length: 6,
        maximum_length: 6,
        buffer: Some(vec![0x61, 0xd83d, 0xde00]),
    };
    assert_eq!(unicode(&unicode_name).unwrap(), "a\u{1f600}");
}

#[test]
fn rcg_ntlm_adapter_preserves_opaque_metadata_and_rejects_missing_or_inconsistent_buffers() {
    for (kind, discriminant) in [
        (ndr::CredentialKeyType::Invalid, 0),
        (ndr::CredentialKeyType::Ium, 1),
        (ndr::CredentialKeyType::DomainUser, 2),
        (ndr::CredentialKeyType::LocalUser, 3),
        (ndr::CredentialKeyType::ExternallySupplied, 4),
    ] {
        let credential = ndr::EncryptedSecrets {
            reserved1: 2,
            reserved2: 1,
            reserved3: 0,
            reserved4: kind,
            ..ntlm_credential()
        };
        let reference = map_ntlm_credential(&credential).unwrap();
        assert_eq!(reference.reserved1, 2);
        assert_eq!(reference.reserved2, 1);
        assert_eq!(reference.reserved3, 0);
        assert_eq!(reference.credential_key_type, discriminant);
        assert_eq!(reference.credential_key, &[0x55; 20]);
        assert_eq!(reference.encrypted_secrets, &[0x66; 32]);
    }
    let mut credential = ntlm_credential();
    credential.reserved6.length += 1;
    assert_eq!(
        map_ntlm_credential(&credential).unwrap_err().error_type,
        ErrorKind::InvalidParameter
    );
    credential.reserved6 = ndr::OctetString::default();
    assert_eq!(
        map_ntlm_credential(&credential).unwrap_err().error_type,
        ErrorKind::InvalidParameter
    );
}

#[test]
fn rcg_ntlm_server_name_is_binary_utf16le_not_decoded_text() {
    let value = ndr::UnicodeString {
        length: 6,
        maximum_length: 6,
        buffer: Some(vec![0x0200, 0xd800, 0xffff]),
    };
    assert_eq!(unicode_bytes(&value).unwrap(), [0, 2, 0, 0xd8, 0xff, 0xff]);
    assert!(unicode(&value).is_err());
    assert_eq!(
        unicode_bytes(&ndr::UnicodeString::default()).unwrap_err().error_type,
        ErrorKind::InvalidParameter
    );
    let mut invalid_length = value;
    invalid_length.length = 4;
    assert_eq!(
        unicode_bytes(&invalid_length).unwrap_err().error_type,
        ErrorKind::InvalidParameter
    );
    assert_eq!(
        unicode_bytes(&ndr::UnicodeString {
            buffer: Some(Vec::new()),
            ..Default::default()
        })
        .unwrap(),
        Vec::<u8>::new()
    );
}
