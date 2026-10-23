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

use futures_util::future::BoxFuture;
use hyper::body::Bytes;
use log::warn;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadBuf, WriteHalf};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::PollSender;
use uuid::Uuid;

use crate::http_auth::{AuthStep, GatewayHttpAuth, basic_authorization, run_http_auth};
use crate::rpc::tsgu::{
    NDR32_TRANSFER_SYNTAX_ID, NDR32_TRANSFER_SYNTAX_VERSION, NonNullRpcContextHandle, TSG_CLIENT_CAPABILITIES,
    TSG_MESSAGING_CAP_REAUTH, TSG_MESSAGING_CAP_SERVICE_MSG, TSPROXY_AUTHORIZE_TUNNEL_OPNUM,
    TSPROXY_CLOSE_CHANNEL_OPNUM, TSPROXY_CLOSE_TUNNEL_OPNUM, TSPROXY_CREATE_CHANNEL_OPNUM, TSPROXY_CREATE_TUNNEL_OPNUM,
    TSPROXY_MAKE_TUNNEL_CALL_OPNUM, TSPROXY_RPC_INTERFACE_ID, TSPROXY_RPC_INTERFACE_VERSION,
    TSPROXY_SEND_TO_SERVER_OPNUM, TSPROXY_SETUP_RECEIVE_PIPE_OPNUM, TsProxyAuthorizeTunnelRequest,
    TsProxyCloseContextRequest, TsProxyCreateChannelRequest, TsProxyCreateTunnelRequest, TsProxyCreateTunnelResponse,
    TsProxyMakeTunnelCallRequest, TsProxySendToServerRequest, TsProxySetupReceivePipeRequest, TsProxyTunnelMessage,
    decode_tsgu_authorize_tunnel_response, decode_tsgu_cancel_tunnel_call_response, decode_tsgu_close_context_response,
    decode_tsgu_create_channel_response, decode_tsgu_create_tunnel_response, decode_tsgu_make_tunnel_call_response,
    decode_tsgu_send_to_server_response, decode_tsgu_setup_receive_pipe_final_response,
};
use crate::rpc::{
    PTYPE_FAULT, PTYPE_RESPONSE, PTYPE_RTS, RPC_AUTH_LEVEL_PACKET_INTEGRITY, RPC_AUTH_TYPE_WINNT,
    RPC_NTLM_VERIFIER_LENGTH, RpcAuthenticatedResponseReassembler, RpcAuthenticationInfo, RpcFragmentSizes,
    RpcNtlmAuth, RpcPresentationContext, RpcSyntaxIdentifier, RpchFlowControl, RpchInChannelRecycleState,
    RpchOutChannelRecycleState, RpchPingSchedule, RpchV2Settings, RpchV2Setup, RpchV2State, RtsCookie, RtsDestination,
    decode_rpc_authenticated_fragment, decode_rpc_bind_ack_with_ntlm_auth, decode_rpc_fault_for_context,
    decode_rts_flow_control_ack, decode_rts_in_recycle_a4, decode_rts_out_recycle_a2, decode_rts_out_recycle_a6,
    decode_rts_out_recycle_a10, decode_rts_out_recycle_r2_a6, decode_rts_out_recycle_r2_b3, encode_rpc_auth_3,
    encode_rpc_bind_with_ntlm_auth, encode_rts_flow_control_ack_with_destination, encode_rts_out_recycle_a11,
    encode_rts_ping, prepare_rpc_authenticated_request_fragments,
};
use crate::rpc_transport::{
    RpchCookieJar, RpchInRequest, RpchRequestHead, RpchResponseHead, drain_body, read_rpch_response_head,
    write_rpch_request_head,
};
use crate::{Error, GwConnectTarget, GwErrorKind, GwRpchRedirectionFlags, GwSmartCardCredentials};

/// IN body reserved by the low-level channel, which does not recycle and stops at this limit.
const CHANNEL_IN_CONTENT_LENGTH: u32 = 128 * 1024;
/// IN body reserved by TS Proxy sessions; it matches the default channel lifetime, as deployed proxies expect.
const SESSION_IN_CONTENT_LENGTH: u32 = 1024 * 1024 * 1024;
const MAX_RESPONSE_STUB_SIZE: usize = 32 * 1024;
const MAX_MESSAGE_STUB_SIZE: usize = 2 * 65_536 + 1024;
const MAX_AUTH_ROUNDS: usize = 8;
/// IN body kept free for RTS PDUs, such as IN_R1/A5 and flow-control acknowledgements, sent before a recycle completes.
const IN_RECYCLE_RESERVE: u32 = 1024;
/// Target data received while a write waits for send window; exceeding it fails the session instead of growing memory.
const MAX_BUFFERED_DATA: usize = 1024 * 1024;
/// RPC proxy backend for the TS Proxy interface: the MS-TSGU 2.1 port 3388 endpoint on the gateway itself.
///
/// The RPC proxy resolves the backend name on the gateway, so `localhost` names the local TS Proxy endpoint
/// regardless of the external gateway host name, not the RDP target.
const TSPROXY_RPC_BACKEND: &str = "localhost:3388";

#[derive(Clone)]
pub(crate) struct RpchHttpCredentials {
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) smart_card: Option<GwSmartCardCredentials>,
}

pub(crate) trait RpchStreamProvider<Out, In>: Send {
    fn open_in(&mut self) -> impl Future<Output = Result<In, Error>> + Send;
    fn open_out(&mut self) -> impl Future<Output = Result<Out, Error>> + Send;
    /// Opens the OUT and IN streams of a separate virtual connection that answers a reauthentication request.
    ///
    /// The returned future owns its inputs so it can run outside the session.
    fn open_reauthentication(&mut self) -> BoxFuture<'static, Result<(Out, In), Error>>;
}

/// Provider for virtual connections that never open successor or reauthentication channels.
struct NoSuccessorChannels;

impl<Out: Send + 'static, In: Send + 'static> RpchStreamProvider<Out, In> for NoSuccessorChannels {
    async fn open_in(&mut self) -> Result<In, Error> {
        Err(Error::new(
            "rpch connection has no successor channels",
            GwErrorKind::UnsupportedFeature,
        ))
    }

    async fn open_out(&mut self) -> Result<Out, Error> {
        Err(Error::new(
            "rpch connection has no successor channels",
            GwErrorKind::UnsupportedFeature,
        ))
    }

    fn open_reauthentication(&mut self) -> BoxFuture<'static, Result<(Out, In), Error>> {
        Box::pin(core::future::ready(Err(Error::new(
            "rpch connection cannot reauthenticate",
            GwErrorKind::UnsupportedFeature,
        ))))
    }
}

/// TS Proxy channel parameters, kept to repeat them on a reauthentication connection.
#[derive(Clone)]
struct TunnelTarget {
    client_name: String,
    resource: String,
    port: u16,
}

struct RpchSession<Out, In, Provider> {
    out: OutChannel,
    input: RpchInRequest<WriteHalf<In>>,
    in_monitor: InMonitor,
    in_successor: Option<(RpchInRequest<WriteHalf<In>>, InMonitor)>,
    out_successor: Option<OutSuccessor<Out>>,
    in_content_length: u32,
    out_auth_rounds: usize,
    cookies: RpchCookieJar,
    provider: Provider,
    setup: RpchV2Setup,
    gateway_host: String,
    target: String,
    session_id: String,
    credentials: RpchHttpCredentials,
    flow_control: RpchFlowControl,
    ping: RpchPingSchedule,
    started: Instant,
    fragment_sizes: RpcFragmentSizes,
    rpc_auth: RpcNtlmAuth,
    rpc_authentication: RpcAuthenticationInfo,
    redirection_flags: GwRpchRedirectionFlags,
    supports_header_signing: bool,
    send_sequence_number: u32,
    receive_sequence_number: u32,
    responses: RpcAuthenticatedResponseReassembler,
    message_responses: RpcAuthenticatedResponseReassembler,
    active_call_id: Option<u32>,
    call_id: u32,
    receive_pipe_call_id: Option<u32>,
    tunnel_context: Option<NonNullRpcContextHandle>,
    channel_context: Option<NonNullRpcContextHandle>,
    tunnel_target: Option<TunnelTarget>,
    /// Capabilities the server returned in `TsProxyCreateTunnel`.
    capabilities: u32,
    /// Keeps one `TsProxyMakeTunnelCall` pending while the server delivers administrative messages.
    messaging: bool,
    message_call_id: Option<u32>,
    /// Tunnel context from a reauthentication message that has not started its connection yet.
    pending_reauth: Option<u64>,
    /// Set once close starts, so responses that tear-down provokes are expected rather than failures.
    closing: bool,
    data: VecDeque<Bytes>,
    buffered_data: usize,
    failed: bool,
}

struct OutSuccessor<Out> {
    channel: OutChannel,
    writer: WriteHalf<Out>,
}

/// Watches the read half of an accepted IN channel, where the proxy writes only error responses.
///
/// [MS-RPCH] 2.1.2.1.3 and 3.2.2.5.11.
struct InMonitor {
    failure: Option<oneshot::Receiver<Error>>,
    task: JoinHandle<()>,
}

impl InMonitor {
    fn spawn<R>(mut read: R) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
    {
        let (sender, failure) = oneshot::channel();
        let task = tokio::spawn(async move {
            let error = match read_rpch_response_head(&mut read).await {
                Ok(response) => in_channel_response_error(&response),
                Err(error) => Error::new("rpch IN channel closed", GwErrorKind::Connect).with_source(error),
            };
            let _ = sender.send(error);
        });
        Self {
            failure: Some(failure),
            task,
        }
    }

    /// Resolves when the proxy fails the IN channel; pending forever once a failure was reported.
    async fn failed(&mut self) -> Error {
        let Some(failure) = self.failure.as_mut() else {
            return core::future::pending().await;
        };
        let result = failure.await;
        self.failure = None;
        result.unwrap_or_else(|_| Error::new("rpch IN monitor stopped", GwErrorKind::Connect))
    }
}

impl Drop for InMonitor {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn in_channel_response_error(response: &RpchResponseHead) -> Error {
    match response.rpc_error() {
        Some(code) => Error::new("rpch IN channel failed", GwErrorKind::GatewayCode(code)),
        None => Error::new("rpch IN channel failed", GwErrorKind::HttpStatus(response.status)),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "internal constructor shared by the stream and tests"
)]
async fn rpch_connect<Out, In, Provider>(
    out_stream: Out,
    in_stream: In,
    provider: Provider,
    gateway_host: &str,
    target: &str,
    credentials: RpchHttpCredentials,
    settings: RpchV2Settings,
    in_content_length: u32,
) -> Result<RpchSession<Out, In, Provider>, Error>
where
    Out: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    In: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    Provider: RpchStreamProvider<Out, In>,
{
    let OpenedChannels {
        channel,
        setup,
        session_id,
        cookies,
        out_auth_rounds,
    } = open_rpch_channels(
        out_stream,
        in_stream,
        gateway_host,
        target,
        &credentials,
        settings,
        in_content_length,
    )
    .await?;
    let (input, in_read) = channel.input.split()?;
    let ping = setup
        .ping_schedule(Duration::ZERO, Duration::ZERO)
        .map_err(|error| custom_err!("create rpch ping schedule", error))?;
    let rpc_auth = RpcNtlmAuth::new(&credentials.username, &credentials.password)?;
    let mut session = RpchSession::<Out, In, Provider> {
        out: channel.out,
        input,
        in_monitor: InMonitor::spawn(in_read),
        in_successor: None,
        out_successor: None,
        in_content_length,
        out_auth_rounds,
        cookies,
        provider,
        setup,
        gateway_host: gateway_host.to_owned(),
        target: target.to_owned(),
        session_id,
        credentials,
        flow_control: channel.flow_control,
        ping,
        started: Instant::now(),
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
        message_responses: RpcAuthenticatedResponseReassembler::new(MAX_MESSAGE_STUB_SIZE),
        active_call_id: None,
        call_id: 0,
        receive_pipe_call_id: None,
        tunnel_context: None,
        channel_context: None,
        tunnel_target: None,
        capabilities: 0,
        messaging: false,
        message_call_id: None,
        pending_reauth: None,
        closing: false,
        data: VecDeque::new(),
        buffered_data: 0,
        failed: false,
    };
    session.bind().await?;
    Ok(session)
}

struct OpenedChannels<Out, In> {
    channel: RpchChannel<Out, In>,
    setup: RpchV2Setup,
    session_id: String,
    cookies: RpchCookieJar,
    /// `401` rounds the OUT channel needed; successor OUT channels repeat them with the same credentials.
    out_auth_rounds: usize,
}

async fn open_rpch_channels<Out, In>(
    out_stream: Out,
    in_stream: In,
    gateway_host: &str,
    target: &str,
    credentials: &RpchHttpCredentials,
    settings: RpchV2Settings,
    in_content_length: u32,
) -> Result<OpenedChannels<Out, In>, Error>
where
    Out: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    In: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let association_group_id = Uuid::new_v4();
    // The session UUID pragma carries the virtual connection cookie on both channels, as deployed proxies expect.
    let virtual_connection_id = Uuid::new_v4();
    let mut setup = RpchV2Setup::new(
        settings,
        RtsCookie::new(virtual_connection_id.to_bytes_le()),
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

    let session_id = virtual_connection_id.to_string();
    let mut cookies = RpchCookieJar::default();
    let mut input = open_in_channel(
        in_stream,
        gateway_host,
        target,
        &session_id,
        credentials,
        in_content_length,
        &mut cookies,
    )
    .await?;

    let (out_stream, out_response, out_auth_rounds) = open_out_channel(
        out_stream,
        gateway_host,
        target,
        &session_id,
        credentials,
        &out_body,
        &mut cookies,
    )
    .await?;
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
        log::debug!("RPCH setup PDU received; state={:?}", setup.state());
    }

    let flow_control = setup
        .flow_control()
        .map_err(|error| custom_err!("create rpch flow control", error))?;
    let timeout = setup
        .in_channel_ping_timeout()
        .ok_or_else(|| Error::new("missing rpch connection timeout", GwErrorKind::Connect))?;
    Ok(OpenedChannels {
        channel: RpchChannel {
            out,
            input,
            flow_control,
            connection_timeout: Duration::from_millis(u64::from(timeout)),
            _out: PhantomData,
        },
        setup,
        session_id,
        cookies,
        out_auth_rounds,
    })
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
        let opened = open_rpch_channels(
            out,
            input,
            host,
            target,
            &credentials,
            settings,
            CHANNEL_IN_CONTENT_LENGTH,
        )
        .await?;
        Ok(opened.channel)
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
    session_id: &str,
    credentials: &RpchHttpCredentials,
    content_length: u32,
    cookies: &mut RpchCookieJar,
) -> Result<RpchInRequest<S>, Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let spn = http_spn(gateway_host)?;
    let mut auth: Option<GatewayHttpAuth> = None;
    let mut use_basic = false;
    let mut input = RpchInRequest::open(
        stream,
        gateway_host,
        target,
        content_length,
        Some(session_id),
        cookies.header(),
        None,
        false,
    )
    .await?;

    for _ in 0..MAX_AUTH_ROUNDS {
        let response = input.receive_response().await?;
        cookies.update(&response.set_cookies)?;
        input.set_cookie(cookies.header());
        log::debug!(
            "RPCH IN response; status={} body_reserved={}",
            response.status,
            input.body_reserved()
        );
        match response.status {
            401 => {
                if use_basic {
                    return Err(Error::new("rpch IN basic authentication", GwErrorKind::HttpStatus(401)));
                }
                // A token answering a server challenge is the final client leg for NTLM, including when
                // SPNEGO still expects the server's mechListMIC.
                let answers_challenge = auth.is_some();
                let authorization =
                    step_http_auth(&mut auth, credentials, &spn, &response.www_authenticate, &mut use_basic).await?;
                let reserve_body =
                    use_basic || answers_challenge || auth.as_ref().is_some_and(GatewayHttpAuth::is_complete);
                input = input.retry(Some(&authorization), reserve_body).await?;
            }
            100 => {
                finish_http_auth(&mut auth, &response.www_authenticate).await?;
                return Ok(input);
            }
            // An empty authentication leg completed; reserve the body on the authenticated connection.
            200 if !input.body_reserved() => {
                finish_http_auth(&mut auth, &response.www_authenticate).await?;
                input = input.retry(None, true).await?;
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
    cookies: &mut RpchCookieJar,
) -> Result<(S, RpchResponseHead, usize), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let content_length =
        u32::try_from(body.len()).map_err(|_| Error::new("rpch CONN/A1 length", GwErrorKind::Encode))?;
    let spn = http_spn(gateway_host)?;
    let mut auth = None;
    let mut authorization = None;
    let mut use_basic = false;

    for round in 0..MAX_AUTH_ROUNDS {
        let cookie = cookies.header();
        write_rpch_request_head(
            &mut stream,
            RpchRequestHead {
                method: "RPC_OUT_DATA",
                host: gateway_host,
                target,
                content_length,
                authorization: authorization.as_deref(),
                cookie: cookie.as_deref(),
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
        log::debug!("RPCH OUT response; status={}", response.status);
        cookies.update(&response.set_cookies)?;
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
                return Ok((stream, response, round));
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
        let target = TunnelTarget {
            client_name: client_name.to_owned(),
            resource: resource.to_owned(),
            port,
        };
        let tunnel = self
            .create_tunnel(TsProxyCreateTunnelRequest::new(TSG_CLIENT_CAPABILITIES), &target)
            .await?;
        self.capabilities = tunnel.capabilities;
        log::debug!("RPCH tunnel created; capabilities=0x{:x}", self.capabilities);
        if let TsProxyTunnelMessage::Consent { text, .. } = &tunnel.message {
            // Consent is accepted without prompting, matching the HTTP transport's default consent handling.
            warn!("RD Gateway consent message accepted: {text}");
        }

        // A pending TsProxyMakeTunnelCall carries administrative and reauthentication messages
        // ([MS-TSGU] 3.2.6.1.3 and 3.6.5.1).
        self.messaging = self.capabilities & TSG_MESSAGING_CAP_SERVICE_MSG != 0;
        self.request_tunnel_message().await?;
        self.create_channel(&target, false).await?;
        self.tunnel_target = Some(target);

        let channel_context = self
            .channel_context
            .ok_or_else(|| Error::new("rpch channel missing", GwErrorKind::Connect))?;
        let request = TsProxySetupReceivePipeRequest::new(channel_context).encode();
        let call_id = self.next_call_id();
        self.send_call(call_id, TSPROXY_SETUP_RECEIVE_PIPE_OPNUM, &request)
            .await?;
        self.receive_pipe_call_id = Some(call_id);
        Ok(())
    }

    /// Answers a reauthentication message on this separate virtual connection ([MS-TSGU] 3.6.4).
    ///
    /// The new tunnel names the original one, then authorizes the user and resource without creating a data channel.
    async fn reauthenticate_tunnel(&mut self, tunnel_context: u64, target: &TunnelTarget) -> Result<(), Error> {
        self.create_tunnel(
            TsProxyCreateTunnelRequest::reauthenticate(TSG_CLIENT_CAPABILITIES, tunnel_context),
            target,
        )
        .await?;
        self.create_channel(target, true).await
    }

    async fn create_tunnel(
        &mut self,
        request: TsProxyCreateTunnelRequest,
        target: &TunnelTarget,
    ) -> Result<TsProxyCreateTunnelResponse, Error> {
        let request = request
            .encode()
            .map_err(|error| custom_err!("encode create tunnel", error))?;
        let response = self.call(TSPROXY_CREATE_TUNNEL_OPNUM, &request).await?;
        let tunnel = decode_tsgu_create_tunnel_response(&response)
            .map_err(|error| custom_err!("decode create tunnel", error))?;
        self.tunnel_context = Some(tunnel.tunnel_context);

        let request = TsProxyAuthorizeTunnelRequest::new(tunnel.tunnel_context, &target.client_name, &[])
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
        Ok(tunnel)
    }

    async fn create_channel(&mut self, target: &TunnelTarget, reauthentication: bool) -> Result<(), Error> {
        let tunnel_context = self
            .tunnel_context
            .ok_or_else(|| Error::new("rpch channel before tunnel", GwErrorKind::Connect))?;
        let request = TsProxyCreateChannelRequest::new(tunnel_context, &target.resource, target.port)
            .encode()
            .map_err(|error| custom_err!("encode create channel", error))?;
        let response = self.call(TSPROXY_CREATE_CHANNEL_OPNUM, &request).await?;
        if reauthentication {
            return crate::rpc::tsgu::decode_tsgu_reauthentication_channel_response(&response)
                .map_err(|error| custom_err!("decode reauthentication resource authorization", error));
        }
        let channel = decode_tsgu_create_channel_response(&response)
            .map_err(|error| custom_err!("decode create channel", error))?;
        self.channel_context = Some(channel.channel_context);
        Ok(())
    }

    /// Issues the next `TsProxyMakeTunnelCall` when messaging is negotiated and none is pending.
    async fn request_tunnel_message(&mut self) -> Result<(), Error> {
        if !self.messaging || self.closing || self.message_call_id.is_some() {
            return Ok(());
        }
        let tunnel_context = self
            .tunnel_context
            .ok_or_else(|| Error::new("rpch message call before tunnel", GwErrorKind::Connect))?;
        let request = TsProxyMakeTunnelCallRequest::new(tunnel_context).encode();
        let call_id = self.next_call_id();
        self.send_call(call_id, TSPROXY_MAKE_TUNNEL_CALL_OPNUM, &request)
            .await?;
        self.message_call_id = Some(call_id);
        Ok(())
    }

    /// Applies a completed `TsProxyMakeTunnelCall`; a failed call only stops messaging.
    fn handle_tunnel_message(&mut self, stub: &[u8]) {
        match decode_tsgu_make_tunnel_call_response(stub) {
            Ok(TsProxyTunnelMessage::None) => {}
            Ok(TsProxyTunnelMessage::Service { text, .. }) => warn!("RD Gateway service message: {text}"),
            Ok(TsProxyTunnelMessage::Consent { text, .. }) => {
                warn!("RD Gateway consent message accepted: {text}");
            }
            Ok(TsProxyTunnelMessage::Reauthenticate { tunnel_context }) => {
                if self.capabilities & TSG_MESSAGING_CAP_REAUTH == 0 {
                    warn!("RD Gateway requested reauthentication without negotiating it");
                } else {
                    self.pending_reauth = Some(tunnel_context);
                }
            }
            Err(error) => {
                if !self.closing {
                    warn!("RD Gateway message call failed; administrative messages are disabled: {error}");
                }
                self.messaging = false;
            }
        }
    }

    /// Spawns the reauthentication connection for `tunnel_context` ([MS-TSGU] 3.6.4).
    ///
    /// The gateway enforces reauthentication by closing the tunnel, so this session keeps running whatever
    /// the outcome.
    fn spawn_reauthentication(&mut self, tunnel_context: u64) -> Result<ReauthTask, Error> {
        let target = self
            .tunnel_target
            .clone()
            .ok_or_else(|| Error::new("rpch reauthentication before channel", GwErrorKind::Connect))?;
        let channels = self.provider.open_reauthentication();
        let gateway_host = self.gateway_host.clone();
        let backend = self.target.clone();
        let credentials = self.credentials.clone();
        let in_content_length = self.in_content_length;
        Ok(ReauthTask(tokio::spawn(async move {
            let (out, input) = channels.await?;
            let mut session = rpch_connect(
                out,
                input,
                NoSuccessorChannels,
                &gateway_host,
                &backend,
                credentials,
                RpchV2Settings::default(),
                in_content_length,
            )
            .await?;
            session.reauthenticate_tunnel(tunnel_context, &target).await?;
            session.close().await
        })))
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

    /// Closes the channel and tunnel in the order of [MS-TSGU] 3.6.5, cancelling a pending message call between them.
    async fn close(&mut self) -> Result<(), Error> {
        self.closing = true;
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
        if let (Some(_), Some(tunnel_context)) = (self.message_call_id, self.tunnel_context) {
            let response = self
                .call(
                    TSPROXY_MAKE_TUNNEL_CALL_OPNUM,
                    &TsProxyMakeTunnelCallRequest::cancel_pending(tunnel_context).encode(),
                )
                .await?;
            if let Err(error) = decode_tsgu_cancel_tunnel_call_response(&response) {
                warn!("RD Gateway rejected message call cancellation: {error}");
            }
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

    #[cfg(test)]
    async fn read_data(&mut self) -> Result<Bytes, Error> {
        self.ensure_active()?;
        let result = self.read_data_inner().await;
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    #[cfg(test)]
    async fn read_data_inner(&mut self) -> Result<Bytes, Error> {
        loop {
            if let Some(data) = self.data.pop_front() {
                self.buffered_data -= data.len();
                return Ok(data);
            }
            let pdu = self.read_out_pdu().await?;
            if self.handle_out_pdu(pdu).await?.is_some() {
                return Err(Error::new("unexpected tsproxy response", GwErrorKind::Decode));
            }
        }
    }

    /// Reads the next OUT channel PDU while watching the IN channels and a successor OUT response for failures.
    ///
    /// Cancel-safe: every awaited source is a channel receiver.
    async fn read_out_pdu(&mut self) -> Result<Vec<u8>, Error> {
        loop {
            let Self {
                out,
                in_monitor,
                in_successor,
                out_successor,
                ..
            } = self;
            let OutChannel { receiver, head, .. } = out;
            let ready = head.is_none();
            let successor_cookies = tokio::select! {
                cookies = out_response_head(head) => cookies?,
                pdu = receiver.recv(), if ready => return pdu.unwrap_or_else(|| Err(Error::new("rpch OUT reader stopped", GwErrorKind::Connect))),
                error = in_monitor.failed() => return Err(error),
                error = successor_in_failed(in_successor) => return Err(error),
                cookies = successor_out_head(out_successor) => cookies?,
            };
            self.cookies.update(&successor_cookies)?;
        }
    }

    async fn handle_out_pdu(&mut self, mut pdu: Vec<u8>) -> Result<Option<(u32, Vec<u8>)>, Error> {
        let pdu_len = pdu.len();
        // Only RPC PDUs count against the receive window ([MS-RPCH] 3.2.1.1.4).
        if pdu.get(2).copied() == Some(PTYPE_RTS) {
            self.handle_rts_pdu(&pdu).await?;
            return Ok(None);
        }
        self.flow_control
            .received_rpc_pdu(pdu_len)
            .map_err(|error| custom_err!("account rpch receive window", error))?;

        match pdu.get(2).copied() {
            // Servers report verification and dispatch failures with unauthenticated faults; surface the
            // status, then fail closed like any other fault.
            Some(PTYPE_FAULT) if pdu.get(10..12) == Some(&[0, 0]) => {
                let fault = decode_rpc_fault_for_context(&pdu, self.fragment_sizes.max_recv(), 0)
                    .map_err(|error| custom_err!("decode unauthenticated rpc fault", error))?;
                Err(Error::new(
                    "unauthenticated rpc fault",
                    GwErrorKind::GatewayCode(fault.status),
                ))
            }
            Some(PTYPE_FAULT | PTYPE_RESPONSE) => {
                let fragment = self.verify_rpc_fragment(&mut pdu)?;
                if fragment.header().ptype() == PTYPE_FAULT {
                    // A failed message call only ends messaging; any other fault fails the session.
                    if self.message_call_id != Some(fragment.header().call_id()) {
                        return Err(Error::new("rpc fault on receive pipe", GwErrorKind::Connect));
                    }
                    self.message_call_id = None;
                    self.message_responses = RpcAuthenticatedResponseReassembler::new(MAX_MESSAGE_STUB_SIZE);
                    self.messaging = false;
                    if !self.closing {
                        warn!("RD Gateway faulted the message call; administrative messages are disabled");
                    }
                    self.consume_out_pdu(pdu_len).await?;
                    return Ok(None);
                }
                let response = fragment
                    .decode_response_fragment_for_context(0)
                    .map_err(|error| custom_err!("decode rpch response", error))?;
                if self.receive_pipe_call_id == Some(response.call_id) {
                    self.queue_receive_pipe_response(pdu_len, response).await?;
                    return Ok(None);
                }
                let is_message = self.message_call_id == Some(response.call_id);
                if !is_message && self.active_call_id != Some(response.call_id) {
                    return Err(Error::new("unexpected tsproxy response call ID", GwErrorKind::Decode));
                }
                let responses = if is_message {
                    &mut self.message_responses
                } else {
                    &mut self.responses
                };
                let response = responses
                    .push_for_context(fragment, 0)
                    .map_err(|error| custom_err!("reassemble tsproxy response", error))?;
                self.consume_out_pdu(pdu_len).await?;
                match response {
                    Some(response) if self.message_call_id == Some(response.call_id) => {
                        self.message_call_id = None;
                        self.handle_tunnel_message(&response.stub);
                        Ok(None)
                    }
                    response => Ok(response.map(|response| (response.call_id, response.stub))),
                }
            }
            _ => Err(Error::new("unexpected rpch OUT pdu", GwErrorKind::Decode)),
        }
    }

    fn ping_due(&self, now: Duration) -> bool {
        self.ping.ping_due(now)
    }

    async fn send_ping(&mut self, now: Duration) -> Result<(), Error> {
        let pdu = encode_rts_ping().map_err(|error| custom_err!("encode rpch ping", error))?;
        self.write_rts(&pdu).await?;
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
        self.write_rpc(&bind).await?;
        let response = loop {
            let pdu = self.read_out_pdu().await?;
            if pdu.get(2).copied() != Some(PTYPE_RTS) {
                break pdu;
            }
            self.handle_rts_pdu(&pdu).await?;
        };
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
        self.write_rpc(&auth_3).await?;
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
        self.active_call_id = Some(call_id);
        // CreateTunnel may return both a 24,000-character certificate chain and a consent message.
        let limit = if opnum == TSPROXY_CREATE_TUNNEL_OPNUM {
            MAX_MESSAGE_STUB_SIZE + 2 * 24_000
        } else {
            MAX_RESPONSE_STUB_SIZE
        };
        self.responses = RpcAuthenticatedResponseReassembler::new(limit);
        self.send_call(call_id, opnum, stub).await?;
        loop {
            let pdu = self.read_out_pdu().await?;
            if let Some((response_call_id, response)) = self.handle_out_pdu(pdu).await? {
                if response_call_id != call_id {
                    return Err(Error::new("unexpected tsproxy response call ID", GwErrorKind::Decode));
                }
                self.active_call_id = None;
                return Ok(response);
            }
        }
    }

    async fn queue_receive_pipe_response(
        &mut self,
        pdu_len: usize,
        response: crate::rpc::RpcResponse<'_>,
    ) -> Result<(), Error> {
        if response.is_last_fragment() {
            let result = decode_tsgu_setup_receive_pipe_final_response(response.stub)
                .map_err(|error| custom_err!("decode receive-pipe terminal result", error))?;
            if self.closing {
                // Closing the channel ends the receive pipe ([MS-TSGU] 3.2.6.3.1).
                self.receive_pipe_call_id = None;
                return self.consume_out_pdu(pdu_len).await;
            }
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
        let buffered = self
            .buffered_data
            .checked_add(response.stub.len())
            .filter(|length| *length <= MAX_BUFFERED_DATA)
            .ok_or_else(|| Error::new("rpch receive buffer exceeded", GwErrorKind::Connect))?;
        self.data.push_back(Bytes::copy_from_slice(response.stub));
        self.buffered_data = buffered;
        self.consume_out_pdu(pdu_len).await
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
            self.write_rpc(&fragment).await?;
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

    /// Applies one RTS PDU from the OUT channel.
    ///
    /// Recycle PDUs are recognised by the state that expects them, so an RTS PDU no state expects fails the
    /// virtual connection ([MS-RPCH] 3.2.2.5.5 through 3.2.2.5.11).
    async fn handle_rts_pdu(&mut self, pdu: &[u8]) -> Result<(), Error> {
        if is_rts_ping(pdu) {
            return Ok(());
        }
        if let Ok(ack) = decode_rts_flow_control_ack(pdu) {
            self.flow_control
                .receive_flow_control_ack(ack)
                .map_err(|error| custom_err!("apply rpch flow-control ack", error))?;
            return Ok(());
        }

        let (in_state, out_state) = {
            let recycle = self.recycling()?;
            (recycle.in_state(), recycle.out_state())
        };
        if in_state == RpchInChannelRecycleState::AwaitingA4
            && (decode_rts_in_recycle_a4(pdu).is_ok() || crate::rpc::decode_rts_in_recycle_r2_a4(pdu).is_ok())
        {
            return self.finish_in_recycling(pdu).await;
        }
        match out_state {
            RpchOutChannelRecycleState::Open if decode_rts_out_recycle_a2(pdu).is_ok() => {
                self.start_out_recycling(pdu).await
            }
            RpchOutChannelRecycleState::AwaitingA6 if decode_rts_out_recycle_a6(pdu).is_ok() => {
                let a7 = self
                    .recycling()?
                    .receive_out_recycle_a6(pdu)
                    .map_err(|error| custom_err!("accept rpch OUT_R1/A6", error))?;
                self.write_rts(&a7).await
            }
            RpchOutChannelRecycleState::AwaitingA6 if decode_rts_out_recycle_r2_a6(pdu).is_ok() => {
                let (a7, c1) = self
                    .recycling()?
                    .receive_out_recycle_r2_a6(pdu)
                    .map_err(|error| custom_err!("accept rpch OUT_R2/A6", error))?;
                self.write_rts(&a7).await?;
                let successor = self.out_successor_mut()?;
                write_out_successor(&mut successor.writer, &c1).await
            }
            RpchOutChannelRecycleState::AwaitingA10 if decode_rts_out_recycle_a10(pdu).is_ok() => {
                let a11 = self
                    .recycling()?
                    .receive_out_recycle_a10(pdu)
                    .map_err(|error| custom_err!("accept rpch OUT_R1/A10", error))?;
                let mut successor = self.take_out_successor()?;
                write_out_successor(&mut successor.writer, &a11).await?;
                self.switch_out_channel(successor);
                let Self {
                    setup, flow_control, ..
                } = self;
                setup
                    .channel_recycling()
                    .map_err(|error| custom_err!("finish rpch OUT recycling", error))?
                    .finish_out_recycling(flow_control)
                    .map_err(|error| custom_err!("finish rpch OUT recycling", error))
            }
            RpchOutChannelRecycleState::AwaitingB3 if decode_rts_out_recycle_r2_b3(pdu).is_ok() => {
                let Self {
                    setup, flow_control, ..
                } = self;
                setup
                    .channel_recycling()
                    .map_err(|error| custom_err!("accept rpch OUT_R2/B3", error))?
                    .receive_out_recycle_b3(pdu, flow_control)
                    .map_err(|error| custom_err!("accept rpch OUT_R2/B3", error))?;
                let successor = self.take_out_successor()?;
                self.switch_out_channel(successor);
                Ok(())
            }
            _ => Err(Error::new("unexpected rpch RTS pdu", GwErrorKind::Decode)),
        }
    }

    fn recycling(&mut self) -> Result<crate::rpc::RpchV2ChannelRecycle<'_>, Error> {
        self.setup
            .channel_recycling()
            .map_err(|error| custom_err!("rpch channel recycling", error))
    }

    fn out_successor_mut(&mut self) -> Result<&mut OutSuccessor<Out>, Error> {
        self.out_successor
            .as_mut()
            .ok_or_else(|| Error::new("missing rpch successor OUT channel", GwErrorKind::Connect))
    }

    fn take_out_successor(&mut self) -> Result<OutSuccessor<Out>, Error> {
        self.out_successor
            .take()
            .ok_or_else(|| Error::new("missing rpch successor OUT channel", GwErrorKind::Connect))
    }

    /// Makes the successor the default OUT channel; dropping the predecessor closes it and ignores its later bytes.
    fn switch_out_channel(&mut self, successor: OutSuccessor<Out>) {
        // A successor whose response head is still unread reports a rejected head through its PDU reader.
        self.out = successor.channel;
        log::debug!("RPCH OUT channel recycled");
    }

    async fn consume_out_pdu(&mut self, pdu_len: usize) -> Result<(), Error> {
        if let Some(ack) = self
            .flow_control
            .consumed_rpc_pdu(pdu_len)
            .map_err(|error| custom_err!("release rpch receive window", error))?
        {
            self.write_rts(
                &encode_rts_flow_control_ack_with_destination(ack, RtsDestination::OutProxy)
                    .map_err(|error| custom_err!("encode rpch flow-control ack", error))?,
            )
            .await?;
        }
        Ok(())
    }

    /// Writes an RTS PDU on the default IN channel; RTS PDUs do not count against the send window.
    async fn write_rts(&mut self, pdu: &[u8]) -> Result<(), Error> {
        let length = u32::try_from(pdu.len()).map_err(|_| Error::new("rpch IN pdu length", GwErrorKind::Encode))?;
        if length > self.input.remaining() {
            return Err(Error::new("rpch IN body exhausted", GwErrorKind::Connect));
        }
        let result = write_in_pdu(&mut self.input, pdu).await;
        if result.is_ok() {
            self.ping.record_send(self.started.elapsed());
        }
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    /// Writes an RPC PDU on the default IN channel once the send window and IN body allow it.
    ///
    /// While waiting, OUT channel PDUs are processed so flow-control acknowledgements and recycle PDUs arrive.
    /// IN recycling starts when a quarter of the IN body remains or the PDU would not fit
    /// ([MS-RPCH] 3.2.2.4.2 and 3.2.2.5.12).
    async fn write_rpc(&mut self, pdu: &[u8]) -> Result<(), Error> {
        let length = u32::try_from(pdu.len()).map_err(|_| Error::new("rpch IN pdu length", GwErrorKind::Encode))?;
        let required = length
            .checked_add(IN_RECYCLE_RESERVE)
            .ok_or_else(|| Error::new("rpch IN pdu length", GwErrorKind::Encode))?;
        if required > self.in_content_length {
            return Err(Error::new("rpch IN pdu exceeds channel lifetime", GwErrorKind::Encode));
        }
        loop {
            let remaining = self.input.remaining();
            if self.in_successor.is_none()
                && self.recycling()?.in_state() == RpchInChannelRecycleState::Open
                && (remaining < self.in_content_length / 4 || required > remaining)
            {
                self.start_in_recycling().await?;
            }
            if required <= self.input.remaining() && self.flow_control.send_available_window() > length {
                break;
            }
            let out_pdu = self.read_out_pdu().await?;
            if self.handle_out_pdu(out_pdu).await?.is_some() {
                return Err(Error::new("unexpected tsproxy response", GwErrorKind::Decode));
            }
        }
        self.flow_control
            .sent_rpc_pdu(pdu.len())
            .map_err(|error| custom_err!("account rpch send window", error))?;
        let result = write_in_pdu(&mut self.input, pdu).await;
        if result.is_ok() {
            self.ping.record_send(self.started.elapsed());
        }
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    /// Opens a successor IN channel and sends IN_R1/A1 on it ([MS-RPCH] 3.2.2.5.12).
    async fn start_in_recycling(&mut self) -> Result<(), Error> {
        let stream = self.provider.open_in().await?;
        let successor = open_in_channel(
            stream,
            &self.gateway_host,
            &self.target,
            &self.session_id,
            &self.credentials,
            self.in_content_length,
            &mut self.cookies,
        )
        .await?;
        let (mut successor, read) = successor.split()?;
        let monitor = InMonitor::spawn(read);
        let a1 = self
            .recycling()?
            .start_in_recycling(RtsCookie::new(*Uuid::new_v4().as_bytes()))
            .map_err(|error| custom_err!("encode rpch IN_R1/A1", error))?;
        write_in_pdu(&mut successor, &a1).await?;
        self.in_successor = Some((successor, monitor));
        log::debug!("RPCH IN channel recycling started");
        Ok(())
    }

    /// Accepts IN_R1/A4 or IN_R2/A4, sends IN_R1/A5 on the predecessor, and makes the successor the default IN channel.
    ///
    /// [MS-RPCH] 3.2.2.5.5.
    async fn finish_in_recycling(&mut self, a4: &[u8]) -> Result<(), Error> {
        let a5 = self
            .recycling()?
            .receive_in_recycle_a4(a4)
            .map_err(|error| custom_err!("accept rpch IN_R1/A4", error))?;
        self.write_rts(&a5).await?;
        let (input, monitor) = self
            .in_successor
            .take()
            .ok_or_else(|| Error::new("missing rpch successor IN channel", GwErrorKind::Connect))?;
        // Dropping the predecessor stops its monitor; a close from the retired proxy is not a failure.
        self.input = input;
        self.in_monitor = monitor;
        let Self {
            setup,
            flow_control,
            ping,
            ..
        } = self;
        setup
            .channel_recycling()
            .map_err(|error| custom_err!("finish rpch IN recycling", error))?
            .finish_in_recycling(flow_control, ping)
            .map_err(|error| custom_err!("finish rpch IN recycling", error))?;
        log::debug!("RPCH IN channel recycled");
        Ok(())
    }

    /// Accepts OUT_R1/A2 or OUT_R2/A2 and opens the successor OUT channel with OUT_R1/A3 ([MS-RPCH] 3.2.2.5.6).
    async fn start_out_recycling(&mut self, a2: &[u8]) -> Result<(), Error> {
        let stream = self.provider.open_out().await?;
        let a3 = self
            .recycling()?
            .receive_out_recycle_a2(a2, RtsCookie::new(*Uuid::new_v4().as_bytes()))
            .map_err(|error| custom_err!("accept rpch OUT_R1/A2", error))?;
        let successor = self.open_successor_out(stream, &a3).await?;
        self.out_successor = Some(successor);
        log::debug!("RPCH OUT channel recycling started");
        Ok(())
    }

    /// Sends the successor OUT channel request, repeating the authentication legs the first OUT channel needed.
    ///
    /// The request body is OUT_R1/A3 followed by the 24-byte OUT_R1/A11 or OUT_R2/C1 sent later.
    /// Rejected legs carry a complete body so the connection can be reused.
    /// The final response is read asynchronously, because the proxy may send it only after OUT_R1/A11 or OUT_R2/C1.
    async fn open_successor_out(&mut self, mut stream: Out, a3: &[u8]) -> Result<OutSuccessor<Out>, Error> {
        let trailer = encode_rts_out_recycle_a11().map_err(|error| custom_err!("encode rpch OUT_R1/A11", error))?;
        let content_length = u32::try_from(a3.len() + trailer.len())
            .map_err(|_| Error::new("rpch OUT_R1/A3 length", GwErrorKind::Encode))?;
        let spn = http_spn(&self.gateway_host)?;
        let mut auth = None;
        let mut authorization: Option<String> = None;
        let mut use_basic = false;

        for round in 0..=self.out_auth_rounds {
            let cookie = self.cookies.header();
            write_rpch_request_head(
                &mut stream,
                RpchRequestHead {
                    method: "RPC_OUT_DATA",
                    host: &self.gateway_host,
                    target: &self.target,
                    content_length,
                    authorization: authorization.as_deref(),
                    cookie: cookie.as_deref(),
                    session_id: Some(&self.session_id),
                    expect_continue: false,
                },
            )
            .await?;
            let last = round == self.out_auth_rounds;
            let body = if last {
                a3.to_vec()
            } else {
                [a3, trailer.as_slice()].concat()
            };
            stream
                .write_all(&body)
                .await
                .map_err(|error| custom_err!("write rpch OUT_R1/A3", error))?;
            stream
                .flush()
                .await
                .map_err(|error| custom_err!("flush rpch OUT_R1/A3", error))?;
            if last {
                break;
            }

            let response = read_rpch_response_head(&mut stream).await?;
            if response.status != 401 {
                return Err(Error::new(
                    "rpch successor OUT authentication diverged",
                    GwErrorKind::HttpStatus(response.status),
                ));
            }
            let length = response.content_length.ok_or_else(|| {
                Error::new(
                    "rpch authentication response missing content length",
                    GwErrorKind::Decode,
                )
            })?;
            drain_body(&mut stream, length).await?;
            self.cookies.update(&response.set_cookies)?;
            if use_basic {
                return Err(Error::new(
                    "rpch successor OUT basic authentication",
                    GwErrorKind::HttpStatus(401),
                ));
            }
            authorization = Some(
                step_http_auth(
                    &mut auth,
                    &self.credentials,
                    &spn,
                    &response.www_authenticate,
                    &mut use_basic,
                )
                .await?,
            );
        }

        let (read, writer) = tokio::io::split(stream);
        let channel = OutChannel::successor(read, auth);
        Ok(OutSuccessor { channel, writer })
    }

    fn next_call_id(&mut self) -> u32 {
        self.call_id = self.call_id.wrapping_add(1);
        self.call_id
    }
}

async fn successor_in_failed<In>(successor: &mut Option<(RpchInRequest<WriteHalf<In>>, InMonitor)>) -> Error {
    match successor {
        Some((_, monitor)) => monitor.failed().await,
        None => core::future::pending().await,
    }
}

/// Resolves with the cookies of an accepted successor OUT response, or its failure.
async fn successor_out_head<Out>(successor: &mut Option<OutSuccessor<Out>>) -> Result<Vec<String>, Error> {
    match successor {
        Some(successor) => out_response_head(&mut successor.channel.head).await,
        None => core::future::pending().await,
    }
}

async fn out_response_head(
    head: &mut Option<oneshot::Receiver<Result<Vec<String>, Error>>>,
) -> Result<Vec<String>, Error> {
    let Some(receiver) = head.as_mut() else {
        return core::future::pending().await;
    };
    let result = receiver.await;
    *head = None;
    result.unwrap_or_else(|_| Err(Error::new("rpch successor OUT reader stopped", GwErrorKind::Connect)))
}

async fn write_out_successor<Out>(writer: &mut WriteHalf<Out>, pdu: &[u8]) -> Result<(), Error>
where
    Out: AsyncWrite,
{
    writer
        .write_all(pdu)
        .await
        .map_err(|error| custom_err!("write rpch successor OUT channel", error))?;
    writer
        .flush()
        .await
        .map_err(|error| custom_err!("flush rpch successor OUT channel", error))
}

/// Opens TLS connections to the gateway for successor IN and OUT channels.
struct GatewayChannelProvider {
    endpoint: String,
    certificate_validation: ironrdp_tls::CertificateValidation,
    certificate_validation_callback: Option<ironrdp_tls::CertificateValidationCallback>,
}

impl GatewayChannelProvider {
    async fn open(&self) -> Result<crate::packet_io::GatewayStream, Error> {
        let (stream, _) = crate::packet_io::open_rpch_gateway_stream(
            &self.endpoint,
            self.certificate_validation,
            self.certificate_validation_callback.clone(),
        )
        .await?;
        Ok(stream)
    }
}

impl RpchStreamProvider<crate::packet_io::GatewayStream, crate::packet_io::GatewayStream> for GatewayChannelProvider {
    async fn open_in(&mut self) -> Result<crate::packet_io::GatewayStream, Error> {
        self.open().await
    }

    async fn open_out(&mut self) -> Result<crate::packet_io::GatewayStream, Error> {
        self.open().await
    }

    fn open_reauthentication(
        &mut self,
    ) -> BoxFuture<'static, Result<(crate::packet_io::GatewayStream, crate::packet_io::GatewayStream), Error>> {
        let endpoint = self.endpoint.clone();
        let certificate_validation = self.certificate_validation;
        let certificate_validation_callback = self.certificate_validation_callback.clone();
        Box::pin(async move {
            let (out, input, _) = crate::packet_io::open_rpch_gateway_channels(
                &endpoint,
                certificate_validation,
                certificate_validation_callback,
            )
            .await?;
            Ok((out, input))
        })
    }
}

/// Reauthentication connection running beside the session; aborted with it.
struct ReauthTask(JoinHandle<Result<(), Error>>);

impl Drop for ReauthTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Resolves when the running reauthentication connection ends; pending forever when none runs.
///
/// Cancel-safe: it only polls the task handle.
async fn reauthentication_finished(task: &mut Option<ReauthTask>) -> Result<(), Error> {
    let Some(ReauthTask(handle)) = task.as_mut() else {
        return core::future::pending().await;
    };
    let result = handle.await;
    *task = None;
    result.unwrap_or_else(|error| Err(custom_err!("rpch reauthentication task", error)))
}

enum StreamCommand {
    Write(Bytes, oneshot::Sender<Result<(), String>>),
    Shutdown(oneshot::Sender<Result<(), String>>),
}

/// Authenticated TS Proxy channel exposed as an asynchronous RDP byte stream.
///
/// RPC packet integrity is established internally with NTLM. TLS/HTTP authentication remains a
/// separate gateway security layer. IN channels are recycled before their lifetime is exhausted and
/// OUT channels are recycled when the proxy requests it ([MS-RPCH] 3.2.2.5.5 through 3.2.2.5.12).
/// An RTS PDU that no recycle state expects, a rejected replacement channel, or an IN channel
/// error response terminates the stream.
/// Writes acknowledge acceptance into a bounded local queue.
/// Flush waits for the corresponding signed RPC send operation; flush, later writes and shutdown report its errors.
pub struct RpchStream {
    commands: PollSender<StreamCommand>,
    received: mpsc::Receiver<Result<Bytes, String>>,
    read_buffer: Option<Bytes>,
    pending_write: Option<oneshot::Receiver<Result<(), String>>>,
    failure: Option<String>,
    shutdown: Option<oneshot::Receiver<Result<(), String>>>,
    shutdown_complete: bool,
    actor: JoinHandle<()>,
    redirection_flags: GwRpchRedirectionFlags,
}

impl RpchStream {
    fn fail(&mut self, message: String) -> io::Error {
        self.failure = Some(message.clone());
        io::Error::other(message)
    }

    fn poll_write_completion(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Some(error) = &self.failure {
            return Poll::Ready(Err(io::Error::other(error.clone())));
        }
        let Some(response) = self.pending_write.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        let result = match Pin::new(response).poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Ok(result)) => result,
            Poll::Ready(Err(error)) => Err(error.to_string()),
        };
        self.pending_write = None;
        Poll::Ready(result.map_err(|error| self.fail(error)))
    }

    /// Opens an authenticated MS-RPCH tunnel and returns its local TCP address.
    ///
    /// RPC packet integrity is always negotiated and verified independently of TLS and HTTP
    /// authentication. Smart-card RPC authentication is not supported.
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
            certificate_validation_callback.clone(),
        )
        .await?;
        let provider = GatewayChannelProvider {
            endpoint: target.gw_endpoint.clone(),
            certificate_validation,
            certificate_validation_callback,
        };
        let stream = Self::connect(
            out,
            input,
            provider,
            target,
            client_name,
            server_port,
            SESSION_IN_CONTENT_LENGTH,
        )
        .await?;
        Ok((stream, client_addr))
    }

    /// Returns device-redirection restrictions from `TsProxyAuthorizeTunnel`.
    pub const fn redirection_flags(&self) -> GwRpchRedirectionFlags {
        self.redirection_flags
    }

    pub(crate) async fn connect<Out, In, Provider>(
        out: Out,
        input: In,
        provider: Provider,
        target: &GwConnectTarget,
        client_name: &str,
        port: u16,
        in_content_length: u32,
    ) -> Result<Self, Error>
    where
        Out: AsyncRead + AsyncWrite + Send + Unpin + 'static,
        In: AsyncRead + AsyncWrite + Send + Unpin + 'static,
        Provider: RpchStreamProvider<Out, In> + Send + 'static,
    {
        let settings = RpchV2Settings::default();
        let mut session = rpch_connect(
            out,
            input,
            provider,
            &target.gw_endpoint,
            TSPROXY_RPC_BACKEND,
            RpchHttpCredentials {
                username: target.gw_user.clone(),
                password: target.gw_pass.clone(),
                smart_card: None,
            },
            settings,
            in_content_length,
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
            failure: None,
            shutdown: None,
            shutdown_complete: false,
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
        if let Some(error) = &self.failure {
            return Poll::Ready(Err(io::Error::other(error.clone())));
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
                    return Poll::Ready(Err(self.fail(error)));
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
        core::task::ready!(self.poll_write_completion(cx))?;
        if self.shutdown.is_some() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "rpch stream is shut down",
            )));
        }

        match self.commands.poll_reserve(cx) {
            Poll::Ready(Ok(())) => {
                let count = buf.len().min(16 * 1024);
                let (response_tx, response_rx) = oneshot::channel();
                if self
                    .commands
                    .send_item(StreamCommand::Write(Bytes::copy_from_slice(&buf[..count]), response_tx))
                    .is_err()
                {
                    return Poll::Ready(Err(self.fail("rpch stream actor stopped".to_owned())));
                }
                self.pending_write = Some(response_rx);
                Poll::Ready(Ok(count))
            }
            Poll::Ready(Err(_)) => Poll::Ready(Err(self.fail("rpch stream actor stopped".to_owned()))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_write_completion(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        core::task::ready!(self.poll_write_completion(cx))?;
        if self.shutdown_complete {
            return Poll::Ready(Ok(()));
        }
        if self.shutdown.is_none() {
            match self.commands.poll_reserve(cx) {
                Poll::Ready(Ok(())) => {
                    let (response_tx, response_rx) = oneshot::channel();
                    if self.commands.send_item(StreamCommand::Shutdown(response_tx)).is_err() {
                        return Poll::Ready(Err(self.fail("rpch stream actor stopped".to_owned())));
                    }
                    self.shutdown = Some(response_rx);
                }
                Poll::Ready(Err(_)) => {
                    return Poll::Ready(Err(self.fail("rpch stream actor stopped".to_owned())));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        let result =
            core::task::ready!(Pin::new(self.shutdown.as_mut().expect("shutdown receiver initialized")).poll(cx));
        let result = result.unwrap_or_else(|error| Err(error.to_string()));
        if let Err(error) = result {
            return Poll::Ready(Err(self.fail(error)));
        }
        self.shutdown_complete = true;
        Poll::Ready(Ok(()))
    }
}

async fn run_rpch_stream<Out, In, Provider>(
    mut session: RpchSession<Out, In, Provider>,
    mut commands: mpsc::Receiver<StreamCommand>,
    received: mpsc::Sender<Result<Bytes, String>>,
) -> Result<(), Error>
where
    Out: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    In: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    Provider: RpchStreamProvider<Out, In>,
{
    let mut reauthentication = None;

    loop {
        if let Some(tunnel_context) = session.pending_reauth.take() {
            if reauthentication.is_some() {
                warn!("RD Gateway reauthentication already in progress; ignoring repeated request");
            } else {
                reauthentication = Some(session.spawn_reauthentication(tunnel_context)?);
            }
        }
        session.request_tunnel_message().await?;

        // Inbound data is delivered from its own branch so a reader that stops reading never stalls writes or
        // shutdown; the OUT channel is not read while data waits, which bounds the buffered data.
        let delivering = !session.data.is_empty();
        let deadline = tokio::time::Instant::from_std(session.started + session.ping.next_ping_time());
        tokio::select! {
            result = reauthentication_finished(&mut reauthentication) => {
                match result {
                    Ok(()) => warn!("RD Gateway reauthentication completed"),
                    Err(error) => warn!("RD Gateway reauthentication failed: {error}"),
                }
            }
            permit = received.reserve(), if delivering => {
                let permit = permit.map_err(|_| Error::new("rpch stream reader closed", GwErrorKind::Connect))?;
                if let Some(data) = session.data.pop_front() {
                    session.buffered_data -= data.len();
                    permit.send(Ok(data));
                }
            }
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
            pdu = session.read_out_pdu(), if !delivering => {
                if session.handle_out_pdu(pdu?).await?.is_some() {
                    return Err(Error::new("unexpected tsproxy response", GwErrorKind::Decode));
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                let now = session.started.elapsed();
                if session.ping_due(now) {
                    session.send_ping(now).await?;
                }
            }
        }
    }
}

async fn write_in_pdu<S>(input: &mut RpchInRequest<S>, pdu: &[u8]) -> Result<(), Error>
where
    S: AsyncWrite + Unpin,
{
    input.write_body(pdu).await?;
    input.flush().await
}

fn is_rts_ping(pdu: &[u8]) -> bool {
    encode_rts_ping().is_ok_and(|ping| pdu == ping)
}

struct OutChannel {
    receiver: mpsc::Receiver<Result<Vec<u8>, Error>>,
    reader: JoinHandle<()>,
    head: Option<oneshot::Receiver<Result<Vec<String>, Error>>>,
}

/// Accepts an OUT channel response body length in the range [MS-RPCH] 2.1.2.1.4 allows.
fn validate_out_content_length(content_length: Option<u32>) -> Result<u32, Error> {
    content_length
        .filter(|length| (128 * 1024..=2 * 1024 * 1024 * 1024).contains(length))
        .ok_or_else(|| Error::new("invalid rpch OUT content length", GwErrorKind::Decode))
}

async fn read_out_pdus<S>(stream: &mut S, mut remaining: u32, sender: &mpsc::Sender<Result<Vec<u8>, Error>>)
where
    S: AsyncRead + Unpin,
{
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
        .send(Err(Error::new("rpch OUT body exhausted", GwErrorKind::Connect)))
        .await;
}

impl OutChannel {
    fn new<S>(mut stream: S, content_length: Option<u32>) -> Result<Self, Error>
    where
        S: AsyncRead + Send + Unpin + 'static,
    {
        let remaining = validate_out_content_length(content_length)?;
        let (sender, receiver) = mpsc::channel(2);
        let reader = tokio::spawn(async move { read_out_pdus(&mut stream, remaining, &sender).await });
        Ok(Self {
            receiver,
            reader,
            head: None,
        })
    }

    /// Reads a successor OUT channel whose response head arrives after the request body completes.
    ///
    /// The head result, carrying the response cookies, is reported separately so the recycle can fail early.
    fn successor<S>(mut stream: S, mut auth: Option<GatewayHttpAuth>) -> Self
    where
        S: AsyncRead + Send + Unpin + 'static,
    {
        let (sender, receiver) = mpsc::channel(2);
        let (head_sender, head) = oneshot::channel();
        let reader = tokio::spawn(async move {
            let accepted = async {
                let response = read_rpch_response_head(&mut stream).await?;
                if response.status != 200 {
                    return Err(Error::new(
                        "rpch successor OUT channel rejected",
                        GwErrorKind::HttpStatus(response.status),
                    ));
                }
                finish_http_auth(&mut auth, &response.www_authenticate).await?;
                Ok((
                    validate_out_content_length(response.content_length)?,
                    response.set_cookies,
                ))
            }
            .await;
            match accepted {
                Ok((remaining, cookies)) => {
                    let _ = head_sender.send(Ok(cookies));
                    read_out_pdus(&mut stream, remaining, &sender).await;
                }
                Err(error) => {
                    let _ = sender
                        .send(Err(Error::new(
                            "rpch successor OUT channel failed",
                            GwErrorKind::Connect,
                        )))
                        .await;
                    let _ = head_sender.send(Err(error));
                }
            }
        });
        Self {
            receiver,
            reader,
            head: Some(head),
        }
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
    use crate::mock_rpch::{
        MockRpchConnector, MockRpchScenario, REAUTH_TUNNEL_CONTEXT, mock_rpch_proxy, mock_rpch_reauth_proxy,
    };

    #[tokio::test]
    async fn rpch_out_requires_bounded_http_content_length() {
        // oracle: spec MS-RPCH §2.1.2.1.4: OUT content length is between 128 KiB and 2 GiB.
        for length in [None, Some(0), Some(131071), Some(2147483649)] {
            assert!(validate_out_content_length(length).is_err());
        }
        for length in [131072, 2147483648] {
            assert_eq!(
                validate_out_content_length(Some(length)).expect("valid OUT body length"),
                length
            );
        }
    }

    /// Smallest IN body the mock accepts, so a few kilobytes of traffic trigger IN channel recycling.
    const TEST_IN_CONTENT_LENGTH: u32 = 128 * 1024;

    /// Mock proxy connector plus, for reauthentication tests, where reauthentication proxies report.
    struct MockProvider(MockRpchConnector, Option<mpsc::UnboundedSender<u64>>);

    impl MockProvider {
        fn open_reauthentication_proxy(&self) -> BoxFuture<'static, Result<(DuplexStream, DuplexStream), Error>> {
            let report = self.1.clone();
            Box::pin(async move {
                let report = report.ok_or_else(|| Error::new("unexpected reauthentication", GwErrorKind::Connect))?;
                let (out, input, _, _) = mock_rpch_reauth_proxy(report);
                Ok((out, input))
            })
        }
    }

    impl RpchStreamProvider<DuplexStream, DuplexStream> for MockProvider {
        async fn open_in(&mut self) -> Result<DuplexStream, Error> {
            self.0
                .connect_in()
                .ok_or_else(|| Error::new("mock proxy stopped", GwErrorKind::Connect))
        }

        async fn open_out(&mut self) -> Result<DuplexStream, Error> {
            self.0
                .connect_out()
                .ok_or_else(|| Error::new("mock proxy stopped", GwErrorKind::Connect))
        }

        fn open_reauthentication(&mut self) -> BoxFuture<'static, Result<(DuplexStream, DuplexStream), Error>> {
            self.open_reauthentication_proxy()
        }
    }

    impl RpchStreamProvider<Box<DuplexStream>, Box<DuplexStream>> for MockProvider {
        async fn open_in(&mut self) -> Result<Box<DuplexStream>, Error> {
            RpchStreamProvider::<DuplexStream, DuplexStream>::open_in(self)
                .await
                .map(Box::new)
        }

        async fn open_out(&mut self) -> Result<Box<DuplexStream>, Error> {
            RpchStreamProvider::<DuplexStream, DuplexStream>::open_out(self)
                .await
                .map(Box::new)
        }

        fn open_reauthentication(
            &mut self,
        ) -> BoxFuture<'static, Result<(Box<DuplexStream>, Box<DuplexStream>), Error>> {
            let channels = self.open_reauthentication_proxy();
            Box::pin(async move {
                let (out, input) = channels.await?;
                Ok((Box::new(out), Box::new(input)))
            })
        }
    }

    fn test_credentials() -> RpchHttpCredentials {
        RpchHttpCredentials {
            username: "CONTOSO\\alice".to_owned(),
            password: "secret".to_owned(),
            smart_card: None,
        }
    }

    async fn connect_session(
        scenario: MockRpchScenario,
    ) -> (
        Result<RpchSession<DuplexStream, DuplexStream, MockProvider>, Error>,
        JoinHandle<()>,
    ) {
        let (out, input, connector, proxy) = mock_rpch_proxy(scenario);
        let session = rpch_connect(
            out,
            input,
            MockProvider(connector, None),
            "rdg.contoso.com",
            TSPROXY_RPC_BACKEND,
            test_credentials(),
            RpchV2Settings::new(128 * 1024, 256 * 1024, 0).expect("valid test settings"),
            TEST_IN_CONTENT_LENGTH,
        )
        .await;
        (session, proxy)
    }

    async fn open_session(
        scenario: MockRpchScenario,
    ) -> (RpchSession<DuplexStream, DuplexStream, MockProvider>, JoinHandle<()>) {
        let (session, proxy) = connect_session(scenario).await;
        let mut session = session.expect("rpch connect");
        session
            .open_tunnel("test-client", "rdp.contoso.com", 3389)
            .await
            .expect("open tunnel");
        (session, proxy)
    }

    async fn open_stream(scenario: MockRpchScenario) -> (RpchStream, JoinHandle<()>) {
        open_stream_reporting_reauth(scenario, None).await
    }

    fn controlled_stream() -> (RpchStream, mpsc::Receiver<StreamCommand>) {
        let (commands, command_rx) = mpsc::channel(8);
        let (received, input) = mpsc::channel(8);
        let actor = tokio::spawn(async move {
            let _received = received;
            core::future::pending::<()>().await;
        });
        (
            RpchStream {
                commands: PollSender::new(commands),
                received: input,
                read_buffer: None,
                pending_write: None,
                failure: None,
                shutdown: None,
                shutdown_complete: false,
                actor,
                redirection_flags: GwRpchRedirectionFlags::default(),
            },
            command_rx,
        )
    }

    async fn open_stream_reporting_reauth(
        scenario: MockRpchScenario,
        reauth_report: Option<mpsc::UnboundedSender<u64>>,
    ) -> (RpchStream, JoinHandle<()>) {
        let (out, input, connector, proxy) = mock_rpch_proxy(scenario);
        let target = GwConnectTarget {
            gw_endpoint: "rdg.contoso.com".to_owned(),
            gw_user: "CONTOSO\\alice".to_owned(),
            gw_pass: "secret".to_owned(),
            smart_card: None,
            server: "rdp.contoso.com".to_owned(),
        };
        let stream = RpchStream::connect(
            Box::new(out),
            Box::new(input),
            MockProvider(connector, reauth_report),
            &target,
            "test-client",
            3389,
            TEST_IN_CONTENT_LENGTH,
        )
        .await
        .expect("authenticated RPCH stream");
        (stream, proxy)
    }

    #[tokio::test]
    async fn rpch_session_runs_ntlm_authenticated_tsproxy_echo() {
        // oracle: spec MS-TSGU §3.6.5.1 and §3.6.5.4: TS Proxy carries the target bytes unchanged.
        let (mut session, proxy) = open_session(MockRpchScenario::Echo).await;
        session
            .send_to_server(Bytes::from_static(b"hello-rdp"))
            .await
            .expect("send target data");
        let data = session.read_data().await.expect("read target data");
        assert_eq!(&*data, b"hello-rdp");
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_poisoned_auth_context_rejects_later_requests() {
        // oracle: regression #2202: an integrity failure prevents later use of the association.
        let (session, proxy) = connect_session(MockRpchScenario::TamperedResponse).await;
        let mut session = session.expect("rpch connect");

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
        // oracle: spec MS-RPCH §2.1.2.1.1 and §2.1.2.1.2: IN and OUT requests authenticate separately.
        let (mut session, proxy) = open_session(MockRpchScenario::BasicAuthentication).await;
        session
            .send_to_server(Bytes::from_static(b"authenticated"))
            .await
            .expect("send target data");
        session.read_data().await.expect("read target data");
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_surfaces_receive_pipe_terminal_hresult() {
        // oracle: spec MS-TSGU §3.6.5.5: the terminal receive-pipe response carries the call return value.
        const E_PROXY_INTERNAL_ERROR: u32 = 0x0000_59d8;
        let (mut session, proxy) = open_session(MockRpchScenario::ReceivePipeError(E_PROXY_INTERNAL_ERROR)).await;
        let error = session.read_data().await.expect_err("receive pipe error");
        assert!(matches!(error.kind(), GwErrorKind::GatewayCode(code) if *code == E_PROXY_INTERNAL_ERROR));
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_recomputes_ping_deadline_when_recycling_changes_timeout() {
        // oracle: spec MS-RPCH §3.2.1.2.2 and §3.2.2.5.5.
        let (mut session, proxy) = open_session(MockRpchScenario::Echo).await;
        for timeout in [600_000u32, 120_000] {
            let successor = RtsCookie::new([0x73; 16]);
            let mut recycling = session.setup.channel_recycling().expect("recycling");
            recycling.start_in_recycling(successor).expect("start IN replacement");
            let a4 = [
                &[5, 0, 20, 3, 16, 0, 0, 0, 52, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 0][..],
                &13u32.to_le_bytes(),
                &0u32.to_le_bytes(),
                &6u32.to_le_bytes(),
                &1u32.to_le_bytes(),
                &0u32.to_le_bytes(),
                &8192u32.to_le_bytes(),
                &2u32.to_le_bytes(),
                &timeout.to_le_bytes(),
            ]
            .concat();
            recycling.receive_in_recycle_a4(&a4).expect("IN_R1/A4");
            recycling
                .finish_in_recycling(&mut session.flow_control, &mut session.ping)
                .expect("switch IN");
            session.ping.record_send(Duration::ZERO);
            assert_eq!(session.ping.next_ping_time(), Duration::from_millis(u64::from(timeout)));
        }
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_records_normal_rpc_sends_for_idle_pings() {
        // oracle: spec MS-RPCH §3.2.1.1.6.2: every sent PDU updates LastPacketSentTimestamp.
        let (mut session, proxy) = open_session(MockRpchScenario::Echo).await;
        session.started = Instant::now() - Duration::from_secs(120);
        session.ping.record_send(Duration::ZERO);
        assert!(session.ping_due(Duration::from_secs(120)));
        echo(&mut session, b"active").await;
        assert!(!session.ping_due(Duration::from_secs(120)));
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_handles_ping_and_flow_control_acknowledgements() {
        // oracle: spec MS-RPCH §3.2.1.1.4 and §3.2.2.6.1: RPC bytes consume credit, RTS pings do not.
        let (mut session, proxy) = open_session(MockRpchScenario::Echo).await;
        assert!(!session.ping_due(Duration::ZERO));
        let due = session.ping.next_ping_time();
        assert!(session.ping_due(due));
        session.send_ping(due).await.expect("send ping");
        assert!(!session.ping_due(due));

        let data = vec![0; 2 * 1024];
        let data = Bytes::from(data);
        session.send_to_server(data.clone()).await.expect("send target data");
        session.read_data().await.expect("read target data");
        assert_eq!(session.flow_control.send_available_window(), 8 * 1024);
        session
            .send_to_server(data)
            .await
            .expect("send after flow-control acknowledgement");
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_rejects_invalid_out_fragment_length() {
        // oracle: regression #2202: malformed peer framing must fail before exposing target data.
        let (session, proxy) = connect_session(MockRpchScenario::InvalidOutFragmentLength).await;
        let Err(error) = session else {
            panic!("invalid fragment length must fail");
        };
        assert!(matches!(error.kind(), GwErrorKind::Decode));
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_stream_accepts_bytes_before_waiting_for_rpc_completion() {
        // oracle: regression #2202: poll_write reports the bytes it accepted rather than returning Pending afterward.
        let (mut stream, mut commands) = controlled_stream();
        let mut context = Context::from_waker(futures_util::task::noop_waker_ref());
        assert!(matches!(
            Pin::new(&mut stream).poll_write(&mut context, b"accepted"),
            Poll::Ready(Ok(8))
        ));
        let Some(StreamCommand::Write(bytes, _response)) = commands.recv().await else {
            panic!("one write command expected");
        };
        assert_eq!(&*bytes, b"accepted");
    }

    #[tokio::test]
    async fn rpch_stream_cancelled_write_does_not_report_a_previous_buffers_count() {
        // oracle: regression #2202: a cancelled subsequent write neither queues its bytes nor reuses an earlier count.
        let (mut stream, mut commands) = controlled_stream();
        let mut context = Context::from_waker(futures_util::task::noop_waker_ref());
        let _ = Pin::new(&mut stream).poll_write(&mut context, b"previous-buffer");
        let Some(StreamCommand::Write(bytes, response)) = commands.recv().await else {
            panic!("first command expected");
        };
        assert_eq!(&*bytes, b"previous-buffer");
        {
            let mut cancelled = Box::pin(stream.write(b"cancelled"));
            assert!(matches!(cancelled.as_mut().poll(&mut context), Poll::Pending));
        }
        response.send(Ok(())).expect("complete first RPC");
        assert!(matches!(
            Pin::new(&mut stream).poll_write(&mut context, b"x"),
            Poll::Ready(Ok(1))
        ));
        let Some(StreamCommand::Write(bytes, _response)) = commands.recv().await else {
            panic!("replacement command expected");
        };
        assert_eq!(&*bytes, b"x");
    }

    #[tokio::test]
    async fn rpch_stream_flush_after_cancelled_write_observes_completion_and_wakes() {
        // oracle: regression #2202: flush must poll the outstanding completion and arrange wakeup.
        let (mut stream, mut commands) = controlled_stream();
        let mut context = Context::from_waker(futures_util::task::noop_waker_ref());
        {
            let mut cancelled = Box::pin(stream.write(b"queued"));
            let _ = cancelled.as_mut().poll(&mut context);
        }
        let Some(StreamCommand::Write(_, response)) = commands.recv().await else {
            panic!("queued command expected");
        };
        let completion = tokio::spawn(async move {
            tokio::task::yield_now().await;
            response.send(Ok(())).expect("writer waits for completion");
        });
        tokio::time::timeout(Duration::from_secs(1), stream.flush())
            .await
            .expect("flush is woken")
            .expect("RPC completion succeeds");
        completion.await.expect("completion task");
    }

    #[tokio::test]
    async fn rpch_stream_completion_failure_rejects_flush_writes_and_shutdown() {
        // oracle: regression #2202: RPC write failure poisons every later write-side operation.
        let (mut stream, mut commands) = controlled_stream();
        let mut context = Context::from_waker(futures_util::task::noop_waker_ref());
        let _ = Pin::new(&mut stream).poll_write(&mut context, b"request");
        let Some(StreamCommand::Write(_, response)) = commands.recv().await else {
            panic!("queued command expected");
        };
        response
            .send(Err("RPC integrity failure".to_owned()))
            .expect("report failure");
        let error = tokio::time::timeout(Duration::from_secs(1), stream.flush())
            .await
            .expect("flush reports failure")
            .expect_err("RPC failure");
        assert!(error.to_string().contains("RPC integrity failure"));
        stream.write_all(b"later").await.expect_err("no later write");
        stream
            .shutdown()
            .await
            .expect_err("no successful shutdown after failure");
        assert!(commands.try_recv().is_err());
    }

    #[tokio::test]
    async fn rpch_stream_shutdown_waits_for_accepted_bytes_and_can_be_polled_again() {
        // oracle: regression #2202: shutdown drains accepted writes before closing the TS Proxy contexts.
        let (mut stream, mut commands) = controlled_stream();
        let mut context = Context::from_waker(futures_util::task::noop_waker_ref());
        let _ = Pin::new(&mut stream).poll_write(&mut context, b"last bytes");
        let Some(StreamCommand::Write(_, response)) = commands.recv().await else {
            panic!("queued write expected");
        };
        assert!(matches!(
            Pin::new(&mut stream).poll_shutdown(&mut context),
            Poll::Pending
        ));
        assert!(commands.try_recv().is_err());
        response.send(Ok(())).expect("write completion");
        assert!(matches!(
            Pin::new(&mut stream).poll_shutdown(&mut context),
            Poll::Pending
        ));
        let Some(StreamCommand::Shutdown(response)) = commands.recv().await else {
            panic!("shutdown after write completion expected");
        };
        response.send(Ok(())).expect("shutdown completion");
        assert!(matches!(
            Pin::new(&mut stream).poll_shutdown(&mut context),
            Poll::Ready(Ok(()))
        ));
        assert!(matches!(
            Pin::new(&mut stream).poll_shutdown(&mut context),
            Poll::Ready(Ok(()))
        ));
        assert!(matches!(
            Pin::new(&mut stream).poll_write(&mut context, b"after close"),
            Poll::Ready(Err(_))
        ));
    }

    #[tokio::test]
    async fn rpch_stream_shutdown_failure_remains_terminal_on_repeated_polls() {
        // oracle: regression #2202: close failures remain errors instead of repolling a completed receiver.
        let (mut stream, mut commands) = controlled_stream();
        let mut context = Context::from_waker(futures_util::task::noop_waker_ref());
        assert!(matches!(
            Pin::new(&mut stream).poll_shutdown(&mut context),
            Poll::Pending
        ));
        let Some(StreamCommand::Shutdown(response)) = commands.recv().await else {
            panic!("shutdown command expected");
        };
        response.send(Err("close failed".to_owned())).expect("close failure");
        assert!(matches!(
            Pin::new(&mut stream).poll_shutdown(&mut context),
            Poll::Ready(Err(_))
        ));
        assert!(matches!(
            Pin::new(&mut stream).poll_shutdown(&mut context),
            Poll::Ready(Err(_))
        ));
        assert!(matches!(
            Pin::new(&mut stream).poll_flush(&mut context),
            Poll::Ready(Err(_))
        ));
        assert!(matches!(
            Pin::new(&mut stream).poll_write(&mut context, b"later"),
            Poll::Ready(Err(_))
        ));
    }

    #[tokio::test]
    async fn rpch_stream_reads_while_writes_are_in_flight_and_preserves_chunks() {
        // oracle: regression #2202: concurrent input/output preserves the chosen target byte sequence.
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
        // oracle: regression #2202: cancelling an application read must not cancel the owned framing reader.
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
        // oracle: regression #2202: a tampered target-data response terminates the authenticated stream.
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
        // oracle: spec MS-TSGU §3.6.4: graceful shutdown closes the channel before the tunnel.
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

    #[tokio::test]
    async fn rpch_stream_writes_and_shuts_down_while_inbound_data_is_unread() {
        // oracle: regression #2202: receive backpressure must not deadlock outgoing commands or shutdown.
        let (mut stream, proxy) = open_stream(MockRpchScenario::Echo).await;
        let mut expected = Vec::new();
        for index in 0..20u8 {
            let chunk = [b'a' + index; 4];
            tokio::time::timeout(Duration::from_secs(1), stream.write_all(&chunk))
                .await
                .expect("write completes while echoed data is unread")
                .expect("write chunk");
            expected.extend_from_slice(&chunk);
        }

        let mut response = vec![0; expected.len()];
        stream.read_exact(&mut response).await.expect("read echoed data");
        assert_eq!(response, expected);
        tokio::time::timeout(Duration::from_secs(1), stream.shutdown())
            .await
            .expect("shutdown completes")
            .expect("close TS Proxy contexts");
        proxy.abort();
    }

    async fn echo(session: &mut RpchSession<DuplexStream, DuplexStream, MockProvider>, data: &[u8]) {
        session
            .send_to_server(Bytes::copy_from_slice(data))
            .await
            .expect("send target data");
        let mut received = Vec::new();
        while received.len() < data.len() {
            received.extend_from_slice(&session.read_data().await.expect("read target data"));
        }
        assert_eq!(received, data);
    }

    #[tokio::test]
    async fn rpch_session_recycles_in_channel_without_losing_data() {
        // oracle: spec MS-RPCH §3.2.2.5.5 and §3.2.2.5.12: successor IN channels preserve the byte stream.
        let (mut session, proxy) = open_session(MockRpchScenario::RecycleIn).await;
        let chunk = vec![0x5a; 2 * 1024];
        // Enough traffic to cross the quarter-remaining threshold of a 128 KiB IN body at least once.
        for round in 0..64u8 {
            let mut data = chunk.clone();
            data[0] = round;
            echo(&mut session, &data).await;
        }
        assert_eq!(
            session.recycling().expect("recycle state").in_state(),
            RpchInChannelRecycleState::Open
        );
        assert!(session.in_successor.is_none());
        session.close().await.expect("close after IN recycling");
        // The proxy finishes once the default IN channel closes.
        drop(session);
        proxy.await.expect("proxy observed IN recycling");
    }

    #[tokio::test]
    async fn rpch_session_recycles_in_r2_without_losing_data() {
        // oracle: spec MS-RPCH §2.2.4.21 and §3.2.2.5.5.
        let (mut session, proxy) = open_session(MockRpchScenario::RecycleInR2).await;
        for round in 0..64u8 {
            echo(&mut session, &[round; 2 * 1024]).await;
        }
        session.close().await.expect("close after IN_R2 recycling");
        drop(session);
        proxy.await.expect("proxy observed IN_R2 recycling");
    }

    #[tokio::test]
    async fn rpch_session_keeps_cookies_from_a_delayed_successor_head() {
        // oracle: regression #2202: cookies arriving after OUT_R1/A11 must survive switching the channel.
        let (mut session, proxy) = open_session(MockRpchScenario::RecycleOutDelayedHead).await;
        echo(&mut session, b"delayed-head").await;
        assert!(session.cookies.header().expect("cookies").contains("Rotated=successor"));
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_preserves_delayed_successor_rejection_status() {
        // oracle: regression #2202: a rejected replacement must expose its HTTP status after the switch.
        let (mut session, proxy) = open_session(MockRpchScenario::RecycleOutRejectedHead).await;
        let error = session
            .send_to_server(Bytes::from_static(b"rejected"))
            .await
            .expect_err("rejected successor");
        assert!(matches!(error.kind(), GwErrorKind::HttpStatus(403)));
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_follows_out_r1_recycling() {
        // oracle: spec MS-RPCH §3.2.2.5.6 through §3.2.2.5.9: OUT_R1 switches after the handshake.
        let (mut session, proxy) = open_session(MockRpchScenario::RecycleOutR1).await;
        echo(&mut session, b"across-out-r1").await;
        assert_eq!(
            session.recycling().expect("recycle state").out_state(),
            RpchOutChannelRecycleState::Open
        );
        assert!(session.out_successor.is_none());
        echo(&mut session, b"after-out-r1").await;
        session.close().await.expect("close after OUT recycling");
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_follows_out_r2_recycling() {
        // oracle: spec MS-RPCH §3.2.2.5.6 through §3.2.2.5.10: OUT_R2 switches on the predecessor's B3.
        let (mut session, proxy) = open_session(MockRpchScenario::RecycleOutR2).await;
        echo(&mut session, b"across-out-r2").await;
        assert_eq!(
            session.recycling().expect("recycle state").out_state(),
            RpchOutChannelRecycleState::Open
        );
        assert!(session.out_successor.is_none());
        echo(&mut session, b"after-out-r2").await;
        session.close().await.expect("close after OUT recycling");
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_stream_carries_data_across_out_recycling() {
        // oracle: spec MS-RPCH §3.2.2.5.10: switching OUT channels preserves target bytes.
        let (mut stream, proxy) = open_stream(MockRpchScenario::RecycleOutR1).await;
        stream.write_all(b"recycled").await.expect("write across recycle");
        let mut response = [0; 8];
        stream.read_exact(&mut response).await.expect("read across recycle");
        assert_eq!(&response, b"recycled");
        stream.shutdown().await.expect("close TS Proxy contexts");
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_surfaces_in_channel_rpc_error() {
        // oracle: spec MS-RPCH §3.2.2.5.11: proxy RPC-Error codes are returned to higher layers.
        const RPC_S_SERVER_UNAVAILABLE: u32 = 0x6ba;
        let (mut session, proxy) = open_session(MockRpchScenario::InHttpError(RPC_S_SERVER_UNAVAILABLE)).await;
        let error = tokio::time::timeout(
            Duration::from_secs(5),
            session.send_to_server(Bytes::from_static(b"rejected")),
        )
        .await
        .expect("IN failure is observed")
        .expect_err("IN channel error fails the call");
        assert!(matches!(error.kind(), GwErrorKind::GatewayCode(code) if *code == RPC_S_SERVER_UNAVAILABLE));
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_negotiates_messaging_and_cancels_pending_call_on_close() {
        // oracle: spec MS-TSGU §3.6.4: pending messages are cancelled between closing channel and tunnel.
        let (mut session, proxy) = open_session(MockRpchScenario::Messaging).await;
        assert_eq!(session.capabilities, TSG_CLIENT_CAPABILITIES);
        assert!(session.messaging);
        // The service message answered the first message call while the channel was created.
        assert!(session.message_call_id.is_none());
        echo(&mut session, b"with-messaging").await;

        session.request_tunnel_message().await.expect("issue message call");
        assert!(session.message_call_id.is_some());
        // The proxy only closes the tunnel once the pending message call was cancelled.
        session.close().await.expect("close with pending message call");
        assert!(session.message_call_id.is_none());
        assert!(!session.messaging);
        drop(session);
        proxy.await.expect("proxy observed close");
    }

    #[tokio::test]
    async fn rpch_stream_carries_data_and_shuts_down_with_messaging() {
        // oracle: spec MS-TSGU §3.6.4: outstanding administrative calls coexist with data and graceful close.
        let (mut stream, proxy) = open_stream(MockRpchScenario::Messaging).await;
        stream.write_all(b"messaging").await.expect("write with messaging");
        let mut response = [0; 9];
        stream.read_exact(&mut response).await.expect("read with messaging");
        assert_eq!(&response, b"messaging");
        stream.shutdown().await.expect("close with pending message call");
        assert!(
            tokio::time::timeout(Duration::from_secs(1), proxy)
                .await
                .expect("proxy observes stream shutdown")
                .is_ok()
        );
    }

    #[tokio::test]
    async fn rpch_stream_keeps_data_and_shutdown_working_after_a_verified_message_fault() {
        // oracle: spec MS-TSGU §3.6.4: asynchronous messaging is optional and can stop after a returned call.
        let (mut stream, proxy) = open_stream(MockRpchScenario::MessageCallFault).await;
        stream
            .write_all(b"after-message-fault")
            .await
            .expect("write after verified fault");
        let mut response = [0; 19];
        stream
            .read_exact(&mut response)
            .await
            .expect("read after verified fault");
        assert_eq!(&response, b"after-message-fault");
        stream
            .shutdown()
            .await
            .expect("close without cancelling a faulted call");
        assert!(
            tokio::time::timeout(Duration::from_secs(1), proxy)
                .await
                .expect("proxy observes shutdown without another message call")
                .is_ok()
        );
    }

    #[tokio::test]
    async fn rpch_session_rejects_a_tampered_message_fault_and_poisoned_association() {
        // oracle: regression #2202: message-call recovery must not bypass RPC packet integrity.
        let (session, proxy) = connect_session(MockRpchScenario::TamperedMessageCallFault).await;
        let mut session = session.expect("authenticated association");
        session
            .open_tunnel("test-client", "rdp.contoso.com", 3389)
            .await
            .expect_err("tamper must fail");
        assert!(!session.rpc_auth.is_complete());
        session
            .send_call(99, TSPROXY_CREATE_TUNNEL_OPNUM, &[])
            .await
            .expect_err("association is poisoned");
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_rejects_an_unsigned_message_fault_and_later_calls() {
        // oracle: regression #2202: unsigned faults terminate the session rather than disable only messaging.
        let (session, proxy) = connect_session(MockRpchScenario::UnsignedMessageCallFault).await;
        let mut session = session.expect("authenticated association");
        let error = session
            .open_tunnel("test-client", "rdp.contoso.com", 3389)
            .await
            .expect_err("unsigned fault must fail");
        assert!(matches!(error.kind(), GwErrorKind::GatewayCode(5)));
        session
            .send_call(99, TSPROXY_CREATE_TUNNEL_OPNUM, &[])
            .await
            .expect_err("session is poisoned");
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_stream_accepts_a_valid_consent_larger_than_32_kib() {
        // oracle: spec MS-TSGU §2.2.9.2.1.9.1.1: string messages allow 65,536 UTF-16 characters.
        let (mut stream, proxy) = open_stream(MockRpchScenario::LargeConsent).await;
        stream.shutdown().await.expect("close after large consent");
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_stream_dispatches_interleaved_large_message_and_call_responses() {
        // oracle: spec MS-RPCE §3.3.2.5.2 and MS-TSGU §2.2.9.2.1.9.1.1.
        let (mut stream, proxy) = open_stream(MockRpchScenario::InterleavedMessage).await;
        stream.write_all(b"interleaved").await.expect("write beside message");
        let mut response = [0; 11];
        stream.read_exact(&mut response).await.expect("read beside message");
        assert_eq!(&response, b"interleaved");
        stream.shutdown().await.expect("close with messaging");
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_stream_discards_partial_message_after_verified_fault() {
        // oracle: spec MS-RPCE §3.3.2.5.2: independent calls are dispatched by call_id.
        let (mut stream, proxy) = open_stream(MockRpchScenario::PartialMessageFault).await;
        stream
            .write_all(b"after-fault")
            .await
            .expect("write after partial message fault");
        let mut response = [0; 11];
        stream
            .read_exact(&mut response)
            .await
            .expect("read after partial message fault");
        assert_eq!(&response, b"after-fault");
        stream.shutdown().await.expect("close after partial message fault");
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_session_bounds_receive_data_while_a_call_waits() {
        // oracle: regression #2202: the receive queue remains bounded while a synchronous call waits.
        let (mut session, proxy) = open_session(MockRpchScenario::ReceivePipeOverflow).await;
        let error = tokio::time::timeout(
            Duration::from_secs(2),
            session.send_to_server(Bytes::from_static(b"request")),
        )
        .await
        .expect("overflow is reported without waiting for a call response")
        .expect_err("buffer limit");
        assert!(error.to_string().contains("receive buffer"));
        assert!(session.data.iter().map(Bytes::len).sum::<usize>() <= MAX_BUFFERED_DATA);
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_stream_rejects_a_malformed_receive_pipe_final() {
        // oracle: spec MS-TSGU §3.6.5.5: a final response contains exactly one four-byte return value.
        let (mut stream, proxy) = open_stream(MockRpchScenario::InvalidReceivePipeFinal).await;
        let mut buffer = [0; 8];
        tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buffer))
            .await
            .expect("terminal result")
            .expect_err("malformed final response is not target data");
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_stream_rejects_an_unsolicited_partial_call_response() {
        // oracle: spec MS-RPCE §3.3.2.5.2: responses must belong to an outstanding call.
        let (mut stream, proxy) = open_stream(MockRpchScenario::UnexpectedResponse).await;
        let mut buffer = [0; 8];
        tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buffer))
            .await
            .expect("unsolicited response is rejected")
            .expect_err("unknown call fails before buffering its continuation");
        proxy.abort();
    }

    #[tokio::test]
    async fn rpch_stream_answers_reauthentication_on_a_separate_connection() {
        // oracle: spec MS-TSGU §3.2.6.1.4: reauthentication authorizes resources without creating a channel.
        let (report, mut reauthenticated) = mpsc::unbounded_channel();
        let (mut stream, proxy) = open_stream_reporting_reauth(MockRpchScenario::Reauth, Some(report)).await;
        let tunnel_context = tokio::time::timeout(Duration::from_secs(5), reauthenticated.recv())
            .await
            .expect("reauthentication connection completes")
            .expect("reauthentication proxy reports");
        assert_eq!(tunnel_context, REAUTH_TUNNEL_CONTEXT);

        stream.write_all(b"after-reauth").await.expect("write after reauth");
        let mut response = [0; 12];
        stream.read_exact(&mut response).await.expect("read after reauth");
        assert_eq!(&response, b"after-reauth");
        stream.shutdown().await.expect("close after reauth");
        proxy.abort();
    }
}
