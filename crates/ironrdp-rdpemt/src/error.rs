//! Error types for the multitransport tunnel layer.
//!
//! These surface at the public API boundary of [`RdpemtTunnel`], and carry both
//! wire-level decode and encode failures from `ironrdp-core` and
//! protocol-level failures from the tunnel state machine.
//!
//! [`RdpemtTunnel`]: crate::RdpemtTunnel

use core::fmt;

use ironrdp_core::{DecodeError, EncodeError};

pub type RdpemtResult<T> = Result<T, RdpemtError>;

pub type RdpemtError = ironrdp_error::Error<RdpemtErrorKind>;

#[non_exhaustive]
#[derive(Debug)]
pub enum RdpemtErrorKind {
    /// A PDU could not be decoded.
    Decode(DecodeError),

    /// A PDU could not be encoded.
    Encode(EncodeError),

    /// The tunnel is not in a state where this operation is meaningful.
    ///
    /// Sending before the tunnel is established, or receiving a create request
    /// on an established tunnel, both land here.
    InvalidState,
}

impl fmt::Display for RdpemtErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(_) => write!(f, "decode error"),
            Self::Encode(_) => write!(f, "encode error"),
            Self::InvalidState => write!(f, "tunnel is in the wrong state for this operation"),
        }
    }
}

#[cfg(feature = "std")]
impl core::error::Error for RdpemtErrorKind {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::Encode(error) => Some(error),
            Self::InvalidState => None,
        }
    }
}

pub trait RdpemtErrorExt {
    fn decode(error: DecodeError) -> Self;
    fn encode(error: EncodeError) -> Self;
    fn invalid_state(context: &'static str) -> Self;
}

impl RdpemtErrorExt for RdpemtError {
    #[track_caller]
    fn decode(error: DecodeError) -> Self {
        Self::new("decode error", RdpemtErrorKind::Decode(error))
    }

    #[track_caller]
    fn encode(error: EncodeError) -> Self {
        Self::new("encode error", RdpemtErrorKind::Encode(error))
    }

    #[track_caller]
    fn invalid_state(context: &'static str) -> Self {
        Self::new(context, RdpemtErrorKind::InvalidState)
    }
}
