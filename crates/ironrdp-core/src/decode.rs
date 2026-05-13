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

/// Structured decode errors carry the fields needed to describe the failure,
/// including a byte `offset` when the error can be associated with
/// a position in the input stream.
///
/// The `offset` is the cursor position at, or nearest to, where the error was
/// detected. Producers without a stream cursor (a `try_from` on a primitive,
/// a constructor, a validator) pass `0`.
///
/// [`DecodeErrorKind::Other`] is reserved for errors that do not fit one of the
/// structured variants and therefore does not carry an offset.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum DecodeErrorKind {
    /// Error when there are not enough bytes to decode.
    NotEnoughBytes {
        /// Number of bytes received.
        received: usize,
        /// Number of bytes expected.
        expected: usize,
        /// Byte offset in the input stream where the shortage was detected.
        ///
        /// `0` indicates that the producer had no stream cursor in scope
        /// (a `try_from` on a primitive, a constructor, a validator).
        offset: usize,
    },
    /// Error when a field is invalid.
    InvalidField {
        /// Name of the invalid field.
        field: &'static str,
        /// Reason for invalidity.
        reason: &'static str,
        /// Byte offset in the input stream where the invalid field was decoded.
        ///
        /// `0` indicates that the producer had no stream cursor in scope.
        offset: usize,
    },
    /// Error when an unexpected message type is encountered.
    UnexpectedMessageType {
        /// The unexpected message type received.
        got: u8,
        /// Byte offset in the input stream where the unexpected type was read.
        ///
        /// `0` indicates that the producer had no stream cursor in scope.
        offset: usize,
    },
    /// Error when an unsupported version is encountered.
    UnsupportedVersion {
        /// The unsupported version received.
        got: u8,
        /// Byte offset in the input stream where the unsupported version was read.
        ///
        /// `0` indicates that the producer had no stream cursor in scope.
        offset: usize,
    },
    /// Error when an unsupported value is encountered (with allocation feature).
    #[cfg(feature = "alloc")]
    UnsupportedValue {
        /// Name of the unsupported value.
        name: &'static str,
        /// The unsupported value.
        value: String,
        /// Byte offset in the input stream where the unsupported value was read.
        ///
        /// `0` indicates that the producer had no stream cursor in scope.
        offset: usize,
    },
    /// Error when an unsupported value is encountered (without allocation feature).
    #[cfg(not(feature = "alloc"))]
    UnsupportedValue {
        /// Name of the unsupported value.
        name: &'static str,
        /// Byte offset in the input stream where the unsupported value was read.
        ///
        /// `0` indicates that the producer had no stream cursor in scope.
        offset: usize,
    },
    /// Generic error for other cases.
    ///
    /// Does not carry an offset: producers of this variant typically do not
    /// have stream-cursor access, and the variant exists precisely for those
    /// cases.
    Other {
        /// Description of the error.
        description: &'static str,
    },
}

#[cfg(feature = "std")]
impl core::error::Error for DecodeErrorKind {}

impl fmt::Display for DecodeErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotEnoughBytes {
                received,
                expected,
                offset,
            } => write!(
                f,
                "not enough bytes provided to decode at offset {offset}: received {received} bytes, expected {expected} bytes"
            ),
            Self::InvalidField { field, reason, offset } => {
                write!(f, "invalid `{field}` at offset {offset}: {reason}")
            }
            Self::UnexpectedMessageType { got, offset } => {
                write!(f, "invalid message type ({got}) at offset {offset}")
            }
            Self::UnsupportedVersion { got, offset } => {
                write!(f, "unsupported version ({got}) at offset {offset}")
            }
            #[cfg(feature = "alloc")]
            Self::UnsupportedValue { name, value, offset } => {
                write!(f, "unsupported {name} ({value}) at offset {offset}")
            }
            #[cfg(not(feature = "alloc"))]
            Self::UnsupportedValue { name, offset } => {
                write!(f, "unsupported {name} at offset {offset}")
            }
            Self::Other { description } => {
                write!(f, "other ({description})")
            }
        }
    }
}

impl NotEnoughBytesErr for DecodeError {
    #[track_caller]
    fn not_enough_bytes(context: &'static str, received: usize, expected: usize, offset: usize) -> Self {
        Self::new(
            context,
            DecodeErrorKind::NotEnoughBytes {
                received,
                expected,
                offset,
            },
        )
    }
}

impl InvalidFieldErr for DecodeError {
    #[track_caller]
    fn invalid_field(context: &'static str, field: &'static str, reason: &'static str, offset: usize) -> Self {
        Self::new(context, DecodeErrorKind::InvalidField { field, reason, offset })
    }
}

impl UnexpectedMessageTypeErr for DecodeError {
    #[track_caller]
    fn unexpected_message_type(context: &'static str, got: u8, offset: usize) -> Self {
        Self::new(context, DecodeErrorKind::UnexpectedMessageType { got, offset })
    }
}

impl UnsupportedVersionErr for DecodeError {
    #[track_caller]
    fn unsupported_version(context: &'static str, got: u8, offset: usize) -> Self {
        Self::new(context, DecodeErrorKind::UnsupportedVersion { got, offset })
    }
}

impl UnsupportedValueErr for DecodeError {
    #[cfg(feature = "alloc")]
    #[track_caller]
    fn unsupported_value(context: &'static str, name: &'static str, value: String, offset: usize) -> Self {
        Self::new(context, DecodeErrorKind::UnsupportedValue { name, value, offset })
    }
    #[cfg(not(feature = "alloc"))]
    #[track_caller]
    fn unsupported_value(context: &'static str, name: &'static str, offset: usize) -> Self {
        Self::new(context, DecodeErrorKind::UnsupportedValue { name, offset })
    }
}

impl OtherErr for DecodeError {
    #[track_caller]
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
