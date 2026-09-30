#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

#[macro_use]
mod macros;

#[doc(hidden)]
pub mod http_auth;
#[cfg(test)]
mod mock_rpch;
mod packet_io;
mod proto;
#[doc(hidden)]
pub mod rpc;
mod rpc_transport;
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "live RPCH transport wiring is intentionally deferred")
)]
mod rpch;
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub mod test_support;
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
use log::warn;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::sync::PollSender;

use self::packet_io::{GatewayTransport, PacketIo, open_gateway_transport};
#[doc(hidden)]
pub use self::proto::{ChannelClosePkt, ReauthMessagePkt, ServiceMessagePkt, gateway_code_label};
use self::proto::{
    ChannelPkt, ChannelResp, DataPkt as HttpDataPkt, ExtendedAuthPkt, HandshakeReqPkt, HandshakeRespPkt, HttpCapsTy,
    HttpExtendedAuth, KeepalivePkt, PktHdr, PktTy, TunnelAuthPkt, TunnelAuthRespPkt, TunnelReqPkt, TunnelRespPkt,
};
pub use self::udp::{
    AaSynData, AaSynDataResp, ConnectPkt, ConnectPktResp, DataPkt, DiscPkt, GwUdpOffer, MAX_CONNECT_REQ_FRAGMENT_SIZE,
    UdpCorrelationInfo, UdpPacketHeader, UdpPktType, encode_connect_request, fragment_connect_pkt,
};

/// Smart-card credentials used for HTTP Negotiate Kerberos PKINIT authentication.
///
/// This type owns application-supplied identity material and reader metadata.
/// It does not perform smart-card I/O or prompt for a PIN.
#[derive(Clone)]
pub struct GwSmartCardCredentials {
    /// User principal name supplied to Kerberos.
    pub username: String,
    /// PIN used to unlock the smart card.
    pub pin: String,
    /// DER-encoded X.509 certificate identifying the user.
    pub certificate: Vec<u8>,
    /// PKCS#1 private key for an emulated smart card.
    ///
    /// Set this to `None` to use the Windows smart-card API.
    pub private_key: Option<Vec<u8>>,
    /// Smart-card reader name.
    pub reader_name: String,
    /// Optional smart-card name.
    pub card_name: Option<String>,
    /// Optional key container name.
    pub container_name: Option<String>,
    /// Optional cryptographic service provider name.
    pub csp_name: Option<String>,
}

impl fmt::Debug for GwSmartCardCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GwSmartCardCredentials")
            .field("username", &"<redacted>")
            .field("pin", &"<redacted>")
            .field("certificate", &"<redacted>")
            .field("private_key", &self.private_key.as_ref().map(|_| "<redacted>"))
            .field("reader_name", &self.reader_name)
            .field("card_name", &self.card_name)
            .field("container_name", &self.container_name)
            .field("csp_name", &self.csp_name)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct GwConnectTarget {
    /// Gateway host with an optional port.
    ///
    /// Omitted ports use the HTTPS default of 443.
    /// Bracket IPv6 literals, such as `[2001:db8::1]:8443`.
    pub gw_endpoint: String,
    pub gw_user: String,
    pub gw_pass: String,
    /// Optional smart-card credentials for HTTP Negotiate Kerberos PKINIT authentication.
    pub smart_card: Option<Box<GwSmartCardCredentials>>,

    pub server: String,
}

/// Policy parameters reported by the gateway during [tunnel authorization][MS-TSGU 2.2.10.17].
///
/// IronRDP exposes these values but does not enforce device redirection restrictions or client-side idle timeouts.
///
/// [MS-TSGU 2.2.10.17]: https://winprotocoldocs-bhdugrdyduf5h2e4.b02.azurefd.net/MS-TSGU/%5bMS-TSGU%5d.pdf#page=70
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GwTunnelPolicy {
    /// Device redirection flags supplied by the gateway, if any.
    pub redirection_flags: Option<u32>,
    /// Idle timeout in minutes supplied by the gateway, if any.
    pub idle_timeout_minutes: Option<u32>,
    /// Statement of health response supplied by the gateway, if any.
    pub soh_response: Option<Vec<u8>>,
}

/// Synchronously decides whether to accept a gateway consent message.
///
/// The callback receives the UTF-16LE consent message decoded from
/// [HTTP_TUNNEL_RESPONSE_OPTIONAL][MS-TSGU 2.2.10.21] as an [HTTP_UNICODE_STRING][MS-TSGU 2.2.10.22].
/// Returning `false` declines the gateway consent and stops tunnel setup.
///
/// A consent message received during later background reauthentication is declined because this
/// borrowed callback is no longer available.
///
/// [MS-TSGU 2.2.10.21]: https://winprotocoldocs-bhdugrdyduf5h2e4.b02.azurefd.net/MS-TSGU/%5bMS-TSGU%5d.pdf#page=72
/// [MS-TSGU 2.2.10.22]: https://winprotocoldocs-bhdugrdyduf5h2e4.b02.azurefd.net/MS-TSGU/%5bMS-TSGU%5d.pdf#page=73
pub type GwConsentCallback<'a> = dyn FnMut(&str) -> bool + Send + 'a;

type Error = ironrdp_error::Error<GwErrorKind>;

/// Gateway authentication selected for the current HTTP connection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GwSessionAuthentication {
    /// Authentication completed during the HTTP or WebSocket setup.
    #[default]
    Http,
    /// NTLM authentication completed with `HTTP_EXTENDED_AUTH_PACKET` messages.
    NtlmSspi,
}

/// An extended authentication method advertised by the gateway.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GwExtendedAuthentication {
    SmartCard,
    PluggableAuthentication,
    NtlmSspi,
    Unknown(u16),
}

#[derive(Debug)]
#[non_exhaustive]
pub enum GwErrorKind {
    InvalidGwTarget,
    InvalidCertificateValidation,
    Connect,
    /// A nonzero HRESULT returned by the gateway during control setup.
    GatewayCode(u32),
    HttpStatus(u16),
    PacketEof,
    UnsupportedFeature,
    UnsupportedExtendedAuthentication(GwExtendedAuthentication),
    ConsentDeclined,
    Custom,
    Encode,
    Decode,
}

trait GwErrorExt {
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
        let x = match self {
            GwErrorKind::InvalidGwTarget => "invalid GW Target",
            GwErrorKind::InvalidCertificateValidation => "invalid certificate validation configuration",
            GwErrorKind::Connect => "connection error",
            GwErrorKind::GatewayCode(code) => match gateway_code_label(*code) {
                Some(label) => return write!(f, "gateway error 0x{code:08x} ({label})"),
                None => return write!(f, "gateway error 0x{code:08x}"),
            },
            GwErrorKind::HttpStatus(status) => return write!(f, "unexpected http status {status}"),
            GwErrorKind::PacketEof => "PacketEOF",
            GwErrorKind::UnsupportedFeature => "unsupported feature",
            GwErrorKind::UnsupportedExtendedAuthentication(method) => {
                return write!(f, "unsupported extended authentication method {method:?}");
            }
            GwErrorKind::ConsentDeclined => "gateway consent declined",
            GwErrorKind::Custom => "custom",
            GwErrorKind::Encode => "encode",
            GwErrorKind::Decode => "decode",
        };
        f.write_str(x)
    }
}

impl core::error::Error for GwErrorKind {}

struct GwConn {
    client_name: String,
    target: GwConnectTarget,
    /// Target resource port presented in HTTP_CHANNEL_PACKET (`port`).
    ///
    /// Common values are `3389` for ordinary RDP and `2179` for Hyper-V VMConnect.
    server_port: u16,
    io: PacketIo,
}

type TransportFactory =
    Box<dyn FnMut() -> Pin<Box<dyn Future<Output = Result<GatewayTransport, Error>> + Send>> + Send>;
type ReauthenticationFuture = Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>;

#[derive(Clone, Copy)]
enum ConsentFallback {
    Accept,
    Reject,
}

fn network_transport_factory(
    target: GwConnectTarget,
    certificate_validation: CertificateValidation,
    certificate_validation_callback: Option<CertificateValidationCallback>,
) -> TransportFactory {
    Box::new(move || {
        let target = target.clone();
        let certificate_validation_callback = certificate_validation_callback.clone();
        Box::pin(async move {
            open_gateway_transport(&target, certificate_validation, certificate_validation_callback)
                .await
                .map(|(transport, _)| transport)
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
    session_authentication: GwSessionAuthentication,
}

impl Drop for GwClient {
    fn drop(&mut self) {
        self.work.abort();
    }
}

impl GwClient {
    /// Open an MS-TSGU tunnel, presenting port `3389` to the gateway.
    ///
    /// Use [`Self::connect_with_port`] when the target resource is not on the ordinary RDP port
    /// (for example Hyper-V VMConnect on `2179`).
    pub async fn connect(
        target: &GwConnectTarget,
        client_name: &str,
    ) -> Result<(GwClient, core::net::SocketAddr), Error> {
        Self::connect_with_certificate_validation(target, client_name, CertificateValidation::default(), None).await
    }

    /// Open an MS-TSGU tunnel with an explicit TLS certificate-validation configuration.
    ///
    /// This presents port `3389` to the gateway.
    ///
    /// Certificate-validation callbacks require the Rustls TLS backend and cannot be
    /// combined with [`CertificateValidation::DangerouslyAcceptInvalidCertificate`].
    pub async fn connect_with_certificate_validation(
        target: &GwConnectTarget,
        client_name: &str,
        certificate_validation: CertificateValidation,
        certificate_validation_callback: Option<CertificateValidationCallback>,
    ) -> Result<(GwClient, core::net::SocketAddr), Error> {
        Self::connect_with_port_and_certificate_validation(
            target,
            client_name,
            3389,
            certificate_validation,
            certificate_validation_callback,
        )
        .await
    }

    /// Open an MS-TSGU tunnel and decide a gateway consent message synchronously.
    ///
    /// The callback is invoked once for each consent message returned while creating the initial tunnel.
    /// Returning `false` stops tunnel setup with [`GwErrorKind::ConsentDeclined`].
    pub async fn connect_with_consent(
        target: &GwConnectTarget,
        client_name: &str,
        consent_callback: &mut GwConsentCallback<'_>,
    ) -> Result<(GwClient, core::net::SocketAddr), Error> {
        Self::connect_with_port_and_consent(target, client_name, 3389, consent_callback).await
    }

    /// Open an MS-TSGU tunnel, presenting `server_port` as HTTP_CHANNEL_PACKET `port`.
    pub async fn connect_with_port(
        target: &GwConnectTarget,
        client_name: &str,
        server_port: u16,
    ) -> Result<(GwClient, core::net::SocketAddr), Error> {
        Self::connect_with_port_and_certificate_validation(
            target,
            client_name,
            server_port,
            CertificateValidation::default(),
            None,
        )
        .await
    }

    /// Open an MS-TSGU tunnel with an explicit TLS certificate-validation configuration.
    ///
    /// Certificate-validation callbacks require the Rustls TLS backend and cannot be
    /// combined with [`CertificateValidation::DangerouslyAcceptInvalidCertificate`].
    pub async fn connect_with_port_and_certificate_validation(
        target: &GwConnectTarget,
        client_name: &str,
        server_port: u16,
        certificate_validation: CertificateValidation,
        certificate_validation_callback: Option<CertificateValidationCallback>,
    ) -> Result<(GwClient, core::net::SocketAddr), Error> {
        if certificate_validation == CertificateValidation::DangerouslyAcceptInvalidCertificate
            && certificate_validation_callback.is_some()
        {
            return Err(Error::new(
                "certificate validation",
                GwErrorKind::InvalidCertificateValidation,
            ));
        }

        Self::connect_with_port_and_optional_consent(
            target,
            client_name,
            server_port,
            certificate_validation,
            certificate_validation_callback,
            None,
        )
        .await
    }

    /// Open an MS-TSGU tunnel and decide a gateway consent message synchronously.
    ///
    /// The callback is invoked once for each consent message returned while creating the initial tunnel.
    /// Returning `false` stops tunnel setup with [`GwErrorKind::ConsentDeclined`].
    pub async fn connect_with_port_and_consent(
        target: &GwConnectTarget,
        client_name: &str,
        server_port: u16,
        consent_callback: &mut GwConsentCallback<'_>,
    ) -> Result<(GwClient, core::net::SocketAddr), Error> {
        Self::connect_with_port_and_optional_consent(
            target,
            client_name,
            server_port,
            CertificateValidation::default(),
            None,
            Some(consent_callback),
        )
        .await
    }

    async fn connect_with_port_and_optional_consent(
        target: &GwConnectTarget,
        client_name: &str,
        server_port: u16,
        certificate_validation: CertificateValidation,
        certificate_validation_callback: Option<CertificateValidationCallback>,
        consent_callback: Option<&mut GwConsentCallback<'_>>,
    ) -> Result<(GwClient, core::net::SocketAddr), Error> {
        let (transport, client_addr) =
            open_gateway_transport(target, certificate_validation, certificate_validation_callback.clone()).await?;
        Self::connect_ws_with_reauth(
            target.clone(),
            client_name,
            server_port,
            transport,
            network_transport_factory(target.clone(), certificate_validation, certificate_validation_callback),
            consent_callback,
        )
        .await
        .map(|x| (x, client_addr))
    }

    #[cfg(feature = "test-support")]
    async fn connect_ws(
        target: GwConnectTarget,
        client_name: &str,
        server_port: u16,
        transport: GatewayTransport,
        consent_callback: Option<&mut GwConsentCallback<'_>>,
    ) -> Result<GwClient, Error> {
        Self::connect_ws_with_reauth(
            target,
            client_name,
            server_port,
            transport,
            Box::new(|| {
                Box::pin(core::future::ready(Err(Error::new(
                    "gateway reauthentication requires a network transport",
                    GwErrorKind::UnsupportedFeature,
                ))))
            }),
            consent_callback,
        )
        .await
    }

    async fn connect_ws_with_reauth(
        target: GwConnectTarget,
        client_name: &str,
        server_port: u16,
        transport: GatewayTransport,
        mut open_transport: TransportFactory,
        consent_callback: Option<&mut GwConsentCallback<'_>>,
    ) -> Result<GwClient, Error> {
        let mut gw = GwConn {
            client_name: client_name.to_owned(),
            target,
            server_port,
            io: transport.io,
        };

        let session_authentication = gw.handshake(transport.session_authentication).await?;
        let reauthentication_consent_fallback = if consent_callback.is_some() {
            ConsentFallback::Reject
        } else {
            ConsentFallback::Accept
        };
        gw.tunnel(None, consent_callback, ConsentFallback::Accept).await?;
        let policy = gw.tunnel_auth().await?;
        gw.channel().await?;

        let (in_tx, in_rx) = tokio::sync::mpsc::channel(4);
        let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Bytes>(4);

        let work = tokio::spawn(async move {
            let iv = Duration::from_secs(15 * 60);
            let mut keepalive_interval = tokio::time::interval_at(tokio::time::Instant::now() + iv, iv);
            let mut inbound_open = true;
            let mut reauthentication: Option<ReauthenticationFuture> = None;

            loop {
                let mut wsbuf = [0u8; 8192];

                tokio::select!(
                    result = async {
                        match &mut reauthentication {
                            Some(reauthentication) => Some(reauthentication.await),
                            None => core::future::pending().await,
                        }
                    } => {
                        reauthentication = None;
                        match result.expect("reauthentication future is present") {
                            Ok(()) => warn!("RD Gateway reauthentication completed"),
                            Err(error) => warn!("RD Gateway reauthentication failed: {error}"),
                        }
                    },
                    _ = keepalive_interval.tick() => {
                        let pos = {
                            let mut cur = WriteCursor::new(&mut wsbuf);
                            KeepalivePkt.encode(&mut cur).map_err(|e| custom_err!("PktEncode", e))?;
                            cur.pos()
                        };

                        gw.io.send_bytes(&wsbuf[..pos]).await?;
                    },
                    next = gw.io.read_packet_buf() => {
                        // A clean close or an exhausted stream ends the work task with
                        // `Ok`, so readers observe end-of-stream rather than an error.
                        let Some(msg) = next? else {
                            return Ok(());
                        };
                        let mut cur = ReadCursor::new(&msg);
                        let hdr = PktHdr::decode(&mut cur).map_err(|e| custom_err!("Header Decode", e))?;

                        let header_length = usize::try_from(hdr.length).map_err(|_| Error::new("PktHdr too big", GwErrorKind::Decode))?;
                        assert!(cur.len() >= header_length - hdr.size());
                        match hdr.ty {
                            PktTy::Keepalive => {
                                continue;
                            },
                            PktTy::Data => {
                                let p = HttpDataPkt::decode(&mut cur).map_err(|e| custom_err!("PktDecode", e))?;
                                if inbound_open && in_tx.send(Bytes::from(p.data.to_vec())).await.is_err() {
                                    // Reader gone or shutdown closed the inbound channel.
                                    // Keep draining outbound bytes before closing the WebSocket.
                                    inbound_open = false;
                                }
                            },
                            PktTy::ServiceMessage => {
                                let msg = ServiceMessagePkt::decode(&mut cur).map_err(|e| custom_err!("PktDecode", e))?;
                                warn!("RD Gateway service message: {}", msg.message);
                            },
                            PktTy::ReauthMessage => {
                                let msg = ReauthMessagePkt::decode(&mut cur).map_err(|e| custom_err!("PktDecode", e))?;
                                if reauthentication.is_some() {
                                    return Err(Error::new("gateway reauthentication already pending", GwErrorKind::Connect));
                                }

                                let client_name = gw.client_name.clone();
                                let target = gw.target.clone();
                                let server_port = gw.server_port;
                                let reauth_tunnel_context = msg.reauth_tunnel_context;
                                let transport = open_transport();
                                reauthentication = Some(Box::pin(async move {
                                    let transport = transport.await?;
                                    let session_authentication = transport.session_authentication;
                                    let mut secondary = GwConn {
                                        client_name,
                                        target,
                                        server_port,
                                        io: transport.io,
                                    };
                                    secondary.handshake(session_authentication).await?;
                                    secondary
                                        .tunnel(Some(reauth_tunnel_context), None, reauthentication_consent_fallback)
                                        .await?;
                                    secondary.tunnel_auth().await?;
                                    secondary.channel().await?;

                                    // A reauthentication channel only updates authorization state for the
                                    // original connection and never carries application data.
                                    Ok(())
                                }));
                                warn!(
                                    "RD Gateway requested reauthentication (context 0x{:016x})",
                                    msg.reauth_tunnel_context
                                );
                            },
                            PktTy::ChannelClose | PktTy::ChannelCloseResponse => {
                                let close = ChannelClosePkt::decode(&mut cur).map_err(|e| custom_err!("PktDecode", e))?;
                                match gateway_code_label(close.status_code) {
                                    Some(label) => warn!("RD Gateway closed the channel ({label})"),
                                    None => warn!("RD Gateway closed the channel (0x{:08x})", close.status_code),
                                }
                                if hdr.ty == PktTy::ChannelClose {
                                    let pos = {
                                        let mut wcur = WriteCursor::new(&mut wsbuf);
                                        ChannelClosePkt { status_code: 0 }
                                            .encode_as(PktTy::ChannelCloseResponse, &mut wcur)
                                            .map_err(|e| custom_err!("PktEncode", e))?;
                                        wcur.pos()
                                    };
                                    gw.io.send_bytes(&wsbuf[..pos]).await?;
                                }
                                return Ok(());
                            },
                            x => {
                                warn!("Unhandled gw packet type {x:?}");
                            }
                        }
                    },
                    next = out_rx.recv() => {
                        let Some(next) = next else {
                            // Local write-side close: finish the WebSocket so poll_shutdown can complete.
                            gw.io.close().await?;
                            return Ok(());
                        };
                        let pkt = HttpDataPkt { data: &next };

                        let pos = {
                            let mut cur = WriteCursor::new(&mut wsbuf);
                            pkt.encode(&mut cur).map_err(|e| custom_err!("PktEncode", e))?;
                            cur.pos()
                        };
                        gw.io.send_bytes(&wsbuf[..pos]).await?;
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
            session_authentication,
        })
    }

    /// Policy parameters reported by the gateway during tunnel authorization.
    pub fn tunnel_policy(&self) -> &GwTunnelPolicy {
        &self.policy
    }

    /// Authentication selected for the initial gateway connection.
    pub fn session_authentication(&self) -> GwSessionAuthentication {
        self.session_authentication
    }
}

impl GwConn {
    async fn send_packet<E: Encode>(&mut self, payload: &E) -> Result<(), Error> {
        let mut buf = vec![0; payload.size()];
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
        let mut msg = self
            .io
            .read_packet_buf()
            .await?
            .ok_or_else(|| Error::new("Stream closed", GwErrorKind::Connect))?;
        let mut cur = ReadCursor::new(&msg);

        let hdr = PktHdr::decode(&mut cur).map_err(|_| Error::new("PktHdr", GwErrorKind::Decode))?;

        let header_length =
            usize::try_from(hdr.length).map_err(|_| Error::new("PktHdr too big", GwErrorKind::Decode))?;
        if cur.len() != header_length - hdr.size() {
            return Err(Error::new("read_packet", GwErrorKind::PacketEof));
        }

        Ok((hdr, msg.split_off(cur.pos())))
    }

    async fn handshake(
        &mut self,
        session_authentication: GwSessionAuthentication,
    ) -> Result<GwSessionAuthentication, Error> {
        let hs = HandshakeReqPkt {
            ver_major: 1,
            ver_minor: 0,
            extended_auth: match session_authentication {
                GwSessionAuthentication::Http => HttpExtendedAuth::HTTP_EXTENDED_AUTH_NONE,
                GwSessionAuthentication::NtlmSspi => HttpExtendedAuth::HTTP_EXTENDED_AUTH_SSPI_NTLM,
            },
            ..HandshakeReqPkt::default()
        };
        self.send_packet(&hs).await?;
        let (hdr, bytes) = self.read_packet().await?;
        if hdr.ty != PktTy::HandshakeResp {
            return Err(Error::new("Handshake", GwErrorKind::Decode));
        }

        let mut cur = ReadCursor::new(&bytes);
        let resp = HandshakeRespPkt::decode(&mut cur).map_err(|_| Error::new("Handshake", GwErrorKind::Decode))?;
        if resp.error_code != 0 {
            return Err(Error::new("Handshake", GwErrorKind::GatewayCode(resp.error_code)));
        }
        if resp.ver_major != 1 || resp.ver_minor != 0 || resp.server_version != 0 {
            return Err(Error::new("Handshake", GwErrorKind::Connect));
        }
        if !cur.eof() {
            return Err(Error::new("Handshake", GwErrorKind::Decode));
        }

        match session_authentication {
            GwSessionAuthentication::NtlmSspi
                if !resp
                    .extended_auth
                    .contains(HttpExtendedAuth::HTTP_EXTENDED_AUTH_SSPI_NTLM) =>
            {
                return Err(Error::new(
                    "Handshake",
                    GwErrorKind::UnsupportedExtendedAuthentication(GwExtendedAuthentication::NtlmSspi),
                ));
            }
            GwSessionAuthentication::NtlmSspi => self.extended_auth_sspi_ntlm().await?,
            GwSessionAuthentication::Http => {}
        }

        Ok(session_authentication)
    }

    async fn extended_auth_sspi_ntlm(&mut self) -> Result<(), Error> {
        let mut auth = http_auth::GatewayHttpAuth::new_extended_auth_ntlm(&self.target.gw_user, &self.target.gw_pass)?;
        let (token, mut complete) = auth.step_extended_auth(None)?;
        self.send_packet(&ExtendedAuthPkt {
            error_code: 0,
            auth_blob: token,
        })
        .await?;

        loop {
            let (hdr, bytes) = self.read_packet().await?;
            if hdr.ty != PktTy::ExtendedAuth {
                return Err(Error::new("extended authentication response", GwErrorKind::Decode));
            }

            let mut cur = ReadCursor::new(&bytes);
            let response = ExtendedAuthPkt::decode(&mut cur)
                .map_err(|_| Error::new("extended authentication", GwErrorKind::Decode))?;
            if !cur.eof() {
                return Err(Error::new("extended authentication", GwErrorKind::Decode));
            }
            if response.error_code != 0 {
                return Err(Error::new(
                    "extended authentication",
                    GwErrorKind::GatewayCode(response.error_code),
                ));
            }
            if complete {
                if response.auth_blob.is_empty() {
                    return Ok(());
                }
                return Err(Error::new("extended authentication", GwErrorKind::Connect));
            }
            if response.auth_blob.is_empty() {
                return Err(Error::new("extended authentication", GwErrorKind::Connect));
            }

            let (token, is_complete) = auth.step_extended_auth(Some(&response.auth_blob))?;
            if token.is_empty() {
                return Err(Error::new("extended authentication", GwErrorKind::Connect));
            }
            self.send_packet(&ExtendedAuthPkt {
                error_code: 0,
                auth_blob: token,
            })
            .await?;
            complete = is_complete;
        }
    }

    async fn tunnel(
        &mut self,
        reauth_tunnel_context: Option<u64>,
        consent_callback: Option<&mut GwConsentCallback<'_>>,
        default_consent: ConsentFallback,
    ) -> Result<(), Error> {
        let req = TunnelReqPkt {
            // No observed server works without this capability.
            caps: HttpCapsTy::MessagingConsentSign.as_u32() | HttpCapsTy::Reauth.as_u32(),
            fields_present: 0,
            reauth_tunnel_context,
            ..TunnelReqPkt::default()
        };
        self.send_packet(&req).await?;

        let (_hdr, bytes) = self.read_packet().await?;
        let mut cur = ReadCursor::new(&bytes);

        let resp = TunnelRespPkt::decode(&mut cur).map_err(|_| Error::new("TunnelDecode", GwErrorKind::Decode))?;
        if resp.status_code != 0 {
            return Err(Error::new("Tunnel", GwErrorKind::GatewayCode(resp.status_code)));
        }
        assert!(cur.eof());
        evaluate_consent_message(&resp.consent_msg, consent_callback, default_consent)?;
        Ok(())
    }

    async fn tunnel_auth(&mut self) -> Result<GwTunnelPolicy, Error> {
        let req = TunnelAuthPkt {
            fields_present: 0,
            client_name: self.client_name.clone(),
        };
        self.send_packet(&req).await?;

        let (_hdr, bytes) = self.read_packet().await?;
        let mut cur = ReadCursor::new(&bytes);
        let resp: TunnelAuthRespPkt =
            TunnelAuthRespPkt::decode(&mut cur).map_err(|_| Error::new("TunnelAuth", GwErrorKind::Decode))?;

        if resp.error_code != 0 {
            return Err(Error::new("TunnelAuth", GwErrorKind::GatewayCode(resp.error_code)));
        }
        Ok(GwTunnelPolicy {
            redirection_flags: resp.redirection_flags,
            idle_timeout_minutes: resp.idle_timeout_minutes,
            soh_response: resp.soh_response,
        })
    }

    async fn channel(&mut self) -> Result<ChannelResp, Error> {
        let req = ChannelPkt {
            resources: vec![self.target.server.clone()],
            port: self.server_port,
            protocol: 3,
        };
        self.send_packet(&req).await?;

        let (hdr, bytes) = self.read_packet().await?;
        assert!(hdr.ty == PktTy::ChannelResp);
        let mut cur: ReadCursor<'_> = ReadCursor::new(&bytes);
        let resp: ChannelResp =
            ChannelResp::decode(&mut cur).map_err(|_| Error::new("ChannelResp", GwErrorKind::Decode))?;
        if resp.error_code() != 0 {
            return Err(Error::new("ChannelCreate", GwErrorKind::GatewayCode(resp.error_code())));
        }
        assert!(cur.eof());
        Ok(resp)
    }
}

fn decode_consent_message(bytes: &[u8]) -> Result<String, Error> {
    if !bytes.len().is_multiple_of(2) {
        return Err(Error::new("TunnelConsent", GwErrorKind::Decode));
    }

    let code_units = bytes
        .chunks_exact(2)
        .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
        .collect::<Vec<_>>();
    let message = String::from_utf16(&code_units).map_err(|_| Error::new("TunnelConsent", GwErrorKind::Decode))?;
    Ok(message.strip_suffix('\0').unwrap_or(&message).to_owned())
}

fn evaluate_consent_message(
    consent_message: &[u8],
    consent_callback: Option<&mut GwConsentCallback<'_>>,
    default_consent: ConsentFallback,
) -> Result<(), Error> {
    if consent_message.is_empty() {
        return Ok(());
    }

    let message = decode_consent_message(consent_message)?;
    if let Some(consent_callback) = consent_callback {
        if consent_callback(&message) {
            return Ok(());
        }
        return Err(Error::new("TunnelConsent", GwErrorKind::ConsentDeclined));
    }

    if matches!(default_consent, ConsentFallback::Reject) {
        Err(Error::new("TunnelConsent", GwErrorKind::ConsentDeclined))
    } else {
        Ok(())
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
        // Closing the outbound sender ends the write queue. Closing the inbound
        // receiver unblocks a worker parked on `in_tx.send` when the caller is not
        // reading, so the work task can close the WebSocket and finish.
        self.tx.close();
        self.rx.close();
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
