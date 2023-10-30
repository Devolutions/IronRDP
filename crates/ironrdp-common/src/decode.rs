use core::fmt;

use crate::{legacy::PduResult, IntoOwned, ReadCursor};

pub type DecodeResult<T> = Result<T, DecodeError>;

pub type DecodeError = ironrdp_error::Error<DecodeErrorKind>;

#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum DecodeErrorKind {
    NotEnoughBytes { received: usize, expected: usize },
    InvalidField { field: &'static str },
    UnexpectedMessageType { got: u8 },
    UnsupportedVersion { got: u8 },
    Other,
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
            Self::InvalidField { field } => {
                write!(f, "invalid `{field}`")
            }
            Self::UnexpectedMessageType { got } => {
                write!(f, "invalid message type ({got})")
            }
            Self::UnsupportedVersion { got } => {
                write!(f, "unsupported version ({got})")
            }
            Self::Other => {
                write!(f, "other error")
            }
        }
    }
}

/// Structure that can be decoded from a binary input.
pub trait Decode<'de>: Sized {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self>;
}

pub fn decode<'de, T>(src: &'de [u8]) -> PduResult<T>
where
    T: Decode<'de>,
{
    let mut cursor = ReadCursor::new(src);
    T::decode(&mut cursor)
}

pub fn decode_cursor<'de, T>(src: &mut ReadCursor<'de>) -> PduResult<T>
where
    T: Decode<'de>,
{
    T::decode(src)
}

pub fn decode_owned<'de, T>(src: &'de [u8]) -> PduResult<T::Owned>
where
    T: Decode<'de> + IntoOwned,
{
    let mut cursor = ReadCursor::new(src);
    T::decode(&mut cursor).map(IntoOwned::into_owned)
}

pub fn decode_owned_cursor<'de, T>(src: &mut ReadCursor<'de>) -> PduResult<T::Owned>
where
    T: Decode<'de> + IntoOwned,
{
    T::decode(src).map(IntoOwned::into_owned)
}
