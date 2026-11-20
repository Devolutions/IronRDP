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
