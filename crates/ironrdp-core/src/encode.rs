#[cfg(feature = "alloc")]
use alloc::string::String;
#[cfg(feature = "alloc")]
use alloc::{vec, vec::Vec};
use core::fmt;

#[cfg(feature = "alloc")]
use crate::WriteBuf;
use crate::{
    InvalidFieldErr, NotEnoughBytesErr, OtherErr, UnexpectedMessageTypeErr, UnsupportedValueErr, UnsupportedVersionErr,
    WriteCursor,
};

/// A result type for encoding operations, which can either succeed with a value of type `T`
/// or fail with an [`EncodeError`].
pub type EncodeResult<T> = Result<T, EncodeError>;

/// An error type specifically for encoding operations, wrapping an [`EncodeErrorKind`].
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
impl core::error::Error for EncodeErrorKind {}

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

/// PDU that can be encoded into its binary form.
///
/// The resulting binary payload is a fully encoded PDU that may be sent to the peer.
///
/// This trait is object-safe and may be used in a dynamic context.
pub trait Encode {
    /// Encodes this PDU in-place using the provided `WriteCursor`.
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()>;

    /// Returns the associated PDU name associated.
    fn name(&self) -> &'static str;

    /// Computes the size in bytes for this PDU.
    fn size(&self) -> usize;
}

crate::assert_obj_safe!(Encode);

/// Encodes the given PDU in-place into the provided buffer and returns the number of bytes written.
pub fn encode<T>(pdu: &T, dst: &mut [u8]) -> EncodeResult<usize>
where
    T: Encode + ?Sized,
{
    let mut cursor = WriteCursor::new(dst);
    encode_cursor(pdu, &mut cursor)?;
    Ok(cursor.pos())
}

/// Encodes the given PDU in-place using the provided `WriteCursor`.
pub fn encode_cursor<T>(pdu: &T, dst: &mut WriteCursor<'_>) -> EncodeResult<()>
where
    T: Encode + ?Sized,
{
    pdu.encode(dst)
}

/// Same as `encode` but resizes the buffer when it is too small to fit the PDU.
#[cfg(feature = "alloc")]
pub fn encode_buf<T>(pdu: &T, buf: &mut WriteBuf) -> EncodeResult<usize>
where
    T: Encode + ?Sized,
{
    let pdu_size = pdu.size();
    let dst = buf.unfilled_to(pdu_size);
    let written = encode(pdu, dst)?;
    debug_assert_eq!(written, pdu_size);
    buf.advance(written);
    Ok(written)
}

/// Same as `encode` but allocates and returns a new buffer each time.
///
/// This is a convenience function, but it’s not very resource efficient.
#[cfg(any(feature = "alloc", test))]
pub fn encode_vec<T>(pdu: &T) -> EncodeResult<Vec<u8>>
where
    T: Encode + ?Sized,
{
    let pdu_size = pdu.size();
    let mut buf = vec![0; pdu_size];
    let written = encode(pdu, buf.as_mut_slice())?;
    debug_assert_eq!(written, pdu_size);
    Ok(buf)
}

/// Gets the name of this PDU.
pub fn name<T: Encode>(pdu: &T) -> &'static str {
    pdu.name()
}

/// Computes the size in bytes for this PDU.
pub fn size<T: Encode>(pdu: &T) -> usize {
    pdu.size()
}

#[cfg(feature = "alloc")]
mod legacy {
    use super::{Encode, EncodeResult};
    use crate::WriteCursor;

    impl Encode for alloc::vec::Vec<u8> {
        fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
            ensure_size!(in: dst, size: self.len());

            dst.write_slice(self);
            Ok(())
        }

        /// Returns the associated PDU name associated.
        fn name(&self) -> &'static str {
            "legacy-pdu-encode"
        }

        /// Computes the size in bytes for this PDU.
        fn size(&self) -> usize {
            self.len()
        }
    }
}
