#![expect(
    unused_crate_dependencies,
    reason = "integration tests link the library crate and do not use its direct dependencies"
)]

use ironrdp_capture_replay::{Plaintext, ReplayError, extract_tunneled_rdp, is_gateway_tunnel};

const DATA_PACKET_HEADER: usize = 8 /* packet header */ + 2 /* data length */;

fn request_head() -> Vec<u8> {
    b"RDG_OUT_DATA /remoteDesktopGateway/ HTTP/1.1\r\nConnection: Upgrade\r\n\r\n".to_vec()
}

fn response_head(status: &str, content_length: usize) -> Vec<u8> {
    format!("HTTP/1.1 {status}\r\nContent-Length: {content_length}\r\n\r\n").into_bytes()
}

/// Build an MS-TSGU packet wrapping `data` as an `HTTP_DATA_PACKET` payload.
fn tsgu_data_packet(data: &[u8]) -> Vec<u8> {
    let length = u32::try_from(DATA_PACKET_HEADER + data.len()).unwrap();
    let mut packet = Vec::new();
    packet.extend(0x0Au16.to_le_bytes());
    packet.extend(0u16.to_le_bytes());
    packet.extend(length.to_le_bytes());
    packet.extend(u16::try_from(data.len()).unwrap().to_le_bytes());
    packet.extend(data);
    packet
}

/// Build a non-data MS-TSGU packet (e.g. tunnel channel control).
fn tsgu_control_packet(packet_type: u16, body: &[u8]) -> Vec<u8> {
    let length = u32::try_from(8 + body.len()).unwrap();
    let mut packet = Vec::new();
    packet.extend(packet_type.to_le_bytes());
    packet.extend(0u16.to_le_bytes());
    packet.extend(length.to_le_bytes());
    packet.extend(body);
    packet
}

/// Encode one WebSocket frame, masking client frames per RFC 6455.
fn websocket_frame(fin: bool, opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut frame = vec![(u8::from(fin) << 7) | opcode];
    let mask_bit = u8::from(mask.is_some()) << 7;
    if payload.len() < 126 {
        frame.push(mask_bit | u8::try_from(payload.len()).unwrap());
    } else if u16::try_from(payload.len()).is_ok() {
        frame.push(mask_bit | 126);
        frame.extend(u16::try_from(payload.len()).unwrap().to_be_bytes());
    } else {
        frame.push(mask_bit | 127);
        frame.extend(u64::try_from(payload.len()).unwrap().to_be_bytes());
    }
    match mask {
        Some(mask) => {
            frame.extend(mask);
            frame.extend(payload.iter().enumerate().map(|(index, byte)| byte ^ mask[index % 4]));
        }
        None => frame.extend(payload),
    }
    frame
}

fn client_frame(payload: &[u8]) -> Vec<u8> {
    websocket_frame(true, 0x2, payload, Some([1, 2, 3, 4]))
}

fn server_frame(payload: &[u8]) -> Vec<u8> {
    websocket_frame(true, 0x2, payload, None)
}

fn tunneled_capture(client_body: Vec<u8>, server_body: Vec<u8>) -> Plaintext {
    let mut client = request_head();
    client.extend(client_body);
    let mut server = response_head("101 Switching Protocols", 0);
    server.extend(server_body);
    Plaintext {
        client: vec![(1, client)],
        server: vec![(2, server)],
    }
}

#[test]
fn detects_gateway_upgrade() {
    let plaintext = tunneled_capture(Vec::new(), Vec::new());
    assert!(is_gateway_tunnel(&plaintext));

    let direct = Plaintext {
        client: vec![(1, vec![3, 0, 0, 11, 6, 0xe0, 0, 0, 0, 0, 0])],
        server: vec![(2, vec![3, 0, 0, 11, 6, 0xd0, 0, 0, 0, 0, 0])],
    };
    assert!(!is_gateway_tunnel(&direct));
}

#[test]
fn extracts_data_packets_from_masked_client_frames() {
    let rdp = [0x03, 0x00, 0x00, 0x13, 0x0e, 0xe0];
    let plaintext = tunneled_capture(
        client_frame(&tsgu_data_packet(&rdp)),
        server_frame(&tsgu_data_packet(&rdp)),
    );

    let inner = extract_tunneled_rdp(&plaintext).unwrap();

    assert_eq!(inner.client, vec![(1, rdp.to_vec())]);
    assert_eq!(inner.server, vec![(2, rdp.to_vec())]);
}

#[test]
fn skips_interim_authentication_round_trips() {
    let rdp = [0x03, 0x00, 0x00, 0x13, 0x0e, 0xe0];
    let mut client = request_head();
    client.extend(request_head());
    client.extend(client_frame(&tsgu_data_packet(&rdp)));
    let mut server = response_head("401 Unauthorized", 24);
    server.extend([0xAA; 24]);
    server.extend(response_head("101 Switching Protocols", 0));
    server.extend(server_frame(&tsgu_data_packet(&rdp)));
    let plaintext = Plaintext {
        client: vec![(1, client)],
        server: vec![(2, server)],
    };

    let inner = extract_tunneled_rdp(&plaintext).unwrap();

    assert_eq!(inner.client, vec![(1, rdp.to_vec())]);
    assert_eq!(inner.server, vec![(2, rdp.to_vec())]);
}

#[test]
fn reassembles_packets_split_across_frames() {
    let rdp: Vec<u8> = (0..200u16).map(|value| u8::try_from(value % 256).unwrap()).collect();
    let packet = tsgu_data_packet(&rdp);
    let split = packet.len() / 2;
    let mut body = websocket_frame(false, 0x2, &packet[..split], Some([9, 9, 9, 9]));
    body.extend(websocket_frame(true, 0x0, &packet[split..], Some([8, 8, 8, 8])));
    let plaintext = tunneled_capture(body, server_frame(&tsgu_data_packet(&rdp)));

    let inner = extract_tunneled_rdp(&plaintext).unwrap();

    assert_eq!(inner.client, vec![(1, rdp.clone())]);
    assert_eq!(inner.server, vec![(2, rdp)]);
}

#[test]
fn merges_packets_coalesced_in_one_frame() {
    let first = [1, 2, 3];
    let second = [4, 5, 6, 7];
    let mut payload = tsgu_control_packet(0x1, &[0; 12]);
    payload.extend(tsgu_data_packet(&first));
    payload.extend(tsgu_data_packet(&second));
    let plaintext = tunneled_capture(client_frame(&payload), server_frame(&tsgu_data_packet(&first)));

    let inner = extract_tunneled_rdp(&plaintext).unwrap();

    assert_eq!(inner.client, vec![(1, first.to_vec()), (1, second.to_vec())]);
    assert_eq!(inner.server, vec![(2, first.to_vec())]);
}

#[test]
fn skips_ping_frames_and_stops_at_close() {
    let rdp = [7, 7, 7];
    let mut body = websocket_frame(true, 0x9, b"ping", Some([0, 0, 0, 0]));
    body.extend(client_frame(&tsgu_data_packet(&rdp)));
    body.extend(websocket_frame(true, 0x8, &[], Some([0, 0, 0, 0])));
    body.extend(client_frame(&tsgu_data_packet(&[9, 9, 9])));
    let plaintext = tunneled_capture(body, server_frame(&tsgu_data_packet(&rdp)));

    let inner = extract_tunneled_rdp(&plaintext).unwrap();

    assert_eq!(inner.client, vec![(1, rdp.to_vec())]);
}

#[test]
fn rejects_tunneled_capture_without_data_packets() {
    let plaintext = tunneled_capture(client_frame(&tsgu_control_packet(0x1, &[0; 12])), server_frame(&[]));

    assert!(matches!(
        extract_tunneled_rdp(&plaintext),
        Err(ReplayError::MissingRdpState)
    ));
}

#[test]
fn rejects_truncated_data_packet() {
    let mut packet = tsgu_data_packet(&[1, 2, 3]);
    packet.truncate(packet.len() - 1);
    let plaintext = tunneled_capture(client_frame(&packet), server_frame(&tsgu_data_packet(&[1])));

    assert!(matches!(
        extract_tunneled_rdp(&plaintext),
        Err(ReplayError::GatewayFraming(_))
    ));
}

#[test]
fn rejects_trailing_truncated_websocket_bytes() {
    let rdp = [1, 2, 3];
    let mut body = client_frame(&tsgu_data_packet(&rdp));
    body.push(0x82);
    let plaintext = tunneled_capture(body, server_frame(&tsgu_data_packet(&rdp)));

    assert!(matches!(
        extract_tunneled_rdp(&plaintext),
        Err(ReplayError::GatewayFraming(_))
    ));
}

#[test]
fn rejects_an_unfinished_websocket_message() {
    let packet = tsgu_data_packet(&[1, 2, 3]);
    let body = websocket_frame(false, 0x2, &packet, Some([9, 9, 9, 9]));
    let plaintext = tunneled_capture(body, server_frame(&tsgu_data_packet(&[1])));

    assert!(matches!(
        extract_tunneled_rdp(&plaintext),
        Err(ReplayError::GatewayFraming(_))
    ));
}

#[test]
fn rejects_a_truncated_packet_after_a_valid_data_packet() {
    let rdp = [1, 2, 3];
    let mut payload = tsgu_data_packet(&rdp);
    payload.extend(&tsgu_data_packet(&[4, 5, 6])[..8]);
    let plaintext = tunneled_capture(client_frame(&payload), server_frame(&tsgu_data_packet(&rdp)));

    assert!(matches!(
        extract_tunneled_rdp(&plaintext),
        Err(ReplayError::GatewayFraming(_))
    ));
}
