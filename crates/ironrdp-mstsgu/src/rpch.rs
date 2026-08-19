//! MS-RPCH v2 client driver for the legacy MS-TSGU RPC-over-HTTP transport.
//!
//! Sequences the IN/OUT channel setup ([MS-RPCH] 3.2.2), the DCE/RPC bind with
//! optional NTLM packet integrity, and the TsProxy tunnel/channel calls
//! ([MS-TSGU] 3.2.6.1) over caller-supplied streams. Streams are generic so the
//! mock server can drive the whole exchange in memory; production wiring uses
//! TLS streams to the RPC proxy.

use core::time::Duration;

use futures_util::FutureExt as _;
use hyper::body::Bytes;
use log::{debug, error, warn};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use uuid::Uuid;

use crate::http_auth::{AuthStep, GatewayHttpAuth};
use crate::rpc::{
    NonNullRpcContextHandle, RpcFragmentSizes, RpcHttpV2FlowControl, RpcHttpV2PingSchedule, RpcHttpV2Settings,
    RpcHttpV2Setup, RpcHttpV2State, RpcNtlmAuth, RpcResponseReassembler, RtsCookie, TSPROXY_AUTHORIZE_TUNNEL_OPNUM,
    TSPROXY_CREATE_CHANNEL_OPNUM, TSPROXY_CREATE_TUNNEL_OPNUM, TSPROXY_MAKE_TUNNEL_CALL_OPNUM,
    TSPROXY_SEND_TO_SERVER_OPNUM, TSPROXY_SETUP_RECEIVE_PIPE_OPNUM, TsProxyAuthorizeTunnelRequest,
    TsProxyCreateChannelRequest, TsProxyCreateTunnelRequest, TsProxyCreateTunnelResponse, TsProxyMakeTunnelCallRequest,
    TsProxySendToServerRequest, TsProxySetupReceivePipeRequest, TsProxyTunnelMessage, decode_rpc_response_fragment,
    decode_rpc_response_fragment_with_ntlm_auth, decode_rts_flow_control_ack, decode_rts_ping,
    decode_tsgu_authorize_tunnel_response, decode_tsgu_bind_ack, decode_tsgu_bind_ack_with_ntlm_auth,
    decode_tsgu_create_channel_response, decode_tsgu_create_tunnel_response, decode_tsgu_make_tunnel_call_response,
    encode_rpc_auth_3, encode_rpc_request_fragments, encode_rpc_request_fragments_with_ntlm_auth,
    encode_rts_flow_control_ack, encode_rts_ping, encode_tsgu_bind, encode_tsgu_bind_with_ntlm_auth,
};
use crate::rpc_transport::{
    RpchInRequest, RpchRequestHead, drain_body, read_rpch_response_head, write_rpch_request_head,
};
use crate::{Error, GwConnectTarget, GwErrorKind};

/// DCE/RPC common header size prefixing every PDU on the wire.
const RPC_COMMON_HEADER: usize = 16;

/// Maximum accepted PDU size on the OUT channel.
const MAX_OUT_PDU: usize = 1024 * 1024;

/// Maximum reassembled response stub size ([MS-RPCE] stub sanity bound).
const MAX_RESPONSE_STUB: usize = 32_767;

const PTYPE_RESPONSE: u8 = 2;
const PTYPE_FAULT: u8 = 3;
const PTYPE_RTS: u8 = 20;

/// TsProxy capability bits advertised in `TsProxyCreateTunnel` ([MS-TSGU] 2.2.9.2.1.1).
const TSG_CAPS: u32 = 0x1 /* quarantine SoH */
    | 0x2 /* idle timeout */
    | 0x4 /* consent message */
    | 0x8 /* service message */
    | 0x10 /* reauthentication */;

/// An established RPCH session: RTS CONN sequence complete and the TsProxy
/// interface bound. Drives TsProxy calls and the RDP data plane.
pub(crate) struct RpchSession<S> {
    out: OutChannel<S>,
    input: RpchInRequest<S>,
    flow_control: RpcHttpV2FlowControl,
    ping: RpcHttpV2PingSchedule,
    fragment_sizes: RpcFragmentSizes,
    rpc_auth: Option<RpcNtlmAuth>,
    /// Signature sequence number for PDUs written to the IN channel.
    send_sequence: u32,
    /// Signature sequence number for PDUs read from the OUT channel.
    receive_sequence: u32,
    responses: RpcResponseReassembler,
    call_id: u32,
    /// Call ID of the pending asynchronous administrative-message request.
    tunnel_message_call_id: Option<u32>,
    /// Call ID of `TsProxySetupReceivePipe`, whose streaming responses carry target-server
    /// data once the tunnel is up ([MS-TSGU] 3.2.6.2.3).
    receive_pipe_call_id: Option<u32>,
    /// Administrative messages received while a TsProxy call was in flight.
    messages: std::collections::VecDeque<TsProxyTunnelMessage>,
    tunnel_context: Option<NonNullRpcContextHandle>,
    channel_context: Option<NonNullRpcContextHandle>,
}

/// Open the RPCH session over two already-connected streams to the RPC proxy.
///
/// `out_stream` carries `RPC_OUT_DATA` (server→client PDUs) and `in_stream` carries
/// `RPC_IN_DATA` (client→server PDUs). Production callers pass separate TLS streams.
/// `rpc_auth` enables NTLM packet integrity; the mock server path leaves it `None`.
#[expect(clippy::too_many_lines, reason = "linear channel and RTS setup sequence")]
pub(crate) async fn rpch_connect<S>(
    mut out_stream: S,
    in_stream: S,
    gateway_host: &str,
    target: &GwConnectTarget,
    settings: RpcHttpV2Settings,
    rpc_auth: Option<RpcNtlmAuth>,
) -> Result<RpchSession<S>, Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // The HTTP-level SessionId on the OUT channel must equal the RTS association group id
    // carried in the IN channel's CONN/B1, or the gateway cannot pair the channels. The
    // cookie crosses the wire in little-endian GUID form ([MS-RPCH] 2.2.4.3).
    let association_group_id = Uuid::new_v4();
    let session_id = association_group_id.to_string();
    let mut setup = RpcHttpV2Setup::with_cookies(
        settings,
        RtsCookie::new(*Uuid::new_v4().as_bytes()),
        RtsCookie::new(*Uuid::new_v4().as_bytes()),
        RtsCookie::new(*Uuid::new_v4().as_bytes()),
        RtsCookie::new(association_group_id.to_bytes_le()),
    );
    let mut http_auth: Option<GatewayHttpAuth> = None;
    let mut authorization: Option<String> = None;
    let proxy_target = rpc_proxy_target(target);
    let spn = format!("HTTP/{gateway_host}");

    // Generate both connection PDUs up front; the wire order below sends CONN/B1 on the
    // IN channel before CONN/A1 on the OUT channel, matching mstsc ([MS-RPCH] 3.2.2.2).
    debug!("RPCH connect: opening channels to {proxy_target} via {gateway_host}");
    setup
        .start_in_request()
        .map_err(|e| custom_err!("rpch out request state", e))?;
    let out_body = setup
        .out_request_body()
        .map_err(|e| custom_err!("rpch CONN/A1 encode", e))?;
    let in_content_length = setup.in_channel_content_length();
    let initial_pdu = setup
        .in_request_initial_pdu()
        .map_err(|e| custom_err!("rpch CONN/B1 encode", e))?;
    let out_content_length =
        u32::try_from(out_body.len()).map_err(|_| Error::new("rpch out body length", GwErrorKind::Encode))?;
    const MAX_AUTH_ROUNDS: usize = 8;

    // IN channel first: the gateway pairs the channels before answering the OUT channel.
    // Authentication probes carry no body (Content-Length: 0); the streaming body is
    // committed only on the request that answers the challenge, matching mstsc.
    // NTLM tokens are bound to their connection, so the IN channel authenticates
    // independently on its own connection.
    debug!("RPCH connect: opening IN channel");
    let mut in_auth: Option<GatewayHttpAuth> = None;
    let mut in_authorization: Option<String> = None;
    let mut input = RpchInRequest::open(in_stream, gateway_host, &proxy_target, None).await?;
    let mut in_rounds = 0;
    let mut input = loop {
        in_rounds += 1;
        if in_rounds > MAX_AUTH_ROUNDS {
            return Err(Error::new("rpch in auth rounds exceeded", GwErrorKind::Connect));
        }
        let response = input
            .receive_response()
            .await
            .map_err(|e| custom_err!("rpch in response", e))?;
        debug!("RPCH IN channel: response status {}", response.status);
        match response.status {
            401 => {
                let final_round = challenge_has_token(&response.www_authenticate);
                in_authorization = Some(step_gateway_auth(
                    &mut in_auth,
                    target,
                    &spn,
                    in_authorization.as_deref(),
                    &response.www_authenticate,
                )?);
                let content_length = if final_round { in_content_length } else { 0 };
                input = input
                    .retry(in_authorization.as_deref(), content_length)
                    .await
                    .map_err(|e| custom_err!("rpch in retry", e))?;
                if final_round {
                    break input;
                }
            }
            // A gateway that accepts the probe without authentication still needs the real
            // request carrying the channel-lifetime length before the body is written.
            200 => {
                input = input
                    .retry(None, in_content_length)
                    .await
                    .map_err(|e| custom_err!("rpch in retry", e))?;
                break input;
            }
            status => {
                return Err(Error::new("rpch in channel rejected", GwErrorKind::HttpStatus(status)));
            }
        }
    };
    input
        .write_body(&initial_pdu)
        .await
        .map_err(|e| custom_err!("rpch CONN/B1", e))?;
    // Ensure CONN/B1 reaches the gateway before the OUT channel's CONN/A1 so the virtual
    // connection can be paired.
    input.flush().await.map_err(|e| custom_err!("rpch CONN/B1 flush", e))?;

    // OUT channel: CONN/A1 body is small and buffered, so committing it on the final
    // authenticated request is safe. The 200 response streams the server-to-client PDUs.
    debug!("RPCH connect: IN channel streaming; opening OUT channel");
    let mut commit_body = false;
    let mut out_rounds = 0;
    let out_response = loop {
        out_rounds += 1;
        if out_rounds > MAX_AUTH_ROUNDS {
            return Err(Error::new("rpch out auth rounds exceeded", GwErrorKind::Connect));
        }
        let content_length = if commit_body { out_content_length } else { 0 };
        write_rpch_request_head(
            &mut out_stream,
            RpchRequestHead {
                method: "RPC_OUT_DATA",
                host: gateway_host,
                target: &proxy_target,
                content_length,
                authorization: authorization.as_deref(),
                cookie: None,
                session_id: Some(&session_id),
                expect_continue: false,
            },
        )
        .await?;
        if commit_body {
            out_stream
                .write_all(&out_body)
                .await
                .map_err(|e| custom_err!("rpch out body", e))?;
        }
        out_stream.flush().await.map_err(|e| custom_err!("rpch out flush", e))?;

        let response = read_rpch_response_head(&mut out_stream).await?;
        debug!("RPCH OUT response status {}", response.status);
        if response.status == 200 && !commit_body {
            // The gateway accepted the request without authentication; the probe carried
            // no body, so reissue the request committing CONN/A1.
            commit_body = true;
            continue;
        }
        if response.status == 401 {
            // The 401 carries an error body that must be drained before the request is
            // retried on this connection.
            if let Some(length) = response.content_length {
                drain_body(&mut out_stream, length)
                    .await
                    .map_err(|e| custom_err!("drain rpch out 401 body", e))?;
            }
            // A challenge bearing a token (the NTLM type-2) is answered with the final
            // type-3; that request commits the body.
            commit_body = challenge_has_token(&response.www_authenticate);
            authorization = Some(step_gateway_auth(
                &mut http_auth,
                target,
                &spn,
                authorization.as_deref(),
                &response.www_authenticate,
            )?);
            continue;
        }
        break response;
    };

    setup
        .accept_out_response(
            out_response.status,
            out_response.content_type.as_deref(),
            out_response.content_length,
        )
        .map_err(|e| custom_err!("rpch out response", e))?;

    // CONN/A3 then CONN/C2 arrive on the OUT channel body. The gateway may interleave
    // flow-control or ping RTS PDUs, which are not part of the handshake and are skipped.
    debug!("RPCH connect: IN channel gated; awaiting CONN/A3 + CONN/C2");
    let mut out = OutChannel::new(out_stream);
    while setup.state() != RpcHttpV2State::Open {
        let pdu = out.read_pdu().await?;
        // Skip flow-control acknowledgements and pings that arrive before the handshake
        // completes; they carry no setup state.
        if decode_rts_flow_control_ack(&pdu).is_ok() || decode_rts_ping(&pdu).is_ok() {
            debug!("RPCH connect: skipping a flow-control/ping RTS PDU during setup");
            continue;
        }
        setup
            .receive_out_pdu(&pdu)
            .map_err(|e| custom_err!("rpch setup pdu", e))?;
    }
    debug!("RPCH connect: CONN sequence complete; binding TsProxy interface");

    let flow_control = setup.flow_control().map_err(|e| custom_err!("rpch flow control", e))?;
    let ping = setup
        .ping_schedule(Duration::ZERO)
        .map_err(|e| custom_err!("rpch ping schedule", e))?;

    let mut session = RpchSession {
        out,
        input,
        flow_control,
        ping,
        fragment_sizes: RpcFragmentSizes::DEFAULT,
        rpc_auth,
        send_sequence: 0,
        receive_sequence: 0,
        responses: RpcResponseReassembler::new(MAX_RESPONSE_STUB),
        call_id: 0,
        tunnel_message_call_id: None,
        receive_pipe_call_id: None,
        messages: std::collections::VecDeque::new(),
        tunnel_context: None,
        channel_context: None,
    };
    session.bind().await?;
    Ok(session)
}

impl<S> RpchSession<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Runs the TsProxy tunnel/channel setup sequence ([MS-TSGU] 3.2.6.1).
    pub(crate) async fn open_tunnel(&mut self, client_name: &str, resource: &str, port: u16) -> Result<(), Error> {
        // TsProxyCreateTunnel.
        let request = TsProxyCreateTunnelRequest::new(TSG_CAPS).encode();
        let stub = self.call(TSPROXY_CREATE_TUNNEL_OPNUM, &request).await?;
        let tunnel: TsProxyCreateTunnelResponse =
            decode_tsgu_create_tunnel_response(&stub).map_err(|e| custom_err!("decode create tunnel", e))?;
        self.tunnel_context = Some(tunnel.tunnel_context);
        debug!("RPCH tunnel created (id {})", tunnel.tunnel_id);

        // TsProxyAuthorizeTunnel with an empty statement of health (NAP not enforced).
        let request = TsProxyAuthorizeTunnelRequest::new(tunnel.tunnel_context, client_name, &[])
            .encode()
            .map_err(|e| custom_err!("encode authorize tunnel", e))?;
        let stub = self.call(TSPROXY_AUTHORIZE_TUNNEL_OPNUM, &request).await?;
        decode_tsgu_authorize_tunnel_response(&stub).map_err(|e| custom_err!("decode authorize tunnel", e))?;

        // Queue the asynchronous administrative-message call ([MS-TSGU] 3.2.6.1.3).
        let request = TsProxyMakeTunnelCallRequest::new(tunnel.tunnel_context).encode();
        let call_id = self.next_call_id();
        self.send_call(call_id, TSPROXY_MAKE_TUNNEL_CALL_OPNUM, &request)
            .await?;
        self.tunnel_message_call_id = Some(call_id);

        // TsProxyCreateChannel for the target resource.
        let request = TsProxyCreateChannelRequest::new(tunnel.tunnel_context, resource, port)
            .encode()
            .map_err(|e| custom_err!("encode create channel", e))?;
        let stub = self.call(TSPROXY_CREATE_CHANNEL_OPNUM, &request).await?;
        let channel =
            decode_tsgu_create_channel_response(&stub).map_err(|e| custom_err!("decode create channel", e))?;
        self.channel_context = Some(channel.channel_context);
        debug!("RPCH channel created (id {})", channel.channel_id);

        // TsProxySetupReceivePipe switches the OUT channel to target-server data. The
        // server streams the RDP byte stream as response fragments on this call; its
        // return value is sent only as the final fragment when the pipe closes
        // ([MS-TSGU] 3.2.6.2.3). Do not wait for a response here: on success there is no
        // discrete reply, so read_data surfaces the data and any terminal error.
        let mut request = [0u8; TsProxySetupReceivePipeRequest::SIZE];
        TsProxySetupReceivePipeRequest::new(channel.channel_context)
            .encode_into(&mut request)
            .map_err(|e| custom_err!("encode setup receive pipe", e))?;
        let call_id = self.next_call_id();
        self.send_call(call_id, TSPROXY_SETUP_RECEIVE_PIPE_OPNUM, &request)
            .await?;
        self.receive_pipe_call_id = Some(call_id);
        Ok(())
    }

    /// Sends target-server-bound bytes through `TsProxySendToServer` ([MS-TSGU] 3.2.6.2.1).
    pub(crate) async fn send_to_server(&mut self, data: &[u8]) -> Result<(), Error> {
        let channel_context = self
            .channel_context
            .ok_or_else(|| Error::new("rpch send before channel create", GwErrorKind::Connect))?;
        let request =
            TsProxySendToServerRequest::new(channel_context, data).map_err(|e| custom_err!("send to server", e))?;
        let stub = request.encode().map_err(|e| custom_err!("send to server", e))?;
        let call_id = self.next_call_id();
        self.send_call(call_id, TSPROXY_SEND_TO_SERVER_OPNUM, &stub).await
    }

    /// Reads the next event from the OUT channel.
    ///
    /// After `TsProxySetupReceivePipe`, response stubs on the OUT channel carry raw
    /// target-server bytes. RTS control PDUs (pings, flow-control acknowledgements)
    /// are handled internally, as are administrative tunnel messages.
    pub(crate) async fn read_data(&mut self) -> Result<RpchRead, Error> {
        if let Some(message) = self.messages.pop_front() {
            return Ok(RpchRead::Message(message));
        }
        loop {
            let pdu = self.out.read_pdu().await?;
            self.flow_control
                .received_rpc_pdu(pdu.len())
                .map_err(|e| custom_err!("rpch receive window", e))?;

            match pdu.get(2).copied() {
                Some(PTYPE_RTS) => {
                    self.handle_rts_pdu(&pdu).await?;
                }
                Some(PTYPE_FAULT) => {
                    return Err(Error::new("rpc fault on receive pipe", GwErrorKind::Connect));
                }
                Some(PTYPE_RESPONSE) => {
                    let response = self.decode_response(&pdu)?;
                    // The pending make-tunnel-call completes with administrative messages.
                    if self.tunnel_message_call_id == Some(response.call_id) {
                        self.tunnel_message_call_id = None;
                        let stub = response.stub.to_vec();
                        self.consume_out_pdu(pdu.len()).await?;
                        self.handle_tunnel_message(&stub).await?;
                        if let Some(message) = self.messages.pop_front() {
                            return Ok(RpchRead::Message(message));
                        }
                        continue;
                    }
                    if self.receive_pipe_call_id == Some(response.call_id) {
                        // The receive pipe's final fragment carries only the 4-byte return
                        // value; a non-zero value is a gateway error ([MS-TSGU] 3.2.6.2.3).
                        // All other fragments on this call are the target-server byte stream.
                        if response.is_last_fragment() && response.stub.len() == 4 {
                            let result = crate::rpc::parse_receive_pipe_final_return_value_stub(
                                response.stub,
                                crate::rpc::RpcStubByteOrder::LittleEndian,
                            )
                            .map_err(|e| custom_err!("decode receive pipe result", e))?;
                            if result != 0 {
                                return Err(Error::new("receive pipe failed", GwErrorKind::GatewayCode(result)));
                            }
                            return Err(Error::new("receive pipe closed", GwErrorKind::Connect));
                        }
                        self.consume_out_pdu(pdu.len()).await?;
                        return Ok(RpchRead::Data(Bytes::copy_from_slice(response.stub)));
                    }
                    // A response on any other call (such as a TsProxySendToServer
                    // acknowledgement) carries a 4-byte return value; surface a non-zero
                    // code and otherwise ignore it so it is not mistaken for target data.
                    if response.is_last_fragment() && response.stub.len() == 4 {
                        let result = crate::rpc::parse_receive_pipe_final_return_value_stub(
                            response.stub,
                            crate::rpc::RpcStubByteOrder::LittleEndian,
                        )
                        .map_err(|e| custom_err!("decode rpc call result", e))?;
                        self.consume_out_pdu(pdu.len()).await?;
                        if result != 0 {
                            return Err(Error::new("rpc call failed", GwErrorKind::GatewayCode(result)));
                        }
                        continue;
                    }
                    self.consume_out_pdu(pdu.len()).await?;
                    continue;
                }
                _ => {
                    return Err(Error::new("unexpected rpch out pdu", GwErrorKind::UnexpectedPacket));
                }
            }
        }
    }

    /// Returns whether a PING must be sent now ([MS-RPCH] 3.2.1.2.1).
    pub(crate) fn ping_due(&self, now: Duration) -> bool {
        self.ping.ping_due(now)
    }

    /// Sends a PING on the IN channel.
    pub(crate) async fn send_ping(&mut self, now: Duration) -> Result<(), Error> {
        let pdu = encode_rts_ping().map_err(|e| custom_err!("encode rpch ping", e))?;
        self.write_in(&pdu).await?;
        self.ping.record_send(now);
        Ok(())
    }

    /// DCE/RPC bind of the TsProxy interface, with optional NTLM packet integrity.
    async fn bind(&mut self) -> Result<(), Error> {
        let call_id = self.next_call_id();
        // Take the auth state out so the NTLM leg and channel writes can both borrow self.
        match self.rpc_auth.take() {
            Some(mut auth) => {
                let type1 = auth.initial_token()?;
                let bind = encode_tsgu_bind_with_ntlm_auth(call_id, self.fragment_sizes, &type1)
                    .map_err(|e| custom_err!("encode authenticated bind", e))?;
                self.write_in(&bind).await?;

                let ack_pdu = self.out.read_pdu().await?;
                let ack = decode_tsgu_bind_ack_with_ntlm_auth(&ack_pdu, self.fragment_sizes)
                    .map_err(|e| custom_err!("decode bind ack", e))?;
                let type3 = auth.continue_token(ack.token())?;
                let auth3 = encode_rpc_auth_3(call_id, self.fragment_sizes, &type3)
                    .map_err(|e| custom_err!("encode rpc auth3", e))?;
                self.write_in(&auth3).await?;
                self.fragment_sizes = ack.binding().fragment_sizes();
                self.rpc_auth = Some(auth);
            }
            None => {
                let bind = encode_tsgu_bind(call_id, self.fragment_sizes).map_err(|e| custom_err!("encode bind", e))?;
                self.write_in(&bind).await?;

                let ack_pdu = self.out.read_pdu().await?;
                let binding = decode_tsgu_bind_ack(&ack_pdu, self.fragment_sizes)
                    .map_err(|e| custom_err!("decode bind ack", e))?;
                self.fragment_sizes = binding.fragment_sizes();
            }
        }
        Ok(())
    }

    /// Drives one synchronous TsProxy call and returns the reassembled response stub.
    async fn call(&mut self, opnum: u16, stub: &[u8]) -> Result<Vec<u8>, Error> {
        let call_id = self.next_call_id();
        self.send_call(call_id, opnum, stub).await?;
        self.receive_response(call_id).await
    }

    /// Reads and reassembles the response for an already-sent call.
    async fn receive_response(&mut self, call_id: u32) -> Result<Vec<u8>, Error> {
        loop {
            let pdu = self.out.read_pdu().await?;
            self.flow_control
                .received_rpc_pdu(pdu.len())
                .map_err(|e| custom_err!("rpch receive window", e))?;

            match pdu.get(2).copied() {
                Some(PTYPE_RTS) => self.handle_rts_pdu(&pdu).await?,
                Some(PTYPE_FAULT) => {
                    let fault_call_id = pdu.get(12..16).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                    if fault_call_id.is_some() && fault_call_id == self.tunnel_message_call_id {
                        // The asynchronous administrative-message call faulted; the main
                        // channel flow can proceed without it.
                        warn!("RPCH asynchronous tunnel-message call faulted");
                        self.tunnel_message_call_id = None;
                        self.consume_out_pdu(pdu.len()).await?;
                        continue;
                    }
                    return Err(Error::new("rpc fault in tsproxy call", GwErrorKind::Connect));
                }
                Some(PTYPE_RESPONSE) => {
                    let response = self.decode_response(&pdu)?;
                    let pushed = self
                        .responses
                        .push(response)
                        .map_err(|e| custom_err!("reassemble rpc response", e))?;
                    if let Some(assembled) = pushed {
                        if self.tunnel_message_call_id == Some(assembled.call_id) {
                            // An administrative message completed the pending tunnel call;
                            // log it and queue the next pending call ([MS-TSGU] 3.2.6.1.3).
                            self.tunnel_message_call_id = None;
                            self.handle_tunnel_message(&assembled.stub).await?;
                            self.consume_out_pdu(pdu.len()).await?;
                            continue;
                        }
                        if assembled.call_id != call_id {
                            return Err(Error::new("tsproxy response call id", GwErrorKind::UnexpectedPacket));
                        }
                        self.consume_out_pdu(pdu.len()).await?;
                        return Ok(assembled.stub);
                    }
                }
                _ => return Err(Error::new("unexpected rpch out pdu", GwErrorKind::UnexpectedPacket)),
            }
        }
    }

    /// Decodes one response PDU, verifying the NTLM signature when bound with auth.
    fn decode_response<'a>(&mut self, pdu: &'a [u8]) -> Result<crate::rpc::RpcResponse<'a>, Error> {
        match self.rpc_auth.as_mut() {
            Some(auth) => {
                let response = decode_rpc_response_fragment_with_ntlm_auth(
                    auth,
                    self.receive_sequence,
                    pdu,
                    self.fragment_sizes.max_recv(),
                )?;
                self.receive_sequence = self.receive_sequence.wrapping_add(1);
                Ok(response)
            }
            None => decode_rpc_response_fragment(pdu, self.fragment_sizes.max_recv())
                .map_err(|e| custom_err!("decode rpc response", e)),
        }
    }

    async fn send_call(&mut self, call_id: u32, opnum: u16, stub: &[u8]) -> Result<(), Error> {
        let fragments = match self.rpc_auth.as_mut() {
            Some(auth) => encode_rpc_request_fragments_with_ntlm_auth(
                auth,
                &mut self.send_sequence,
                call_id,
                opnum,
                stub,
                self.fragment_sizes,
            )?,
            None => encode_rpc_request_fragments(call_id, opnum, stub, self.fragment_sizes)
                .map_err(|e| custom_err!("encode rpc request", e))?,
        };
        for fragment in fragments {
            self.write_in(&fragment).await?;
        }
        Ok(())
    }

    /// Decodes one administrative tunnel message, queues it for [`Self::read_data`],
    /// and re-queues the pending call ([MS-TSGU] 3.2.6.1.3).
    async fn handle_tunnel_message(&mut self, stub: &[u8]) -> Result<(), Error> {
        let message =
            decode_tsgu_make_tunnel_call_response(stub).map_err(|e| custom_err!("decode tunnel message", e))?;
        match message {
            TsProxyTunnelMessage::None => {}
            message => {
                if matches!(message, TsProxyTunnelMessage::Reauthenticate { .. }) {
                    return Err(Error::new(
                        "rpch reauthentication is not supported on this transport",
                        GwErrorKind::UnsupportedFeature,
                    ));
                }
                self.messages.push_back(message);
            }
        }

        // Re-queue the pending administrative-message call when a tunnel exists.
        if let Some(tunnel_context) = self.tunnel_context {
            let request = TsProxyMakeTunnelCallRequest::new(tunnel_context).encode();
            let call_id = self.next_call_id();
            self.send_call(call_id, TSPROXY_MAKE_TUNNEL_CALL_OPNUM, &request)
                .await?;
            self.tunnel_message_call_id = Some(call_id);
        }
        Ok(())
    }

    /// Handles an RTS PDU from the OUT channel (flow-control acks, pings).
    async fn handle_rts_pdu(&mut self, pdu: &[u8]) -> Result<(), Error> {
        // Ping PDUs are a no-op keepalive from the proxy.
        if decode_rts_ping(pdu).is_ok() {
            return Ok(());
        }
        match decode_rts_flow_control_ack(pdu) {
            Ok(ack) => {
                self.flow_control
                    .receive_flow_control_ack(ack)
                    .map_err(|e| custom_err!("rpch flow-control ack", e))?;
                Ok(())
            }
            Err(_) => {
                warn!("unhandled RTS PDU on RPCH OUT channel");
                Ok(())
            }
        }
    }

    /// Releases an OUT-channel PDU from the receive window, sending a flow-control
    /// acknowledgement when enough capacity has been reclaimed ([MS-RPCH] 3.2.1.5.1).
    async fn consume_out_pdu(&mut self, pdu_len: usize) -> Result<(), Error> {
        if let Some(ack) = self
            .flow_control
            .consumed_rpc_pdu(pdu_len)
            .map_err(|e| custom_err!("rpch receive accounting", e))?
        {
            let pdu = encode_rts_flow_control_ack(ack).map_err(|e| custom_err!("encode flow-control ack", e))?;
            self.write_in(&pdu).await?;
        }
        Ok(())
    }

    /// Writes one PDU to the IN channel with send-window accounting.
    async fn write_in(&mut self, pdu: &[u8]) -> Result<(), Error> {
        self.flow_control
            .sent_rpc_pdu(pdu.len())
            .map_err(|e| custom_err!("rpch send window", e))?;
        self.input
            .write_body(pdu)
            .await
            .map_err(|e| custom_err!("rpch in write", e))?;
        self.ping.record_send(Duration::ZERO);
        Ok(())
    }

    fn next_call_id(&mut self) -> u32 {
        self.call_id = self.call_id.wrapping_add(1);
        self.call_id
    }
}

/// One event read from the RPCH OUT channel.
#[derive(Debug)]
pub(crate) enum RpchRead {
    /// Target-server bytes from the receive pipe.
    Data(Bytes),
    /// An administrative tunnel message (consent, service, reauthentication).
    Message(TsProxyTunnelMessage),
}

/// Drives an RPCH session over a byte channel, surfacing it as an `AsyncRead`/`AsyncWrite` stream.
///
/// `read_data` consumes the OUT channel, `send_to_server` produces IN-channel PDUs.
/// A background task bridges them so the returned value can be polled like a socket.
pub struct RpchStream {
    work: tokio::task::JoinHandle<Result<(), Error>>,
    /// Set once the work task has completed; its `JoinHandle` must not be polled again
    /// (tokio panics if a completed `JoinHandle` is polled).
    work_done: bool,
    rx: tokio::sync::mpsc::Receiver<Bytes>,
    rx_bufs: Vec<Bytes>,
    tx: tokio_util::sync::PollSender<Bytes>,
}

impl RpchStream {
    pub(crate) fn new<S>(mut session: RpchSession<S>) -> Self
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (in_tx, in_rx) = tokio::sync::mpsc::channel::<Bytes>(4);
        let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Bytes>(4);

        let work = tokio::spawn(async move {
            loop {
                tokio::select!(
                    read = session.read_data() => {
                        match read {
                            Ok(RpchRead::Data(data)) => {
                                in_tx.send(data).await.map_err(|e| custom_err!("forward rpch data", e))?;
                            }
                            Ok(RpchRead::Message(message)) => {
                                warn!("RPCH administrative message: {message:?}");
                            }
                            Err(e) => {
                                error!("RPCH receive pipe failed: {e:#}");
                                return Err(e);
                            }
                        }
                    },
                    next = out_rx.recv() => {
                        let next = next.ok_or_else(|| Error::new("rpch write channel closed", GwErrorKind::Connect))?;
                        if let Err(e) = session.send_to_server(&next).await {
                            error!("RPCH send to server failed: {e:#}");
                            return Err(e);
                        }
                    }
                );
            }
        });

        Self {
            work,
            work_done: false,
            rx: in_rx,
            rx_bufs: Vec::new(),
            tx: tokio_util::sync::PollSender::new(out_tx),
        }
    }
}

impl AsyncRead for RpchStream {
    fn poll_read(
        mut self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> core::task::Poll<std::io::Result<()>> {
        if !self.work_done {
            match self.work.poll_unpin(cx) {
                core::task::Poll::Ready(Err(e)) => {
                    self.work_done = true;
                    return core::task::Poll::Ready(Err(std::io::Error::other(e)));
                }
                core::task::Poll::Ready(Ok(Err(e))) => {
                    self.work_done = true;
                    return core::task::Poll::Ready(Err(std::io::Error::other(e.to_string())));
                }
                core::task::Poll::Ready(Ok(Ok(()))) => {
                    self.work_done = true;
                }
                core::task::Poll::Pending => (),
            }
        }

        if let core::task::Poll::Ready(Some(new_buf)) = self.rx.poll_recv(cx) {
            self.rx_bufs.push(new_buf);
        }

        let mut n = 0;
        self.rx_bufs.retain_mut(|rx_buf| {
            let rem = buf.remaining();
            if rem == 0 {
                return true;
            }
            let max = core::cmp::min(rem, rx_buf.len());
            buf.put_slice(&rx_buf[..max]);
            n += max;
            let _ = rx_buf.split_to(max);
            !rx_buf.is_empty()
        });

        if n > 0 {
            return core::task::Poll::Ready(Ok(()));
        }
        if self.work_done {
            // The work task ended and no data is buffered: the gateway stream is closed.
            return core::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "rpch tunnel closed",
            )));
        }
        core::task::Poll::Pending
    }
}

impl AsyncWrite for RpchStream {
    fn poll_write(
        mut self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
        buf: &[u8],
    ) -> core::task::Poll<Result<usize, std::io::Error>> {
        if !self.work_done {
            match self.work.poll_unpin(cx) {
                core::task::Poll::Ready(Err(e)) => {
                    self.work_done = true;
                    return core::task::Poll::Ready(Err(std::io::Error::other(e)));
                }
                core::task::Poll::Ready(Ok(Err(e))) => {
                    self.work_done = true;
                    return core::task::Poll::Ready(Err(std::io::Error::other(e.to_string())));
                }
                core::task::Poll::Ready(Ok(Ok(()))) => {
                    self.work_done = true;
                }
                core::task::Poll::Pending => (),
            }
        }

        match self.tx.poll_reserve(cx) {
            core::task::Poll::Ready(Ok(())) => {
                if self.tx.send_item(Bytes::from(buf.to_vec())).is_err() {
                    return core::task::Poll::Ready(Err(std::io::Error::other("Sender closed")));
                }
                core::task::Poll::Ready(Ok(buf.len()))
            }
            core::task::Poll::Ready(Err(err)) => core::task::Poll::Ready(Err(std::io::Error::other(err))),
            core::task::Poll::Pending => core::task::Poll::Pending,
        }
    }

    fn poll_flush(
        self: core::pin::Pin<&mut Self>,
        _cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Result<(), std::io::Error>> {
        core::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        mut self: core::pin::Pin<&mut Self>,
        _cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Result<(), std::io::Error>> {
        self.tx.close();
        core::task::Poll::Ready(Ok(()))
    }
}

/// Streaming reader for DCE/RPC PDUs on the RPC_OUT_DATA response body.
struct OutChannel<S> {
    stream: S,
    buf: Vec<u8>,
}

impl<S> OutChannel<S>
where
    S: AsyncRead + Unpin,
{
    fn new(stream: S) -> Self {
        Self {
            stream,
            buf: Vec::new(),
        }
    }

    /// Reads one complete PDU, framed by the common header fragment length.
    async fn read_pdu(&mut self) -> Result<Vec<u8>, Error> {
        loop {
            if self.buf.len() >= RPC_COMMON_HEADER {
                let fragment_length = usize::from(u16::from_le_bytes(
                    self.buf[8..10].try_into().expect("fragment length slice"),
                ));
                if !(RPC_COMMON_HEADER..=MAX_OUT_PDU).contains(&fragment_length) {
                    return Err(Error::new("rpch out pdu length", GwErrorKind::Decode));
                }
                if self.buf.len() >= fragment_length {
                    return Ok(self.buf.drain(..fragment_length).collect());
                }
            }
            let mut chunk = [0u8; 8192];
            let read = match self.stream.read(&mut chunk).await {
                Ok(n) => n,
                // RD Gateways end the streaming OUT channel with a bare TCP FIN and skip the
                // TLS close_notify, which rustls reports as UnexpectedEof; treat it as the
                // end of the stream rather than a transport error.
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => 0,
                Err(e) => return Err(custom_err!("rpch out read", e)),
            };
            if read == 0 {
                return Err(Error::new("rpch out stream closed", GwErrorKind::PacketEof));
            }
            self.buf.extend_from_slice(&chunk[..read]);
        }
    }
}

/// The `/rpc/rpcproxy.dll?<server>:<port>` target of RPCH requests ([MS-RPCH] 2.1.2.1.1).
/// The RPC proxy request target is the gateway's own TS Proxy RPC endpoint, not the RDP
/// resource — the resource is named later by `TsProxyCreateChannel` ([MS-RPCH] 2.1.3.1,
/// [MS-TSGU] 3.2.6.1). The TS Proxy service listens on the gateway loopback at 3388.
fn rpc_proxy_target(_target: &GwConnectTarget) -> String {
    "localhost:3388".to_owned()
}

/// Advances the gateway HTTP authentication state from a 401 challenge set and
/// returns the `Authorization` header for the retry.
/// Whether a 401 challenge set carries a token (the NTLM type-2 challenge) rather than
/// bare scheme offers. Answering a token challenge produces the final type-3 token, whose
/// request commits the streaming body ([MS-RPCH] 3.2.2.2).
fn challenge_has_token(www_authenticate: &[String]) -> bool {
    www_authenticate
        .iter()
        .any(|value| value.starts_with("Negotiate ") || value.starts_with("NTLM "))
}

fn step_gateway_auth(
    http_auth: &mut Option<GatewayHttpAuth>,
    target: &GwConnectTarget,
    spn: &str,
    current_authorization: Option<&str>,
    www_authenticate: &[String],
) -> Result<String, Error> {
    let challenges: Vec<&str> = www_authenticate.iter().map(String::as_str).collect();
    let step = if let Some(auth) = http_auth.as_mut() {
        auth.step_www_authenticate(challenges)?
    } else {
        let (auth, step) = GatewayHttpAuth::from_challenges_ntlm_only(
            &target.gw_user,
            &target.gw_pass,
            target.smart_card.as_deref(),
            Some(spn.to_owned()),
            &challenges,
            true,
        )?;
        *http_auth = Some(auth);
        step
    };

    match step {
        AuthStep::Continue(header) => Ok(header),
        // Basic retries the same replayable head, so it needs no gateway state.
        AuthStep::TryBasic => Ok(crate::http_auth::basic_authorization(&target.gw_user, &target.gw_pass)),
        AuthStep::Complete => match current_authorization {
            Some(header) => Ok(header.to_owned()),
            None => Err(Error::new("rpch auth completed without a token", GwErrorKind::Connect)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock_rpch::mock_rpch_proxy;

    fn target() -> GwConnectTarget {
        GwConnectTarget {
            gw_endpoint: "rdg.contoso.com".to_owned(),
            gw_user: "CONTOSO\\alice".to_owned(),
            gw_pass: "secret".to_owned(),
            server: "rdp.contoso.com".to_owned(),
            server_port: 3389,
            smart_card: None,
        }
    }

    #[tokio::test]
    async fn rpch_session_runs_setup_and_echoes_data() {
        let (out, input, proxy) = mock_rpch_proxy();
        let settings = RpcHttpV2Settings::new(128 * 1024, 256 * 1024, 0).expect("valid settings");
        let mut session = Box::pin(rpch_connect(out, input, "rdg.contoso.com", &target(), settings, None))
            .await
            .expect("rpch connect");

        session
            .open_tunnel("test-client", "rdp.contoso.com", 3389)
            .await
            .expect("open tunnel");

        session.send_to_server(b"hello-rdp").await.expect("send");
        match session.read_data().await.expect("read data") {
            RpchRead::Data(data) => assert_eq!(&*data, b"hello-rdp"),
            other => panic!("expected data, got {other:?}"),
        }

        drop(session);
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_reports_service_messages() {
        let (out, input, proxy) = crate::mock_rpch::mock_rpch_proxy_with_service_message(Some("scheduled maintenance"));
        let settings = RpcHttpV2Settings::new(128 * 1024, 256 * 1024, 0).expect("valid settings");
        let mut session = Box::pin(rpch_connect(out, input, "rdg.contoso.com", &target(), settings, None))
            .await
            .expect("rpch connect");

        session
            .open_tunnel("test-client", "rdp.contoso.com", 3389)
            .await
            .expect("open tunnel");

        match session.read_data().await.expect("read message") {
            RpchRead::Message(TsProxyTunnelMessage::Service {
                display_mandatory,
                text,
            }) => {
                assert!(display_mandatory);
                assert_eq!(text, "scheduled maintenance");
            }
            other => panic!("expected service message, got {other:?}"),
        }

        drop(session);
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_surfaces_receive_pipe_error() {
        const E_PROXY_INTERNALERROR: u32 = 0x59d8;
        let (out, input, proxy) = crate::mock_rpch::mock_rpch_proxy_with_receive_pipe_error(E_PROXY_INTERNALERROR);
        let settings = RpcHttpV2Settings::new(128 * 1024, 256 * 1024, 0).expect("valid settings");
        let mut session = Box::pin(rpch_connect(out, input, "rdg.contoso.com", &target(), settings, None))
            .await
            .expect("rpch connect");

        // The receive pipe is fire-and-stream: open_tunnel returns without waiting, so the
        // gateway's terminal error surfaces on the first read ([MS-TSGU] 3.2.6.2.3).
        session
            .open_tunnel("test-client", "rdp.contoso.com", 3389)
            .await
            .expect("open tunnel");

        let err = session.read_data().await.expect_err("receive pipe error");
        assert!(matches!(err.kind(), GwErrorKind::GatewayCode(code) if *code == E_PROXY_INTERNALERROR));

        drop(session);
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_sends_ping_after_connection_timeout() {
        let (out, input, proxy) = mock_rpch_proxy();
        let settings = RpcHttpV2Settings::new(128 * 1024, 256 * 1024, 0).expect("valid settings");
        let mut session = Box::pin(rpch_connect(out, input, "rdg.contoso.com", &target(), settings, None))
            .await
            .expect("rpch connect");

        // The mock negotiated a 120-second connection timeout; nothing is due at t=0.
        assert!(!session.ping_due(Duration::ZERO));
        assert!(session.ping_due(Duration::from_secs(120)));
        session.send_ping(Duration::from_secs(120)).await.expect("ping");
        assert!(!session.ping_due(Duration::from_secs(120)));

        drop(session);
        proxy.abort();
    }
}
