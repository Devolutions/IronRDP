#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

#[macro_use]
mod macros;

#[doc(hidden)]
pub mod http_auth;
mod packet_io;
mod proto;
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
use log::warn;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::sync::PollSender;

use self::packet_io::{PacketIo, open_websocket_transport};
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
    io: PacketIo,
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
        let (io, client_addr) = open_websocket_transport(target).await?;
        Self::connect_ws(target.clone(), client_name, server_port, io)
            .await
            .map(|x| (x, client_addr))
    }

    async fn connect_ws(
        target: GwConnectTarget,
        client_name: &str,
        server_port: u16,
        io: PacketIo,
    ) -> Result<GwClient, Error> {
        let mut gw = GwConn {
            client_name: client_name.to_owned(),
            target,
            server_port,
            io,
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

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> Poll<Result<(), io::Error>> {
        // Closing the sender ends the outbound queue; the work task then closes the
        // WebSocket so a local write-side EOF can finish.
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
