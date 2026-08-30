//! In-process RPC proxy used only by the RPCH session orchestration tests.

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};

use crate::rpc::tsgu::{
    TSPROXY_AUTHORIZE_TUNNEL_OPNUM, TSPROXY_CREATE_CHANNEL_OPNUM, TSPROXY_CREATE_TUNNEL_OPNUM,
    TSPROXY_MAKE_TUNNEL_CALL_OPNUM, TSPROXY_SEND_TO_SERVER_OPNUM, TSPROXY_SETUP_RECEIVE_PIPE_OPNUM,
};
use crate::rpc::{
    PFC_FIRST_FRAG, PFC_LAST_FRAG, PTYPE_BIND, PTYPE_REQUEST, PTYPE_RTS, RpcResponse, RtsCookie, RtsFlowControlAck,
    encode_rpc_response, encode_rpc_response_fragment, encode_rts_flow_control_ack,
};

const HTTP_HEAD_LIMIT: usize = 16 * 1024;
const PEER_RECEIVE_WINDOW: u32 = 8 * 1024;
const TUNNEL_CONTEXT: [u8; 20] = [0xaa; 20];
const CHANNEL_CONTEXT: [u8; 20] = [0xbb; 20];

#[derive(Clone, Copy)]
pub(crate) enum MockRpchScenario {
    Echo,
    Message(MockTunnelMessage),
    ReceivePipeError(u32),
    InvalidOutFragmentLength,
}

#[derive(Clone, Copy)]
pub(crate) enum MockTunnelMessage {
    Service(&'static str),
    Reauthenticate(u64),
}

pub(crate) fn mock_rpch_proxy(scenario: MockRpchScenario) -> (DuplexStream, DuplexStream, tokio::task::JoinHandle<()>) {
    let (out_client, out_server) = tokio::io::duplex(256 * 1024);
    let (in_client, in_server) = tokio::io::duplex(256 * 1024);
    let task = tokio::spawn(run_proxy(out_server, in_server, scenario));
    (out_client, in_client, task)
}

async fn run_proxy(mut out: DuplexStream, mut input: DuplexStream, scenario: MockRpchScenario) {
    let Some(in_cookie) = accept_in_channel(&mut input).await else {
        return;
    };
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
    if matches!(scenario, MockRpchScenario::InvalidOutFragmentLength) {
        let mut invalid = [0; 16];
        invalid[..8].copy_from_slice(&[5, 0, PTYPE_RTS, 3, 0x10, 0, 0, 0]);
        invalid[8..10].copy_from_slice(&15u16.to_le_bytes());
        let _ = write_all(&mut out, &invalid).await;
        return;
    }
    if write_all(&mut out, &rts_conn_a3(120_000)).await.is_err()
        || write_all(&mut out, &rts_conn_c2(PEER_RECEIVE_WINDOW, 120_000))
            .await
            .is_err()
    {
        return;
    }

    let receive_pipe_result = match scenario {
        MockRpchScenario::ReceivePipeError(result) => Some(result),
        _ => None,
    };
    let mut message = match scenario {
        MockRpchScenario::Message(message) => Some(message),
        _ => None,
    };
    let mut tunnel_message_call_id = None;
    let mut receive_pipe_call_id = None;
    let mut bytes_received = 0u32;

    loop {
        let Some(pdu) = read_pdu(&mut input).await else {
            return;
        };
        let Ok(length) = u32::try_from(pdu.len()) else {
            return;
        };
        bytes_received = match bytes_received.checked_add(length) {
            Some(bytes_received) => bytes_received,
            None => return,
        };

        match pdu.get(2).copied() {
            Some(PTYPE_BIND) => {
                if write_all(&mut out, &bind_ack(call_id(&pdu))).await.is_err() {
                    return;
                }
            }
            Some(PTYPE_REQUEST) => {
                let Some(opnum) = pdu
                    .get(22..24)
                    .and_then(|bytes| bytes.try_into().ok())
                    .map(u16::from_le_bytes)
                else {
                    return;
                };
                let request_call_id = call_id(&pdu);
                if opnum == TSPROXY_SETUP_RECEIVE_PIPE_OPNUM {
                    receive_pipe_call_id = Some(request_call_id);
                    if let Some(result) = receive_pipe_result {
                        if write_response(&mut out, request_call_id, &result.to_le_bytes())
                            .await
                            .is_err()
                        {
                            return;
                        }
                    } else if let (Some(message), Some(call_id)) = (message.take(), tunnel_message_call_id) {
                        let response = match message {
                            MockTunnelMessage::Service(text) => service_message_response(text),
                            MockTunnelMessage::Reauthenticate(tunnel_context) => {
                                reauthenticate_message_response(tunnel_context)
                            }
                        };
                        if write_response(&mut out, call_id, &response).await.is_err() {
                            return;
                        }
                    }
                    continue;
                }

                match opnum {
                    TSPROXY_CREATE_TUNNEL_OPNUM => {
                        if write_fragmented_response(
                            &mut out,
                            request_call_id,
                            &create_tunnel_response(&TUNNEL_CONTEXT, 7),
                        )
                        .await
                        .is_err()
                        {
                            return;
                        }
                    }
                    TSPROXY_AUTHORIZE_TUNNEL_OPNUM => {
                        if write_response(&mut out, request_call_id, &authorize_tunnel_response())
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    TSPROXY_MAKE_TUNNEL_CALL_OPNUM => {
                        if message.is_some() {
                            tunnel_message_call_id = Some(request_call_id);
                        }
                    }
                    TSPROXY_CREATE_CHANNEL_OPNUM => {
                        if write_response(&mut out, request_call_id, &create_channel_response(&CHANNEL_CONTEXT, 3))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    TSPROXY_SEND_TO_SERVER_OPNUM => {
                        let Some(data) = send_to_server_data(&pdu) else {
                            return;
                        };
                        let Ok(ack) = encode_rts_flow_control_ack(RtsFlowControlAck::new(
                            bytes_received,
                            PEER_RECEIVE_WINDOW,
                            in_cookie,
                        )) else {
                            return;
                        };
                        let response_call_id = receive_pipe_call_id.unwrap_or(request_call_id);
                        if write_all(&mut out, &ack).await.is_err()
                            || write_response(&mut out, response_call_id, data).await.is_err()
                        {
                            return;
                        }
                    }
                    _ => return,
                }
            }
            Some(PTYPE_RTS) => {}
            _ => return,
        }
    }
}

async fn accept_in_channel(input: &mut DuplexStream) -> Option<RtsCookie> {
    if read_request_content_length(input).await? != 0 {
        return None;
    }
    write_all(input, b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
        .await
        .ok()?;
    if read_request_content_length(input).await? == 0 {
        return None;
    }
    let conn_b1 = read_pdu(input).await?;
    conn_b1.get(52..68)?.try_into().ok().map(RtsCookie::new)
}

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
    let mut conn_a1 = [0; 76];
    out.read_exact(&mut conn_a1).await.is_ok()
}

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

async fn read_http_head(stream: &mut DuplexStream) -> Option<Vec<u8>> {
    let mut head = Vec::with_capacity(256);
    let mut byte = [0; 1];
    loop {
        stream.read_exact(&mut byte).await.ok()?;
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            return Some(head);
        }
        if head.len() == HTTP_HEAD_LIMIT {
            return None;
        }
    }
}

async fn read_pdu(stream: &mut DuplexStream) -> Option<Vec<u8>> {
    let mut header = [0; 16];
    stream.read_exact(&mut header).await.ok()?;
    let length = usize::from(u16::from_le_bytes(header[8..10].try_into().ok()?));
    if length < header.len() {
        return None;
    }
    let mut pdu = header.to_vec();
    pdu.resize(length, 0);
    stream.read_exact(&mut pdu[header.len()..]).await.ok()?;
    Some(pdu)
}

async fn write_response(stream: &mut DuplexStream, call_id: u32, stub: &[u8]) -> tokio::io::Result<()> {
    write_all(stream, &encode_rpc_response(call_id, stub).expect("test response fits")).await
}

async fn write_fragmented_response(stream: &mut DuplexStream, call_id: u32, stub: &[u8]) -> tokio::io::Result<()> {
    let split_at = stub.len() / 2;
    let first = encode_rpc_response_fragment(RpcResponse {
        call_id,
        pfc_flags: PFC_FIRST_FRAG,
        alloc_hint: u32::try_from(stub.len()).expect("test response fits"),
        cancel_count: 0,
        reserved: 0,
        stub: &stub[..split_at],
    })
    .expect("test response fits");
    let second = encode_rpc_response_fragment(RpcResponse {
        call_id,
        pfc_flags: PFC_LAST_FRAG,
        alloc_hint: u32::try_from(stub.len() - split_at).expect("test response fits"),
        cancel_count: 0,
        reserved: 0,
        stub: &stub[split_at..],
    })
    .expect("test response fits");
    write_all(stream, &first).await?;
    write_all(stream, &second).await
}

fn send_to_server_data(pdu: &[u8]) -> Option<&[u8]> {
    let buffer_length = usize::try_from(u32::from_be_bytes(pdu.get(52..56)?.try_into().ok()?)).ok()?;
    pdu.get(56..56usize.checked_add(buffer_length)?)
}

fn call_id(pdu: &[u8]) -> u32 {
    u32::from_le_bytes(pdu[12..16].try_into().expect("complete DCE/RPC header"))
}

async fn write_all(stream: &mut DuplexStream, bytes: &[u8]) -> tokio::io::Result<()> {
    stream.write_all(bytes).await
}

fn bind_ack(call_id: u32) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&0x2000u16.to_le_bytes());
    body.extend_from_slice(&0x2000u16.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&0u16.to_le_bytes());
    body.extend_from_slice(&[0, 0]);
    body.extend_from_slice(&[1, 0, 0, 0]);
    body.extend_from_slice(&[0, 0, 0, 0]);
    body.extend_from_slice(&[
        0x04, 0x5d, 0x88, 0x8a, 0xeb, 0x1c, 0xc9, 0x11, 0x9f, 0xe8, 0x08, 0x00, 0x2b, 0x10, 0x48, 0x60,
    ]);
    body.extend_from_slice(&2u32.to_le_bytes());

    let mut pdu = vec![5, 0, 12, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0];
    pdu.extend_from_slice(
        &u16::try_from(16 + body.len())
            .expect("test bind acknowledgement fits")
            .to_le_bytes(),
    );
    pdu.extend_from_slice(&0u16.to_le_bytes());
    pdu.extend_from_slice(&call_id.to_le_bytes());
    pdu.extend_from_slice(&body);
    pdu
}

fn rts_conn_a3(connection_timeout: u32) -> Vec<u8> {
    rts_pdu(
        &[[2, 0, 0, 0].as_slice(), connection_timeout.to_le_bytes().as_slice()].concat(),
        1,
    )
}

fn rts_conn_c2(receive_window_size: u32, connection_timeout: u32) -> Vec<u8> {
    rts_pdu(
        &[
            [6, 0, 0, 0].as_slice(),
            1u32.to_le_bytes().as_slice(),
            [0, 0, 0, 0].as_slice(),
            receive_window_size.to_le_bytes().as_slice(),
            [2, 0, 0, 0].as_slice(),
            connection_timeout.to_le_bytes().as_slice(),
        ]
        .concat(),
        3,
    )
}

fn rts_pdu(commands: &[u8], command_count: u16) -> Vec<u8> {
    let mut pdu = vec![5, 0, PTYPE_RTS, 3, 0x10, 0, 0, 0];
    pdu.extend_from_slice(&u16::try_from(20 + commands.len()).expect("test RTS fits").to_le_bytes());
    pdu.extend_from_slice(&0u16.to_le_bytes());
    pdu.extend_from_slice(&0u32.to_le_bytes());
    pdu.extend_from_slice(&0u16.to_le_bytes());
    pdu.extend_from_slice(&command_count.to_le_bytes());
    pdu.extend_from_slice(commands);
    pdu
}

fn create_tunnel_response(tunnel_context: &[u8; 20], tunnel_id: u32) -> Vec<u8> {
    [
        0x0002_0000u32.to_le_bytes().as_slice(),
        0x0000_4552u32.to_le_bytes().as_slice(),
        0x0000_4552u32.to_le_bytes().as_slice(),
        0x0002_0004u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        &[
            0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0x99, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0,
        ],
        0x0002_0008u32.to_le_bytes().as_slice(),
        0x5452u16.to_le_bytes().as_slice(),
        0x5643u16.to_le_bytes().as_slice(),
        0x0002_000cu32.to_le_bytes().as_slice(),
        1u32.to_le_bytes().as_slice(),
        1u16.to_le_bytes().as_slice(),
        1u16.to_le_bytes().as_slice(),
        0u16.to_le_bytes().as_slice(),
        0u16.to_le_bytes().as_slice(),
        1u32.to_le_bytes().as_slice(),
        1u32.to_le_bytes().as_slice(),
        1u32.to_le_bytes().as_slice(),
        3u32.to_le_bytes().as_slice(),
        tunnel_context,
        tunnel_id.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
    ]
    .concat()
}

fn authorize_tunnel_response() -> Vec<u8> {
    [
        0x0002_0000u32.to_le_bytes().as_slice(),
        0x0000_5052u32.to_le_bytes().as_slice(),
        0x0000_5052u32.to_le_bytes().as_slice(),
        0x0002_0004u32.to_le_bytes().as_slice(),
        0x0000_5152u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
    ]
    .concat()
}

fn create_channel_response(channel_context: &[u8; 20], channel_id: u32) -> Vec<u8> {
    [
        channel_context.as_slice(),
        channel_id.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
    ]
    .concat()
}

fn service_message_response(text: &str) -> Vec<u8> {
    let mut text: Vec<u8> = text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    let text_length = u32::try_from(text.len() / 2).expect("test message fits");
    let mut response = [
        0x0002_0000u32.to_le_bytes().as_slice(),
        0x0000_4750u32.to_le_bytes().as_slice(),
        0x0000_4750u32.to_le_bytes().as_slice(),
        0x0002_0004u32.to_le_bytes().as_slice(),
        1u32.to_le_bytes().as_slice(),
        2u32.to_le_bytes().as_slice(),
        1u32.to_le_bytes().as_slice(),
        2u32.to_le_bytes().as_slice(),
        0x0002_0008u32.to_le_bytes().as_slice(),
        1u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        text_length.to_le_bytes().as_slice(),
        0x0002_000cu32.to_le_bytes().as_slice(),
        text_length.to_le_bytes().as_slice(),
    ]
    .concat();
    response.append(&mut text);
    response.resize((response.len() + 3) & !3, 0);
    response.extend_from_slice(&0u32.to_le_bytes());
    response
}

fn reauthenticate_message_response(tunnel_context: u64) -> Vec<u8> {
    let mut response = [
        0x0002_0000u32.to_le_bytes().as_slice(),
        0x0000_4750u32.to_le_bytes().as_slice(),
        0x0000_4750u32.to_le_bytes().as_slice(),
        0x0002_0004u32.to_le_bytes().as_slice(),
        1u32.to_le_bytes().as_slice(),
        3u32.to_le_bytes().as_slice(),
        1u32.to_le_bytes().as_slice(),
        3u32.to_le_bytes().as_slice(),
        0x0002_0008u32.to_le_bytes().as_slice(),
    ]
    .concat();
    response.resize((response.len() + 7) & !7, 0);
    response.extend_from_slice(&tunnel_context.to_le_bytes());
    response.extend_from_slice(&0u32.to_le_bytes());
    response
}
