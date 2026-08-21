//! Local listeners that relay TCP through an MS-TSGU RD Gateway tunnel.
//!
//! The RD Gateway tunneling protocol ([MS-TSGU]) relays a TCP byte stream to any
//! reachable target host:port; it is not limited to RDP. `gw-forward` uses that to
//! expose a fixed local forward (SSH `-L`-style) or a SOCKS5 CONNECT proxy.
//!
//! Each inbound connection opens an independent gateway tunnel via
//! [`ironrdp_mstsgu::GwClient`] and relays bytes bidirectionally.
//!
//! [MS-TSGU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/

#![allow(clippy::print_stderr)]

mod socks5;

use anyhow::Context as _;
use ironrdp_mstsgu::{GwClient, GwConnectTarget};
use tokio::net::TcpListener;

/// Max payload `GwClient` can encode into its 8192-byte workspace.
const HTTP_PACKET_HEADER_SIZE: usize = 8;
const DATA_PKT_LENGTH_SIZE: usize = 2;
const TUNNEL_WRITE_MAX: usize = 8192 - HTTP_PACKET_HEADER_SIZE - DATA_PKT_LENGTH_SIZE;
const LOCAL_WRITE_MAX: usize = 8192;

/// Configuration for opening a gateway tunnel.
#[derive(Clone, Debug)]
pub(crate) struct GatewayTunnelConfig {
    /// Gateway host:port (for example `rdg.contoso.com:443`).
    pub(crate) gateway_endpoint: String,
    /// Gateway username for HTTP authentication.
    pub(crate) username: String,
    /// Gateway password for HTTP authentication.
    pub(crate) password: String,
    /// Client name presented to the gateway.
    pub(crate) client_name: String,
}

/// Listen on `listen_addr` and forward each inbound TCP connection to
/// `target_host:target_port` through the RD Gateway (SSH `-L`-style local forwarding).
///
/// Runs until the listener errors. Each accepted connection opens an independent tunnel.
/// A local half-close is not forwarded: `GwClient::poll_shutdown` is a no-op on master.
pub(crate) async fn run_port_forward(
    config: GatewayTunnelConfig,
    listen_addr: &str,
    target_host: &str,
    target_port: u16,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(listen_addr).await.context("bind forward listener")?;

    loop {
        let (inbound, peer) = listener.accept().await.context("accept forward connection")?;
        let config = config.clone();
        let target_host = target_host.to_owned();
        tokio::spawn(async move {
            if let Err(e) = relay_forward(config, inbound, &target_host, target_port).await {
                eprintln!("forward connection from {peer} failed: {e}");
            }
        });
    }
}

/// Listen on `listen_addr` and serve SOCKS5 CONNECT, opening a gateway tunnel to each
/// requested destination. Lets any SOCKS5-capable program reach internal hosts through
/// the gateway without per-target configuration.
pub(crate) async fn run_socks5(config: GatewayTunnelConfig, listen_addr: &str) -> anyhow::Result<()> {
    let listener = TcpListener::bind(listen_addr).await.context("bind socks5 listener")?;

    loop {
        let (inbound, peer) = listener.accept().await.context("accept socks5 connection")?;
        let config = config.clone();
        tokio::spawn(async move {
            if let Err(e) = relay_socks5(config, inbound).await {
                eprintln!("socks5 connection from {peer} failed: {e}");
            }
        });
    }
}

async fn open_tunnel(config: &GatewayTunnelConfig, target_host: &str, target_port: u16) -> anyhow::Result<GwClient> {
    let target = GwConnectTarget {
        gw_endpoint: config.gateway_endpoint.clone(),
        gw_user: config.username.clone(),
        gw_pass: config.password.clone(),
        server: target_host.to_owned(),
    };

    let (client, _addr) = GwClient::connect_with_port(&target, &config.client_name, target_port)
        .await
        .context("open gateway tunnel")?;
    Ok(client)
}

async fn relay_forward(
    config: GatewayTunnelConfig,
    mut inbound: tokio::net::TcpStream,
    target_host: &str,
    target_port: u16,
) -> anyhow::Result<()> {
    let mut tunnel = open_tunnel(&config, target_host, target_port).await?;
    relay(&mut inbound, &mut tunnel).await
}

async fn relay_socks5(config: GatewayTunnelConfig, mut inbound: tokio::net::TcpStream) -> anyhow::Result<()> {
    let target = match socks5::negotiate(&mut inbound).await {
        Ok(target) => target,
        Err(socks5::NegotiateFailure::Greeting(e)) => return Err(e),
        Err(socks5::NegotiateFailure::Request { error, reply }) => {
            return Err(socks5::reject_request(&mut inbound, error, reply).await);
        }
    };

    match open_tunnel(&config, &target.host, target.port).await {
        Ok(mut tunnel) => {
            socks5::write_reply(&mut inbound, socks5::REP_SUCCESS).await?;
            relay(&mut inbound, &mut tunnel).await
        }
        Err(e) => {
            let err = e.context("open socks5 tunnel");
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
async fn relay<A, B>(local: &mut A, tunnel: &mut B) -> anyhow::Result<()>
where
    A: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + ?Sized,
    B: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + ?Sized,
{
    tokio::io::copy_bidirectional_with_sizes(local, tunnel, TUNNEL_WRITE_MAX, LOCAL_WRITE_MAX)
        .await
        .context("relay streams")?;
    Ok(())
}
