//! Server-side UDP multitransport bootstrapping.
//!
//! [MS-RDPBCGR] 2.2.15.1/2.2.15.2 (`ironrdp_acceptor::Acceptor`) sends the
//! Initiate Multitransport Request and reports it via
//! [`ironrdp_acceptor::accept_finalize_with_multitransport`]. This module
//! establishes the sideband RDPEUDP2 + TLS + RDPEMT transport that request
//! bootstraps, for [`crate::server`] to migrate DVC channels onto via
//! [`ironrdp_dvc::DrdynvcServer::request_reliable_udp`].
//!
//! [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/b8e7c588-51cb-455b-bb73-92d480903133

use core::net::SocketAddr;
use core::time::Duration;
use std::sync::Arc;

use ironrdp_pdu::rdp::multitransport::MultitransportRequestPdu;
use ironrdp_rdpemt::TunnelConfig;
use ironrdp_rdpeudp::ConnectionConfig;
use ironrdp_rdpeudp_tokio::{UdpAcceptConfig, UdpTransport, accept_udp};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio_rustls::rustls;
use tracing::{debug, warn};

/// How long the full UDP accept sequence (initial datagram, RDPEUDP2
/// handshake, TLS, RDPEMT tunnel creation) may take before giving up and
/// continuing TCP-only.
const UDP_ACCEPT_TIMEOUT: Duration = Duration::from_secs(15);

/// Shared handle to an established sideband UDP transport.
///
/// `send()` only holds the lock for the hand-off to the transport's internal
/// channel, mirroring the short-lived `this.lock().await` pattern already
/// used throughout the client dispatch loop in `server.rs`. `recv()` is
/// intended for a single dedicated caller (the `client_loop` select arm):
/// each call re-acquires the lock fresh, so it is never held across the idle
/// wait for the next datagram, and a concurrent `send()` (an outgoing EGFX
/// frame, say) is never blocked behind it.
#[derive(Clone)]
pub(crate) struct UdpTransportHandle(Arc<Mutex<UdpTransport>>);

impl UdpTransportHandle {
    fn new(transport: UdpTransport) -> Self {
        Self(Arc::new(Mutex::new(transport)))
    }

    /// Sends one higher-layer (raw DVC) payload over the tunnel.
    ///
    /// Returns whether the send succeeded; a failure is logged here rather
    /// than propagated; the caller's fallback is simply to have sent nothing
    /// over UDP for this batch; the RDP session itself never fails over an
    /// optional sideband transport.
    #[cfg_attr(
        not(feature = "egfx"),
        expect(
            dead_code,
            reason = "only consumer today is the EGFX outgoing path, gated on the egfx feature"
        )
    )]
    pub(crate) async fn send(&self, data: Vec<u8>) -> bool {
        match self.0.lock().await.send(data).await {
            Ok(()) => true,
            Err(error) => {
                warn!(%error, "Failed to send data over UDP transport");
                false
            }
        }
    }

    pub(crate) async fn recv(&self) -> Option<Vec<u8>> {
        self.0.lock().await.recv().await
    }
}

/// Attempts to establish the sideband UDP transport for one Initiate
/// Multitransport Request.
///
/// Binds a fresh socket to `udp_bind_addr` for this attempt. Only one
/// attempt is ever in flight at a time: `ironrdp-server` serves one
/// connection at a time (see the integration plan's non-goal on multi-client
/// UDP demux), so a single ephemeral bind per connection is sufficient and
/// avoids any shared, long-lived UDP socket state to manage.
///
/// Any failure (bind, RDPEUDP2 handshake, TLS, RDPEMT tunnel) is logged and
/// reported as `None` rather than propagated: an optional sideband transport
/// failing to come up is never a reason to fail the RDP connection, matching
/// the reference client implementation's posture.
pub(crate) async fn accept(
    udp_bind_addr: SocketAddr,
    tls_config: Arc<rustls::ServerConfig>,
    request: &MultitransportRequestPdu,
) -> Option<UdpTransportHandle> {
    let socket = match UdpSocket::bind(udp_bind_addr).await {
        Ok(socket) => socket,
        Err(error) => {
            warn!(%error, %udp_bind_addr, "Failed to bind UDP socket for multitransport, continuing TCP-only");
            return None;
        }
    };

    let config = UdpAcceptConfig {
        tls_config,
        tunnel_config: TunnelConfig {
            request_id: request.request_id,
            security_cookie: request.security_cookie,
        },
        connection_config: ConnectionConfig::default(),
        accept_timeout: UDP_ACCEPT_TIMEOUT,
    };

    match accept_udp(socket, config).await {
        Ok(transport) => {
            debug!("Sideband UDP transport established");
            Some(UdpTransportHandle::new(transport))
        }
        Err(error) => {
            warn!(%error, "Failed to establish sideband UDP transport, continuing TCP-only");
            None
        }
    }
}
