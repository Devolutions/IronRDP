//! Multitransport bootstrapping orchestrator.
//!
//! When an RDP server sends an Initiate Multitransport Request PDU on
//! the TCP connection, the client must:
//!
//! 1. Parse the request (requestId + requestedProtocol + securityCookie)
//! 2. Establish a UDP transport using `connect_udp()`
//! 3. Send an Initiate Multitransport Response PDU (S_OK or E_ABORT)
//!    back on the TCP connection
//!
//! `MultitransportBootstrap` orchestrates this sequence. The upstream
//! `ironrdp-connector` doesn't implement multitransport yet, so this
//! acts as a standalone shim that an application wires into its
//! connection flow.

use core::net::SocketAddr;

use ironrdp_pdu::rdp::multitransport::{MultitransportRequestPdu, MultitransportResponsePdu, RequestedProtocol};
use ironrdp_rdpemt::{RdpemtError, RdpemtErrorExt as _, TunnelConfig};
use ironrdp_rdpeudp::ConnectionConfig;

use crate::error::{UdpTransportError, UdpTransportErrorExt as _};
use crate::transport::{UdpTransport, UdpTransportConfig, connect_udp};

/// Orchestrates the multitransport connection sequence.
///
/// Created from the raw Initiate Multitransport Request PDU payload
/// received on the TCP MCS message channel. Call [`connect()`] to
/// establish the UDP transport, then [`response_pdu()`] to get the
/// bytes to send back on TCP.
///
/// [`connect()`]: MultitransportBootstrap::connect
/// [`response_pdu()`]: MultitransportBootstrap::response_pdu
///
/// # Example
///
/// ```ignore
/// // In the TCP connection handler, after receiving MultitransportRequestPdu:
/// let mut bootstrap = MultitransportBootstrap::new(request);
///
/// // Attempt the UDP connection
/// let _ = bootstrap.connect(server_addr, "server.example.com".into(), Default::default()).await;
///
/// // Always send the response back on TCP (S_OK or E_ABORT)
/// let response_bytes = bootstrap.response_pdu().expect("response available after connect");
/// tcp_writer.write_all(&response_bytes).await?;
///
/// // If successful, use the transport
/// if let Some(transport) = bootstrap.take_transport() {
///     // ... use transport for DVC data
/// }
/// ```
pub struct MultitransportBootstrap {
    request: MultitransportRequestPdu,
    transport: Option<UdpTransport>,
    response: Option<MultitransportResponsePdu>,
}

impl MultitransportBootstrap {
    /// Create from a parsed `MultitransportRequestPdu`.
    pub fn new(request: MultitransportRequestPdu) -> Self {
        Self {
            request,
            transport: None,
            response: None,
        }
    }

    /// Parse from raw PDU bytes.
    ///
    /// The bytes are the Initiate Multitransport Request PDU (after TPKT +
    /// X224 + MCS have been stripped by the existing ironrdp-pdu stack, but
    /// including the leading `BasicSecurityHeader`): `MultitransportRequestPdu`
    /// decodes and validates that header itself, rejecting anything whose
    /// flags are not exactly `SEC_TRANSPORT_REQ`.
    pub fn from_pdu(pdu_bytes: &[u8]) -> Result<Self, UdpTransportError> {
        let request: MultitransportRequestPdu = ironrdp_core::decode(pdu_bytes)
            .map_err(|error| UdpTransportError::rdpemt("decode multitransport request", RdpemtError::decode(error)))?;

        Ok(Self::new(request))
    }

    /// Attempt to establish the UDP transport.
    ///
    /// On success, stores the transport and prepares an `S_OK` response.
    /// On failure, prepares an `E_ABORT` response and returns the error.
    ///
    /// After calling this, use [`response_pdu()`] to get the bytes to
    /// send back to the server on the TCP connection.
    ///
    /// [`response_pdu()`]: MultitransportBootstrap::response_pdu
    pub async fn connect(
        &mut self,
        server_addr: SocketAddr,
        server_name: String,
        connection_config: ConnectionConfig,
    ) -> Result<(), UdpTransportError> {
        // This driver only implements the reliable transport (RDPEUDP2 + TLS).
        // UdpFecL (lossy RDPEUDP + DTLS) is a distinct wire protocol this crate
        // does not speak; connecting the reliable stack anyway and reporting
        // S_OK would tell the server a different transport succeeded than the
        // one it asked for, so reject it before attempting a connection.
        if self.request.requested_protocol != RequestedProtocol::UdpFecR {
            self.response = Some(MultitransportResponsePdu::abort(self.request.request_id));
            return Err(UdpTransportError::unsupported_protocol(
                "multitransport connect",
                self.request.requested_protocol,
            ));
        }

        let tunnel_config = TunnelConfig {
            request_id: self.request.request_id,
            security_cookie: self.request.security_cookie,
        };
        let mut config = UdpTransportConfig::new(server_addr, server_name, tunnel_config);
        config.connection_config = connection_config;

        match connect_udp(config).await {
            Ok(transport) => {
                self.transport = Some(transport);
                self.response = Some(MultitransportResponsePdu::success(self.request.request_id));
                Ok(())
            }
            Err(e) => {
                // A stale transport from an earlier successful call must not
                // survive a failed reconnect: the response below tells the
                // server the connection is down, so is_connected() and
                // take_transport() must agree, not keep handing out a
                // transport the server was just told doesn't exist.
                self.transport = None;
                self.response = Some(MultitransportResponsePdu::abort(self.request.request_id));
                Err(e)
            }
        }
    }

    /// Get the response PDU bytes to send back on the TCP connection.
    ///
    /// Returns `None` if [`connect()`] hasn't been called yet.
    ///
    /// # Panics
    ///
    /// Panics if the response fails to encode. The response is a fixed-size
    /// structure built by this crate, so a failure here means the encoder and
    /// the type have gone out of sync rather than anything a caller can cause.
    ///
    /// [`connect()`]: MultitransportBootstrap::connect
    pub fn response_pdu(&self) -> Option<Vec<u8>> {
        self.response
            .as_ref()
            .map(|r| ironrdp_core::encode_vec(r).expect("MultitransportResponsePdu encoding cannot fail"))
    }

    /// The original request from the server.
    pub fn request(&self) -> &MultitransportRequestPdu {
        &self.request
    }

    /// Take ownership of the established UDP transport.
    ///
    /// Returns `None` if the connection failed or hasn't been attempted.
    pub fn take_transport(&mut self) -> Option<UdpTransport> {
        self.transport.take()
    }

    /// Whether the UDP transport was established.
    pub fn is_connected(&self) -> bool {
        self.transport.is_some()
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};

    use super::*;

    fn request(
        request_id: u32,
        security_cookie: [u8; 16],
        requested_protocol: RequestedProtocol,
    ) -> MultitransportRequestPdu {
        MultitransportRequestPdu {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::TRANSPORT_REQ,
            },
            request_id,
            requested_protocol,
            security_cookie,
        }
    }

    #[test]
    fn new_from_request() {
        let bootstrap = MultitransportBootstrap::new(request(42, [0xAB; 16], RequestedProtocol::UdpFecR));
        assert_eq!(bootstrap.request().request_id, 42);
        assert!(!bootstrap.is_connected());
        assert!(bootstrap.response_pdu().is_none());
    }

    #[test]
    fn from_pdu_roundtrip() {
        let encoded = ironrdp_core::encode_vec(&request(99, [0xCC; 16], RequestedProtocol::UdpFecR)).expect("encode");
        let bootstrap = MultitransportBootstrap::from_pdu(&encoded).expect("decode");
        assert_eq!(bootstrap.request().request_id, 99);
        assert_eq!(bootstrap.request().security_cookie, [0xCC; 16]);
    }

    #[test]
    fn from_pdu_rejects_garbage() {
        let result = MultitransportBootstrap::from_pdu(&[0xFF, 0xFF]);
        assert!(result.is_err());
    }

    /// `UdpFecL` (lossy RDPEUDP + DTLS) is not implemented by this driver.
    /// `connect()` must reject it with `abort` before attempting the
    /// reliable transport it does implement, rather than silently
    /// connecting the wrong protocol and reporting success.
    #[tokio::test]
    async fn connect_rejects_unsupported_protocol_without_attempting_transport() {
        let mut bootstrap = MultitransportBootstrap::new(request(7, [0x11; 16], RequestedProtocol::UdpFecL));

        // An address nothing is listening on: if the guard didn't short-circuit
        // before connect_udp, this would hang until the handshake timeout
        // instead of returning immediately.
        let unreachable_addr: SocketAddr = "127.0.0.1:1".parse().expect("valid loopback address");

        let result = bootstrap
            .connect(unreachable_addr, "localhost".into(), ConnectionConfig::default())
            .await;

        // Checking the specific error kind (not just is_err()) is the point:
        // a driver that attempted the connection anyway would also end up
        // Err here, just as HandshakeTimeout instead, ten seconds later.
        let error = result.expect_err("UdpFecL must be rejected");
        assert!(matches!(
            error.kind(),
            crate::error::UdpTransportErrorKind::UnsupportedProtocol {
                requested: RequestedProtocol::UdpFecL
            }
        ));

        let response = bootstrap.response_pdu().expect("response set after connect");
        let decoded: MultitransportResponsePdu = ironrdp_core::decode(&response).expect("decode response");
        assert!(!decoded.is_success());
        assert!(!bootstrap.is_connected());
    }
}
