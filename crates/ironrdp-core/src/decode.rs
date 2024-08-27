#[cfg(feature = "alloc")]
use alloc::string::String;
use core::fmt;

use crate::{
    InvalidFieldErr, NotEnoughBytesErr, OtherErr, ReadCursor, UnexpectedMessageTypeErr, UnsupportedValueErr,
    UnsupportedVersionErr,
};

/// A result type for decoding operations, which can either succeed with a value of type `T`
/// or fail with an [`DecodeError`].
pub type DecodeResult<T> = Result<T, DecodeError>;

/// An error type specifically for encoding operations, wrapping an [`DecodeErrorKind`].
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

/// Trait for types that can be decoded from a byte stream.
///
/// This trait is implemented by types that can be deserialized from a sequence of bytes.
pub trait Decode<'de>: Sized {
    /// Decodes an instance of `Self` from the given byte stream.
    ///
    /// # Arguments
    ///
    /// * `src` - A mutable reference to a `ReadCursor` containing the bytes to decode.
    ///
    /// # Returns
    ///
    /// Returns a `DecodeResult<Self>`, which is either the successfully decoded instance
    /// or a `DecodeError` if decoding fails.
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self>;
}

/// Decodes a value of type `T` from a byte slice.
///
/// This function creates a `ReadCursor` from the input byte slice and uses it to decode
/// a value of type `T` that implements the `Decode` trait.
///
/// # Arguments
///
/// * `src` - A byte slice containing the data to be decoded.
///
/// # Returns
///
/// Returns a `DecodeResult<T>`, which is either the successfully decoded value
/// or a `DecodeError` if decoding fails.
pub fn decode<'de, T>(src: &'de [u8]) -> DecodeResult<T>
where
    T: Decode<'de>,
{
    let mut cursor = ReadCursor::new(src);
    T::decode(&mut cursor)
}

/// Decodes a value of type `T` from a `ReadCursor`.
///
/// This function uses the provided `ReadCursor` to decode a value of type `T`
/// that implements the `Decode` trait.
///
/// # Arguments
///
/// * `src` - A mutable reference to a `ReadCursor` containing the bytes to be decoded.
///
/// # Returns
///
/// Returns a `DecodeResult<T>`, which is either the successfully decoded value
/// or a `DecodeError` if decoding fails.
pub fn decode_cursor<'de, T>(src: &mut ReadCursor<'de>) -> DecodeResult<T>
where
    T: Decode<'de>,
{
    T::decode(src)
}

/// Similar to `Decode` but unconditionally returns an owned type.
pub trait DecodeOwned: Sized {
    /// Decodes an instance of `Self` from the given byte stream.
    ///
    /// # Arguments
    ///
    /// * `src` - A mutable reference to a `ReadCursor` containing the bytes to decode.
    ///
    /// # Returns
    ///
    /// Returns a `DecodeResult<Self>`, which is either the successfully decoded instance
    /// or a `DecodeError` if decoding fails.
    fn decode_owned(src: &mut ReadCursor<'_>) -> DecodeResult<Self>;
}

/// Decodes an owned value of type `T` from a byte slice.
///
/// This function creates a `ReadCursor` from the input byte slice and uses it to decode
/// an owned value of type `T` that implements the `DecodeOwned` trait.
///
/// # Arguments
///
/// * `src` - A byte slice containing the data to be decoded.
///
/// # Returns
///
/// Returns a `DecodeResult<T>`, which is either the successfully decoded owned value
/// or a `DecodeError` if decoding fails.
pub fn decode_owned<T: DecodeOwned>(src: &[u8]) -> DecodeResult<T> {
    let mut cursor = ReadCursor::new(src);
    T::decode_owned(&mut cursor)
}

/// Decodes an owned value of type `T` from a `ReadCursor`.
///
/// This function uses the provided `ReadCursor` to decode an owned value of type `T`
/// that implements the `DecodeOwned` trait.
///
/// # Arguments
///
/// * `src` - A mutable reference to a `ReadCursor` containing the bytes to be decoded.
///
/// # Returns
///
/// Returns a `DecodeResult<T>`, which is either the successfully decoded owned value
/// or a `DecodeError` if decoding fails.
pub fn decode_owned_cursor<T: DecodeOwned>(src: &mut ReadCursor<'_>) -> DecodeResult<T> {
    T::decode_owned(src)
}
