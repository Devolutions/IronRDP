#![allow(unused_crate_dependencies)]

use core::time::Duration;

use ironrdp_mstsgu::rpc::{
    PTYPE_REQUEST, PTYPE_RESPONSE, PTYPE_RTS, RPC_AUTH_LEVEL_PACKET_INTEGRITY, RPC_AUTH_TYPE_WINNT,
    RPC_NTLM_VERIFIER_LENGTH, RPC_SECURITY_TRAILER_SIZE, RpcAuthenticationInfo, RpcNtlmAuth, RtsCookie, RtsDestination,
    RtsFlowControlAck, decode_rts_flow_control_ack_with_destination, encode_rpc_response,
    encode_rts_flow_control_ack_with_destination, encode_rts_ping, prepare_rpc_authenticated_pdu,
};
use ironrdp_mstsgu::{GwConnectTarget, GwErrorKind, RpchClient};
use ironrdp_tls::CertificateValidation;
use sspi::{
    AuthIdentity, BufferType, CredentialUse, DataRepresentation, Ntlm, SecurityBuffer, SecurityBufferRef,
    ServerRequestFlags, Sspi as _, SspiImpl as _, Username,
};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_native_tls::native_tls::{Identity, TlsAcceptor as NativeTlsAcceptor};
use tokio_native_tls::{TlsAcceptor, TlsStream};

type ServerStream = TlsStream<TcpStream>;

#[tokio::test]
async fn live_rpch_authenticates_both_channels_and_preserves_caller_verifiers() {
    let (mut client, mut input, mut out, _) = live_channel(8192).await;
    assert_eq!(client.connection_timeout(), Duration::from_secs(120));
    assert_eq!(client.send_available_window(), 8192);

    client.send_ping().await.expect("send idle ping");
    assert_eq!(read_pdu(&mut input).await, encode_rts_ping().expect("ping"));
    assert_eq!(client.send_available_window(), 8192);

    let request = authenticated_fragment(PTYPE_REQUEST, 16);
    client
        .send_fragment(&request)
        .await
        .expect("send caller-signed request");
    assert_eq!(read_pdu(&mut input).await, request);
    assert_eq!(
        client.send_available_window(),
        8192 - u32::try_from(request.len()).expect("length")
    );

    let response = authenticated_fragment(PTYPE_RESPONSE, 16);
    out.write_all(&response[..7]).await.expect("split header");
    out.write_all(&response[7..]).await.expect("remaining fragment");
    assert_eq!(
        client.receive_fragment().await.expect("receive unverified response"),
        response
    );
}

#[tokio::test]
async fn live_rpch_rejects_unsigned_malformed_and_batched_requests_before_writing() {
    let (mut client, mut input, _out, _) = live_channel(8192).await;
    let valid = authenticated_fragment(PTYPE_REQUEST, 16);
    let mut bad_version = valid.clone();
    bad_version[0] = 4;
    let mut bad_drep = valid.clone();
    bad_drep[4] = 0;
    let mut bad_auth = valid.clone();
    bad_auth[10..12].copy_from_slice(&u16::MAX.to_le_bytes());
    let mut unsigned = valid.clone();
    unsigned[10..12].copy_from_slice(&0u16.to_le_bytes());
    let mut low_integrity = valid.clone();
    let security_trailer = low_integrity.len() - usize::from(RPC_NTLM_VERIFIER_LENGTH) - RPC_SECURITY_TRAILER_SIZE;
    low_integrity[security_trailer + 1] = RPC_AUTH_LEVEL_PACKET_INTEGRITY - 1;
    for invalid in [
        Vec::new(),
        valid[..15].to_vec(),
        valid[..valid.len() - 1].to_vec(),
        [valid.as_slice(), valid.as_slice()].concat(),
        bad_version,
        bad_drep,
        bad_auth,
        unsigned,
        low_integrity,
        encode_rts_ping().expect("ping"),
    ] {
        assert!(client.send_fragment(&invalid).await.is_err());
        assert_eq!(client.send_available_window(), 8192);
    }
    client.send_ping().await.expect("send marker");
    assert_eq!(read_pdu(&mut input).await, encode_rts_ping().expect("ping"));
}

#[tokio::test]
async fn live_rpch_respects_send_credit_and_excludes_rts_from_accounting() {
    let fragment = authenticated_fragment(PTYPE_REQUEST, 8192);
    let length = u32::try_from(fragment.len()).expect("length");
    let (mut client, mut input, mut out, cookie) = live_channel(2 * length).await;
    client.send_fragment(&fragment).await.expect("first fragment");
    assert_eq!(read_pdu(&mut input).await, fragment);
    assert_eq!(client.send_available_window(), length);
    assert!(client.send_fragment(&fragment).await.is_err());
    assert_eq!(client.send_available_window(), length);

    out.write_all(&encode_rts_ping().expect("ping"))
        .await
        .expect("peer ping");
    let ack = encode_rts_flow_control_ack_with_destination(
        RtsFlowControlAck::new(length, 2 * length, cookie),
        RtsDestination::Client,
    )
    .expect("ack");
    out.write_all(&ack).await.expect("peer ack");
    let response = encode_rpc_response(1, b"response").expect("response");
    out.write_all(&response).await.expect("peer response");
    assert_eq!(client.receive_fragment().await.expect("response after RTS"), response);
    assert_eq!(client.send_available_window(), 2 * length);
    client.send_fragment(&fragment).await.expect("credit replenished");
    assert_eq!(read_pdu(&mut input).await, fragment);
}

#[tokio::test]
async fn live_rpch_releases_receive_credit_and_stops_at_declared_out_body() {
    let (mut client, mut input, mut out, _) = live_channel(256 * 1024).await;
    let response = encode_rpc_response(1, &vec![0x42; 65511]).expect("maximum fragment");
    assert_eq!(response.len(), 65535);
    let second_response = encode_rpc_response(2, &vec![0x42; 65416]).expect("second fragment");
    for response in [&response, &second_response] {
        out.write_all(response).await.expect("peer response");
        assert_eq!(&client.receive_fragment().await.expect("large response"), response);
    }
    let ack = decode_rts_flow_control_ack_with_destination(&read_pdu(&mut input).await).expect("receive-window ack");
    assert_eq!(ack.destination(), RtsDestination::OutProxy);
    let ack = ack.acknowledgement();
    assert_eq!(ack.bytes_received(), 130975);
    assert_eq!(ack.available_window(), 128 * 1024);
    out.write_all(&response[..16]).await.expect("oversized final header");
    let error = client.receive_fragment().await.expect_err("body overrun");
    assert!(error.to_string().contains("exceeds OUT body"), "{error}");
}

#[tokio::test]
async fn live_rpch_stops_at_in_body_limit_without_recycling() {
    let (mut client, mut input, _out, _) = live_channel(256 * 1024).await;
    let fragment = authenticated_fragment(PTYPE_REQUEST, 60000);
    for _ in 0..2 {
        client.send_fragment(&fragment).await.expect("large request");
        assert_eq!(read_pdu(&mut input).await, fragment);
    }
    let credit = client.send_available_window();
    let error = client.send_fragment(&fragment).await.expect_err("body exhausted");
    assert!(matches!(error.kind(), GwErrorKind::UnsupportedFeature));
    assert_eq!(client.send_available_window(), credit);
}

#[tokio::test]
async fn live_rpch_rejects_malformed_peer_headers_and_unsupported_rts() {
    for variant in 0..3 {
        let (mut client, _input, mut out, _) = live_channel(8192).await;
        let mut pdu = encode_rts_ping().expect("ping");
        match variant {
            0 => pdu[0] = 4,
            1 => pdu[8..10].copy_from_slice(&15u16.to_le_bytes()),
            _ => pdu[16] = 0x40,
        }
        out.write_all(&pdu).await.expect("bad peer PDU");
        let error = client.receive_fragment().await.expect_err("reject bad PDU");
        if variant == 2 {
            assert!(matches!(error.kind(), GwErrorKind::UnsupportedFeature));
        } else {
            assert!(error.to_string().contains("validate rpch pdu header"), "{error}");
        }
    }
}

#[test]
fn rpc_ntlm_context_signs_and_verifies_integrity_fragments() {
    let username = "rpch-test-user";
    let password = "rpch-test-password";
    let mut client = RpcNtlmAuth::new(username, password).expect("create client NTLM context");
    let mut server = Ntlm::new();
    let identity = AuthIdentity {
        username: Username::parse(username).expect("parse test username"),
        password: password.to_owned().into(),
    };
    let mut server_credentials = server
        .acquire_credentials_handle()
        .with_credential_use(CredentialUse::Inbound)
        .with_auth_data(&identity)
        .execute(&mut server)
        .expect("acquire server credentials")
        .credentials_handle;

    let mut server_input = [SecurityBuffer::new(
        client.initial_token().expect("NTLM Type 1"),
        BufferType::Token,
    )];
    let mut server_output = [SecurityBuffer::new(Vec::new(), BufferType::Token)];
    let builder = server
        .accept_security_context()
        .with_credentials_handle(&mut server_credentials)
        .with_context_requirements(
            ServerRequestFlags::INTEGRITY
                | ServerRequestFlags::REPLAY_DETECT
                | ServerRequestFlags::SEQUENCE_DETECT
                | ServerRequestFlags::ALLOCATE_MEMORY,
        )
        .with_target_data_representation(DataRepresentation::Native)
        .with_input(&mut server_input)
        .with_output(&mut server_output);
    let response = server
        .accept_security_context_impl(builder)
        .expect("initialize server security context")
        .resolve_to_result()
        .expect("accept NTLM Type 1");
    assert_eq!(response.status, sspi::SecurityStatus::ContinueNeeded);
    let type3 = client
        .continue_token(&server_output[0].buffer)
        .expect("produce NTLM Type 3");

    server_input[0].buffer = type3;
    server_output[0].buffer.clear();
    let builder = server
        .accept_security_context()
        .with_credentials_handle(&mut server_credentials)
        .with_context_requirements(
            ServerRequestFlags::INTEGRITY
                | ServerRequestFlags::REPLAY_DETECT
                | ServerRequestFlags::SEQUENCE_DETECT
                | ServerRequestFlags::ALLOCATE_MEMORY,
        )
        .with_target_data_representation(DataRepresentation::Native)
        .with_input(&mut server_input)
        .with_output(&mut server_output);
    let response = server
        .accept_security_context_impl(builder)
        .expect("continue server security context")
        .resolve_to_result()
        .expect("accept NTLM Type 3");
    assert_eq!(response.status, sspi::SecurityStatus::CompleteNeeded);
    server.complete_auth_token(&mut []).expect("complete NTLM context");
    assert!(client.is_complete());

    let authentication = RpcAuthenticationInfo {
        auth_type: RPC_AUTH_TYPE_WINNT,
        auth_level: RPC_AUTH_LEVEL_PACKET_INTEGRITY,
        auth_context_id: 1,
        verifier_length: RPC_NTLM_VERIFIER_LENGTH,
    };
    let mut last_response = Vec::new();
    for (sequence_number, header_signing) in [(0, false), (1, true)] {
        let mut request = rpc_fragment(PTYPE_REQUEST, authentication, b"request");
        client
            .sign_fragment(&mut request, authentication, sequence_number, header_signing)
            .expect("sign request");
        server_verify_fragment(&mut server, &mut request, sequence_number, header_signing);

        let mut response = rpc_fragment(PTYPE_RESPONSE, authentication, b"response");
        server_sign_fragment(&mut server, &mut response, sequence_number, header_signing);
        client
            .verify_fragment(&mut response, authentication, sequence_number, header_signing)
            .unwrap_or_else(|error| panic!("verify response (header signing {header_signing}): {error}"));
        last_response = response;
    }

    last_response[16] ^= 1;
    assert!(
        client
            .verify_fragment(&mut last_response, authentication, 1, true)
            .is_err(),
        "tampered RPC body must fail NTLM verification"
    );
    assert!(
        !client.is_complete(),
        "failed verification must poison the NTLM association"
    );
    let mut subsequent_request = rpc_fragment(PTYPE_REQUEST, authentication, b"request after tampering");
    assert!(
        client
            .sign_fragment(&mut subsequent_request, authentication, 2, true)
            .is_err(),
        "a poisoned NTLM association must not sign later requests"
    );
}

#[tokio::test]
async fn live_rpch_validates_configuration_before_connecting() {
    let target = gateway_target(1);
    for backend in ["backend", "backend:0", "backend:65536", "backend:3388\r\nInjected: yes"] {
        let result = RpchClient::connect(&target, backend).await;
        assert!(matches!(result, Err(error) if matches!(error.kind(), GwErrorKind::InvalidGwTarget)));
    }
    let result = RpchClient::connect_with_certificate_validation(
        &target,
        "backend:3388",
        CertificateValidation::DangerouslyAcceptInvalidCertificate,
        Some(std::sync::Arc::new(|_, _, _| true)),
    )
    .await;
    assert!(matches!(result, Err(error) if matches!(error.kind(), GwErrorKind::InvalidCertificateValidation)));
}

#[tokio::test]
async fn live_rpch_strict_tls_rejects_self_signed_gateway() {
    let (listener, acceptor) = tls_listener().await;
    let target = gateway_target(listener.local_addr().expect("address").port());
    let server = async {
        let (stream, _) = listener.accept().await.expect("accept TLS");
        let _ = acceptor.accept(stream).await;
    };
    let (result, ()) = tokio::join!(RpchClient::connect(&target, "backend:3388"), server);
    assert!(matches!(result, Err(error) if error.to_string().contains("tls connect")));
}

async fn live_channel(peer_window: u32) -> (RpchClient, ServerStream, ServerStream, RtsCookie) {
    let (listener, acceptor) = tls_listener().await;
    let port = listener.local_addr().expect("address").port();
    let target = gateway_target(port);
    let server = async {
        let (input, _) = listener.accept().await.expect("accept IN");
        let mut input = acceptor.accept(input).await.expect("IN TLS");
        let (out, _) = listener.accept().await.expect("accept OUT");
        let mut out = acceptor.accept(out).await.expect("OUT TLS");
        let head = read_head(&mut input).await;
        assert!(head.starts_with("RPC_IN_DATA /rpc/rpcproxy.dll?backend:3388 HTTP/1.1\r\n"));
        assert!(head.contains(&format!("Host: localhost:{port}\r\n")));
        assert!(head.contains("Content-Length: 131072\r\n"));
        assert!(head.contains("Expect: 100-continue\r\n"));
        assert!(!head.contains("Authorization:"));
        input
            .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 6\r\nWWW-Authenticate: Basic\r\n\r\ndenied")
            .await
            .expect("IN challenge");
        assert!(
            read_head(&mut input)
                .await
                .contains("Authorization: Basic dXNlcjpwYXNz\r\n")
        );
        input
            .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
            .await
            .expect("IN accepted");
        for authenticated in [false, true] {
            let head = read_head(&mut out).await;
            assert!(head.starts_with("RPC_OUT_DATA /rpc/rpcproxy.dll?backend:3388 HTTP/1.1\r\n"));
            assert!(head.contains("Content-Length: 76\r\n"));
            assert_eq!(head.contains("Authorization: Basic dXNlcjpwYXNz\r\n"), authenticated);
            let a1 = read_pdu(&mut out).await;
            assert_eq!(a1.len(), 76);
            assert_eq!(a1[2], PTYPE_RTS);
            if !authenticated {
                out.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nWWW-Authenticate: Basic\r\n\r\n")
                    .await
                    .expect("OUT challenge");
            }
        }
        out.write_all(b"HTTP/1.1 200 Success\r\nContent-Type: application/rpc\r\nContent-Length: 131072\r\n\r\n")
            .await
            .expect("OUT accepted");
        let b1 = read_pdu(&mut input).await;
        assert_eq!(b1.len(), 104);
        let cookie = RtsCookie::new(b1[52..68].try_into().expect("IN cookie"));
        out.write_all(&rts(&[2, 120000], 1)).await.expect("CONN/A3");
        out.write_all(&rts(&[6, 1, 0, peer_window, 2, 180000], 3))
            .await
            .expect("CONN/C2");
        (input, out, cookie)
    };
    let connect = RpchClient::connect_with_certificate_validation(
        &target,
        "backend:3388",
        CertificateValidation::DangerouslyAcceptInvalidCertificate,
        None,
    );
    let (client, (input, out, cookie)) = tokio::join!(connect, server);
    (client.expect("live channel"), input, out, cookie)
}

fn rts(words: &[u32], commands: u16) -> Vec<u8> {
    let mut pdu = vec![5, 0, PTYPE_RTS, 3, 0x10, 0, 0, 0];
    pdu.extend_from_slice(&u16::try_from(20 + words.len() * 4).expect("RTS length").to_le_bytes());
    pdu.extend_from_slice(&[0; 6]);
    pdu.extend_from_slice(&0u16.to_le_bytes());
    pdu.extend_from_slice(&commands.to_le_bytes());
    for word in words {
        pdu.extend_from_slice(&word.to_le_bytes());
    }
    pdu
}

fn authenticated_fragment(ptype: u8, body_length: usize) -> Vec<u8> {
    prepare_rpc_authenticated_pdu(
        ptype,
        3,
        1,
        &vec![0x42; body_length],
        RpcAuthenticationInfo {
            auth_type: 10,
            auth_level: 5,
            auth_context_id: 0,
            verifier_length: 16,
        },
    )
    .expect("prepare authentication framing")
    .finish(&[0xa5; 16])
    .expect("opaque test verifier")
}

fn rpc_fragment(ptype: u8, authentication: RpcAuthenticationInfo, body: &[u8]) -> Vec<u8> {
    prepare_rpc_authenticated_pdu(ptype, 3, 7, body, authentication)
        .expect("prepare integrity PDU")
        .finish(&[0; 16])
        .expect("reserve integrity verifier")
}

fn server_sign_fragment(server: &mut Ntlm, fragment: &mut [u8], sequence_number: u32, header_signing: bool) {
    let header = ironrdp_mstsgu::rpc::RpcCommonHeader::decode(fragment).expect("RPC common header");
    assert_eq!(usize::from(header.fragment_length()), fragment.len());
    let verifier_offset = fragment.len() - usize::from(header.auth_length());
    let security_trailer_offset = verifier_offset - RPC_SECURITY_TRAILER_SIZE;
    let data_start = if header_signing {
        0
    } else {
        ironrdp_mstsgu::rpc::RPC_COMMON_HEADER_SIZE
    };
    let (data, verifier) = fragment.split_at_mut(verifier_offset);
    let data_end = if header_signing {
        verifier_offset
    } else {
        security_trailer_offset
    };
    let mut buffers = [
        SecurityBufferRef::token_buf(verifier),
        SecurityBufferRef::data_buf(&mut data[data_start..data_end]),
    ];
    server
        .make_signature(0, &mut buffers, sequence_number)
        .expect("server signature");
    assert_eq!(security_trailer_offset + RPC_SECURITY_TRAILER_SIZE, verifier_offset);
}

fn server_verify_fragment(server: &mut Ntlm, fragment: &mut [u8], sequence_number: u32, header_signing: bool) {
    let header = ironrdp_mstsgu::rpc::RpcCommonHeader::decode(fragment).expect("RPC common header");
    assert_eq!(usize::from(header.fragment_length()), fragment.len());
    let verifier_offset = fragment.len() - usize::from(header.auth_length());
    let security_trailer_offset = verifier_offset - RPC_SECURITY_TRAILER_SIZE;
    let data_start = if header_signing {
        0
    } else {
        ironrdp_mstsgu::rpc::RPC_COMMON_HEADER_SIZE
    };
    let (data, verifier) = fragment.split_at_mut(verifier_offset);
    let data_end = if header_signing {
        verifier_offset
    } else {
        security_trailer_offset
    };
    let mut buffers = [
        SecurityBufferRef::token_buf(verifier),
        SecurityBufferRef::data_buf(&mut data[data_start..data_end]),
    ];
    server
        .verify_signature(&mut buffers, sequence_number)
        .expect("verify client signature");
}

async fn read_head<S: AsyncRead + Unpin>(stream: &mut S) -> String {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        assert!(bytes.len() < 16384, "bounded request headers");
        bytes.push(stream.read_u8().await.expect("request header byte"));
    }
    String::from_utf8(bytes).expect("request headers")
}

async fn read_pdu<S: AsyncRead + Unpin>(stream: &mut S) -> Vec<u8> {
    let mut header = [0; 16];
    stream.read_exact(&mut header).await.expect("RPC header");
    let length = usize::from(u16::from_le_bytes([header[8], header[9]]));
    assert!(16 <= length);
    let mut pdu = header.to_vec();
    pdu.resize(length, 0);
    stream.read_exact(&mut pdu[16..]).await.expect("RPC body");
    pdu
}

async fn tls_listener() -> (TcpListener, TlsAcceptor) {
    let identity = Identity::from_pkcs8(
        include_bytes!("../../ironrdp-tls/tests/certs/server-cert.pem"),
        include_bytes!("../../ironrdp-tls/tests/certs/server-key.pem"),
    )
    .expect("TLS identity");
    let acceptor = TlsAcceptor::from(NativeTlsAcceptor::new(identity).expect("TLS acceptor"));
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("loopback listener");
    (listener, acceptor)
}

fn gateway_target(port: u16) -> GwConnectTarget {
    GwConnectTarget {
        gw_endpoint: format!("localhost:{port}"),
        gw_user: "user".to_owned(),
        gw_pass: "pass".to_owned(),
        smart_card: None,
        server: "rdp-server.test".to_owned(),
    }
}
