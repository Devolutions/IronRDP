//! [MS-TSGU] https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
//! * This implements a MVP (in terms of recentness) state needed to connect through microsoft rdp gateway.
//! * This only supports the HTTPS protocol with Websocket (and not the legacy HTTP, HTTP-RPC or UDP protocols).
//! * This does implement reconnection/reauthentication.
//! * This only supports basic auth.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use hyper::body::Bytes;
use ironrdp_core::{Decode, Encode, ReadCursor, WriteCursor};
use tokio::{io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt}, net::TcpStream, sync::oneshot};
use tokio_native_tls::TlsStream;
use tokio_tungstenite::{tungstenite::{handshake::client::generate_key, http::{self}, protocol::Role, Message}, WebSocketStream};
use futures_util::{stream::{SplitSink, SplitStream}, SinkExt, StreamExt};

mod proto;
use proto::*;

#[derive(Debug)]
struct GwConnectTarget {
    gw_endpoint: String,
    gw_user: String,
    gw_pass: String,

    server: String,
}

struct GwClient {   
    target: GwConnectTarget,
    ws_sink: SplitSink<WebSocketStream<TlsStream<TcpStream>>, Message>,
    ws_stream: SplitStream<WebSocketStream<TlsStream<TcpStream>>>,
}

impl GwClient {
    async fn connect(target: &GwConnectTarget) -> Result<TlsStream<TcpStream>, Box<dyn std::error::Error>> {
        let gw_host = target.gw_endpoint.split(":").nth(0).unwrap();


        let stream = TcpStream::connect(&target.gw_endpoint).await?;
        let stream = tokio_native_tls::TlsConnector::from(tokio_native_tls::native_tls::TlsConnector::new().unwrap())
            .connect(gw_host, stream).await?;

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
        let resp = sender.send_request(req).await?;
        println!("RESP: {:?}", resp.status());
        assert_eq!(resp.status(), http::StatusCode::SWITCHING_PROTOCOLS);
    
        let _ = tx.send(()); // TODO: Not needed since it doesnt keep alive conn?
    
        let stream = res.await?.io.into_inner();
        Ok(stream)
    }

    async fn connect_ws(target: GwConnectTarget, tls_stream: TlsStream<TcpStream>) -> GwClient {
        let ws_stream: WebSocketStream<_> = WebSocketStream::from_raw_socket(tls_stream, Role::Client, None).await;
        let (ws_sink, ws_stream) = ws_stream.split();
        GwClient { target, ws_sink, ws_stream }
    }

    async fn send_packet<'a, E: Encode>(&mut self, payload: &E) -> Result<(), Box<dyn std::error::Error>> {
        let mut buf = [0u8; 4096];
        let pos = {
            let mut cur = WriteCursor::new(&mut buf);
            payload.encode(&mut cur).unwrap();
            cur.pos() as usize
        };
        self.ws_sink.send(Message::Binary(Bytes::copy_from_slice(&buf[..pos]))).await?;
        Ok(())
    }

    async fn read_packet(&mut self) -> Result<(PktHdr, Bytes), Box<dyn std::error::Error>> {
        let mut msg = self.ws_stream.next().await.unwrap().unwrap().into_data();
        let mut cur = ReadCursor::new(&msg);
        let hdr = PktHdr::decode(&mut cur).unwrap();
        assert!(cur.len() >= hdr.length as usize - hdr.size());

        Ok((hdr, msg.split_off(cur.pos())))
    }

    async fn handshake(&mut self) -> Result<(), Box<dyn std::error::Error>> {
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
        assert_eq!(resp.error_code, 0);

        assert_eq!(resp.extended_auth, 7); //TODO....
        Ok(())
    }

    async fn tunnel(&mut self) -> Result<(), Box<dyn std::error::Error>> {
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
        assert_eq!(resp.status_code, 0);
        assert!(cur.eof());
        Ok(())
    }

    async fn tunnel_auth(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let req = TunnelAuthPkt { fields_present: 0, client_name: "testpc".to_string() };
        self.send_packet(&req).await?;
        
        let (hdr, bytes) = self.read_packet().await?;
        let mut cur = ReadCursor::new(&bytes);
        let resp = TunnelAuthRespPkt::decode(&mut cur).unwrap();

        println!("TUNNEL AUTH RESP: {:?}", resp);
        assert_eq!(resp.error_code, 0);
        Ok(())
    }

    async fn channel(&mut self) -> Result<(), Box<dyn std::error::Error>> {
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
        assert_eq!(resp.error_code, 0);
        assert!(cur.eof());
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {   
    env_logger::init();

    let target = GwConnectTarget {
        gw_endpoint: "gw:443".to_string(),
        gw_user: "".to_string(),
        gw_pass: "".to_string(),
        server: "workstation".to_string(),
    };

    let conn = GwClient::connect(&target).await?;
    let mut cl = GwClient::connect_ws(target, conn).await;
    cl.handshake().await?;
    cl.tunnel().await?;
    cl.tunnel_auth().await?;
    cl.channel().await?;

    println!("Listening...");
    let listener = tokio::net::TcpListener::bind("localhost:3389").await?;
    let (conn, addr) = listener.accept().await?;
    println!("Got conn");

    let (mut connr, mut connw) = conn.into_split();

    // Spawn a simple 1:1 proxy
    // -> conn
    tokio::spawn(async move {
        while let Some(next) = cl.ws_stream.next().await {
            let msg = next.unwrap().into_data();
            println!("recv through websocket {}", msg.len());
            let mut cur = ReadCursor::new(&msg);
            let hdr = PktHdr::decode(&mut cur).unwrap();
            assert!(cur.len() >= hdr.length as usize - hdr.size());
            assert_eq!(hdr.ty, PktTy::Data);
            let p = DataPkt::decode(&mut cur).unwrap();
            (&mut connw).write_all(p.data).await.unwrap();
        }
    });

    // conn -> 
    let mut wsbuf = [0u8; 8192];
    loop {
        let mut buf = [0u8; 4096];
        let n = connr.read(&mut buf).await?;
        println!("send through websocket {n}");
        if n == 0 {
            break
        }
        
        let pkt = DataPkt {
            data: &buf[..n],
        };

        let pos = {
            let mut cur = WriteCursor::new(&mut wsbuf);
            pkt.encode(&mut cur).unwrap();
            cur.pos() as usize
        };
        cl.ws_sink.send(Message::Binary(Bytes::copy_from_slice(&wsbuf[..pos]))).await?;
    }

    Ok(())
}
