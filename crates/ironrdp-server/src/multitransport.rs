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
use ironrdp_rdpeudp_tokio::{UdpAcceptConfig, UdpTransport, UdpTransportSender, accept_udp};
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
/// `send()` uses `UdpTransportSender`, cloned once at construction from the
/// underlying `UdpTransport`. That handle is entirely independent of the
/// `Mutex` below, so it never contends with (or blocks behind) `recv()`,
/// which holds that lock for the full duration of its idle wait for the
/// next datagram: Sending and receiving already run over separate channels
/// fed by separate background tasks, so the two were never meant to share
/// one lock. `recv()` is intended for a single dedicated caller (the
/// `client_loop` select arm).
#[derive(Clone)]
pub(crate) struct UdpTransportHandle {
    sender: UdpTransportSender,
    transport: Arc<Mutex<UdpTransport>>,
}

impl UdpTransportHandle {
    fn new(transport: UdpTransport) -> Self {
        let sender = transport.sender();
        Self {
            sender,
            transport: Arc::new(Mutex::new(transport)),
        }
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
        match self.sender.send(data).await {
            Ok(()) => true,
            Err(error) => {
                warn!(%error, "Failed to send data over UDP transport");
                false
            }
        }
    }

    pub(crate) async fn recv(&self) -> Option<Vec<u8>> {
        self.transport.lock().await.recv().await
    }
}

/// Attempts to establish the sideband UDP transport for one Initiate
/// Multitransport Request.
///
/// Binds a fresh socket to `udp_bind_addr` for this attempt, avoiding any
/// shared, long-lived UDP socket state to manage. This assumes at most one
/// live attempt at a time; under
/// [`RdpServerOptions::preempt_existing_session`](crate::server::RdpServerOptions::preempt_existing_session)
/// a candidate session's negotiation can overlap the still-live session it
/// is preempting, and `udp_bind_addr` is typically the same host and port for
/// both (see [`RdpServerBuilder::with_udp_transport`](crate::RdpServerBuilder::with_udp_transport)),
/// so the second bind fails with `AddrInUse`. That failure is handled below
/// like any other: The overlapping connection degrades to TCP-only rather
/// than failing.
///
/// Any failure (bind, RDPEUDP2 handshake, TLS, RDPEMT tunnel) is logged and
/// reported as `None` rather than propagated: An optional sideband transport
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
