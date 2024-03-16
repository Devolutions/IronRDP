#![cfg_attr(not(feature = "std"), no_std)]
#![allow(unused)] // FIXME(#61): remove this annotation

// TODO: this crate is WIP

#[macro_use]
extern crate tracing;

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

// Re-export ironrdp_pdu crate for convenience
#[rustfmt::skip] // do not re-order this pub use
pub use ironrdp_pdu as pdu;
use ironrdp_svc::{self, impl_as_any, AsAny, SvcMessage};
use pdu::dvc::gfx::ServerPdu;
use pdu::dvc::{self, DataFirstPdu, DataPdu};
use pdu::write_buf::WriteBuf;
use pdu::{assert_obj_safe, cast_length, custom_err, encode_vec, other_err, PduEncode, PduParsing as _, PduResult};

mod complete_data;
use complete_data::CompleteData;

mod client;
pub use client::*;

mod server;
pub use server::*;

pub mod display;

/// Represents a message that, when encoded, forms a complete PDU for a given dynamic virtual channel.
/// This means a message that is ready to be wrapped in [`dvc::CommonPdu::DataFirst`] and [`dvc::CommonPdu::Data`] PDUs
/// (being split into multiple of such PDUs if necessary).
pub trait DvcPduEncode: PduEncode {}
pub type DvcMessages = Vec<Box<dyn DvcPduEncode + Send>>;

/// For legacy reasons, we implement [`DvcPduEncode`] for [`Vec<u8>`].
impl DvcPduEncode for Vec<u8> {}

/// A type that is a Dynamic Virtual Channel (DVC)
///
/// Dynamic virtual channels may be created at any point during the RDP session.
/// The Dynamic Virtual Channel APIs exist to address limitations of Static Virtual Channels:
///   - Limited number of channels
///   - Packet reconstruction
pub trait DvcProcessor: AsAny + Send + Sync {
    /// The name of the channel, e.g. "Microsoft::Windows::RDS::DisplayControl"
    fn channel_name(&self) -> &str;

    /// Returns any messages that should be sent immediately
    /// upon the channel being created.
    fn start(&mut self, _channel_id: u32) -> PduResult<DvcMessages>;

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<DvcMessages>;

    fn close(&mut self, _channel_id: u32) {}
}

assert_obj_safe!(DvcProcessor);

const DATA_MAX_SIZE: usize = 1590;

pub(crate) fn encode_dvc_messages(
    channel_id: u32,
    messages: DvcMessages,
    flags: Option<ironrdp_svc::ChannelFlags>,
) -> PduResult<Vec<SvcMessage>> {
    let mut res = Vec::new();
    for msg in messages {
        let total_size = msg.size();

        let msg = encode_vec(msg.as_ref())?;
        let mut off = 0;

        while off < total_size {
            let rem = total_size.checked_sub(off).unwrap();
            let size = core::cmp::min(rem, DATA_MAX_SIZE);

            let pdu = if off == 0 && total_size >= DATA_MAX_SIZE {
                let total_size = cast_length!("encode_dvc_messages", "totalDataSize", total_size)?;
                dvc::CommonPdu::DataFirst(dvc::DataFirstPdu::new(channel_id, total_size, DATA_MAX_SIZE))
            } else {
                dvc::CommonPdu::Data(dvc::DataPdu::new(channel_id, size))
            };

            let end = off
                .checked_add(size)
                .ok_or_else(|| other_err!("encode_dvc_messages", "overflow occurred"))?;
            let mut data = Vec::new();
            pdu.to_buffer(&mut data)
                .map_err(|e| custom_err!("encode_dvc_messages", e))?;
            data.extend_from_slice(&msg[off..end]);
            let mut svc = SvcMessage::from(data);
            if let Some(flags) = flags {
                svc = svc.with_flags(flags);
            }
            res.push(svc);
            off = end;
        }
    }

    Ok(res)
}
