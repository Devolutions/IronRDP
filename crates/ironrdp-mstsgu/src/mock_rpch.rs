//! In-process mock RPC proxy for the MS-RPCH v2 driver tests.
//!
//! Speaks raw HTTP/1 heads plus the RTS CONN setup, the unauthenticated DCE/RPC
//! bind, and the TsProxy call sequence over two in-memory duplex streams.

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};

use crate::rpc::{RpcContextHandle, fixtures};

/// `TsProxyCreateTunnel` RPC operation number.
const OP_CREATE_TUNNEL: u16 = 1;
const OP_AUTHORIZE_TUNNEL: u16 = 2;
const OP_MAKE_TUNNEL_CALL: u16 = 3;
const OP_CREATE_CHANNEL: u16 = 4;
const OP_SETUP_RECEIVE_PIPE: u16 = 8;
const OP_SEND_TO_SERVER: u16 = 9;

const PTYPE_REQUEST: u8 = 0;
const PTYPE_BIND: u8 = 11;

const TUNNEL_CONTEXT: [u8; RpcContextHandle::SIZE] = [0xAA; RpcContextHandle::SIZE];
const CHANNEL_CONTEXT: [u8; RpcContextHandle::SIZE] = [0xBB; RpcContextHandle::SIZE];

/// Client-side ends of a mock RPC proxy: `(out, in)` streams for `rpch_connect`.
pub(crate) fn mock_rpch_proxy() -> (DuplexStream, DuplexStream, tokio::task::JoinHandle<()>) {
    mock_rpch_proxy_with(None, None)
}

/// Mock proxy that completes the pending make-tunnel-call with a service message.
pub(crate) fn mock_rpch_proxy_with_service_message(
    service_message: Option<&'static str>,
) -> (DuplexStream, DuplexStream, tokio::task::JoinHandle<()>) {
    mock_rpch_proxy_with(service_message, None)
}

/// Mock proxy whose `TsProxySetupReceivePipe` fails with the given gateway error code.
pub(crate) fn mock_rpch_proxy_with_receive_pipe_error(
    result: u32,
) -> (DuplexStream, DuplexStream, tokio::task::JoinHandle<()>) {
    mock_rpch_proxy_with(None, Some(result))
}

fn mock_rpch_proxy_with(
    service_message: Option<&'static str>,
    receive_pipe_result: Option<u32>,
) -> (DuplexStream, DuplexStream, tokio::task::JoinHandle<()>) {
    let (out_client, out_server) = tokio::io::duplex(256 * 1024);
    let (in_client, in_server) = tokio::io::duplex(256 * 1024);
    let task =
        tokio::spawn(async move { run_proxy(out_server, in_server, service_message, receive_pipe_result).await });
    (out_client, in_client, task)
}

async fn run_proxy(
    mut out: DuplexStream,
    mut input: DuplexStream,
    service_message: Option<&'static str>,
    receive_pipe_result: Option<u32>,
) {
    // The client authenticates each channel with a Content-Length: 0 probe, then resends
    // the request carrying the body. This mock enforces no authentication, so it accepts
    // the probe with 200 and reads the resent request's body.
    //
    // IN channel is established before the OUT channel, matching the client driver.
    if !accept_in_channel(&mut input).await {
        return;
    }

    // OUT channel: probe, then request head + CONN/A1 body, then the streaming response.
    if !accept_out_channel(&mut out).await {
        return;
    }
    if write_all(
        &mut out,
        b"HTTP/1.1 200 OK\r\nContent-Type: application/rpc\r\nContent-Length: 1073741824\r\n\r\n",
    )
    .await
    .is_err()
    {
        return;
    }
    if write_all(&mut out, &fixtures::rts_conn_a3(120_000)).await.is_err() {
        return;
    }
    if write_all(&mut out, &fixtures::rts_conn_c2(1, 128 * 1024, 120_000))
        .await
        .is_err()
    {
        return;
    }

    // TsProxySetupReceivePipe switches the OUT channel to the target-server byte stream; on
    // success it has no discrete response, so the echoed payload travels on that call.
    let mut receive_pipe_call_id: Option<u32> = None;

    loop {
        let Some(pdu) = read_pdu(&mut input).await else {
            return;
        };
        match pdu.get(2).copied() {
            Some(PTYPE_BIND) => {
                let call_id = call_id(&pdu);
                if write_all(&mut out, &fixtures::bind_ack(call_id)).await.is_err() {
                    return;
                }
            }
            Some(PTYPE_REQUEST) => {
                let call_id = call_id(&pdu);
                // Request body: alloc hint (4) + context id (2) + opnum (2) + stub.
                let Some(opnum) = pdu
                    .get(22..24)
                    .map(|b| u16::from_le_bytes(b.try_into().expect("opnum")))
                else {
                    return;
                };
                if opnum == OP_SETUP_RECEIVE_PIPE {
                    receive_pipe_call_id = Some(call_id);
                    // On failure the pipe answers with the terminal return-value fragment; on
                    // success there is no response and the data stream follows on this call.
                    if let Some(result) = receive_pipe_result {
                        let stub = fixtures::receive_pipe_final_return_value(result);
                        if write_all(&mut out, &fixtures::rpc_response(call_id, &stub))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    continue;
                }
                let stub = match opnum {
                    OP_CREATE_TUNNEL => fixtures::create_tunnel_response(&TUNNEL_CONTEXT, 7),
                    OP_AUTHORIZE_TUNNEL => fixtures::authorize_tunnel_response(),
                    OP_CREATE_CHANNEL => fixtures::create_channel_response(&CHANNEL_CONTEXT, 3),
                    // Administrative-message calls stay pending unless the mock was
                    // configured to complete with a service message.
                    OP_MAKE_TUNNEL_CALL => match service_message {
                        Some(text) => fixtures::make_tunnel_call_service_response(text),
                        None => continue,
                    },
                    OP_SEND_TO_SERVER => {
                        // Echo the first payload buffer back as receive-pipe data.
                        // TsProxySendToServer length fields are little-endian on the wire
                        // (matching mstsc; [MS-TSGU] 3.6.5.1 notwithstanding).
                        let Some(buffer_len) = pdu
                            .get(52..56)
                            .map(|b| u32::from_le_bytes(b.try_into().expect("buffer length")))
                            .map(|len| usize::try_from(len).expect("buffer length"))
                        else {
                            return;
                        };
                        let Some(data) = pdu.get(56..56 + buffer_len) else {
                            return;
                        };
                        data.to_vec()
                    }
                    _ => continue,
                };
                // Send-to-server data is echoed on the receive pipe's call, matching the
                // gateway's streaming responses ([MS-TSGU] 3.2.6.2.3).
                let response_call_id = if opnum == OP_SEND_TO_SERVER {
                    receive_pipe_call_id.unwrap_or(call_id)
                } else {
                    call_id
                };
                if write_all(&mut out, &fixtures::rpc_response(response_call_id, &stub))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            _ => {}
        }
    }
}

/// Reads one HTTP/1 request or interim response head (up to `\r\n\r\n`).
/// Reads one request head and returns its `Content-Length`.
async fn read_request_content_length(stream: &mut DuplexStream) -> Option<usize> {
    let head = read_http_head(stream).await?;
    let text = core::str::from_utf8(&head).ok()?;
    text.split("\r\n").find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse().ok())?
    })
}

/// Accepts the IN channel: a Content-Length: 0 probe answered 200, then the streaming
/// request whose first body PDU is CONN/B1.
async fn accept_in_channel(input: &mut DuplexStream) -> bool {
    if read_request_content_length(input).await != Some(0) {
        return false;
    }
    if write_all(input, b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
        .await
        .is_err()
    {
        return false;
    }
    if read_request_content_length(input).await.is_none() {
        return false;
    }
    // CONN/B1 is the first body PDU; consume it before the bind.
    read_pdu(input).await.is_some()
}

/// Accepts the OUT channel: a Content-Length: 0 probe answered 200, then the request
/// carrying the 76-byte CONN/A1 body.
async fn accept_out_channel(out: &mut DuplexStream) -> bool {
    if read_request_content_length(out).await != Some(0) {
        return false;
    }
    if write_all(out, b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
        .await
        .is_err()
    {
        return false;
    }
    if read_request_content_length(out).await != Some(76) {
        return false;
    }
    let mut conn_a1 = [0u8; 76];
    out.read_exact(&mut conn_a1).await.is_ok()
}

async fn read_http_head(stream: &mut DuplexStream) -> Option<Vec<u8>> {
    let mut head = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await.ok()?;
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            return Some(head);
        }
        if head.len() > 16 * 1024 {
            return None;
        }
    }
}

/// Reads one DCE/RPC PDU, framed by the common-header fragment length.
async fn read_pdu(stream: &mut DuplexStream) -> Option<Vec<u8>> {
    let mut header = [0u8; 16];
    stream.read_exact(&mut header).await.ok()?;
    let length = usize::from(u16::from_le_bytes(header[8..10].try_into().expect("fragment length")));
    if !(16..=1024 * 1024).contains(&length) {
        return None;
    }
    let mut pdu = header.to_vec();
    pdu.resize(length, 0);
    stream.read_exact(&mut pdu[16..]).await.ok()?;
    Some(pdu)
}

fn call_id(pdu: &[u8]) -> u32 {
    u32::from_le_bytes(pdu[12..16].try_into().expect("call id"))
}

async fn write_all(stream: &mut DuplexStream, bytes: &[u8]) -> tokio::io::Result<()> {
    stream.write_all(bytes).await
}
