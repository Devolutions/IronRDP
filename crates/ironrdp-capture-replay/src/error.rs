use ironrdp_capture::CaptureError;
use thiserror::Error;

/// Errors that make an offline replay unreliable.
#[derive(Debug, Error)]
pub enum ReplayError {
    /// Capture decoding failed.
    #[error(transparent)]
    Capture(#[from] CaptureError),
    /// The capture does not contain a complete RDP connection sequence.
    #[error("capture does not contain enough negotiated RDP state")]
    MissingRdpState,
    /// The decrypted stream begins with CredSSP (NLA) messages that RDP framing cannot skip.
    #[error("capture begins with CredSSP messages that replay cannot frame")]
    CredsspPrefix,
    /// The server did not assign all requested static channels.
    #[error("capture does not contain server-assigned static channel IDs")]
    MissingChannelMap,
    /// The MCS user channel could not be recovered.
    #[error("capture does not contain the client MCS user channel")]
    MissingUserChannel,
    /// The active RDP share identifier could not be recovered.
    #[error("capture does not contain an activated RDP share ID")]
    MissingShareId,
    /// Captured connection state cannot configure an offline router safely.
    #[error("capture contains contradictory routing state")]
    ContradictoryRoutingState,
    /// The capture requires the DRDYNVC static channel.
    #[error("capture does not contain the DRDYNVC static channel")]
    MissingDrdynvcChannel,
    /// A captured dynamic channel could not be attached.
    #[error("captured dynamic channel could not be attached")]
    DynamicChannelAttachment,
}

impl ReplayError {
    /// Stable reason code when this error is a documented replay limitation.
    ///
    /// Returns `None` for unreadable input and failures that indicate a replay defect.
    pub fn unsupported_reason(&self) -> Option<&'static str> {
        match self {
            Self::Capture(error) => match error {
                CaptureError::UnsupportedTransport => Some("unsupported-transport"),
                CaptureError::StandardSecurity => Some("standard-security"),
                CaptureError::UnsupportedTls => Some("unsupported-tls"),
                CaptureError::MissingTunneledTlsSecret => Some("missing-tunneled-tls-secret"),
                CaptureError::TlsKeyUpdate => Some("tls-key-update"),
                CaptureError::MissingTunneledRdp => Some("missing-rdp-state"),
                CaptureError::Io(_)
                | CaptureError::Pcap(_)
                | CaptureError::MissingTcpFlow
                | CaptureError::MissingTlsSecret
                | CaptureError::TlsAuthentication
                | CaptureError::GatewayFraming(_) => None,
            },
            Self::MissingRdpState => Some("missing-rdp-state"),
            Self::CredsspPrefix => Some("credssp-prefix"),
            Self::MissingDrdynvcChannel => Some("missing-drdynvc-channel"),
            Self::DynamicChannelAttachment => Some("dynamic-channel-attachment"),
            Self::MissingChannelMap
            | Self::MissingUserChannel
            | Self::MissingShareId
            | Self::ContradictoryRoutingState => None,
        }
    }
}
