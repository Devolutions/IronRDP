//! In-process mock RD Gateway for integration-style tests.
//!
//! Speaks the MS-TSGU packet sequence over an in-memory duplex stream:
//! handshake → tunnel create → tunnel auth → channel create → data echo.
//! Failure injection points let tests exercise error paths without a real gateway.

use hyper::body::Bytes;
use ironrdp_core::{Decode as _, Encode, ReadCursor, WriteCursor};
use tokio::io::{AsyncWriteExt as _, DuplexStream};

use crate::packet_io::PacketIo;
use crate::proto::{
    ChannelResp, DataPkt, HandshakeRespPkt, HttpExtendedAuth, PktHdr, PktTy, ReauthMessagePkt, TunnelAuthRespPkt,
    TunnelRespPkt,
};

/// Tunable behavior of the mock gateway.
#[derive(Clone, Debug)]
pub(crate) struct MockGatewayConfig {
    /// Extended auth modes advertised in the handshake response.
    pub handshake_extended_auth: HttpExtendedAuth,
    /// Status code for tunnel create (`0` = success).
    pub tunnel_status_code: u32,
    /// Error code for tunnel auth (`0` = success).
    pub tunnel_auth_error_code: u32,
    /// Error code for channel create (`0` = success).
    pub channel_error_code: u32,
    /// Consent message sent in the tunnel response, when set.
    pub consent_message: Option<String>,
    /// Device redirection policy flags from tunnel auth, when set.
    pub redir_flags: Option<u32>,
    /// Idle timeout in minutes from tunnel auth, when set.
    pub idle_timeout_minutes: Option<u32>,
    /// When set, send a reauth message with this context after channel create.
    pub reauth_tunnel_context: Option<u64>,
    /// Channel close status code sent after channel create, when set.
    pub close_status_code: Option<u32>,
}

impl Default for MockGatewayConfig {
    fn default() -> Self {
        Self {
            handshake_extended_auth: HttpExtendedAuth::HTTP_EXTENDED_AUTH_NONE,
            tunnel_status_code: 0,
            tunnel_auth_error_code: 0,
            channel_error_code: 0,
            consent_message: None,
            redir_flags: None,
            idle_timeout_minutes: None,
            reauth_tunnel_context: None,
            close_status_code: None,
        }
    }
}

/// One end of a mock gateway connection pair: `client_io` plugs into
/// `GwClient::connect_after_transport`, the returned task runs the gateway.
pub(crate) fn mock_gateway(config: MockGatewayConfig) -> (PacketIo, tokio::task::JoinHandle<Vec<PktTy>>) {
    let (client, server) = tokio::io::duplex(64 * 1024);
    let task = tokio::spawn(async move { run_gateway(server, config).await });
    (PacketIo::duplex(client), task)
}

/// A factory of mock gateway connections, for reauth secondary-connection tests.
///
/// Each `open()` call spawns a fresh mock gateway and returns the client transport.
/// The secondary connection's tunnel request is expected to carry the reauth tunnel
/// context, which is recorded in `seen_reauth_context`.
pub(crate) struct MockGatewayFactory {
    config: MockGatewayConfig,
    /// Only the first connection injects the reauth trigger; the secondary
    /// connection must complete cleanly or the client would reauth forever.
    reauth_pending: std::sync::Mutex<bool>,
    pub(crate) tasks: std::sync::Mutex<Vec<tokio::task::JoinHandle<Vec<PktTy>>>>,
    pub(crate) seen_reauth_context: std::sync::Mutex<Option<u64>>,
}

impl MockGatewayFactory {
    pub(crate) fn new(config: MockGatewayConfig) -> std::sync::Arc<Self> {
        let reauth_pending = config.reauth_tunnel_context.is_some();
        std::sync::Arc::new(Self {
            config,
            reauth_pending: std::sync::Mutex::new(reauth_pending),
            tasks: std::sync::Mutex::new(Vec::new()),
            seen_reauth_context: std::sync::Mutex::new(None),
        })
    }

    pub(crate) fn open(self: &std::sync::Arc<Self>) -> PacketIo {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let mut config = self.config.clone();
        {
            let mut pending = self.reauth_pending.lock().expect("reauth pending lock");
            if !*pending {
                config.reauth_tunnel_context = None;
            }
            *pending = false;
        }
        let seen = std::sync::Arc::clone(self);
        let task = tokio::spawn(async move { run_gateway_tracking_reauth(server, config, seen).await });
        self.tasks.lock().expect("tasks lock").push(task);
        PacketIo::duplex(client)
    }
}

/// Like `run_gateway`, but records the reauth tunnel context from a tunnel-create
/// request that carries one (secondary reauth connection).
async fn run_gateway_tracking_reauth(
    stream: DuplexStream,
    config: MockGatewayConfig,
    factory: std::sync::Arc<MockGatewayFactory>,
) -> Vec<PktTy> {
    run_gateway_inner(stream, config, Some(factory)).await
}

/// Run the gateway side; returns the packet types received, for assertions.
async fn run_gateway(stream: DuplexStream, config: MockGatewayConfig) -> Vec<PktTy> {
    run_gateway_inner(stream, config, None).await
}

async fn run_gateway_inner(
    mut stream: DuplexStream,
    config: MockGatewayConfig,
    reauth_tracking: Option<std::sync::Arc<MockGatewayFactory>>,
) -> Vec<PktTy> {
    let mut received = Vec::new();

    // Handshake
    if read_packet(&mut stream, &mut received).await.is_none() {
        return received;
    }
    let resp = HandshakeRespPkt {
        error_code: 0,
        ver_major: 1,
        ver_minor: 0,
        server_version: 0,
        extended_auth: config.handshake_extended_auth,
    };
    if send_packet(&mut stream, &resp).await.is_err() {
        return received;
    }

    // Tunnel create
    let Some((_, tunnel_body)) = read_packet(&mut stream, &mut received).await else {
        return received;
    };
    // Record the reauth tunnel context when the tunnel-create request carries one
    // (HTTP_TUNNEL_PACKET_FIELD_REAUTH, [MS-TSGU] 2.2.10.19).
    if let Some(tracking) = &reauth_tracking {
        let mut cur = ReadCursor::new(&tunnel_body);
        let _caps = cur.read_u32();
        let fields_present = cur.read_u16();
        let _reserved = cur.read_u16();
        if fields_present & 0x2 != 0 && cur.len() >= 8 {
            let context = cur.read_u64();
            *tracking.seen_reauth_context.lock().expect("reauth context lock") = Some(context);
        }
    }
    let resp = TunnelRespPkt {
        status_code: config.tunnel_status_code,
        tunnel_id: Some(1),
        consent_msg: config
            .consent_message
            .as_ref()
            .map(|msg| msg.encode_utf16().flat_map(u16::to_le_bytes).collect())
            .unwrap_or_default(),
        ..TunnelRespPkt::default()
    };
    if send_packet(&mut stream, &resp).await.is_err() {
        return received;
    }
    if config.tunnel_status_code != 0 {
        return received;
    }

    // Tunnel auth
    if read_packet(&mut stream, &mut received).await.is_none() {
        return received;
    }
    let resp = TunnelAuthRespPkt {
        error_code: config.tunnel_auth_error_code,
        redir_flags: config.redir_flags,
        idle_timeout_minutes: config.idle_timeout_minutes,
        ..TunnelAuthRespPkt::default()
    };
    if send_packet(&mut stream, &resp).await.is_err() {
        return received;
    }
    if config.tunnel_auth_error_code != 0 {
        return received;
    }

    // Channel create
    if read_packet(&mut stream, &mut received).await.is_none() {
        return received;
    }
    let resp = if config.channel_error_code == 0 {
        ChannelResp::success(1)
    } else {
        ChannelResp {
            error_code: config.channel_error_code,
            ..ChannelResp::default()
        }
    };
    if send_packet(&mut stream, &resp).await.is_err() {
        return received;
    }
    if config.channel_error_code != 0 {
        return received;
    }

    // Optional post-connect injections, then data echo loop.
    if let Some(context) = config.reauth_tunnel_context {
        let _ = send_packet(
            &mut stream,
            &ReauthMessagePkt {
                reauth_tunnel_context: context,
            },
        )
        .await;
    }
    if let Some(status_code) = config.close_status_code {
        let _ = send_packet(&mut stream, &crate::proto::ChannelClosePkt { status_code }).await;
        return received;
    }

    while let Some((hdr, body)) = read_packet(&mut stream, &mut received).await {
        // Channel close completes the close handshake ([MS-TSGU] 3.3.5.5).
        if hdr.ty == PktTy::ChannelClose {
            let _ = send_packet(&mut stream, &crate::proto::ChannelClosePkt { status_code: 0 }).await;
            return received;
        }
        if hdr.ty != PktTy::Data {
            continue;
        }
        let mut cur = ReadCursor::new(&body);
        let Ok(data) = DataPkt::decode(&mut cur) else {
            break;
        };
        if send_packet(&mut stream, &DataPkt { data: data.data }).await.is_err() {
            break;
        }
    }

    received
}

async fn read_packet(stream: &mut DuplexStream, received: &mut Vec<PktTy>) -> Option<(PktHdr, Bytes)> {
    use tokio::io::AsyncReadExt as _;

    let mut header = [0u8; 8];
    stream.read_exact(&mut header).await.ok()?;
    let mut cur = ReadCursor::new(&header);
    let hdr = PktHdr::decode(&mut cur).ok()?;
    let body_len = usize::try_from(hdr.length).ok()?.checked_sub(8)?;
    let mut body = vec![0u8; body_len];
    stream.read_exact(&mut body).await.ok()?;
    received.push(hdr.ty);
    Some((hdr, Bytes::from(body)))
}

async fn send_packet(stream: &mut DuplexStream, packet: &impl Encode) -> Result<(), ()> {
    let mut buf = vec![0u8; packet.size()];
    let mut cur = WriteCursor::new(&mut buf);
    packet.encode(&mut cur).map_err(|_| ())?;
    stream.write_all(&buf).await.map_err(|_| ())
}
