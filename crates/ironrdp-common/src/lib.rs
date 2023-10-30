#![cfg_attr(not(feature = "std"), no_std)]
#![doc = include_str!("../README.md")]
#![warn(clippy::std_instead_of_alloc)]
#![warn(clippy::std_instead_of_core)]
// #![warn(missing_docs)] // TODO: this crate should be well documented

#[cfg(feature = "alloc")]
extern crate alloc;

#[macro_use]
mod macros;

mod as_any;
mod bytes;
mod cursor;
mod decode;
mod encode;
mod fixed_string;
mod hint;
mod into_owned;
mod marker;
mod named;
mod string;
#[cfg(feature = "alloc")]
mod write_buf;

pub mod legacy;
pub mod prelude;

// Flat API hierarchy of common traits and types

pub use self::as_any::*;
pub use self::bytes::*;
pub use self::cursor::*;
pub use self::decode::*;
pub use self::encode::*;
pub use self::fixed_string::*;
pub use self::hint::*;
pub use self::into_owned::*;
pub use self::marker::*;
pub use self::named::*;
pub use self::string::*;
#[cfg(feature = "alloc")]
pub use self::write_buf::*;

#[cfg(feature = "alloc")]
pub use ironrdp_error::StringError;
pub use ironrdp_error::{err_desc, StrError};

pub(crate) mod private {
    #[doc(hidden)]
    pub trait Sealed {}
}
