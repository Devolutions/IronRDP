//! Test-only hooks for exercising the gateway HTTP transport without a TLS listener.

use hyper::body::Bytes;

use crate::packet_io::{PacketIo, open_test_transport, select_gateway_tls_upgrade_for_test};
use crate::{Error, GwClient, GwConnectTarget, GwConsentCallback};

/// In-memory gateway transport used by the registered integration tests.
pub struct GatewayTransport(PacketIo);

impl GatewayTransport {
    /// Connect the client side of the mock OUT and IN HTTP connections.
    pub async fn connect(
        out_stream: tokio::io::DuplexStream,
        in_stream: tokio::io::DuplexStream,
    ) -> Result<Self, String> {
        open_test_transport(out_stream, in_stream)
            .await
            .map(Self)
            .map_err(|error| error.to_string())
    }

    /// Send one MS-TSGU packet to the mock gateway.
    pub async fn send_packet(&mut self, packet: &[u8]) -> Result<(), String> {
        self.0.send_bytes(packet).await.map_err(|error| error.to_string())
    }

    /// Read one MS-TSGU packet from the mock gateway.
    pub async fn read_packet(&mut self) -> Result<Option<Bytes>, String> {
        self.0.read_packet_buf().await.map_err(|error| error.to_string())
    }

    /// Finish the mock gateway IN request body.
    pub async fn close(&mut self) -> Result<(), String> {
        self.0.close().await.map_err(|error| error.to_string())
    }

    /// Establish a gateway tunnel through this test transport.
    pub async fn connect_tunnel(
        self,
        target: GwConnectTarget,
        client_name: &str,
        server_port: u16,
        consent_callback: Option<&mut GwConsentCallback<'_>>,
    ) -> Result<GwClient, Error> {
        GwClient::connect_ws(target, client_name, server_port, self.0, consent_callback).await
    }
}

/// Evaluate a consent message received during tunnel creation.
pub fn evaluate_consent_message(
    consent_message: &[u8],
    consent_callback: Option<&mut GwConsentCallback<'_>>,
) -> Result<(), Error> {
    crate::evaluate_consent_message(consent_message, consent_callback)
}

/// Report which TLS upgrader a gateway connection selects.
pub fn tls_upgrade_selection(
    certificate_validation: ironrdp_tls::CertificateValidation,
    certificate_validation_callback: Option<ironrdp_tls::CertificateValidationCallback>,
) -> (ironrdp_tls::CertificateValidation, bool) {
    let selection = select_gateway_tls_upgrade_for_test(certificate_validation, certificate_validation_callback);
    (selection.certificate_validation, selection.uses_callback)
}
