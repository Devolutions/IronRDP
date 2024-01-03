#![cfg_attr(not(feature = "std"), no_std)]
#![allow(unused)] // FIXME(#61): remove this annotation

// TODO: this crate is WIP

#[macro_use]
extern crate tracing;

extern crate alloc;

// Re-export ironrdp_pdu crate for convenience
#[rustfmt::skip] // do not re-order this pub use
pub use ironrdp_pdu as pdu;
use pdu::write_buf::WriteBuf;
use pdu::{assert_obj_safe, PduResult};

use alloc::boxed::Box;

mod client;
pub use client::*;

/// A type that is a Dynamic Virtual Channel (DVC)
///
/// Dynamic virtual channels may be created at any point during the RDP session.
/// The Dynamic Virtual Channel APIs exist to address limitations of Static Virtual Channels:
///   - Limited number of channels
///   - Packet reconstruction
pub trait DvcProcessor: Send + Sync {
    fn channel_name(&self) -> &str;

    fn process(&mut self, channel_id: u32, payload: &[u8], output: &mut WriteBuf) -> PduResult<()>;
}

assert_obj_safe!(DvcProcessor);
