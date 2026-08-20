//! Packet I/O over the MS-TSGU HTTPS WebSocket transport.

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt as _, StreamExt as _};
use http_body_util::BodyExt as _;
use hyper::body::Bytes;
use ironrdp_tls::TlsStream;
use log::error;
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::{Message, http};

use crate::http_auth::{AuthStep, GatewayHttpAuth, basic_authorization, www_authenticate_values};
use crate::{Error, GwConnectTarget, GwErrorKind};

/// WebSocket byte transport used after the HTTP upgrade completes.
pub(crate) struct PacketIo {
    sink: SplitSink<WebSocketStream<TlsStream<TcpStream>>, Message>,
    stream: SplitStream<WebSocketStream<TlsStream<TcpStream>>>,
}

impl PacketIo {
    pub(crate) async fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.sink
            .send(Message::Binary(Bytes::copy_from_slice(bytes)))
            .await
            .map_err(|e| custom_err!("ws send", e))?;
        Ok(())
    }

    /// Sends a WebSocket close frame so a local write-side EOF can finish.
    pub(crate) async fn close(&mut self) -> Result<(), Error> {
        self.sink
            .send(Message::Close(None))
            .await
            .map_err(|e| custom_err!("websocket close", e))?;
        self.sink
            .flush()
            .await
            .map_err(|e| custom_err!("websocket close flush", e))?;
        Ok(())
    }

    /// Reads one WebSocket message as an MS-TSGU packet buffer.
    ///
    /// Returns `Ok(None)` on a clean close or an exhausted stream.
    pub(crate) async fn read_packet_buf(&mut self) -> Result<Option<Bytes>, Error> {
        let msg = match self.stream.next().await {
            None => return Ok(None),
            Some(Ok(msg)) => msg,
            Some(Err(e)) => return Err(custom_err!("Stream", e)),
        };
        if matches!(msg, Message::Close(_)) {
            return Ok(None);
        }
        Ok(Some(msg.into_data()))
    }
}

/// Open a TLS WebSocket to the gateway and authenticate the upgrade.
pub(crate) async fn open_websocket_transport(
    target: &GwConnectTarget,
) -> Result<(PacketIo, core::net::SocketAddr), Error> {
    let gw_host = target
        .gw_endpoint
        .split(":")
        .next()
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

    let ws_stream = WebSocketStream::from_raw_socket(stream, Role::Client, None).await;
    let (sink, stream) = ws_stream.split();
    Ok((PacketIo { sink, stream }, client_addr))
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
        .map_err(|e| custom_err!("http auth task", e))?
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
