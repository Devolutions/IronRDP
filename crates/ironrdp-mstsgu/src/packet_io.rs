//! Packet I/O over the MS-TSGU HTTPS WebSocket or dual HTTP transport.

use core::convert::Infallible;
use std::collections::BTreeMap;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{FutureExt as _, SinkExt as _, StreamExt as _};
use http_body_util::channel::{Channel as BodyChannel, Sender as BodySender};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt as _, Empty};
use hyper::body::{Bytes, Incoming};
use ironrdp_core::{Decode as _, ReadCursor};
use ironrdp_tls::{CertificateValidation, CertificateValidationCallback};
use log::error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::{Message, http};

use crate::http_auth::{AuthStep, GatewayHttpAuth, basic_authorization, www_authenticate_values};
use crate::proto::PktHdr;
use crate::{Error, GwConnectTarget, GwErrorKind};

pub(crate) trait GatewayIo: AsyncRead + AsyncWrite + Unpin {}

impl<T: AsyncRead + AsyncWrite + Unpin + ?Sized> GatewayIo for T {}

type GatewayStream = Box<dyn GatewayIo + Send>;
type RdgBody = BoxBody<Bytes, Infallible>;

const MAX_AUTH_RESPONSE_SIZE: usize = 16 * 1024;
const MAX_PACKET_SIZE: usize = 16 * 1024 * 1024;
const OUT_SEED_SIZE: usize = 10;
const PACKET_HEADER_SIZE: usize = 2 /* type */ + 2 /* reserved */ + 4 /* length */;

struct RdgRequestContext<'a> {
    method: &'a str,
    gw_host: &'a str,
    connection_id: &'a str,
    target: &'a GwConnectTarget,
    websocket_upgrade: bool,
}

/// Session cookies returned by an RD Gateway or its load balancer.
///
/// MS-TSGU can establish its OUT and IN channels on different connections, so
/// cookies returned by either authenticated request are replayed on the next one.
#[derive(Default)]
struct GatewayCookies {
    values: BTreeMap<String, String>,
}

impl GatewayCookies {
    fn capture(&mut self, headers: &http::HeaderMap) {
        for value in &headers.get_all(hyper::header::SET_COOKIE) {
            let Ok(value) = value.to_str() else {
                continue;
            };
            let Some((name, value)) = value.split(';').next().and_then(|cookie| cookie.split_once('=')) else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            self.values.insert(name.to_owned(), format!("{name}={value}"));
        }
    }

    fn header_value(&self) -> Option<String> {
        (!self.values.is_empty()).then(|| self.values.values().cloned().collect::<Vec<_>>().join("; "))
    }
}

/// Backing transport for MS-TSGU protocol packets after HTTP setup.
pub(crate) enum PacketIo {
    WebSocket {
        sink: SplitSink<WebSocketStream<GatewayStream>, Message>,
        stream: SplitStream<WebSocketStream<GatewayStream>>,
    },
    /// Legacy dual channel: read from the OUT response body and write to the IN request body.
    DualHttp {
        out_body: Incoming,
        out_buf: Vec<u8>,
        in_tx: Option<BodySender<Bytes>>,
        in_response: Option<tokio::task::JoinHandle<Result<(), Error>>>,
        _out_stop_tx: oneshot::Sender<()>,
        _out_conn: tokio::task::JoinHandle<()>,
        _in_conn: tokio::task::JoinHandle<()>,
    },
}

impl PacketIo {
    pub(crate) async fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.check_in_response()?;
        match self {
            PacketIo::WebSocket { sink, .. } => {
                sink.send(Message::Binary(Bytes::copy_from_slice(bytes)))
                    .await
                    .map_err(|e| custom_err!("websocket send", e))?;
                Ok(())
            }
            PacketIo::DualHttp { in_tx, .. } => {
                let in_tx = in_tx
                    .as_mut()
                    .ok_or_else(|| Error::new("dual-http in stream closed", GwErrorKind::Connect))?;
                in_tx
                    .send_data(Bytes::copy_from_slice(bytes))
                    .await
                    .map_err(|e| custom_err!("dual-http send", e))?;
                Ok(())
            }
        }
    }

    /// Finishes the WebSocket or dual HTTP IN stream so local write-side EOF can complete.
    pub(crate) async fn close(&mut self) -> Result<(), Error> {
        self.check_in_response()?;
        match self {
            PacketIo::WebSocket { sink, .. } => {
                sink.send(Message::Close(None))
                    .await
                    .map_err(|e| custom_err!("websocket close", e))?;
                sink.flush()
                    .await
                    .map_err(|e| custom_err!("websocket close flush", e))?;
                Ok(())
            }
            PacketIo::DualHttp { in_tx, in_response, .. } => {
                let _ = in_tx.take();
                if let Some(response) = in_response.take() {
                    response
                        .await
                        .map_err(|e| custom_err!("dual-http in response task", e))??;
                }
                Ok(())
            }
        }
    }

    /// Reads one complete MS-TSGU packet buffer.
    ///
    /// Returns `Ok(None)` on a clean close or an exhausted stream.
    pub(crate) async fn read_packet_buf(&mut self) -> Result<Option<Bytes>, Error> {
        self.check_in_response()?;
        match self {
            PacketIo::WebSocket { stream, .. } => {
                let msg = match stream.next().await {
                    None => return Ok(None),
                    Some(Ok(msg)) => msg,
                    Some(Err(e)) => return Err(custom_err!("websocket read", e)),
                };
                if matches!(msg, Message::Close(_)) {
                    return Ok(None);
                }
                Ok(Some(msg.into_data()))
            }
            PacketIo::DualHttp { out_body, out_buf, .. } => read_complete_packet(out_body, out_buf).await,
        }
    }

    fn check_in_response(&mut self) -> Result<(), Error> {
        let PacketIo::DualHttp { in_response, .. } = self else {
            return Ok(());
        };
        let Some(task) = in_response.as_ref() else {
            return Ok(());
        };
        if !task.is_finished() {
            return Ok(());
        }
        let task = in_response
            .take()
            .ok_or_else(|| Error::new("dual-http in response task missing", GwErrorKind::Connect))?;
        let result = task
            .now_or_never()
            .ok_or_else(|| Error::new("dual-http in response task pending", GwErrorKind::Connect))?;
        result.map_err(|e| custom_err!("dual-http in response task", e))?
    }
}

/// Open a TLS transport to the gateway, preferring a WebSocket upgrade and falling back to dual HTTP.
pub(crate) async fn open_gateway_transport(
    target: &GwConnectTarget,
    certificate_validation: CertificateValidation,
    certificate_validation_callback: Option<CertificateValidationCallback>,
) -> Result<(PacketIo, core::net::SocketAddr), Error> {
    let gw_host = target
        .gw_endpoint
        .split(':')
        .next()
        .ok_or_else(|| Error::new("connect", GwErrorKind::InvalidGwTarget))?;
    let gw_host = gw_host.to_owned();

    let stream = TcpStream::connect(&target.gw_endpoint)
        .await
        .map_err(|e| custom_err!("tcp connect", e))?;
    let client_addr = stream
        .local_addr()
        .map_err(|e| custom_err!("get socket local address", e))?;
    let stream = upgrade_gateway_tls(
        stream,
        &gw_host,
        certificate_validation,
        certificate_validation_callback.clone(),
    )
    .await?;

    let in_gw_host = gw_host.clone();
    open_transport_with_out_stream(stream, client_addr, &gw_host, target, move || async move {
        let stream = TcpStream::connect(&target.gw_endpoint)
            .await
            .map_err(|e| custom_err!("tcp connect", e))?;
        upgrade_gateway_tls(
            stream,
            &in_gw_host,
            certificate_validation,
            certificate_validation_callback,
        )
        .await
    })
    .await
}

async fn upgrade_gateway_tls(
    stream: TcpStream,
    gw_host: &str,
    certificate_validation: CertificateValidation,
    certificate_validation_callback: Option<CertificateValidationCallback>,
) -> Result<GatewayStream, Error> {
    let (stream, _) = match select_gateway_tls_upgrade(certificate_validation, certificate_validation_callback) {
        GatewayTlsUpgrade::CertificateValidation(certificate_validation) => {
            ironrdp_tls::upgrade_with_certificate_validation(stream, gw_host, certificate_validation).await
        }
        GatewayTlsUpgrade::CertificateValidationCallback(certificate_validation_callback) => {
            ironrdp_tls::upgrade_with_certificate_validation_callback(stream, gw_host, certificate_validation_callback)
                .await
        }
    }
    .map_err(|e| custom_err!("tls connect", e))?;

    Ok(Box::new(stream))
}

enum GatewayTlsUpgrade {
    CertificateValidation(CertificateValidation),
    CertificateValidationCallback(CertificateValidationCallback),
}

#[cfg(feature = "test-support")]
pub(crate) struct GatewayTlsUpgradeSelection {
    pub(crate) certificate_validation: CertificateValidation,
    pub(crate) uses_callback: bool,
}

fn select_gateway_tls_upgrade(
    certificate_validation: CertificateValidation,
    certificate_validation_callback: Option<CertificateValidationCallback>,
) -> GatewayTlsUpgrade {
    match certificate_validation_callback {
        Some(certificate_validation_callback) => {
            GatewayTlsUpgrade::CertificateValidationCallback(certificate_validation_callback)
        }
        None => GatewayTlsUpgrade::CertificateValidation(certificate_validation),
    }
}

#[cfg(feature = "test-support")]
pub(crate) fn select_gateway_tls_upgrade_for_test(
    certificate_validation: CertificateValidation,
    certificate_validation_callback: Option<CertificateValidationCallback>,
) -> GatewayTlsUpgradeSelection {
    let uses_callback = matches!(
        select_gateway_tls_upgrade(certificate_validation, certificate_validation_callback),
        GatewayTlsUpgrade::CertificateValidationCallback(_)
    );
    GatewayTlsUpgradeSelection {
        certificate_validation,
        uses_callback,
    }
}

async fn open_transport_with_out_stream<F, Fut>(
    out_stream: GatewayStream,
    client_addr: core::net::SocketAddr,
    gw_host: &str,
    target: &GwConnectTarget,
    open_in_stream: F,
) -> Result<(PacketIo, core::net::SocketAddr), Error>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<GatewayStream, Error>>,
{
    let connection_id = format!("{{{}}}", uuid::Uuid::new_v4());
    let spn = format!("HTTP/{gw_host}");
    let out_io = hyper_util::rt::tokio::TokioIo::new(out_stream);
    let (mut out_sender, out_conn) = hyper::client::conn::http1::handshake(out_io)
        .await
        .map_err(|e| custom_err!("http/1 out handshake", e))?;
    let (out_stop_tx, out_stop_rx) = oneshot::channel();
    let mut out_conn = Box::pin(out_conn.without_shutdown());
    let out_conn_jh = tokio::task::spawn(async move {
        tokio::select! {
            result = &mut out_conn => {
                match result {
                    Ok(parts) => Some(parts),
                    Err(error) => {
                        error!("RD Gateway OUT HTTP connection error: {error:?}");
                        None
                    }
                }
            }
            _ = out_stop_rx => None,
        }
    });

    let mut cookies = GatewayCookies::default();
    let context = RdgRequestContext {
        method: "RDG_OUT_DATA",
        gw_host,
        connection_id: &connection_id,
        target,
        websocket_upgrade: true,
    };
    let response = authenticated_rdg_request(&mut out_sender, &context, &spn, &mut cookies).await?;

    match response.status() {
        http::StatusCode::SWITCHING_PROTOCOLS => {
            let parts = out_conn_jh
                .await
                .map_err(|e| custom_err!("websocket upgrade join", e))?
                .ok_or_else(|| Error::new("websocket upgrade connection lost", GwErrorKind::Connect))?;
            let stream = WebSocketStream::from_raw_socket(parts.io.into_inner(), Role::Client, None).await;
            let (sink, stream) = stream.split();
            Ok((PacketIo::WebSocket { sink, stream }, client_addr))
        }
        http::StatusCode::OK => {
            let mut out_body = response.into_body();
            let out_buf = skip_out_seed(&mut out_body).await?;
            drop(out_sender);
            let out_conn = tokio::spawn(async move {
                let _ = out_conn_jh.await;
            });
            let io = open_in_channel(
                open_in_stream().await?,
                gw_host,
                target,
                &connection_id,
                &spn,
                &mut cookies,
                out_body,
                out_buf,
                out_stop_tx,
                out_conn,
            )
            .await?;
            Ok((io, client_addr))
        }
        status => {
            drain_response_body(response.into_body(), "drain rdg out response body").await?;
            Err(Error::new("rdg out data", GwErrorKind::HttpStatus(status.as_u16())))
        }
    }
}

#[cfg(feature = "test-support")]
pub(crate) async fn open_test_transport(
    out_stream: tokio::io::DuplexStream,
    in_stream: tokio::io::DuplexStream,
) -> Result<PacketIo, Error> {
    let target = GwConnectTarget {
        gw_endpoint: "gateway.test:443".to_owned(),
        gw_user: "user".to_owned(),
        gw_pass: "pass".to_owned(),
        smart_card: None,
        server: "server.test".to_owned(),
    };
    let client_addr = "127.0.0.1:0"
        .parse()
        .map_err(|e| custom_err!("parse test client address", e))?;
    open_transport_with_out_stream(
        Box::new(out_stream),
        client_addr,
        "gateway.test",
        &target,
        move || async move {
            let stream: GatewayStream = Box::new(in_stream);
            Ok(stream)
        },
    )
    .await
    .map(|(io, _)| io)
}

#[expect(
    clippy::too_many_arguments,
    reason = "carries both established dual HTTP channel states"
)]
async fn open_in_channel(
    in_stream: GatewayStream,
    gw_host: &str,
    target: &GwConnectTarget,
    connection_id: &str,
    spn: &str,
    cookies: &mut GatewayCookies,
    out_body: Incoming,
    out_buf: Vec<u8>,
    out_stop_tx: oneshot::Sender<()>,
    out_conn: tokio::task::JoinHandle<()>,
) -> Result<PacketIo, Error> {
    let in_io = hyper_util::rt::tokio::TokioIo::new(in_stream);
    let (mut in_sender, in_conn) = hyper::client::conn::http1::handshake(in_io)
        .await
        .map_err(|e| custom_err!("http/1 in handshake", e))?;
    let in_conn = tokio::spawn(async move {
        if let Err(error) = in_conn.await {
            error!("RD Gateway IN HTTP connection error: {error:?}");
        }
    });

    let context = RdgRequestContext {
        method: "RDG_IN_DATA",
        gw_host,
        connection_id,
        target,
        websocket_upgrade: false,
    };
    let response = authenticated_rdg_request(&mut in_sender, &context, spn, cookies).await?;
    if response.status() != http::StatusCode::OK {
        let status = response.status();
        drain_response_body(response.into_body(), "drain rdg in response body").await?;
        return Err(Error::new("rdg in data", GwErrorKind::HttpStatus(status.as_u16())));
    }
    drain_response_body(response.into_body(), "drain rdg in authentication body").await?;

    let (in_tx, in_body) = BodyChannel::<Bytes>::new(16);
    let write_req = build_rdg_request(&context, None, false, cookies, in_body.boxed(), true)?;

    let in_response = tokio::spawn(async move {
        let response = in_sender
            .send_request(write_req)
            .await
            .map_err(|e| custom_err!("send rdg in data request", e))?;
        let status = response.status();
        drain_response_body(response.into_body(), "drain rdg in data response").await?;
        if status != http::StatusCode::OK {
            return Err(Error::new("rdg in data", GwErrorKind::HttpStatus(status.as_u16())));
        }
        Ok(())
    });

    Ok(PacketIo::DualHttp {
        out_body,
        out_buf,
        in_tx: Some(in_tx),
        in_response: Some(in_response),
        _out_stop_tx: out_stop_tx,
        _out_conn: out_conn,
        _in_conn: in_conn,
    })
}

async fn authenticated_rdg_request(
    sender: &mut hyper::client::conn::http1::SendRequest<RdgBody>,
    context: &RdgRequestContext<'_>,
    spn: &str,
    cookies: &mut GatewayCookies,
) -> Result<http::Response<Incoming>, Error> {
    let mut http_auth: Option<GatewayHttpAuth> = None;
    let mut authorization: Option<String> = None;
    let mut use_basic = false;
    const MAX_AUTH_ROUNDS: usize = 8;

    for _ in 0..MAX_AUTH_ROUNDS {
        let request = build_rdg_request(
            context,
            authorization.as_deref(),
            use_basic,
            cookies,
            empty_rdg_body(),
            false,
        )?;
        let response = sender
            .send_request(request)
            .await
            .map_err(|e| custom_err!("send rdg authentication request", e))?;
        cookies.capture(response.headers());

        if response.status() != http::StatusCode::UNAUTHORIZED {
            if let Some(mut auth) = http_auth {
                let challenges: Vec<String> = www_authenticate_values(response.headers())
                    .into_iter()
                    .map(str::to_owned)
                    .collect();
                run_http_auth(move || auth.finish_www_authenticate(challenges.iter().map(String::as_str))).await?;
            }
            return Ok(response);
        }

        if use_basic {
            let status = response.status();
            drain_response_body(response.into_body(), "drain rdg basic authentication body").await?;
            return Err(Error::new(
                "rdg basic authentication",
                GwErrorKind::HttpStatus(status.as_u16()),
            ));
        }

        let challenges: Vec<String> = www_authenticate_values(response.headers())
            .into_iter()
            .map(str::to_owned)
            .collect();
        drain_response_body(response.into_body(), "drain rdg authentication body").await?;

        let user = context.target.gw_user.clone();
        let pass = context.target.gw_pass.clone();
        let smart_card = context.target.smart_card.clone();
        let target_name = spn.to_owned();
        let step = if let Some(mut auth) = http_auth.take() {
            let (auth, step) = run_http_auth(move || {
                let refs: Vec<&str> = challenges.iter().map(String::as_str).collect();
                let step = auth.step_www_authenticate(refs)?;
                Ok((auth, step))
            })
            .await?;
            http_auth = Some(auth);
            step
        } else {
            let (auth, step) = run_http_auth(move || {
                let refs: Vec<&str> = challenges.iter().map(String::as_str).collect();
                GatewayHttpAuth::from_challenges(&user, &pass, smart_card.as_deref(), Some(target_name), &refs)
            })
            .await?;
            http_auth = auth;
            step
        };

        match step {
            AuthStep::Continue(next) => authorization = Some(next),
            AuthStep::TryBasic => use_basic = true,
            AuthStep::Complete => {
                return Err(Error::new(
                    "rdg authentication completed without a successful response",
                    GwErrorKind::Connect,
                ));
            }
        }
    }

    Err(Error::new("rdg authentication rounds exceeded", GwErrorKind::Connect))
}

fn empty_rdg_body() -> RdgBody {
    Empty::<Bytes>::new().boxed()
}

fn build_rdg_request(
    context: &RdgRequestContext<'_>,
    authorization: Option<&str>,
    use_basic: bool,
    cookies: &GatewayCookies,
    body: RdgBody,
    content_type: bool,
) -> Result<http::Request<RdgBody>, Error> {
    let mut request = http::Request::builder()
        .method(context.method)
        .uri("/remoteDesktopGateway/")
        .header(hyper::header::HOST, context.gw_host)
        .header("Rdg-Connection-Id", context.connection_id)
        .header(hyper::header::ACCEPT, "*/*")
        .header(hyper::header::CACHE_CONTROL, "no-cache")
        .header(hyper::header::PRAGMA, "no-cache");
    if let Some(cookie_header) = cookies.header_value() {
        request = request.header(hyper::header::COOKIE, cookie_header);
    }
    if context.websocket_upgrade {
        request = request
            .header(hyper::header::CONNECTION, "Upgrade")
            .header(hyper::header::UPGRADE, "websocket")
            .header(hyper::header::SEC_WEBSOCKET_VERSION, "13")
            .header(hyper::header::SEC_WEBSOCKET_KEY, generate_key());
    }
    if use_basic {
        request = request.header(
            hyper::header::AUTHORIZATION,
            basic_authorization(&context.target.gw_user, &context.target.gw_pass),
        );
    } else if let Some(authorization) = authorization {
        request = request.header(hyper::header::AUTHORIZATION, authorization);
    }
    if content_type {
        request = request.header(hyper::header::CONTENT_TYPE, "application/octet-stream");
    }
    request.body(body).map_err(|e| custom_err!("build rdg request", e))
}

async fn read_complete_packet(body: &mut Incoming, buf: &mut Vec<u8>) -> Result<Option<Bytes>, Error> {
    while buf.len() < PACKET_HEADER_SIZE {
        if !pull_body_chunk(body, buf).await? {
            return if buf.is_empty() {
                Ok(None)
            } else {
                Err(Error::new("dual-http packet header truncated", GwErrorKind::PacketEof))
            };
        }
    }
    let total = {
        let mut cursor = ReadCursor::new(buf);
        let header = PktHdr::decode(&mut cursor).map_err(|e| custom_err!("decode packet header", e))?;
        usize::try_from(header.length).map_err(|_| Error::new("packet length", GwErrorKind::Decode))?
    };
    if !(PACKET_HEADER_SIZE..=MAX_PACKET_SIZE).contains(&total) {
        return Err(Error::new("invalid packet length", GwErrorKind::Decode));
    }
    while buf.len() < total {
        if !pull_body_chunk(body, buf).await? {
            return Err(Error::new("dual-http packet truncated", GwErrorKind::PacketEof));
        }
    }
    let packet = Bytes::copy_from_slice(&buf[..total]);
    buf.drain(..total);
    Ok(Some(packet))
}

async fn pull_body_chunk(body: &mut Incoming, buf: &mut Vec<u8>) -> Result<bool, Error> {
    loop {
        let Some(frame) = body.frame().await else {
            return Ok(false);
        };
        let frame = frame.map_err(|e| custom_err!("dual-http out read", e))?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        if data.is_empty() {
            continue;
        }
        if buf.len().saturating_add(data.len()) > MAX_PACKET_SIZE {
            return Err(Error::new("dual-http packet exceeds limit", GwErrorKind::Decode));
        }
        buf.extend_from_slice(&data);
        return Ok(true);
    }
}

async fn skip_out_seed(body: &mut Incoming) -> Result<Vec<u8>, Error> {
    let mut skipped = 0usize;
    while skipped < OUT_SEED_SIZE {
        let Some(frame) = body.frame().await else {
            return Ok(Vec::new());
        };
        let frame = frame.map_err(|e| custom_err!("read rdg out seed", e))?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        let needed = OUT_SEED_SIZE - skipped;
        if data.len() <= needed {
            skipped += data.len();
        } else {
            if data.len() - needed > MAX_PACKET_SIZE {
                return Err(Error::new("dual-http packet exceeds limit", GwErrorKind::Decode));
            }
            return Ok(data[needed..].to_vec());
        }
    }
    Ok(Vec::new())
}

async fn drain_response_body(mut body: Incoming, context: &'static str) -> Result<(), Error> {
    let mut size = 0usize;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|e| custom_err!(context, e))?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        size = size
            .checked_add(data.len())
            .ok_or_else(|| Error::new("http response body too large", GwErrorKind::Decode))?;
        if size > MAX_AUTH_RESPONSE_SIZE {
            return Err(Error::new("http response body too large", GwErrorKind::Decode));
        }
    }
    Ok(())
}

async fn run_http_auth<T, F>(f: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, Error> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| custom_err!("http auth task", e))?
}
