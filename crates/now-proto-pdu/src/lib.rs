//! This crate provides implementation of [NOW_PROTO] protocol.
//!
//! [NOW_PROTO]: ../../../docs/NOW-spec.md
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[macro_use]
mod macros;

#[cfg(all(test, feature = "std"))]
mod test_utils;

mod core;
mod exec;
mod message;
mod session;
mod system;

mod error;

pub use error::{PduError, PduErrorExt, PduErrorKind};
pub use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};

pub type PduResult<T> = Result<T, PduError>;

pub use core::*;
pub use exec::*;
pub use message::*;
pub use session::*;
pub use system::*;

//use cursor::{ReadCursor, WriteCursor};

/// PDU that can be encoded into its binary form.
///
/// The resulting binary payload is a fully encoded PDU that may be sent to the peer.
///
/// This trait is object-safe and may be used in a dynamic context.
pub trait PduEncode {
    /// Encodes this PDU in-place using the provided `WriteCursor`.
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()>;

    /// Returns the associated PDU name associated.
    fn name(&self) -> &'static str;

    /// Computes the size in bytes for this PDU.
    fn size(&self) -> usize;
}

assert_obj_safe!(PduEncode);

/// Encodes the given PDU in-place into the provided buffer and returns the number of bytes written.
pub fn encode<T: PduEncode>(pdu: &T, dst: &mut [u8]) -> PduResult<usize> {
    let mut cursor = WriteCursor::new(dst);
    encode_cursor(pdu, &mut cursor)?;
    Ok(cursor.pos())
}

/// Same as `encode_pdu` but resizes the buffer when it is too small to fit the PDU.
pub fn encode_buf<T: PduEncode>(pdu: &T, buf: &mut alloc::vec::Vec<u8>) -> PduResult<usize> {
    let pdu_size = pdu.size();

    if buf.len() < pdu_size {
        buf.resize(pdu_size, 0);
    }

    encode(pdu, buf)
}

/// Encodes the given PDU in-place using the provided `WriteCursor`.
pub fn encode_cursor<T: PduEncode>(pdu: &T, dst: &mut WriteCursor<'_>) -> PduResult<()> {
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
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self>;
}

pub fn decode<'de, T: PduDecode<'de>>(src: &'de [u8]) -> PduResult<T> {
    let mut cursor = ReadCursor::new(src);
    T::decode(&mut cursor)
}

pub fn decode_cursor<'de, T: PduDecode<'de>>(src: &mut ReadCursor<'de>) -> PduResult<T> {
    T::decode(src)
}

/// Similar to `PduDecode` but unconditionally returns an owned type.
pub trait PduDecodeOwned: Sized {
    fn decode_owned(src: &mut ReadCursor<'_>) -> PduResult<Self>;
}

pub fn decode_owned<T: PduDecodeOwned>(src: &[u8]) -> PduResult<T> {
    let mut cursor = ReadCursor::new(src);
    T::decode_owned(&mut cursor)
}

pub fn decode_owned_cursor<T: PduDecodeOwned>(src: &mut ReadCursor<'_>) -> PduResult<T> {
    T::decode_owned(src)
}

/// Trait used to produce an owned version of a given PDU.
pub trait IntoOwnedPdu: Sized {
    type Owned: 'static;

    fn into_owned_pdu(self) -> Self::Owned;
}
