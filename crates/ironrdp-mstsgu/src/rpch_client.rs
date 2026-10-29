use ironrdp_tls::{CertificateValidation, CertificateValidationCallback};

use crate::packet_io::{GatewayStream, open_rpch_gateway_stream, parse_gateway_endpoint};
use crate::rpch::{RpchChannel, RpchHttpCredentials};
use crate::{Error, GwConnectTarget, GwErrorKind};

/// A bounded HTTPS RPC-over-HTTP v2 channel, not an authenticated TS Proxy RPC association.
///
/// This transport performs HTTP authentication and the [MS-RPCH] CONN sequence.
/// Callers own RPC bind authentication, packet signing, response verification and MS-TSGU calls.
/// HTTP authentication does not satisfy the MS-TSGU packet-integrity requirement.
/// This API is explicitly selected and is never an automatic fallback from [`crate::GwClient`].
///
/// Channel recycling is unsupported: the IN body is limited to 128 KiB, including RTS traffic.
/// An unsupported RTS sequence or exhausted window/body returns an error rather than opening a replacement channel.
/// Drop the client after any I/O error or cancelled operation; reads and writes are not cancellation-safe.
/// Callers must supply operation deadlines and send idle pings before the negotiated connection timeout.
/// HTTP cookie continuity and ongoing IN-channel error-response monitoring are not implemented.
///
/// [MS-RPCH]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpch/
pub struct RpchClient {
    channel: RpchChannel<GatewayStream, GatewayStream>,
}

impl RpchClient {
    /// Opens both HTTPS channels using strict gateway certificate validation.
    ///
    /// `rpc_target` is the RPC proxy backend authority, usually `gateway-host:3388`.
    /// It is not the destination RDP server from `target.server`.
    /// Credentials and proxy environment settings are shared with [`crate::GwClient`].
    pub async fn connect(target: &GwConnectTarget, rpc_target: &str) -> Result<Self, Error> {
        Self::connect_with_certificate_validation(target, rpc_target, CertificateValidation::Strict, None).await
    }

    /// Opens the channel with the same certificate policy and callback semantics as [`crate::GwClient`].
    pub async fn connect_with_certificate_validation(
        target: &GwConnectTarget,
        rpc_target: &str,
        certificate_validation: CertificateValidation,
        certificate_validation_callback: Option<CertificateValidationCallback>,
    ) -> Result<Self, Error> {
        if certificate_validation == CertificateValidation::DangerouslyAcceptInvalidCertificate
            && certificate_validation_callback.is_some()
        {
            return Err(Error::new(
                "certificate validation",
                GwErrorKind::InvalidCertificateValidation,
            ));
        }
        if !crate::rpc_transport::is_valid_target(rpc_target) {
            return Err(Error::new(
                "invalid rpch backend authority",
                GwErrorKind::InvalidGwTarget,
            ));
        }
        let gateway = parse_gateway_endpoint(&target.gw_endpoint)?;
        let (input, _) = open_rpch_gateway_stream(
            &target.gw_endpoint,
            certificate_validation,
            certificate_validation_callback.clone(),
        )
        .await?;
        let (out, _) = open_rpch_gateway_stream(
            &target.gw_endpoint,
            certificate_validation,
            certificate_validation_callback,
        )
        .await?;
        let channel = RpchChannel::open(
            out,
            input,
            &gateway.endpoint,
            rpc_target,
            RpchHttpCredentials {
                username: target.gw_user.clone(),
                password: target.gw_pass.clone(),
                smart_card: target.smart_card.as_deref().cloned(),
            },
        )
        .await?;
        Ok(Self { channel })
    }

    /// Returns the peer's remaining send credit; a fragment must be strictly smaller than this value.
    ///
    /// Receive peer flow-control acknowledgements with [`Self::receive_fragment`] to replenish credit.
    pub fn send_available_window(&self) -> u32 {
        self.channel.send_available_window()
    }

    /// Returns the negotiated IN-channel connection timeout for caller-driven idle pings.
    pub fn connection_timeout(&self) -> core::time::Duration {
        self.channel.connection_timeout()
    }

    /// Sends exactly one caller-encoded DCE/RPC fragment, preserving its authentication trailer.
    ///
    /// RTS traffic belongs to this transport and is rejected here.
    /// Request fragments must carry a packet-integrity trailer produced by the caller's RPC security provider.
    /// This method validates framing, not RPC authentication or negotiated fragment-size limits.
    pub async fn send_fragment(&mut self, fragment: &[u8]) -> Result<(), Error> {
        self.channel.send(fragment).await
    }

    /// Receives one unverified DCE/RPC fragment, processing peer pings and flow-control acknowledgements.
    ///
    /// The caller must verify the RPC authentication trailer before trusting the payload.
    /// Transport receive credit is released when the complete fragment is returned.
    pub async fn receive_fragment(&mut self) -> Result<Vec<u8>, Error> {
        self.channel.receive().await
    }

    /// Sends an RTS ping without consuming RPC send credit.
    pub async fn send_ping(&mut self) -> Result<(), Error> {
        self.channel.ping().await
    }
}
