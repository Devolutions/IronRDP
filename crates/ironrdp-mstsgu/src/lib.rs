//! [MS-TSGU] https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
//! * This implements a MVP (in terms of recentness) state needed to connect through microsoft rdp gateway.
//! * This only supports the HTTPS protocol with Websocket (and not the legacy HTTP, HTTP-RPC or UDP protocols).
//! * This does implement reconnection/reauthentication.
//! * This only supports basic auth.
use std::{fmt::Display, task::Poll};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use hyper::body::Bytes;
use ironrdp_core::{Decode, Encode, ReadCursor, WriteCursor};
use tokio::{io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt}, net::TcpStream, sync::oneshot};
use tokio_native_tls::TlsStream;
use tokio_tungstenite::{tungstenite::{handshake::client::generate_key, http::{self}, protocol::Role, Message}, WebSocketStream};
use futures_util::{stream::{SplitSink, SplitStream}, SinkExt, StreamExt};

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
    Connect,
    PacketEOF,
    Custom
}

trait GwErrorExt {
    fn custom<E>(context: &'static str, e: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static;
}

impl GwErrorExt for ironrdp_error::Error<GwErrorKind> {
    fn custom<E>(context: &'static str, e: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static,
    {
        Self::new(context, GwErrorKind::Custom).with_source(e)
    }
}

impl Display for GwErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl std::error::Error for GwErrorKind {}

/// Creates a `ConnectorError` with `Custom` kind and a source error attached to it
#[macro_export]
macro_rules! custom_err {
    ( $context:expr, $source:expr $(,)? ) => {{
        <$crate::Error as $crate::GwErrorExt>::custom($context, $source)
    }};
}


struct GwConn {   
    target: GwConnectTarget,
    ws_sink: SplitSink<WebSocketStream<TlsStream<TcpStream>>, Message>,
    ws_stream: SplitStream<WebSocketStream<TlsStream<TcpStream>>>,
}

pub struct GwClient {
    rx: tokio::sync::mpsc::Receiver<Bytes>,
    rx_bufs: Vec<Bytes>,
    tx: PollSender<Bytes>,
}

impl GwClient {
    pub async fn connect(target: &GwConnectTarget) -> Result<TlsStream<TcpStream>, Error> {
        let gw_host = target.gw_endpoint.split(":").nth(0).unwrap();

        let stream = TcpStream::connect(&target.gw_endpoint)
            .await
            .map_err(|e| custom_err!("TCP connect", e))?;
        let stream = tokio_native_tls::TlsConnector::from(tokio_native_tls::native_tls::TlsConnector::new().unwrap())
            .connect(gw_host, stream)
            .await
            .map_err(|e| custom_err!("TLS connect", e))?;

        let ba = STANDARD.encode(format!("{}:{}", target.gw_user, target.gw_pass));
        let req = http::Request::builder()
            .method("RDG_OUT_DATA")
            .header(hyper::header::HOST, gw_host)
            .header("Rdg-Connection-Id", format!("{{{}}}", uuid::Uuid::new_v4()))
            .uri("/remoteDesktopGateway/")
            .header(hyper::header::AUTHORIZATION, format!("Basic {}", ba))
            
            .header(hyper::header::CONNECTION, "Upgrade")
            .header(hyper::header::UPGRADE, "websocket")
            .header(hyper::header::SEC_WEBSOCKET_VERSION, "13")
            .header(hyper::header::SEC_WEBSOCKET_KEY, generate_key())
            .body(http_body_util::Empty::<Bytes>::new())
            .expect("Failed to build request");
    
        println!("SENDING");
        let stream = hyper_util::rt::tokio::TokioIo::new(stream);
        let (mut sender, mut conn) = hyper::client::conn::http1::handshake(stream).await.expect("Handshake failed");
        let (tx, rx) = oneshot::channel();
    
        let res = tokio::task::spawn(async move {
            tokio::select! {
                v = &mut conn => println!("Connection failed: {:?}", v),
                _ = rx => println!("RX"),
            }
            return conn.into_parts()
        });
        let resp = sender.send_request(req)
            .await
            .map_err(|e| custom_err!("WS Upgrade Send error", e))?;;
        println!("RESP: {:?}", resp.status());
        
        // assert_eq!(resp.status(), http::StatusCode::SWITCHING_PROTOCOLS);
        if resp.status() != http::StatusCode::SWITCHING_PROTOCOLS {
            return Err(Error::new("WS Upgrade", GwErrorKind::Connect));
        }
    
        let _ = tx.send(()); // TODO: Not needed since it doesnt keep alive conn?
        let stream = res.await.map_err(|e| custom_err!("WS join unwrap", e))?.io.into_inner();
        Ok(stream)
    }

    pub async fn connect_ws(target: GwConnectTarget, tls_stream: TlsStream<TcpStream>) -> Result<GwClient, Error> {
        let ws_stream: WebSocketStream<_> = WebSocketStream::from_raw_socket(tls_stream, Role::Client, None).await;
        let (ws_sink, ws_stream) = ws_stream.split();
        let mut gw = GwConn { target, ws_sink, ws_stream };
        
        gw.handshake().await?;
        gw.tunnel().await?;
        gw.tunnel_auth().await?;
        gw.channel().await?;

        let (in_tx, in_rx) = tokio::sync::mpsc::channel(4);
        let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Bytes>(4);

        tokio::spawn(async move {
            loop {
                let mut wsbuf = [0u8; 8192];
                
                tokio::select!(
                    next = gw.ws_stream.next() => {
                        let msg = next.expect("WS Stream END/DEAD TODO").unwrap().into_data();
                        println!("recv through websocket {}", msg.len());
                        let mut cur = ReadCursor::new(&msg);
                        let hdr = PktHdr::decode(&mut cur).unwrap();
                        assert!(cur.len() >= hdr.length as usize - hdr.size()); // TODO
                        println!("{:?}", msg);
                        assert_eq!(hdr.ty, PktTy::Data); // TODO
                        let p = DataPkt::decode(&mut cur).unwrap();
                        in_tx.send(Bytes::from(p.data.to_vec())).await.unwrap();
                    },
                    next = out_rx.recv() => {
                        let next = next.expect("WS Sink END/DEAD TODO");
                        let pkt = DataPkt { data: &next };
                        println!("send through websocket {}", next.len());
                
                        let pos = {
                            let mut cur = WriteCursor::new(&mut wsbuf);
                            pkt.encode(&mut cur).unwrap();
                            cur.pos() as usize
                        };
                        gw.ws_sink.send(Message::Binary(Bytes::copy_from_slice(&wsbuf[..pos]))).await.unwrap();
                    }
                );
            }
        });

        Ok(GwClient { rx: in_rx, rx_bufs: vec![], tx: PollSender::new(out_tx) })
    }
}

impl GwConn {
    async fn send_packet<'a, E: Encode>(&mut self, payload: &E) -> Result<(), Error> {
        let mut buf = [0u8; 4096];
        let pos = {
            let mut cur = WriteCursor::new(&mut buf);
            payload.encode(&mut cur).unwrap();
            cur.pos() as usize
        };
        self.ws_sink.send(Message::Binary(Bytes::copy_from_slice(&buf[..pos])))
            .await
            .map_err(|e| custom_err!("WS Send error", e))?;
        Ok(())
    }

    async fn read_packet(&mut self) -> Result<(PktHdr, Bytes), Error> {
        let mut msg = self.ws_stream.next().await.unwrap().unwrap().into_data();
        let mut cur = ReadCursor::new(&msg);
        let hdr = PktHdr::decode(&mut cur).unwrap();
        
        // assert!(cur.len() >= hdr.length as usize - hdr.size());
        if cur.len() != hdr.length as usize - hdr.size() {
            return Err(Error::new("read_packet", GwErrorKind::PacketEOF));
        }

        Ok((hdr, msg.split_off(cur.pos())))
    }

    async fn handshake(&mut self) -> Result<(), Error> {
        let hs = HandshakeReqPkt {
            ver_major: 1,
            ver_minor: 0,
            ..HandshakeReqPkt::default()
        };
        self.send_packet(&hs).await?;
        let (hdr, bytes) = self.read_packet().await?;

        let mut cur = ReadCursor::new(&bytes);
        let resp = HandshakeRespPkt::decode(&mut cur).unwrap();
        println!("HANDSHAKE RESP: {:?}", resp);
        if resp.error_code != 0 {
            return Err(Error::new("Handshake", GwErrorKind::Connect));
        }

        assert_eq!(resp.extended_auth, 7); //TODO....
        Ok(())
    }

    async fn tunnel(&mut self) -> Result<(), Error> {
        const HTTP_CAPABILITY_MESSAGING_CONSENT_SIGN: u32 = 0x4;
        let req = TunnelReqPkt {
            caps: HTTP_CAPABILITY_MESSAGING_CONSENT_SIGN,
            fields_present: 0,
            ..TunnelReqPkt::default()
        };
        self.send_packet(&req).await?;

        let (hdr, bytes) = self.read_packet().await?;
        let mut cur = ReadCursor::new(&bytes);
        let resp = TunnelRespPkt::decode(&mut cur).unwrap();

        println!("TUNNEL RESP: {:?}", resp);
        if resp.status_code != 0 {
            return Err(Error::new("Tunnel", GwErrorKind::Connect));
        }
        assert!(cur.eof());
        Ok(())
    }

    async fn tunnel_auth(&mut self) -> Result<(), Error> {
        let req = TunnelAuthPkt { fields_present: 0, client_name: "testpc".to_string() };
        self.send_packet(&req).await?;
        
        let (hdr, bytes) = self.read_packet().await?;
        let mut cur = ReadCursor::new(&bytes);
        let resp = TunnelAuthRespPkt::decode(&mut cur).unwrap();

        println!("TUNNEL AUTH RESP: {:?}", resp);
        if resp.error_code != 0 {
            return Err(Error::new("TunnelAuth", GwErrorKind::Connect));
        }
        Ok(())
    }

    async fn channel(&mut self) -> Result<(), Error> {
        let req = ChannelPkt {
            resources: vec![ self.target.server.to_string() ],
            port: 3389,
            protocol: 3
        };
        self.send_packet(&req).await?;

        let (hdr, bytes) = self.read_packet().await?;
        // TODO: asserts atleast for hdr.type missing since the port
        let mut cur = ReadCursor::new(&bytes);
        let resp = ChannelResp::decode(&mut cur).unwrap();
        println!("CHANNEL RESP: {:?}", resp);
        // status 0x800759DD E_PROXY_TS_CONNECTFAILED
        if resp.error_code != 0 {
            return Err(Error::new("ChannelCreate", GwErrorKind::Connect));
        }
        assert!(cur.eof());
        Ok(())
    }
}

impl AsyncRead for GwClient {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // Get new bufs
        if let Poll::Ready(Some(new_buf)) = self.rx.poll_recv(cx) {
            self.rx_bufs.push(new_buf);
        }

        // Read from all queued bufs
        let mut n = 0;
        self.rx_bufs.retain_mut(|rx_buf| {
            let rem = buf.remaining();
            if rem == 0 {
                return true
            }
            let max = std::cmp::min(rem, rx_buf.len());
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
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> Poll<Result<usize, std::io::Error>> {

        match self.tx.poll_reserve(cx) {
            Poll::Ready(Ok(())) => {
                self.tx.send_item(Bytes::from(buf.to_vec())).unwrap();
                return Poll::Ready(Ok(buf.len()))
            },
            Poll::Ready(Err(_)) => todo!(),
            _ => (),
        }

        Poll::Pending
    }
    
    fn poll_flush(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Result<(), std::io::Error>> {
        Poll::Ready(Ok(())) // TODO NOP
    }
    
    fn poll_shutdown(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Result<(), std::io::Error>> {
        todo!()
    }
}
