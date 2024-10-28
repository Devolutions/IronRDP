#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[macro_use]
extern crate ironrdp_pdu;

// Ensure that we do not compile on platforms with less than 4 bytes per u32. It is pretty safe
// to assume that NOW-PROTO will not ever be used on 8/16-bit MCUs or CPUs.
//
// This is required to safely cast u32 to usize without additional checks.
const_assert!(size_of::<usize>() >= 4);

#[macro_use]
mod macros;

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
