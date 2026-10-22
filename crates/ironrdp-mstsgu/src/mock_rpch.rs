//! In-process RPC proxy used only by the RPCH session orchestration tests.

use sspi::{
    AuthIdentity, AuthIdentityBuffers, BufferType, CredentialUse, DataRepresentation, Ntlm, SecurityBuffer,
    SecurityBufferRef, ServerRequestFlags, Sspi as _, SspiImpl as _, Username,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};

use crate::rpc::tsgu::{
    TSPROXY_AUTHORIZE_TUNNEL_OPNUM, TSPROXY_CLOSE_CHANNEL_OPNUM, TSPROXY_CLOSE_TUNNEL_OPNUM,
    TSPROXY_CREATE_CHANNEL_OPNUM, TSPROXY_CREATE_TUNNEL_OPNUM, TSPROXY_MAKE_TUNNEL_CALL_OPNUM,
    TSPROXY_SEND_TO_SERVER_OPNUM, TSPROXY_SETUP_RECEIVE_PIPE_OPNUM,
};
use crate::rpc::{
    PFC_FIRST_FRAG, PFC_LAST_FRAG, PFC_SUPPORT_HEADER_SIGN, PTYPE_BIND, PTYPE_REQUEST, PTYPE_RPC_AUTH_3, PTYPE_RTS,
    RPC_AUTH_LEVEL_PACKET_INTEGRITY, RPC_AUTH_TYPE_WINNT, RPC_COMMON_HEADER_SIZE, RPC_NTLM_VERIFIER_LENGTH,
    RPC_SECURITY_TRAILER_SIZE, RpcAuthenticationInfo, RtsCookie, RtsFlowControlAck, decode_rpc_authenticated_fragment,
    encode_rts_flow_control_ack, prepare_rpc_authenticated_pdu,
};

const HTTP_HEAD_LIMIT: usize = 16 * 1024;
const PEER_RECEIVE_WINDOW: u32 = 8 * 1024;
const TUNNEL_CONTEXT: [u8; 20] = [0xaa; 20];
const CHANNEL_CONTEXT: [u8; 20] = [0xbb; 20];

#[derive(Clone, Copy)]
pub(crate) enum MockRpchScenario {
    Echo,
    Message(MockTunnelMessage),
    MessageBeforeCreateChannel(MockTunnelMessage),
    ReceivePipeError(u32),
    TamperedResponse,
    TamperedReceivePipeResponse,
    FragmentedReceivePipePdu,
    InvalidOutFragmentLength,
    BasicAuthentication,
}

#[derive(Clone, Copy)]
pub(crate) enum MockTunnelMessage {
    Service(&'static str),
    Reauthenticate(u64),
}

struct MockRpcSecurity {
    ntlm: Ntlm,
    credentials: Option<AuthIdentityBuffers>,
    receive_sequence: u32,
    send_sequence: u32,
    authentication: RpcAuthenticationInfo,
}

impl MockRpcSecurity {
    fn new() -> Self {
        let mut ntlm = Ntlm::new();
        let identity = AuthIdentity {
            username: Username::parse("CONTOSO\\alice").expect("valid test account"),
            password: "secret".to_owned().into(),
        };
        let credentials = ntlm
            .acquire_credentials_handle()
            .with_credential_use(CredentialUse::Inbound)
            .with_auth_data(&identity)
            .execute(&mut ntlm)
            .expect("acquire proxy NTLM credentials")
            .credentials_handle;
        Self {
            ntlm,
            credentials,
            receive_sequence: 0,
            send_sequence: 0,
            authentication: RpcAuthenticationInfo {
                auth_type: RPC_AUTH_TYPE_WINNT,
                auth_level: RPC_AUTH_LEVEL_PACKET_INTEGRITY,
                auth_context_id: 1,
                verifier_length: RPC_NTLM_VERIFIER_LENGTH,
            },
        }
    }

    fn accept_token(&mut self, token: &[u8]) -> Option<(Vec<u8>, sspi::SecurityStatus)> {
        let mut input = [SecurityBuffer::new(token.to_vec(), BufferType::Token)];
        let mut output = [SecurityBuffer::new(Vec::new(), BufferType::Token)];
        let builder = self
            .ntlm
            .accept_security_context()
            .with_credentials_handle(&mut self.credentials)
            .with_context_requirements(
                ServerRequestFlags::INTEGRITY
                    | ServerRequestFlags::REPLAY_DETECT
                    | ServerRequestFlags::SEQUENCE_DETECT
                    | ServerRequestFlags::ALLOCATE_MEMORY,
            )
            .with_target_data_representation(DataRepresentation::Native)
            .with_input(&mut input)
            .with_output(&mut output);
        let result = self
            .ntlm
            .accept_security_context_impl(builder)
            .ok()?
            .resolve_to_result()
            .ok()?;
        Some((output[0].buffer.clone(), result.status))
    }

    fn verify_request(&mut self, pdu: &mut [u8]) -> Option<Vec<u8>> {
        let (header, trailer) = {
            let fragment = decode_rpc_authenticated_fragment(pdu, u16::MAX).ok()?;
            (fragment.header(), fragment.security_trailer())
        };
        if trailer.auth_type != self.authentication.auth_type
            || trailer.auth_level != self.authentication.auth_level
            || trailer.auth_context_id != self.authentication.auth_context_id
            || header.auth_length() != self.authentication.verifier_length
        {
            return None;
        }
        let verifier_offset = pdu.len().checked_sub(usize::from(header.auth_length()))?;
        let (data, signature) = pdu.split_at_mut(verifier_offset);
        self.ntlm
            .verify_signature(
                &mut [
                    SecurityBufferRef::token_buf(signature),
                    SecurityBufferRef::data_buf(&mut data[..verifier_offset]),
                ],
                self.receive_sequence,
            )
            .ok()?;
        self.receive_sequence = self.receive_sequence.checked_add(1)?;
        let verified = decode_rpc_authenticated_fragment(pdu, u16::MAX)
            .ok()?
            .verify_with(|_, _| Ok::<(), core::convert::Infallible>(()))
            .ok()?;
        Some(verified.body().to_vec())
    }

    fn signed_response(&mut self, call_id: u32, flags: u8, stub: &[u8]) -> Result<Vec<u8>, crate::rpc::RpcPduError> {
        let mut body = Vec::with_capacity(8 + stub.len());
        body.extend_from_slice(&u32::try_from(stub.len()).unwrap_or(u32::MAX).to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&[0, 0]);
        body.extend_from_slice(stub);
        let mut pdu =
            prepare_rpc_authenticated_pdu(crate::rpc::PTYPE_RESPONSE, flags, call_id, &body, self.authentication)?
                .finish(&[0; 16])?;
        sign_server_fragment(&mut self.ntlm, &mut pdu, self.send_sequence);
        self.send_sequence = self.send_sequence.checked_add(1).expect("test sequence fits");
        Ok(pdu)
    }

    fn bind_ack(call_id: u32, type2_token: &[u8]) -> Vec<u8> {
        encode_authenticated_token_pdu(crate::rpc::PTYPE_BIND_ACK, call_id, bind_ack_body(), type2_token)
    }
}

pub(crate) fn mock_rpch_proxy(scenario: MockRpchScenario) -> (DuplexStream, DuplexStream, tokio::task::JoinHandle<()>) {
    let (out_client, out_server) = tokio::io::duplex(256 * 1024);
    let (in_client, in_server) = tokio::io::duplex(256 * 1024);
    let task = tokio::spawn(run_proxy(out_server, in_server, scenario));
    (out_client, in_client, task)
}

async fn run_proxy(mut out: DuplexStream, mut input: DuplexStream, scenario: MockRpchScenario) {
    let requires_basic_auth = matches!(scenario, MockRpchScenario::BasicAuthentication);
    if !begin_in_channel(&mut input, requires_basic_auth).await {
        return;
    }
    if !accept_out_channel(&mut out, requires_basic_auth).await {
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
    let Some(in_cookie) = complete_in_channel(&mut input).await else {
        return;
    };
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
        MockRpchScenario::Message(message) | MockRpchScenario::MessageBeforeCreateChannel(message) => Some(message),
        _ => None,
    };
    let message_before_create_channel = matches!(scenario, MockRpchScenario::MessageBeforeCreateChannel(_));
    let mut tunnel_message_call_id = None;
    let mut receive_pipe_call_id = None;
    let mut receive_pipe_started = false;
    let mut bytes_received = 0u32;
    let mut rpc_security = MockRpcSecurity::new();

    loop {
        let Some(mut pdu) = read_pdu(&mut input).await else {
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
                let Some(type1_token) = auth_token(&pdu) else {
                    return;
                };
                let Some((type2_token, status)) = rpc_security.accept_token(&type1_token) else {
                    return;
                };
                if status != sspi::SecurityStatus::ContinueNeeded {
                    return;
                }
                let bind_ack = MockRpcSecurity::bind_ack(call_id(&pdu), &type2_token);
                if write_all(&mut out, &bind_ack).await.is_err() {
                    return;
                }
            }
            Some(PTYPE_RPC_AUTH_3) => {
                let Some(type3_token) = auth_token(&pdu) else {
                    return;
                };
                let Some((_, status)) = rpc_security.accept_token(&type3_token) else {
                    return;
                };
                if status != sspi::SecurityStatus::CompleteNeeded {
                    return;
                }
                rpc_security
                    .ntlm
                    .complete_auth_token(&mut [])
                    .expect("complete server NTLM context");
            }
            Some(PTYPE_REQUEST) => {
                let Some(body) = rpc_security.verify_request(&mut pdu) else {
                    return;
                };
                let Some(opnum) = body
                    .get(6..8)
                    .and_then(|bytes| bytes.try_into().ok())
                    .map(u16::from_le_bytes)
                else {
                    return;
                };
                let request_call_id = call_id(&pdu);
                if opnum == TSPROXY_SETUP_RECEIVE_PIPE_OPNUM {
                    receive_pipe_call_id = Some(request_call_id);
                    if let Some(result) = receive_pipe_result {
                        if write_response(&mut out, &mut rpc_security, request_call_id, &result.to_le_bytes())
                            .await
                            .is_err()
                        {
                            return;
                        }
                    } else if let (Some(message), Some(call_id)) = (message.take(), tunnel_message_call_id)
                        && write_fragmented_response(
                            &mut out,
                            &mut rpc_security,
                            call_id,
                            &tunnel_message_response(message),
                        )
                        .await
                        .is_err()
                    {
                        return;
                    }
                    continue;
                }

                match opnum {
                    TSPROXY_CREATE_TUNNEL_OPNUM => {
                        if matches!(scenario, MockRpchScenario::TamperedResponse) {
                            let Ok(mut response) = rpc_security.signed_response(
                                request_call_id,
                                PFC_FIRST_FRAG | PFC_LAST_FRAG,
                                &create_tunnel_response(&TUNNEL_CONTEXT, 7),
                            ) else {
                                return;
                            };
                            *response.last_mut().expect("response verifier") ^= 1;
                            if write_all(&mut out, &response).await.is_err() {
                                return;
                            }
                            continue;
                        }
                        if write_fragmented_response(
                            &mut out,
                            &mut rpc_security,
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
                        if write_response(
                            &mut out,
                            &mut rpc_security,
                            request_call_id,
                            &authorize_tunnel_response(),
                        )
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
                        if message_before_create_channel {
                            if let (Some(message), Some(call_id)) = (message.take(), tunnel_message_call_id) {
                                if write_response(
                                    &mut out,
                                    &mut rpc_security,
                                    call_id,
                                    &tunnel_message_response(message),
                                )
                                .await
                                .is_err()
                                {
                                    return;
                                }
                            }
                        }
                        if write_response(
                            &mut out,
                            &mut rpc_security,
                            request_call_id,
                            &create_channel_response(&CHANNEL_CONTEXT, 3),
                        )
                        .await
                        .is_err()
                        {
                            return;
                        }
                    }
                    TSPROXY_SEND_TO_SERVER_OPNUM => {
                        let Some(data) = send_to_server_data(&body) else {
                            return;
                        };
                        let Ok(ack) = encode_rts_flow_control_ack(RtsFlowControlAck::new(
                            bytes_received,
                            PEER_RECEIVE_WINDOW,
                            in_cookie,
                        )) else {
                            return;
                        };
                        if write_all(&mut out, &ack).await.is_err()
                            || write_response(&mut out, &mut rpc_security, request_call_id, &0u32.to_le_bytes())
                                .await
                                .is_err()
                        {
                            return;
                        }
                        if matches!(scenario, MockRpchScenario::TamperedReceivePipeResponse) {
                            let Ok(mut response) = rpc_security.signed_response(
                                receive_pipe_call_id.unwrap_or(request_call_id),
                                PFC_FIRST_FRAG | PFC_LAST_FRAG,
                                data,
                            ) else {
                                return;
                            };
                            *response.last_mut().expect("response verifier") ^= 1;
                            if write_all(&mut out, &response).await.is_err() {
                                return;
                            }
                        } else {
                            let flags = if receive_pipe_started {
                                0
                            } else {
                                receive_pipe_started = true;
                                PFC_FIRST_FRAG
                            };
                            let response = match rpc_security.signed_response(
                                receive_pipe_call_id.unwrap_or(request_call_id),
                                flags,
                                data,
                            ) {
                                Ok(response) => response,
                                Err(_) => return,
                            };
                            let write_result = if matches!(scenario, MockRpchScenario::FragmentedReceivePipePdu) {
                                write_fragmented_transport_pdu(&mut out, &response).await
                            } else {
                                write_all(&mut out, &response).await
                            };
                            if write_result.is_err() {
                                return;
                            }
                        }
                    }
                    TSPROXY_CLOSE_CHANNEL_OPNUM | TSPROXY_CLOSE_TUNNEL_OPNUM => {
                        let response = [0u8; 24];
                        if write_response(&mut out, &mut rpc_security, request_call_id, &response)
                            .await
                            .is_err()
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

async fn begin_in_channel(input: &mut DuplexStream, requires_basic_auth: bool) -> bool {
    let Some(request) = read_request_head(input).await else {
        return false;
    };
    if !valid_in_request(&request, false) {
        return false;
    }
    if write_all(
        input,
        if requires_basic_auth {
            b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nWWW-Authenticate: Basic\r\n\r\n"
        } else {
            b"HTTP/1.1 100 Continue\r\n\r\n"
        },
    )
    .await
    .is_err()
    {
        return false;
    }

    if requires_basic_auth {
        let Some(request) = read_request_head(input).await else {
            return false;
        };
        if !valid_in_request(&request, true) || write_all(input, b"HTTP/1.1 100 Continue\r\n\r\n").await.is_err() {
            return false;
        }
    }

    true
}

async fn complete_in_channel(input: &mut DuplexStream) -> Option<RtsCookie> {
    let conn_b1 = read_pdu(input).await?;
    conn_b1.get(52..68)?.try_into().ok().map(RtsCookie::new)
}

async fn accept_out_channel(out: &mut DuplexStream, requires_basic_auth: bool) -> bool {
    let Some(request) = read_request_head(out).await else {
        return false;
    };
    if !valid_out_request(&request, false) || !read_out_body(out).await {
        return false;
    }

    if requires_basic_auth {
        if write_all(
            out,
            b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nWWW-Authenticate: Basic\r\n\r\n",
        )
        .await
        .is_err()
        {
            return false;
        }
        let Some(request) = read_request_head(out).await else {
            return false;
        };
        valid_out_request(&request, true) && read_out_body(out).await
    } else {
        true
    }
}

async fn read_out_body(out: &mut DuplexStream) -> bool {
    let mut conn_a1 = [0; 76];
    out.read_exact(&mut conn_a1).await.is_ok()
}

async fn read_request_head(stream: &mut DuplexStream) -> Option<String> {
    String::from_utf8(read_http_head(stream).await?).ok()
}

fn request_content_length(request: &str) -> Option<usize> {
    request.split("\r\n").find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse().ok())?
    })
}

fn request_has_basic_auth(request: &str) -> bool {
    request
        .split("\r\n")
        .any(|line| line == "Authorization: Basic Q09OVE9TT1xhbGljZTpzZWNyZXQ=")
}

fn valid_in_request(request: &str, has_basic_auth: bool) -> bool {
    request_content_length(request).is_some_and(|length| length >= 128 * 1024)
        && request.contains("Expect: 100-continue")
        && request_has_basic_auth(request) == has_basic_auth
}

fn valid_out_request(request: &str, has_basic_auth: bool) -> bool {
    request_content_length(request) == Some(76) && request_has_basic_auth(request) == has_basic_auth
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

async fn write_response(
    stream: &mut DuplexStream,
    security: &mut MockRpcSecurity,
    call_id: u32,
    stub: &[u8],
) -> tokio::io::Result<()> {
    let response = security
        .signed_response(call_id, PFC_FIRST_FRAG | PFC_LAST_FRAG, stub)
        .expect("test response fits");
    write_all(stream, &response).await
}

async fn write_fragmented_response(
    stream: &mut DuplexStream,
    security: &mut MockRpcSecurity,
    call_id: u32,
    stub: &[u8],
) -> tokio::io::Result<()> {
    let split_at = stub.len() / 2;
    let first = security
        .signed_response(call_id, PFC_FIRST_FRAG, &stub[..split_at])
        .expect("test response fits");
    let second = security
        .signed_response(call_id, PFC_LAST_FRAG, &stub[split_at..])
        .expect("test response fits");
    write_all(stream, &first).await?;
    write_all(stream, &second).await
}

fn send_to_server_data(body: &[u8]) -> Option<&[u8]> {
    let buffer_length = usize::try_from(u32::from_be_bytes(body.get(36..40)?.try_into().ok()?)).ok()?;
    body.get(40..40usize.checked_add(buffer_length)?)
}

fn call_id(pdu: &[u8]) -> u32 {
    u32::from_le_bytes(pdu[12..16].try_into().expect("complete DCE/RPC header"))
}

async fn write_all(stream: &mut DuplexStream, bytes: &[u8]) -> tokio::io::Result<()> {
    stream.write_all(bytes).await
}

async fn write_fragmented_transport_pdu(stream: &mut DuplexStream, bytes: &[u8]) -> tokio::io::Result<()> {
    for chunk in bytes.chunks(3) {
        stream.write_all(chunk).await?;
        tokio::task::yield_now().await;
    }
    Ok(())
}

fn bind_ack_body() -> Vec<u8> {
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

    body
}

fn encode_authenticated_token_pdu(ptype: u8, call_id: u32, mut body: Vec<u8>, token: &[u8]) -> Vec<u8> {
    let padding = (16 - body.len() % 16) % 16;
    body.resize(body.len() + padding, 0);
    let fragment_length = RPC_COMMON_HEADER_SIZE + body.len() + RPC_SECURITY_TRAILER_SIZE + token.len();
    let mut pdu = vec![
        5,
        0,
        ptype,
        PFC_FIRST_FRAG | PFC_LAST_FRAG | PFC_SUPPORT_HEADER_SIGN,
        0x10,
        0,
        0,
        0,
    ];
    pdu.extend_from_slice(&u16::try_from(fragment_length).expect("test PDU fits").to_le_bytes());
    pdu.extend_from_slice(&u16::try_from(token.len()).expect("test token fits").to_le_bytes());
    pdu.extend_from_slice(&call_id.to_le_bytes());
    pdu.extend_from_slice(&body);
    pdu.extend_from_slice(&[
        RPC_AUTH_TYPE_WINNT,
        RPC_AUTH_LEVEL_PACKET_INTEGRITY,
        u8::try_from(padding).expect("test padding fits"),
        0,
    ]);
    pdu.extend_from_slice(&1u32.to_le_bytes());
    pdu.extend_from_slice(token);
    pdu
}

fn auth_token(pdu: &[u8]) -> Option<Vec<u8>> {
    let parsed = decode_rpc_authenticated_fragment(pdu, u16::MAX).ok()?;
    (parsed.security_trailer().auth_type == RPC_AUTH_TYPE_WINNT).then(|| parsed.verifier().to_vec())
}

fn sign_server_fragment(server: &mut Ntlm, fragment: &mut [u8], sequence_number: u32) {
    let header = crate::rpc::RpcCommonHeader::decode(fragment).expect("RPC response header");
    let verifier_offset = fragment.len() - usize::from(header.auth_length());
    let (data, verifier) = fragment.split_at_mut(verifier_offset);
    server
        .make_signature(
            0,
            &mut [
                SecurityBufferRef::token_buf(verifier),
                SecurityBufferRef::data_buf(&mut data[..verifier_offset]),
            ],
            sequence_number,
        )
        .expect("sign mock RPC response");
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

fn tunnel_message_response(message: MockTunnelMessage) -> Vec<u8> {
    match message {
        MockTunnelMessage::Service(text) => service_message_response(text),
        MockTunnelMessage::Reauthenticate(tunnel_context) => reauthenticate_message_response(tunnel_context),
    }
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
