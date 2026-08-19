//! MS-TSGU RPC-over-HTTP (RPCH) tunnel extraction for captured TLS-decrypted flows.
//!
//! The legacy RD Gateway transport carries the tunnelled RDP session inside DCE/RPC
//! calls on two HTTP/1 connections: an IN channel (`RPC_IN_DATA`, client to server) and
//! an OUT channel (`RPC_OUT_DATA`, server to client). This module unwraps the DCE/RPC
//! and TsProxy framing and recovers the tunnelled RDP byte stream ([MS-TSGU] 3.6).

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

/// `TsProxySendToServer` operation number ([MS-TSGU] 3.6.5.1).
const SEND_TO_SERVER_OPNUM: u16 = 9;

/// Which RPCH channel a decrypted flow carries, keyed by its HTTP method.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RpchChannel {
    /// `RPC_IN_DATA`: client-to-server tunnel data (`TsProxySendToServer`).
    In,
    /// `RPC_OUT_DATA`: server-to-client tunnel data (the receive pipe).
    Out,
}

/// Classify a decrypted RPCH flow, or `None` when it is not an RPCH tunnel.
pub(crate) fn rpch_channel(plaintext: &Plaintext) -> Option<RpchChannel> {
    let client = flatten(&plaintext.client);
    if client.starts_with(b"RPC_IN_DATA ") {
        Some(RpchChannel::In)
    } else if client.starts_with(b"RPC_OUT_DATA ") {
        Some(RpchChannel::Out)
    } else {
        None
    }
}

/// Extract the tunnelled RDP bytes from a pair of decrypted RPCH channel flows.
///
/// `input` is the IN-channel flow (client requests) and `output` is the OUT-channel
/// flow (server responses). The returned streams contain raw RDP traffic (starting
/// with the X.224 connection request) exactly as a direct TCP capture would carry it.
pub(crate) fn extract_tunneled_rdp(input: &Plaintext, output: &Plaintext) -> Result<Plaintext, ReplayError> {
    let client_flat = flatten(&input.client);
    let server_flat = flatten(&output.server);
    let client_body = strip_http_heads(&client_flat, Direction::Client);
    let server_body = strip_http_heads(&server_flat, Direction::Server);

    let client = client_rdp_bytes(client_body)?;
    let server = server_rdp_bytes(server_body)?;
    if client.is_empty() || server.is_empty() {
        return Err(ReplayError::MissingRdpState);
    }
    Ok(Plaintext { client, server })
}

#[derive(Clone, Copy)]
enum Direction {
    Client,
    Server,
}

/// Strip every HTTP request/response head, returning the concatenated RPC body.
///
/// Authentication adds 401 round trips before the 200 that opens the streaming body;
/// each head ends at `\r\n\r\n`, and a server head with a small `Content-Length`
/// (an error page) is followed by that body before the next head.
fn strip_http_heads(bytes: &[u8], direction: Direction) -> &[u8] {
    let mut offset = 0;
    loop {
        let Some(rel) = bytes[offset..].windows(4).position(|w| w == b"\r\n\r\n") else {
            return &bytes[bytes.len()..];
        };
        let head_end = offset + rel + 4;
        let head = &bytes[offset..head_end];
        let Ok(text) = core::str::from_utf8(head) else {
            return &bytes[offset..];
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
                _ => return &bytes[head_end..],
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
            return rest;
        }
    }
}

struct Pdu<'a> {
    ptype: u8,
    call_id: u32,
    /// The operation number for request PDUs (the last request-header field).
    opnum: u16,
    /// The data stub, excluding any trailing security trailer and authentication token.
    stub: &'a [u8],
}

/// Iterate the well-formed DCE/RPC PDUs in a channel body, yielding each data stub.
///
/// A connection using packet integrity pads the stub, then appends an 8-byte security
/// trailer and an authentication token. The stub therefore ends at `frag_len -
/// auth_len - 8 - auth_pad_len`, where `auth_pad_len` is read from the security trailer
/// itself ([MS-RPCE] 2.2.2.10 and 2.2.2.11). `alloc_hint` cannot be used here: on a
/// continuation fragment it advertises the *remaining* stub, not this fragment's share.
fn pdus(mut bytes: &[u8], header_size: usize) -> impl Iterator<Item = Pdu<'_>> {
    core::iter::from_fn(move || {
        let header = bytes.get(..COMMON_HEADER)?;
        if header[0] != 5 {
            return None;
        }
        let ptype = header[2];
        let fragment_length = usize::from(u16::from_le_bytes([header[8], header[9]]));
        if !(COMMON_HEADER..=MAX_FRAGMENT).contains(&fragment_length) {
            return None;
        }
        let auth_length = usize::from(u16::from_le_bytes([header[10], header[11]]));
        let call_id = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
        let pdu = bytes.get(..fragment_length)?;
        bytes = &bytes[fragment_length..];
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
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .unwrap_or(u16::MAX)
        } else {
            u16::MAX
        };
        Some(Pdu {
            ptype,
            call_id,
            opnum,
            stub,
        })
    })
}

/// Recover the client-to-server RDP stream from IN-channel `TsProxySendToServer`
/// request stubs ([MS-TSGU] 2.2.9.3).
fn client_rdp_bytes(body: &[u8]) -> Result<PacketStream, ReplayError> {
    let mut output = Vec::new();
    for pdu in pdus(body, REQUEST_HEADER) {
        if pdu.ptype != PTYPE_REQUEST || pdu.opnum != SEND_TO_SERVER_OPNUM {
            continue;
        }
        let data = send_to_server_payload(pdu.stub)?;
        if !data.is_empty() {
            output.push((usize::try_from(pdu.call_id).unwrap_or(0), data));
        }
    }
    Ok(output)
}

/// Parse a `TsProxySendToServer` stub: context handle, total bytes, buffer count, then
/// per-buffer length-prefixed data. The data-plane length fields are little-endian on
/// the wire (matching mstsc), like the NDR control stubs.
fn send_to_server_payload(stub: &[u8]) -> Result<Vec<u8>, ReplayError> {
    const HANDLE: usize = 20;
    let framing = || ReplayError::GatewayFraming("malformed send-to-server stub".to_owned());
    let mut cursor = stub.get(HANDLE..).ok_or_else(framing)?;
    let _total = read_le_u32(&mut cursor).ok_or_else(framing)?;
    let buffer_count = usize::try_from(read_le_u32(&mut cursor).ok_or_else(framing)?).map_err(|_| framing())?;
    if buffer_count > 3 {
        return Err(framing());
    }
    let mut lengths = Vec::with_capacity(buffer_count);
    for _ in 0..buffer_count {
        lengths.push(usize::try_from(read_le_u32(&mut cursor).ok_or_else(framing)?).map_err(|_| framing())?);
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
fn server_rdp_bytes(body: &[u8]) -> Result<PacketStream, ReplayError> {
    let mut output = Vec::new();
    // The receive pipe carries every server-to-client RDP byte on one RPC call: the
    // `TsProxySetupReceivePipe` call. That call's response stream starts with the X.224
    // connection confirm (TPKT). Other interleaved responses (bind ack, tunnel/channel
    // setup, MakeTunnelCall admin replies) use their own call ids and are not RDP.
    let mut receive_pipe_call = None;
    for pdu in pdus(body, RESPONSE_HEADER) {
        if pdu.ptype != PTYPE_RESPONSE {
            continue;
        }
        let stub = pdu.stub;
        match receive_pipe_call {
            None => {
                // The receive pipe's first data response is the X.224 connection confirm.
                if starts_with_tpkt(stub) {
                    receive_pipe_call = Some(pdu.call_id);
                } else {
                    continue;
                }
            }
            // After the pipe opens, its data is a contiguous RDP byte stream split
            // arbitrarily across responses; keep only this call's stubs.
            Some(call) if call != pdu.call_id => continue,
            Some(_) => {}
        }
        if !stub.is_empty() {
            output.push((usize::try_from(pdu.call_id).unwrap_or(0), stub.to_vec()));
        }
    }
    Ok(output)
}

/// A tunnelled RDP record begins with a TPKT header (`03 00`, big-endian length).
fn starts_with_tpkt(stub: &[u8]) -> bool {
    matches!(stub, [0x03, 0x00, ..])
}

fn read_le_u32(cursor: &mut &[u8]) -> Option<u32> {
    let (field, rest) = cursor.split_at_checked(4)?;
    *cursor = rest;
    Some(u32::from_le_bytes(field.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Common 16-byte header: version, ptype, flags, data rep, fragment length,
    /// auth length (0 = unsigned), call id.
    fn common_header(ptype: u8, call_id: u32, fragment_length: usize) -> Vec<u8> {
        let mut pdu = vec![5, 0, ptype, 0x03, 0x10, 0, 0, 0];
        pdu.extend_from_slice(&u16::try_from(fragment_length).unwrap().to_le_bytes());
        pdu.extend_from_slice(&0u16.to_le_bytes()); // auth length
        pdu.extend_from_slice(&call_id.to_le_bytes());
        pdu
    }

    fn request_pdu(call_id: u32, opnum: u16, stub: &[u8]) -> Vec<u8> {
        let fragment_length = REQUEST_HEADER + stub.len();
        let mut pdu = common_header(PTYPE_REQUEST, call_id, fragment_length);
        pdu.extend_from_slice(&u32::try_from(stub.len()).unwrap().to_le_bytes()); // alloc hint
        pdu.extend_from_slice(&0u16.to_le_bytes()); // context id
        pdu.extend_from_slice(&opnum.to_le_bytes());
        pdu.extend_from_slice(stub);
        pdu
    }

    fn response_pdu(call_id: u32, stub: &[u8]) -> Vec<u8> {
        let fragment_length = RESPONSE_HEADER + stub.len();
        let mut pdu = common_header(PTYPE_RESPONSE, call_id, fragment_length);
        pdu.extend_from_slice(&u32::try_from(stub.len()).unwrap().to_le_bytes()); // alloc hint
        pdu.extend_from_slice(&0u16.to_le_bytes()); // context id
        pdu.extend_from_slice(&[0, 0]); // cancel count + pad
        pdu.extend_from_slice(stub);
        pdu
    }

    fn send_to_server_stub(buffers: &[&[u8]]) -> Vec<u8> {
        let mut stub = vec![0u8; 20]; // context handle
        let total: usize = buffers.iter().map(|b| b.len()).sum::<usize>() + 4 * buffers.len();
        stub.extend_from_slice(&u32::try_from(total).unwrap().to_le_bytes());
        stub.extend_from_slice(&u32::try_from(buffers.len()).unwrap().to_le_bytes());
        for buffer in buffers {
            stub.extend_from_slice(&u32::try_from(buffer.len()).unwrap().to_le_bytes());
        }
        for buffer in buffers {
            stub.extend_from_slice(buffer);
        }
        stub
    }

    #[test]
    fn detects_rpch_tunnel() {
        let plaintext = Plaintext {
            client: vec![(1, b"RPC_OUT_DATA /rpc/rpcproxy.dll?x HTTP/1.1\r\n\r\n".to_vec())],
            server: vec![(2, b"HTTP/1.1 200 OK\r\n\r\n".to_vec())],
        };
        assert_eq!(rpch_channel(&plaintext), Some(RpchChannel::Out));
    }

    #[test]
    fn extracts_send_to_server_payload() {
        let rdp = [0x03, 0x00, 0x00, 0x13, 0x0e, 0xe0];
        let stub = send_to_server_stub(&[&rdp]);
        let body = request_pdu(7, SEND_TO_SERVER_OPNUM, &stub);

        let stream = client_rdp_bytes(&body).unwrap();

        assert_eq!(stream, vec![(7, rdp.to_vec())]);
    }

    #[test]
    fn skips_non_data_requests() {
        let control = request_pdu(1, 1, &[0u8; 32]); // create tunnel
        let rdp = [9u8; 8];
        let mut body = control;
        body.extend(request_pdu(2, SEND_TO_SERVER_OPNUM, &send_to_server_stub(&[&rdp])));

        let stream = client_rdp_bytes(&body).unwrap();

        assert_eq!(stream, vec![(2, rdp.to_vec())]);
    }

    #[test]
    fn extracts_receive_pipe_data_after_first_tpkt() {
        let mut body = response_pdu(1, &[0u8; 16]); // control response, no TPKT
        let cc = [0x03, 0x00, 0x00, 0x13, 0x0e, 0xd0];
        body.extend(response_pdu(6, &cc)); // receive-pipe call opens with the TPKT
        body.extend(response_pdu(7, &[0u8; 40])); // interleaved admin response, other call
        let data = [0x03, 0x00, 0x00, 0x20, 0x02, 0xf0];
        body.extend(response_pdu(6, &data)); // more receive-pipe data

        let stream = server_rdp_bytes(&body).unwrap();

        assert_eq!(stream, vec![(6, cc.to_vec()), (6, data.to_vec())]);
    }

    #[test]
    fn strips_authentication_round_trips() {
        let rdp = [1u8, 2, 3];
        let mut client = b"RPC_IN_DATA /rpc/rpcproxy.dll?x HTTP/1.1\r\nContent-Length: 0\r\n\r\n".to_vec();
        client.extend(b"RPC_IN_DATA /rpc/rpcproxy.dll?x HTTP/1.1\r\nContent-Length: 1073741824\r\n\r\n");
        client.extend(request_pdu(1, SEND_TO_SERVER_OPNUM, &send_to_server_stub(&[&rdp])));
        let input = Plaintext {
            client: vec![(1, client)],
            server: Vec::new(),
        };
        let mut server = b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 13\r\n\r\nAccess Denied".to_vec();
        server.extend(b"HTTP/1.1 200 OK\r\n\r\n");
        server.extend(response_pdu(1, &[0x03, 0x00, 0x00, 0x13, 0x0e, 0xd0]));
        let output = Plaintext {
            client: Vec::new(),
            server: vec![(2, server)],
        };

        let inner = extract_tunneled_rdp(&input, &output).unwrap();

        assert_eq!(inner.client, vec![(1, rdp.to_vec())]);
        assert_eq!(inner.server, vec![(1, vec![0x03, 0x00, 0x00, 0x13, 0x0e, 0xd0])]);
    }
}
