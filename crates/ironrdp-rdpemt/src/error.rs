//! Error types for the multitransport tunnel layer.
//!
//! These surface at the public API boundary of [`RdpemtTunnel`], and carry both
//! wire-level decode and encode failures from `ironrdp-core` and
//! protocol-level failures from the tunnel state machine.
//!
//! [`RdpemtTunnel`]: crate::RdpemtTunnel

use core::fmt;

use ironrdp_core::{DecodeError, EncodeError};

use crate::pdu::TunnelAction;

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

    /// The Action field carries a value this implementation does not know.
    UnknownAction { action: u8 },

    /// The Flags field is non-zero.
    ///
    /// [MS-RDPEMT] 2.2.1.1 requires it to be set to zero.
    NonZeroFlags { flags: u8 },

    /// HeaderLength is below the four-byte minimum.
    HeaderTooSmall { length: u8 },

    /// HeaderLength is not four on a PDU that requires exactly four.
    ///
    /// [MS-RDPEMT] 3.1.5.3 fixes it at 0x04 for tunnel create request and
    /// response.
    InvalidHeaderLength { action: TunnelAction, length: u8 },

    /// The peer rejected the tunnel creation request.
    TunnelRejected { hr_response: u32 },

    /// The request ID or security cookie did not match the pending request.
    CookieMismatch,
}

impl fmt::Display for RdpemtErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(_) => write!(f, "decode error"),
            Self::Encode(_) => write!(f, "encode error"),
            Self::InvalidState => write!(f, "tunnel is in the wrong state for this operation"),
            Self::UnknownAction { action } => write!(f, "unknown tunnel action: {action:#x}"),
            Self::NonZeroFlags { flags } => write!(f, "non-zero flags in tunnel header: {flags:#x}"),
            Self::HeaderTooSmall { length } => {
                write!(f, "header length too small: {length}, minimum is 4")
            }
            Self::InvalidHeaderLength { action, length } => {
                write!(f, "header length must be 4 for {action:?}, got {length}")
            }
            Self::TunnelRejected { hr_response } => {
                write!(f, "tunnel creation rejected: HRESULT {hr_response:#010x}")
            }
            Self::CookieMismatch => write!(f, "request ID or security cookie mismatch"),
        }
    }
}

#[cfg(feature = "std")]
impl core::error::Error for RdpemtErrorKind {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::Encode(error) => Some(error),
            Self::InvalidState
            | Self::UnknownAction { .. }
            | Self::NonZeroFlags { .. }
            | Self::HeaderTooSmall { .. }
            | Self::InvalidHeaderLength { .. }
            | Self::TunnelRejected { .. }
            | Self::CookieMismatch => None,
        }
    }
}

pub trait RdpemtErrorExt {
    fn decode(error: DecodeError) -> Self;
    fn encode(error: EncodeError) -> Self;
    fn invalid_state(context: &'static str) -> Self;
    fn unknown_action(context: &'static str, action: u8) -> Self;
    fn non_zero_flags(context: &'static str, flags: u8) -> Self;
    fn header_too_small(context: &'static str, length: u8) -> Self;
    fn invalid_header_length(context: &'static str, action: TunnelAction, length: u8) -> Self;
    fn tunnel_rejected(context: &'static str, hr_response: u32) -> Self;
    fn cookie_mismatch(context: &'static str) -> Self;
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

    #[track_caller]
    fn unknown_action(context: &'static str, action: u8) -> Self {
        Self::new(context, RdpemtErrorKind::UnknownAction { action })
    }

    #[track_caller]
    fn non_zero_flags(context: &'static str, flags: u8) -> Self {
        Self::new(context, RdpemtErrorKind::NonZeroFlags { flags })
    }

    #[track_caller]
    fn header_too_small(context: &'static str, length: u8) -> Self {
        Self::new(context, RdpemtErrorKind::HeaderTooSmall { length })
    }

    #[track_caller]
    fn invalid_header_length(context: &'static str, action: TunnelAction, length: u8) -> Self {
        Self::new(context, RdpemtErrorKind::InvalidHeaderLength { action, length })
    }

    #[track_caller]
    fn tunnel_rejected(context: &'static str, hr_response: u32) -> Self {
        Self::new(context, RdpemtErrorKind::TunnelRejected { hr_response })
    }

    #[track_caller]
    fn cookie_mismatch(context: &'static str) -> Self {
        Self::new(context, RdpemtErrorKind::CookieMismatch)
    }
}
