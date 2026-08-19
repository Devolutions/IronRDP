//! Packet I/O over WebSocket or legacy dual-channel HTTP ([MS-TSGU] 1.3.2 / 3.3.5.1).

use core::convert::Infallible;
use std::collections::BTreeMap;

use base64::Engine as _;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt as _, StreamExt as _};
use http_body_util::channel::Channel as BodyChannel;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt as _, Empty};
use hyper::body::{Bytes, Incoming};
use hyper::client::conn::http1::SendRequest;
use ironrdp_core::{Decode as _, ReadCursor};
use ironrdp_tls::{CertificateValidation, CertificateValidationCallback, TlsStream};
use log::{debug, error, info};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio_socks::tcp::Socks5Stream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::protocol::Role;
use url::Url;

use crate::http_auth::{
    AuthStep, GatewayHttpAuth, basic_authorization, challenges_offer_basic, www_authenticate_values,
};
use crate::proto::PktHdr;
use crate::{Error, GwConnectTarget, GwErrorKind, parse_gateway_endpoint};

/// Erased HTTP body so auth (`Empty`) and dual-channel writes (`Channel`) share one `SendRequest`.
type RdgBody = BoxBody<Bytes, Infallible>;

struct RdgRequestContext<'a> {
    method: &'a str,
    gw_host: &'a str,
    connection_id: &'a str,
    target: &'a GwConnectTarget,
    websocket_upgrade: bool,
}

fn empty_rdg_body() -> RdgBody {
    Empty::<Bytes>::new().boxed()
}

/// Session cookies returned by an RD Gateway or its load balancer.
///
/// MS-TSGU may split a session across multiple HTTP connections, so cookies returned by one
/// connection must be replayed on each subsequent request.
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

enum GatewayProxy {
    HttpConnect {
        authority: String,
        authorization: Option<String>,
    },
    Socks5 {
        authority: String,
        credentials: Option<(String, String)>,
    },
}

fn proxy_from_url(value: &str) -> Result<GatewayProxy, Error> {
    let url = Url::parse(value).map_err(|e| custom_err!("parse HTTPS proxy URL", e))?;
    let host = url
        .host_str()
        .ok_or_else(|| Error::new("HTTPS proxy host", GwErrorKind::InvalidGwTarget))?;
    let port = url.port_or_known_default().expect("http URL has a known default port");
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let credentials = (!url.username().is_empty() || url.password().is_some())
        .then(|| (url.username().to_owned(), url.password().unwrap_or_default().to_owned()));

    match url.scheme() {
        "http" => {
            let authorization = credentials.map(|(username, password)| {
                let credentials = format!("{username}:{password}");
                format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(credentials.as_bytes())
                )
            });
            Ok(GatewayProxy::HttpConnect {
                authority,
                authorization,
            })
        }
        "socks5" | "socks5h" => Ok(GatewayProxy::Socks5 { authority, credentials }),
        _ => Err(Error::new("HTTPS proxy scheme", GwErrorKind::UnsupportedFeature)),
    }
}

fn no_proxy_matches(gw_host: &str, no_proxy: &str) -> bool {
    no_proxy.split(',').map(str::trim).any(|entry| {
        if entry == "*" {
            return true;
        }
        let entry = entry.strip_prefix('.').unwrap_or(entry);
        if gw_host.eq_ignore_ascii_case(entry) {
            return true;
        }

        let Some(suffix) = gw_host.get(gw_host.len().saturating_sub(entry.len())..) else {
            return false;
        };
        let Some(prefix) = gw_host.get(..gw_host.len().saturating_sub(entry.len())) else {
            return false;
        };
        suffix.eq_ignore_ascii_case(entry) && prefix.ends_with('.')
    })
}

fn proxy_from_environment(gw_host: &str) -> Result<Option<GatewayProxy>, Error> {
    let no_proxy = std::env::var("NO_PROXY")
        .ok()
        .or_else(|| std::env::var("no_proxy").ok());
    if no_proxy
        .as_deref()
        .is_some_and(|value| no_proxy_matches(gw_host, value))
    {
        return Ok(None);
    }

    let value = std::env::var("HTTPS_PROXY")
        .ok()
        .or_else(|| std::env::var("https_proxy").ok());
    value.as_deref().map(proxy_from_url).transpose()
}

async fn read_proxy_response_headers(stream: &mut TcpStream) -> Result<Vec<u8>, Error> {
    const MAX_RESPONSE_SIZE: usize = 16 * 1024;
    let mut response = Vec::with_capacity(128);
    loop {
        if response.len() == MAX_RESPONSE_SIZE {
            return Err(Error::new("HTTP CONNECT proxy response headers", GwErrorKind::Decode));
        }
        response.push(
            stream
                .read_u8()
                .await
                .map_err(|e| custom_err!("read HTTP CONNECT response", e))?,
        );
        if response.ends_with(b"\r\n\r\n") {
            return Ok(response);
        }
    }
}

async fn tcp_connect(connect_addr: &str, gw_host: &str) -> Result<(TcpStream, core::net::SocketAddr), Error> {
    let Some(proxy) = proxy_from_environment(gw_host)? else {
        let stream = TcpStream::connect(connect_addr)
            .await
            .map_err(|e| custom_err!("tcp connect", e))?;
        let client_addr = stream
            .local_addr()
            .map_err(|e| custom_err!("get socket local address", e))?;
        return Ok((stream, client_addr));
    };

    match proxy {
        GatewayProxy::HttpConnect {
            authority,
            authorization,
        } => {
            let mut stream = TcpStream::connect(&authority)
                .await
                .map_err(|e| custom_err!("connect HTTPS proxy", e))?;
            let client_addr = stream
                .local_addr()
                .map_err(|e| custom_err!("get HTTPS proxy local address", e))?;
            let mut request = format!("CONNECT {connect_addr} HTTP/1.1\r\nHost: {connect_addr}\r\n");
            if let Some(authorization) = authorization {
                request.push_str("Proxy-Authorization: ");
                request.push_str(&authorization);
                request.push_str("\r\n");
            }
            request.push_str("Proxy-Connection: keep-alive\r\n\r\n");
            stream
                .write_all(request.as_bytes())
                .await
                .map_err(|e| custom_err!("send HTTP CONNECT request", e))?;

            let response = read_proxy_response_headers(&mut stream).await?;
            let response =
                core::str::from_utf8(&response).map_err(|e| custom_err!("decode HTTP CONNECT response", e))?;
            let status = response
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|status| status.parse::<u16>().ok())
                .ok_or_else(|| Error::new("HTTP CONNECT proxy status", GwErrorKind::Decode))?;
            if status != http::StatusCode::OK.as_u16() {
                return Err(Error::new("HTTP CONNECT proxy", GwErrorKind::HttpStatus(status)));
            }

            Ok((stream, client_addr))
        }
        GatewayProxy::Socks5 { authority, credentials } => {
            let stream = if let Some((username, password)) = credentials {
                Socks5Stream::connect_with_password(authority.as_str(), connect_addr, &username, &password)
                    .await
                    .map_err(|e| custom_err!("connect SOCKS5 proxy", e))?
            } else {
                Socks5Stream::connect(authority.as_str(), connect_addr)
                    .await
                    .map_err(|e| custom_err!("connect SOCKS5 proxy", e))?
            };
            let client_addr = stream
                .local_addr()
                .map_err(|e| custom_err!("get SOCKS5 proxy local address", e))?;

            Ok((stream.into_inner(), client_addr))
        }
    }
}

/// Backing transport for MS-TSGU protocol packets after HTTP setup.
pub(crate) enum PacketIo {
    WebSocket {
        sink: SplitSink<WebSocketStream<TlsStream<TcpStream>>, Message>,
        stream: SplitStream<WebSocketStream<TlsStream<TcpStream>>>,
        /// Whether the HTTP layer authenticated with Basic (no SSPI NTLM blobs
        /// available for the post-handshake extended-auth exchange).
        http_auth_basic: bool,
    },
    /// Legacy dual channel: read from OUT response body, write to IN request body.
    DualHttp {
        out_body: Incoming,
        out_buf: Vec<u8>,
        in_tx: http_body_util::channel::Sender<Bytes>,
        /// Whether the HTTP layer authenticated with Basic.
        http_auth_basic: bool,
        _out_stop_tx: oneshot::Sender<()>,
        _out_conn: tokio::task::JoinHandle<()>,
        _in_conn: tokio::task::JoinHandle<()>,
    },
    /// In-memory transport for mock gateway tests.
    #[cfg(test)]
    Duplex {
        stream: tokio::io::DuplexStream,
        buf: Vec<u8>,
    },
}

impl PacketIo {
    /// Whether the HTTP layer authenticated with Basic. Basic produces no SSPI NTLM
    /// blobs, so the MS-TSGU extended-auth exchange must not be advertised.
    pub(crate) fn http_auth_basic(&self) -> bool {
        match self {
            PacketIo::WebSocket { http_auth_basic, .. } => *http_auth_basic,
            PacketIo::DualHttp { http_auth_basic, .. } => *http_auth_basic,
            #[cfg(test)]
            PacketIo::Duplex { .. } => false,
        }
    }

    /// Wrap one side of an in-memory duplex pair for mock gateway tests.
    #[cfg(test)]
    pub(crate) fn duplex(stream: tokio::io::DuplexStream) -> Self {
        Self::Duplex {
            stream,
            buf: Vec::new(),
        }
    }

    pub(crate) async fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        match self {
            PacketIo::WebSocket { sink, .. } => {
                sink.send(Message::Binary(Bytes::copy_from_slice(bytes)))
                    .await
                    .map_err(|e| custom_err!("websocket send", e))?;
                Ok(())
            }
            PacketIo::DualHttp { in_tx, .. } => {
                in_tx
                    .send_data(Bytes::copy_from_slice(bytes))
                    .await
                    .map_err(|e| custom_err!("dual-http send", e))?;
                Ok(())
            }
            #[cfg(test)]
            PacketIo::Duplex { stream, .. } => {
                stream
                    .write_all(bytes)
                    .await
                    .map_err(|e| custom_err!("duplex send", e))?;
                Ok(())
            }
        }
    }

    /// Closes the transport gracefully: a WebSocket close frame, or the end of the
    /// dual-channel IN request body.
    pub(crate) async fn close(&mut self) -> Result<(), Error> {
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
            PacketIo::DualHttp { in_tx, .. } => {
                let _ = in_tx; // dropping the sender ends the IN request body
                Ok(())
            }
            #[cfg(test)]
            PacketIo::Duplex { stream, .. } => {
                stream.shutdown().await.map_err(|e| custom_err!("duplex close", e))?;
                Ok(())
            }
        }
    }

    /// Read one complete MS-TSGU packet buffer (header + body).
    pub(crate) async fn read_packet_buf(&mut self) -> Result<Bytes, Error> {
        match self {
            PacketIo::WebSocket { stream, .. } => {
                let msg = stream
                    .next()
                    .await
                    .ok_or_else(|| Error::new("websocket stream closed", GwErrorKind::Connect))?
                    .map_err(|e| custom_err!("websocket read", e))?
                    .into_data();
                Ok(msg)
            }
            PacketIo::DualHttp { out_body, out_buf, .. } => read_complete_packet(out_body, out_buf).await,
            #[cfg(test)]
            PacketIo::Duplex { stream, buf } => read_complete_packet_duplex(stream, buf).await,
        }
    }
}

/// Read one complete MS-TSGU packet from a duplex stream (header length prefixed).
#[cfg(test)]
async fn read_complete_packet_duplex(stream: &mut tokio::io::DuplexStream, buf: &mut Vec<u8>) -> Result<Bytes, Error> {
    while buf.len() < PktHdr::FIXED_PART_SIZE {
        pull_duplex_chunk(stream, buf).await?;
    }

    let hdr = {
        let mut cur = ReadCursor::new(buf);
        PktHdr::decode(&mut cur).map_err(|e| custom_err!("decode packet header", e))?
    };
    let total = usize::try_from(hdr.length).map_err(|_| Error::new("packet header length", GwErrorKind::Decode))?;
    if total < PktHdr::FIXED_PART_SIZE {
        return Err(Error::new("packet length smaller than header", GwErrorKind::Decode));
    }

    while buf.len() < total {
        pull_duplex_chunk(stream, buf).await?;
    }

    let packet = Bytes::copy_from_slice(&buf[..total]);
    buf.drain(..total);
    Ok(packet)
}

#[cfg(test)]
async fn pull_duplex_chunk(stream: &mut tokio::io::DuplexStream, buf: &mut Vec<u8>) -> Result<(), Error> {
    let mut chunk = [0u8; 4096];
    let n = stream
        .read(&mut chunk)
        .await
        .map_err(|e| custom_err!("duplex read", e))?;
    if n == 0 {
        return Err(Error::new("duplex stream closed", GwErrorKind::Connect));
    }
    buf.extend_from_slice(&chunk[..n]);
    Ok(())
}

async fn read_complete_packet(body: &mut Incoming, buf: &mut Vec<u8>) -> Result<Bytes, Error> {
    while buf.len() < PktHdr::FIXED_PART_SIZE {
        pull_body_chunk(body, buf).await?;
    }

    let hdr = {
        let mut cur = ReadCursor::new(buf);
        PktHdr::decode(&mut cur).map_err(|e| custom_err!("decode packet header", e))?
    };
    let total = usize::try_from(hdr.length).map_err(|_| Error::new("packet header length", GwErrorKind::Decode))?;
    if total < PktHdr::FIXED_PART_SIZE {
        return Err(Error::new("packet length smaller than header", GwErrorKind::Decode));
    }

    while buf.len() < total {
        pull_body_chunk(body, buf).await?;
    }

    let packet = Bytes::copy_from_slice(&buf[..total]);
    buf.drain(..total);
    Ok(packet)
}

async fn pull_body_chunk(body: &mut Incoming, buf: &mut Vec<u8>) -> Result<(), Error> {
    loop {
        let frame = body
            .frame()
            .await
            .ok_or_else(|| Error::new("dual-http out stream closed", GwErrorKind::Connect))?
            .map_err(|e| custom_err!("dual-http out read", e))?;
        if let Ok(data) = frame.into_data() {
            if data.is_empty() {
                continue;
            }
            buf.extend_from_slice(&data);
            return Ok(());
        }
    }
}

/// Open transport: prefer WebSocket upgrade; fall back to dual-channel HTTP on HTTP 200.
///
/// Some gateways answer the SSPI (Negotiate/NTLM) handshake on the RDG_OUT_DATA channel
/// by requesting a TLS renegotiation, which rustls does not support; the connection is
/// reset mid-handshake. When that happens and the gateway offered Basic, retry once on a
/// fresh connection using Basic (safe: the whole transport is TLS).
pub(crate) async fn open_transport_prefer_websocket(
    target: &GwConnectTarget,
    certificate_validation: CertificateValidation,
    certificate_validation_callback: Option<CertificateValidationCallback>,
) -> Result<(PacketIo, core::net::SocketAddr), Error> {
    let attempt = match rdg_out_handshake(target, certificate_validation, certificate_validation_callback.clone()).await
    {
        Ok(Ok(transport)) => return Ok(transport),
        Ok(Err(error)) => {
            // The handshake completed the auth exchange but failed at the transport step.
            return Err(error);
        }
        Err(attempt) => attempt,
    };

    if attempt.saw_reset && attempt.basic_offered {
        info!("RD Gateway reset the OUT channel during SSPI auth; retrying with Basic");
        return rdg_out_handshake_basic(
            target,
            certificate_validation,
            certificate_validation_callback,
            &attempt.connection_id,
        )
        .await;
    }

    Err(attempt.error)
}

/// Outcome of a failed RDG OUT-channel handshake, tracking whether the connection was
/// reset mid-auth and whether the gateway advertised Basic. Carries the connection id so
/// a Basic retry reuses the channel identity the gateway saw during the first attempt.
struct RdgOutFailure {
    error: Error,
    saw_reset: bool,
    basic_offered: bool,
    connection_id: String,
}

/// Retry the OUT-channel handshake using HTTP Basic on a fresh connection, reusing the
/// connection id from the failed SSPI attempt.
async fn rdg_out_handshake_basic(
    target: &GwConnectTarget,
    certificate_validation: CertificateValidation,
    certificate_validation_callback: Option<CertificateValidationCallback>,
    connection_id: &str,
) -> Result<(PacketIo, core::net::SocketAddr), Error> {
    let (gw_host, connect_addr) = parse_gateway_endpoint(&target.gw_endpoint)?;
    let spn = format!("HTTP/{gw_host}");

    let (out_stream, client_addr) = tls_connect(
        &connect_addr,
        gw_host,
        certificate_validation,
        certificate_validation_callback.clone(),
    )
    .await?;

    let out_io = hyper_util::rt::tokio::TokioIo::new(out_stream);
    let (mut out_sender, out_conn) = hyper::client::conn::http1::handshake(out_io)
        .await
        .map_err(|e| custom_err!("http/1 out handshake", e))?;

    // The Basic retry sends a single committing request, so the upgrade completes the
    // connection driver. Poll it with `without_shutdown`, which yields the connection
    // parts once the upgrade finishes instead of shutting the IO down.
    let out_conn_jh = tokio::task::spawn(out_conn.without_shutdown());

    let mut cookies = GatewayCookies::default();
    let request_context = RdgRequestContext {
        method: "RDG_OUT_DATA",
        gw_host,
        connection_id,
        target,
        websocket_upgrade: true,
    };
    let req = build_rdg_request(&request_context, None, true, &cookies, empty_rdg_body())?;
    let resp = out_sender
        .send_request(req)
        .await
        .map_err(|e| custom_err!("rdg out send", e))?;
    cookies.capture(resp.headers());

    if resp.status() == http::StatusCode::SWITCHING_PROTOCOLS {
        let parts = out_conn_jh
            .await
            .map_err(|e| custom_err!("websocket upgrade join", e))?
            .map_err(|e| custom_err!("rdg out upgrade", e))?;
        let tls_stream = parts.io.into_inner();
        let ws_stream = WebSocketStream::from_raw_socket(tls_stream, Role::Client, None).await;
        let (sink, stream) = ws_stream.split();
        return Ok((
            PacketIo::WebSocket {
                sink,
                stream,
                http_auth_basic: true,
            },
            client_addr,
        ));
    }

    if resp.status() == http::StatusCode::OK {
        info!("RD Gateway WebSocket upgrade unavailable; using dual-channel HTTP");
        let out_conn_task = tokio::task::spawn(async move {
            let _ = out_conn_jh.await;
        });
        let (out_stop_tx, out_stop_rx) = oneshot::channel::<()>();
        drop(out_stop_rx);
        let mut out_body = resp.into_body();
        let leftover = skip_seed_payload(&mut out_body).await?;
        drop(out_sender);
        let dual = open_in_channel(
            &connect_addr,
            gw_host,
            connection_id,
            target,
            &spn,
            certificate_validation,
            certificate_validation_callback,
            &mut cookies,
            out_stop_tx,
            out_body,
            leftover,
            out_conn_task,
            true,
        )
        .await?;
        return Ok((dual, client_addr));
    }

    Err(Error::new(
        "rdg out data",
        GwErrorKind::HttpStatus(resp.status().as_u16()),
    ))
}

/// Run the RDG OUT-channel handshake (WebSocket upgrade or dual-channel HTTP), driving
/// the SSPI auth exchange. On failure, reports whether a mid-auth connection reset and a
/// Basic challenge were observed so the caller can retry with Basic.
#[expect(clippy::too_many_lines, reason = "linear request/response auth sequence")]
async fn rdg_out_handshake(
    target: &GwConnectTarget,
    certificate_validation: CertificateValidation,
    certificate_validation_callback: Option<CertificateValidationCallback>,
) -> Result<Result<(PacketIo, core::net::SocketAddr), Error>, RdgOutFailure> {
    let connection_id = format!("{{{}}}", uuid::Uuid::new_v4());
    let mut failure = RdgOutFailure {
        error: Error::new("rdg out data", GwErrorKind::Connect),
        saw_reset: false,
        basic_offered: false,
        connection_id: connection_id.clone(),
    };

    let (gw_host, connect_addr) = match parse_gateway_endpoint(&target.gw_endpoint) {
        Ok(ok) => ok,
        Err(error) => {
            failure.error = error;
            return Err(failure);
        }
    };
    let spn = format!("HTTP/{gw_host}");

    let (out_stream, client_addr) = match tls_connect(
        &connect_addr,
        gw_host,
        certificate_validation,
        certificate_validation_callback.clone(),
    )
    .await
    {
        Ok(ok) => ok,
        Err(error) => {
            failure.error = error;
            return Err(failure);
        }
    };

    let out_io = hyper_util::rt::tokio::TokioIo::new(out_stream);
    let (mut out_sender, mut out_conn) = match hyper::client::conn::http1::handshake(out_io).await {
        Ok(ok) => ok,
        Err(e) => {
            failure.error = custom_err!("http/1 out handshake", e);
            return Err(failure);
        }
    };

    let (out_stop_tx, out_stop_rx) = oneshot::channel::<()>();
    let out_conn_jh = tokio::task::spawn(async move {
        tokio::select! {
            res = &mut out_conn => {
                if let Err(e) = res {
                    error!("RD Gateway OUT HTTP connection error: {e:?}");
                }
                None
            }
            _ = out_stop_rx => {
                Some(out_conn.into_parts())
            }
        }
    });

    let mut http_auth: Option<GatewayHttpAuth> = None;
    let mut authorization: Option<String> = None;
    let mut use_basic = false;
    let mut cookies = GatewayCookies::default();
    let request_context = RdgRequestContext {
        method: "RDG_OUT_DATA",
        gw_host,
        connection_id: &connection_id,
        target,
        websocket_upgrade: true,
    };
    const MAX_AUTH_ROUNDS: usize = 8;

    for _ in 0..MAX_AUTH_ROUNDS {
        let req = match build_rdg_request(
            &request_context,
            authorization.as_deref(),
            use_basic,
            &cookies,
            empty_rdg_body(),
        ) {
            Ok(req) => req,
            Err(error) => {
                failure.error = error;
                return Err(failure);
            }
        };

        let resp = match out_sender.send_request(req).await {
            Ok(resp) => resp,
            Err(e) => {
                failure.saw_reset = error_chain_has_connection_reset(&e);
                failure.error = custom_err!("rdg out send", e);
                return Err(failure);
            }
        };
        cookies.capture(resp.headers());

        if resp.status() == http::StatusCode::UNAUTHORIZED {
            let challenges = www_authenticate_values(resp.headers());
            failure.basic_offered |= challenges_offer_basic(&challenges);
        }

        let is_upgrade = resp.status() == http::StatusCode::SWITCHING_PROTOCOLS;
        let is_dual_http = resp.status() == http::StatusCode::OK;
        if is_upgrade || is_dual_http {
            return match finish_rdg_out_response(
                resp,
                target,
                &connect_addr,
                gw_host,
                &connection_id,
                &spn,
                certificate_validation,
                certificate_validation_callback,
                cookies,
                out_sender,
                out_stop_tx,
                out_conn_jh,
                client_addr,
                use_basic,
            )
            .await
            {
                Ok(ok) => Ok(Ok(ok)),
                Err(error) => {
                    failure.error = error;
                    Err(failure)
                }
            };
        }

        if resp.status() != http::StatusCode::UNAUTHORIZED {
            failure.error = Error::new("rdg out data", GwErrorKind::HttpStatus(resp.status().as_u16()));
            return Err(failure);
        }

        if use_basic {
            failure.error = Error::new("rdg out basic auth", GwErrorKind::HttpStatus(resp.status().as_u16()));
            return Err(failure);
        }

        let challenges: Vec<String> = www_authenticate_values(resp.headers())
            .into_iter()
            .map(str::to_owned)
            .collect();
        if let Err(e) = resp.into_body().collect().await {
            failure.error = custom_err!("drain rdg out auth body", e);
            return Err(failure);
        }

        let challenge_refs: Vec<&str> = challenges.iter().map(String::as_str).collect();
        let step = match if let Some(auth) = http_auth.as_mut() {
            auth.step_www_authenticate(challenge_refs)
        } else {
            GatewayHttpAuth::from_challenges(
                &target.gw_user,
                &target.gw_pass,
                target.smart_card.as_deref(),
                Some(spn.clone()),
                &challenge_refs,
            )
            .map(|(auth, step)| {
                http_auth = Some(auth);
                step
            })
        } {
            Ok(step) => step,
            Err(error) => {
                failure.error = error;
                return Err(failure);
            }
        };

        match step {
            AuthStep::Continue(next) => authorization = Some(next),
            AuthStep::TryBasic => use_basic = true,
            AuthStep::Complete => {
                failure.error = Error::new(
                    "rdg out auth complete without upgrade or ok",
                    GwErrorKind::HttpStatus(http::StatusCode::UNAUTHORIZED.as_u16()),
                );
                return Err(failure);
            }
        }
    }

    failure.error = Error::new("rdg out auth rounds exceeded", GwErrorKind::Connect);
    Err(failure)
}

/// Whether an error chain carries an `io::ErrorKind::ConnectionReset`, which rustls
/// reports when the peer closes the connection during a TLS renegotiation.
fn error_chain_has_connection_reset(error: &(dyn core::error::Error + 'static)) -> bool {
    let mut source = error.source();
    while let Some(cause) = source {
        if let Some(io) = cause.downcast_ref::<std::io::Error>()
            && io.kind() == std::io::ErrorKind::ConnectionReset
        {
            return true;
        }
        source = cause.source();
    }
    false
}

/// Complete the OUT-channel response once it is not an auth challenge: upgrade to a
/// WebSocket on 101, or open the dual-channel HTTP transport on 200.
#[expect(clippy::too_many_arguments, reason = "carries the established OUT connection state")]
async fn finish_rdg_out_response(
    resp: http::Response<Incoming>,
    target: &GwConnectTarget,
    connect_addr: &str,
    gw_host: &str,
    connection_id: &str,
    spn: &str,
    certificate_validation: CertificateValidation,
    certificate_validation_callback: Option<CertificateValidationCallback>,
    mut cookies: GatewayCookies,
    out_sender: SendRequest<RdgBody>,
    out_stop_tx: oneshot::Sender<()>,
    out_conn_jh: tokio::task::JoinHandle<
        Option<hyper::client::conn::http1::Parts<hyper_util::rt::tokio::TokioIo<TlsStream<TcpStream>>>>,
    >,
    client_addr: core::net::SocketAddr,
    http_auth_basic: bool,
) -> Result<(PacketIo, core::net::SocketAddr), Error> {
    if resp.status() == http::StatusCode::SWITCHING_PROTOCOLS {
        let _ = out_stop_tx.send(());
        let parts = out_conn_jh
            .await
            .map_err(|e| custom_err!("websocket upgrade join", e))?
            .ok_or_else(|| Error::new("websocket upgrade connection lost", GwErrorKind::Connect))?;
        let tls_stream = parts.io.into_inner();
        let ws_stream = WebSocketStream::from_raw_socket(tls_stream, Role::Client, None).await;
        let (sink, stream) = ws_stream.split();
        return Ok((
            PacketIo::WebSocket {
                sink,
                stream,
                http_auth_basic,
            },
            client_addr,
        ));
    }

    info!("RD Gateway WebSocket upgrade unavailable; using dual-channel HTTP");
    // Keep the OUT HTTP driver alive while the response body is read.
    // Keeping the stop sender in PacketIo delays its shutdown until the transport is dropped.
    let out_conn_task = tokio::task::spawn(async move {
        let _ = out_conn_jh.await;
    });

    let mut out_body = resp.into_body();
    let leftover = skip_seed_payload(&mut out_body).await?;
    drop(out_sender);

    let dual = open_in_channel(
        connect_addr,
        gw_host,
        connection_id,
        target,
        spn,
        certificate_validation,
        certificate_validation_callback,
        &mut cookies,
        out_stop_tx,
        out_body,
        leftover,
        out_conn_task,
        http_auth_basic,
    )
    .await?;
    Ok((dual, client_addr))
}

#[expect(
    clippy::too_many_arguments,
    reason = "a dual HTTP connection requires independent IN and OUT connection state"
)]
async fn open_in_channel(
    connect_addr: &str,
    gw_host: &str,
    connection_id: &str,
    target: &GwConnectTarget,
    spn: &str,
    certificate_validation: CertificateValidation,
    certificate_validation_callback: Option<CertificateValidationCallback>,
    cookies: &mut GatewayCookies,
    out_stop_tx: oneshot::Sender<()>,
    out_body: Incoming,
    out_buf: Vec<u8>,
    out_conn_task: tokio::task::JoinHandle<()>,
    http_auth_basic: bool,
) -> Result<PacketIo, Error> {
    let (in_stream, _) = tls_connect(
        connect_addr,
        gw_host,
        certificate_validation,
        certificate_validation_callback,
    )
    .await?;
    let in_io = hyper_util::rt::tokio::TokioIo::new(in_stream);
    let (mut in_sender, in_conn) = hyper::client::conn::http1::handshake(in_io)
        .await
        .map_err(|e| custom_err!("http/1 in handshake", e))?;

    let in_conn_task = tokio::task::spawn(async move {
        if let Err(e) = in_conn.await {
            error!("RD Gateway IN HTTP connection error: {e:?}");
        }
    });

    let request_context = RdgRequestContext {
        method: "RDG_IN_DATA",
        gw_host,
        connection_id,
        target,
        websocket_upgrade: false,
    };
    let in_resp = authenticated_rdg_request(&mut in_sender, &request_context, spn, cookies).await?;

    if in_resp.status() != http::StatusCode::OK {
        return Err(Error::new(
            "rdg in data",
            GwErrorKind::HttpStatus(in_resp.status().as_u16()),
        ));
    }
    let _ = in_resp
        .into_body()
        .collect()
        .await
        .map_err(|e| custom_err!("drain rdg in body", e))?;

    let (in_tx, in_body) = BodyChannel::<Bytes>::new(16);
    let host_header = host_header_value(gw_host);
    let mut write_req = http::Request::builder()
        .method("RDG_IN_DATA")
        .uri("/remoteDesktopGateway/")
        .header(hyper::header::HOST, host_header.as_str())
        .header("Rdg-Connection-Id", connection_id);
    if let Some(cookie_header) = cookies.header_value() {
        write_req = write_req.header(hyper::header::COOKIE, cookie_header);
    }
    let write_req = write_req
        .body(in_body.boxed())
        .map_err(|e| custom_err!("build rdg in write request", e))?;

    tokio::task::spawn(async move {
        match in_sender.send_request(write_req).await {
            Ok(resp) => {
                debug!("RD Gateway IN write-channel HTTP status {}", resp.status().as_u16());
                let _ = resp.into_body().collect().await;
            }
            Err(e) => error!("RD Gateway IN write-channel error: {e:?}"),
        }
    });

    Ok(PacketIo::DualHttp {
        out_body,
        out_buf,
        in_tx,
        http_auth_basic,
        _out_stop_tx: out_stop_tx,
        _out_conn: out_conn_task,
        _in_conn: in_conn_task,
    })
}

async fn authenticated_rdg_request(
    sender: &mut SendRequest<RdgBody>,
    request_context: &RdgRequestContext<'_>,
    spn: &str,
    cookies: &mut GatewayCookies,
) -> Result<http::Response<Incoming>, Error> {
    authenticated_http_request(
        sender,
        &request_context.target.gw_user,
        &request_context.target.gw_pass,
        request_context.target.smart_card.as_deref(),
        spn,
        cookies,
        |authorization, use_basic, cookies| {
            build_rdg_request(request_context, authorization, use_basic, cookies, empty_rdg_body())
        },
    )
    .await
}

/// Sends a retry-safe request through the shared gateway HTTP authentication loop.
///
/// The closure can build either modern RDG or future RPCH requests, while the
/// TLS, proxy, cookie, and NTLM/Negotiate authentication machinery remains
/// shared. It is suitable only for request bodies that can be retried safely.
async fn authenticated_http_request<F>(
    sender: &mut SendRequest<RdgBody>,
    username: &str,
    password: &str,
    smart_card: Option<&crate::GwSmartCardCredentials>,
    spn: &str,
    cookies: &mut GatewayCookies,
    mut build_request: F,
) -> Result<http::Response<Incoming>, Error>
where
    F: FnMut(Option<&str>, bool, &GatewayCookies) -> Result<http::Request<RdgBody>, Error>,
{
    let mut http_auth: Option<GatewayHttpAuth> = None;
    let mut authorization: Option<String> = None;
    let mut use_basic = false;
    const MAX_AUTH_ROUNDS: usize = 8;

    for _ in 0..MAX_AUTH_ROUNDS {
        let req = build_request(authorization.as_deref(), use_basic, cookies)?;

        let resp = sender
            .send_request(req)
            .await
            .map_err(|e| custom_err!("gateway http auth send", e))?;
        cookies.capture(resp.headers());

        if resp.status() != http::StatusCode::UNAUTHORIZED {
            return Ok(resp);
        }

        if use_basic {
            return Err(Error::new(
                "gateway http basic auth",
                GwErrorKind::HttpStatus(resp.status().as_u16()),
            ));
        }

        let challenges: Vec<String> = www_authenticate_values(resp.headers())
            .into_iter()
            .map(str::to_owned)
            .collect();
        let _ = resp
            .into_body()
            .collect()
            .await
            .map_err(|e| custom_err!("drain gateway http auth body", e))?;

        let challenge_refs: Vec<&str> = challenges.iter().map(String::as_str).collect();
        let step = if let Some(auth) = http_auth.as_mut() {
            auth.step_www_authenticate(challenge_refs)?
        } else {
            let (auth, step) = GatewayHttpAuth::from_challenges(
                username,
                password,
                smart_card,
                Some(spn.to_owned()),
                &challenge_refs,
            )?;
            http_auth = Some(auth);
            step
        };

        match step {
            AuthStep::Continue(next) => authorization = Some(next),
            AuthStep::TryBasic => use_basic = true,
            AuthStep::Complete => {
                return Err(Error::new(
                    "gateway http auth complete without success",
                    GwErrorKind::HttpStatus(http::StatusCode::UNAUTHORIZED.as_u16()),
                ));
            }
        }
    }

    Err(Error::new("gateway http auth rounds exceeded", GwErrorKind::Connect))
}

fn build_rdg_request(
    request_context: &RdgRequestContext<'_>,
    authorization: Option<&str>,
    use_basic: bool,
    cookies: &GatewayCookies,
    body: RdgBody,
) -> Result<http::Request<RdgBody>, Error> {
    let host_header = host_header_value(request_context.gw_host);
    let mut req = http::Request::builder()
        .method(request_context.method)
        .uri("/remoteDesktopGateway/")
        .header(hyper::header::HOST, host_header.as_str())
        .header("Rdg-Connection-Id", request_context.connection_id);
    if let Some(cookie_header) = cookies.header_value() {
        req = req.header(hyper::header::COOKIE, cookie_header);
    }

    if request_context.websocket_upgrade {
        req = req
            .header(hyper::header::CONNECTION, "Upgrade")
            .header(hyper::header::UPGRADE, "websocket")
            .header(hyper::header::SEC_WEBSOCKET_VERSION, "13")
            .header(hyper::header::SEC_WEBSOCKET_KEY, generate_key());
    }

    if use_basic {
        req = req.header(
            hyper::header::AUTHORIZATION,
            basic_authorization(&request_context.target.gw_user, &request_context.target.gw_pass),
        );
    } else if let Some(auth_header) = authorization {
        req = req.header(hyper::header::AUTHORIZATION, auth_header);
    }

    req.body(body).map_err(|e| custom_err!("build rdg request", e))
}

fn host_header_value(gw_host: &str) -> String {
    if gw_host.contains(':') {
        format!("[{gw_host}]")
    } else {
        gw_host.to_owned()
    }
}

/// Open the legacy MS-RPCH v2 transport: two TLS streams carrying RPC_IN_DATA and
/// RPC_OUT_DATA, driven through the RTS/DCE/RPC setup in [`crate::rpch`].
///
/// Used for gateways that do not support the WebSocket upgrade or dual-channel HTTP.
pub(crate) async fn open_rpch_transport(
    target: &GwConnectTarget,
    client_name: &str,
    certificate_validation: CertificateValidation,
    certificate_validation_callback: Option<CertificateValidationCallback>,
) -> Result<(crate::rpch::RpchStream, core::net::SocketAddr), Error> {
    let (gw_host, connect_addr) = parse_gateway_endpoint(&target.gw_endpoint)?;

    // RPC-over-HTTP uses separate IN and OUT connections ([MS-RPCH] 2.1.1).
    let (out_stream, client_addr) = tls_connect(
        &connect_addr,
        gw_host,
        certificate_validation,
        certificate_validation_callback.clone(),
    )
    .await?;
    let (in_stream, _) = tls_connect(
        &connect_addr,
        gw_host,
        certificate_validation,
        certificate_validation_callback,
    )
    .await?;

    // RPC-level NTLM packet integrity when credentials are present.
    let rpc_auth = if target.gw_pass.is_empty() {
        None
    } else {
        Some(crate::rpc::RpcNtlmAuth::new(&target.gw_user, &target.gw_pass)?)
    };

    // Match the reference client (mstsc): a 1 GB channel lifetime (the IN channel
    // Content-Length), a 64 KB receive window, and a 300 s keepalive ([MS-RPCH] 2.2.3.5).
    let settings = crate::rpc::RpcHttpV2Settings::new(64 * 1024, 1_073_741_824, 300_000)
        .map_err(|e| custom_err!("rpch settings", e))?;
    let mut session = Box::pin(crate::rpch::rpch_connect(
        out_stream, in_stream, gw_host, target, settings, rpc_auth,
    ))
    .await?;
    // Establish the TsProxy tunnel and channel to the target resource before handing the
    // stream to the caller; without this the session cannot carry target-server data.
    Box::pin(session.open_tunnel(client_name, &target.server, target.server_port)).await?;
    Ok((crate::rpch::RpchStream::new(session), client_addr))
}

async fn tls_connect(
    connect_addr: &str,
    gw_host: &str,
    certificate_validation: CertificateValidation,
    certificate_validation_callback: Option<CertificateValidationCallback>,
) -> Result<(TlsStream<TcpStream>, core::net::SocketAddr), Error> {
    let (stream, client_addr) = tcp_connect(connect_addr, gw_host).await?;

    let tls_upgrade = if let Some(callback) = certificate_validation_callback {
        ironrdp_tls::upgrade_with_certificate_validation_callback(stream, gw_host, callback).await
    } else {
        ironrdp_tls::upgrade_with_certificate_validation(stream, gw_host, certificate_validation).await
    };
    let (stream, _) = tls_upgrade.map_err(|e| custom_err!("tls connect", e))?;
    Ok((stream, client_addr))
}

/// Skip the reverse-proxy seed payload on OUT ([MS-TSGU] 3.3.5.1 / FreeRDP ~10 bytes).
///
/// Returns any bytes past the seed that already arrived in the same body frame.
async fn skip_seed_payload(body: &mut Incoming) -> Result<Vec<u8>, Error> {
    const SEED_LEN: usize = 10;
    let mut skipped = 0usize;

    while skipped < SEED_LEN {
        let frame = match body.frame().await {
            None => {
                debug!("RD Gateway OUT body ended during seed skip ({skipped} bytes)");
                return Ok(Vec::new());
            }
            Some(Err(e)) => return Err(custom_err!("dual-http seed read", e)),
            Some(Ok(frame)) => frame,
        };
        let Ok(data) = frame.into_data() else {
            continue;
        };
        if data.is_empty() {
            continue;
        }
        let need = SEED_LEN - skipped;
        if data.len() <= need {
            skipped += data.len();
        } else {
            return Ok(data[need..].to_vec());
        }
    }

    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_header_brackets_ipv6() {
        assert_eq!(host_header_value("2001:db8::1"), "[2001:db8::1]");
        assert_eq!(host_header_value("rdg.example"), "rdg.example");
    }

    #[test]
    fn gateway_cookies_replace_and_replay_values() {
        let mut cookies = GatewayCookies::default();
        let mut headers = http::HeaderMap::new();
        headers.append(
            hyper::header::SET_COOKIE,
            "ARRAffinity=first; Path=/".parse().expect("header"),
        );
        headers.append(hyper::header::SET_COOKIE, "route=west; Secure".parse().expect("header"));
        cookies.capture(&headers);
        assert_eq!(cookies.header_value().as_deref(), Some("ARRAffinity=first; route=west"));

        let mut replacement = http::HeaderMap::new();
        replacement.append(
            hyper::header::SET_COOKIE,
            "ARRAffinity=second; Path=/".parse().expect("header"),
        );
        cookies.capture(&replacement);
        assert_eq!(
            cookies.header_value().as_deref(),
            Some("ARRAffinity=second; route=west")
        );
    }

    #[test]
    fn proxy_url_builds_basic_authorization() {
        let GatewayProxy::HttpConnect {
            authority,
            authorization,
        } = proxy_from_url("http://proxy-user:proxy-pass@proxy.example:8080").expect("proxy")
        else {
            panic!("expected HTTP CONNECT proxy");
        };
        assert_eq!(authority, "proxy.example:8080");
        assert_eq!(authorization.as_deref(), Some("Basic cHJveHktdXNlcjpwcm94eS1wYXNz"));
    }

    #[test]
    fn socks_proxy_url_preserves_credentials() {
        let GatewayProxy::Socks5 { authority, credentials } =
            proxy_from_url("socks5h://proxy-user:proxy-pass@proxy.example:1080").expect("proxy")
        else {
            panic!("expected SOCKS5 proxy");
        };
        assert_eq!(authority, "proxy.example:1080");
        assert_eq!(credentials, Some(("proxy-user".to_owned(), "proxy-pass".to_owned())));
    }

    #[test]
    fn no_proxy_matches_exact_hosts_and_suffixes() {
        assert!(no_proxy_matches("rdg.example.test", "example.test"));
        assert!(no_proxy_matches("rdg.example.test", ".example.test"));
        assert!(no_proxy_matches("rdg.example.test", "rdg.example.test"));
        assert!(!no_proxy_matches("not-example.test", "example.test"));
    }
}
