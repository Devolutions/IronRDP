//! Open an MS-TSGU RD Gateway tunnel to an arbitrary target host:port as a byte stream.
//!
//! The RD Gateway tunneling protocol is not RDP-specific: the gateway relays a TCP byte
//! stream between the client and any reachable target. This module opens such a tunnel
//! and surfaces it as a generic [`AsyncRead`]/[`AsyncWrite`] stream, so any TCP-based
//! protocol (SSH, a database, a custom socket) can traverse the gateway.

use tokio::io::{AsyncRead, AsyncWrite};

use ironrdp_mstsgu::{GwClient, GwConnectTarget, RpchStream};
use ironrdp_tls::CertificateValidation;

use crate::error::{ForwardError, ForwardErrorKind, Result};

/// A bidirectional byte stream carried over an RD Gateway tunnel.
pub trait TunnelStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> TunnelStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

/// Which MS-TSGU transport to use for the gateway tunnel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GatewayTransport {
    /// Modern WebSocket transport (the default; requires gateway WebSocket support).
    #[default]
    WebSocket,
    /// Legacy RPC-over-HTTP transport ([MS-RPCH] v2) for gateways without WebSocket.
    Rpc,
}

/// Configuration for opening a gateway tunnel.
#[derive(Clone, Debug)]
pub struct GatewayTunnelConfig {
    /// Gateway host:port (for example `rdg.contoso.com:443`).
    pub gateway_endpoint: String,
    /// Gateway username for HTTP authentication.
    pub username: String,
    /// Gateway password for HTTP authentication.
    pub password: String,
    /// Which gateway transport to use.
    pub transport: GatewayTransport,
    /// Client name presented to the gateway.
    pub client_name: String,
    /// TLS certificate policy for the gateway HTTPS leg.
    pub certificate_validation: CertificateValidation,
}

/// Open one gateway tunnel to `target_host:target_port`.
///
/// Returns a byte stream bridged to the target through the gateway. Each call opens an
/// independent tunnel; a port-forward or SOCKS5 listener calls this once per inbound
/// connection.
pub async fn open_tunnel(
    config: &GatewayTunnelConfig,
    target_host: &str,
    target_port: u16,
) -> Result<Box<dyn TunnelStream>> {
    let target = GwConnectTarget {
        gw_endpoint: config.gateway_endpoint.clone(),
        gw_user: config.username.clone(),
        gw_pass: config.password.clone(),
        server: target_host.to_owned(),
        server_port: target_port,
        smart_card: None,
    };

    match config.transport {
        GatewayTransport::WebSocket => {
            let (client, _addr) = GwClient::connect_with_certificate_validation(
                &target,
                &config.client_name,
                config.certificate_validation,
                None,
            )
            .await
            .map_err(|e| ForwardError::new("open gateway tunnel", ForwardErrorKind::Tunnel).with_source(e))?;
            Ok(Box::new(client))
        }
        GatewayTransport::Rpc => {
            let (stream, _addr): (RpchStream, _) =
                GwClient::connect_rpch(&target, &config.client_name, config.certificate_validation, None)
                    .await
                    .map_err(|e| {
                        ForwardError::new("open gateway rpc tunnel", ForwardErrorKind::Tunnel).with_source(e)
                    })?;
            Ok(Box::new(stream))
        }
    }
}
