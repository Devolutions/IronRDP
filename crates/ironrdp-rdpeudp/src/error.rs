//! Error types for the RDP-UDP connection state machine.
//!
//! These surface at the public API boundary of [`RdpeudpConnection`], and carry
//! both wire-level decode and encode failures from `ironrdp-core` and
//! protocol-level failures from the state machine itself.
//!
//! [`RdpeudpConnection`]: crate::RdpeudpConnection

use core::fmt;

use ironrdp_core::{DecodeError, EncodeError};

pub type RdpeudpResult<T> = Result<T, RdpeudpError>;

pub type RdpeudpError = ironrdp_error::Error<RdpeudpErrorKind>;

#[non_exhaustive]
#[derive(Debug)]
pub enum RdpeudpErrorKind {
    /// A datagram could not be decoded.
    Decode(DecodeError),

    /// A datagram could not be encoded.
    Encode(EncodeError),

    /// The connection is not in a state where this operation is meaningful.
    ///
    /// Sending before the handshake completes, or handling a datagram after the
    /// connection is closed, both land here.
    InvalidState,

    /// The send window is full.
    ///
    /// The caller should drain [`poll_transmit`] and wait for acknowledgements
    /// to open the window before retrying.
    ///
    /// [`poll_transmit`]: crate::RdpeudpConnection::poll_transmit
    SendBufferFull,

    /// The connection has been closed, locally or by the idle timeout.
    ConnectionClosed,

    /// A datagram decoded cleanly but is not valid for the current state.
    InvalidPacket { reason: &'static str },
}

impl fmt::Display for RdpeudpErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(_) => write!(f, "decode error"),
            Self::Encode(_) => write!(f, "encode error"),
            Self::InvalidState => write!(f, "connection is in the wrong state for this operation"),
            Self::SendBufferFull => write!(f, "send buffer is full"),
            Self::ConnectionClosed => write!(f, "connection is closed"),
            Self::InvalidPacket { reason } => write!(f, "invalid packet: {reason}"),
        }
    }
}

#[cfg(feature = "std")]
impl core::error::Error for RdpeudpErrorKind {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::Encode(error) => Some(error),
            Self::InvalidState | Self::SendBufferFull | Self::ConnectionClosed | Self::InvalidPacket { .. } => None,
        }
    }
}

pub trait RdpeudpErrorExt {
    fn decode(error: DecodeError) -> Self;
    fn encode(error: EncodeError) -> Self;
    fn invalid_state(context: &'static str) -> Self;
    fn send_buffer_full(context: &'static str) -> Self;
    fn connection_closed(context: &'static str) -> Self;
    fn invalid_packet(context: &'static str, reason: &'static str) -> Self;
}

impl RdpeudpErrorExt for RdpeudpError {
    #[track_caller]
    fn decode(error: DecodeError) -> Self {
        Self::new("decode error", RdpeudpErrorKind::Decode(error))
    }

    #[track_caller]
    fn encode(error: EncodeError) -> Self {
        Self::new("encode error", RdpeudpErrorKind::Encode(error))
    }

    #[track_caller]
    fn invalid_state(context: &'static str) -> Self {
        Self::new(context, RdpeudpErrorKind::InvalidState)
    }

    #[track_caller]
    fn send_buffer_full(context: &'static str) -> Self {
        Self::new(context, RdpeudpErrorKind::SendBufferFull)
    }

    #[track_caller]
    fn connection_closed(context: &'static str) -> Self {
        Self::new(context, RdpeudpErrorKind::ConnectionClosed)
    }

    #[track_caller]
    fn invalid_packet(context: &'static str, reason: &'static str) -> Self {
        Self::new(context, RdpeudpErrorKind::InvalidPacket { reason })
    }
}
