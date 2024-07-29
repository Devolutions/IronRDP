//! This crate provides implementation of [NOW_PROTO] protocol.
//!
//! [NOW_PROTO]: ../../../docs/NOW-spec.md
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[macro_use]
extern crate ironrdp_pdu;

#[macro_use]
mod macros;

#[cfg(all(test, feature = "std"))]
mod test_utils;

mod core;
mod exec;
mod message;
mod session;
mod system;

pub use core::*;
pub use exec::*;
pub use message::*;
pub use session::*;
pub use system::*;
