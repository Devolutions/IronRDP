//! This module provides [`PduError`] and [`PduErrorKind`] types based on
//! reduced functionality IronRDP's `ironrdp-error` module.

use core::fmt;

#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum PduErrorKind {
    NotEnoughBytes { received: usize, expected: usize },
    InvalidMessage { field: &'static str, reason: &'static str },
    UnexpectedMessageKind { class: u8, kind: u8 },
    Other { description: &'static str },
}

#[cfg(feature = "std")]
impl std::error::Error for PduErrorKind {}

impl fmt::Display for PduErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotEnoughBytes { received, expected } => write!(
                f,
                "not enough bytes provided to decode: received {received} bytes, expected {expected} bytes"
            ),
            Self::InvalidMessage { field, reason } => {
                write!(f, "invalid `{field}`: {reason}")
            }
            Self::UnexpectedMessageKind { class, kind } => {
                write!(f, "invalid message kind (CLASS: {class}; KIND: {kind})")
            }
            Self::Other { description } => {
                write!(f, "{description}")
            }
        }
    }
}

pub type PduError = ironrdp_error::Error<PduErrorKind>;

pub trait PduErrorExt {
    fn not_enough_bytes(context: &'static str, received: usize, expected: usize) -> Self;
    fn invalid_message(context: &'static str, field: &'static str, reason: &'static str) -> Self;
    fn unexpected_message_kind(context: &'static str, class: u8, kind: u8) -> Self;
    fn other(context: &'static str, description: &'static str) -> Self;
}

impl PduErrorExt for PduError {
    fn not_enough_bytes(context: &'static str, received: usize, expected: usize) -> Self {
        Self::new(context, PduErrorKind::NotEnoughBytes { received, expected })
    }

    fn invalid_message(context: &'static str, field: &'static str, reason: &'static str) -> Self {
        Self::new(context, PduErrorKind::InvalidMessage { field, reason })
    }

    fn unexpected_message_kind(context: &'static str, class: u8, kind: u8) -> Self {
        Self::new(context, PduErrorKind::UnexpectedMessageKind { class, kind })
    }

    fn other(context: &'static str, description: &'static str) -> Self {
        Self::new(context, PduErrorKind::Other { description })
    }
}
