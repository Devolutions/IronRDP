#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![cfg_attr(not(feature = "std"), no_std)]
#![warn(clippy::std_instead_of_alloc)]
#![warn(clippy::std_instead_of_core)]
#![cfg_attr(doc, warn(missing_docs))]

#[cfg(feature = "alloc")]
extern crate alloc;

mod macros;

mod as_any;
mod cursor;
mod decode;
mod encode;
mod error;
mod into_owned;
#[cfg(feature = "alloc")]
mod non_empty;
mod padding;
#[cfg(feature = "alloc")]
mod write_buf;

// Flat API hierarchy of common traits and types.
//
// Each `pub use` lists its exports explicitly so that adding a new `pub`
// item to a module is a conscious public-API commitment rather than
// auto-public via a wildcard.

pub use self::as_any::AsAny;
pub use self::cursor::{NotEnoughBytesError, ReadCursor, WriteCursor};
pub use self::decode::{
    Decode, DecodeError, DecodeErrorKind, DecodeOwned, DecodeResult, decode, decode_cursor, decode_owned,
    decode_owned_cursor,
};
#[cfg(feature = "alloc")]
pub use self::encode::encode_buf;
#[cfg(any(feature = "alloc", test))]
pub use self::encode::encode_vec;
pub use self::encode::{Encode, EncodeError, EncodeErrorKind, EncodeResult, encode, encode_cursor, name, size};
pub use self::error::{
    InvalidFieldErr, NotEnoughBytesErr, OtherErr, UnexpectedMessageTypeErr, UnsupportedValueErr, UnsupportedVersionErr,
    WithSource, invalid_field_err, invalid_field_err_with_source, not_enough_bytes_err, other_err,
    other_err_with_source, unexpected_message_type_err, unsupported_value_err, unsupported_version_err,
};
pub use self::into_owned::IntoOwned;
#[cfg(feature = "alloc")]
pub use self::non_empty::NonEmpty;
pub use self::padding::{read_padding, write_padding};
#[cfg(feature = "alloc")]
pub use self::write_buf::WriteBuf;
