#[cfg(feature = "alloc")]
use alloc::string::String;
use core::fmt;

use crate::{
    InvalidFieldErr, NotEnoughBytesErr, OtherErr, UnexpectedMessageTypeErr, UnsupportedValueErr, UnsupportedVersionErr,
};

/// Result type for decode operations, wrapping a value or a DecodeError.
pub type DecodeResult<T> = Result<T, DecodeError>;

/// Custom error type for decode operations.
pub type DecodeError = ironrdp_error::Error<DecodeErrorKind>;

/// Enum representing different kinds of decode errors.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum DecodeErrorKind {
    /// Error when there are not enough bytes to decode.
    NotEnoughBytes {
        /// Number of bytes received.
        received: usize,
        /// Number of bytes expected.
        expected: usize,
    },
    /// Error when a field is invalid.
    InvalidField {
        /// Name of the invalid field.
        field: &'static str,
        /// Reason for invalidity.
        reason: &'static str,
    },
    /// Error when an unexpected message type is encountered.
    UnexpectedMessageType {
        /// The unexpected message type received.
        got: u8,
    },
    /// Error when an unsupported version is encountered.
    UnsupportedVersion {
        /// The unsupported version received.
        got: u8,
    },
    /// Error when an unsupported value is encountered (with allocation feature).
    #[cfg(feature = "alloc")]
    UnsupportedValue {
        /// Name of the unsupported value.
        name: &'static str,
        /// The unsupported value.
        value: String,
    },
    /// Error when an unsupported value is encountered (without allocation feature).
    #[cfg(not(feature = "alloc"))]
    UnsupportedValue {
        /// Name of the unsupported value.
        name: &'static str,
    },
    /// Generic error for other cases.
    Other {
        /// Description of the error.
        description: &'static str,
    },
}

#[cfg(feature = "std")]
impl std::error::Error for DecodeErrorKind {}

impl fmt::Display for DecodeErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotEnoughBytes { received, expected } => write!(
                f,
                "not enough bytes provided to decode: received {received} bytes, expected {expected} bytes"
            ),
            Self::InvalidField { field, reason } => {
                write!(f, "invalid `{field}`: {reason}")
            }
            Self::UnexpectedMessageType { got } => {
                write!(f, "invalid message type ({got})")
            }
            Self::UnsupportedVersion { got } => {
                write!(f, "unsupported version ({got})")
            }
            #[cfg(feature = "alloc")]
            Self::UnsupportedValue { name, value } => {
                write!(f, "unsupported {name} ({value})")
            }
            #[cfg(not(feature = "alloc"))]
            Self::UnsupportedValue { name } => {
                write!(f, "unsupported {name}")
            }
            Self::Other { description } => {
                write!(f, "other ({description})")
            }
        }
    }
}

impl NotEnoughBytesErr for DecodeError {
    fn not_enough_bytes(context: &'static str, received: usize, expected: usize) -> Self {
        Self::new(context, DecodeErrorKind::NotEnoughBytes { received, expected })
    }
}

impl InvalidFieldErr for DecodeError {
    fn invalid_field(context: &'static str, field: &'static str, reason: &'static str) -> Self {
        Self::new(context, DecodeErrorKind::InvalidField { field, reason })
    }
}

impl UnexpectedMessageTypeErr for DecodeError {
    fn unexpected_message_type(context: &'static str, got: u8) -> Self {
        Self::new(context, DecodeErrorKind::UnexpectedMessageType { got })
    }
}

impl UnsupportedVersionErr for DecodeError {
    fn unsupported_version(context: &'static str, got: u8) -> Self {
        Self::new(context, DecodeErrorKind::UnsupportedVersion { got })
    }
}

impl UnsupportedValueErr for DecodeError {
    #[cfg(feature = "alloc")]
    fn unsupported_value(context: &'static str, name: &'static str, value: String) -> Self {
        Self::new(context, DecodeErrorKind::UnsupportedValue { name, value })
    }
    #[cfg(not(feature = "alloc"))]
    fn unsupported_value(context: &'static str, name: &'static str) -> Self {
        Self::new(context, DecodeErrorKind::UnsupportedValue { name })
    }
}

impl OtherErr for DecodeError {
    fn other(context: &'static str, description: &'static str) -> Self {
        Self::new(context, DecodeErrorKind::Other { description })
    }
}
