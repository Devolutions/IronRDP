//! MS-TSGU RPC-over-HTTP (RPCH) tunnel extraction for captured TLS-decrypted flows.
//!
//! The legacy RD Gateway transport carries the tunneled RDP session inside DCE/RPC
//! calls on two HTTP/1 connections: an IN channel (`RPC_IN_DATA`, client to server) and
//! an OUT channel (`RPC_OUT_DATA`, server to client). This module unwraps the DCE/RPC
//! and TsProxy framing and recovers the tunneled RDP byte stream ([MS-TSGU] 3.6).

use crate::transport::flatten;
use crate::{PacketStream, Plaintext, ReplayError};

/// DCE/RPC common header length.
const COMMON_HEADER: usize = 16;
/// DCE/RPC response PDUs add alloc-hint/context/cancel fields before the stub.
const RESPONSE_HEADER: usize = COMMON_HEADER + 8;
/// DCE/RPC request PDUs add alloc-hint/context/opnum fields before the stub.
const REQUEST_HEADER: usize = COMMON_HEADER + 8;
/// Largest accepted RPC fragment, comfortably above the negotiated fragment size.
const MAX_FRAGMENT: usize = 64 * 1024;

const PTYPE_REQUEST: u8 = 0;
const PTYPE_RESPONSE: u8 = 2;
const PFC_FIRST_FRAG: u8 = 0x01;
const PFC_LAST_FRAG: u8 = 0x02;

/// `TsProxySendToServer` operation number ([MS-TSGU] 3.6.5.1).
const SEND_TO_SERVER_OPNUM: u16 = 9;

/// Which RPCH channel a decrypted flow carries, keyed by its HTTP method.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RpchChannel {
    /// `RPC_IN_DATA`: client-to-server tunnel data (`TsProxySendToServer`).
    In,
    /// `RPC_OUT_DATA`: server-to-client tunnel data (the receive pipe).
    Out,
}

/// Classify a decrypted RPCH flow, or `None` when it is not an RPCH tunnel.
pub fn rpch_channel(plaintext: &Plaintext) -> Option<RpchChannel> {
    let client = flatten(&plaintext.client);
    if client.starts_with(b"RPC_IN_DATA ") {
        Some(RpchChannel::In)
    } else if client.starts_with(b"RPC_OUT_DATA ") {
        Some(RpchChannel::Out)
    } else {
        None
    }
}

/// Pair IN and OUT channel flows, or `None` when no RPCH tunnel is present.
///
/// A single IN or OUT HTTP flow is not a complete tunnel. Channel matching for
/// extraction tries every IN/OUT combination; this helper only reports whether
/// both channel types exist.
pub fn pair_rpch_channels(flows: &[Plaintext]) -> Result<Option<(&Plaintext, &Plaintext)>, ReplayError> {
    let input = flows.iter().find(|flow| rpch_channel(flow) == Some(RpchChannel::In));
    let output = flows.iter().find(|flow| rpch_channel(flow) == Some(RpchChannel::Out));
    match (input, output) {
        (Some(input), Some(output)) => Ok(Some((input, output))),
        (None, None) => Ok(None),
        _ => Err(ReplayError::GatewayFraming(
            "incomplete RPC-over-HTTP tunnel".to_owned(),
        )),
    }
}

/// Unwrap a paired RPCH tunnel from decrypted gateway-port flows.
///
/// Candidate IN and OUT channels are tried until one pair yields an inner RDP
/// stream. A single IN or OUT HTTP flow is not a complete tunnel.
pub fn extract_rpch_from_flows(flows: &[Plaintext]) -> Result<Option<Plaintext>, ReplayError> {
    let inputs: Vec<_> = flows
        .iter()
        .filter(|flow| rpch_channel(flow) == Some(RpchChannel::In))
        .collect();
    let outputs: Vec<_> = flows
        .iter()
        .filter(|flow| rpch_channel(flow) == Some(RpchChannel::Out))
        .collect();
    if inputs.is_empty() && outputs.is_empty() {
        return Ok(None);
    }
    if inputs.is_empty() || outputs.is_empty() {
        return Err(ReplayError::GatewayFraming(
            "incomplete RPC-over-HTTP tunnel".to_owned(),
        ));
    }
    let mut last_error = None;
    for input in inputs {
        for output in outputs.iter().copied() {
            match extract_rpch_tunneled_rdp(input, output) {
                Ok(plaintext) => return Ok(Some(plaintext)),
                Err(error) => last_error = Some(error),
            }
        }
    }
    Err(last_error.unwrap_or(ReplayError::MissingRdpState))
}

/// Extract the tunneled RDP bytes from a pair of decrypted RPCH channel flows.
///
/// `input` is the IN-channel flow (client requests) and `output` is the OUT-channel
/// flow (server responses). The returned streams contain raw RDP traffic (starting
/// with the X.224 connection request) exactly as a direct TCP capture would carry it.
pub fn extract_rpch_tunneled_rdp(input: &Plaintext, output: &Plaintext) -> Result<Plaintext, ReplayError> {
    let (client_flat, client_packets) = concat_stream(&input.client);
    let (server_flat, server_packets) = concat_stream(&output.server);
    let client_start = strip_http_heads(&client_flat, Direction::Client);
    let server_start = strip_http_heads(&server_flat, Direction::Server);

    let client = client_rdp_bytes(&client_flat[client_start..], &client_packets, client_start)?;
    let server = server_rdp_bytes(&server_flat[server_start..], &server_packets, server_start)?;
    if client.is_empty() || server.is_empty() {
        return Err(ReplayError::MissingRdpState);
    }
    Ok(Plaintext { client, server })
}

fn concat_stream(stream: &PacketStream) -> (Vec<u8>, Vec<(usize, usize)>) {
    let mut bytes = Vec::new();
    let mut packet_offsets = Vec::with_capacity(stream.len());
    for (packet, chunk) in stream {
        packet_offsets.push((bytes.len(), *packet));
        bytes.extend_from_slice(chunk);
    }
    (bytes, packet_offsets)
}

fn packet_at(packet_offsets: &[(usize, usize)], offset: usize) -> usize {
    let index = packet_offsets.partition_point(|(chunk_offset, _)| *chunk_offset <= offset);
    packet_offsets
        .get(index.saturating_sub(1))
        .map(|(_, packet)| *packet)
        .unwrap_or(0)
}

#[derive(Clone, Copy)]
enum Direction {
    Client,
    Server,
}

/// Strip every HTTP request/response head, returning the offset of the RPC body.
///
/// Authentication adds 401 round trips before the 200 that opens the streaming body;
/// each head ends at `\r\n\r\n`, and a server head with a small `Content-Length`
/// (an error page) is followed by that body before the next head.
fn strip_http_heads(bytes: &[u8], direction: Direction) -> usize {
    let mut offset = 0;
    loop {
        let Some(rel) = bytes[offset..].windows(4).position(|window| window == b"\r\n\r\n") else {
            return bytes.len();
        };
        let head_end = offset + rel + 4;
        let head = &bytes[offset..head_end];
        let Ok(text) = core::str::from_utf8(head) else {
            return offset;
        };
        let content_length = text.split("\r\n").find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        });
        // A server success head (2xx) is followed by the streaming RPC body; a server
        // error head with a small body is drained before the next head.
        if matches!(direction, Direction::Server) && text.starts_with("HTTP/1.1 2") {
            match content_length {
                Some(n) if n < MAX_FRAGMENT => {
                    offset = (head_end + n).min(bytes.len());
                    continue;
                }
                _ => return head_end,
            }
        }
        let next = match direction {
            Direction::Client => head_end,
            Direction::Server => (head_end + content_length.unwrap_or(0)).min(bytes.len()),
        };
        let rest = &bytes[next..];
        let starts_head =
            rest.starts_with(b"RPC_IN_DATA ") || rest.starts_with(b"RPC_OUT_DATA ") || rest.starts_with(b"HTTP/");
        if starts_head && next < bytes.len() {
            offset = next;
        } else {
            return next;
        }
    }
}

struct Pdu {
    ptype: u8,
    call_id: u32,
    /// The operation number for request PDUs (the last request-header field).
    opnum: u16,
    /// The reassembled data stub, excluding trailing security trailers.
    stub: Vec<u8>,
    packet: usize,
}

struct Fragment<'a> {
    ptype: u8,
    flags: u8,
    call_id: u32,
    opnum: u16,
    stub: &'a [u8],
    offset: usize,
}

/// Iterate well-formed DCE/RPC fragments in a channel body.
///
/// A connection using packet integrity pads the stub, then appends an 8-byte security
/// trailer and an authentication token. The stub therefore ends at `frag_len -
/// auth_len - 8 - auth_pad_len`, where `auth_pad_len` is read from the security trailer
/// itself ([MS-RPCE] 2.2.2.10 and 2.2.2.11). `alloc_hint` cannot be used here: on a
/// continuation fragment it advertises the *remaining* stub, not this fragment's share.
fn fragments(mut bytes: &[u8], header_size: usize) -> impl Iterator<Item = Fragment<'_>> {
    let mut offset = 0;
    core::iter::from_fn(move || {
        let header = bytes.get(..COMMON_HEADER)?;
        if header[0] != 5 {
            return None;
        }
        let ptype = header[2];
        let flags = header[3];
        let fragment_length = usize::from(u16::from_le_bytes([header[8], header[9]]));
        if !(COMMON_HEADER..=MAX_FRAGMENT).contains(&fragment_length) {
            return None;
        }
        let auth_length = usize::from(u16::from_le_bytes([header[10], header[11]]));
        let call_id = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
        let pdu = bytes.get(..fragment_length)?;
        let fragment_offset = offset;
        bytes = &bytes[fragment_length..];
        offset += fragment_length;
        // Security trailer layout: auth type, auth level, auth pad length, reserved,
        // context id. The pad length precedes the trailer, so it shrinks the stub.
        let stub_end = if auth_length > 0 {
            let sec_trailer = fragment_length.checked_sub(auth_length + 8)?;
            let auth_pad_len = usize::from(*pdu.get(sec_trailer + 2)?);
            sec_trailer.checked_sub(auth_pad_len)?
        } else {
            fragment_length
        };
        // Control PDUs (bind, auth3) may carry no data stub; clamp instead of stopping so
        // later data PDUs are still visited.
        let stub = if stub_end >= header_size {
            &pdu[header_size..stub_end]
        } else {
            &[][..]
        };
        // Request PDUs carry the opnum as the last two header bytes before the stub.
        let opnum = if ptype == PTYPE_REQUEST {
            pdu.get(header_size - 2..header_size)
                .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
                .unwrap_or(u16::MAX)
        } else {
            u16::MAX
        };
        Some(Fragment {
            ptype,
            flags,
            call_id,
            opnum,
            stub,
            offset: fragment_offset,
        })
    })
}

/// Reassemble DCE/RPC request fragments that share a call id into complete PDUs.
fn request_pdus(bytes: &[u8], packet_offsets: &[(usize, usize)], body_start: usize) -> Vec<Pdu> {
    let mut complete = Vec::new();
    let mut pending: Option<Pdu> = None;
    for fragment in fragments(bytes, REQUEST_HEADER) {
        let first = fragment.flags & PFC_FIRST_FRAG != 0;
        let last = fragment.flags & PFC_LAST_FRAG != 0;
        match pending.as_mut() {
            Some(pdu) if !first && pdu.call_id == fragment.call_id && pdu.ptype == fragment.ptype => {
                pdu.stub.extend_from_slice(fragment.stub);
            }
            _ => {
                pending = Some(Pdu {
                    ptype: fragment.ptype,
                    call_id: fragment.call_id,
                    opnum: fragment.opnum,
                    stub: fragment.stub.to_vec(),
                    packet: packet_at(packet_offsets, body_start + fragment.offset),
                });
            }
        }
        if last {
            if let Some(pdu) = pending.take() {
                complete.push(pdu);
            }
        }
    }
    complete
}

/// Recover the client-to-server RDP stream from IN-channel `TsProxySendToServer`
/// request stubs ([MS-TSGU] 2.2.9.3).
fn client_rdp_bytes(
    body: &[u8],
    packet_offsets: &[(usize, usize)],
    body_start: usize,
) -> Result<PacketStream, ReplayError> {
    let mut output = Vec::new();
    for pdu in request_pdus(body, packet_offsets, body_start) {
        if pdu.ptype != PTYPE_REQUEST || pdu.opnum != SEND_TO_SERVER_OPNUM {
            continue;
        }
        let data = send_to_server_payload(&pdu.stub)?;
        if !data.is_empty() {
            output.push((pdu.packet, data));
        }
    }
    Ok(output)
}

/// Parse a `TsProxySendToServer` stub: context handle, total bytes, buffer count, then
/// per-buffer length-prefixed data.
///
/// [MS-TSGU] 3.6.5.1 requires network byte order. Some clients emit little-endian
/// lengths, so both encodings are accepted when `numBuffers` is in 1..=3.
fn send_to_server_payload(stub: &[u8]) -> Result<Vec<u8>, ReplayError> {
    parse_send_to_server(stub, u32::from_be_bytes).or_else(|_| parse_send_to_server(stub, u32::from_le_bytes))
}

fn parse_send_to_server(stub: &[u8], from_bytes: fn([u8; 4]) -> u32) -> Result<Vec<u8>, ReplayError> {
    const HANDLE: usize = 20;
    let framing = || ReplayError::GatewayFraming("malformed send-to-server stub".to_owned());
    let mut cursor = stub.get(HANDLE..).ok_or_else(framing)?;
    let _total = read_u32(&mut cursor, from_bytes).ok_or_else(framing)?;
    let buffer_count =
        usize::try_from(read_u32(&mut cursor, from_bytes).ok_or_else(framing)?).map_err(|_| framing())?;
    if !(1..=3).contains(&buffer_count) {
        return Err(framing());
    }
    let mut lengths = Vec::with_capacity(buffer_count);
    for _ in 0..buffer_count {
        lengths.push(usize::try_from(read_u32(&mut cursor, from_bytes).ok_or_else(framing)?).map_err(|_| framing())?);
    }
    let mut data = Vec::new();
    for length in lengths {
        let buffer = cursor.get(..length).ok_or_else(framing)?;
        data.extend_from_slice(buffer);
        cursor = &cursor[length..];
    }
    Ok(data)
}

/// Recover the server-to-client RDP stream from OUT-channel receive-pipe response
/// stubs. After `TsProxySetupReceivePipe` the data-plane responses carry the raw RDP
/// bytes directly; control responses (bind ack, tunnel/channel setup) are skipped by
/// their non-RDP stub content.
fn server_rdp_bytes(
    body: &[u8],
    packet_offsets: &[(usize, usize)],
    body_start: usize,
) -> Result<PacketStream, ReplayError> {
    let mut output = Vec::new();
    // The receive pipe carries every server-to-client RDP byte on one RPC call: the
    // `TsProxySetupReceivePipe` call. Ordinary data fragments stay non-final for the
    // session lifetime; PFC_LAST_FRAG closes the pipe and the stub is the HRESULT
    // ([MS-TSGU] 3.2.6.2.2). Other interleaved responses use their own call ids.
    let mut receive_pipe_call = None;
    for fragment in fragments(body, RESPONSE_HEADER) {
        if fragment.ptype != PTYPE_RESPONSE {
            continue;
        }
        match receive_pipe_call {
            None => {
                // The receive pipe's first data response is the X.224 connection confirm.
                if starts_with_tpkt(fragment.stub) {
                    receive_pipe_call = Some(fragment.call_id);
                } else {
                    continue;
                }
            }
            Some(call) if call != fragment.call_id => continue,
            Some(_) => {}
        }
        // The final pipe PDU is a 4-byte return code, not RDP payload.
        if fragment.flags & PFC_LAST_FRAG != 0 && fragment.stub.len() == 4 {
            continue;
        }
        if !fragment.stub.is_empty() {
            output.push((
                packet_at(packet_offsets, body_start + fragment.offset),
                fragment.stub.to_vec(),
            ));
        }
    }
    Ok(output)
}

/// A tunneled RDP record begins with a TPKT header (`03 00`, big-endian length).
fn starts_with_tpkt(stub: &[u8]) -> bool {
    matches!(stub, [0x03, 0x00, ..])
}

fn read_u32(cursor: &mut &[u8], from_bytes: fn([u8; 4]) -> u32) -> Option<u32> {
    let (field, rest) = cursor.split_at_checked(4)?;
    *cursor = rest;
    Some(from_bytes(field.try_into().ok()?))
}
