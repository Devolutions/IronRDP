#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

#[macro_use]
mod macros;

#[doc(hidden)]
pub mod http_auth;
mod proto;
#[doc(hidden)]
pub mod rpc;
mod udp;

use core::fmt;
use core::fmt::Display;
use core::pin::Pin;
use core::task::Poll;
use core::time::Duration;
use std::io;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{FutureExt as _, SinkExt as _, StreamExt as _};
use http_body_util::BodyExt as _;
use hyper::body::Bytes;
use ironrdp_core::{Decode as _, Encode, ReadCursor, WriteCursor};
use ironrdp_tls::TlsStream;
use log::{error, warn};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::{Message, http};
use tokio_util::sync::PollSender;

use self::http_auth::{AuthStep, GatewayHttpAuth, basic_authorization, www_authenticate_values};
#[doc(hidden)]
pub use self::proto::{ChannelClosePkt, ReauthMessagePkt, ServiceMessagePkt, gateway_code_label};
use self::proto::{
    ChannelPkt, ChannelResp, DataPkt as HttpDataPkt, HandshakeReqPkt, HandshakeRespPkt, HttpCapsTy, KeepalivePkt,
    PktHdr, PktTy, TunnelAuthPkt, TunnelAuthRespPkt, TunnelReqPkt, TunnelRespPkt,
};
pub use self::udp::{
    AaSynData, AaSynDataResp, ConnectPkt, ConnectPktResp, DataPkt, DiscPkt, GwUdpOffer, MAX_CONNECT_REQ_FRAGMENT_SIZE,
    UdpCorrelationInfo, UdpPacketHeader, UdpPktType, encode_connect_request, fragment_connect_pkt,
};

#[derive(Clone, Debug)]
pub struct GwConnectTarget {
    pub gw_endpoint: String,
    pub gw_user: String,
    pub gw_pass: String,

    pub server: String,
}

type Error = ironrdp_error::Error<GwErrorKind>;

#[derive(Debug)]
#[non_exhaustive]
pub enum GwErrorKind {
    InvalidGwTarget,
    Connect,
    PacketEof,
    UnsupportedFeature,
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
            GwErrorKind::Connect => "connection error",
            GwErrorKind::PacketEof => "PacketEOF",
            GwErrorKind::UnsupportedFeature => "unsupported feature",
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
    ws_sink: SplitSink<WebSocketStream<TlsStream<TcpStream>>, Message>,
    ws_stream: SplitStream<WebSocketStream<TlsStream<TcpStream>>>,
}

pub struct GwClient {
    work: tokio::task::JoinHandle<Result<(), Error>>,
    /// Set once the work task has completed; its `JoinHandle` must not be polled again
    /// (tokio panics if a completed `JoinHandle` is polled).
    work_done: bool,
    rx: tokio::sync::mpsc::Receiver<Bytes>,
    rx_bufs: Vec<Bytes>,
    tx: PollSender<Bytes>,
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
        Self::connect_with_port(target, client_name, 3389).await
    }

    /// Open an MS-TSGU tunnel, presenting `server_port` as HTTP_CHANNEL_PACKET `port`.
    pub async fn connect_with_port(
        target: &GwConnectTarget,
        client_name: &str,
        server_port: u16,
    ) -> Result<(GwClient, core::net::SocketAddr), Error> {
        let gw_host = target
            .gw_endpoint
            .split(":")
            .nth(0)
            .ok_or_else(|| Error::new("Connect", GwErrorKind::InvalidGwTarget))?;

        let stream = TcpStream::connect(&target.gw_endpoint)
            .await
            .map_err(|e| custom_err!("TCP connect", e))?;
        let client_addr = stream
            .local_addr()
            .map_err(|e| custom_err!("get socket local address", e))?;

        let (stream, _) = ironrdp_tls::upgrade(stream, gw_host)
            .await
            .map_err(|e| custom_err!("TLS connect", e))?;

        let connection_id = format!("{{{}}}", uuid::Uuid::new_v4());
        let spn = format!("HTTP/{gw_host}");

        let stream = hyper_util::rt::tokio::TokioIo::new(stream);
        let (mut sender, mut conn) = hyper::client::conn::http1::handshake(stream)
            .await
            .map_err(|e| custom_err!("H1 Handshake", e))?;
        let (tx, rx) = oneshot::channel();

        let jh = tokio::task::spawn(async move {
            tokio::select! {
                Err(e) = &mut conn => error!("Handshake error: {:?}", e),
                _ = rx => (),
            }
            conn.into_parts()
        });
        websocket_upgrade_with_auth(&mut sender, gw_host, &connection_id, target, &spn).await?;

        let _ = tx.send(()); // TODO: Not needed since it doesnt keep alive conn?
        let stream = jh.await.map_err(|e| custom_err!("WS join", e))?.io.into_inner();

        Self::connect_ws(target.clone(), client_name, server_port, stream)
            .await
            .map(|x| (x, client_addr))
    }

    async fn connect_ws(
        target: GwConnectTarget,
        client_name: &str,
        server_port: u16,
        tls_stream: TlsStream<TcpStream>,
    ) -> Result<GwClient, Error> {
        let ws_stream: WebSocketStream<_> = WebSocketStream::from_raw_socket(tls_stream, Role::Client, None).await;
        let (ws_sink, ws_stream) = ws_stream.split();
        let mut gw = GwConn {
            client_name: client_name.to_owned(),
            target,
            server_port,
            ws_sink,
            ws_stream,
        };

        gw.handshake().await?;
        gw.tunnel().await?;
        gw.tunnel_auth().await?;
        gw.channel().await?;

        let (in_tx, in_rx) = tokio::sync::mpsc::channel(4);
        let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Bytes>(4);

        let work = tokio::spawn(async move {
            let iv = Duration::from_secs(15 * 60);
            let mut keepalive_interval = tokio::time::interval_at(tokio::time::Instant::now() + iv, iv);

            loop {
                let mut wsbuf = [0u8; 8192];

                tokio::select!(
                    _ = keepalive_interval.tick() => {
                        let pos = {
                            let mut cur = WriteCursor::new(&mut wsbuf);
                            KeepalivePkt.encode(&mut cur).map_err(|e| custom_err!("PktEncode", e))?;
                            cur.pos()
                        };

                        gw.ws_sink.send(Message::Binary(Bytes::copy_from_slice(&wsbuf[..pos]))).await.map_err(|e| custom_err!("ws send", e))?;
                    },
                    next = gw.ws_stream.next() => {
                        let msg = match next {
                            // A clean close or an exhausted stream ends the work task with
                            // `Ok`, so readers observe end-of-stream rather than an error.
                            None => return Ok(()),
                            Some(Ok(msg)) => msg,
                            Some(Err(e)) => return Err(custom_err!("Stream", e)),
                        };
                        if matches!(msg, Message::Close(_)) {
                            return Ok(());
                        }
                        let msg = msg.into_data();
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
                                in_tx.send(Bytes::from(p.data.to_vec())).await.map_err(|e| custom_err!("in_tx dead", e))?;
                            },
                            PktTy::ServiceMessage => {
                                let msg = ServiceMessagePkt::decode(&mut cur).map_err(|e| custom_err!("PktDecode", e))?;
                                warn!("RD Gateway service message: {}", msg.message);
                            },
                            PktTy::ReauthMessage => {
                                let msg = ReauthMessagePkt::decode(&mut cur).map_err(|e| custom_err!("PktDecode", e))?;
                                warn!(
                                    "RD Gateway requested reauthentication (context 0x{:016x}); mid-session reauth is not performed",
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
                                    gw.ws_sink
                                        .send(Message::Binary(Bytes::copy_from_slice(&wsbuf[..pos])))
                                        .await
                                        .map_err(|e| custom_err!("ws send", e))?;
                                }
                                return Ok(());
                            },
                            x => {
                                warn!("Unhandled gw packet type {x:?}");
                            }
                        }
                    },
                    next = out_rx.recv() => {
                        let next = next.ok_or_else(|| Error::new("WS Sink Dead", GwErrorKind::Connect))?;
                        let pkt = HttpDataPkt { data: &next };

                        let pos = {
                            let mut cur = WriteCursor::new(&mut wsbuf);
                            pkt.encode(&mut cur).map_err(|e| custom_err!("PktEncode", e))?;
                            cur.pos()
                        };
                        gw.ws_sink.send(Message::Binary(Bytes::copy_from_slice(&wsbuf[..pos]))).await.map_err(|e| custom_err!("ws send", e))?;
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
        })
    }
}

/// Challenge-first WebSocket upgrade: omit Authorization until 401, then Negotiate/NTLM/Basic.
async fn websocket_upgrade_with_auth(
    sender: &mut hyper::client::conn::http1::SendRequest<http_body_util::Empty<Bytes>>,
    gw_host: &str,
    connection_id: &str,
    target: &GwConnectTarget,
    spn: &str,
) -> Result<(), Error> {
    let mut http_auth: Option<GatewayHttpAuth> = None;
    let mut authorization: Option<String> = None;
    let mut use_basic = false;
    const MAX_AUTH_ROUNDS: usize = 8;

    for _ in 0..MAX_AUTH_ROUNDS {
        let req = build_ws_upgrade_request(
            gw_host,
            connection_id,
            if use_basic {
                Some(basic_authorization(&target.gw_user, &target.gw_pass))
            } else {
                authorization.clone()
            }
            .as_deref(),
        )?;

        let resp = sender
            .send_request(req)
            .await
            .map_err(|e| custom_err!("WS Upgrade Send error", e))?;

        if resp.status() == http::StatusCode::SWITCHING_PROTOCOLS {
            if let Some(mut auth) = http_auth.take() {
                let challenges: Vec<String> = www_authenticate_values(resp.headers())
                    .into_iter()
                    .map(str::to_owned)
                    .collect();
                run_http_auth(move || auth.finish_www_authenticate(challenges.iter().map(String::as_str))).await?;
            }
            return Ok(());
        }

        if resp.status() != http::StatusCode::UNAUTHORIZED {
            return Err(Error::new("WS Upgrade", GwErrorKind::Connect));
        }

        if use_basic {
            return Err(Error::new("websocket upgrade basic auth", GwErrorKind::Connect));
        }

        let challenges: Vec<String> = www_authenticate_values(resp.headers())
            .into_iter()
            .map(str::to_owned)
            .collect();
        resp.into_body()
            .collect()
            .await
            .map_err(|e| custom_err!("drain websocket upgrade auth body", e))?;

        let user = target.gw_user.clone();
        let pass = target.gw_pass.clone();
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
                GatewayHttpAuth::from_challenges(&user, &pass, Some(target_name), &refs)
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
                    "websocket upgrade auth complete without switching protocols",
                    GwErrorKind::Connect,
                ));
            }
        }
    }

    Err(Error::new(
        "websocket upgrade auth rounds exceeded",
        GwErrorKind::Connect,
    ))
}

async fn run_http_auth<T, F>(f: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, Error> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::custom("http auth task", e))?
}

fn build_ws_upgrade_request(
    gw_host: &str,
    connection_id: &str,
    authorization: Option<&str>,
) -> Result<http::Request<http_body_util::Empty<Bytes>>, Error> {
    let mut req = http::Request::builder()
        .method("RDG_OUT_DATA")
        .header(hyper::header::HOST, gw_host)
        .header("Rdg-Connection-Id", connection_id)
        .uri("/remoteDesktopGateway/")
        .header(hyper::header::CONNECTION, "Upgrade")
        .header(hyper::header::UPGRADE, "websocket")
        .header(hyper::header::SEC_WEBSOCKET_VERSION, "13")
        .header(hyper::header::SEC_WEBSOCKET_KEY, generate_key());

    if let Some(authorization) = authorization {
        req = req.header(hyper::header::AUTHORIZATION, authorization);
    }

    req.body(http_body_util::Empty::<Bytes>::new())
        .map_err(|e| custom_err!("failed to build request", e))
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
        self.ws_sink
            .send(Message::Binary(Bytes::copy_from_slice(&buf[..pos])))
            .await
            .map_err(|e| custom_err!("WebSocket send error", e))?;
        Ok(())
    }

    async fn read_packet(&mut self) -> Result<(PktHdr, Bytes), Error> {
        let mut msg = self
            .ws_stream
            .next()
            .await
            .ok_or_else(|| Error::new("Stream closed", GwErrorKind::Connect))?
            .map_err(|e| custom_err!("WS err", e))?
            .into_data();
        let mut cur = ReadCursor::new(&msg);

        let hdr = PktHdr::decode(&mut cur).map_err(|_| Error::new("PktHdr", GwErrorKind::Decode))?;

        let header_length =
            usize::try_from(hdr.length).map_err(|_| Error::new("PktHdr too big", GwErrorKind::Decode))?;
        if cur.len() != header_length - hdr.size() {
            return Err(Error::new("read_packet", GwErrorKind::PacketEof));
        }

        Ok((hdr, msg.split_off(cur.pos())))
    }

    async fn handshake(&mut self) -> Result<(), Error> {
        // For NTLM we would include extended_auth: NTLM_SSPI in this handshake req here.
        let hs = HandshakeReqPkt {
            ver_major: 1,
            ver_minor: 0,
            ..HandshakeReqPkt::default()
        };
        self.send_packet(&hs).await?;
        let (_hdr, bytes) = self.read_packet().await?;

        let mut cur = ReadCursor::new(&bytes);
        let resp = HandshakeRespPkt::decode(&mut cur).map_err(|_| Error::new("Handshake", GwErrorKind::Decode))?;
        if resp.error_code != 0 || resp.ver_major != 1 || resp.ver_minor != 0 || resp.server_version != 0 {
            return Err(Error::new("Handshake", GwErrorKind::Connect));
        }
        Ok(())
    }

    async fn tunnel(&mut self) -> Result<(), Error> {
        let req = TunnelReqPkt {
            // Havent seen any server working without this.
            caps: HttpCapsTy::MessagingConsentSign.as_u32(),
            fields_present: 0,
            ..TunnelReqPkt::default()
        };
        self.send_packet(&req).await?;

        let (_hdr, bytes) = self.read_packet().await?;
        let mut cur = ReadCursor::new(&bytes);

        let resp = TunnelRespPkt::decode(&mut cur).map_err(|_| Error::new("TunnelDecode", GwErrorKind::Decode))?;
        if resp.status_code != 0 {
            return Err(Error::new("Tunnel", GwErrorKind::Connect));
        }
        assert!(cur.eof());
        if !resp.consent_msg.is_empty() {
            return Err(Error::new(
                "Received consent message but showing it not implemented",
                GwErrorKind::UnsupportedFeature,
            ));
        }
        Ok(())
    }

    async fn tunnel_auth(&mut self) -> Result<(), Error> {
        let req = TunnelAuthPkt {
            fields_present: 0,
            client_name: self.client_name.clone(),
        };
        self.send_packet(&req).await?;

        let (_hdr, bytes) = self.read_packet().await?;
        let mut cur = ReadCursor::new(&bytes);
        let resp: TunnelAuthRespPkt =
            TunnelAuthRespPkt::decode(&mut cur).map_err(|_| Error::new("TunnelAuth", GwErrorKind::Decode))?;

        if resp.error_code() != 0 {
            return Err(Error::new("TunnelAuth", GwErrorKind::Connect));
        }
        Ok(())
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
            return Err(Error::new("ChannelCreate", GwErrorKind::Connect));
        }
        assert!(cur.eof());
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

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut core::task::Context<'_>) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }
}
