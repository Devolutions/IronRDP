#![allow(clippy::arithmetic_side_effects)] // FIXME: remove
#![allow(clippy::cast_lossless)] // FIXME: remove
#![allow(clippy::cast_possible_truncation)] // FIXME: remove
#![allow(clippy::cast_possible_wrap)] // FIXME: remove
#![allow(clippy::cast_sign_loss)] // FIXME: remove

use core::fmt;
use ironrdp_core::{ReadCursor, WriteCursor};

#[cfg(feature = "alloc")]
use ironrdp_core::WriteBuf;
use ironrdp_error::Source;

#[macro_use]
mod macros;

pub mod codecs;
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
pub mod utf16;
pub mod utils;
pub mod x224;

pub(crate) mod basic_output;
pub(crate) mod ber;
pub(crate) mod crypto;
pub(crate) mod per;

pub use crate::basic_output::{bitmap, fast_path, pointer, surface_commands};
pub use crate::rdp::vc::dvc;

pub type DecodeResult<T> = Result<T, DecodeError>;

pub type DecodeError = ironrdp_error::Error<DecodeErrorKind>;

#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum DecodeErrorKind {
    NotEnoughBytes { received: usize, expected: usize },
    InvalidField { field: &'static str, reason: &'static str },
    UnexpectedMessageType { got: u8 },
    UnsupportedVersion { got: u8 },
    UnsupportedValue { name: &'static str, value: String },
    Other { description: &'static str },
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
            Self::UnsupportedValue { name, value } => {
                write!(f, "unsupported {name} ({value})")
            }
            Self::Other { description } => {
                write!(f, "other ({description})")
            }
        }
    }
}

pub trait WithSource {
    #[must_use]
    fn with_source<E: Source>(self, source: E) -> Self;
}

impl WithSource for DecodeError {
    fn with_source<E: Source>(self, source: E) -> Self {
        self.with_source(source)
    }
}

impl WithSource for EncodeError {
    fn with_source<E: Source>(self, source: E) -> Self {
        self.with_source(source)
    }
}

impl WithSource for PduError {
    fn with_source<E: Source>(self, source: E) -> Self {
        self.with_source(source)
    }
}

pub trait NotEnoughBytesErr {
    fn not_enough_bytes(context: &'static str, received: usize, expected: usize) -> Self;
}

impl NotEnoughBytesErr for DecodeError {
    fn not_enough_bytes(context: &'static str, received: usize, expected: usize) -> Self {
        Self::new(context, DecodeErrorKind::NotEnoughBytes { received, expected })
    }
}

impl NotEnoughBytesErr for EncodeError {
    fn not_enough_bytes(context: &'static str, received: usize, expected: usize) -> Self {
        Self::new(context, EncodeErrorKind::NotEnoughBytes { received, expected })
    }
}

pub fn not_enough_bytes_err<T: NotEnoughBytesErr>(context: &'static str, received: usize, expected: usize) -> T {
    T::not_enough_bytes(context, received, expected)
}

pub trait InvalidFieldErr {
    fn invalid_field(context: &'static str, field: &'static str, reason: &'static str) -> Self;
}

impl InvalidFieldErr for DecodeError {
    fn invalid_field(context: &'static str, field: &'static str, reason: &'static str) -> Self {
        Self::new(context, DecodeErrorKind::InvalidField { field, reason })
    }
}

impl InvalidFieldErr for EncodeError {
    fn invalid_field(context: &'static str, field: &'static str, reason: &'static str) -> Self {
        Self::new(context, EncodeErrorKind::InvalidField { field, reason })
    }
}

pub fn invalid_field_err_with_source<T: InvalidFieldErr + WithSource, E: Source>(
    context: &'static str,
    field: &'static str,
    reason: &'static str,
    source: E,
) -> T {
    T::invalid_field(context, field, reason).with_source(source)
}

pub fn invalid_field_err<T: InvalidFieldErr>(context: &'static str, field: &'static str, reason: &'static str) -> T {
    T::invalid_field(context, field, reason)
}

pub trait UnexpectedMessageTypeErr {
    fn unexpected_message_type(context: &'static str, got: u8) -> Self;
}

impl UnexpectedMessageTypeErr for DecodeError {
    fn unexpected_message_type(context: &'static str, got: u8) -> Self {
        Self::new(context, DecodeErrorKind::UnexpectedMessageType { got })
    }
}

impl UnexpectedMessageTypeErr for EncodeError {
    fn unexpected_message_type(context: &'static str, got: u8) -> Self {
        Self::new(context, EncodeErrorKind::UnexpectedMessageType { got })
    }
}

pub fn unexpected_message_type_err<T: UnexpectedMessageTypeErr>(context: &'static str, got: u8) -> T {
    T::unexpected_message_type(context, got)
}

pub trait UnsupportedVersionErr {
    fn unsupported_version(context: &'static str, got: u8) -> Self;
}

impl UnsupportedVersionErr for DecodeError {
    fn unsupported_version(context: &'static str, got: u8) -> Self {
        Self::new(context, DecodeErrorKind::UnsupportedVersion { got })
    }
}

impl UnsupportedVersionErr for EncodeError {
    fn unsupported_version(context: &'static str, got: u8) -> Self {
        Self::new(context, EncodeErrorKind::UnsupportedVersion { got })
    }
}

pub fn unsupported_version_err<T: UnsupportedVersionErr>(context: &'static str, got: u8) -> T {
    T::unsupported_version(context, got)
}

pub trait UnsupportedValueErr {
    fn unsupported_value(context: &'static str, name: &'static str, value: String) -> Self;
}

impl UnsupportedValueErr for DecodeError {
    fn unsupported_value(context: &'static str, name: &'static str, value: String) -> Self {
        Self::new(context, DecodeErrorKind::UnsupportedValue { name, value })
    }
}

impl UnsupportedValueErr for EncodeError {
    fn unsupported_value(context: &'static str, name: &'static str, value: String) -> Self {
        Self::new(context, EncodeErrorKind::UnsupportedValue { name, value })
    }
}

pub fn unsupported_value_err<T: UnsupportedValueErr>(context: &'static str, name: &'static str, value: String) -> T {
    T::unsupported_value(context, name, value)
}

pub trait OtherErr {
    fn other(context: &'static str, description: &'static str) -> Self;
}

impl OtherErr for DecodeError {
    fn other(context: &'static str, description: &'static str) -> Self {
        Self::new(context, DecodeErrorKind::Other { description })
    }
}

impl OtherErr for EncodeError {
    fn other(context: &'static str, description: &'static str) -> Self {
        Self::new(context, EncodeErrorKind::Other { description })
    }
}

impl OtherErr for PduError {
    fn other(context: &'static str, description: &'static str) -> Self {
        Self::new(context, PduErrorKind::Other { description })
    }
}

pub fn other_err<T: OtherErr>(context: &'static str, description: &'static str) -> T {
    T::other(context, description)
}

pub fn other_err_with_source<T: OtherErr + WithSource, E: Source>(
    context: &'static str,
    description: &'static str,
    source: E,
) -> T {
    T::other(context, description).with_source(source)
}

pub type EncodeResult<T> = Result<T, EncodeError>;

pub type EncodeError = ironrdp_error::Error<EncodeErrorKind>;

#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum EncodeErrorKind {
    NotEnoughBytes { received: usize, expected: usize },
    InvalidField { field: &'static str, reason: &'static str },
    UnexpectedMessageType { got: u8 },
    UnsupportedVersion { got: u8 },
    UnsupportedValue { name: &'static str, value: String },
    Other { description: &'static str },
}

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
            Self::UnsupportedValue { name, value } => {
                write!(f, "unsupported {name} ({value})")
            }
            Self::Other { description } => {
                write!(f, "other ({description})")
            }
        }
    }
}

pub type PduResult<T> = Result<T, PduError>;

pub type PduError = ironrdp_error::Error<PduErrorKind>;

#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum PduErrorKind {
    Encode,
    Decode,
    Other { description: &'static str },
}

pub trait PduErrorExt {
    fn decode<E: Source>(context: &'static str, source: E) -> Self;

    fn encode<E: Source>(context: &'static str, source: E) -> Self;
}

impl PduErrorExt for PduError {
    fn decode<E: Source>(context: &'static str, source: E) -> Self {
        Self::new(context, PduErrorKind::Decode).with_source(source)
    }

    fn encode<E: Source>(context: &'static str, source: E) -> Self {
        Self::new(context, PduErrorKind::Encode).with_source(source)
    }
}

impl std::error::Error for PduErrorKind {}

impl fmt::Display for PduErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode => {
                write!(f, "encode error")
            }
            Self::Decode => {
                write!(f, "decode error")
            }
            Self::Other { description } => {
                write!(f, "other ({description})")
            }
        }
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
pub trait Encode {
    /// Encodes this PDU in-place using the provided `WriteCursor`.
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()>;

    /// Returns the associated PDU name associated.
    fn name(&self) -> &'static str;

    /// Computes the size in bytes for this PDU.
    fn size(&self) -> usize;
}

ironrdp_core::assert_obj_safe!(Encode);

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

/// PDU that can be decoded from a binary input.
///
/// The binary payload must be a full PDU, not some subset of it.
pub trait Decode<'de>: Sized {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self>;
}

pub fn decode<'de, T>(src: &'de [u8]) -> DecodeResult<T>
where
    T: Decode<'de>,
{
    let mut cursor = ReadCursor::new(src);
    T::decode(&mut cursor)
}

pub fn decode_cursor<'de, T>(src: &mut ReadCursor<'de>) -> DecodeResult<T>
where
    T: Decode<'de>,
{
    T::decode(src)
}

/// Similar to `Decode` but unconditionally returns an owned type.
pub trait DecodeOwned: Sized {
    fn decode_owned(src: &mut ReadCursor<'_>) -> DecodeResult<Self>;
}

pub fn decode_owned<T: DecodeOwned>(src: &[u8]) -> DecodeResult<T> {
    let mut cursor = ReadCursor::new(src);
    T::decode_owned(&mut cursor)
}

pub fn decode_owned_cursor<T: DecodeOwned>(src: &mut ReadCursor<'_>) -> DecodeResult<T> {
    T::decode_owned(src)
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum Action {
    FastPath = 0x00,
    X224 = 0x03,
}

impl Action {
    pub fn from_fp_output_header(fp_output_header: u8) -> Result<Self, u8> {
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
pub fn find_size(bytes: &[u8]) -> DecodeResult<Option<PduInfo>> {
    macro_rules! ensure_enough {
        ($bytes:expr, $len:expr) => {
            if $bytes.len() < $len {
                return Ok(None);
            }
        };
    }

    ensure_enough!(bytes, 1);
    let fp_output_header = bytes[0];

    let action = Action::from_fp_output_header(fp_output_header)
        .map_err(|unknown_action| unexpected_message_type_err("fpOutputHeader", unknown_action))?;

    match action {
        Action::X224 => {
            ensure_enough!(bytes, tpkt::TpktHeader::SIZE);
            let tpkt = tpkt::TpktHeader::read(&mut ReadCursor::new(bytes))?;

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

pub trait PduHint: Send + Sync + fmt::Debug + 'static {
    /// Finds next PDU size by reading the next few bytes.
    ///
    /// Returns `Some((hint_matching, size))` if the size is known.
    /// Returns `None` if the size cannot be determined yet.
    fn find_size(&self, bytes: &[u8]) -> DecodeResult<Option<(bool, usize)>>;
}

// Matches both X224 and FastPath pdus
#[derive(Clone, Copy, Debug)]
pub struct RdpHint;

pub const RDP_HINT: RdpHint = RdpHint;

impl PduHint for RdpHint {
    fn find_size(&self, bytes: &[u8]) -> DecodeResult<Option<(bool, usize)>> {
        find_size(bytes).map(|opt| opt.map(|info| (true, info.length)))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct X224Hint;

pub const X224_HINT: X224Hint = X224Hint;

impl PduHint for X224Hint {
    fn find_size(&self, bytes: &[u8]) -> DecodeResult<Option<(bool, usize)>> {
        match find_size(bytes)? {
            Some(pdu_info) => {
                let res = (pdu_info.action == Action::X224, pdu_info.length);
                Ok(Some(res))
            }
            None => Ok(None),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FastPathHint;

pub const FAST_PATH_HINT: FastPathHint = FastPathHint;

impl PduHint for FastPathHint {
    fn find_size(&self, bytes: &[u8]) -> DecodeResult<Option<(bool, usize)>> {
        match find_size(bytes)? {
            Some(pdu_info) => {
                let res = (pdu_info.action == Action::FastPath, pdu_info.length);
                Ok(Some(res))
            }
            None => Ok(None),
        }
    }
}

pub use legacy::*;

// TODO: Delete these traits at some point
mod legacy {
    use thiserror::Error;

    use crate::{Encode, EncodeResult, WriteCursor};

    pub trait PduBufferParsing<'a>: Sized {
        type Error;

        fn from_buffer(mut buffer: &'a [u8]) -> Result<Self, Self::Error> {
            Self::from_buffer_consume(&mut buffer)
        }
        fn from_buffer_consume(buffer: &mut &'a [u8]) -> Result<Self, Self::Error>;
        fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error>;
        fn buffer_length(&self) -> usize;
    }

    impl Encode for Vec<u8> {
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

    #[derive(Debug, Error)]
    pub enum RdpError {
        #[error("IO error")]
        IOError(#[from] std::io::Error),
        #[error("received invalid action code: {0}")]
        InvalidActionCode(u8),
    }
}
