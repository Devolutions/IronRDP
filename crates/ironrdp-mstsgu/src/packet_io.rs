//! Packet I/O over the MS-TSGU HTTPS WebSocket or dual HTTP transport.

use core::convert::Infallible;
use core::fmt;
use core::net::IpAddr;
use core::pin::Pin;
use core::task::{Context, Poll};
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
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio_socks::tcp::Socks5Stream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::{Message, http};

use crate::http_auth::{AuthStep, GatewayHttpAuth, basic_authorization, split_auth_challenge, www_authenticate_values};
use crate::proto::PktHdr;
use crate::{Error, GwConnectTarget, GwErrorKind, GwSessionAuthentication};

pub(crate) trait GatewayIo: AsyncRead + AsyncWrite + Unpin {}

impl<T: AsyncRead + AsyncWrite + Unpin + ?Sized> GatewayIo for T {}

type GatewayStream = Box<dyn GatewayIo + Send>;
type RdgBody = BoxBody<Bytes, Infallible>;

const MAX_AUTH_RESPONSE_SIZE: usize = 16 * 1024;
const MAX_PACKET_SIZE: usize = 16 * 1024 * 1024;
const MAX_PROXY_RESPONSE_SIZE: usize = 16 * 1024;
const OUT_SEED_SIZE: usize = 10;
const PACKET_HEADER_SIZE: usize = 2 /* type */ + 2 /* reserved */ + 4 /* length */;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProxyScheme {
    Http,
    Https,
    Socks5,
    Socks5h,
}

impl ProxyScheme {
    fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
            Self::Socks5 | Self::Socks5h => 1080,
        }
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
            Self::Socks5 => "socks5",
            Self::Socks5h => "socks5h",
        }
    }
}

#[derive(Clone)]
pub(crate) struct ProxyCredentials {
    pub(crate) username: String,
    pub(crate) password: String,
}

#[derive(Clone)]
pub(crate) struct ProxyConfig {
    pub(crate) scheme: ProxyScheme,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) credentials: Option<ProxyCredentials>,
}

impl fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyConfig")
            .field("scheme", &self.scheme)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("credentials", &self.credentials.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

struct GatewayEndpoint {
    host: String,
    port: u16,
    endpoint: String,
}

struct BufferedGatewayStream {
    inner: GatewayStream,
    buffered: Vec<u8>,
}

impl AsyncRead for BufferedGatewayStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        if !self.buffered.is_empty() && buf.remaining() != 0 {
            let count = self.buffered.len().min(buf.remaining());
            buf.put_slice(&self.buffered[..count]);
            self.buffered.drain(..count);
            return Poll::Ready(Ok(()));
        }

        Pin::new(&mut *self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for BufferedGatewayStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut *self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.inner).poll_shutdown(cx)
    }
}

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

pub(crate) struct GatewayTransport {
    pub(crate) io: PacketIo,
    pub(crate) session_authentication: GwSessionAuthentication,
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
) -> Result<(GatewayTransport, core::net::SocketAddr), Error> {
    let gateway = parse_gateway_endpoint(&target.gw_endpoint)?;
    let gw_host = gateway.host.clone();
    let proxy = proxy_from_environment(&gateway)?;

    let (stream, client_addr) = open_gateway_tcp(&gateway, proxy.as_ref()).await?;
    let stream = upgrade_gateway_tls(
        stream,
        &gw_host,
        &target.gw_endpoint,
        certificate_validation,
        certificate_validation_callback.clone(),
    )
    .await?;

    let in_gw_host = gw_host.clone();
    let in_gateway = GatewayEndpoint {
        host: gateway.host,
        port: gateway.port,
        endpoint: gateway.endpoint,
    };
    open_transport_with_out_stream(stream, client_addr, &gw_host, target, move || async move {
        let (stream, _) = open_gateway_tcp(&in_gateway, proxy.as_ref()).await?;
        upgrade_gateway_tls(
            stream,
            &in_gw_host,
            &target.gw_endpoint,
            certificate_validation,
            certificate_validation_callback,
        )
        .await
    })
    .await
}

async fn upgrade_gateway_tls(
    stream: GatewayStream,
    gw_host: &str,
    gw_endpoint: &str,
    certificate_validation: CertificateValidation,
    certificate_validation_callback: Option<CertificateValidationCallback>,
) -> Result<GatewayStream, Error> {
    let (stream, _) = match select_gateway_tls_upgrade(certificate_validation, certificate_validation_callback) {
        GatewayTlsUpgrade::CertificateValidation(certificate_validation) => {
            ironrdp_tls::upgrade_with_certificate_validation(stream, gw_host, certificate_validation).await
        }
        GatewayTlsUpgrade::CertificateValidationCallback(certificate_validation_callback) => {
            ironrdp_tls::upgrade_with_certificate_validation_callback_for_endpoint(
                stream,
                gw_host,
                gw_endpoint,
                certificate_validation_callback,
            )
            .await
        }
    }
    .map_err(|e| custom_err!("tls connect", e))?;

    Ok(Box::new(stream))
}

fn parse_gateway_endpoint(endpoint: &str) -> Result<GatewayEndpoint, Error> {
    let (host, port) = endpoint
        .rsplit_once(':')
        .ok_or_else(|| Error::new("connect", GwErrorKind::InvalidGwTarget))?;
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    if host.is_empty()
        || host
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(Error::new("connect", GwErrorKind::InvalidGwTarget));
    }

    let port = port
        .parse()
        .map_err(|_| Error::new("connect", GwErrorKind::InvalidGwTarget))?;

    Ok(GatewayEndpoint {
        host: host.to_owned(),
        port,
        endpoint: endpoint.to_owned(),
    })
}

#[cfg(feature = "test-support")]
pub(crate) fn gateway_endpoint_is_valid(endpoint: &str) -> bool {
    parse_gateway_endpoint(endpoint).is_ok()
}

async fn open_gateway_tcp(
    gateway: &GatewayEndpoint,
    proxy: Option<&ProxyConfig>,
) -> Result<(GatewayStream, core::net::SocketAddr), Error> {
    match proxy {
        Some(proxy) => open_proxy_tunnel(proxy, gateway).await,
        None => {
            let stream = TcpStream::connect(&gateway.endpoint)
                .await
                .map_err(|e| custom_err!("tcp connect", e))?;
            let client_addr = stream
                .local_addr()
                .map_err(|e| custom_err!("get socket local address", e))?;
            Ok((Box::new(stream), client_addr))
        }
    }
}

async fn open_proxy_tunnel(
    proxy: &ProxyConfig,
    gateway: &GatewayEndpoint,
) -> Result<(GatewayStream, core::net::SocketAddr), Error> {
    let stream = TcpStream::connect((proxy.host.as_str(), proxy.port))
        .await
        .map_err(|e| custom_err!("proxy tcp connect", e))?;
    let client_addr = stream
        .local_addr()
        .map_err(|e| custom_err!("get socket local address", e))?;

    let stream = match proxy.scheme {
        ProxyScheme::Http => connect_http_proxy(stream, proxy, gateway).await?,
        ProxyScheme::Https => {
            let (stream, _) = ironrdp_tls::upgrade(stream, &proxy.host)
                .await
                .map_err(|e| custom_err!("proxy tls connect", e))?;
            connect_http_proxy(Box::new(stream), proxy, gateway).await?
        }
        ProxyScheme::Socks5 | ProxyScheme::Socks5h => connect_socks_proxy(stream, proxy, gateway).await?,
    };

    Ok((stream, client_addr))
}

async fn connect_http_proxy(
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + 'static,
    proxy: &ProxyConfig,
    gateway: &GatewayEndpoint,
) -> Result<GatewayStream, Error> {
    let mut stream: GatewayStream = Box::new(stream);
    let authority = gateway_authority(gateway);
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(credentials) = &proxy.credentials {
        request.push_str("Proxy-Authorization: ");
        request.push_str(&basic_authorization(&credentials.username, &credentials.password));
        request.push_str("\r\n");
    }
    request.push_str("\r\n");

    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| custom_err!("proxy connect request", e))?;
    stream
        .flush()
        .await
        .map_err(|e| custom_err!("proxy connect request", e))?;
    read_http_connect_response(stream).await
}

async fn connect_socks_proxy(
    stream: TcpStream,
    proxy: &ProxyConfig,
    gateway: &GatewayEndpoint,
) -> Result<GatewayStream, Error> {
    let stream = match (proxy.scheme, &proxy.credentials) {
        (ProxyScheme::Socks5, Some(credentials)) => {
            let target = resolve_socks_target(gateway).await?;
            Socks5Stream::connect_with_password_and_socket(stream, target, &credentials.username, &credentials.password)
                .await
                .map_err(|e| custom_err!("socks proxy connect", e))?
        }
        (ProxyScheme::Socks5, None) => {
            let target = resolve_socks_target(gateway).await?;
            Socks5Stream::connect_with_socket(stream, target)
                .await
                .map_err(|e| custom_err!("socks proxy connect", e))?
        }
        (ProxyScheme::Socks5h, Some(credentials)) => {
            // `socks5h` passes the gateway hostname to the proxy for DNS resolution.
            Socks5Stream::connect_with_password_and_socket(
                stream,
                (gateway.host.as_str(), gateway.port),
                &credentials.username,
                &credentials.password,
            )
            .await
            .map_err(|e| custom_err!("socks proxy connect", e))?
        }
        (ProxyScheme::Socks5h, None) => {
            Socks5Stream::connect_with_socket(stream, (gateway.host.as_str(), gateway.port))
                .await
                .map_err(|e| custom_err!("socks proxy connect", e))?
        }
        (ProxyScheme::Http | ProxyScheme::Https, _) => {
            return Err(Error::new("invalid proxy URL", GwErrorKind::Connect));
        }
    };

    Ok(Box::new(stream.into_inner()))
}

async fn resolve_socks_target(gateway: &GatewayEndpoint) -> Result<core::net::SocketAddr, Error> {
    // `socks5` resolves the gateway locally before making the SOCKS request.
    tokio::net::lookup_host((gateway.host.as_str(), gateway.port))
        .await
        .map_err(|e| custom_err!("resolve gateway for socks proxy", e))?
        .next()
        .ok_or_else(|| Error::new("resolve gateway for socks proxy", GwErrorKind::Connect))
}

fn gateway_authority(gateway: &GatewayEndpoint) -> String {
    if gateway.host.contains(':') {
        format!("[{}]:{}", gateway.host, gateway.port)
    } else {
        format!("{}:{}", gateway.host, gateway.port)
    }
}

pub(crate) async fn read_http_connect_response(mut stream: GatewayStream) -> Result<GatewayStream, Error> {
    let mut response = Vec::new();
    let header_end = loop {
        let mut chunk = [0u8; 1024];
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|e| custom_err!("proxy connect response", e))?;
        if read == 0 {
            return Err(Error::new("proxy connect response truncated", GwErrorKind::Connect));
        }
        response.extend_from_slice(&chunk[..read]);

        if let Some(header_end) = find_header_end(&response) {
            if header_end > MAX_PROXY_RESPONSE_SIZE {
                return Err(Error::new("proxy connect response too large", GwErrorKind::Connect));
            }
            break header_end;
        }
        if MAX_PROXY_RESPONSE_SIZE < response.len() {
            return Err(Error::new("proxy connect response too large", GwErrorKind::Connect));
        }
    };

    validate_http_connect_response(&response[..header_end])?;
    if header_end == response.len() {
        Ok(stream)
    } else {
        Ok(Box::new(BufferedGatewayStream {
            inner: stream,
            buffered: response[header_end..].to_vec(),
        }))
    }
}

fn find_header_end(response: &[u8]) -> Option<usize> {
    response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

fn validate_http_connect_response(response: &[u8]) -> Result<(), Error> {
    let status_line = response
        .split(|byte| *byte == b'\n')
        .next()
        .and_then(|line| line.strip_suffix(b"\r"))
        .ok_or_else(|| Error::new("invalid proxy connect response", GwErrorKind::Connect))?;
    let mut parts = status_line
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|part| !part.is_empty());
    let version = parts
        .next()
        .ok_or_else(|| Error::new("invalid proxy connect response", GwErrorKind::Connect))?;
    let status = parts
        .next()
        .and_then(|status| core::str::from_utf8(status).ok())
        .and_then(|status| status.parse::<u16>().ok())
        .ok_or_else(|| Error::new("invalid proxy connect response", GwErrorKind::Connect))?;
    if !(version == b"HTTP/1.0" || version == b"HTTP/1.1") {
        return Err(Error::new("invalid proxy connect response", GwErrorKind::Connect));
    }
    if !(200..300).contains(&status) {
        return Err(Error::new("proxy connect", GwErrorKind::HttpStatus(status)));
    }
    Ok(())
}

fn proxy_from_environment(gateway: &GatewayEndpoint) -> Result<Option<ProxyConfig>, Error> {
    proxy_from_values(
        &gateway.host,
        gateway.port,
        environment_value("HTTPS_PROXY", "https_proxy")?,
        environment_value("NO_PROXY", "no_proxy")?,
    )
}

fn environment_value(uppercase: &str, lowercase: &str) -> Result<Option<String>, Error> {
    match std::env::var(uppercase) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => match std::env::var(lowercase) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => {
                Err(Error::new("proxy environment is not unicode", GwErrorKind::Connect))
            }
        },
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(Error::new("proxy environment is not unicode", GwErrorKind::Connect))
        }
    }
}

pub(crate) fn proxy_from_values(
    gateway_host: &str,
    gateway_port: u16,
    proxy: Option<String>,
    no_proxy: Option<String>,
) -> Result<Option<ProxyConfig>, Error> {
    let Some(proxy) = proxy else {
        return Ok(None);
    };
    if no_proxy
        .as_deref()
        .is_some_and(|no_proxy| no_proxy_matches(gateway_host, gateway_port, no_proxy))
    {
        return Ok(None);
    }
    parse_proxy_url(&proxy).map(Some)
}

pub(crate) fn parse_proxy_url(value: &str) -> Result<ProxyConfig, Error> {
    let uri = value
        .parse::<http::Uri>()
        .map_err(|_| Error::new("invalid proxy URL", GwErrorKind::Connect))?;
    let scheme = match uri.scheme_str() {
        Some(scheme) if scheme.eq_ignore_ascii_case("http") => ProxyScheme::Http,
        Some(scheme) if scheme.eq_ignore_ascii_case("https") => ProxyScheme::Https,
        Some(scheme) if scheme.eq_ignore_ascii_case("socks5") => ProxyScheme::Socks5,
        Some(scheme) if scheme.eq_ignore_ascii_case("socks5h") => ProxyScheme::Socks5h,
        _ => return Err(Error::new("unsupported proxy URL scheme", GwErrorKind::Connect)),
    };
    let authority = uri
        .authority()
        .ok_or_else(|| Error::new("invalid proxy URL", GwErrorKind::Connect))?;
    if uri.path() != "/" || uri.query().is_some() {
        return Err(Error::new("invalid proxy URL", GwErrorKind::Connect));
    }
    let credentials = proxy_credentials(authority.as_str())?;
    let host = authority.host();
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    if host.is_empty() {
        return Err(Error::new("invalid proxy URL", GwErrorKind::Connect));
    }

    let port = proxy_port(authority.as_str(), scheme.default_port())?;

    Ok(ProxyConfig {
        scheme,
        host: host.to_owned(),
        port,
        credentials,
    })
}

fn proxy_port(authority: &str, default_port: u16) -> Result<u16, Error> {
    let authority = authority.rsplit_once('@').map_or(authority, |(_, authority)| authority);
    let port = if let Some(authority) = authority.strip_prefix('[') {
        authority
            .split_once(']')
            .and_then(|(_, suffix)| suffix.strip_prefix(':'))
    } else {
        authority
            .rsplit_once(':')
            .filter(|(host, _)| !host.contains(':'))
            .map(|(_, port)| port)
    };
    port.map_or(Ok(default_port), |port| {
        port.parse()
            .map_err(|_| Error::new("invalid proxy URL", GwErrorKind::Connect))
    })
}

fn proxy_credentials(authority: &str) -> Result<Option<ProxyCredentials>, Error> {
    let Some((userinfo, host)) = authority.rsplit_once('@') else {
        return Ok(None);
    };
    if userinfo.contains('@') || host.is_empty() {
        return Err(Error::new("invalid proxy URL", GwErrorKind::Connect));
    }
    let (username, password) = userinfo
        .split_once(':')
        .ok_or_else(|| Error::new("invalid proxy URL", GwErrorKind::Connect))?;
    let username = percent_decode(username)?;
    let password = percent_decode(password)?;
    if username.is_empty() || password.is_empty() {
        return Err(Error::new("invalid proxy URL", GwErrorKind::Connect));
    }

    Ok(Some(ProxyCredentials { username, password }))
}

fn percent_decode(value: &str) -> Result<String, Error> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut input = value.as_bytes().iter().copied();
    while let Some(byte) = input.next() {
        if byte != b'%' {
            bytes.push(byte);
            continue;
        }
        let high = input
            .next()
            .and_then(hex_value)
            .ok_or_else(|| Error::new("invalid proxy URL", GwErrorKind::Connect))?;
        let low = input
            .next()
            .and_then(hex_value)
            .ok_or_else(|| Error::new("invalid proxy URL", GwErrorKind::Connect))?;
        bytes.push((high << 4) | low);
    }
    String::from_utf8(bytes).map_err(|_| Error::new("invalid proxy URL", GwErrorKind::Connect))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn no_proxy_matches(host: &str, port: u16, no_proxy: &str) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    let is_ip = host.parse::<IpAddr>().is_ok();
    no_proxy.split(',').map(str::trim).any(|entry| {
        if entry == "*" {
            return true;
        }
        if entry.is_empty() {
            return false;
        }
        let (entry, entry_port) = split_no_proxy_port(entry);
        if entry_port.is_some_and(|entry_port| entry_port != port) {
            return false;
        }
        if is_ip {
            return entry.eq_ignore_ascii_case(&host);
        }

        let entry = entry.trim_end_matches('.').to_ascii_lowercase();
        let host = host.trim_end_matches('.');
        if entry.starts_with('.') {
            return host.ends_with(&entry) && entry.len() < host.len();
        }
        host == entry || host.strip_suffix(&entry).is_some_and(|prefix| prefix.ends_with('.'))
    })
}

fn split_no_proxy_port(entry: &str) -> (&str, Option<u16>) {
    if let Some(entry) = entry.strip_prefix('[') {
        if let Some((host, port)) = entry.split_once("]:") {
            return (host, port.parse().ok());
        }
    }
    if entry.parse::<IpAddr>().is_ok() {
        return (entry, None);
    }
    let Some((host, port)) = entry.rsplit_once(':') else {
        return (entry, None);
    };
    if host.contains(':') {
        return (entry, None);
    }
    match port.parse() {
        Ok(port) => (host, Some(port)),
        Err(_) => (entry, None),
    }
}

enum GatewayTlsUpgrade {
    CertificateValidation(CertificateValidation),
    CertificateValidationCallback(CertificateValidationCallback),
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

async fn open_transport_with_out_stream<F, Fut>(
    out_stream: GatewayStream,
    client_addr: core::net::SocketAddr,
    gw_host: &str,
    target: &GwConnectTarget,
    open_in_stream: F,
) -> Result<(GatewayTransport, core::net::SocketAddr), Error>
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
    let authenticated = authenticated_rdg_request(&mut out_sender, &context, &spn, &mut cookies, None).await?;
    let session_authentication = authenticated.session_authentication;
    let response = authenticated.response;

    match response.status() {
        http::StatusCode::SWITCHING_PROTOCOLS => {
            let parts = out_conn_jh
                .await
                .map_err(|e| custom_err!("websocket upgrade join", e))?
                .ok_or_else(|| Error::new("websocket upgrade connection lost", GwErrorKind::Connect))?;
            let stream = WebSocketStream::from_raw_socket(parts.io.into_inner(), Role::Client, None).await;
            let (sink, stream) = stream.split();
            Ok((
                GatewayTransport {
                    io: PacketIo::WebSocket { sink, stream },
                    session_authentication,
                },
                client_addr,
            ))
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
                session_authentication,
            )
            .await?;
            Ok((
                GatewayTransport {
                    io,
                    session_authentication,
                },
                client_addr,
            ))
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
) -> Result<GatewayTransport, Error> {
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
    .map(|(transport, _)| transport)
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
    session_authentication: GwSessionAuthentication,
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
    let authenticated =
        authenticated_rdg_request(&mut in_sender, &context, spn, cookies, Some(session_authentication)).await?;
    let response = authenticated.response;
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
    requested_session_authentication: Option<GwSessionAuthentication>,
) -> Result<AuthenticatedRdgResponse, Error> {
    let mut http_auth: Option<GatewayHttpAuth> = None;
    let mut session_authentication = requested_session_authentication.unwrap_or_default();
    let mut authorization =
        (session_authentication == GwSessionAuthentication::NtlmSspi).then(|| "SSPI_NTLM".to_owned());
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
            return Ok(AuthenticatedRdgResponse {
                response,
                session_authentication,
            });
        }

        if requested_session_authentication == Some(GwSessionAuthentication::NtlmSspi) {
            let status = response.status();
            drain_response_body(response.into_body(), "drain rdg extended authentication body").await?;
            return Err(Error::new(
                "rdg extended authentication",
                GwErrorKind::HttpStatus(status.as_u16()),
            ));
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
        let step = if requested_session_authentication.is_none()
            && http_auth.is_none()
            && smart_card.is_none()
            && challenges
                .iter()
                .any(|challenge| split_auth_challenge(challenge, "SSPI_NTLM").is_some())
        {
            session_authentication = GwSessionAuthentication::NtlmSspi;
            AuthStep::Continue("SSPI_NTLM".to_owned())
        } else if let Some(mut auth) = http_auth.take() {
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

struct AuthenticatedRdgResponse {
    response: http::Response<Incoming>,
    session_authentication: GwSessionAuthentication,
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
