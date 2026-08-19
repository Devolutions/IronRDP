#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

#[macro_use]
mod macros;

mod http_auth;
#[cfg(test)]
mod mock;
#[cfg(test)]
mod mock_rpch;
mod packet_io;
mod proto;
#[expect(
    dead_code,
    reason = "the staged raw stubs and DCE/RPC codecs are consumed when the RPC-over-HTTP runtime is integrated"
)]
mod rpc;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "consumed when the RPC-over-HTTP fallback is validated and enabled"
    )
)]
mod rpch;
// Consumed only by tests until the RPC-over-HTTP runtime is integrated.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "consumed when the RPC-over-HTTP runtime is integrated")
)]
mod rpc_transport;
mod udp;

use core::fmt;
use core::fmt::Display;
use core::pin::Pin;
use core::task::Poll;
use core::time::Duration;
use std::io;

use futures_util::FutureExt as _;
use hyper::body::Bytes;
use ironrdp_core::{Decode as _, Encode, ReadCursor, WriteCursor};
use ironrdp_tls::{CertificateValidation, CertificateValidationCallback};
use log::{debug, info, warn};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::sync::PollSender;

use self::http_auth::NtlmHttpAuth;
use self::packet_io::{PacketIo, open_rpch_transport, open_transport_prefer_websocket};
pub use self::rpch::RpchStream;

use self::proto::{
    ChannelClosePkt, ChannelPkt, ChannelResp, DataPkt, ExtendedAuthPkt, HandshakeReqPkt, HandshakeRespPkt, HttpCapsTy,
    HttpExtendedAuth, KeepalivePkt, PktHdr, PktTy, ReauthMessagePkt, ServiceMessagePkt, TunnelAuthPkt,
    TunnelAuthRespPkt, TunnelReqPkt, TunnelRespPkt, gateway_code_label,
};

pub use self::udp::{
    AaSynData, AaSynDataResp, ConnectPkt, ConnectPktResp, GwUdpOffer, UdpCorrelationInfo, UdpPacketHeader, UdpPktType,
    encode_connect_request, fragment_connect_pkt,
};

/// Parameters for opening an MS-TSGU tunnel to a target RDP resource.
#[derive(Clone, Debug)]
pub struct GwConnectTarget {
    /// Gateway host:port (for example `rdg.contoso.com:443`).
    pub gw_endpoint: String,
    /// Gateway username used for HTTP NTLM/Negotiate or Basic authentication.
    pub gw_user: String,
    /// Gateway password used for HTTP NTLM/Negotiate or Basic authentication.
    pub gw_pass: String,

    /// Target resource hostname as presented to the gateway (HTTP_CHANNEL_PACKET resources).
    pub server: String,
    /// Target resource port as presented to the gateway (HTTP_CHANNEL_PACKET `port`).
    ///
    /// Common values are `3389` for ordinary RDP and `2179` for Hyper-V VMConnect.
    pub server_port: u16,

    /// Smart-card credentials for gateway authentication via Kerberos PKINIT.
    ///
    /// When set, the HTTP authentication layer uses the Negotiate scheme with smart-card
    /// credentials instead of the password. Requires the `smartcard` feature.
    pub smart_card: Option<Box<GwSmartCardCredentials>>,
}

/// Smart-card credentials for RD Gateway authentication ([MS-TSGU] 2.2.5.3.10 SMARTCARD).
///
/// The gateway is authenticated through the HTTP Negotiate scheme using Kerberos PKINIT
/// with the smart card identity, mirroring the FreeRDP `SmartcardLogon` gateway path.
#[derive(Clone)]
pub struct GwSmartCardCredentials {
    /// Smart card PIN code.
    pub pin: String,
    /// DER-encoded X.509 certificate identifying the user.
    pub certificate: Vec<u8>,
    /// PKCS#1 private key for emulated smart cards; `None` for system-provided cards.
    pub private_key: Option<Vec<u8>>,
    /// Smart card reader name (empty for emulated cards).
    pub reader_name: String,
    /// Optional smart card name.
    pub card_name: Option<String>,
    /// Optional key container name.
    pub container_name: Option<String>,
    /// Optional CSP name.
    pub csp_name: Option<String>,
}

impl fmt::Debug for GwSmartCardCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GwSmartCardCredentials")
            .field("pin", &"<redacted>")
            .field("certificate", &self.certificate)
            .field("private_key", &self.private_key.as_ref().map(|_| "<redacted>"))
            .field("reader_name", &self.reader_name)
            .field("card_name", &self.card_name)
            .field("container_name", &self.container_name)
            .field("csp_name", &self.csp_name)
            .finish()
    }
}

pub(crate) type Error = ironrdp_error::Error<GwErrorKind>;

/// Error kind for MS-TSGU client operations.
///
/// Stage-specific detail is carried in the error context string and, when available, in the
/// numeric payload of [`GwErrorKind::HttpStatus`] / [`GwErrorKind::GatewayCode`].
#[derive(Debug)]
#[non_exhaustive]
pub enum GwErrorKind {
    InvalidGwTarget,
    Connect,
    /// Unexpected HTTP status while establishing the gateway transport (WebSocket or dual HTTP).
    HttpStatus(u16),
    /// Non-success gateway protocol status or HRESULT from a handshake/tunnel/channel response.
    GatewayCode(u32),
    /// Packet type did not match the expected stage response.
    UnexpectedPacket,
    /// The user or a policy callback declined the connection (for example a consent prompt).
    AccessDenied,
    PacketEof,
    UnsupportedFeature,
    Custom,
    Encode,
    Decode,
}

pub(crate) trait GwErrorExt {
    fn custom<E>(context: &'static str, e: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static;
}

impl GwErrorExt for ironrdp_error::Error<GwErrorKind> {
    #[track_caller]
    fn custom<E>(context: &'static str, e: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static,
    {
        Self::new(context, GwErrorKind::Custom).with_source(e)
    }
}

impl Display for GwErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GwErrorKind::InvalidGwTarget => f.write_str("invalid gateway target"),
            GwErrorKind::Connect => f.write_str("connection error"),
            GwErrorKind::HttpStatus(status) => write!(f, "unexpected HTTP status {status}"),
            GwErrorKind::GatewayCode(code) => match gateway_code_label(*code) {
                Some(label) => write!(f, "gateway error code 0x{code:08x} ({label})"),
                None => write!(f, "gateway error code 0x{code:08x}"),
            },
            GwErrorKind::UnexpectedPacket => f.write_str("unexpected gateway packet type"),
            GwErrorKind::AccessDenied => f.write_str("access denied"),
            GwErrorKind::PacketEof => f.write_str("truncated gateway packet"),
            GwErrorKind::UnsupportedFeature => f.write_str("unsupported feature"),
            GwErrorKind::Custom => f.write_str("custom"),
            GwErrorKind::Encode => f.write_str("encode"),
            GwErrorKind::Decode => f.write_str("decode"),
        }
    }
}

impl core::error::Error for GwErrorKind {}

/// Device redirection policy flags sent by the gateway in the tunnel authorization
/// response ([MS-TSGU] 2.2.5.3.7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GwTunnelRedirFlags(pub u32);

impl GwTunnelRedirFlags {
    /// Device redirection is enabled for all devices.
    pub const ENABLE_ALL: u32 = 0x8000_0000;
    /// Device redirection is disabled for all devices.
    pub const DISABLE_ALL: u32 = 0x4000_0000;
    /// Drive redirection is disabled.
    pub const DISABLE_DRIVE: u32 = 0x1;
    /// Printer redirection is disabled.
    pub const DISABLE_PRINTER: u32 = 0x2;
    /// Port redirection is disabled.
    pub const DISABLE_PORT: u32 = 0x4;
    /// Clipboard redirection is disabled.
    pub const DISABLE_CLIPBOARD: u32 = 0x8;
    /// Plug and play device redirection is disabled.
    pub const DISABLE_PNP: u32 = 0x10;

    /// `true` when clipboard redirection is disabled by policy.
    pub fn clipboard_disabled(self) -> bool {
        self.disabled(Self::DISABLE_CLIPBOARD)
    }

    /// `true` when drive redirection is disabled by policy.
    pub fn drives_disabled(self) -> bool {
        self.disabled(Self::DISABLE_DRIVE)
    }

    /// `true` when all device redirection is disabled by policy.
    pub fn all_disabled(self) -> bool {
        self.0 & Self::DISABLE_ALL != 0
    }

    fn disabled(self, flag: u32) -> bool {
        self.all_disabled() || self.0 & Self::ENABLE_ALL == 0 && self.0 & flag != 0
    }
}

/// Policy parameters negotiated during tunnel authorization.
#[derive(Clone, Copy, Debug, Default)]
pub struct GwTunnelPolicy {
    /// Device redirection policy flags, when the gateway supplied them.
    pub redir_flags: Option<GwTunnelRedirFlags>,
    /// Idle timeout in minutes advertised by the gateway, when supplied.
    ///
    /// The gateway disconnects sessions idle beyond this limit; the value is
    /// informational for the client (for example to schedule UI warnings).
    pub idle_timeout_minutes: Option<u32>,
}

/// Called with the gateway consent message before the connection proceeds.
///
/// Return `true` to accept the consent terms and continue, `false` to abort.
pub type GwConsentCallback = std::sync::Arc<dyn Fn(&str) -> bool + Send + Sync>;

struct GwConn {
    client_name: String,
    target: GwConnectTarget,
    io: PacketIo,
    consent_callback: Option<GwConsentCallback>,
    policy: GwTunnelPolicy,
}

/// Options for [`GwClient::connect_with_options`].
#[derive(Default)]
pub struct GwConnectOptions {
    /// TLS certificate policy for the gateway HTTPS leg (same surface as direct RDP).
    pub certificate_validation: CertificateValidation,
    /// Optional callback to prompt/accept untrusted gateway certificates (Rustls only).
    pub certificate_validation_callback: Option<CertificateValidationCallback>,
    /// Optional gateway consent prompt; without it, consent text is logged and accepted.
    pub consent_callback: Option<GwConsentCallback>,
}

impl fmt::Debug for GwConnectOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GwConnectOptions")
            .field("certificate_validation", &self.certificate_validation)
            .field(
                "certificate_validation_callback",
                &self.certificate_validation_callback.as_ref().map(|_| "<callback>"),
            )
            .field(
                "consent_callback",
                &self.consent_callback.as_ref().map(|_| "<callback>"),
            )
            .finish()
    }
}

/// Opens a fresh gateway transport for a reauthentication secondary connection.
type TransportFactory =
    Box<dyn FnMut() -> Pin<Box<dyn Future<Output = Result<PacketIo, Error>> + Send + 'static>> + Send>;

/// Test-only connection path without a reauth transport factory.
#[cfg(test)]
fn unsupported_reauth() -> impl Future<Output = Result<PacketIo, Error>> {
    core::future::ready(Err(Error::new(
        "gateway reauthentication requires a transport factory",
        GwErrorKind::UnsupportedFeature,
    )))
}

/// Build the transport factory that reopens a network gateway transport for reauthentication.
fn network_transport_factory(
    target: GwConnectTarget,
    certificate_validation: CertificateValidation,
    certificate_validation_callback: Option<CertificateValidationCallback>,
) -> TransportFactory {
    Box::new(move || {
        let target = target.clone();
        let certificate_validation_callback = certificate_validation_callback.clone();
        Box::pin(async move {
            open_transport_prefer_websocket(&target, certificate_validation, certificate_validation_callback)
                .await
                .map(|(io, _)| io)
        })
    })
}

pub struct GwClient {
    work: tokio::task::JoinHandle<Result<(), Error>>,
    /// Set once the work task has completed; its `JoinHandle` must not be polled again
    /// (tokio panics if a completed `JoinHandle` is polled).
    work_done: bool,
    rx: tokio::sync::mpsc::Receiver<Bytes>,
    rx_bufs: Vec<Bytes>,
    tx: PollSender<Bytes>,
    policy: GwTunnelPolicy,
}

impl Drop for GwClient {
    fn drop(&mut self) {
        self.work.abort();
    }
}

/// Split `host`, `host:port`, `[ipv6]`, or `[ipv6]:port` into (host_for_sni, connect_addr).
///
/// When no port is present, `:443` is assumed (RD Gateway HTTPS default).
pub(crate) fn parse_gateway_endpoint(endpoint: &str) -> Result<(&str, String), Error> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return Err(Error::new("empty gateway endpoint", GwErrorKind::InvalidGwTarget));
    }

    if let Some(rest) = endpoint.strip_prefix('[') {
        let Some((host, after)) = rest.split_once(']') else {
            return Err(Error::new(
                "invalid ipv6 gateway endpoint",
                GwErrorKind::InvalidGwTarget,
            ));
        };
        if after.is_empty() {
            return Ok((host, format!("[{host}]:443")));
        }
        if let Some(port) = after.strip_prefix(':') {
            if port.is_empty() {
                return Err(Error::new("missing gateway port", GwErrorKind::InvalidGwTarget));
            }
            return Ok((host, format!("[{host}]:{port}")));
        }
        return Err(Error::new(
            "invalid ipv6 gateway endpoint",
            GwErrorKind::InvalidGwTarget,
        ));
    }

    // host:port where host has no colons (IPv4 or DNS name).
    if let Some((host, port)) = endpoint.rsplit_once(':')
        && !host.is_empty()
        && !host.contains(':')
        && !port.is_empty()
        && port.chars().all(|c| c.is_ascii_digit())
    {
        return Ok((host, endpoint.to_owned()));
    }

    // Bare host / IPv4 without port.
    if !endpoint.contains(']') {
        return Ok((endpoint, format!("{endpoint}:443")));
    }

    Err(Error::new("parse gateway endpoint", GwErrorKind::InvalidGwTarget))
}

impl GwClient {
    /// Open an MS-TSGU tunnel using the default TLS certificate policy
    /// ([`CertificateValidation::DangerouslyAcceptInvalidCertificate`]).
    ///
    /// Prefer [`Self::connect_with_certificate_validation`] when the product has an explicit
    /// authentication level or certificate prompt policy.
    pub async fn connect(
        target: &GwConnectTarget,
        client_name: &str,
    ) -> Result<(GwClient, core::net::SocketAddr), Error> {
        Self::connect_with_certificate_validation(target, client_name, CertificateValidation::default(), None).await
    }

    /// Open an MS-TSGU tunnel with the same TLS certificate policy surface as direct RDP.
    ///
    /// When `certificate_validation_callback` is `Some`, the Rustls backend may prompt/accept
    /// untrusted gateway certificates through that callback. The `native-tls` backend rejects
    /// callback-based validation.
    pub async fn connect_with_certificate_validation(
        target: &GwConnectTarget,
        client_name: &str,
        certificate_validation: CertificateValidation,
        certificate_validation_callback: Option<CertificateValidationCallback>,
    ) -> Result<(GwClient, core::net::SocketAddr), Error> {
        Self::connect_with_options(
            target,
            client_name,
            GwConnectOptions {
                certificate_validation,
                certificate_validation_callback,
                consent_callback: None,
            },
        )
        .await
    }

    /// Open an MS-TSGU tunnel with explicit connection options.
    ///
    /// When [`GwConnectOptions::consent_callback`] is set, a gateway consent message is
    /// presented through it; declining aborts the connection. Without a callback the
    /// consent message is logged and auto-accepted.
    pub async fn connect_with_options(
        target: &GwConnectTarget,
        client_name: &str,
        options: GwConnectOptions,
    ) -> Result<(GwClient, core::net::SocketAddr), Error> {
        let (io, client_addr) = open_transport_prefer_websocket(
            target,
            options.certificate_validation,
            options.certificate_validation_callback.clone(),
        )
        .await?;

        let client = Self::connect_after_transport_with_reauth(
            target.clone(),
            client_name,
            io,
            network_transport_factory(
                target.clone(),
                options.certificate_validation,
                options.certificate_validation_callback,
            ),
            options.consent_callback,
        )
        .await?;
        Ok((client, client_addr))
    }

    /// Policy parameters negotiated during tunnel authorization.
    pub fn tunnel_policy(&self) -> GwTunnelPolicy {
        self.policy
    }

    /// Open an MS-TSGU tunnel over the legacy RPC-over-HTTP transport (MS-RPCH v2).
    ///
    /// For gateways that do not support the WebSocket upgrade or dual-channel HTTP.
    /// Returns the tunnel as a boxed byte stream ready for the RDP layer.
    pub async fn connect_rpch(
        target: &GwConnectTarget,
        client_name: &str,
        certificate_validation: CertificateValidation,
        certificate_validation_callback: Option<CertificateValidationCallback>,
    ) -> Result<(RpchStream, core::net::SocketAddr), Error> {
        Box::pin(open_rpch_transport(
            target,
            client_name,
            certificate_validation,
            certificate_validation_callback,
        ))
        .await
    }

    async fn connect_after_transport_with_reauth(
        target: GwConnectTarget,
        client_name: &str,
        io: PacketIo,
        mut open_transport: TransportFactory,
        consent_callback: Option<GwConsentCallback>,
    ) -> Result<GwClient, Error> {
        let mut gw = GwConn {
            client_name: client_name.to_owned(),
            target,
            io,
            consent_callback: consent_callback.clone(),
            policy: GwTunnelPolicy::default(),
        };

        gw.handshake().await?;
        gw.tunnel(None).await?;
        gw.tunnel_auth().await?;
        gw.channel().await?;
        let client_name = gw.client_name.clone();
        let target = gw.target.clone();
        let policy = gw.policy;

        let (in_tx, in_rx) = tokio::sync::mpsc::channel(4);
        let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Bytes>(4);

        let work = tokio::spawn(async move {
            let iv = Duration::from_secs(15 * 60);
            let mut keepalive_interval = tokio::time::interval_at(tokio::time::Instant::now() + iv, iv);

            loop {
                let mut pktbuf = [0u8; 8192];

                tokio::select!(
                    _ = keepalive_interval.tick() => {
                        let pos = {
                            let mut cur = WriteCursor::new(&mut pktbuf);
                            KeepalivePkt.encode(&mut cur).map_err(|e| custom_err!("encode keepalive", e))?;
                            cur.pos()
                        };
                        gw.io.send_bytes(&pktbuf[..pos]).await?;
                    },
                    msg = gw.io.read_packet_buf() => {
                        let msg = msg?;
                        let mut cur = ReadCursor::new(&msg);
                        let hdr = PktHdr::decode(&mut cur).map_err(|e| custom_err!("decode packet header", e))?;

                        let header_length = usize::try_from(hdr.length).map_err(|_| Error::new("packet header length", GwErrorKind::Decode))?;
                        if cur.len() < header_length.saturating_sub(hdr.size()) {
                            return Err(Error::new("data loop packet body", GwErrorKind::PacketEof));
                        }
                        match hdr.ty {
                            PktTy::Keepalive => {
                                continue;
                            },
                            PktTy::Data => {
                                let p = DataPkt::decode(&mut cur).map_err(|e| custom_err!("decode data packet", e))?;
                                in_tx.send(Bytes::from(p.data.to_vec())).await.map_err(|e| custom_err!("forward inbound data", e))?;
                            },
                            PktTy::ServiceMessage => {
                                let msg = ServiceMessagePkt::decode(&mut cur)
                                    .map_err(|e| custom_err!("decode service message", e))?;
                                warn!("RD Gateway service message: {}", msg.message);
                            },
                            PktTy::ReauthMessage => {
                                let msg = ReauthMessagePkt::decode(&mut cur)
                                    .map_err(|e| custom_err!("decode reauth message", e))?;
                                // MS-TSGU 4.1.3: repeat the connection setup sequence on a
                                // secondary connection, passing the tunnel context from the
                                // gateway in the tunnel-create packet, then swap the data path.
                                info!(
                                    "RD Gateway requested reauthentication (context 0x{:016x}); opening secondary connection",
                                    msg.reauth_tunnel_context
                                );
                                let new_io = open_transport().await?;
                                let mut secondary = GwConn {
                                    client_name: client_name.clone(),
                                    target: target.clone(),
                                    io: new_io,
                                    consent_callback: consent_callback.clone(),
                                    policy: GwTunnelPolicy::default(),
                                };
                                secondary.handshake().await?;
                                secondary.tunnel(Some(msg.reauth_tunnel_context)).await?;
                                secondary.tunnel_auth().await?;
                                secondary.channel().await?;
                                gw = secondary;
                                info!("RD Gateway reauthentication completed");
                            },
                            PktTy::ChannelClose | PktTy::ChannelCloseResponse => {
                                let close = ChannelClosePkt::decode(&mut cur)
                                    .map_err(|e| custom_err!("decode channel close", e))?;
                                if close.status_code != 0 {
                                    return Err(Error::new(
                                        "channel closed by gateway",
                                        GwErrorKind::GatewayCode(close.status_code),
                                    ));
                                }
                                return Err(Error::new("channel closed by gateway", GwErrorKind::Connect));
                            },
                            x => {
                                warn!("Unhandled gw packet type {x:?}");
                            }
                        }
                    },
                    next = out_rx.recv() => {
                        let Some(next) = next else {
                            // The client closed the write side: close the channel, then
                            // the transport ([MS-TSGU] 3.3.5.5).
                            let close = ChannelClosePkt { status_code: 0 };
                            let pos = {
                                let mut cur = WriteCursor::new(&mut pktbuf);
                                close.encode(&mut cur).map_err(|e| custom_err!("encode channel close", e))?;
                                cur.pos()
                            };
                            gw.io.send_bytes(&pktbuf[..pos]).await?;
                            gw.io.close().await?;
                            return Ok(());
                        };
                        let pkt = DataPkt { data: &next };

                        let pos = {
                            let mut cur = WriteCursor::new(&mut pktbuf);
                            pkt.encode(&mut cur).map_err(|e| custom_err!("encode data packet", e))?;
                            cur.pos()
                        };
                        gw.io.send_bytes(&pktbuf[..pos]).await?;
                    }
                );
            }
        });

        Ok(GwClient {
            work,
            work_done: false,
            rx: in_rx,
            rx_bufs: vec![],
            tx: PollSender::new(out_tx),
            policy,
        })
    }
}

impl GwConn {
    async fn send_packet<E: Encode>(&mut self, payload: &E) -> Result<(), Error> {
        let mut buf = [0u8; 4096];
        let pos = {
            let mut cur = WriteCursor::new(&mut buf);
            payload
                .encode(&mut cur)
                .map_err(|e| Error::new("packet encode", GwErrorKind::Encode).with_source(e))?;
            cur.pos()
        };
        self.io.send_bytes(&buf[..pos]).await
    }

    async fn read_packet(&mut self) -> Result<(PktHdr, Bytes), Error> {
        let mut msg = self.io.read_packet_buf().await?;
        let mut cur = ReadCursor::new(&msg);

        let hdr = PktHdr::decode(&mut cur).map_err(|e| custom_err!("decode packet header", e))?;

        let header_length =
            usize::try_from(hdr.length).map_err(|_| Error::new("packet header length", GwErrorKind::Decode))?;
        if cur.len() != header_length.saturating_sub(hdr.size()) {
            return Err(Error::new("read packet body", GwErrorKind::PacketEof));
        }

        Ok((hdr, msg.split_off(cur.pos())))
    }

    async fn handshake(&mut self) -> Result<(), Error> {
        // Advertise SSPI NTLM extended auth so gateways that require the post-handshake
        // NTLM blob exchange (MS-TSGU 3.3.5.3) can select it. HTTP NTLM already done
        // during WebSocket upgrade is independent and usually yields NONE here.
        // Smart-card credentials authenticate via Kerberos PKINIT, and a Basic HTTP
        // fallback produces no NTLM blobs, so extended auth is not advertised in either
        // case.
        let hs = HandshakeReqPkt {
            ver_major: 1,
            ver_minor: 0,
            extended_auth: if self.target.smart_card.is_some() || self.io.http_auth_basic() {
                HttpExtendedAuth::HTTP_EXTENDED_AUTH_NONE
            } else {
                HttpExtendedAuth::HTTP_EXTENDED_AUTH_SSPI_NTLM
            },
            ..HandshakeReqPkt::default()
        };
        self.send_packet(&hs).await?;
        let (hdr, bytes) = self.read_packet().await?;
        if hdr.ty != PktTy::HandshakeResp {
            return Err(Error::new("handshake response", GwErrorKind::UnexpectedPacket));
        }

        let mut cur = ReadCursor::new(&bytes);
        let resp = HandshakeRespPkt::decode(&mut cur).map_err(|e| custom_err!("decode handshake response", e))?;
        if resp.error_code != 0 {
            return Err(Error::new("handshake", GwErrorKind::GatewayCode(resp.error_code)));
        }
        if resp.ver_major != 1 || resp.ver_minor != 0 || resp.server_version != 0 {
            return Err(Error::new("handshake version negotiation", GwErrorKind::Connect));
        }

        // The response advertises the extended-auth methods the gateway supports; it is a
        // capability mask, not a request. Only run the SSPI NTLM blob exchange when the
        // HTTP layer authenticated with NTLM — a Basic fallback produces no NTLM blobs, so
        // the exchange is skipped (the gateway treats a NONE advertisement as sufficient).
        if !self.io.http_auth_basic()
            && resp
                .extended_auth
                .contains(HttpExtendedAuth::HTTP_EXTENDED_AUTH_SSPI_NTLM)
        {
            self.extended_auth_sspi_ntlm().await?;
        } else if !self.io.http_auth_basic()
            && self.target.smart_card.is_none()
            && resp
                .extended_auth
                .intersects(HttpExtendedAuth::HTTP_EXTENDED_AUTH_SC | HttpExtendedAuth::HTTP_EXTENDED_AUTH_PAA)
        {
            return Err(Error::new(
                "gateway requested smart-card or PAA extended auth",
                GwErrorKind::UnsupportedFeature,
            ));
        }

        Ok(())
    }

    /// MS-TSGU 3.3.5.3.2–3.3.5.3.3: exchange `HTTP_EXTENDED_AUTH_PACKET` NTLM blobs.
    async fn extended_auth_sspi_ntlm(&mut self) -> Result<(), Error> {
        let mut ntlm = NtlmHttpAuth::new(&self.target.gw_user, &self.target.gw_pass)?;
        let (token, mut complete) = ntlm.step_token(None)?;
        self.send_packet(&ExtendedAuthPkt::client_blob(token)).await?;

        const MAX_ROUNDS: usize = 8;
        for _ in 0..MAX_ROUNDS {
            if complete {
                return Ok(());
            }

            let (hdr, bytes) = self.read_packet().await?;
            if hdr.ty != PktTy::ExtendedAuth {
                return Err(Error::new("extended auth response", GwErrorKind::UnexpectedPacket));
            }
            let mut cur = ReadCursor::new(&bytes);
            let resp = ExtendedAuthPkt::decode(&mut cur).map_err(|e| custom_err!("decode extended auth", e))?;
            if resp.error_code() != 0 {
                return Err(Error::new("extended auth", GwErrorKind::GatewayCode(resp.error_code())));
            }

            let (token, done) = ntlm.step_token(Some(resp.blob()))?;
            complete = done;
            if !token.is_empty() {
                self.send_packet(&ExtendedAuthPkt::client_blob(token)).await?;
            }
            if complete {
                return Ok(());
            }
        }

        Err(Error::new("extended auth ntlm rounds exceeded", GwErrorKind::Connect))
    }

    async fn tunnel(&mut self, reauth_tunnel_context: Option<u64>) -> Result<(), Error> {
        let req = TunnelReqPkt {
            // Advertise the messaging and reauth caps most corporate RDG policies expect.
            // Consent text is logged and auto-accepted until a UI path exists.
            caps: HttpCapsTy::MessagingConsentSign.as_u32()
                | HttpCapsTy::MessagingServiceMsg.as_u32()
                | HttpCapsTy::IdleTimeout.as_u32()
                | HttpCapsTy::Reauth.as_u32(),
            fields_present: 0,
            reauth_tunnel_context,
            ..TunnelReqPkt::default()
        };
        self.send_packet(&req).await?;

        let (hdr, bytes) = self.read_packet().await?;
        if hdr.ty != PktTy::TunnelResp {
            return Err(Error::new("tunnel response", GwErrorKind::UnexpectedPacket));
        }
        let mut cur = ReadCursor::new(&bytes);

        let resp = TunnelRespPkt::decode(&mut cur).map_err(|e| custom_err!("decode tunnel response", e))?;
        if resp.status_code != 0 {
            return Err(Error::new("tunnel create", GwErrorKind::GatewayCode(resp.status_code)));
        }
        if !cur.eof() {
            return Err(Error::new("tunnel response trailing bytes", GwErrorKind::Decode));
        }
        if !resp.consent_msg.is_empty() {
            let text = String::from_utf16_lossy(
                &resp
                    .consent_msg
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect::<Vec<_>>(),
            )
            .trim_end_matches('\0')
            .to_owned();
            match &self.consent_callback {
                Some(callback) => {
                    if !callback(&text) {
                        return Err(Error::new(
                            "gateway consent declined by user",
                            GwErrorKind::AccessDenied,
                        ));
                    }
                }
                // Auto-accept until the product wires a consent UI through the callback.
                None => warn!("RD Gateway consent message (auto-accepted): {text}"),
            }
        }
        Ok(())
    }

    async fn tunnel_auth(&mut self) -> Result<(), Error> {
        // NAP statement-of-health is not generated yet; send client name only.
        let req = TunnelAuthPkt {
            fields_present: 0,
            client_name: self.client_name.clone(),
            statement_of_health: None,
        };
        self.send_packet(&req).await?;

        let (hdr, bytes) = self.read_packet().await?;
        if hdr.ty != PktTy::TunnelAuthResponse {
            return Err(Error::new("tunnel auth response", GwErrorKind::UnexpectedPacket));
        }
        let mut cur = ReadCursor::new(&bytes);
        let resp = TunnelAuthRespPkt::decode(&mut cur).map_err(|e| custom_err!("decode tunnel auth response", e))?;

        if resp.error_code() != 0 {
            return Err(Error::new("tunnel auth", GwErrorKind::GatewayCode(resp.error_code())));
        }

        if let Some(flags) = resp.redir_flags {
            info!("RD Gateway device redirection policy: 0x{flags:08x}");
            self.policy.redir_flags = Some(GwTunnelRedirFlags(flags));
        }
        if let Some(minutes) = resp.idle_timeout_minutes {
            info!("RD Gateway idle timeout: {minutes} minute(s)");
            self.policy.idle_timeout_minutes = Some(minutes);
        }
        if let Some(soh) = &resp.soh_response {
            debug!("RD Gateway SoH response present ({} bytes)", soh.len());
        }
        Ok(())
    }

    async fn channel(&mut self) -> Result<ChannelResp, Error> {
        let req = ChannelPkt {
            resources: vec![self.target.server.clone()],
            port: self.target.server_port,
            protocol: 3,
        };
        self.send_packet(&req).await?;

        let (hdr, bytes) = self.read_packet().await?;
        if hdr.ty != PktTy::ChannelResp {
            return Err(Error::new("channel response", GwErrorKind::UnexpectedPacket));
        }
        let mut cur: ReadCursor<'_> = ReadCursor::new(&bytes);
        let resp = ChannelResp::decode(&mut cur).map_err(|e| custom_err!("decode channel response", e))?;
        if resp.error_code() != 0 {
            return Err(Error::new(
                "channel create",
                GwErrorKind::GatewayCode(resp.error_code()),
            ));
        }
        if !cur.eof() {
            return Err(Error::new("channel response trailing bytes", GwErrorKind::Decode));
        }
        if resp.udp_port() != 0 && !resp.authn_cookie().is_empty() {
            let offer = GwUdpOffer {
                port: resp.udp_port(),
                authn_cookie: resp.authn_cookie().to_vec(),
            };
            // DTLS + MS-RDPEUDP data path is not opened yet; CONNECT_PKT helpers are public for
            // callers that implement the side channel themselves.
            debug!(
                "RD Gateway offered UDP channel parameters (port={}, cookie_len={})",
                offer.port,
                offer.authn_cookie.len()
            );
            let _ = offer;
        } else if resp.udp_port() != 0 || !resp.authn_cookie().is_empty() {
            debug!(
                "RD Gateway UDP offer incomplete (port={}, cookie_len={})",
                resp.udp_port(),
                resp.authn_cookie().len()
            );
        }
        Ok(resp)
    }
}

impl AsyncRead for GwClient {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // Propagate error or premature exit (?)
        if !self.work_done {
            match self.work.poll_unpin(cx) {
                Poll::Ready(Err(e)) => {
                    self.work_done = true;
                    return Poll::Ready(Err(io::Error::other(e)));
                }
                Poll::Ready(Ok(Err(e))) => {
                    self.work_done = true;
                    return Poll::Ready(Err(io::Error::other(e)));
                }
                Poll::Ready(Ok(Ok(()))) => {
                    self.work_done = true;
                }
                Poll::Pending => (),
            }
        }

        // Get new bufs
        if let Poll::Ready(Some(new_buf)) = self.rx.poll_recv(cx) {
            self.rx_bufs.push(new_buf);
        }

        // Read from all queued bufs
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
            return Poll::Ready(Ok(()));
        }
        if self.work_done {
            // The work task ended and no data is buffered: the gateway stream is closed.
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "gateway tunnel closed",
            )));
        }
        Poll::Pending
    }
}

impl AsyncWrite for GwClient {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        // Propagate error or premature exit (?)
        if !self.work_done {
            match self.work.poll_unpin(cx) {
                Poll::Ready(Err(e)) => {
                    self.work_done = true;
                    return Poll::Ready(Err(io::Error::other(e)));
                }
                Poll::Ready(Ok(Err(e))) => {
                    self.work_done = true;
                    return Poll::Ready(Err(io::Error::other(e)));
                }
                Poll::Ready(Ok(Ok(()))) => {
                    self.work_done = true;
                }
                Poll::Pending => (),
            }
        }

        match self.tx.poll_reserve(cx) {
            Poll::Ready(Ok(())) => {
                if self.tx.send_item(Bytes::from(buf.to_vec())).is_err() {
                    return Poll::Ready(Err(io::Error::other("Sender closed")));
                }
                return Poll::Ready(Ok(buf.len()));
            }
            Poll::Ready(Err(err)) => {
                return Poll::Ready(Err(io::Error::other(err)));
            }
            Poll::Pending => (),
        }

        Poll::Pending
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut core::task::Context<'_>) -> Poll<Result<(), io::Error>> {
        // TODO: call flush on the backing sink (e.g. websocket, but atleast for that backend doesnt seem to matter)?
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> Poll<Result<(), io::Error>> {
        // Closing the sender ends the outbound queue; the work task then performs the
        // MS-TSGU channel close and the transport close handshake.
        self.tx.close();
        if self.work_done {
            return Poll::Ready(Ok(()));
        }
        match self.work.poll_unpin(cx) {
            Poll::Ready(Ok(Ok(()))) => {
                self.work_done = true;
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Ok(Err(e))) => {
                self.work_done = true;
                Poll::Ready(Err(io::Error::other(e)))
            }
            Poll::Ready(Err(e)) => {
                self.work_done = true;
                Poll::Ready(Err(io::Error::other(e)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;
    use crate::mock::{MockGatewayConfig, mock_gateway};

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

    async fn connect_with_mock(
        config: MockGatewayConfig,
    ) -> Result<(GwClient, tokio::task::JoinHandle<Vec<PktTy>>), Error> {
        connect_with_mock_consent(config, None).await
    }

    async fn connect_with_mock_consent(
        config: MockGatewayConfig,
        consent_callback: Option<GwConsentCallback>,
    ) -> Result<(GwClient, tokio::task::JoinHandle<Vec<PktTy>>), Error> {
        let (io, gateway) = mock_gateway(config);
        let client = GwClient::connect_after_transport_with_reauth(
            target(),
            "test-client",
            io,
            Box::new(|| Box::pin(unsupported_reauth())),
            consent_callback,
        )
        .await?;
        Ok((client, gateway))
    }

    #[tokio::test]
    async fn mock_gateway_happy_path_echoes_data() {
        let (mut client, gateway) = connect_with_mock(MockGatewayConfig::default())
            .await
            .expect("connect through mock gateway");

        client.write_all(b"hello-rdp").await.expect("write data");
        let mut buf = [0u8; 9];
        client.read_exact(&mut buf).await.expect("read echo");
        assert_eq!(&buf, b"hello-rdp");

        drop(client);
        let received = gateway.await.expect("gateway task");
        assert_eq!(
            received,
            [
                PktTy::HandshakeReq,
                PktTy::TunnelCreate,
                PktTy::TunnelAuth,
                PktTy::ChannelCreate,
                PktTy::Data,
            ]
        );
    }

    #[tokio::test]
    async fn mock_gateway_tunnel_create_failure_surfaces_status() {
        let result = connect_with_mock(MockGatewayConfig {
            tunnel_status_code: 0x8007_59DA, // E_PROXY_RAP_ACCESSDENIED
            ..MockGatewayConfig::default()
        })
        .await;

        let error = result.err().expect("tunnel create must fail");
        let text = error.to_string();
        assert!(text.contains("tunnel create"), "unexpected error: {text}");
    }

    #[tokio::test]
    async fn mock_gateway_tunnel_auth_failure_surfaces_status() {
        let result = connect_with_mock(MockGatewayConfig {
            tunnel_auth_error_code: 0x8007_59ED, // E_PROXY_QUARANTINE_ACCESSDENIED
            ..MockGatewayConfig::default()
        })
        .await;

        let error = result.err().expect("tunnel auth must fail");
        let text = error.to_string();
        assert!(text.contains("tunnel auth"), "unexpected error: {text}");
    }

    #[tokio::test]
    async fn mock_gateway_channel_failure_surfaces_status() {
        let result = connect_with_mock(MockGatewayConfig {
            channel_error_code: 0x0000_59DD, // E_PROXY_TS_CONNECTFAILED
            ..MockGatewayConfig::default()
        })
        .await;

        let error = result.err().expect("channel create must fail");
        let text = error.to_string();
        assert!(text.contains("channel create"), "unexpected error: {text}");
    }

    #[tokio::test]
    async fn mock_gateway_consent_message_is_accepted_and_logged() {
        let (client, gateway) = connect_with_mock(MockGatewayConfig {
            consent_message: Some("Authorized users only".to_owned()),
            ..MockGatewayConfig::default()
        })
        .await
        .expect("consent message should not block connect");
        drop(client);
        gateway.await.expect("gateway task");
    }

    #[tokio::test]
    async fn mock_gateway_consent_decline_aborts_connect() {
        let result = connect_with_mock_consent(
            MockGatewayConfig {
                consent_message: Some("Authorized users only".to_owned()),
                ..MockGatewayConfig::default()
            },
            Some(std::sync::Arc::new(|_| false)),
        )
        .await;

        let error = result.err().expect("declined consent must abort");
        assert!(
            error.to_string().contains("consent declined"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn mock_gateway_consent_callback_receives_message_text() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let seen_clone = std::sync::Arc::clone(&seen);
        let (client, gateway) = connect_with_mock_consent(
            MockGatewayConfig {
                consent_message: Some("Authorized users only".to_owned()),
                ..MockGatewayConfig::default()
            },
            Some(std::sync::Arc::new(move |text: &str| {
                *seen_clone.lock().expect("seen lock") = text.to_owned();
                true
            })),
        )
        .await
        .expect("accepted consent connects");

        assert_eq!(*seen.lock().expect("seen lock"), "Authorized users only");
        drop(client);
        gateway.await.expect("gateway task");
    }

    #[tokio::test]
    async fn mock_gateway_tunnel_policy_is_captured() {
        let (client, gateway) = connect_with_mock(MockGatewayConfig {
            redir_flags: Some(GwTunnelRedirFlags::DISABLE_CLIPBOARD | GwTunnelRedirFlags::DISABLE_DRIVE),
            idle_timeout_minutes: Some(15),
            ..MockGatewayConfig::default()
        })
        .await
        .expect("connect through mock gateway");

        let policy = client.tunnel_policy();
        let flags = policy.redir_flags.expect("redir flags");
        assert!(flags.clipboard_disabled());
        assert!(flags.drives_disabled());
        assert!(!flags.all_disabled());
        assert_eq!(policy.idle_timeout_minutes, Some(15));

        drop(client);
        gateway.await.expect("gateway task");
    }

    #[test]
    fn redir_flags_all_disabled_disables_everything() {
        let flags = GwTunnelRedirFlags(GwTunnelRedirFlags::DISABLE_ALL);
        assert!(flags.clipboard_disabled());
        assert!(flags.drives_disabled());
        assert!(flags.all_disabled());

        let enabled = GwTunnelRedirFlags(GwTunnelRedirFlags::ENABLE_ALL);
        assert!(!enabled.clipboard_disabled());
        assert!(!enabled.all_disabled());
    }

    #[tokio::test]
    async fn mock_gateway_reauth_opens_secondary_connection() {
        use crate::mock::MockGatewayFactory;

        const REAUTH_CONTEXT: u64 = 0x0123_4567_89ab_cdef;
        let factory = MockGatewayFactory::new(MockGatewayConfig {
            reauth_tunnel_context: Some(REAUTH_CONTEXT),
            ..MockGatewayConfig::default()
        });

        let first = factory.open();
        let transport_factory = {
            let factory = std::sync::Arc::clone(&factory);
            Box::new(
                move || -> Pin<Box<dyn Future<Output = Result<PacketIo, Error>> + Send>> {
                    let factory = std::sync::Arc::clone(&factory);
                    Box::pin(async move { Ok(factory.open()) })
                },
            )
        };
        let mut client =
            GwClient::connect_after_transport_with_reauth(target(), "test-client", first, transport_factory, None)
                .await
                .expect("connect through mock gateway");

        // Wait for the secondary connection to finish before writing: data sent
        // while the reauth handoff is in flight may still flow on the old channel.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if factory
                    .seen_reauth_context
                    .lock()
                    .expect("reauth context lock")
                    .is_some()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("reauth must complete");

        assert_eq!(
            *factory.seen_reauth_context.lock().expect("reauth context lock"),
            Some(REAUTH_CONTEXT),
            "secondary connection must carry the reauth tunnel context"
        );

        // Data flows through the secondary connection after reauthentication.
        client.write_all(b"after-reauth").await.expect("write data");
        let mut buf = [0u8; 12];
        client.read_exact(&mut buf).await.expect("read echo");
        assert_eq!(&buf, b"after-reauth");

        drop(client);
        let tasks = core::mem::take(&mut *factory.tasks.lock().expect("tasks lock"));
        for task in tasks {
            task.abort();
        }
    }

    #[tokio::test]
    async fn mock_gateway_graceful_shutdown_closes_channel() {
        let (mut client, gateway) = connect_with_mock(MockGatewayConfig::default())
            .await
            .expect("connect through mock gateway");

        client.shutdown().await.expect("graceful shutdown");
        drop(client);

        let received = gateway.await.expect("gateway task");
        assert_eq!(
            received,
            [
                PktTy::HandshakeReq,
                PktTy::TunnelCreate,
                PktTy::TunnelAuth,
                PktTy::ChannelCreate,
                PktTy::ChannelClose,
            ]
        );
    }

    #[tokio::test]
    async fn mock_gateway_channel_close_with_error_fails_stream() {
        let (mut client, gateway) = connect_with_mock(MockGatewayConfig {
            close_status_code: Some(0x0000_59F6), // E_PROXY_SESSIONTIMEOUT
            ..MockGatewayConfig::default()
        })
        .await
        .expect("connect through mock gateway");

        let mut buf = [0u8; 1];
        let error = client.read_exact(&mut buf).await.expect_err("close must fail the read");
        assert!(
            error.to_string().contains("channel closed"),
            "unexpected error: {error}"
        );

        drop(client);
        gateway.await.expect("gateway task");
    }

    #[test]
    fn parse_endpoint_defaults_to_443() {
        let (host, addr) = parse_gateway_endpoint("rdg.contoso.com").unwrap();
        assert_eq!(host, "rdg.contoso.com");
        assert_eq!(addr, "rdg.contoso.com:443");
    }

    #[test]
    fn parse_endpoint_keeps_explicit_port() {
        let (host, addr) = parse_gateway_endpoint("rdg.contoso.com:8443").unwrap();
        assert_eq!(host, "rdg.contoso.com");
        assert_eq!(addr, "rdg.contoso.com:8443");
    }

    #[test]
    fn parse_endpoint_ipv6_bracketed() {
        let (host, addr) = parse_gateway_endpoint("[2001:db8::1]").unwrap();
        assert_eq!(host, "2001:db8::1");
        assert_eq!(addr, "[2001:db8::1]:443");

        let (host, addr) = parse_gateway_endpoint("[2001:db8::1]:8443").unwrap();
        assert_eq!(host, "2001:db8::1");
        assert_eq!(addr, "[2001:db8::1]:8443");
    }

    #[test]
    fn gateway_code_display_includes_label() {
        let kind = GwErrorKind::GatewayCode(0x8007_59DA);
        assert!(kind.to_string().contains("E_PROXY_RAP_ACCESSDENIED"));
    }
}
