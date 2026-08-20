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

/// HTTP method tokens that can start a WebSocket gateway request head.
const REQUEST_HEAD_PREFIXES: [&[u8]; 2] = [b"RDG_OUT_DATA ".as_slice(), b"RDG_IN_DATA ".as_slice()];

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
