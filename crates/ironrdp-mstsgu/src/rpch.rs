//! MS-RPCH/MS-TSGU session orchestration.
//!
//! The low-level client accepts caller-owned IN and OUT streams and caller-signed RPC fragments.
//! The live gateway stream owns the NTLM association, RPC packet integrity and TS Proxy sequence.

use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;
use std::collections::VecDeque;
use std::io;
use std::time::Instant;

use hyper::body::Bytes;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadBuf};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::PollSender;
use uuid::Uuid;

use crate::http_auth::{AuthStep, GatewayHttpAuth, basic_authorization, run_http_auth};
use crate::rpc::tsgu::{
    NDR32_TRANSFER_SYNTAX_ID, NDR32_TRANSFER_SYNTAX_VERSION, NonNullRpcContextHandle, TSPROXY_AUTHORIZE_TUNNEL_OPNUM,
    TSPROXY_CLOSE_CHANNEL_OPNUM, TSPROXY_CLOSE_TUNNEL_OPNUM, TSPROXY_CREATE_CHANNEL_OPNUM, TSPROXY_CREATE_TUNNEL_OPNUM,
    TSPROXY_MAKE_TUNNEL_CALL_OPNUM, TSPROXY_RPC_INTERFACE_ID, TSPROXY_RPC_INTERFACE_VERSION,
    TSPROXY_SEND_TO_SERVER_OPNUM, TSPROXY_SETUP_RECEIVE_PIPE_OPNUM, TsProxyAuthorizeTunnelRequest,
    TsProxyCloseContextRequest, TsProxyCreateChannelRequest, TsProxyCreateTunnelRequest, TsProxyMakeTunnelCallRequest,
    TsProxySendToServerRequest, TsProxySetupReceivePipeRequest, TsProxyTunnelMessage,
    decode_tsgu_authorize_tunnel_response, decode_tsgu_close_context_response, decode_tsgu_create_channel_response,
    decode_tsgu_create_tunnel_response, decode_tsgu_make_tunnel_call_response, decode_tsgu_send_to_server_response,
    decode_tsgu_setup_receive_pipe_final_response,
};
use crate::rpc::{
    PTYPE_FAULT, PTYPE_RESPONSE, PTYPE_RTS, RPC_AUTH_LEVEL_PACKET_INTEGRITY, RPC_AUTH_TYPE_WINNT,
    RPC_NTLM_VERIFIER_LENGTH, RpcAuthenticatedResponseReassembler, RpcAuthenticationInfo, RpcFragmentSizes,
    RpcNtlmAuth, RpcPresentationContext, RpcSyntaxIdentifier, RpchFlowControl, RpchPingSchedule, RpchV2Settings,
    RpchV2Setup, RpchV2State, RtsCookie, RtsDestination, decode_rpc_authenticated_fragment,
    decode_rpc_bind_ack_with_ntlm_auth, decode_rts_flow_control_ack, encode_rpc_auth_3, encode_rpc_bind_with_ntlm_auth,
    encode_rts_flow_control_ack_with_destination, encode_rts_ping, prepare_rpc_authenticated_request_fragments,
};
use crate::rpc_transport::{
    RpchInRequest, RpchRequestHead, drain_body, read_rpch_response_head, write_rpch_request_head,
};
use crate::{Error, GwConnectTarget, GwErrorKind, GwRpchRedirectionFlags, GwSmartCardCredentials};

const IN_CONTENT_LENGTH: u32 = 128 * 1024;
const MAX_RESPONSE_STUB_SIZE: usize = 32 * 1024;
const MAX_TUNNEL_MESSAGE_STUB_SIZE: usize = 256 * 1024;
const MAX_AUTH_ROUNDS: usize = 8;
const IN_RECYCLE_RESERVE: u32 = 1024;

#[derive(Clone)]
pub(crate) struct RpchHttpCredentials {
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) smart_card: Option<GwSmartCardCredentials>,
}

trait RpchStreamProvider<Out, In>: Send {
    fn open_in(&mut self) -> impl Future<Output = Result<In, Error>> + Send;
    fn open_out(&mut self) -> impl Future<Output = Result<Out, Error>> + Send;
}

struct RpchSession<Out, In, Provider> {
    out: OutChannel,
    input: RpchInRequest<In>,
    provider: Provider,
    setup: RpchV2Setup,
    gateway_host: String,
    target: String,
    session_id: String,
    credentials: RpchHttpCredentials,
    flow_control: RpchFlowControl,
    connection_timeout: Duration,
    ping: RpchPingSchedule,
    fragment_sizes: RpcFragmentSizes,
    rpc_auth: RpcNtlmAuth,
    rpc_authentication: RpcAuthenticationInfo,
    redirection_flags: GwRpchRedirectionFlags,
    supports_header_signing: bool,
    send_sequence_number: u32,
    receive_sequence_number: u32,
    responses: RpcAuthenticatedResponseReassembler,
    tunnel_message_responses: RpcAuthenticatedResponseReassembler,
    call_id: u32,
    tunnel_message_call_id: Option<u32>,
    receive_pipe_call_id: Option<u32>,
    tunnel_context: Option<NonNullRpcContextHandle>,
    channel_context: Option<NonNullRpcContextHandle>,
    data: VecDeque<Bytes>,
    messages: VecDeque<TsProxyTunnelMessage>,
    failed: bool,
    _out: PhantomData<fn() -> Out>,
}

async fn rpch_connect<Out, In, Provider>(
    out_stream: Out,
    in_stream: In,
    provider: Provider,
    gateway_host: &str,
    target: &str,
    credentials: RpchHttpCredentials,
    settings: RpchV2Settings,
) -> Result<RpchSession<Out, In, Provider>, Error>
where
    Out: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    In: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    Provider: RpchStreamProvider<Out, In>,
{
    let (channel, setup, session_id) =
        open_rpch_channels(out_stream, in_stream, gateway_host, target, &credentials, settings).await?;
    let ping = setup
        .ping_schedule(Duration::ZERO, Duration::ZERO)
        .map_err(|error| custom_err!("create rpch ping schedule", error))?;
    let rpc_auth = RpcNtlmAuth::new(&credentials.username, &credentials.password)?;
    let mut session = RpchSession::<Out, In, Provider> {
        out: channel.out,
        input: channel.input,
        provider,
        setup,
        gateway_host: gateway_host.to_owned(),
        target: target.to_owned(),
        session_id,
        credentials,
        flow_control: channel.flow_control,
        connection_timeout: channel.connection_timeout,
        ping,
        fragment_sizes: RpcFragmentSizes::DEFAULT,
        rpc_auth,
        rpc_authentication: RpcAuthenticationInfo {
            auth_type: RPC_AUTH_TYPE_WINNT,
            auth_level: RPC_AUTH_LEVEL_PACKET_INTEGRITY,
            auth_context_id: 1,
            verifier_length: RPC_NTLM_VERIFIER_LENGTH,
        },
        redirection_flags: GwRpchRedirectionFlags::default(),
        supports_header_signing: false,
        send_sequence_number: 0,
        receive_sequence_number: 0,
        responses: RpcAuthenticatedResponseReassembler::new(MAX_RESPONSE_STUB_SIZE),
        tunnel_message_responses: RpcAuthenticatedResponseReassembler::new(MAX_TUNNEL_MESSAGE_STUB_SIZE),
        call_id: 0,
        tunnel_message_call_id: None,
        receive_pipe_call_id: None,
        tunnel_context: None,
        channel_context: None,
        data: VecDeque::new(),
        messages: VecDeque::new(),
        failed: false,
        _out: PhantomData,
    };
    session.bind().await?;
    Ok(session)
}

async fn open_rpch_channels<Out, In>(
    out_stream: Out,
    in_stream: In,
    gateway_host: &str,
    target: &str,
    credentials: &RpchHttpCredentials,
    settings: RpchV2Settings,
) -> Result<(RpchChannel<Out, In>, RpchV2Setup, String), Error>
where
    Out: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    In: AsyncRead + AsyncWrite + Send + Unpin + 'static,
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

    let mut input = open_in_channel(in_stream, gateway_host, target, credentials, IN_CONTENT_LENGTH).await?;

    let session_id = association_group_id.to_string();
    let (out_stream, out_response) =
        open_out_channel(out_stream, gateway_host, target, &session_id, credentials, &out_body).await?;
    setup
        .accept_out_response(out_response.status)
        .map_err(|error| custom_err!("accept rpch OUT response", error))?;
    input.write_body(&in_initial_pdu).await?;
    input.flush().await?;

    let mut out = OutChannel::new(out_stream, out_response.content_length)?;
    while setup.state() != RpchV2State::Open {
        let pdu = out.read_pdu().await?;
        if is_rts_ping(&pdu) || decode_rts_flow_control_ack(&pdu).is_ok() {
            continue;
        }
        setup
            .receive_out_pdu(&pdu)
            .map_err(|error| custom_err!("decode rpch setup pdu", error))?;
    }

    let flow_control = setup
        .flow_control()
        .map_err(|error| custom_err!("create rpch flow control", error))?;
    let timeout = setup
        .in_channel_ping_timeout()
        .ok_or_else(|| Error::new("missing rpch connection timeout", GwErrorKind::Connect))?;
    Ok((
        RpchChannel {
            out,
            input,
            flow_control,
            connection_timeout: Duration::from_millis(u64::from(timeout)),
            _out: PhantomData,
        },
        setup,
        session_id,
    ))
}

pub(crate) struct RpchChannel<Out, In> {
    out: OutChannel,
    input: RpchInRequest<In>,
    flow_control: RpchFlowControl,
    connection_timeout: Duration,
    _out: PhantomData<fn() -> Out>,
}

impl<Out, In> RpchChannel<Out, In>
where
    Out: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    In: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    pub(crate) async fn open(
        out: Out,
        input: In,
        host: &str,
        target: &str,
        credentials: RpchHttpCredentials,
    ) -> Result<Self, Error> {
        let settings = RpchV2Settings::new(128 * 1024, 256 * 1024, 0)
            .map_err(|error| custom_err!("rpch channel settings", error))?;
        let (channel, _, _) = open_rpch_channels(out, input, host, target, &credentials, settings).await?;
        Ok(channel)
    }

    pub(crate) fn send_available_window(&self) -> u32 {
        self.flow_control.send_available_window()
    }

    pub(crate) fn connection_timeout(&self) -> Duration {
        self.connection_timeout
    }

    pub(crate) async fn send(&mut self, pdu: &[u8]) -> Result<(), Error> {
        let header =
            crate::rpc::RpcCommonHeader::decode(pdu).map_err(|error| custom_err!("validate rpch fragment", error))?;
        if usize::from(header.fragment_length()) != pdu.len() || header.ptype() == PTYPE_RTS {
            return Err(Error::new("expected one non-RTS rpc fragment", GwErrorKind::Encode));
        }
        if header.ptype() == crate::rpc::PTYPE_REQUEST {
            let fragment = decode_rpc_authenticated_fragment(pdu, u16::MAX)
                .map_err(|error| custom_err!("rpch request requires packet integrity", error))?;
            if fragment.security_trailer().auth_level < RPC_AUTH_LEVEL_PACKET_INTEGRITY {
                return Err(Error::new(
                    "rpch request requires packet integrity",
                    GwErrorKind::Encode,
                ));
            }
        }
        if self.input.remaining() < u32::from(header.fragment_length()) {
            return Err(Error::new(
                "live rpch IN body exhausted",
                GwErrorKind::UnsupportedFeature,
            ));
        }
        self.flow_control
            .sent_rpc_pdu(pdu.len())
            .map_err(|error| custom_err!("account rpch send window", error))?;
        write_in_pdu(&mut self.input, pdu).await
    }

    pub(crate) async fn receive(&mut self) -> Result<Vec<u8>, Error> {
        loop {
            let pdu = self.out.read_pdu().await?;
            let header = crate::rpc::RpcCommonHeader::decode(&pdu)
                .map_err(|error| custom_err!("validate rpch fragment", error))?;
            if header.ptype() == PTYPE_RTS {
                if is_rts_ping(&pdu) {
                    continue;
                }
                let ack = decode_rts_flow_control_ack(&pdu).map_err(|_| {
                    Error::new("live rpch RTS sequence is unsupported", GwErrorKind::UnsupportedFeature)
                })?;
                self.flow_control
                    .receive_flow_control_ack(ack)
                    .map_err(|error| custom_err!("apply rpch flow-control ack", error))?;
                continue;
            }
            self.flow_control
                .received_rpc_pdu(pdu.len())
                .map_err(|error| custom_err!("account rpch receive window", error))?;
            if let Some(ack) = self
                .flow_control
                .consumed_rpc_pdu(pdu.len())
                .map_err(|error| custom_err!("release rpch receive window", error))?
            {
                let ack = encode_rts_flow_control_ack_with_destination(ack, RtsDestination::OutProxy)
                    .map_err(|error| custom_err!("encode rpch flow-control ack", error))?;
                write_in_pdu(&mut self.input, &ack).await?;
            }
            return Ok(pdu);
        }
    }

    pub(crate) async fn ping(&mut self) -> Result<(), Error> {
        let pdu = encode_rts_ping().map_err(|error| custom_err!("encode rpch ping", error))?;
        write_in_pdu(&mut self.input, &pdu).await
    }
}

async fn open_in_channel<S>(
    stream: S,
    gateway_host: &str,
    target: &str,
    credentials: &RpchHttpCredentials,
    content_length: u32,
) -> Result<RpchInRequest<S>, Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let spn = http_spn(gateway_host)?;
    let mut auth = None;
    let mut use_basic = false;
    let mut input = RpchInRequest::open(stream, gateway_host, target, content_length, None).await?;

    for _ in 0..MAX_AUTH_ROUNDS {
        let response = input.receive_response().await?;
        match response.status {
            401 => {
                if use_basic {
                    return Err(Error::new("rpch IN basic authentication", GwErrorKind::HttpStatus(401)));
                }
                let authorization =
                    step_http_auth(&mut auth, credentials, &spn, &response.www_authenticate, &mut use_basic).await?;
                input = input.retry(Some(&authorization)).await?;
            }
            100 => {
                finish_http_auth(&mut auth, &response.www_authenticate).await?;
                return Ok(input);
            }
            200 => {
                return Err(Error::new(
                    "rpch IN request completed before its body",
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
    credentials: &RpchHttpCredentials,
    body: &[u8],
) -> Result<(S, crate::rpc_transport::RpchResponseHead), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let content_length =
        u32::try_from(body.len()).map_err(|_| Error::new("rpch CONN/A1 length", GwErrorKind::Encode))?;
    let spn = http_spn(gateway_host)?;
    let mut auth = None;
    let mut authorization = None;
    let mut use_basic = false;

    for _ in 0..MAX_AUTH_ROUNDS {
        write_rpch_request_head(
            &mut stream,
            RpchRequestHead {
                method: "RPC_OUT_DATA",
                host: gateway_host,
                target,
                content_length,
                authorization: authorization.as_deref(),
                cookie: None,
                session_id: Some(session_id),
                expect_continue: false,
            },
        )
        .await?;
        stream
            .write_all(body)
            .await
            .map_err(|error| custom_err!("write rpch CONN/A1", error))?;
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
                if use_basic {
                    return Err(Error::new(
                        "rpch OUT basic authentication",
                        GwErrorKind::HttpStatus(401),
                    ));
                }
                authorization = Some(
                    step_http_auth(&mut auth, credentials, &spn, &response.www_authenticate, &mut use_basic).await?,
                );
            }
            _ => {
                finish_http_auth(&mut auth, &response.www_authenticate).await?;
                return Ok((stream, response));
            }
        }
    }

    Err(Error::new(
        "rpch OUT authentication rounds exceeded",
        GwErrorKind::Connect,
    ))
}

async fn step_http_auth(
    http_auth: &mut Option<GatewayHttpAuth>,
    credentials: &RpchHttpCredentials,
    spn: &str,
    www_authenticate: &[String],
    use_basic: &mut bool,
) -> Result<String, Error> {
    let challenges = www_authenticate.to_vec();
    let step = match http_auth.take() {
        Some(mut auth) => {
            let (auth, step) = run_http_auth(move || {
                let challenges: Vec<&str> = challenges.iter().map(String::as_str).collect();
                let step = auth.step_www_authenticate(challenges)?;
                Ok((auth, step))
            })
            .await?;
            *http_auth = Some(auth);
            step
        }
        None => {
            let credentials = credentials.clone();
            let spn = spn.to_owned();
            let (new_auth, step) = run_http_auth(move || {
                let challenges: Vec<&str> = challenges.iter().map(String::as_str).collect();
                GatewayHttpAuth::from_challenges(
                    &credentials.username,
                    &credentials.password,
                    credentials.smart_card.as_ref(),
                    Some(spn),
                    &challenges,
                )
            })
            .await?;
            *http_auth = new_auth;
            step
        }
    };

    match step {
        AuthStep::Continue(authorization) => Ok(authorization),
        AuthStep::Complete => Err(Error::new(
            "rpch authentication completed without a successful response",
            GwErrorKind::Connect,
        )),
        AuthStep::TryBasic => {
            *use_basic = true;
            Ok(basic_authorization(&credentials.username, &credentials.password))
        }
    }
}

fn http_spn(authority: &str) -> Result<String, Error> {
    let authority = authority
        .parse::<hyper::http::uri::Authority>()
        .map_err(|_| Error::new("invalid rpch gateway authority", GwErrorKind::Connect))?;
    let host = authority.host().trim_matches(['[', ']']);
    Ok(format!("HTTP/{host}"))
}

async fn finish_http_auth(http_auth: &mut Option<GatewayHttpAuth>, www_authenticate: &[String]) -> Result<(), Error> {
    let Some(mut auth) = http_auth.take() else {
        return Ok(());
    };
    let challenges = www_authenticate.to_vec();
    auth = run_http_auth(move || {
        let challenges: Vec<&str> = challenges.iter().map(String::as_str).collect();
        auth.finish_www_authenticate(challenges)?;
        Ok(auth)
    })
    .await?;
    *http_auth = Some(auth);
    Ok(())
}

impl<Out, In, Provider> RpchSession<Out, In, Provider>
where
    Out: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    In: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    Provider: RpchStreamProvider<Out, In>,
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
        let authorization = decode_tsgu_authorize_tunnel_response(&response)
            .map_err(|error| custom_err!("decode authorize tunnel", error))?;
        self.redirection_flags = GwRpchRedirectionFlags {
            enable_all: authorization.redirection_flags.enable_all,
            disable_all: authorization.redirection_flags.disable_all,
            drive_disabled: authorization.redirection_flags.drive_disabled,
            printer_disabled: authorization.redirection_flags.printer_disabled,
            port_disabled: authorization.redirection_flags.port_disabled,
            clipboard_disabled: authorization.redirection_flags.clipboard_disabled,
            pnp_disabled: authorization.redirection_flags.pnp_disabled,
        };

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

    async fn send_to_server(&mut self, data: Bytes) -> Result<(), Error> {
        let channel_context = self
            .channel_context
            .ok_or_else(|| Error::new("rpch send before channel create", GwErrorKind::Connect))?;
        let request = TsProxySendToServerRequest::new(channel_context, &data)
            .and_then(|request| request.encode())
            .map_err(|error| custom_err!("encode send-to-server", error))?;
        let response = self.call(TSPROXY_SEND_TO_SERVER_OPNUM, &request).await?;
        let result = decode_tsgu_send_to_server_response(&response)
            .map_err(|error| custom_err!("decode send-to-server", error))?;
        if result == 0 {
            Ok(())
        } else {
            Err(Error::new("send-to-server failed", GwErrorKind::GatewayCode(result)))
        }
    }

    async fn close(&mut self) -> Result<(), Error> {
        if let Some(channel_context) = self.channel_context.take() {
            let response = self
                .call(
                    TSPROXY_CLOSE_CHANNEL_OPNUM,
                    &TsProxyCloseContextRequest::new(channel_context).encode(),
                )
                .await?;
            decode_tsgu_close_context_response(&response)
                .map_err(|error| custom_err!("decode close-channel response", error))?;
        }
        if let Some(tunnel_context) = self.tunnel_context.take() {
            let response = self
                .call(
                    TSPROXY_CLOSE_TUNNEL_OPNUM,
                    &TsProxyCloseContextRequest::new(tunnel_context).encode(),
                )
                .await?;
            decode_tsgu_close_context_response(&response)
                .map_err(|error| custom_err!("decode close-tunnel response", error))?;
        }
        Ok(())
    }

    async fn read_data(&mut self) -> Result<RpchRead, Error> {
        self.ensure_active()?;
        let result = self.read_data_inner().await;
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    async fn read_data_inner(&mut self) -> Result<RpchRead, Error> {
        if let Some(message) = self.messages.pop_front() {
            return Ok(RpchRead::Message(message));
        }
        if let Some(data) = self.data.pop_front() {
            return Ok(RpchRead::Data(data));
        }
        loop {
            let pdu = self.out.read_pdu().await?;
            if self.handle_out_pdu(pdu).await?.is_some() {
                return Err(Error::new("unexpected tsproxy response", GwErrorKind::Decode));
            }
            if let Some(message) = self.messages.pop_front() {
                return Ok(RpchRead::Message(message));
            }
            if let Some(data) = self.data.pop_front() {
                return Ok(RpchRead::Data(data));
            }
        }
    }

    async fn handle_out_pdu(&mut self, mut pdu: Vec<u8>) -> Result<Option<(u32, Vec<u8>)>, Error> {
        let pdu_len = pdu.len();
        self.flow_control
            .received_rpc_pdu(pdu_len)
            .map_err(|error| custom_err!("account rpch receive window", error))?;

        match pdu.get(2).copied() {
            Some(PTYPE_RTS) => {
                self.handle_rts_pdu(&pdu).await?;
                self.consume_out_pdu(pdu_len).await?;
                Ok(None)
            }
            Some(PTYPE_FAULT | PTYPE_RESPONSE) => {
                let fragment = self.verify_rpc_fragment(&mut pdu)?;
                if fragment.header().ptype() == PTYPE_FAULT {
                    return Err(Error::new("rpc fault on receive pipe", GwErrorKind::Connect));
                }
                let response = fragment
                    .decode_response_fragment_for_context(0)
                    .map_err(|error| custom_err!("decode rpch response", error))?;
                if self.tunnel_message_call_id == Some(response.call_id) {
                    self.handle_tunnel_message_response(pdu_len, fragment).await?;
                    return Ok(None);
                }
                if self.receive_pipe_call_id == Some(response.call_id) {
                    self.queue_receive_pipe_response(pdu_len, response).await?;
                    return Ok(None);
                }
                let response = self
                    .responses
                    .push_for_context(fragment, 0)
                    .map_err(|error| custom_err!("reassemble tsproxy response", error))?;
                self.consume_out_pdu(pdu_len).await?;
                Ok(response.map(|response| (response.call_id, response.stub)))
            }
            _ => Err(Error::new("unexpected rpch OUT pdu", GwErrorKind::Decode)),
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
        let initial_token = self.rpc_auth.initial_token()?;
        let bind = encode_rpc_bind_with_ntlm_auth(call_id, self.fragment_sizes, 0, &contexts, &initial_token)
            .map_err(|error| custom_err!("encode authenticated tsgu bind", error))?;
        self.write_in(&bind).await?;
        let response = self.out.read_pdu().await?;
        self.flow_control
            .received_rpc_pdu(response.len())
            .map_err(|error| custom_err!("account tsgu bind response", error))?;
        let binding = decode_rpc_bind_ack_with_ntlm_auth(&response, self.fragment_sizes)
            .map_err(|error| custom_err!("decode authenticated tsgu bind", error))?;
        if binding.bind_ack().call_id != call_id
            || binding.bind_ack().results.len() != 1
            || binding.bind_ack().results[0].result != 0
        {
            return Err(Error::new("tsproxy rpc bind rejected", GwErrorKind::Connect));
        }
        self.fragment_sizes = binding.bind_ack().fragment_sizes;
        self.supports_header_signing = binding.supports_header_signing();
        let response_token = binding.token().to_vec();
        let continuation_token = self.rpc_auth.continue_token(&response_token)?;
        let auth_3 = encode_rpc_auth_3(call_id, self.fragment_sizes, &continuation_token)
            .map_err(|error| custom_err!("encode tsgu rpc_auth_3", error))?;
        self.write_in(&auth_3).await?;
        self.consume_out_pdu(response.len()).await
    }

    async fn call(&mut self, opnum: u16, stub: &[u8]) -> Result<Vec<u8>, Error> {
        self.ensure_active()?;
        let result = self.call_inner(opnum, stub).await;
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    async fn call_inner(&mut self, opnum: u16, stub: &[u8]) -> Result<Vec<u8>, Error> {
        let call_id = self.next_call_id();
        self.send_call(call_id, opnum, stub).await?;
        loop {
            let pdu = self.out.read_pdu().await?;
            if let Some((response_call_id, response)) = self.handle_out_pdu(pdu).await? {
                if response_call_id != call_id {
                    return Err(Error::new("unexpected tsproxy response call ID", GwErrorKind::Decode));
                }
                return Ok(response);
            }
        }
    }

    async fn handle_tunnel_message_response(
        &mut self,
        pdu_len: usize,
        fragment: crate::rpc::RpcVerifiedAuthenticatedFragment<'_>,
    ) -> Result<(), Error> {
        let response = self
            .tunnel_message_responses
            .push_for_context(fragment, 0)
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

    async fn queue_receive_pipe_response(
        &mut self,
        pdu_len: usize,
        response: crate::rpc::RpcResponse<'_>,
    ) -> Result<(), Error> {
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
        self.data.push_back(Bytes::copy_from_slice(response.stub));
        self.consume_out_pdu(pdu_len).await
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
        self.ensure_active()?;
        for prepared in prepare_rpc_authenticated_request_fragments(
            call_id,
            0,
            opnum,
            stub,
            self.fragment_sizes,
            self.rpc_authentication,
        )
        .map_err(|error| custom_err!("encode tsproxy request", error))?
        {
            let mut fragment = prepared
                .finish(&[0; 16])
                .map_err(|error| custom_err!("finish authenticated tsproxy request", error))?;
            self.rpc_auth.sign_fragment(
                &mut fragment,
                self.rpc_authentication,
                self.send_sequence_number,
                self.supports_header_signing,
            )?;
            self.send_sequence_number = self
                .send_sequence_number
                .checked_add(1)
                .ok_or_else(|| Error::new("rpc send sequence exhausted", GwErrorKind::Connect))?;
            self.write_in(&fragment).await?;
        }
        Ok(())
    }

    fn ensure_active(&self) -> Result<(), Error> {
        if self.failed || !self.rpc_auth.is_complete() {
            return Err(Error::new("rpch session is unusable", GwErrorKind::Connect));
        }
        Ok(())
    }

    fn verify_rpc_fragment<'a>(
        &mut self,
        fragment: &'a mut [u8],
    ) -> Result<crate::rpc::RpcVerifiedAuthenticatedFragment<'a>, Error> {
        self.rpc_auth.verify_fragment(
            fragment,
            self.rpc_authentication,
            self.receive_sequence_number,
            self.supports_header_signing,
        )?;
        self.receive_sequence_number = self
            .receive_sequence_number
            .checked_add(1)
            .ok_or_else(|| Error::new("rpc receive sequence exhausted", GwErrorKind::Connect))?;

        let unverified = decode_rpc_authenticated_fragment(fragment, self.fragment_sizes.max_recv())
            .map_err(|error| custom_err!("decode authenticated rpch fragment", error))?;
        match unverified.verify_with(|_, _| Ok::<(), core::convert::Infallible>(())) {
            Ok(verified) => Ok(verified),
            Err(never) => match never {},
        }
    }

    async fn handle_rts_pdu(&mut self, pdu: &[u8]) -> Result<(), Error> {
        if is_rts_ping(pdu) {
            return Ok(());
        }
        match decode_rts_flow_control_ack(pdu) {
            Ok(ack) => {
                let _ = self
                    .flow_control
                    .receive_flow_control_ack(ack)
                    .map_err(|error| custom_err!("apply rpch flow-control ack", error))?;
                Ok(())
            }
            Err(_) => self.recycle_out_channel(pdu).await,
        }
    }

    async fn consume_out_pdu(&mut self, pdu_len: usize) -> Result<(), Error> {
        if let Some(ack) = self
            .flow_control
            .consumed_rpc_pdu(pdu_len)
            .map_err(|error| custom_err!("release rpch receive window", error))?
        {
            self.write_in(
                &encode_rts_flow_control_ack_with_destination(ack, RtsDestination::OutProxy)
                    .map_err(|error| custom_err!("encode rpch flow-control ack", error))?,
            )
            .await?;
        }
        Ok(())
    }

    async fn write_in(&mut self, pdu: &[u8]) -> Result<(), Error> {
        let length = u32::try_from(pdu.len()).map_err(|_| Error::new("rpch IN pdu length", GwErrorKind::Encode))?;
        if length
            .checked_add(IN_RECYCLE_RESERVE)
            .is_none_or(|required| required > self.input.remaining())
        {
            self.recycle_in_channel().await?;
        }
        self.flow_control
            .sent_rpc_pdu(pdu.len())
            .map_err(|error| custom_err!("account rpch send window", error))?;
        let result = write_in_pdu(&mut self.input, pdu).await;
        if result.is_err() {
            self.failed = true;
        }
        result?;
        Ok(())
    }

    async fn recycle_in_channel(&mut self) -> Result<(), Error> {
        let Self {
            out,
            input,
            provider,
            setup,
            gateway_host,
            target,
            credentials,
            flow_control,
            ping,
            ..
        } = self;
        let successor_stream = provider.open_in().await?;
        let mut successor =
            open_in_channel(successor_stream, gateway_host, target, credentials, IN_CONTENT_LENGTH).await?;
        let successor_cookie = RtsCookie::new(*Uuid::new_v4().as_bytes());
        let mut recycle = setup
            .channel_recycling()
            .map_err(|error| custom_err!("start rpch IN recycling", error))?;
        let a1 = recycle
            .start_in_recycling(successor_cookie)
            .map_err(|error| custom_err!("encode rpch IN_R1/A1", error))?;
        write_in_pdu(&mut successor, &a1).await?;
        let a4 = out.read_pdu().await?;
        let a5 = recycle
            .receive_in_recycle_a4(&a4)
            .map_err(|error| custom_err!("decode rpch IN_R1/A4", error))?;
        write_in_pdu(input, &a5).await?;
        *input = successor;
        recycle
            .finish_in_recycling(flow_control, ping)
            .map_err(|error| custom_err!("finish rpch IN recycling", error))
    }

    async fn recycle_out_channel(&mut self, a2: &[u8]) -> Result<(), Error> {
        let Self {
            out,
            input,
            provider,
            setup,
            gateway_host,
            target,
            session_id,
            credentials,
            flow_control,
            ..
        } = self;
        let successor_stream = provider.open_out().await?;
        let successor_cookie = RtsCookie::new(*Uuid::new_v4().as_bytes());
        let mut recycle = setup
            .channel_recycling()
            .map_err(|error| custom_err!("start rpch OUT recycling", error))?;
        let a3 = recycle
            .receive_out_recycle_a2(a2, successor_cookie)
            .map_err(|error| custom_err!("decode rpch OUT_R1/A2", error))?;
        let (successor_stream, response) =
            open_out_channel(successor_stream, gateway_host, target, session_id, credentials, &a3).await?;
        if response.status != 200 {
            return Err(Error::new(
                "rpch OUT replacement rejected",
                GwErrorKind::HttpStatus(response.status),
            ));
        }
        let a6 = out.read_pdu().await?;
        let a7 = recycle
            .receive_out_recycle_a6(&a6)
            .map_err(|error| custom_err!("decode rpch OUT_R1/A6", error))?;
        write_in_pdu(input, &a7).await?;
        let mut successor = OutChannel::new(successor_stream, response.content_length)?;
        let a10 = successor.read_pdu().await?;
        let a11 = recycle
            .receive_out_recycle_a10(&a10)
            .map_err(|error| custom_err!("decode rpch OUT_R1/A10", error))?;
        write_in_pdu(input, &a11).await?;
        *out = successor;
        recycle
            .finish_out_recycling(flow_control)
            .map_err(|error| custom_err!("finish rpch OUT recycling", error))
    }

    fn next_call_id(&mut self) -> u32 {
        self.call_id = self.call_id.wrapping_add(1);
        self.call_id
    }
}

struct UnsupportedChannelRecycling;

impl<'out, 'input>
    RpchStreamProvider<
        Box<dyn crate::packet_io::GatewayIo + Send + 'out>,
        Box<dyn crate::packet_io::GatewayIo + Send + 'input>,
    > for UnsupportedChannelRecycling
{
    async fn open_in(&mut self) -> Result<Box<dyn crate::packet_io::GatewayIo + Send + 'input>, Error> {
        Err(Error::new(
            "rpch IN channel recycling is unavailable",
            GwErrorKind::UnsupportedFeature,
        ))
    }

    async fn open_out(&mut self) -> Result<Box<dyn crate::packet_io::GatewayIo + Send + 'out>, Error> {
        Err(Error::new(
            "rpch OUT channel recycling is unavailable",
            GwErrorKind::UnsupportedFeature,
        ))
    }
}

enum StreamCommand {
    Write(Bytes, tokio::sync::oneshot::Sender<Result<(), String>>),
    Shutdown(tokio::sync::oneshot::Sender<Result<(), String>>),
}

/// Authenticated TS Proxy channel exposed as an asynchronous RDP byte stream.
///
/// RPC packet integrity is established internally with NTLM. TLS/HTTP authentication remains a
/// separate gateway security layer. The bounded channel lifetime is not recycled; a server RTS
/// recycle request or exhausted IN body terminates the stream explicitly.
pub struct RpchStream {
    commands: PollSender<StreamCommand>,
    received: mpsc::Receiver<Result<Bytes, String>>,
    read_buffer: Option<Bytes>,
    pending_write: Option<(usize, tokio::sync::oneshot::Receiver<Result<(), String>>)>,
    shutdown: Option<tokio::sync::oneshot::Receiver<Result<(), String>>>,
    actor: JoinHandle<()>,
    redirection_flags: GwRpchRedirectionFlags,
}

impl RpchStream {
    /// Opens an authenticated MS-RPCH tunnel and returns its local TCP address.
    ///
    /// RPC packet integrity is always negotiated and verified independently of TLS and HTTP
    /// authentication. Smart-card RPC authentication and channel recycling are not supported.
    pub async fn connect_with_port_and_certificate_validation(
        target: &GwConnectTarget,
        client_name: &str,
        server_port: u16,
        certificate_validation: ironrdp_tls::CertificateValidation,
        certificate_validation_callback: Option<ironrdp_tls::CertificateValidationCallback>,
    ) -> Result<(Self, core::net::SocketAddr), Error> {
        if target.smart_card.is_some() {
            return Err(Error::new(
                "rpch smart-card authentication is unsupported",
                GwErrorKind::UnsupportedFeature,
            ));
        }
        if certificate_validation == ironrdp_tls::CertificateValidation::DangerouslyAcceptInvalidCertificate
            && certificate_validation_callback.is_some()
        {
            return Err(Error::new(
                "certificate validation",
                GwErrorKind::InvalidCertificateValidation,
            ));
        }

        let (out, input, client_addr) = crate::packet_io::open_rpch_gateway_channels(
            &target.gw_endpoint,
            certificate_validation,
            certificate_validation_callback,
        )
        .await?;
        let stream = Self::connect(out, input, target, client_name, server_port).await?;
        Ok((stream, client_addr))
    }

    /// Returns device-redirection restrictions from `TsProxyAuthorizeTunnel`.
    pub const fn redirection_flags(&self) -> GwRpchRedirectionFlags {
        self.redirection_flags
    }

    pub(crate) async fn connect(
        out: crate::packet_io::GatewayStream,
        input: crate::packet_io::GatewayStream,
        target: &GwConnectTarget,
        client_name: &str,
        port: u16,
    ) -> Result<Self, Error> {
        let settings = RpchV2Settings::new(128 * 1024, 256 * 1024, 0)
            .map_err(|error| custom_err!("rpch channel settings", error))?;
        let mut session: RpchSession<
            crate::packet_io::GatewayStream,
            crate::packet_io::GatewayStream,
            UnsupportedChannelRecycling,
        > = rpch_connect::<
            crate::packet_io::GatewayStream,
            crate::packet_io::GatewayStream,
            UnsupportedChannelRecycling,
        >(
            out,
            input,
            UnsupportedChannelRecycling,
            &target.gw_endpoint,
            &target.server,
            RpchHttpCredentials {
                username: target.gw_user.clone(),
                password: target.gw_pass.clone(),
                smart_card: None,
            },
            settings,
        )
        .await?;
        session.open_tunnel(client_name, &target.server, port).await?;
        let redirection_flags = session.redirection_flags;

        let (command_tx, command_rx) = mpsc::channel(8);
        let (received_tx, received_rx) = mpsc::channel(8);
        let actor = tokio::spawn(async move {
            if let Err(error) = run_rpch_stream(session, command_rx, received_tx.clone()).await {
                let _ = received_tx.send(Err(error.to_string())).await;
            }
        });
        Ok(Self {
            commands: PollSender::new(command_tx),
            received: received_rx,
            read_buffer: None,
            pending_write: None,
            shutdown: None,
            actor,
            redirection_flags,
        })
    }
}

impl Drop for RpchStream {
    fn drop(&mut self) {
        self.actor.abort();
    }
}

impl AsyncRead for RpchStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        loop {
            if let Some(bytes) = self.read_buffer.as_mut() {
                let count = bytes.len().min(buf.remaining());
                let chunk = bytes.split_to(count);
                buf.put_slice(&chunk);
                if bytes.is_empty() {
                    self.read_buffer = None;
                }
                return Poll::Ready(Ok(()));
            }
            match Pin::new(&mut self.received).poll_recv(cx) {
                Poll::Ready(Some(Ok(bytes))) => self.read_buffer = Some(bytes),
                Poll::Ready(Some(Err(error))) => {
                    return Poll::Ready(Err(io::Error::other(error)));
                }
                Poll::Ready(None) => {
                    return Poll::Ready(Err(io::Error::new(io::ErrorKind::UnexpectedEof, "rpch stream closed")));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl AsyncWrite for RpchStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        if let Some((count, response)) = &mut self.pending_write {
            return match Pin::new(response).poll(cx) {
                Poll::Ready(Ok(Ok(()))) => {
                    let count = *count;
                    self.pending_write = None;
                    Poll::Ready(Ok(count))
                }
                Poll::Ready(Ok(Err(error))) => {
                    self.pending_write = None;
                    Poll::Ready(Err(io::Error::other(error)))
                }
                Poll::Ready(Err(error)) => {
                    self.pending_write = None;
                    Poll::Ready(Err(io::Error::other(error)))
                }
                Poll::Pending => Poll::Pending,
            };
        }

        match self.commands.poll_reserve(cx) {
            Poll::Ready(Ok(())) => {
                let count = buf.len().min(16 * 1024);
                let (response_tx, response_rx) = tokio::sync::oneshot::channel();
                if self
                    .commands
                    .send_item(StreamCommand::Write(Bytes::copy_from_slice(&buf[..count]), response_tx))
                    .is_err()
                {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "rpch stream actor stopped",
                    )));
                }
                self.pending_write = Some((count, response_rx));
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Poll::Ready(Err(_)) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "rpch stream is shut down",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.pending_write.is_some() {
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.shutdown.is_none() {
            match self.commands.poll_reserve(cx) {
                Poll::Ready(Ok(())) => {
                    let (response_tx, response_rx) = tokio::sync::oneshot::channel();
                    if self.commands.send_item(StreamCommand::Shutdown(response_tx)).is_err() {
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "rpch stream actor stopped",
                        )));
                    }
                    self.shutdown = Some(response_rx);
                }
                Poll::Ready(Err(_)) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "rpch stream actor stopped",
                    )));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        match Pin::new(self.shutdown.as_mut().expect("shutdown receiver initialized")).poll(cx) {
            Poll::Ready(Ok(Ok(()))) => Poll::Ready(Ok(())),
            Poll::Ready(Ok(Err(error))) => Poll::Ready(Err(io::Error::other(error))),
            Poll::Ready(Err(error)) => Poll::Ready(Err(io::Error::other(error))),
            Poll::Pending => Poll::Pending,
        }
    }
}

type GatewayRpchSession =
    RpchSession<crate::packet_io::GatewayStream, crate::packet_io::GatewayStream, UnsupportedChannelRecycling>;

async fn run_rpch_stream(
    mut session: GatewayRpchSession,
    mut commands: mpsc::Receiver<StreamCommand>,
    received: mpsc::Sender<Result<Bytes, String>>,
) -> Result<(), Error> {
    let started = Instant::now();
    let interval = (session.connection_timeout / 2).max(Duration::from_secs(1));
    let mut keepalive = tokio::time::interval(interval);
    keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);
    keepalive.tick().await;

    loop {
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(StreamCommand::Write(bytes, response)) => {
                        let result = session.send_to_server(bytes).await;
                        let response_result = match &result {
                            Ok(()) => Ok(()),
                            Err(error) => Err(error.to_string()),
                        };
                        let _ = response.send(response_result);
                        result?;
                        send_queued_stream_data(&mut session, received.clone()).await?;
                    }
                    Some(StreamCommand::Shutdown(response)) => {
                        let result = session.close().await;
                        let response_result = match &result {
                            Ok(()) => Ok(()),
                            Err(error) => Err(error.to_string()),
                        };
                        let _ = response.send(response_result);
                        return result;
                    }
                    None => return Ok(()),
                }
            }
            pdu = session.out.read_pdu() => {
                let pdu = pdu?;
                if session.handle_out_pdu(pdu).await?.is_some() {
                    return Err(Error::new("unexpected tsproxy response", GwErrorKind::Decode));
                }
                send_queued_stream_data(&mut session, received.clone()).await?;
            }
            _ = keepalive.tick() => {
                let now = started.elapsed();
                if session.ping_due(now) {
                    session.send_ping(now).await?;
                }
            }
        }
    }
}

async fn send_queued_stream_data(
    session: &mut GatewayRpchSession,
    received: mpsc::Sender<Result<Bytes, String>>,
) -> Result<(), Error> {
    while let Some(message) = session.messages.pop_front() {
        match message {
            TsProxyTunnelMessage::None => {}
            TsProxyTunnelMessage::Service { text, .. } => log::warn!("RD Gateway service message: {text}"),
            TsProxyTunnelMessage::Consent { .. } => {
                return Err(Error::new(
                    "rpch consent message requires an application callback",
                    GwErrorKind::UnsupportedFeature,
                ));
            }
            TsProxyTunnelMessage::Reauthenticate { .. } => {
                return Err(Error::new(
                    "rpch HTTP reauthentication is unsupported",
                    GwErrorKind::UnsupportedFeature,
                ));
            }
        }
    }
    while let Some(data) = session.data.pop_front() {
        received
            .clone()
            .reserve_owned()
            .await
            .map_err(|_| Error::new("rpch stream reader closed", GwErrorKind::Connect))?
            .send(Ok(data));
    }
    Ok(())
}

async fn write_in_pdu<S>(input: &mut RpchInRequest<S>, pdu: &[u8]) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    input.write_body(pdu).await?;
    input.flush().await
}

fn is_rts_ping(pdu: &[u8]) -> bool {
    encode_rts_ping().is_ok_and(|ping| pdu == ping)
}

#[derive(Debug)]
enum RpchRead {
    Data(Bytes),
    Message(TsProxyTunnelMessage),
}

struct OutChannel {
    receiver: mpsc::Receiver<Result<Vec<u8>, Error>>,
    reader: JoinHandle<()>,
    remaining: u32,
}

impl OutChannel {
    fn new<S>(mut stream: S, content_length: Option<u32>) -> Result<Self, Error>
    where
        S: AsyncRead + Send + Unpin + 'static,
    {
        let remaining = content_length
            .filter(|length| (128 * 1024..=2 * 1024 * 1024 * 1024).contains(length))
            .ok_or_else(|| Error::new("invalid rpch OUT content length", GwErrorKind::Decode))?;
        let (sender, receiver) = mpsc::channel(2);
        let reader = tokio::spawn(async move {
            let mut remaining = remaining;
            while remaining >= 16 {
                let mut header = [0; 16];
                if let Err(error) = stream.read_exact(&mut header).await {
                    let _ = sender.send(Err(custom_err!("read rpch pdu header", error))).await;
                    return;
                }
                let common = match crate::rpc::RpcCommonHeader::decode_prefix(&header) {
                    Ok(header) => header,
                    Err(error) => {
                        let _ = sender
                            .send(Err(
                                Error::new("validate rpch pdu header", GwErrorKind::Decode).with_source(error)
                            ))
                            .await;
                        return;
                    }
                };
                let length = usize::from(common.fragment_length());
                if remaining < u32::from(common.fragment_length()) {
                    let _ = sender
                        .send(Err(Error::new("rpch pdu exceeds OUT body", GwErrorKind::Decode)))
                        .await;
                    return;
                }
                let mut pdu = header.to_vec();
                pdu.resize(length, 0);
                if let Err(error) = stream.read_exact(&mut pdu[header.len()..]).await {
                    let _ = sender.send(Err(custom_err!("read rpch pdu body", error))).await;
                    return;
                }
                remaining -= u32::from(common.fragment_length());
                if sender.send(Ok(pdu)).await.is_err() {
                    return;
                }
            }
            let _ = sender
                .send(Err(Error::new(
                    "rpch OUT body exhausted",
                    GwErrorKind::UnsupportedFeature,
                )))
                .await;
        });
        Ok(Self {
            receiver,
            reader,
            remaining,
        })
    }

    async fn read_pdu(&mut self) -> Result<Vec<u8>, Error> {
        match self.receiver.recv().await {
            Some(result) => result,
            None => Err(Error::new("rpch OUT reader stopped", GwErrorKind::Connect)),
        }
    }
}

impl Drop for OutChannel {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};

    use super::*;
    use crate::mock_rpch::{MockRpchScenario, MockTunnelMessage, mock_rpch_proxy};

    #[tokio::test]
    async fn rpch_out_requires_bounded_http_content_length() {
        for length in [None, Some(0), Some(131071), Some(2147483649)] {
            assert!(OutChannel::new(tokio::io::empty(), length).is_err());
        }
        for length in [131072, 2147483648] {
            let channel = OutChannel::new(tokio::io::empty(), Some(length)).expect("valid OUT body length");
            assert_eq!(channel.remaining, length);
        }
    }

    struct NoRecycle;

    impl RpchStreamProvider<DuplexStream, DuplexStream> for NoRecycle {
        async fn open_in(&mut self) -> Result<DuplexStream, Error> {
            Err(Error::new("unexpected IN channel recycle", GwErrorKind::Connect))
        }

        async fn open_out(&mut self) -> Result<DuplexStream, Error> {
            Err(Error::new("unexpected OUT channel recycle", GwErrorKind::Connect))
        }
    }

    async fn open_session(
        scenario: MockRpchScenario,
    ) -> (RpchSession<DuplexStream, DuplexStream, NoRecycle>, JoinHandle<()>) {
        let (out, input, proxy) = mock_rpch_proxy(scenario);
        let mut session = rpch_connect(
            out,
            input,
            NoRecycle,
            "rdg.contoso.com",
            "rdp.contoso.com:3389",
            RpchHttpCredentials {
                username: "CONTOSO\\alice".to_owned(),
                password: "secret".to_owned(),
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

    async fn open_stream(scenario: MockRpchScenario) -> (RpchStream, JoinHandle<()>) {
        let (out, input, proxy) = mock_rpch_proxy(scenario);
        let target = GwConnectTarget {
            gw_endpoint: "rdg.contoso.com".to_owned(),
            gw_user: "CONTOSO\\alice".to_owned(),
            gw_pass: "secret".to_owned(),
            smart_card: None,
            server: "rdp.contoso.com:3389".to_owned(),
        };
        let stream = RpchStream::connect(Box::new(out), Box::new(input), &target, "test-client", 3389)
            .await
            .expect("authenticated RPCH stream");
        (stream, proxy)
    }

    #[tokio::test]
    async fn rpch_session_runs_ntlm_authenticated_tsproxy_echo() {
        let (mut session, proxy) = open_session(MockRpchScenario::Echo).await;
        session
            .send_to_server(Bytes::from_static(b"hello-rdp"))
            .await
            .expect("send target data");
        match session.read_data().await.expect("read target data") {
            RpchRead::Data(data) => assert_eq!(&*data, b"hello-rdp"),
            event => panic!("expected target data, got {event:?}"),
        }
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_poisoned_auth_context_rejects_later_requests() {
        let (out, input, proxy) = mock_rpch_proxy(MockRpchScenario::TamperedResponse);
        let mut session = rpch_connect(
            out,
            input,
            NoRecycle,
            "rdg.contoso.com",
            "rdp.contoso.com:3389",
            RpchHttpCredentials {
                username: "CONTOSO\\alice".to_owned(),
                password: "secret".to_owned(),
                smart_card: None,
            },
            RpchV2Settings::new(128 * 1024, 256 * 1024, 0).expect("valid test settings"),
        )
        .await
        .expect("rpch connect");

        assert!(
            session
                .open_tunnel("test-client", "rdp.contoso.com", 3389)
                .await
                .is_err()
        );
        assert!(!session.rpc_auth.is_complete());
        assert!(session.send_call(2, TSPROXY_CREATE_TUNNEL_OPNUM, &[]).await.is_err());
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_authenticates_each_http_channel_independently() {
        let (mut session, proxy) = open_session(MockRpchScenario::BasicAuthentication).await;
        session
            .send_to_server(Bytes::from_static(b"authenticated"))
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
        let data = Bytes::from(data);
        session.send_to_server(data.clone()).await.expect("send target data");
        let read = session.read_data().await;
        assert!(
            matches!(read, Ok(RpchRead::Data(_))),
            "expected target data, got {read:?}"
        );
        assert_eq!(session.flow_control.send_available_window(), 8 * 1024);
        session
            .send_to_server(data)
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
            NoRecycle,
            "rdg.contoso.com",
            "rdp.contoso.com:3389",
            RpchHttpCredentials {
                username: "CONTOSO\\alice".to_owned(),
                password: "secret".to_owned(),
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

    #[tokio::test]
    async fn rpch_stream_reads_while_writes_are_in_flight_and_preserves_chunks() {
        let (stream, proxy) = open_stream(MockRpchScenario::FragmentedReceivePipePdu).await;
        let (mut reader, mut writer) = tokio::io::split(stream);
        let mut response = [0; 10];

        let (read_result, write_result) = tokio::join!(reader.read_exact(&mut response), async {
            writer.write_all(b"first-").await?;
            writer.write_all(b"last").await
        });

        read_result.expect("read response while writing");
        write_result.expect("write both chunks");
        assert_eq!(&response, b"first-last");
        writer.shutdown().await.expect("close TS Proxy contexts");
        drop(reader);
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_stream_read_cancellation_does_not_lose_inbound_framing() {
        let (mut stream, proxy) = open_stream(MockRpchScenario::Echo).await;
        let mut scratch = [0; 8];
        assert!(
            tokio::time::timeout(Duration::from_millis(10), stream.read(&mut scratch))
                .await
                .is_err()
        );

        stream
            .write_all(b"after-cancel")
            .await
            .expect("write after cancelled read");
        let mut response = [0; 12];
        stream.read_exact(&mut response).await.expect("read framed response");
        assert_eq!(&response, b"after-cancel");
        stream.shutdown().await.expect("close TS Proxy contexts");
        assert!(stream.actor.is_finished());
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_stream_tampered_receive_pipe_response_fails_closed() {
        let (mut stream, proxy) = open_stream(MockRpchScenario::TamperedReceivePipeResponse).await;
        stream.write_all(b"request").await.expect("send authenticated request");
        let mut response = [0; 7];
        let error = tokio::time::timeout(Duration::from_secs(1), stream.read_exact(&mut response))
            .await
            .expect("tamper error is delivered")
            .expect_err("tampered response must fail");
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert!(stream.actor.is_finished());
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_stream_shutdown_closes_channel_and_tunnel() {
        let (mut stream, proxy) = open_stream(MockRpchScenario::Echo).await;
        stream.shutdown().await.expect("close TS Proxy contexts");
        assert!(stream.actor.is_finished());
        assert!(
            tokio::time::timeout(Duration::from_secs(1), proxy)
                .await
                .expect("proxy observes stream shutdown")
                .is_ok()
        );
    }
}
