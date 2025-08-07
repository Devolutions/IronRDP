//! [MS-TSGU] https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
//! * This implements a MVP (in terms of recentness) state needed to connect through microsoft rdp gateway.
//! * This only supports the HTTPS protocol with Websocket (and not the legacy HTTP, HTTP-RPC or UDP protocols).
//! * This does not implement reconnection/reauthentication.
//! * This only supports basic auth.
use core::pin::Pin;
use core::time::Duration;
use core::{fmt, fmt::Display, task::Poll};
use std::io;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures_util::{
    stream::{SplitSink, SplitStream},
    FutureExt as _, SinkExt as _, StreamExt as _,
};
use hyper::body::Bytes;
use ironrdp_core::{Decode as _, Encode, ReadCursor, WriteCursor};
use ironrdp_tls::TlsStream;
use log::{error, warn};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    sync::oneshot,
};
use tokio_tungstenite::{
    tungstenite::{
        handshake::client::generate_key,
        http::{self},
        protocol::Role,
        Message,
    },
    WebSocketStream,
};

mod proto;
use proto::*;
use tokio_util::sync::PollSender;

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
    PacketEOF,
    UnsupportedFeature,
    Custom,
    Decode,
}

trait GwErrorExt {
    fn custom<E>(context: &'static str, e: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static;
}

impl GwErrorExt for ironrdp_error::Error<GwErrorKind> {
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
            GwErrorKind::InvalidGwTarget => "Invalid GW Target",
            GwErrorKind::Connect => "Connection error",
            GwErrorKind::PacketEOF => "PacketEOF",
            GwErrorKind::UnsupportedFeature => "Unsupported feature",
            GwErrorKind::Custom => "Custom",
            GwErrorKind::Decode => "Decode",
        };
        f.write_str(x)
    }
}

impl core::error::Error for GwErrorKind {}

/// Creates a `ConnectorError` with `Custom` kind and a source error attached to it
#[macro_export]
macro_rules! custom_err {
    ( $context:expr, $source:expr $(,)? ) => {{
        <$crate::Error as $crate::GwErrorExt>::custom($context, $source)
    }};
}

struct GwConn {
    client_name: String,
    target: GwConnectTarget,
    ws_sink: SplitSink<WebSocketStream<TlsStream<TcpStream>>, Message>,
    ws_stream: SplitStream<WebSocketStream<TlsStream<TcpStream>>>,
}

pub struct GwClient {
    work: tokio::task::JoinHandle<Result<(), Error>>,
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
    pub async fn connect(
        target: &GwConnectTarget,
        client_name: &str,
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

        let ba = STANDARD.encode(format!("{}:{}", target.gw_user, target.gw_pass));
        let req = http::Request::builder()
            .method("RDG_OUT_DATA")
            .header(hyper::header::HOST, gw_host)
            .header("Rdg-Connection-Id", format!("{{{}}}", uuid::Uuid::new_v4()))
            .uri("/remoteDesktopGateway/")
            .header(hyper::header::AUTHORIZATION, format!("Basic {ba}"))
            .header(hyper::header::CONNECTION, "Upgrade")
            .header(hyper::header::UPGRADE, "websocket")
            .header(hyper::header::SEC_WEBSOCKET_VERSION, "13")
            .header(hyper::header::SEC_WEBSOCKET_KEY, generate_key())
            .body(http_body_util::Empty::<Bytes>::new())
            .expect("Failed to build request");

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
        let resp = sender
            .send_request(req)
            .await
            .map_err(|e| custom_err!("WS Upgrade Send error", e))?;

        if resp.status() != http::StatusCode::SWITCHING_PROTOCOLS {
            return Err(Error::new("WS Upgrade", GwErrorKind::Connect));
        }

        let _ = tx.send(()); // TODO: Not needed since it doesnt keep alive conn?
        let stream = jh.await.map_err(|e| custom_err!("WS join", e))?.io.into_inner();

        Self::connect_ws(target.clone(), client_name, stream)
            .await
            .map(|x| (x, client_addr))
    }

    async fn connect_ws(
        target: GwConnectTarget,
        client_name: &str,
        tls_stream: TlsStream<TcpStream>,
    ) -> Result<GwClient, Error> {
        let ws_stream: WebSocketStream<_> = WebSocketStream::from_raw_socket(tls_stream, Role::Client, None).await;
        let (ws_sink, ws_stream) = ws_stream.split();
        let mut gw = GwConn {
            client_name: client_name.to_owned(),
            target,
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
            let mut keepalive_interval: tokio::time::Interval =
                tokio::time::interval_at(tokio::time::Instant::now() + iv, iv);

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
                        let tmp = next.ok_or_else(|| Error::new("WS Stream Dead", GwErrorKind::Connect))?;
                        let msg = tmp.map_err(|e| custom_err!("Stream", e))?.into_data();
                        let mut cur = ReadCursor::new(&msg);
                        let hdr = PktHdr::decode(&mut cur).map_err(|e| custom_err!("Header Decode", e))?;

                        assert!(cur.len() >= hdr.length as usize - hdr.size());
                        match hdr.ty {
                            PktTy::Keepalive => {
                                continue;
                            },
                            PktTy::Data => {
                                let p = DataPkt::decode(&mut cur).map_err(|e| custom_err!("PktDecode", e))?;
                                in_tx.send(Bytes::from(p.data.to_vec())).await.map_err(|e| custom_err!("in_tx dead", e))?;
                            },
                            x => {
                                warn!("Unhandled gw packet type {x:?}");
                            }
                        }
                    },
                    next = out_rx.recv() => {
                        let next = next.ok_or_else(|| Error::new("WS Sink Dead", GwErrorKind::Connect))?;
                        let pkt = DataPkt { data: &next };

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
            rx: in_rx,
            rx_bufs: vec![],
            tx: PollSender::new(out_tx),
        })
    }
}

impl GwConn {
    async fn send_packet<E: Encode>(&mut self, payload: &E) -> Result<(), Error> {
        let mut buf = [0u8; 4096];
        let pos = {
            let mut cur = WriteCursor::new(&mut buf);
            payload.encode(&mut cur).unwrap();
            cur.pos()
        };
        self.ws_sink
            .send(Message::Binary(Bytes::copy_from_slice(&buf[..pos])))
            .await
            .map_err(|e| custom_err!("WS Send error", e))?;
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
        if cur.len() != hdr.length as usize - hdr.size() {
            return Err(Error::new("read_packet", GwErrorKind::PacketEOF));
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
            caps: HttpCapsTy::MessagingConsentSign as u32,
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

        if resp.error_code != 0 {
            return Err(Error::new("TunnelAuth", GwErrorKind::Connect));
        }
        Ok(())
    }

    async fn channel(&mut self) -> Result<ChannelResp, Error> {
        let req = ChannelPkt {
            resources: vec![self.target.server.clone()],
            port: 3389,
            protocol: 3,
        };
        self.send_packet(&req).await?;

        let (hdr, bytes) = self.read_packet().await?;
        assert!(hdr.ty == PktTy::ChannelResp);
        let mut cur: ReadCursor<'_> = ReadCursor::new(&bytes);
        let resp: ChannelResp =
            ChannelResp::decode(&mut cur).map_err(|_| Error::new("ChannelResp", GwErrorKind::Decode))?;
        if resp.error_code != 0 {
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
        match self.work.poll_unpin(cx) {
            Poll::Ready(Err(e)) => return Poll::Ready(Err(io::Error::other(e))),
            Poll::Ready(Ok(Err(e))) => return Poll::Ready(Err(io::Error::other(e))),
            Poll::Ready(_) => return Poll::Ready(Err(io::Error::other("Premature Work Task end?"))),
            _ => (),
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
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
}

impl AsyncWrite for GwClient {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        // Propagate error or premature exit (?)
        match self.work.poll_unpin(cx) {
            Poll::Ready(Err(e)) => return Poll::Ready(Err(io::Error::other(e))),
            Poll::Ready(Ok(Err(e))) => return Poll::Ready(Err(io::Error::other(e))),
            Poll::Ready(_) => return Poll::Ready(Err(io::Error::other("Premature Work Task end?"))),
            Poll::Pending => (),
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
