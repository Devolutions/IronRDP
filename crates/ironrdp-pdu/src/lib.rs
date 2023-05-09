use core::fmt;

use cursor::WriteCursor;

use crate::cursor::ReadCursor;

#[macro_use]
mod macros;

pub mod codecs;
pub mod cursor;
pub mod gcc;
pub mod geometry;
pub mod input;
pub mod mcs;
pub mod nego;
pub mod padding;
pub mod pcb;
pub mod rdp;
pub mod tpdu;
pub mod tpkt;
pub mod utils;
pub mod x224;

pub(crate) mod basic_output;
pub(crate) mod ber;
pub(crate) mod crypto;
pub(crate) mod per;

pub use crate::basic_output::{bitmap, fast_path, surface_commands};
pub use crate::rdp::vc::dvc;

pub type Result<T> = core::result::Result<T, Error>;

#[non_exhaustive]
#[derive(Debug)]
pub enum Error {
    NotEnoughBytes {
        name: &'static str,
        received: usize,
        expected: usize,
    },
    InvalidMessage {
        name: &'static str,
        field: &'static str,
        reason: &'static str,
    },
    UnexpectedMessageType {
        name: &'static str,
        got: u8,
    },
    UnsupportedVersion {
        name: &'static str,
        got: u8,
    },
    Other {
        context: &'static str,
        reason: &'static str,
    },
    Custom(Box<dyn std::error::Error + Sync + Send + 'static>),
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let Error::Custom(e) = &self {
            Some(e.as_ref())
        } else {
            None
        }
    }
}

impl From<Error> for std::io::Error {
    fn from(error: Error) -> Self {
        std::io::Error::new(std::io::ErrorKind::Other, error)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotEnoughBytes {
                name,
                received,
                expected,
            } => write!(
                f,
                "not enough bytes provided to decode {name}: received {received} bytes, expected {expected} bytes"
            ),
            Error::InvalidMessage { name, field, reason } => {
                write!(f, "invalid `{field}` in {name}: {reason}")
            }
            Error::UnexpectedMessageType { name, got } => {
                write!(f, "invalid message type ({got}) for {name}")
            }
            Error::UnsupportedVersion { name, got } => {
                write!(f, "unsupported version ({got}) for {name}")
            }
            Error::Other { context, reason } => {
                write!(f, "{reason} ({context})")
            }
            Error::Custom(e) => {
                if f.alternate() {
                    write!(f, "{e}")?;

                    let mut next_source = e.source();
                    while let Some(e) = next_source {
                        write!(f, ", caused by: {e}")?;
                        next_source = e.source();
                    }
                } else {
                    write!(f, "custom")?;
                }

                Ok(())
            }
        }
    }
}

impl Error {
    pub fn custom<E>(e: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static,
    {
        Self::Custom(Box::new(e))
    }
}

/// An RDP PDU.
pub trait Pdu {
    /// Name associated to this PDU.
    const NAME: &'static str;
}

/// PDU that can be encoded into its binary form.
///
/// The resulting binary payload is a fully encoded PDU that may be sent to the peer.
///
/// This trait is object-safe and may be used in a dynamic context.
pub trait PduEncode {
    /// Encodes this PDU in-place using the provided `WriteCursor`.
    fn encode(&self, dst: &mut WriteCursor<'_>) -> Result<()>;

    /// Returns the associated PDU name associated.
    fn name(&self) -> &'static str;

    /// Computes the size in bytes for this PDU.
    fn size(&self) -> usize;
}

assert_obj_safe!(PduEncode);

/// Encodes the given PDU in-place into the provided buffer and returns the number of bytes written.
pub fn encode<T: PduEncode>(pdu: &T, dst: &mut [u8]) -> Result<usize> {
    let mut cursor = WriteCursor::new(dst);
    encode_cursor(pdu, &mut cursor)?;
    Ok(cursor.pos())
}

/// Same as `encode_pdu` but resizes the buffer when it is too small to fit the PDU.
pub fn encode_buf<T: PduEncode>(pdu: &T, buf: &mut Vec<u8>) -> Result<usize> {
    let pdu_size = pdu.size();

    if buf.len() < pdu_size {
        buf.resize(pdu_size, 0);
    }

    encode(pdu, buf)
}

/// Encodes the given PDU in-place using the provided `WriteCursor`.
pub fn encode_cursor<T: PduEncode>(pdu: &T, dst: &mut WriteCursor<'_>) -> Result<()> {
    pdu.encode(dst)
}

/// Gets the name of this PDU.
pub fn name<T: PduEncode>(pdu: &T) -> &'static str {
    pdu.name()
}

/// Computes the size in bytes for this PDU.
pub fn size<T: PduEncode>(pdu: &T) -> usize {
    pdu.size()
}

/// PDU that can be decoded from a binary input.
///
/// The binary payload must be a full PDU, not some subset of it.
pub trait PduDecode<'de>: Sized {
    fn decode(src: &mut ReadCursor<'de>) -> Result<Self>;
}

pub fn decode<'de, T: PduDecode<'de>>(src: &'de [u8]) -> Result<T> {
    let mut cursor = ReadCursor::new(src);
    T::decode(&mut cursor)
}

pub fn decode_cursor<'de, T: PduDecode<'de>>(src: &mut ReadCursor<'de>) -> Result<T> {
    T::decode(src)
}

/// Similar to `PduDecode` but unconditionally returns an owned type.
pub trait PduDecodeOwned: Sized {
    fn decode_owned(src: &mut ReadCursor<'_>) -> Result<Self>;
}

pub fn decode_owned<T: PduDecodeOwned>(src: &[u8]) -> Result<T> {
    let mut cursor = ReadCursor::new(src);
    T::decode_owned(&mut cursor)
}

pub fn decode_owned_cursor<T: PduDecodeOwned>(src: &mut ReadCursor<'_>) -> Result<T> {
    T::decode_owned(src)
}

/// Trait used to produce an owned version of a given PDU.
pub trait IntoOwnedPdu: Sized {
    type Owned: 'static;

    fn into_owned_pdu(self) -> Self::Owned;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum Action {
    FastPath = 0x00,
    X224 = 0x03,
}

impl Action {
    pub fn from_fp_output_header(fp_output_header: u8) -> core::result::Result<Self, u8> {
        match fp_output_header & 0b11 {
            0x00 => Ok(Self::FastPath),
            0x03 => Ok(Self::X224),
            unknown_action_bits => Err(unknown_action_bits),
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct PduInfo {
    pub action: Action,
    pub length: usize,
}

/// Finds next RDP PDU size by reading the next few bytes.
pub fn find_size(bytes: &[u8]) -> Result<Option<PduInfo>> {
    macro_rules! ensure_enough {
        ($bytes:expr, $len:expr) => {
            if $bytes.len() < $len {
                return Ok(None);
            }
        };
    }

    ensure_enough!(bytes, 1);
    let fp_output_header = bytes[0];

    let action =
        Action::from_fp_output_header(fp_output_header).map_err(|unknown_action| Error::UnexpectedMessageType {
            name: "fpOutputHeader",
            got: unknown_action,
        })?;

    match action {
        Action::X224 => {
            let tpkt = crate::tpkt::TpktHeader::read(&mut ReadCursor::new(bytes))?;

            Ok(Some(PduInfo {
                action,
                length: tpkt.packet_length(),
            }))
        }
        Action::FastPath => {
            ensure_enough!(bytes, 2);
            let a = bytes[1];

            let fast_path_length = if a & 0x80 != 0 {
                ensure_enough!(bytes, 3);
                let b = bytes[2];

                ((u16::from(a) & !0x80) << 8) + u16::from(b)
            } else {
                u16::from(a)
            };

            Ok(Some(PduInfo {
                action,
                length: usize::from(fast_path_length),
            }))
        }
    }
}

pub trait PduHint: core::fmt::Debug {
    /// Finds next PDU size by reading the next few bytes.
    fn find_size(&self, bytes: &[u8]) -> Result<Option<usize>>;
}

#[derive(Clone, Copy, Debug)]
pub struct X224Hint;

pub const X224_HINT: X224Hint = X224Hint;

impl PduHint for X224Hint {
    fn find_size(&self, bytes: &[u8]) -> Result<Option<usize>> {
        match find_size(bytes)? {
            Some(pdu_info) => {
                debug_assert_eq!(pdu_info.action, Action::X224);
                Ok(Some(pdu_info.length))
            }
            None => Ok(None),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FastPathHint;

pub const FAST_PATH_HINT: FastPathHint = FastPathHint;

impl PduHint for FastPathHint {
    fn find_size(&self, bytes: &[u8]) -> Result<Option<usize>> {
        match find_size(bytes)? {
            Some(pdu_info) => {
                debug_assert_eq!(pdu_info.action, Action::FastPath);
                Ok(Some(pdu_info.length))
            }
            None => Ok(None),
        }
    }
}

pub use legacy::*;

// TODO: Delete these traits at some point
mod legacy {
    use thiserror::Error;

    pub trait PduParsing {
        type Error;

        fn from_buffer(stream: impl std::io::Read) -> Result<Self, Self::Error>
        where
            Self: Sized;
        fn to_buffer(&self, stream: impl std::io::Write) -> Result<(), Self::Error>;
        fn buffer_length(&self) -> usize;
    }

    /// Blanket implementation for references to types implementing PduParsing. Only encoding is supported.
    ///
    /// This helps removing a few copies.
    impl<T: PduParsing> PduParsing for &T {
        type Error = T::Error;

        fn from_buffer(_: impl std::io::Read) -> Result<Self, Self::Error>
        where
            Self: Sized,
        {
            panic!("Can’t return a reference to a local value")
        }

        fn to_buffer(&self, stream: impl std::io::Write) -> Result<(), Self::Error> {
            T::to_buffer(self, stream)
        }

        fn buffer_length(&self) -> usize {
            T::buffer_length(self)
        }
    }

    pub trait PduBufferParsing<'a>: Sized {
        type Error;

        fn from_buffer(mut buffer: &'a [u8]) -> Result<Self, Self::Error> {
            Self::from_buffer_consume(&mut buffer)
        }
        fn from_buffer_consume(buffer: &mut &'a [u8]) -> Result<Self, Self::Error>;
        fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error>;
        fn buffer_length(&self) -> usize;
    }

    #[derive(Debug, Error)]
    pub enum RdpError {
        #[error("IO error")]
        IOError(#[from] std::io::Error),
        #[error("Surface Commands error")]
        FastPathError(#[from] crate::fast_path::FastPathError),
        #[error("Received invalid action code: {0}")]
        InvalidActionCode(u8),
    }
}
