//! Local listeners that relay connections through an RD Gateway tunnel.

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tracing::{debug, info, instrument, warn};

use crate::error::{ForwardError, ForwardErrorKind, Result};
use crate::socks5;
use crate::tunnel::{GatewayTunnelConfig, open_tunnel};

/// Max payload `GwClient` can encode into its 8192-byte workspace.
const HTTP_PACKET_HEADER_SIZE: usize = 8;
const DATA_PKT_LENGTH_SIZE: usize = 2;
const TUNNEL_WRITE_MAX: usize = 8192 - HTTP_PACKET_HEADER_SIZE - DATA_PKT_LENGTH_SIZE;
const LOCAL_WRITE_MAX: usize = 8192;

/// Listen on `listen_addr` and forward each inbound TCP connection to
/// `target_host:target_port` through the RD Gateway (SSH `-L`-style local forwarding).
///
/// Runs until the listener errors. Each accepted connection opens an independent tunnel.
/// A local half-close is not forwarded: `GwClient::poll_shutdown` is a no-op on master.
#[instrument(skip(config), fields(listen = %listen_addr, target = %format_args!("{target_host}:{target_port}")))]
pub async fn run_port_forward(
    config: GatewayTunnelConfig,
    listen_addr: &str,
    target_host: &str,
    target_port: u16,
) -> Result<()> {
    let listener = TcpListener::bind(listen_addr)
        .await
        .map_err(|e| ForwardError::new("bind forward listener", ForwardErrorKind::Listener).with_source(e))?;
    let local = listener
        .local_addr()
        .map_err(|e| ForwardError::new("forward listener address", ForwardErrorKind::Listener).with_source(e))?;
    info!(%local, %target_host, target_port, "Port forward listening");

    loop {
        let (inbound, peer) = listener
            .accept()
            .await
            .map_err(|e| ForwardError::new("accept forward connection", ForwardErrorKind::Listener).with_source(e))?;
        debug!(%peer, "Accepted forward connection");
        let config = config.clone();
        let target_host = target_host.to_owned();
        tokio::spawn(async move {
            if let Err(e) = relay_forward(config, inbound, &target_host, target_port).await {
                warn!(%peer, error = %e, "Forward connection failed");
            }
        });
    }
}

/// Listen on `listen_addr` and serve SOCKS5 CONNECT, opening a gateway tunnel to each
/// requested destination. Lets any SOCKS5-capable program reach internal hosts through
/// the gateway without per-target configuration.
#[instrument(skip(config), fields(listen = %listen_addr))]
pub async fn run_socks5(config: GatewayTunnelConfig, listen_addr: &str) -> Result<()> {
    let listener = TcpListener::bind(listen_addr)
        .await
        .map_err(|e| ForwardError::new("bind socks5 listener", ForwardErrorKind::Listener).with_source(e))?;
    let local = listener
        .local_addr()
        .map_err(|e| ForwardError::new("socks5 listener address", ForwardErrorKind::Listener).with_source(e))?;
    info!(%local, "SOCKS5 proxy listening");

    loop {
        let (inbound, peer) = listener
            .accept()
            .await
            .map_err(|e| ForwardError::new("accept socks5 connection", ForwardErrorKind::Listener).with_source(e))?;
        debug!(%peer, "Accepted socks5 connection");
        let config = config.clone();
        tokio::spawn(async move {
            if let Err(e) = relay_socks5(config, inbound).await {
                warn!(%peer, error = %e, "SOCKS5 connection failed");
            }
        });
    }
}

async fn relay_forward(
    config: GatewayTunnelConfig,
    mut inbound: tokio::net::TcpStream,
    target_host: &str,
    target_port: u16,
) -> Result<()> {
    let mut tunnel = open_tunnel(&config, target_host, target_port).await?;
    relay(&mut inbound, &mut *tunnel).await
}

async fn relay_socks5(config: GatewayTunnelConfig, mut inbound: tokio::net::TcpStream) -> Result<()> {
    let target = match socks5::negotiate(&mut inbound).await {
        Ok(target) => target,
        Err(socks5::NegotiateFailure::Greeting(e)) => return Err(e),
        Err(socks5::NegotiateFailure::Request { error, reply }) => {
            return Err(socks5::reject_request(&mut inbound, error, reply).await);
        }
    };
    debug!(host = %target.host, port = target.port, "SOCKS5 CONNECT");

    match open_tunnel(&config, &target.host, target.port).await {
        Ok(mut tunnel) => {
            socks5::write_reply(&mut inbound, socks5::REP_SUCCESS).await?;
            relay(&mut inbound, &mut *tunnel).await
        }
        Err(e) => {
            let err = ForwardError::new("open socks5 tunnel", ForwardErrorKind::Tunnel).with_source(e);
            Err(socks5::reject_request(&mut inbound, err, socks5::REP_GENERAL_FAILURE).await)
        }
    }
}

/// Bidirectionally copy bytes between the local connection and the gateway tunnel until
/// either side closes.
///
/// Local-to-gateway writes are capped at [`TUNNEL_WRITE_MAX`] so each write fits in the
/// 8192-byte MS-TSGU data-packet workspace used by `GwClient`.
/// `GwClient::poll_shutdown` is a no-op on master, so a local half-close is not signaled
/// to the target.
async fn relay<A, B>(local: &mut A, tunnel: &mut B) -> Result<()>
where
    A: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + ?Sized,
    B: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + ?Sized,
{
    let mut to_tunnel = vec![0u8; TUNNEL_WRITE_MAX];
    let mut to_local = vec![0u8; LOCAL_WRITE_MAX];
    let mut local_eof = false;
    let mut tunnel_eof = false;

    while !local_eof || !tunnel_eof {
        tokio::select! {
            result = local.read(&mut to_tunnel), if !local_eof => {
                let n = result.map_err(|e| ForwardError::new("read local stream", ForwardErrorKind::Io).with_source(e))?;
                if n == 0 {
                    local_eof = true;
                    let _ = tunnel.shutdown().await;
                } else {
                    tunnel
                        .write_all(&to_tunnel[..n])
                        .await
                        .map_err(|e| ForwardError::new("write gateway stream", ForwardErrorKind::Io).with_source(e))?;
                }
            }
            result = tunnel.read(&mut to_local), if !tunnel_eof => {
                let n = result.map_err(|e| ForwardError::new("read gateway stream", ForwardErrorKind::Io).with_source(e))?;
                if n == 0 {
                    tunnel_eof = true;
                    let _ = local.shutdown().await;
                } else {
                    local
                        .write_all(&to_local[..n])
                        .await
                        .map_err(|e| ForwardError::new("write local stream", ForwardErrorKind::Io).with_source(e))?;
                }
            }
        }
    }
    Ok(())
}
