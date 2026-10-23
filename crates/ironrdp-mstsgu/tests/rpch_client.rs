#![allow(unused_crate_dependencies)]

#[cfg(any(feature = "native-tls", feature = "rustls"))]
use core::time::Duration;

use ironrdp_mstsgu::rpc::{
    PTYPE_FAULT, PTYPE_REQUEST, PTYPE_RESPONSE, RPC_AUTH_LEVEL_PACKET_INTEGRITY, RPC_AUTH_TYPE_WINNT,
    RPC_NTLM_VERIFIER_LENGTH, RPC_SECURITY_TRAILER_SIZE, RpcAuthenticationInfo, RpcNtlmAuth,
    prepare_rpc_authenticated_pdu,
};
#[cfg(any(feature = "native-tls", feature = "rustls"))]
use ironrdp_mstsgu::rpc::{
    PTYPE_RTS, RtsCookie, RtsDestination, RtsFlowControlAck, decode_rts_flow_control_ack_with_destination,
    encode_rpc_response, encode_rts_flow_control_ack_with_destination, encode_rts_ping,
};
use ironrdp_mstsgu::{GwConnectTarget, GwErrorKind, RpchClient};
use ironrdp_tls::CertificateValidation;
use sspi::{
    AuthIdentity, BufferType, CredentialUse, DataRepresentation, Ntlm, SecurityBuffer, SecurityBufferRef,
    ServerRequestFlags, Sspi as _, SspiImpl as _, Username,
};
#[cfg(any(feature = "native-tls", feature = "rustls"))]
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};
#[cfg(any(feature = "native-tls", feature = "rustls"))]
use tokio::net::{TcpListener, TcpStream};
#[cfg(any(feature = "native-tls", feature = "rustls"))]
use tokio_native_tls::native_tls::{Identity, TlsAcceptor as NativeTlsAcceptor};
#[cfg(any(feature = "native-tls", feature = "rustls"))]
use tokio_native_tls::{TlsAcceptor, TlsStream};

#[cfg(any(feature = "native-tls", feature = "rustls"))]
type ServerStream = TlsStream<TcpStream>;

#[cfg(any(feature = "native-tls", feature = "rustls"))]
#[tokio::test]
async fn live_rpch_authenticates_both_channels_and_preserves_caller_verifiers() {
    // oracle: regression #2202 keeps the low-level transport opaque to caller-owned RPC verifiers.
    // oracle: spec MS-RPCH §2.1.2.1.7 and §2.1.2.1.8 carry RPC PDUs in channel bodies.
    // oracle: spec MS-RPCE §2.2.2.11 places the opaque authentication token after sec_trailer.
    let (mut client, mut input, mut out, _, _) = live_channel(8192).await;
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

#[cfg(any(feature = "native-tls", feature = "rustls"))]
#[tokio::test]
async fn live_rpch_rejects_unsigned_malformed_and_batched_requests_before_writing() {
    let (mut client, mut input, _out, _, _) = live_channel(8192).await;
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

#[cfg(any(feature = "native-tls", feature = "rustls"))]
#[tokio::test]
async fn live_rpch_respects_send_credit_and_excludes_rts_from_accounting() {
    // oracle: spec MS-RPCH §3.2.1.1.4 excludes RTS from RPC flow-control accounting.
    // oracle: spec MS-RPCH §3.2.1.4.1.3 and §3.2.1.5.1.2 govern send credit and acknowledgements.
    let fragment = authenticated_fragment(PTYPE_REQUEST, 8192);
    let length = u32::try_from(fragment.len()).expect("length");
    let (mut client, mut input, mut out, cookie, _) = live_channel(2 * length).await;
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

#[cfg(any(feature = "native-tls", feature = "rustls"))]
#[tokio::test]
async fn live_rpch_releases_receive_credit_and_stops_at_declared_out_body() {
    // oracle: spec MS-RPCH §3.2.1.4.1.1 releases consumed RPC PDUs and advertises available credit.
    // oracle: spec MS-RPCH §2.1.2.1.8 forbids an OUT PDU from exceeding the declared Content-Length.
    let (mut client, mut input, mut out, _, receive_window) = live_channel(256 * 1024).await;
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
    // oracle: spec MS-RPCH §3.2.1.5.1.1 counts the two delivered RPC PDUs, excluding RTS setup bytes.
    assert_eq!(
        ack.bytes_received(),
        u32::try_from(response.len() + second_response.len()).expect("received bytes")
    );
    // oracle: spec MS-RPCH §3.2.1.1.5.1.2 restores the advertised window when no RPC PDUs remain queued.
    assert_eq!(ack.available_window(), receive_window);
    out.write_all(&response[..16]).await.expect("oversized final header");
    let error = client.receive_fragment().await.expect_err("body overrun");
    assert!(error.to_string().contains("exceeds OUT body"), "{error}");
}

#[cfg(any(feature = "native-tls", feature = "rustls"))]
#[tokio::test]
async fn live_rpch_stops_at_in_body_limit_without_recycling() {
    // oracle: spec MS-RPCH §2.1.2.1.7 forbids an IN PDU from exceeding the declared Content-Length.
    let (mut client, mut input, _out, _, _) = live_channel(256 * 1024).await;
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

#[cfg(any(feature = "native-tls", feature = "rustls"))]
#[tokio::test]
async fn live_rpch_rejects_peer_header_with_unsupported_version() {
    // oracle: spec MS-RPCH §2.2.3.2 requires rpc_vers to be 5.
    let (mut client, _input, mut out, _, _) = live_channel(8192).await;
    let mut pdu = encode_rts_ping().expect("ping");
    pdu[0] = 4;
    out.write_all(&pdu).await.expect("bad peer version");
    let error = client.receive_fragment().await.expect_err("reject bad version");
    assert!(error.to_string().contains("validate rpch pdu header"), "{error}");
}

#[cfg(any(feature = "native-tls", feature = "rustls"))]
#[tokio::test]
async fn live_rpch_rejects_peer_fragment_length_smaller_than_header() {
    // oracle: spec MS-RPCH §2.2.3.2 includes the common header in frag_length.
    let (mut client, _input, mut out, _, _) = live_channel(8192).await;
    let mut pdu = encode_rts_ping().expect("ping");
    pdu[8..10].copy_from_slice(&15u16.to_le_bytes());
    out.write_all(&pdu).await.expect("bad peer length");
    let error = client.receive_fragment().await.expect_err("reject bad length");
    assert!(error.to_string().contains("validate rpch pdu header"), "{error}");
}

#[cfg(any(feature = "native-tls", feature = "rustls"))]
#[tokio::test]
async fn live_rpch_rejects_unsupported_rts_flags() {
    // oracle: regression #2202 requires unsupported RTS shapes to fail explicitly.
    let (mut client, _input, mut out, _, _) = live_channel(8192).await;
    let mut pdu = encode_rts_ping().expect("ping");
    pdu[16] = 0x40;
    out.write_all(&pdu).await.expect("unsupported RTS flags");
    let error = client.receive_fragment().await.expect_err("reject unsupported RTS");
    assert!(matches!(error.kind(), GwErrorKind::UnsupportedFeature));
}

fn established_rpc_ntlm_contexts() -> (RpcNtlmAuth, Ntlm) {
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
    (client, server)
}

#[test]
fn rpc_ntlm_context_signs_and_verifies_padded_fragments_in_sequence() {
    // oracle: spec MS-RPCE §3.3.1.5.2.2 protects the padded stub, plus headers only when negotiated.
    // oracle: spec MS-NLMP §3.4.4 requires ordered sequence numbers starting at zero.
    for header_signing in [false, true] {
        let (mut client, mut server) = established_rpc_ntlm_contexts();
        let authentication = ntlm_authentication();
        for (sequence, length) in (0..=33)
            .chain([63, 64, 65, 255, 256, 257, 4095, 4096, 4097])
            .enumerate()
        {
            let sequence = u32::try_from(sequence).expect("sequence number");
            let body = (0..length)
                .map(|index| u8::try_from(index % 251).expect("body byte"))
                .collect::<Vec<_>>();
            let mut request = rpc_fragment(PTYPE_REQUEST, authentication, &body);
            client
                .sign_fragment(&mut request, authentication, sequence, header_signing)
                .expect("sign request");
            server_verify_fragment(&mut server, &mut request, sequence, header_signing, 24);

            let mut response = rpc_fragment(PTYPE_RESPONSE, authentication, &body);
            server_sign_fragment(&mut server, &mut response, sequence, header_signing, 24);
            client
                .verify_fragment(&mut response, authentication, sequence, header_signing)
                .unwrap_or_else(|error| {
                    panic!("verify length {length}, sequence {sequence}, header signing {header_signing}: {error}")
                });
        }
    }
}

#[test]
fn rpc_ntlm_tampered_stub_poisons_established_context() {
    // oracle: spec MS-RPCE §3.3.1.5.2.2 requires body integrity in both header-signing modes.
    // oracle: regression #2202 requires a failed verifier to poison the association.
    for header_signing in [false, true] {
        let (mut client, mut server) = established_rpc_ntlm_contexts();
        let authentication = ntlm_authentication();
        let mut response = rpc_fragment(PTYPE_RESPONSE, authentication, b"response");
        server_sign_fragment(&mut server, &mut response, 0, header_signing, 24);
        response[24] ^= 1;
        assert!(
            client
                .verify_fragment(&mut response, authentication, 0, header_signing)
                .is_err()
        );
        assert_failed_context_refuses_fragments(&mut client, &mut server, authentication);
    }
}

#[test]
fn rpc_ntlm_request_object_uuid_precedes_the_signed_stub() {
    // oracle: spec C706 §12.6.4, "The request PDU", places the optional object UUID at bytes 24..40.
    // oracle: spec MS-RPCE §3.3.1.5.2.2 excludes the object UUID unless header signing is negotiated.
    for header_signing in [false, true] {
        let (mut client, mut server) = established_rpc_ntlm_contexts();
        let authentication = ntlm_authentication();
        for length in 0..=17 {
            let mut request = rpc_object_request(authentication, &vec![0x51; length]);
            let sequence = u32::try_from(length).expect("sequence");
            client
                .sign_fragment(&mut request, authentication, sequence, header_signing)
                .expect("sign request with object");
            server_verify_fragment(&mut server, &mut request, sequence, header_signing, 40);
        }
    }
}

#[test]
fn rpc_ntlm_fault_header_precedes_the_verified_stub() {
    // oracle: spec C706 §12.6.4, "The fault PDU", places fault stub data at byte 32.
    // oracle: spec MS-RPCE §3.3.1.5.2.2 selects header and padded body coverage.
    for header_signing in [false, true] {
        let (mut client, mut server) = established_rpc_ntlm_contexts();
        let authentication = ntlm_authentication();
        for length in 0..=17 {
            let mut fault = rpc_fragment(PTYPE_FAULT, authentication, &vec![0x63; length]);
            let sequence = u32::try_from(length).expect("sequence");
            server_sign_fragment(&mut server, &mut fault, sequence, header_signing, 32);
            client
                .verify_fragment(&mut fault, authentication, sequence, header_signing)
                .expect("verify fault");
        }
    }
}

#[test]
fn rpc_ntlm_without_header_signing_does_not_protect_pdu_specific_headers() {
    // oracle: spec MS-RPCE §3.3.1.5.2.2 requires no integrity protection for headers in this mode.
    let (mut client, mut server) = established_rpc_ntlm_contexts();
    let authentication = ntlm_authentication();
    let mut request = rpc_fragment(PTYPE_REQUEST, authentication, b"request");
    client
        .sign_fragment(&mut request, authentication, 0, false)
        .expect("sign stub only");
    request[22] ^= 1; // opnum
    server_verify_fragment(&mut server, &mut request, 0, false, 24);

    let mut response = rpc_fragment(PTYPE_RESPONSE, authentication, b"response");
    server_sign_fragment(&mut server, &mut response, 0, false, 24);
    response[22] ^= 1; // cancel_count
    client
        .verify_fragment(&mut response, authentication, 0, false)
        .expect("unprotected response header");
}

#[test]
fn rpc_ntlm_header_signing_rejects_pdu_specific_header_tampering() {
    // oracle: spec MS-RPCE §3.3.1.5.2.2 protects the entire PDU header when negotiated.
    let (mut client, mut server) = established_rpc_ntlm_contexts();
    let authentication = ntlm_authentication();
    let mut response = rpc_fragment(PTYPE_RESPONSE, authentication, b"response");
    server_sign_fragment(&mut server, &mut response, 0, true, 24);
    response[22] ^= 1; // cancel_count
    assert!(client.verify_fragment(&mut response, authentication, 0, true).is_err());
    assert_failed_context_refuses_fragments(&mut client, &mut server, authentication);
}

#[test]
fn rpc_ntlm_header_signing_rejects_security_trailer_tampering() {
    // oracle: spec MS-RPCE §3.3.1.5.2.2 protects sec_trailer when header signing is negotiated.
    let (mut client, mut server) = established_rpc_ntlm_contexts();
    let authentication = ntlm_authentication();
    let mut response = rpc_fragment(PTYPE_RESPONSE, authentication, b"response");
    server_sign_fragment(&mut server, &mut response, 0, true, 24);
    let security_trailer_offset = response.len() - 24;
    response[security_trailer_offset + 3] ^= 1; // auth_reserved
    assert!(client.verify_fragment(&mut response, authentication, 0, true).is_err());
    assert_failed_context_refuses_fragments(&mut client, &mut server, authentication);
}

#[test]
fn rpc_ntlm_signing_unsupported_pdu_type_poisons_established_context() {
    // oracle: regression #2202 rejects unknown NTLM protection layouts rather than assuming a stub offset.
    let (mut client, mut server) = established_rpc_ntlm_contexts();
    let authentication = ntlm_authentication();
    let mut request = rpc_fragment(PTYPE_REQUEST, authentication, b"request");
    request[2] = 11; // bind
    let error = client
        .sign_fragment(&mut request, authentication, 0, false)
        .expect_err("unsupported PDU");
    assert!(matches!(error.kind(), GwErrorKind::Decode));
    assert_failed_context_refuses_fragments(&mut client, &mut server, authentication);
}

#[test]
fn rpc_ntlm_verifying_response_with_request_object_flag_poisons_established_context() {
    // oracle: spec C706 §12.6.4 permits the optional object UUID only on request PDUs.
    // oracle: regression #2202 rejects unsupported NTLM protection layouts.
    let (mut client, mut server) = established_rpc_ntlm_contexts();
    let authentication = ntlm_authentication();
    let mut response = rpc_fragment(PTYPE_RESPONSE, authentication, b"response");
    response[3] |= 0x80; // PFC_OBJECT_UUID
    let error = client
        .verify_fragment(&mut response, authentication, 0, true)
        .expect_err("invalid object flag");
    assert!(matches!(error.kind(), GwErrorKind::Decode));
    assert_failed_context_refuses_fragments(&mut client, &mut server, authentication);
}

#[test]
fn rpc_ntlm_signing_rejects_padding_that_overlaps_pdu_specific_headers() {
    // oracle: spec C706 §12.6.4 and MS-RPCE §2.2.2.11 put padding after, not within, the PDU header.
    let authentication = ntlm_authentication();
    for header_signing in [false, true] {
        for (name, mut fragment) in [
            ("request", rpc_fragment(PTYPE_REQUEST, authentication, b"")),
            ("object request", rpc_object_request(authentication, b"")),
            ("response", rpc_fragment(PTYPE_RESPONSE, authentication, b"")),
            ("fault", rpc_fragment(PTYPE_FAULT, authentication, b"")),
        ] {
            let (mut client, mut server) = established_rpc_ntlm_contexts();
            let trailer = fragment.len() - 24;
            fragment[trailer + 2] += 1;
            assert!(
                client
                    .sign_fragment(&mut fragment, authentication, 0, header_signing)
                    .is_err(),
                "{name} header must not be consumed as padding"
            );
            assert_failed_context_refuses_fragments(&mut client, &mut server, authentication);
        }
    }
}

#[test]
fn rpc_ntlm_verifying_truncated_object_header_poisons_established_context() {
    // oracle: spec C706 §12.6.4, "The request PDU", requires 40 header bytes with PFC_OBJECT_UUID.
    let (mut client, mut server) = established_rpc_ntlm_contexts();
    let authentication = ntlm_authentication();
    let mut request = rpc_fragment(PTYPE_REQUEST, authentication, b"");
    request[3] |= 0x80;
    assert!(client.verify_fragment(&mut request, authentication, 0, false).is_err());
    assert_failed_context_refuses_fragments(&mut client, &mut server, authentication);
}

#[test]
fn rpc_ntlm_signing_malformed_header_poisons_established_context() {
    // oracle: regression #2202 requires fail-closed fragment validation.
    let (mut client, mut server) = established_rpc_ntlm_contexts();
    let authentication = ntlm_authentication();
    let mut request = rpc_fragment(PTYPE_REQUEST, authentication, b"request");
    request[0] = 4;
    assert!(client.sign_fragment(&mut request, authentication, 0, true).is_err());
    assert_failed_context_refuses_fragments(&mut client, &mut server, authentication);
}

#[test]
fn rpc_ntlm_verifying_malformed_trailer_poisons_established_context() {
    // oracle: regression #2202 requires fail-closed fragment validation.
    let (mut client, mut server) = established_rpc_ntlm_contexts();
    let authentication = ntlm_authentication();
    let mut response = rpc_fragment(PTYPE_RESPONSE, authentication, b"response");
    let trailer_offset = response.len() - usize::from(RPC_NTLM_VERIFIER_LENGTH) - RPC_SECURITY_TRAILER_SIZE;
    response[trailer_offset + 2] = u8::MAX;
    assert!(client.verify_fragment(&mut response, authentication, 0, true).is_err());
    assert_failed_context_refuses_fragments(&mut client, &mut server, authentication);
}

#[test]
fn rpc_ntlm_signing_mismatched_trailer_poisons_established_context() {
    // oracle: regression #2202 requires fail-closed fragment validation.
    let (mut client, mut server) = established_rpc_ntlm_contexts();
    let authentication = ntlm_authentication();
    let mut request = rpc_fragment(PTYPE_REQUEST, authentication, b"request");
    let trailer_offset = request.len() - usize::from(RPC_NTLM_VERIFIER_LENGTH) - RPC_SECURITY_TRAILER_SIZE;
    request[trailer_offset + 4..trailer_offset + 8].copy_from_slice(&2u32.to_le_bytes());
    assert!(client.sign_fragment(&mut request, authentication, 0, true).is_err());
    assert_failed_context_refuses_fragments(&mut client, &mut server, authentication);
}

#[test]
fn rpc_ntlm_verifying_mismatched_trailer_poisons_established_context() {
    // oracle: regression #2202 requires fail-closed fragment validation.
    let (mut client, mut server) = established_rpc_ntlm_contexts();
    let authentication = ntlm_authentication();
    let mut response = rpc_fragment(PTYPE_RESPONSE, authentication, b"response");
    let trailer_offset = response.len() - usize::from(RPC_NTLM_VERIFIER_LENGTH) - RPC_SECURITY_TRAILER_SIZE;
    response[trailer_offset + 4..trailer_offset + 8].copy_from_slice(&2u32.to_le_bytes());
    assert!(client.verify_fragment(&mut response, authentication, 0, true).is_err());
    assert_failed_context_refuses_fragments(&mut client, &mut server, authentication);
}

#[test]
fn rpc_ntlm_signing_invalid_authentication_poisons_established_context() {
    // oracle: regression #2202 requires fail-closed fragment validation.
    let (mut client, mut server) = established_rpc_ntlm_contexts();
    let authentication = ntlm_authentication();
    let mut request = rpc_fragment(PTYPE_REQUEST, authentication, b"request");
    let invalid_authentication = RpcAuthenticationInfo {
        verifier_length: 15,
        ..authentication
    };
    assert!(
        client
            .sign_fragment(&mut request, invalid_authentication, 0, true)
            .is_err()
    );
    assert_failed_context_refuses_fragments(&mut client, &mut server, authentication);
}

#[test]
fn rpc_ntlm_verifying_invalid_authentication_poisons_established_context() {
    // oracle: regression #2202 requires fail-closed fragment validation.
    let (mut client, mut server) = established_rpc_ntlm_contexts();
    let authentication = ntlm_authentication();
    let mut response = rpc_fragment(PTYPE_RESPONSE, authentication, b"response");
    let invalid_authentication = RpcAuthenticationInfo {
        auth_level: 4,
        ..authentication
    };
    assert!(
        client
            .verify_fragment(&mut response, invalid_authentication, 0, true)
            .is_err()
    );
    assert_failed_context_refuses_fragments(&mut client, &mut server, authentication);
}

fn assert_failed_context_refuses_fragments(
    client: &mut RpcNtlmAuth,
    server: &mut Ntlm,
    authentication: RpcAuthenticationInfo,
) {
    assert!(!client.is_complete(), "invalid fragment must poison the NTLM context");
    let mut request = rpc_fragment(PTYPE_REQUEST, authentication, b"valid request");
    assert!(client.sign_fragment(&mut request, authentication, 0, true).is_err());
    let mut response = rpc_fragment(PTYPE_RESPONSE, authentication, b"valid response");
    server_sign_fragment(server, &mut response, 0, true, 24);
    assert!(
        client.verify_fragment(&mut response, authentication, 0, true).is_err(),
        "a failed NTLM context must reject even a correctly signed response"
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

#[cfg(any(feature = "native-tls", feature = "rustls"))]
#[tokio::test]
async fn live_rpch_strict_tls_rejects_self_signed_gateway() {
    let (listener, acceptor) = tls_listener().await;
    let target = gateway_target(listener.local_addr().expect("address").port());
    let server = async {
        let (stream, _) = listener.accept().await.expect("accept TLS");
        let _ = acceptor.accept(stream).await;
    };
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::try_join!(RpchClient::connect(&target, "backend:3388"), async {
            server.await;
            Ok(())
        })
    })
    .await
    .expect("bounded strict TLS rejection");
    assert!(matches!(result, Err(error) if error.to_string().contains("tls connect")));
}

#[cfg(any(feature = "native-tls", feature = "rustls"))]
async fn live_channel(peer_window: u32) -> (RpchClient, ServerStream, ServerStream, RtsCookie, u32) {
    let (listener, acceptor) = tls_listener().await;
    let port = listener.local_addr().expect("address").port();
    let target = gateway_target(port);
    let server = async {
        let (input, _) = listener.accept().await.expect("accept IN");
        let mut input = acceptor.accept(input).await.expect("IN TLS");
        let (out, _) = listener.accept().await.expect("accept OUT");
        let mut out = acceptor.accept(out).await.expect("OUT TLS");
        let head = read_head(&mut input).await;
        // oracle: spec MS-RPCH §2.1.2.1.1 and §2.2.2 define the IN method, Host, and RPC proxy URL.
        assert!(head.starts_with("RPC_IN_DATA /rpc/rpcproxy.dll?backend:3388 HTTP/1.1\r\n"));
        assert!(head.contains(&format!("Host: 127.0.0.1:{port}\r\n")));
        // oracle: regression #2202 authenticates the IN channel with an empty probe before streaming its body.
        assert!(head.contains("Content-Length: 0\r\n"));
        assert!(!head.contains("Expect:"));
        assert!(!head.contains("Authorization:"));
        input
            .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 6\r\nWWW-Authenticate: Basic\r\n\r\ndenied")
            .await
            .expect("IN challenge");
        let head = read_head(&mut input).await;
        // oracle: regression #2202 retries the Basic challenge with the fixture's user:pass credentials.
        assert!(head.contains("Authorization: Basic dXNlcjpwYXNz\r\n"));
        // oracle: spec MS-RPCH §2.1.2.1.1 permits the fixture's 128-KiB IN channel lifetime.
        assert!(head.contains("Content-Length: 131072\r\n"));
        // oracle: regression #2202 gates the IN streaming body on HTTP 100 Continue.
        assert!(head.contains("Expect: 100-continue\r\n"));
        input
            .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
            .await
            .expect("IN accepted");
        let mut receive_window = 0;
        for authenticated in [false, true] {
            let head = read_head(&mut out).await;
            // oracle: spec MS-RPCH §2.1.2.1.2 and §2.2.2 define the OUT method, URL, and 76-byte body.
            assert!(head.starts_with("RPC_OUT_DATA /rpc/rpcproxy.dll?backend:3388 HTTP/1.1\r\n"));
            assert!(head.contains("Content-Length: 76\r\n"));
            // oracle: regression #2202 retries the OUT challenge with the fixture's user:pass credentials.
            assert_eq!(head.contains("Authorization: Basic dXNlcjpwYXNz\r\n"), authenticated);
            let a1 = read_pdu(&mut out).await;
            assert_eq!(a1.len(), 76);
            assert_eq!(a1[2], PTYPE_RTS);
            // oracle: spec MS-RPCH §2.2.4.2 puts CONN/A1's ReceiveWindowSize value at bytes 72..76.
            receive_window = u32::from_le_bytes(a1[72..76].try_into().expect("client receive window"));
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
        (input, out, cookie, receive_window)
    };
    let connect = RpchClient::connect_with_certificate_validation(
        &target,
        "backend:3388",
        CertificateValidation::DangerouslyAcceptInvalidCertificate,
        None,
    );
    let (client, (input, out, cookie, receive_window)) = tokio::time::timeout(
        Duration::from_secs(10),
        Box::pin(async move { tokio::try_join!(connect, async { Ok(server.await) }) }),
    )
    .await
    .expect("bounded loopback channel establishment")
    .expect("live channel");
    (client, input, out, cookie, receive_window)
}

#[cfg(any(feature = "native-tls", feature = "rustls"))]
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

#[cfg(any(feature = "native-tls", feature = "rustls"))]
fn authenticated_fragment(ptype: u8, body_length: usize) -> Vec<u8> {
    let mut fragment = rpc_fragment(
        ptype,
        RpcAuthenticationInfo {
            auth_type: 10,
            auth_level: 5,
            auth_context_id: 0,
            verifier_length: 16,
        },
        &vec![0x42; body_length],
    );
    let verifier_offset = fragment.len() - 16;
    fragment[verifier_offset..].copy_from_slice(&[0xa5; 16]);
    fragment
}

fn rpc_fragment(ptype: u8, authentication: RpcAuthenticationInfo, body: &[u8]) -> Vec<u8> {
    // oracle: spec C706 §12.6.4, "The request PDU", "The response PDU", and "The fault PDU".
    assert!(matches!(ptype, PTYPE_REQUEST | PTYPE_RESPONSE | PTYPE_FAULT));
    let mut pdu_body = u32::try_from(body.len())
        .expect("allocation hint")
        .to_le_bytes()
        .to_vec();
    pdu_body.extend_from_slice(&0u16.to_le_bytes()); // p_cont_id
    pdu_body.extend_from_slice(&if ptype == PTYPE_REQUEST {
        9u16.to_le_bytes() // opnum
    } else {
        [0, 0] // cancel_count, reserved
    });
    if ptype == PTYPE_FAULT {
        pdu_body.extend_from_slice(&0u32.to_le_bytes()); // status
        pdu_body.extend_from_slice(&[0; 4]); // reserved2
    }
    pdu_body.extend_from_slice(body);
    prepare_rpc_authenticated_pdu(ptype, 3, 7, &pdu_body, authentication)
        .expect("prepare integrity PDU")
        .finish(&[0; 16])
        .expect("reserve integrity verifier")
}

fn rpc_object_request(authentication: RpcAuthenticationInfo, body: &[u8]) -> Vec<u8> {
    // oracle: spec C706 §12.6.4, "The request PDU", puts the UUID between opnum and the stub.
    let mut pdu_body = u32::try_from(body.len())
        .expect("allocation hint")
        .to_le_bytes()
        .to_vec();
    pdu_body.extend_from_slice(&[0, 0, 9, 0]); // p_cont_id, opnum
    pdu_body.extend_from_slice(&[0x42; 16]); // object
    pdu_body.extend_from_slice(body);
    prepare_rpc_authenticated_pdu(PTYPE_REQUEST, 0x83, 7, &pdu_body, authentication)
        .expect("prepare object request")
        .finish(&[0; 16])
        .expect("reserve integrity verifier")
}

fn ntlm_authentication() -> RpcAuthenticationInfo {
    RpcAuthenticationInfo {
        auth_type: RPC_AUTH_TYPE_WINNT,
        auth_level: RPC_AUTH_LEVEL_PACKET_INTEGRITY,
        auth_context_id: 1,
        verifier_length: RPC_NTLM_VERIFIER_LENGTH,
    }
}

fn server_sign_fragment(
    server: &mut Ntlm,
    fragment: &mut [u8],
    sequence_number: u32,
    header_signing: bool,
    body_offset: usize,
) {
    let mut buffers = server_signature_buffers(fragment, header_signing, body_offset);
    server
        .make_signature(0, &mut buffers, sequence_number)
        .expect("server signature");
}

fn server_verify_fragment(
    server: &mut Ntlm,
    fragment: &mut [u8],
    sequence_number: u32,
    header_signing: bool,
    body_offset: usize,
) {
    let mut buffers = server_signature_buffers(fragment, header_signing, body_offset);
    server
        .verify_signature(&mut buffers, sequence_number)
        .expect("verify client signature");
}

fn server_signature_buffers(
    fragment: &mut [u8],
    header_signing: bool,
    body_offset: usize,
) -> [SecurityBufferRef<'_>; 2] {
    // oracle: spec C706 §12.6.4 defines stub offsets; callers supply them independently of production parsing.
    // oracle: spec MS-RPCE §2.2.2.11 and MS-NLMP §2.2.2.9 define the 8-byte trailer and 16-byte verifier.
    assert_eq!(fragment[10..12], 16u16.to_le_bytes());
    assert_eq!(
        usize::from(u16::from_le_bytes([fragment[8], fragment[9]])),
        fragment.len()
    );
    let verifier_offset = fragment.len() - 16;
    let data_start = if header_signing { 0 } else { body_offset };
    let data_end = if header_signing {
        verifier_offset
    } else {
        verifier_offset - 8
    };
    let (data, verifier) = fragment.split_at_mut(verifier_offset);
    [
        SecurityBufferRef::token_buf(verifier),
        SecurityBufferRef::data_buf(&mut data[data_start..data_end]),
    ]
}

#[cfg(any(feature = "native-tls", feature = "rustls"))]
async fn read_head<S: AsyncRead + Unpin>(stream: &mut S) -> String {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        assert!(bytes.len() < 16384, "bounded request headers");
        bytes.push(stream.read_u8().await.expect("request header byte"));
    }
    String::from_utf8(bytes).expect("request headers")
}

#[cfg(any(feature = "native-tls", feature = "rustls"))]
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

#[cfg(any(feature = "native-tls", feature = "rustls"))]
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
        gw_endpoint: format!("127.0.0.1:{port}"),
        gw_user: "user".to_owned(),
        gw_pass: "pass".to_owned(),
        smart_card: None,
        server: "rdp-server.test".to_owned(),
    }
}
