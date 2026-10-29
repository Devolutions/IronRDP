//! In-process RPC proxy used only by the RPCH session orchestration tests.

use sspi::{
    AuthIdentity, AuthIdentityBuffers, BufferType, CredentialUse, DataRepresentation, Ntlm, SecurityBuffer,
    SecurityBufferRef, ServerRequestFlags, Sspi as _, SspiImpl as _, Username,
};
use std::collections::VecDeque;

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, DuplexStream};
use tokio::sync::mpsc;

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
    ReceivePipeError(u32),
    TamperedResponse,
    TamperedReceivePipeResponse,
    FragmentedReceivePipePdu,
    InvalidOutFragmentLength,
    BasicAuthentication,
    /// The client recycles its IN channel once its bounded IN body runs low.
    RecycleIn,
    RecycleInR2,
    /// The proxy recycles the OUT channel with the OUT_R1 sequence on the first send.
    RecycleOutR1,
    /// The proxy recycles the OUT channel with the OUT_R2 sequence on the first send.
    RecycleOutR2,
    RecycleOutDelayedHead,
    RecycleOutRejectedHead,
    /// The proxy fails the IN channel with an RPC-Error response on the first send.
    InHttpError(u32),
    /// The proxy negotiates consent and service messages, sends a consent message with the tunnel, answers the
    /// first message call with a service message and keeps the next one pending until the client cancels it.
    Messaging,
    MessageCallFault,
    TamperedMessageCallFault,
    UnsignedMessageCallFault,
    InterleavedMessage,
    LargeConsent,
    PartialMessageFault,
    ReceivePipeOverflow,
    InvalidReceivePipeFinal,
    UnexpectedResponse,
    /// The proxy negotiates reauthentication, answers the first message call with a reauthentication message and
    /// keeps the next one pending until the client cancels it.
    Reauth,
    /// The reauthentication connection: the proxy requires a reauthentication tunnel naming [`REAUTH_TUNNEL_CONTEXT`]
    /// and reports it once the client closes that tunnel.
    ReauthConnection,
}

/// Tunnel context the [`MockRpchScenario::Reauth`] proxy names in its reauthentication message.
pub(crate) const REAUTH_TUNNEL_CONTEXT: u64 = 0x1122_3344_5566_7788;
const CONSENT_MESSAGE: &str = "Accept the gateway policy";
const SERVICE_MESSAGE: &str = "Maintenance tonight";
const TSG_PACKET_TYPE_REAUTH: u32 = 0x0000_5250;
const TSG_TUNNEL_CALL_ASYNC_MSG_REQUEST: u32 = 1;
const TSG_TUNNEL_CANCEL_ASYNC_MSG_REQUEST: u32 = 2;
/// HRESULT_FROM_WIN32(ERROR_CANCELLED), returned for the message call the client cancelled.
const CANCELLED_HRESULT: u32 = 0x8007_04c7;

/// Load-balancer cookie the mock sets on the first response; every later request must return it.
const LB_COOKIE: &str = "RpchLb=node1";
const STREAM_BUFFER: usize = 256 * 1024;

/// Opens additional IN and OUT connections to a running mock proxy, as channel recycling does.
pub(crate) struct MockRpchConnector {
    in_connections: mpsc::UnboundedSender<DuplexStream>,
    out_connections: mpsc::UnboundedSender<DuplexStream>,
}

impl MockRpchConnector {
    pub(crate) fn connect_in(&self) -> Option<DuplexStream> {
        let (client, server) = tokio::io::duplex(STREAM_BUFFER);
        self.in_connections.send(server).ok()?;
        Some(client)
    }

    pub(crate) fn connect_out(&self) -> Option<DuplexStream> {
        let (client, server) = tokio::io::duplex(STREAM_BUFFER);
        self.out_connections.send(server).ok()?;
        Some(client)
    }
}

/// A PDU read from an IN connection, or `None` once that connection closed.
type InEvent = (usize, Option<Vec<u8>>);

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

    fn fault_response(&mut self, call_id: u32, authenticated: bool) -> Result<Vec<u8>, crate::rpc::RpcPduError> {
        let fault = crate::rpc::encode_rpc_fault(crate::rpc::RpcFault {
            call_id,
            pfc_flags: PFC_FIRST_FRAG | PFC_LAST_FRAG,
            alloc_hint: 0,
            cancel_count: 0,
            reserved: 0,
            status: 5,
            reserved2: 0,
            stub: &[],
        })?;
        if !authenticated {
            return Ok(fault);
        }
        let mut pdu = prepare_rpc_authenticated_pdu(
            crate::rpc::PTYPE_FAULT,
            PFC_FIRST_FRAG | PFC_LAST_FRAG,
            call_id,
            &fault[RPC_COMMON_HEADER_SIZE..],
            self.authentication,
        )?
        .finish(&[0; 16])?;
        sign_server_fragment(&mut self.ntlm, &mut pdu, self.send_sequence);
        self.send_sequence = self.send_sequence.checked_add(1).expect("test sequence fits");
        Ok(pdu)
    }
}

type MockProxy = (
    DuplexStream,
    DuplexStream,
    MockRpchConnector,
    tokio::task::JoinHandle<()>,
);

pub(crate) fn mock_rpch_proxy(scenario: MockRpchScenario) -> MockProxy {
    spawn_proxy(scenario, None)
}

/// Starts the [`MockRpchScenario::ReauthConnection`] proxy, which sends the tunnel context it was asked to
/// reauthenticate to `report` once the client closes the reauthentication tunnel.
pub(crate) fn mock_rpch_reauth_proxy(report: mpsc::UnboundedSender<u64>) -> MockProxy {
    spawn_proxy(MockRpchScenario::ReauthConnection, Some(report))
}

fn spawn_proxy(scenario: MockRpchScenario, reauth_report: Option<mpsc::UnboundedSender<u64>>) -> MockProxy {
    let (out_client, out_server) = tokio::io::duplex(STREAM_BUFFER);
    let (in_client, in_server) = tokio::io::duplex(STREAM_BUFFER);
    let (in_connections, in_accept) = mpsc::unbounded_channel();
    let (out_connections, out_accept) = mpsc::unbounded_channel();
    let task = tokio::spawn(run_proxy(
        out_server,
        in_server,
        in_accept,
        out_accept,
        scenario,
        reauth_report,
    ));
    (
        out_client,
        in_client,
        MockRpchConnector {
            in_connections,
            out_connections,
        },
        task,
    )
}

fn spawn_in_reader<R>(id: usize, mut read: R, events: mpsc::UnboundedSender<InEvent>)
where
    R: AsyncRead + Send + Unpin + 'static,
{
    tokio::spawn(async move {
        loop {
            let pdu = read_pdu(&mut read).await;
            let closed = pdu.is_none();
            if events.send((id, pdu)).is_err() || closed {
                return;
            }
        }
    });
}

#[expect(
    clippy::too_many_lines,
    reason = "one scripted proxy keeps every scenario's sequence in one place"
)]
async fn run_proxy(
    mut out: DuplexStream,
    mut input: DuplexStream,
    mut in_accept: mpsc::UnboundedReceiver<DuplexStream>,
    mut out_accept: mpsc::UnboundedReceiver<DuplexStream>,
    scenario: MockRpchScenario,
    reauth_report: Option<mpsc::UnboundedSender<u64>>,
) {
    let requires_basic_auth = matches!(scenario, MockRpchScenario::BasicAuthentication);
    if !begin_in_channel(&mut input, requires_basic_auth, false).await {
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
    let Some(mut in_cookie) = complete_in_channel(&mut input).await else {
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

    let (in_read, mut in_write) = tokio::io::split(input);
    let (events_sender, mut events) = mpsc::unbounded_channel();
    spawn_in_reader(0, in_read, events_sender.clone());
    let mut active_in = 0;
    let mut next_in = 1;
    let mut successor_in: Option<(usize, RtsCookie)> = None;
    let mut deferred = VecDeque::new();
    let mut ready: VecDeque<InEvent> = VecDeque::new();
    let mut in_recycled = false;
    let mut out_recycled = false;

    let receive_pipe_result = match scenario {
        MockRpchScenario::ReceivePipeError(result) => Some(result),
        _ => None,
    };
    let mut receive_pipe_call_id = None;
    let mut receive_pipe_started = false;
    // Only RPC PDUs count against the receive window ([MS-RPCH] 3.2.1.1.4).
    let mut bytes_received = 0u32;
    let mut rpc_security = MockRpcSecurity::new();
    let message_call_fault = matches!(
        scenario,
        MockRpchScenario::MessageCallFault
            | MockRpchScenario::TamperedMessageCallFault
            | MockRpchScenario::UnsignedMessageCallFault
    );
    let interleaved_message = matches!(
        scenario,
        MockRpchScenario::InterleavedMessage | MockRpchScenario::PartialMessageFault
    );
    let messaging = matches!(
        scenario,
        MockRpchScenario::Messaging | MockRpchScenario::Reauth | MockRpchScenario::LargeConsent
    ) || message_call_fault
        || interleaved_message;
    let mut message_calls = 0u32;
    let mut pending_message_call = None;
    let mut message_call_cancelled = false;
    let mut reauth_tunnel_context = None;

    loop {
        let (id, pdu) = match ready.pop_front() {
            Some(event) => event,
            None => tokio::select! {
                event = events.recv() => match event {
                    Some(event) => event,
                    None => return,
                },
                Some(mut stream) = in_accept.recv() => {
                    let events = events_sender.clone();
                    let id = next_in;
                    next_in += 1;
                    tokio::spawn(async move {
                        if begin_in_channel(&mut stream, false, true).await {
                            spawn_in_reader(id, stream, events);
                        }
                    });
                    continue;
                }
            },
        };
        let Some(mut pdu) = pdu else {
            if id == active_in {
                return;
            }
            continue;
        };

        if id != active_in {
            match successor_in {
                None if is_in_recycle_a1(&pdu) => {
                    let Some(cookie) = pdu.get(72..88).and_then(|cookie| cookie.try_into().ok()) else {
                        return;
                    };
                    successor_in = Some((id, RtsCookie::new(cookie)));
                    // oracle: spec MS-RPCH §2.2.4.21: IN_R2/A4 contains only Destination(FDClient).
                    let response = if matches!(scenario, MockRpchScenario::RecycleInR2) {
                        vec![
                            5, 0, 20, 3, 16, 0, 0, 0, 28, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 13, 0, 0, 0, 0, 0, 0, 0,
                        ]
                    } else {
                        rts_in_recycle_a4(PEER_RECEIVE_WINDOW, 120_000)
                    };
                    if write_all(&mut out, &response).await.is_err() {
                        return;
                    }
                }
                Some((successor, _)) if successor == id => deferred.push_back((id, Some(pdu))),
                _ => return,
            }
            continue;
        }
        if let Some((successor, cookie)) = successor_in {
            if is_in_recycle_a5(&pdu, cookie) {
                active_in = successor;
                in_cookie = cookie;
                bytes_received = 0;
                successor_in = None;
                in_recycled = true;
                ready.extend(deferred.drain(..));
                continue;
            }
        }

        if pdu.get(2).copied() != Some(PTYPE_RTS) {
            let Ok(length) = u32::try_from(pdu.len()) else {
                return;
            };
            bytes_received = match bytes_received.checked_add(length) {
                Some(bytes_received) => bytes_received,
                None => return,
            };
        }

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
                let Some(stub) = body.get(8..) else {
                    return;
                };
                if opnum == TSPROXY_SETUP_RECEIVE_PIPE_OPNUM {
                    if matches!(scenario, MockRpchScenario::ReauthConnection) {
                        return;
                    }
                    receive_pipe_call_id = Some(request_call_id);
                    if matches!(scenario, MockRpchScenario::UnexpectedResponse) {
                        let response = rpc_security
                            .signed_response(0xffff, PFC_FIRST_FRAG, b"unsolicited")
                            .expect("response");
                        let _ = write_all(&mut out, &response).await;
                    }
                    if matches!(scenario, MockRpchScenario::InvalidReceivePipeFinal) {
                        let _ = write_response(&mut out, &mut rpc_security, request_call_id, &[0; 8]).await;
                    }
                    if let Some(result) = receive_pipe_result {
                        if write_response(&mut out, &mut rpc_security, request_call_id, &result.to_le_bytes())
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    continue;
                }

                match opnum {
                    TSPROXY_CREATE_TUNNEL_OPNUM => {
                        let reauth = (stub.get(..4) == Some(TSG_PACKET_TYPE_REAUTH.to_le_bytes().as_slice()))
                            .then(|| stub.get(16..24)?.try_into().ok().map(u64::from_le_bytes))
                            .flatten();
                        if matches!(scenario, MockRpchScenario::ReauthConnection) != reauth.is_some() {
                            return;
                        }
                        reauth_tunnel_context = reauth;
                        let response = if messaging {
                            // The client must advertise every messaging capability it handles.
                            if stub.get(stub.len().saturating_sub(4)..) != Some(0x1fu32.to_le_bytes().as_slice()) {
                                return;
                            }
                            let consent = if matches!(scenario, MockRpchScenario::LargeConsent) {
                                "c".repeat(20 * 1024)
                            } else {
                                CONSENT_MESSAGE.to_owned()
                            };
                            create_tunnel_caps_response(&TUNNEL_CONTEXT, 7, &consent)
                        } else {
                            create_tunnel_response(&TUNNEL_CONTEXT, 7)
                        };
                        if matches!(scenario, MockRpchScenario::TamperedResponse) {
                            let Ok(mut response) = rpc_security.signed_response(
                                request_call_id,
                                PFC_FIRST_FRAG | PFC_LAST_FRAG,
                                &response,
                            ) else {
                                return;
                            };
                            *response.last_mut().expect("response verifier") ^= 1;
                            if write_all(&mut out, &response).await.is_err() {
                                return;
                            }
                            continue;
                        }
                        if write_fragmented_response(&mut out, &mut rpc_security, request_call_id, &response)
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    TSPROXY_MAKE_TUNNEL_CALL_OPNUM => {
                        let Some(proc_id) = stub.get(20..24).and_then(|bytes| bytes.try_into().ok()) else {
                            return;
                        };
                        if !messaging {
                            return;
                        }
                        match u32::from_le_bytes(proc_id) {
                            TSG_TUNNEL_CALL_ASYNC_MSG_REQUEST => {
                                if pending_message_call.is_some() {
                                    return;
                                }
                                message_calls += 1;
                                if interleaved_message && message_calls == 1 {
                                    pending_message_call = Some(request_call_id);
                                    continue;
                                }
                                if message_call_fault {
                                    if message_calls != 1 {
                                        return;
                                    }
                                    let Ok(mut fault) = rpc_security.fault_response(
                                        request_call_id,
                                        !matches!(scenario, MockRpchScenario::UnsignedMessageCallFault),
                                    ) else {
                                        return;
                                    };
                                    if matches!(scenario, MockRpchScenario::TamperedMessageCallFault) {
                                        *fault.last_mut().expect("fault verifier") ^= 1;
                                    }
                                    if write_all(&mut out, &fault).await.is_err() {
                                        return;
                                    }
                                    continue;
                                }
                                if message_calls > 1 {
                                    pending_message_call = Some(request_call_id);
                                    continue;
                                }
                                let message = if matches!(scenario, MockRpchScenario::Reauth) {
                                    reauth_message_response(REAUTH_TUNNEL_CONTEXT)
                                } else {
                                    service_message_response(SERVICE_MESSAGE)
                                };
                                if write_response(&mut out, &mut rpc_security, request_call_id, &message)
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            TSG_TUNNEL_CANCEL_ASYNC_MSG_REQUEST => {
                                let Some(pending) = pending_message_call.take() else {
                                    return;
                                };
                                message_call_cancelled = true;
                                let cancelled = [0u32.to_le_bytes(), CANCELLED_HRESULT.to_le_bytes()].concat();
                                let accepted = [0u32.to_le_bytes(), 0u32.to_le_bytes()].concat();
                                if write_response(&mut out, &mut rpc_security, pending, &cancelled)
                                    .await
                                    .is_err()
                                    || write_response(&mut out, &mut rpc_security, request_call_id, &accepted)
                                        .await
                                        .is_err()
                                {
                                    return;
                                }
                            }
                            _ => return,
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
                    TSPROXY_CREATE_CHANNEL_OPNUM => {
                        let message = service_message_response(&"x".repeat(20 * 1024));
                        if interleaved_message {
                            let first = rpc_security
                                .signed_response(
                                    pending_message_call.expect("pending message"),
                                    PFC_FIRST_FRAG,
                                    &message[..1024],
                                )
                                .expect("first message fragment");
                            if write_all(&mut out, &first).await.is_err() {
                                return;
                            }
                            if matches!(scenario, MockRpchScenario::PartialMessageFault) {
                                let fault = rpc_security
                                    .fault_response(pending_message_call.take().expect("pending message"), true)
                                    .expect("signed fault");
                                if write_all(&mut out, &fault).await.is_err() {
                                    return;
                                }
                            }
                        }
                        let channel_context = if matches!(scenario, MockRpchScenario::ReauthConnection) {
                            [0; 20]
                        } else {
                            CHANNEL_CONTEXT
                        };
                        if write_response(
                            &mut out,
                            &mut rpc_security,
                            request_call_id,
                            &create_channel_response(&channel_context, 3),
                        )
                        .await
                        .is_err()
                        {
                            return;
                        }
                        if matches!(scenario, MockRpchScenario::InterleavedMessage) {
                            let call = pending_message_call.take().expect("pending message");
                            for (index, chunk) in message[1024..].chunks(1024).enumerate() {
                                let end = 1024 + (index + 1) * 1024;
                                let flags = if message.len() <= end { PFC_LAST_FRAG } else { 0 };
                                let response = rpc_security
                                    .signed_response(call, flags, chunk)
                                    .expect("message continuation");
                                if write_all(&mut out, &response).await.is_err() {
                                    return;
                                }
                            }
                        }
                        if matches!(scenario, MockRpchScenario::PartialMessageFault) {
                            message_call_cancelled = true;
                        }
                    }
                    TSPROXY_SEND_TO_SERVER_OPNUM => {
                        if matches!(scenario, MockRpchScenario::ReceivePipeOverflow) {
                            for index in 0..514 {
                                let flags = if index == 0 { PFC_FIRST_FRAG } else { 0 };
                                let response = rpc_security
                                    .signed_response(receive_pipe_call_id.expect("receive pipe"), flags, &[0; 2 * 1024])
                                    .expect("receive data");
                                if write_all(&mut out, &response).await.is_err() {
                                    return;
                                }
                            }
                            continue;
                        }
                        let Some(data) = send_to_server_data(&body) else {
                            return;
                        };
                        if let MockRpchScenario::InHttpError(code) = scenario {
                            let response = format!("HTTP/1.1 503 RPC Error: {code:x}\r\nContent-Length: 0\r\n\r\n");
                            let _ = in_write.write_all(response.as_bytes()).await;
                            continue;
                        }
                        let r2 = match scenario {
                            MockRpchScenario::RecycleOutR1 => Some(false),
                            MockRpchScenario::RecycleOutR2 => Some(true),
                            MockRpchScenario::RecycleOutDelayedHead | MockRpchScenario::RecycleOutRejectedHead => {
                                Some(false)
                            }
                            _ => None,
                        };
                        if let Some(r2) = r2.filter(|_| !out_recycled) {
                            out_recycled = true;
                            let Some(successor) =
                                recycle_out_channel(&mut out, &mut out_accept, &mut events, active_in, r2, scenario)
                                    .await
                            else {
                                return;
                            };
                            out = successor;
                        }
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
                        if opnum == TSPROXY_CLOSE_CHANNEL_OPNUM
                            && matches!(scenario, MockRpchScenario::ReauthConnection)
                        {
                            return;
                        }
                        // The scripted recycle scenarios must have run before the client closes.
                        if matches!(scenario, MockRpchScenario::RecycleIn | MockRpchScenario::RecycleInR2)
                            && !in_recycled
                        {
                            return;
                        }
                        // A pending message call must be cancelled before the tunnel closes ([MS-TSGU] 3.6.5).
                        if opnum == TSPROXY_CLOSE_TUNNEL_OPNUM {
                            if pending_message_call.is_some()
                                || (messaging && !message_call_fault && !message_call_cancelled)
                            {
                                return;
                            }
                            if let (Some(report), Some(tunnel_context)) = (&reauth_report, reauth_tunnel_context) {
                                let _ = report.send(tunnel_context);
                            }
                        }
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

/// Runs the proxy side of OUT channel recycling ([MS-RPCH] 3.2.2.5.6 through 3.2.2.5.10).
///
/// Returns the successor OUT connection once it is the default OUT channel.
async fn recycle_out_channel(
    predecessor: &mut DuplexStream,
    out_accept: &mut mpsc::UnboundedReceiver<DuplexStream>,
    events: &mut mpsc::UnboundedReceiver<InEvent>,
    active_in: usize,
    r2: bool,
    scenario: MockRpchScenario,
) -> Option<DuplexStream> {
    const RTS_FLAG_RECYCLE_CHANNEL: u16 = 0x0004;
    const RTS_FLAG_OUT_CHANNEL: u16 = 0x0010;
    const RTS_FLAG_EOF: u16 = 0x0020;

    let destination_client = [13u32.to_le_bytes(), 0u32.to_le_bytes()].concat();
    let ance = 10u32.to_le_bytes();
    write_all(
        predecessor,
        &rts_pdu_with_flags(RTS_FLAG_RECYCLE_CHANNEL, &destination_client, 1),
    )
    .await
    .ok()?;

    let mut successor = out_accept.recv().await?;
    let request = read_request_head(&mut successor).await?;
    if !request.starts_with("RPC_OUT_DATA /rpc/rpcproxy.dll?localhost:3388 ")
        || request_content_length(&request) != Some(120)
        || !request_has_cookie(&request)
    {
        return None;
    }
    let mut a3 = [0; 96];
    successor.read_exact(&mut a3).await.ok()?;
    let delayed = matches!(
        scenario,
        MockRpchScenario::RecycleOutDelayedHead | MockRpchScenario::RecycleOutRejectedHead
    );
    if !delayed {
        write_all(
            &mut successor,
            b"HTTP/1.1 200 OK\r\nContent-Type: application/rpc\r\nContent-Length: 1073741824\r\n\r\n",
        )
        .await
        .ok()?;
    }

    let (a6, a7_flags, a7_count) = if r2 {
        (
            rts_pdu_with_flags(0, &[destination_client.as_slice(), &ance].concat(), 2),
            RTS_FLAG_OUT_CHANNEL,
            3,
        )
    } else {
        let commands = [
            destination_client.as_slice(),
            &6u32.to_le_bytes(),
            &1u32.to_le_bytes(),
            &2u32.to_le_bytes(),
            &120_000u32.to_le_bytes(),
        ]
        .concat();
        (
            rts_pdu_with_flags(RTS_FLAG_OUT_CHANNEL, &commands, 3),
            RTS_FLAG_OUT_CHANNEL,
            2,
        )
    };
    write_all(predecessor, &a6).await.ok()?;
    loop {
        let (id, pdu) = events.recv().await?;
        let pdu = pdu?;
        if id != active_in || pdu.get(2).copied() != Some(PTYPE_RTS) {
            return None;
        }
        if rts_flags(&pdu) == Some(a7_flags) && rts_command_count(&pdu) == Some(a7_count) {
            break;
        }
    }

    let mut trailer = [0; 24];
    if r2 {
        successor.read_exact(&mut trailer).await.ok()?;
        write_all(predecessor, &rts_pdu_with_flags(RTS_FLAG_EOF, &ance, 1))
            .await
            .ok()?;
    } else {
        write_all(predecessor, &rts_pdu_with_flags(0, &ance, 1)).await.ok()?;
        successor.read_exact(&mut trailer).await.ok()?;
    }
    if delayed {
        tokio::time::sleep(core::time::Duration::from_millis(10)).await;
        let response: &[u8] = if matches!(scenario, MockRpchScenario::RecycleOutRejectedHead) {
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n"
        } else {
            b"HTTP/1.1 200 OK\r\nContent-Type: application/rpc\r\nContent-Length: 1073741824\r\nSet-Cookie: Rotated=successor\r\n\r\n"
        };
        write_all(&mut successor, response).await.ok()?;
    }
    Some(successor)
}

fn rts_flags(pdu: &[u8]) -> Option<u16> {
    pdu.get(16..18)?.try_into().ok().map(u16::from_le_bytes)
}

fn rts_command_count(pdu: &[u8]) -> Option<u16> {
    pdu.get(18..20)?.try_into().ok().map(u16::from_le_bytes)
}

fn is_in_recycle_a1(pdu: &[u8]) -> bool {
    pdu.get(2).copied() == Some(PTYPE_RTS) && rts_flags(pdu) == Some(0x0004) && rts_command_count(pdu) == Some(4)
}

fn is_in_recycle_a5(pdu: &[u8], successor: RtsCookie) -> bool {
    pdu.get(2).copied() == Some(PTYPE_RTS)
        && rts_flags(pdu) == Some(0)
        && rts_command_count(pdu) == Some(1)
        && pdu.get(20..24) == Some(3u32.to_le_bytes().as_slice())
        && pdu.get(24..40) == Some(successor.as_bytes().as_slice())
}

fn rts_in_recycle_a4(receive_window_size: u32, connection_timeout: u32) -> Vec<u8> {
    rts_pdu(
        &[
            [13, 0, 0, 0].as_slice(),
            0u32.to_le_bytes().as_slice(),
            [6, 0, 0, 0].as_slice(),
            1u32.to_le_bytes().as_slice(),
            [0, 0, 0, 0].as_slice(),
            receive_window_size.to_le_bytes().as_slice(),
            [2, 0, 0, 0].as_slice(),
            connection_timeout.to_le_bytes().as_slice(),
        ]
        .concat(),
        4,
    )
}

async fn begin_in_channel(input: &mut DuplexStream, requires_basic_auth: bool, probe_has_cookie: bool) -> bool {
    // The first leg is an empty authentication probe; the next one reserves the streaming body.
    let Some(request) = read_request_head(input).await else {
        return false;
    };
    if !valid_in_request(&request, false, false) || request_has_cookie(&request) != probe_has_cookie {
        return false;
    }
    let probe_response = if requires_basic_auth {
        format!(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nWWW-Authenticate: Basic\r\nSet-Cookie: {LB_COOKIE}; Path=/; HttpOnly\r\n\r\n"
        )
    } else {
        format!("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nSet-Cookie: {LB_COOKIE}; Path=/; HttpOnly\r\n\r\n")
    };
    if write_all(input, probe_response.as_bytes()).await.is_err() {
        return false;
    }

    let Some(request) = read_request_head(input).await else {
        return false;
    };
    valid_in_request(&request, true, requires_basic_auth)
        && request_has_cookie(&request)
        && write_all(input, b"HTTP/1.1 100 Continue\r\n\r\n").await.is_ok()
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

async fn read_request_head<S: AsyncRead + Unpin>(stream: &mut S) -> Option<String> {
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

fn valid_in_request(request: &str, reserves_body: bool, has_basic_auth: bool) -> bool {
    let valid_length = if reserves_body {
        request_content_length(request).is_some_and(|length| length >= 128 * 1024)
    } else {
        request_content_length(request) == Some(0)
    };
    request.starts_with("RPC_IN_DATA /rpc/rpcproxy.dll?localhost:3388 ")
        && valid_length
        && request.contains("Expect: 100-continue") == reserves_body
        && request_has_basic_auth(request) == has_basic_auth
}

fn valid_out_request(request: &str, has_basic_auth: bool) -> bool {
    request.starts_with("RPC_OUT_DATA /rpc/rpcproxy.dll?localhost:3388 ")
        && request_content_length(request) == Some(76)
        && request_has_basic_auth(request) == has_basic_auth
        && request_has_cookie(request)
}

fn request_has_cookie(request: &str) -> bool {
    request.split("\r\n").any(|line| line == format!("Cookie: {LB_COOKIE}"))
}

async fn read_http_head<S: AsyncRead + Unpin>(stream: &mut S) -> Option<Vec<u8>> {
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

async fn read_pdu<S: AsyncRead + Unpin>(stream: &mut S) -> Option<Vec<u8>> {
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
    let chunk_size = (stub.len() / 2).clamp(1, 2048);
    for (index, chunk) in stub.chunks(chunk_size).enumerate() {
        let mut flags = if index == 0 { PFC_FIRST_FRAG } else { 0 };
        if stub.len() <= (index + 1) * chunk_size {
            flags |= PFC_LAST_FRAG;
        }
        let response = security
            .signed_response(call_id, flags, chunk)
            .expect("test response fits");
        write_all(stream, &response).await?;
    }
    Ok(())
}

fn send_to_server_data(body: &[u8]) -> Option<&[u8]> {
    let buffer_length = usize::try_from(u32::from_be_bytes(body.get(36..40)?.try_into().ok()?)).ok()?;
    body.get(40..40usize.checked_add(buffer_length)?)
}

fn call_id(pdu: &[u8]) -> u32 {
    u32::from_le_bytes(pdu[12..16].try_into().expect("complete DCE/RPC header"))
}

async fn write_all<S: AsyncWrite + Unpin>(stream: &mut S, bytes: &[u8]) -> tokio::io::Result<()> {
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
    rts_pdu_with_flags(0, commands, command_count)
}

fn rts_pdu_with_flags(flags: u16, commands: &[u8], command_count: u16) -> Vec<u8> {
    let mut pdu = vec![5, 0, PTYPE_RTS, 3, 0x10, 0, 0, 0];
    pdu.extend_from_slice(&u16::try_from(20 + commands.len()).expect("test RTS fits").to_le_bytes());
    pdu.extend_from_slice(&0u16.to_le_bytes());
    pdu.extend_from_slice(&0u32.to_le_bytes());
    pdu.extend_from_slice(&flags.to_le_bytes());
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

/// `TSG_PACKET_CAPS_RESPONSE` negotiating every messaging capability and carrying [`CONSENT_MESSAGE`]
/// ([MS-TSGU] 2.2.9.2.1.6).
fn create_tunnel_caps_response(tunnel_context: &[u8; 20], tunnel_id: u32, consent: &str) -> Vec<u8> {
    const TSG_PACKET_TYPE_CAPS_RESPONSE: u32 = 0x0000_4350;
    const TSG_ASYNC_MESSAGE_CONSENT: u32 = 1;
    [
        0x0002_0000u32.to_le_bytes().as_slice(),
        TSG_PACKET_TYPE_CAPS_RESPONSE.to_le_bytes().as_slice(),
        TSG_PACKET_TYPE_CAPS_RESPONSE.to_le_bytes().as_slice(),
        0x0002_0004u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
        &[0x5a; 16],
        0x0002_0008u32.to_le_bytes().as_slice(),
        1u32.to_le_bytes().as_slice(),
        TSG_ASYNC_MESSAGE_CONSENT.to_le_bytes().as_slice(),
        1u32.to_le_bytes().as_slice(),
        TSG_ASYNC_MESSAGE_CONSENT.to_le_bytes().as_slice(),
        0x0002_000cu32.to_le_bytes().as_slice(),
        0x5452u16.to_le_bytes().as_slice(),
        0x5643u16.to_le_bytes().as_slice(),
        0x0002_0010u32.to_le_bytes().as_slice(),
        1u32.to_le_bytes().as_slice(),
        1u16.to_le_bytes().as_slice(),
        1u16.to_le_bytes().as_slice(),
        0u16.to_le_bytes().as_slice(),
        0u16.to_le_bytes().as_slice(),
        1u32.to_le_bytes().as_slice(),
        1u32.to_le_bytes().as_slice(),
        1u32.to_le_bytes().as_slice(),
        0x1fu32.to_le_bytes().as_slice(),
        &string_message(consent, 0x0002_0014),
        tunnel_context,
        tunnel_id.to_le_bytes().as_slice(),
        0u32.to_le_bytes().as_slice(),
    ]
    .concat()
}

/// `TSG_PACKET_STRING_MESSAGE` referent and its deferred, 4-byte padded string ([MS-TSGU] 2.2.9.2.1.9.1.1).
fn string_message(text: &str, referent: u32) -> Vec<u8> {
    let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let count = u32::try_from(units.len()).expect("test message fits");
    let mut output = [
        1u32.to_le_bytes(),
        0u32.to_le_bytes(),
        count.to_le_bytes(),
        referent.to_le_bytes(),
        count.to_le_bytes(),
        0u32.to_le_bytes(),
        count.to_le_bytes(),
    ]
    .concat();
    output.extend(units.iter().flat_map(|unit| unit.to_le_bytes()));
    output.resize(output.len().next_multiple_of(4), 0);
    output
}

/// `TsProxyMakeTunnelCall` response header for a present message of `message_type` ([MS-TSGU] 2.2.9.2.1.9).
fn message_response_header(message_type: u32) -> Vec<u8> {
    const TSG_PACKET_TYPE_MESSAGE_PACKET: u32 = 0x0000_4750;
    [
        0x0002_0000u32.to_le_bytes(),
        TSG_PACKET_TYPE_MESSAGE_PACKET.to_le_bytes(),
        TSG_PACKET_TYPE_MESSAGE_PACKET.to_le_bytes(),
        0x0002_0004u32.to_le_bytes(),
        1u32.to_le_bytes(),
        message_type.to_le_bytes(),
        1u32.to_le_bytes(),
        message_type.to_le_bytes(),
        0x0002_0008u32.to_le_bytes(),
    ]
    .concat()
}

fn service_message_response(text: &str) -> Vec<u8> {
    const TSG_ASYNC_MESSAGE_SERVICE: u32 = 2;
    [
        message_response_header(TSG_ASYNC_MESSAGE_SERVICE),
        string_message(text, 0x0002_000c),
        0u32.to_le_bytes().to_vec(),
    ]
    .concat()
}

/// A reauthentication message; its 8-byte aligned `TSG_PACKET_REAUTH_MESSAGE` follows 4 bytes of padding.
fn reauth_message_response(tunnel_context: u64) -> Vec<u8> {
    const TSG_ASYNC_MESSAGE_REAUTH: u32 = 3;
    [
        message_response_header(TSG_ASYNC_MESSAGE_REAUTH),
        0u32.to_le_bytes().to_vec(),
        tunnel_context.to_le_bytes().to_vec(),
        0u32.to_le_bytes().to_vec(),
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
