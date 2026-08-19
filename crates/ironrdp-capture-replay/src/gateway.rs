//! MS-TSGU gateway tunnel extraction for captured TLS-decrypted HTTPS flows.
//!
//! Modern RD Gateway clients carry MS-TSGU packets in WebSocket binary frames
//! after an `RDG_OUT_DATA` upgrade. This module unwraps that framing and
//! recovers the tunneled RDP byte stream ([MS-TSGU] 2.2.10).

use crate::transport::flatten;
use crate::{PacketStream, Plaintext, ReplayError};

/// `HTTP_DATA_PACKET` payload prefix: 2-byte length after the packet header.
const DATA_PACKET_HEADER: usize = 8 /* packet header */ + 2 /* data length */;
/// Maximum accepted WebSocket frame payload, larger than any TLS record batch.
const MAX_WEBSOCKET_PAYLOAD: usize = 16 * 1024 * 1024;

/// Returns `true` when the decrypted TLS streams carry an RDG WebSocket upgrade.
pub fn is_gateway_tunnel(plaintext: &Plaintext) -> bool {
    flatten(&plaintext.client).starts_with(b"RDG_OUT_DATA /remoteDesktopGateway/")
}

/// Extract the tunneled RDP bytes from a decrypted RDG WebSocket session.
///
/// The returned streams contain raw RDP traffic (starting with the X.224
/// connection request) exactly as a direct TCP capture would carry it.
pub fn extract_tunneled_rdp(plaintext: &Plaintext) -> Result<Plaintext, ReplayError> {
    let client = websocket_payloads(&plaintext.client, true)?;
    let server = websocket_payloads(&plaintext.server, false)?;
    Ok(Plaintext {
        client: data_packets(&client)?,
        server: data_packets(&server)?,
    })
}

/// HTTP method tokens that can start a gateway request head.
const REQUEST_HEAD_PREFIXES: [&[u8]; 4] = [
    b"RDG_OUT_DATA ".as_slice(),
    b"RDG_IN_DATA ".as_slice(),
    b"RPC_OUT_DATA ".as_slice(),
    b"RPC_IN_DATA ".as_slice(),
];

/// Split one direction into WebSocket message payloads, skipping the HTTP heads.
///
/// Client-to-server frames are masked per RFC 6455; server frames are not.
/// Authentication can add request/response round trips (e.g. an NTLM 401
/// challenge) before the final WebSocket upgrade, so interim HTTP heads are
/// skipped until the bytes no longer look like HTTP.
fn websocket_payloads(stream: &PacketStream, masked: bool) -> Result<PacketStream, ReplayError> {
    let mut bytes = Vec::new();
    let mut packet_offsets = Vec::with_capacity(stream.len());
    for (packet, chunk) in stream {
        packet_offsets.push((bytes.len(), *packet));
        bytes.extend_from_slice(chunk);
    }

    let head_prefixes: &[&[u8]] = if masked {
        &REQUEST_HEAD_PREFIXES
    } else {
        &[b"HTTP/".as_slice()]
    };
    let mut head_end = 0;
    loop {
        let head_start = head_end;
        head_end += bytes[head_end..]
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|position| position + 4)
            .ok_or(ReplayError::UnsupportedTransport)?;
        // Interim responses such as NTLM 401 challenges carry an error body.
        if !masked {
            head_end += content_length(&bytes[head_start..head_end]);
        }
        // WebSocket frames start with a non-ASCII opcode byte, never a head.
        match bytes.get(head_end..) {
            Some(rest) if head_prefixes.iter().any(|prefix| rest.starts_with(prefix)) => {}
            _ => break,
        }
    }

    let mut frames = Vec::new();
    let mut offset = head_end;
    let mut message = Vec::new();
    let mut message_packet = 0usize;
    while offset < bytes.len() {
        let Some(frame) = parse_websocket_frame(&bytes[offset..], masked) else {
            break;
        };
        let packet_index = packet_offsets.partition_point(|(chunk_offset, _)| *chunk_offset <= offset) - 1;
        offset += frame.encoded_len;

        match frame.opcode {
            // Continuation or binary data.
            0x0 | 0x2 => {
                if message.is_empty() {
                    message_packet = packet_offsets[packet_index].1;
                }
                message.extend_from_slice(&frame.payload);
                if frame.fin {
                    frames.push((message_packet, core::mem::take(&mut message)));
                }
            }
            // Ping/pong carry no channel data.
            0x9 | 0xA => {}
            // Close ends the tunnel.
            0x8 => break,
            _ => return Err(ReplayError::UnsupportedTransport),
        }
    }

    Ok(frames)
}

/// Extract the `Content-Length` of an HTTP head, if present.
fn content_length(head: &[u8]) -> usize {
    let Ok(head) = core::str::from_utf8(head) else {
        return 0;
    };
    head.split("\r\n")
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0)
}

struct WebsocketFrame {
    encoded_len: usize,
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

fn parse_websocket_frame(bytes: &[u8], masked: bool) -> Option<WebsocketFrame> {
    let first = *bytes.first()?;
    let second = *bytes.get(1)?;
    let mut offset = 2;
    let length = match second & 0x7f {
        126 => {
            let length = u16::from_be_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?);
            offset += 2;
            usize::from(length)
        }
        127 => {
            let length = u64::from_be_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?);
            offset += 8;
            usize::try_from(length).ok()?
        }
        length => usize::from(length),
    };
    if length > MAX_WEBSOCKET_PAYLOAD {
        return None;
    }

    let mask = if second & 0x80 != 0 {
        let mask: [u8; 4] = bytes.get(offset..offset + 4)?.try_into().ok()?;
        offset += 4;
        Some(mask)
    } else {
        None
    };
    if masked != mask.is_some() {
        return None;
    }

    let mut payload = bytes.get(offset..offset + length)?.to_vec();
    if let Some(mask) = mask {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % 4];
        }
    }

    Some(WebsocketFrame {
        encoded_len: offset + length,
        fin: first & 0x80 != 0,
        opcode: first & 0x0f,
        payload,
    })
}

/// Keep the RDP payloads of `HTTP_DATA_PACKET` packets in tunnel order.
fn data_packets(frames: &PacketStream) -> Result<PacketStream, ReplayError> {
    let mut output = Vec::new();
    let mut pending = Vec::new();
    for (packet, frame) in frames {
        pending.extend_from_slice(frame);
        while let Some(header) = pending.get(..8) {
            let length = usize::try_from(u32::from_le_bytes(
                header[4..8].try_into().expect("header length slice"),
            ))
            .map_err(|_| ReplayError::GatewayFraming("packet length is too large".to_owned()))?;
            if !(8..=MAX_WEBSOCKET_PAYLOAD).contains(&length) {
                return Err(ReplayError::GatewayFraming("packet length is invalid".to_owned()));
            }
            if pending.len() < length {
                break;
            }
            let packet_bytes = pending[..length].to_vec();
            pending.drain(..length);

            let packet_type = u16::from_le_bytes(packet_bytes[..2].try_into().expect("packet type slice"));
            if packet_type == 0x0A {
                // HTTP_DATA_PACKET ([MS-TSGU] 2.2.10.10).
                let data = packet_bytes
                    .get(DATA_PACKET_HEADER..)
                    .ok_or_else(|| ReplayError::GatewayFraming("truncated data packet".to_owned()))?;
                let declared = usize::from(u16::from_le_bytes(
                    packet_bytes[8..10].try_into().expect("data length slice"),
                ));
                if declared != data.len() {
                    return Err(ReplayError::GatewayFraming(
                        "data packet length does not match its header".to_owned(),
                    ));
                }
                output.push((*packet, data.to_vec()));
            }
        }
    }

    if output.is_empty() {
        return Err(ReplayError::MissingRdpState);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_head() -> Vec<u8> {
        b"RDG_OUT_DATA /remoteDesktopGateway/ HTTP/1.1\r\nConnection: Upgrade\r\n\r\n".to_vec()
    }

    fn response_head(status: &str, content_length: usize) -> Vec<u8> {
        format!("HTTP/1.1 {status}\r\nContent-Length: {content_length}\r\n\r\n").into_bytes()
    }

    /// Build an MS-TSGU packet of the given type wrapping `data` as an
    /// `HTTP_DATA_PACKET` payload.
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

        assert!(extract_tunneled_rdp(&plaintext).is_err());
    }
}
