#![cfg_attr(not(feature = "std"), no_std)]
#![allow(unused)] // FIXME(#61): remove this annotation

// TODO: this crate is WIP

#[macro_use]
extern crate tracing;

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;

// Re-export ironrdp_pdu crate for convenience
#[rustfmt::skip] // do not re-order this pub use
pub use ironrdp_pdu as pdu;
use pdu::write_buf::WriteBuf;
use pdu::{assert_obj_safe, PduEncode, PduResult};

mod complete_data;
use complete_data::CompleteData;

mod client;
pub use client::*;

mod server;
pub use server::*;

pub type DvcMessages = Vec<Box<dyn PduEncode + Send>>;

/// A type that is a Dynamic Virtual Channel (DVC)
///
/// Dynamic virtual channels may be created at any point during the RDP session.
/// The Dynamic Virtual Channel APIs exist to address limitations of Static Virtual Channels:
///   - Limited number of channels
///   - Packet reconstruction
pub trait DvcProcessor: Send + Sync {
    fn channel_name(&self) -> &str;

    fn start(&mut self, _channel_id: u32) -> PduResult<DvcMessages>;

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<DvcMessages>;

    fn close(&mut self, _channel_id: u32) {}
}

assert_obj_safe!(DvcProcessor);
