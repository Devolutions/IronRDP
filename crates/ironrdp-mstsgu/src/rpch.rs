//! Crate-internal MS-RPCH/MS-TSGU session orchestration.
//!
//! This module accepts caller-owned IN and OUT streams and deliberately does
//! not open network connections or expose a production transport.
//! It drives only unprotected DCE/RPC calls.

use core::time::Duration;
use std::collections::VecDeque;

use hyper::body::Bytes;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use uuid::Uuid;

use crate::http_auth::{AuthStep, GatewayHttpAuth, basic_authorization};
use crate::rpc::tsgu::{
    NDR32_TRANSFER_SYNTAX_ID, NDR32_TRANSFER_SYNTAX_VERSION, NonNullRpcContextHandle, TSPROXY_AUTHORIZE_TUNNEL_OPNUM,
    TSPROXY_CREATE_CHANNEL_OPNUM, TSPROXY_CREATE_TUNNEL_OPNUM, TSPROXY_MAKE_TUNNEL_CALL_OPNUM,
    TSPROXY_RPC_INTERFACE_ID, TSPROXY_RPC_INTERFACE_VERSION, TSPROXY_SEND_TO_SERVER_OPNUM,
    TSPROXY_SETUP_RECEIVE_PIPE_OPNUM, TsProxyAuthorizeTunnelRequest, TsProxyCreateChannelRequest,
    TsProxyCreateTunnelRequest, TsProxyMakeTunnelCallRequest, TsProxySendToServerRequest,
    TsProxySetupReceivePipeRequest, TsProxyTunnelMessage, decode_tsgu_authorize_tunnel_response,
    decode_tsgu_create_channel_response, decode_tsgu_create_tunnel_response, decode_tsgu_make_tunnel_call_response,
    decode_tsgu_setup_receive_pipe_final_response,
};
use crate::rpc::{
    PTYPE_FAULT, PTYPE_RESPONSE, PTYPE_RTS, RpcFragmentSizes, RpcPresentationContext, RpcResponseReassembler,
    RpcSyntaxIdentifier, RpchFlowControl, RpchPingSchedule, RpchV2Settings, RpchV2Setup, RpchV2State, RtsCookie,
    decode_rpc_bind_ack, decode_rpc_response_fragment, decode_rts_flow_control_ack, encode_rpc_bind,
    encode_rpc_request_fragments, encode_rts_flow_control_ack, encode_rts_ping,
};
use crate::rpc_transport::{
    RpchInRequest, RpchRequestHead, drain_body, read_rpch_response_head, write_rpch_request_head,
};
use crate::{Error, GwErrorKind, GwSmartCardCredentials};

const IN_CONTENT_LENGTH: u32 = 128 * 1024;
const MAX_RESPONSE_STUB_SIZE: usize = 32 * 1024;
const MAX_TUNNEL_MESSAGE_STUB_SIZE: usize = 256 * 1024;
const MAX_AUTH_ROUNDS: usize = 8;

struct RpchHttpCredentials<'a> {
    username: &'a str,
    password: &'a str,
    smart_card: Option<&'a GwSmartCardCredentials>,
}

struct RpchSession<Out, In> {
    out: OutChannel<Out>,
    input: RpchInRequest<In>,
    flow_control: RpchFlowControl,
    ping: RpchPingSchedule,
    fragment_sizes: RpcFragmentSizes,
    responses: RpcResponseReassembler,
    tunnel_message_responses: RpcResponseReassembler,
    call_id: u32,
    tunnel_message_call_id: Option<u32>,
    receive_pipe_call_id: Option<u32>,
    tunnel_context: Option<NonNullRpcContextHandle>,
    channel_context: Option<NonNullRpcContextHandle>,
    messages: VecDeque<TsProxyTunnelMessage>,
}

async fn rpch_connect<Out, In>(
    out_stream: Out,
    in_stream: In,
    gateway_host: &str,
    target: &str,
    credentials: RpchHttpCredentials<'_>,
    settings: RpchV2Settings,
) -> Result<RpchSession<Out, In>, Error>
where
    Out: AsyncRead + AsyncWrite + Unpin,
    In: AsyncRead + AsyncWrite + Unpin,
{
    let association_group_id = Uuid::new_v4();
    let mut setup = RpchV2Setup::new(
        settings,
        RtsCookie::new(*Uuid::new_v4().as_bytes()),
        RtsCookie::new(*Uuid::new_v4().as_bytes()),
        RtsCookie::new(*Uuid::new_v4().as_bytes()),
        RtsCookie::new(association_group_id.to_bytes_le()),
    );
    setup
        .start_in_request()
        .map_err(|error| custom_err!("start rpch IN request", error))?;
    let out_body = setup
        .out_request_body()
        .map_err(|error| custom_err!("encode rpch CONN/A1", error))?;
    let in_initial_pdu = setup
        .in_request_initial_pdu()
        .map_err(|error| custom_err!("encode rpch CONN/B1", error))?;

    let mut input = open_in_channel(in_stream, gateway_host, target, &credentials, IN_CONTENT_LENGTH).await?;
    input.write_body(&in_initial_pdu).await?;
    input.flush().await?;

    let session_id = association_group_id.to_string();
    let (out_stream, out_response) =
        open_out_channel(out_stream, gateway_host, target, &session_id, &credentials, &out_body).await?;
    setup
        .accept_out_response(out_response.status)
        .map_err(|error| custom_err!("accept rpch OUT response", error))?;

    let mut out = OutChannel::new(out_stream);
    while setup.state() != RpchV2State::Open {
        let pdu = out.read_pdu().await?;
        if is_rts_ping(&pdu) || decode_rts_flow_control_ack(&pdu).is_ok() {
            continue;
        }
        setup
            .receive_out_pdu(&pdu)
            .map_err(|error| custom_err!("decode rpch setup pdu", error))?;
    }

    let mut session = RpchSession {
        out,
        input,
        flow_control: setup
            .flow_control()
            .map_err(|error| custom_err!("create rpch flow control", error))?,
        ping: setup
            .ping_schedule(Duration::ZERO, Duration::ZERO)
            .map_err(|error| custom_err!("create rpch ping schedule", error))?,
        fragment_sizes: RpcFragmentSizes::DEFAULT,
        responses: RpcResponseReassembler::new(MAX_RESPONSE_STUB_SIZE),
        tunnel_message_responses: RpcResponseReassembler::new(MAX_TUNNEL_MESSAGE_STUB_SIZE),
        call_id: 0,
        tunnel_message_call_id: None,
        receive_pipe_call_id: None,
        tunnel_context: None,
        channel_context: None,
        messages: VecDeque::new(),
    };
    session.bind().await?;
    Ok(session)
}

async fn open_in_channel<S>(
    stream: S,
    gateway_host: &str,
    target: &str,
    credentials: &RpchHttpCredentials<'_>,
    content_length: u32,
) -> Result<RpchInRequest<S>, Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let spn = format!("HTTP/{gateway_host}");
    let mut auth = None;
    let mut authorization = None;
    let mut input = RpchInRequest::open(stream, gateway_host, target, None).await?;

    for _ in 0..MAX_AUTH_ROUNDS {
        let response = input.receive_response().await?;
        match response.status {
            401 => {
                let commits_body = challenge_has_token(&response.www_authenticate);
                authorization = Some(step_http_auth(
                    &mut auth,
                    credentials,
                    &spn,
                    authorization.as_deref(),
                    &response.www_authenticate,
                )?);
                input = input
                    .retry(authorization.as_deref(), if commits_body { content_length } else { 0 })
                    .await?;
                if commits_body {
                    return Ok(input);
                }
            }
            200 if response.content_length == Some(0) => {
                return input.retry(authorization.as_deref(), content_length).await;
            }
            200 => {
                return Err(Error::new(
                    "rpch IN authentication response has a body",
                    GwErrorKind::Decode,
                ));
            }
            status => return Err(Error::new("rpch IN channel rejected", GwErrorKind::HttpStatus(status))),
        }
    }

    Err(Error::new(
        "rpch IN authentication rounds exceeded",
        GwErrorKind::Connect,
    ))
}

async fn open_out_channel<S>(
    mut stream: S,
    gateway_host: &str,
    target: &str,
    session_id: &str,
    credentials: &RpchHttpCredentials<'_>,
    body: &[u8],
) -> Result<(S, crate::rpc_transport::RpchResponseHead), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let content_length =
        u32::try_from(body.len()).map_err(|_| Error::new("rpch CONN/A1 length", GwErrorKind::Encode))?;
    let spn = format!("HTTP/{gateway_host}");
    let mut auth = None;
    let mut authorization = None;
    let mut commits_body = false;

    for _ in 0..MAX_AUTH_ROUNDS {
        write_rpch_request_head(
            &mut stream,
            RpchRequestHead {
                method: "RPC_OUT_DATA",
                host: gateway_host,
                target,
                content_length: if commits_body { content_length } else { 0 },
                authorization: authorization.as_deref(),
                cookie: None,
                session_id: Some(session_id),
                expect_continue: false,
            },
        )
        .await?;
        if commits_body {
            stream
                .write_all(body)
                .await
                .map_err(|error| custom_err!("write rpch CONN/A1", error))?;
        }
        stream
            .flush()
            .await
            .map_err(|error| custom_err!("flush rpch CONN/A1", error))?;

        let response = read_rpch_response_head(&mut stream).await?;
        match response.status {
            401 => {
                let length = response.content_length.ok_or_else(|| {
                    Error::new(
                        "rpch authentication response missing content length",
                        GwErrorKind::Decode,
                    )
                })?;
                drain_body(&mut stream, length).await?;
                commits_body = challenge_has_token(&response.www_authenticate);
                authorization = Some(step_http_auth(
                    &mut auth,
                    credentials,
                    &spn,
                    authorization.as_deref(),
                    &response.www_authenticate,
                )?);
            }
            200 if !commits_body && response.content_length == Some(0) => commits_body = true,
            200 if !commits_body => {
                return Err(Error::new(
                    "rpch OUT authentication response has a body",
                    GwErrorKind::Decode,
                ));
            }
            _ => return Ok((stream, response)),
        }
    }

    Err(Error::new(
        "rpch OUT authentication rounds exceeded",
        GwErrorKind::Connect,
    ))
}

fn step_http_auth(
    auth: &mut Option<GatewayHttpAuth>,
    credentials: &RpchHttpCredentials<'_>,
    spn: &str,
    current_authorization: Option<&str>,
    www_authenticate: &[String],
) -> Result<String, Error> {
    let challenges: Vec<&str> = www_authenticate.iter().map(String::as_str).collect();
    let step = match auth.as_mut() {
        Some(auth) => auth.step_www_authenticate(challenges)?,
        None => {
            let (new_auth, step) = GatewayHttpAuth::from_challenges(
                credentials.username,
                credentials.password,
                credentials.smart_card,
                Some(spn.to_owned()),
                &challenges,
            )?;
            *auth = new_auth;
            step
        }
    };

    match step {
        AuthStep::Continue(authorization) => Ok(authorization),
        AuthStep::Complete => current_authorization
            .map(str::to_owned)
            .ok_or_else(|| Error::new("rpch authentication completed without a token", GwErrorKind::Connect)),
        AuthStep::TryBasic => Ok(basic_authorization(credentials.username, credentials.password)),
    }
}

fn challenge_has_token(www_authenticate: &[String]) -> bool {
    www_authenticate.iter().any(|challenge| {
        let mut fields = challenge.split_ascii_whitespace();
        matches!(fields.next(), Some(scheme) if scheme.eq_ignore_ascii_case("Negotiate") || scheme.eq_ignore_ascii_case("NTLM"))
            && fields.next().is_some()
    })
}

impl<Out, In> RpchSession<Out, In>
where
    Out: AsyncRead + AsyncWrite + Unpin,
    In: AsyncRead + AsyncWrite + Unpin,
{
    async fn open_tunnel(&mut self, client_name: &str, resource: &str, port: u16) -> Result<(), Error> {
        let request = TsProxyCreateTunnelRequest::new(3)
            .encode()
            .map_err(|error| custom_err!("encode create tunnel", error))?;
        let response = self.call(TSPROXY_CREATE_TUNNEL_OPNUM, &request).await?;
        let tunnel = decode_tsgu_create_tunnel_response(&response)
            .map_err(|error| custom_err!("decode create tunnel", error))?;
        self.tunnel_context = Some(tunnel.tunnel_context);

        let request = TsProxyAuthorizeTunnelRequest::new(tunnel.tunnel_context, client_name, &[])
            .encode()
            .map_err(|error| custom_err!("encode authorize tunnel", error))?;
        let response = self.call(TSPROXY_AUTHORIZE_TUNNEL_OPNUM, &request).await?;
        decode_tsgu_authorize_tunnel_response(&response)
            .map_err(|error| custom_err!("decode authorize tunnel", error))?;

        self.queue_tunnel_message_call(tunnel.tunnel_context).await?;

        let request = TsProxyCreateChannelRequest::new(tunnel.tunnel_context, resource, port)
            .encode()
            .map_err(|error| custom_err!("encode create channel", error))?;
        let response = self.call(TSPROXY_CREATE_CHANNEL_OPNUM, &request).await?;
        let channel = decode_tsgu_create_channel_response(&response)
            .map_err(|error| custom_err!("decode create channel", error))?;
        self.channel_context = Some(channel.channel_context);

        let request = TsProxySetupReceivePipeRequest::new(channel.channel_context).encode();
        let call_id = self.next_call_id();
        self.send_call(call_id, TSPROXY_SETUP_RECEIVE_PIPE_OPNUM, &request)
            .await?;
        self.receive_pipe_call_id = Some(call_id);
        Ok(())
    }

    async fn send_to_server(&mut self, data: &[u8]) -> Result<(), Error> {
        let channel_context = self
            .channel_context
            .ok_or_else(|| Error::new("rpch send before channel create", GwErrorKind::Connect))?;
        let request = TsProxySendToServerRequest::new(channel_context, data)
            .and_then(|request| request.encode())
            .map_err(|error| custom_err!("encode send-to-server", error))?;
        let call_id = self.next_call_id();
        self.send_call(call_id, TSPROXY_SEND_TO_SERVER_OPNUM, &request).await
    }

    async fn read_data(&mut self) -> Result<RpchRead, Error> {
        if let Some(message) = self.messages.pop_front() {
            return Ok(RpchRead::Message(message));
        }
        loop {
            let pdu = self.out.read_pdu().await?;
            self.flow_control
                .received_rpc_pdu(pdu.len())
                .map_err(|error| custom_err!("account rpch receive window", error))?;

            match pdu.get(2).copied() {
                Some(PTYPE_RTS) => {
                    self.handle_rts_pdu(&pdu)?;
                    self.consume_out_pdu(pdu.len()).await?;
                }
                Some(PTYPE_FAULT) => return Err(Error::new("rpc fault on receive pipe", GwErrorKind::Connect)),
                Some(PTYPE_RESPONSE) => {
                    let response = decode_rpc_response_fragment(&pdu, self.fragment_sizes.max_recv())
                        .map_err(|error| custom_err!("decode rpch response", error))?;
                    if self.tunnel_message_call_id == Some(response.call_id) {
                        self.handle_tunnel_message_response(pdu.len(), response).await?;
                        if let Some(message) = self.messages.pop_front() {
                            return Ok(RpchRead::Message(message));
                        }
                    } else if self.receive_pipe_call_id == Some(response.call_id) {
                        if response.is_last_fragment() && response.stub.len() == 4 {
                            let result = decode_tsgu_setup_receive_pipe_final_response(response.stub)
                                .map_err(|error| custom_err!("decode receive-pipe terminal result", error))?;
                            return Err(Error::new(
                                if result == 0 {
                                    "receive pipe closed"
                                } else {
                                    "receive pipe failed"
                                },
                                if result == 0 {
                                    GwErrorKind::Connect
                                } else {
                                    GwErrorKind::GatewayCode(result)
                                },
                            ));
                        }
                        let data = Bytes::copy_from_slice(response.stub);
                        self.consume_out_pdu(pdu.len()).await?;
                        return Ok(RpchRead::Data(data));
                    } else {
                        self.consume_out_pdu(pdu.len()).await?;
                    }
                }
                _ => return Err(Error::new("unexpected rpch OUT pdu", GwErrorKind::Decode)),
            }
        }
    }

    fn ping_due(&self, now: Duration) -> bool {
        self.ping.ping_due(now)
    }

    async fn send_ping(&mut self, now: Duration) -> Result<(), Error> {
        let pdu = encode_rts_ping().map_err(|error| custom_err!("encode rpch ping", error))?;
        self.write_in(&pdu).await?;
        self.ping.record_send(now);
        Ok(())
    }

    async fn bind(&mut self) -> Result<(), Error> {
        let transfer_syntaxes = [RpcSyntaxIdentifier::new(
            NDR32_TRANSFER_SYNTAX_ID,
            NDR32_TRANSFER_SYNTAX_VERSION,
        )];
        let contexts = [RpcPresentationContext {
            context_id: 0,
            abstract_syntax: RpcSyntaxIdentifier::new(TSPROXY_RPC_INTERFACE_ID, TSPROXY_RPC_INTERFACE_VERSION),
            transfer_syntaxes: &transfer_syntaxes,
        }];
        let call_id = self.next_call_id();
        let bind = encode_rpc_bind(call_id, self.fragment_sizes, 0, &contexts)
            .map_err(|error| custom_err!("encode tsgu bind", error))?;
        self.write_in(&bind).await?;
        let response = self.out.read_pdu().await?;
        self.flow_control
            .received_rpc_pdu(response.len())
            .map_err(|error| custom_err!("account tsgu bind response", error))?;
        let binding = decode_rpc_bind_ack(&response, self.fragment_sizes)
            .map_err(|error| custom_err!("decode tsgu bind", error))?;
        self.fragment_sizes = binding.fragment_sizes;
        self.consume_out_pdu(response.len()).await
    }

    async fn call(&mut self, opnum: u16, stub: &[u8]) -> Result<Vec<u8>, Error> {
        let call_id = self.next_call_id();
        self.send_call(call_id, opnum, stub).await?;
        loop {
            let pdu = self.out.read_pdu().await?;
            self.flow_control
                .received_rpc_pdu(pdu.len())
                .map_err(|error| custom_err!("account rpch receive window", error))?;
            match pdu.get(2).copied() {
                Some(PTYPE_RTS) => {
                    self.handle_rts_pdu(&pdu)?;
                    self.consume_out_pdu(pdu.len()).await?;
                }
                Some(PTYPE_FAULT) => return Err(Error::new("rpc fault in tsproxy call", GwErrorKind::Connect)),
                Some(PTYPE_RESPONSE) => {
                    let response = decode_rpc_response_fragment(&pdu, self.fragment_sizes.max_recv())
                        .map_err(|error| custom_err!("decode tsproxy response", error))?;
                    if self.tunnel_message_call_id == Some(response.call_id) {
                        self.handle_tunnel_message_response(pdu.len(), response).await?;
                        continue;
                    }
                    if response.call_id != call_id {
                        self.consume_out_pdu(pdu.len()).await?;
                        return Err(Error::new("unexpected tsproxy response call ID", GwErrorKind::Decode));
                    }
                    let response = self
                        .responses
                        .push(response)
                        .map_err(|error| custom_err!("reassemble tsproxy response", error))?;
                    self.consume_out_pdu(pdu.len()).await?;
                    if let Some(response) = response {
                        return Ok(response.stub);
                    }
                }
                _ => return Err(Error::new("unexpected rpch OUT pdu", GwErrorKind::Decode)),
            }
        }
    }

    async fn handle_tunnel_message_response(
        &mut self,
        pdu_len: usize,
        response: crate::rpc::RpcResponse<'_>,
    ) -> Result<(), Error> {
        let response = self
            .tunnel_message_responses
            .push(response)
            .map_err(|error| custom_err!("reassemble tunnel message", error))?;
        self.consume_out_pdu(pdu_len).await?;
        let Some(response) = response else {
            return Ok(());
        };

        let message = decode_tsgu_make_tunnel_call_response(&response.stub)
            .map_err(|error| custom_err!("decode tunnel message", error))?;
        self.tunnel_message_call_id = None;
        if let Some(tunnel_context) = self.tunnel_context {
            self.queue_tunnel_message_call(tunnel_context).await?;
        }
        if !matches!(message, TsProxyTunnelMessage::None) {
            self.messages.push_back(message);
        }
        Ok(())
    }

    async fn queue_tunnel_message_call(&mut self, tunnel_context: NonNullRpcContextHandle) -> Result<(), Error> {
        let call_id = self.next_call_id();
        self.send_call(
            call_id,
            TSPROXY_MAKE_TUNNEL_CALL_OPNUM,
            &TsProxyMakeTunnelCallRequest::new(tunnel_context).encode(),
        )
        .await?;
        self.tunnel_message_call_id = Some(call_id);
        Ok(())
    }

    async fn send_call(&mut self, call_id: u32, opnum: u16, stub: &[u8]) -> Result<(), Error> {
        for fragment in encode_rpc_request_fragments(call_id, 0, opnum, stub, self.fragment_sizes)
            .map_err(|error| custom_err!("encode tsproxy request", error))?
        {
            self.write_in(&fragment).await?;
        }
        Ok(())
    }

    fn handle_rts_pdu(&mut self, pdu: &[u8]) -> Result<(), Error> {
        if is_rts_ping(pdu) {
            return Ok(());
        }
        let ack =
            decode_rts_flow_control_ack(pdu).map_err(|error| custom_err!("decode rpch flow-control ack", error))?;
        self.flow_control
            .receive_flow_control_ack(ack)
            .map_err(|error| custom_err!("apply rpch flow-control ack", error))?;
        Ok(())
    }

    async fn consume_out_pdu(&mut self, pdu_len: usize) -> Result<(), Error> {
        if let Some(ack) = self
            .flow_control
            .consumed_rpc_pdu(pdu_len)
            .map_err(|error| custom_err!("release rpch receive window", error))?
        {
            self.write_in(
                &encode_rts_flow_control_ack(ack)
                    .map_err(|error| custom_err!("encode rpch flow-control ack", error))?,
            )
            .await?;
        }
        Ok(())
    }

    async fn write_in(&mut self, pdu: &[u8]) -> Result<(), Error> {
        self.flow_control
            .sent_rpc_pdu(pdu.len())
            .map_err(|error| custom_err!("account rpch send window", error))?;
        self.input.write_body(pdu).await?;
        self.input.flush().await?;
        Ok(())
    }

    fn next_call_id(&mut self) -> u32 {
        self.call_id = self.call_id.wrapping_add(1);
        self.call_id
    }
}

fn is_rts_ping(pdu: &[u8]) -> bool {
    encode_rts_ping().is_ok_and(|ping| pdu == ping)
}

#[derive(Debug)]
enum RpchRead {
    Data(Bytes),
    Message(TsProxyTunnelMessage),
}

struct OutChannel<S> {
    stream: S,
}

impl<S> OutChannel<S>
where
    S: AsyncRead + Unpin,
{
    fn new(stream: S) -> Self {
        Self { stream }
    }

    async fn read_pdu(&mut self) -> Result<Vec<u8>, Error> {
        let mut header = [0; 16];
        self.stream
            .read_exact(&mut header)
            .await
            .map_err(|error| custom_err!("read rpch pdu header", error))?;
        let length = usize::from(u16::from_le_bytes([header[8], header[9]]));
        if length < header.len() {
            return Err(Error::new("invalid rpch pdu length", GwErrorKind::Decode));
        }
        let mut pdu = header.to_vec();
        pdu.resize(length, 0);
        self.stream
            .read_exact(&mut pdu[header.len()..])
            .await
            .map_err(|error| custom_err!("read rpch pdu body", error))?;
        Ok(pdu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock_rpch::{MockRpchScenario, MockTunnelMessage, mock_rpch_proxy};
    use tokio::io::DuplexStream;

    async fn open_session(
        scenario: MockRpchScenario,
    ) -> (RpchSession<DuplexStream, DuplexStream>, tokio::task::JoinHandle<()>) {
        let (out, input, proxy) = mock_rpch_proxy(scenario);
        let mut session = rpch_connect(
            out,
            input,
            "rdg.contoso.com",
            "rdp.contoso.com:3389",
            RpchHttpCredentials {
                username: "CONTOSO\\alice",
                password: "secret",
                smart_card: None,
            },
            RpchV2Settings::new(128 * 1024, 256 * 1024, 0).expect("valid test settings"),
        )
        .await
        .expect("rpch connect");
        session
            .open_tunnel("test-client", "rdp.contoso.com", 3389)
            .await
            .expect("open tunnel");
        (session, proxy)
    }

    #[tokio::test]
    async fn rpch_session_runs_setup_reassembles_control_response_and_echoes_data() {
        let (mut session, proxy) = open_session(MockRpchScenario::Echo).await;
        session.send_to_server(b"hello-rdp").await.expect("send target data");
        match session.read_data().await.expect("read target data") {
            RpchRead::Data(data) => assert_eq!(&*data, b"hello-rdp"),
            event => panic!("expected target data, got {event:?}"),
        }
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_authenticates_each_http_channel_independently() {
        let (mut session, proxy) = open_session(MockRpchScenario::BasicAuthentication).await;
        session
            .send_to_server(b"authenticated")
            .await
            .expect("send target data");
        assert!(matches!(session.read_data().await, Ok(RpchRead::Data(_))));
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_surfaces_service_messages() {
        let (mut session, proxy) = open_session(MockRpchScenario::Message(MockTunnelMessage::Service(
            "scheduled maintenance",
        )))
        .await;
        match session.read_data().await.expect("read service message") {
            RpchRead::Message(TsProxyTunnelMessage::Service {
                display_mandatory,
                text,
            }) => {
                assert!(display_mandatory);
                assert_eq!(text, "scheduled maintenance");
            }
            event => panic!("expected service message, got {event:?}"),
        }
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_surfaces_reauthentication_messages_without_authentication() {
        let (mut session, proxy) = open_session(MockRpchScenario::Message(MockTunnelMessage::Reauthenticate(
            0x0123_4567_89ab_cdef,
        )))
        .await;
        match session.read_data().await.expect("read reauthentication message") {
            RpchRead::Message(TsProxyTunnelMessage::Reauthenticate { tunnel_context }) => {
                assert_eq!(tunnel_context, 0x0123_4567_89ab_cdef);
            }
            event => panic!("expected reauthentication message, got {event:?}"),
        }
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_queues_tunnel_messages_before_control_responses() {
        let (mut session, proxy) = open_session(MockRpchScenario::MessageBeforeCreateChannel(
            MockTunnelMessage::Service("scheduled maintenance"),
        ))
        .await;
        match session.read_data().await.expect("read queued service message") {
            RpchRead::Message(TsProxyTunnelMessage::Service {
                display_mandatory,
                text,
            }) => {
                assert!(display_mandatory);
                assert_eq!(text, "scheduled maintenance");
            }
            event => panic!("expected service message, got {event:?}"),
        }
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_surfaces_receive_pipe_terminal_hresult() {
        const E_PROXY_INTERNAL_ERROR: u32 = 0x0000_59d8;
        let (mut session, proxy) = open_session(MockRpchScenario::ReceivePipeError(E_PROXY_INTERNAL_ERROR)).await;
        let error = session.read_data().await.expect_err("receive pipe error");
        assert!(matches!(error.kind(), GwErrorKind::GatewayCode(code) if *code == E_PROXY_INTERNAL_ERROR));
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_handles_ping_and_flow_control_acknowledgements() {
        let (mut session, proxy) = open_session(MockRpchScenario::Echo).await;
        assert!(!session.ping_due(Duration::ZERO));
        assert!(session.ping_due(Duration::from_secs(120)));
        session.send_ping(Duration::from_secs(120)).await.expect("send ping");
        assert!(!session.ping_due(Duration::from_secs(120)));

        let data = vec![0; 2 * 1024];
        session.send_to_server(&data).await.expect("send target data");
        let read = session.read_data().await;
        assert!(
            matches!(read, Ok(RpchRead::Data(_))),
            "expected target data, got {read:?}"
        );
        assert_eq!(session.flow_control.send_available_window(), 8 * 1024);
        session
            .send_to_server(&data)
            .await
            .expect("send after flow-control acknowledgement");
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_rejects_invalid_out_fragment_length() {
        let (out, input, proxy) = mock_rpch_proxy(MockRpchScenario::InvalidOutFragmentLength);
        let error = match rpch_connect(
            out,
            input,
            "rdg.contoso.com",
            "rdp.contoso.com:3389",
            RpchHttpCredentials {
                username: "CONTOSO\\alice",
                password: "secret",
                smart_card: None,
            },
            RpchV2Settings::new(128 * 1024, 256 * 1024, 0).expect("valid test settings"),
        )
        .await
        {
            Ok(_) => panic!("invalid fragment length must fail"),
            Err(error) => error,
        };
        assert!(matches!(error.kind(), GwErrorKind::Decode));
        proxy.abort();
    }
}
