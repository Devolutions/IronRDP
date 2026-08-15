use ironrdp_rdpemt::pdu::*;
use ironrdp_rdpemt::*;
fn test_config() -> TunnelConfig {
    TunnelConfig {
        request_id: 7,
        security_cookie: [
            0xe2, 0xf0, 0xd1, 0x08, 0x56, 0x7f, 0xb4, 0x3a, 0xdc, 0xf4, 0xb3, 0xdc, 0x16, 0x92, 0x1e, 0x3a,
        ],
    }
}

// ── Client happy path ──

#[test]
fn client_produces_create_request_on_construction() {
    let tunnel = RdpemtTunnel::client(test_config());
    assert!(!tunnel.is_established());
    assert!(!tunnel.is_failed());
    assert_eq!(tunnel.side(), Side::Client);
}

#[test]
fn client_poll_pdu_returns_create_request() {
    let mut tunnel = RdpemtTunnel::client(test_config());
    let pdu_bytes = tunnel.poll_pdu().expect("should have CreateRequest");

    // Verify it decodes as a CreateRequest
    let pdu: TunnelPdu = ironrdp_core::decode(&pdu_bytes).expect("decode");
    match pdu {
        TunnelPdu::CreateRequest(req) => {
            assert_eq!(req.request_id, 7);
            assert_eq!(req.security_cookie, test_config().security_cookie);
        }
        other => panic!("expected CreateRequest, got {other:?}"),
    }

    // No more PDUs
    assert!(tunnel.poll_pdu().is_none());
}

#[test]
fn client_handles_successful_create_response() {
    let mut tunnel = RdpemtTunnel::client(test_config());
    let _create_req = tunnel.poll_pdu().expect("CreateRequest");

    // Build a successful CreateResponse
    let response = TunnelCreateResponse {
        hr_response: TunnelCreateResponse::S_OK,
    };
    let response_bytes = ironrdp_core::encode_vec(&response).expect("encode");

    tunnel.handle_pdu(&response_bytes).expect("handle ok");
    assert!(tunnel.is_established());

    let event = tunnel.poll_event().expect("should have Established event");
    assert_eq!(event, TunnelEvent::Established);
    assert!(tunnel.poll_event().is_none());
}

#[test]
fn client_handles_rejected_create_response() {
    let mut tunnel = RdpemtTunnel::client(test_config());
    let _create_req = tunnel.poll_pdu().expect("CreateRequest");

    let response = TunnelCreateResponse {
        hr_response: 0x8000_0001,
    };
    let response_bytes = ironrdp_core::encode_vec(&response).expect("encode");

    tunnel.handle_pdu(&response_bytes).expect("handle ok");
    assert!(tunnel.is_failed());
    assert!(!tunnel.is_established());

    let event = tunnel.poll_event().expect("should have Failed event");
    assert_eq!(
        event,
        TunnelEvent::Failed {
            hr_response: 0x8000_0001
        }
    );
}

// ── Server happy path ──

#[test]
fn server_starts_in_created() {
    let tunnel = RdpemtTunnel::server(test_config());
    assert!(!tunnel.is_established());
    assert!(!tunnel.is_failed());
    assert_eq!(tunnel.side(), Side::Server);
    // No outgoing PDUs initially
}

#[test]
fn server_handles_matching_create_request() {
    let mut server = RdpemtTunnel::server(test_config());

    let request = TunnelCreateRequest {
        request_id: 7,
        security_cookie: test_config().security_cookie,
    };
    let request_bytes = ironrdp_core::encode_vec(&request).expect("encode");

    server.handle_pdu(&request_bytes).expect("handle ok");
    assert!(server.is_established());

    // Server should have enqueued a successful CreateResponse
    let response_bytes = server.poll_pdu().expect("should have CreateResponse");
    let pdu: TunnelPdu = ironrdp_core::decode(&response_bytes).expect("decode");
    match pdu {
        TunnelPdu::CreateResponse(resp) => {
            assert!(resp.is_success());
        }
        other => panic!("expected CreateResponse, got {other:?}"),
    }

    let event = server.poll_event().expect("should have Established event");
    assert_eq!(event, TunnelEvent::Established);
}

/// MS-RDPEMT §3.1.5.5 forbids sending data before CreateResponse is sent. A
/// caller reacting to `Established` and calling `send_data` before draining
/// `poll_pdu` must still see CreateResponse ahead of that data on the wire.
#[test]
fn established_event_does_not_let_send_data_race_ahead_of_create_response() {
    let mut server = RdpemtTunnel::server(test_config());
    let request = TunnelCreateRequest {
        request_id: 7,
        security_cookie: test_config().security_cookie,
    };
    let request_bytes = ironrdp_core::encode_vec(&request).expect("encode");

    server.handle_pdu(&request_bytes).expect("handle ok");

    // React to Established immediately, before draining poll_pdu, exactly
    // the ordering the caller is free to use.
    while let Some(event) = server.poll_event() {
        if event == TunnelEvent::Established {
            server
                .send_data(b"racing data")
                .expect("send_data succeeds once Established");
        }
    }

    let first: TunnelPdu = ironrdp_core::decode(&server.poll_pdu().expect("first pdu")).expect("decode");
    assert!(
        matches!(first, TunnelPdu::CreateResponse(_)),
        "CreateResponse must be the first PDU on the wire, got {first:?}"
    );

    let second: TunnelPdu = ironrdp_core::decode(&server.poll_pdu().expect("second pdu")).expect("decode");
    assert!(matches!(second, TunnelPdu::Data(_)));
}

#[test]
fn server_rejects_mismatching_request_id() {
    let mut server = RdpemtTunnel::server(test_config());

    let request = TunnelCreateRequest {
        request_id: 999, // wrong ID
        security_cookie: test_config().security_cookie,
    };
    let request_bytes = ironrdp_core::encode_vec(&request).expect("encode");

    server.handle_pdu(&request_bytes).expect("handle ok");
    assert!(server.is_failed());

    // Server sends a rejection response
    let response_bytes = server.poll_pdu().expect("should have CreateResponse");
    let pdu: TunnelPdu = ironrdp_core::decode(&response_bytes).expect("decode");
    match pdu {
        TunnelPdu::CreateResponse(resp) => {
            assert!(!resp.is_success());
        }
        other => panic!("expected CreateResponse, got {other:?}"),
    }

    let event = server.poll_event().expect("should have Failed event");
    match event {
        TunnelEvent::Failed { .. } => {}
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn server_rejects_mismatching_cookie() {
    let mut server = RdpemtTunnel::server(test_config());

    let request = TunnelCreateRequest {
        request_id: 7,
        security_cookie: [0xFF; 16], // wrong cookie
    };
    let request_bytes = ironrdp_core::encode_vec(&request).expect("encode");

    server.handle_pdu(&request_bytes).expect("handle ok");
    assert!(server.is_failed());
}

// ── Data transfer ──

#[test]
fn established_tunnel_sends_data() {
    let mut tunnel = RdpemtTunnel::client(test_config());
    let _create_req = tunnel.poll_pdu();

    let response = TunnelCreateResponse {
        hr_response: TunnelCreateResponse::S_OK,
    };
    let response_bytes = ironrdp_core::encode_vec(&response).expect("encode");
    tunnel.handle_pdu(&response_bytes).expect("handle ok");
    let _event = tunnel.poll_event();

    // Send data
    tunnel.send_data(b"hello world").expect("send ok");
    let data_pdu_bytes = tunnel.poll_pdu().expect("should have Data PDU");

    let pdu: TunnelPdu = ironrdp_core::decode(&data_pdu_bytes).expect("decode");
    match pdu {
        TunnelPdu::Data(data) => {
            assert_eq!(data.higher_layer_data, b"hello world");
        }
        other => panic!("expected Data, got {other:?}"),
    }
}

#[test]
fn established_tunnel_receives_data() {
    let mut tunnel = RdpemtTunnel::client(test_config());
    let _create_req = tunnel.poll_pdu();

    let response = TunnelCreateResponse {
        hr_response: TunnelCreateResponse::S_OK,
    };
    let response_bytes = ironrdp_core::encode_vec(&response).expect("encode");
    tunnel.handle_pdu(&response_bytes).expect("handle ok");
    let _event = tunnel.poll_event();

    // Simulate receiving a Data PDU
    let incoming = TunnelData {
        sub_headers: Vec::new(),
        higher_layer_data: b"from server".to_vec(),
    };
    let incoming_bytes = ironrdp_core::encode_vec(&incoming).expect("encode");

    tunnel.handle_pdu(&incoming_bytes).expect("handle ok");

    let event = tunnel.poll_event().expect("should have Data event");
    assert_eq!(
        event,
        TunnelEvent::Data {
            sub_headers: Vec::new(),
            data: b"from server".to_vec(),
        }
    );
}

/// A sub-header carried on a received Data PDU (e.g. an auto-detect
/// bandwidth-measurement result, MS-RDPBCGR Section 2.2.14) must reach the
/// caller through TunnelEvent::Data, not be discarded.
#[test]
fn established_tunnel_surfaces_sub_headers_on_receive() {
    let mut tunnel = RdpemtTunnel::client(test_config());
    let _create_req = tunnel.poll_pdu();

    let response = TunnelCreateResponse {
        hr_response: TunnelCreateResponse::S_OK,
    };
    let response_bytes = ironrdp_core::encode_vec(&response).expect("encode");
    tunnel.handle_pdu(&response_bytes).expect("handle ok");
    let _event = tunnel.poll_event();

    let sub_header = TunnelSubHeader {
        sub_header_type: SubHeaderType::AutoDetectResponse,
        data: vec![0xAA, 0xBB],
    };
    let incoming = TunnelData {
        sub_headers: vec![sub_header.clone()],
        higher_layer_data: b"payload".to_vec(),
    };
    let incoming_bytes = ironrdp_core::encode_vec(&incoming).expect("encode");

    tunnel.handle_pdu(&incoming_bytes).expect("handle ok");

    let event = tunnel.poll_event().expect("should have Data event");
    assert_eq!(
        event,
        TunnelEvent::Data {
            sub_headers: vec![sub_header],
            data: b"payload".to_vec(),
        }
    );
}

/// A sub-header attached via send_data_with_sub_headers must reach the wire
/// and round-trip through the peer's handle_pdu.
#[test]
fn send_data_with_sub_headers_round_trips() {
    let mut client = RdpemtTunnel::client(test_config());
    let create_req = client.poll_pdu().expect("CreateRequest");

    let mut server = RdpemtTunnel::server(test_config());
    server.handle_pdu(&create_req).expect("server handle CreateRequest");
    let create_resp = server.poll_pdu().expect("CreateResponse");
    let _ = server.poll_event();

    client.handle_pdu(&create_resp).expect("client handle CreateResponse");
    let _ = client.poll_event();

    let sub_header = TunnelSubHeader {
        sub_header_type: SubHeaderType::AutoDetectRequest,
        data: vec![0x01, 0x02, 0x03],
    };
    client
        .send_data_with_sub_headers(vec![sub_header.clone()], b"measurement")
        .expect("send ok");
    let data_bytes = client.poll_pdu().expect("Data PDU");

    server.handle_pdu(&data_bytes).expect("server handle Data");
    let event = server.poll_event().expect("should have Data event");
    assert_eq!(
        event,
        TunnelEvent::Data {
            sub_headers: vec![sub_header],
            data: b"measurement".to_vec(),
        }
    );
}

// ── State violation tests ──

#[test]
fn send_data_before_established_fails() {
    let mut tunnel = RdpemtTunnel::client(test_config());
    let _create_req = tunnel.poll_pdu();

    let result = tunnel.send_data(b"too early");
    assert!(result.is_err());
}

#[test]
fn handle_data_before_established_fails() {
    let mut tunnel = RdpemtTunnel::client(test_config());
    let _create_req = tunnel.poll_pdu();

    let data = TunnelData {
        sub_headers: Vec::new(),
        higher_layer_data: b"premature".to_vec(),
    };
    let data_bytes = ironrdp_core::encode_vec(&data).expect("encode");

    let result = tunnel.handle_pdu(&data_bytes);
    assert!(result.is_err());
}

#[test]
fn handle_create_request_on_established_client_fails() {
    let mut tunnel = RdpemtTunnel::client(test_config());
    let _create_req = tunnel.poll_pdu();

    let response = TunnelCreateResponse {
        hr_response: TunnelCreateResponse::S_OK,
    };
    let response_bytes = ironrdp_core::encode_vec(&response).expect("encode");
    tunnel.handle_pdu(&response_bytes).expect("handle ok");
    let _event = tunnel.poll_event();

    // Try to handle a CreateRequest on an established client: invalid
    let request = TunnelCreateRequest {
        request_id: 7,
        security_cookie: test_config().security_cookie,
    };
    let request_bytes = ironrdp_core::encode_vec(&request).expect("encode");

    let result = tunnel.handle_pdu(&request_bytes);
    assert!(result.is_err());
}

#[test]
fn send_data_on_failed_tunnel_fails() {
    let mut tunnel = RdpemtTunnel::client(test_config());
    let _create_req = tunnel.poll_pdu();

    let response = TunnelCreateResponse {
        hr_response: 0x8000_FFFF,
    };
    let response_bytes = ironrdp_core::encode_vec(&response).expect("encode");
    tunnel.handle_pdu(&response_bytes).expect("handle ok");

    let result = tunnel.send_data(b"dead tunnel");
    assert!(result.is_err());
}

// ── End-to-end: client + server ──

#[test]
fn end_to_end_handshake_and_data() {
    let config = test_config();

    // Create both sides
    let mut client = RdpemtTunnel::client(config.clone());
    let mut server = RdpemtTunnel::server(config);

    // Client produces CreateRequest
    let create_req_bytes = client.poll_pdu().expect("client CreateRequest");
    assert!(client.poll_pdu().is_none());

    // Server processes CreateRequest, produces CreateResponse
    server
        .handle_pdu(&create_req_bytes)
        .expect("server handle CreateRequest");
    assert!(server.is_established());
    let create_resp_bytes = server.poll_pdu().expect("server CreateResponse");
    assert!(server.poll_pdu().is_none());

    // Client processes CreateResponse
    client
        .handle_pdu(&create_resp_bytes)
        .expect("client handle CreateResponse");
    assert!(client.is_established());

    // Drain events
    assert_eq!(server.poll_event(), Some(TunnelEvent::Established));
    assert_eq!(client.poll_event(), Some(TunnelEvent::Established));

    // Client sends data to server
    client.send_data(b"request data").expect("client send");
    let data_to_server = client.poll_pdu().expect("client Data PDU");

    server.handle_pdu(&data_to_server).expect("server handle Data");
    assert_eq!(
        server.poll_event(),
        Some(TunnelEvent::Data {
            sub_headers: Vec::new(),
            data: b"request data".to_vec(),
        })
    );

    // Server sends data to client
    server.send_data(b"response data").expect("server send");
    let data_to_client = server.poll_pdu().expect("server Data PDU");

    client.handle_pdu(&data_to_client).expect("client handle Data");
    assert_eq!(
        client.poll_event(),
        Some(TunnelEvent::Data {
            sub_headers: Vec::new(),
            data: b"response data".to_vec(),
        })
    );
}
