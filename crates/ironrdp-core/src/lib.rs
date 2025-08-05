#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![cfg_attr(not(feature = "std"), no_std)]
#![warn(clippy::std_instead_of_alloc)]
#![warn(clippy::std_instead_of_core)]
#![cfg_attr(doc, warn(missing_docs))]

#[cfg(feature = "alloc")]
extern crate alloc;

#[macro_use]
mod macros;

mod as_any;
mod cursor;
mod decode;
mod encode;
mod error;
mod into_owned;
mod padding;
#[cfg(feature = "alloc")]
mod write_buf;

// Flat API hierarchy of common traits and types

pub use self::as_any::*;
pub use self::cursor::*;
pub use self::decode::*;
pub use self::encode::*;
pub use self::error::*;
pub use self::into_owned::*;
pub use self::padding::*;
#[cfg(feature = "alloc")]
pub use self::write_buf::*;
