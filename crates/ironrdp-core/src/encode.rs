#[cfg(feature = "alloc")]
use alloc::string::String;
use core::fmt;

use crate::{
    InvalidFieldErr, NotEnoughBytesErr, OtherErr, UnexpectedMessageTypeErr, UnsupportedValueErr, UnsupportedVersionErr,
};

/// A result type for encoding operations, which can either succeed with a value of type `T`
/// or fail with an `EncodeError`.
pub type EncodeResult<T> = Result<T, EncodeError>;

/// An error type specifically for encoding operations, wrapping an `EncodeErrorKind`.
pub type EncodeError = ironrdp_error::Error<EncodeErrorKind>;

/// Represents the different kinds of errors that can occur during encoding operations.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum EncodeErrorKind {
    /// Indicates that there were not enough bytes to complete the encoding operation.
    NotEnoughBytes {
        /// The number of bytes actually received.
        received: usize,
        /// The number of bytes expected or required.
        expected: usize,
    },
    /// Indicates that a field in the data being encoded is invalid.
    InvalidField {
        /// The name of the invalid field.
        field: &'static str,
        /// The reason why the field is considered invalid.
        reason: &'static str,
    },
    /// Indicates that an unexpected message type was encountered during encoding.
    UnexpectedMessageType {
        /// The unexpected message type that was received.
        got: u8,
    },
    /// Indicates that an unsupported version was encountered during encoding.
    UnsupportedVersion {
        /// The unsupported version that was received.
        got: u8,
    },
    /// Indicates that an unsupported value was encountered during encoding.
    #[cfg(feature = "alloc")]
    UnsupportedValue {
        /// The name of the field or parameter with the unsupported value.
        name: &'static str,
        /// The unsupported value that was received.
        value: String,
    },
    /// Indicates that an unsupported value was encountered during encoding (no-alloc version).
    #[cfg(not(feature = "alloc"))]
    UnsupportedValue {
        /// The name of the field or parameter with the unsupported value.
        name: &'static str,
    },
    /// Represents any other error that doesn't fit into the above categories.
    Other {
        /// A description of the error.
        description: &'static str,
    },
}

#[cfg(feature = "std")]
impl std::error::Error for EncodeErrorKind {}

impl fmt::Display for EncodeErrorKind {
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

impl NotEnoughBytesErr for EncodeError {
    fn not_enough_bytes(context: &'static str, received: usize, expected: usize) -> Self {
        Self::new(context, EncodeErrorKind::NotEnoughBytes { received, expected })
    }
}

impl InvalidFieldErr for EncodeError {
    fn invalid_field(context: &'static str, field: &'static str, reason: &'static str) -> Self {
        Self::new(context, EncodeErrorKind::InvalidField { field, reason })
    }
}

impl UnexpectedMessageTypeErr for EncodeError {
    fn unexpected_message_type(context: &'static str, got: u8) -> Self {
        Self::new(context, EncodeErrorKind::UnexpectedMessageType { got })
    }
}

impl UnsupportedVersionErr for EncodeError {
    fn unsupported_version(context: &'static str, got: u8) -> Self {
        Self::new(context, EncodeErrorKind::UnsupportedVersion { got })
    }
}

impl UnsupportedValueErr for EncodeError {
    #[cfg(feature = "alloc")]
    fn unsupported_value(context: &'static str, name: &'static str, value: String) -> Self {
        Self::new(context, EncodeErrorKind::UnsupportedValue { name, value })
    }
    #[cfg(not(feature = "alloc"))]
    fn unsupported_value(context: &'static str, name: &'static str) -> Self {
        Self::new(context, EncodeErrorKind::UnsupportedValue { name })
    }
}

impl OtherErr for EncodeError {
    fn other(context: &'static str, description: &'static str) -> Self {
        Self::new(context, EncodeErrorKind::Other { description })
    }
}
