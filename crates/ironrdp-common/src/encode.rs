#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::fmt;

use crate::WriteCursor;

pub type EncodeResult<T> = Result<T, EncodeError>;

pub type EncodeError = ironrdp_error::Error<EncodeErrorKind>;

#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum EncodeErrorKind {
    NotEnoughSpace { actual: usize, required: usize },
    InvalidField { field: &'static str },
    Other,
}

#[cfg(feature = "std")]
impl std::error::Error for EncodeErrorKind {}

impl fmt::Display for EncodeErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotEnoughSpace { actual, required } => write!(
                f,
                "not enough space to encode: has {actual} bytes, required {required} bytes"
            ),
            Self::InvalidField { field } => {
                write!(f, "invalid `{field}`")
            }
            Self::Other => {
                write!(f, "other error")
            }
        }
    }
}

/// Structure that can be encoded into a binary format.
///
/// This trait is object-safe and may be used in a dynamic context.
pub trait Encode {
    /// Encodes this structure in-place using the provided `WriteCursor`.
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()>;

    /// Computes the size in bytes for this structure.
    fn size(&self) -> usize;
}

assert_obj_safe!(Encode);

/// Encodes the given structure in-place into the provided buffer and returns the number of bytes written.
pub fn encode<T>(pdu: &T, dst: &mut [u8]) -> EncodeResult<usize>
where
    T: Encode + ?Sized,
{
    let mut cursor = WriteCursor::new(dst);
    encode_cursor(pdu, &mut cursor)?;
    Ok(cursor.pos())
}

/// Encodes the given structure in-place using the provided `WriteCursor`.
pub fn encode_cursor<T>(pdu: &T, dst: &mut WriteCursor<'_>) -> EncodeResult<()>
where
    T: Encode + ?Sized,
{
    pdu.encode(dst)
}

/// Same as `encode` but resizes the buffer when it is too small to fit the whole structure.
#[cfg(feature = "alloc")]
pub fn encode_buf<T>(pdu: &T, buf: &mut crate::WriteBuf) -> EncodeResult<usize>
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
#[cfg(feature = "alloc")]
pub fn encode_vec<T>(pdu: &T) -> EncodeResult<Vec<u8>>
where
    T: Encode + ?Sized,
{
    let pdu_size = pdu.size();
    let mut buf = alloc::vec![0; pdu_size];
    let written = encode(pdu, buf.as_mut_slice())?;
    debug_assert_eq!(written, pdu_size);
    Ok(buf)
}

/// Computes the size in bytes for this structure.
pub fn size<T: Encode>(pdu: &T) -> usize {
    pdu.size()
}
