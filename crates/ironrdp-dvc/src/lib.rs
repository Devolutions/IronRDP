#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use core::any::TypeId;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use pdu::DrdynvcDataPdu;

// Re-export ironrdp_pdu crate for convenience
#[rustfmt::skip] // do not re-order this pub use
pub use ironrdp_pdu;
use ironrdp_core::{AsAny, Encode, EncodeResult, assert_obj_safe, cast_length, encode_vec, other_err};
use ironrdp_pdu::{PduResult, decode_err, pdu_other_err};
use ironrdp_svc::SvcMessage;

mod complete_data;
use complete_data::CompleteData;

mod client;
pub use client::*;

mod server;
pub use server::*;

pub mod pdu;

/// Represents a message that, when encoded, forms a complete PDU for a given dynamic virtual channel.
/// This means a message that is ready to be wrapped in [`pdu::DataFirstPdu`] and [`pdu::DataPdu`] PDUs
/// (being split into multiple of such PDUs if necessary).
pub trait DvcEncode: Encode + Send {}
pub type DvcMessage = Box<dyn DvcEncode>;

/// A type that is a Dynamic Virtual Channel (DVC)
///
/// Dynamic virtual channels may be created at any point during the RDP session.
/// The Dynamic Virtual Channel APIs exist to address limitations of Static Virtual Channels:
///   - Limited number of channels
///   - Packet reconstruction
pub trait DvcProcessor: AsAny + Send {
    /// The name of the channel, e.g. "Microsoft::Windows::RDS::DisplayControl"
    fn channel_name(&self) -> &str;

    /// Returns any messages that should be sent immediately
    /// upon the channel being created.
    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>>;

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>>;

    fn close(&mut self, _channel_id: u32) {}
}

assert_obj_safe!(DvcProcessor);

pub fn encode_dvc_messages(
    channel_id: u32,
    messages: Vec<DvcMessage>,
    flags: ironrdp_svc::ChannelFlags,
) -> EncodeResult<Vec<SvcMessage>> {
    let mut res = Vec::new();
    for msg in messages {
        let total_length = msg.size();
        let needs_splitting = total_length >= DrdynvcDataPdu::MAX_DATA_SIZE;

        let msg = encode_vec(msg.as_ref())?;
        let mut off = 0;

        while off < total_length {
            let first = off == 0;

            #[expect(clippy::missing_panics_doc, reason = "unreachable panic (checked underflow)")]
            let remaining_length = total_length.checked_sub(off).expect("never overflow");
            let size = core::cmp::min(remaining_length, DrdynvcDataPdu::MAX_DATA_SIZE);
            let end = off
                .checked_add(size)
                .ok_or_else(|| other_err!("encode_dvc_messages", "overflow occurred"))?;

            let pdu = if needs_splitting && first {
                DrdynvcDataPdu::DataFirst(pdu::DataFirstPdu::new(
                    channel_id,
                    cast_length!("total_length", total_length)?,
                    msg[off..end].to_vec(),
                ))
            } else {
                DrdynvcDataPdu::Data(pdu::DataPdu::new(channel_id, msg[off..end].to_vec()))
            };

            let svc = SvcMessage::from(pdu).with_flags(flags);

            res.push(svc);
            off = end;
        }
    }

    Ok(res)
}

pub struct DynamicVirtualChannel {
    channel_processor: Box<dyn DvcProcessor + Send>,
    complete_data: CompleteData,
    /// The channel ID assigned by the server.
    ///
    /// This field is `None` until the server assigns a channel ID.
    channel_id: Option<DynamicChannelId>,
}

impl DynamicVirtualChannel {
    fn from_boxed(processor: Box<dyn DvcProcessor + Send>) -> Self {
        Self {
            channel_processor: processor,
            complete_data: CompleteData::new(),
            channel_id: None,
        }
    }

    fn processor_type_id(&self) -> TypeId {
        self.channel_processor.as_any().type_id()
    }

    pub fn is_open(&self) -> bool {
        self.channel_id.is_some()
    }

    pub fn channel_id(&self) -> Option<DynamicChannelId> {
        self.channel_id
    }

    pub fn channel_processor_downcast_ref<T: DvcProcessor>(&self) -> Option<&T> {
        self.channel_processor.as_any().downcast_ref()
    }

    fn start(&mut self) -> PduResult<Vec<DvcMessage>> {
        if let Some(channel_id) = self.channel_id {
            self.channel_processor.start(channel_id)
        } else {
            Err(pdu_other_err!("DynamicVirtualChannel::start", "channel ID not set"))
        }
    }

    fn process(&mut self, pdu: DrdynvcDataPdu) -> PduResult<Vec<DvcMessage>> {
        let channel_id = pdu.channel_id();
        let complete_data = self.complete_data.process_data(pdu).map_err(|e| decode_err!(e))?;
        if let Some(complete_data) = complete_data {
            self.channel_processor.process(channel_id, &complete_data)
        } else {
            Ok(Vec::new())
        }
    }

    fn channel_name(&self) -> &str {
        self.channel_processor.channel_name()
    }
}

pub type DynamicChannelName = String;
pub type DynamicChannelId = u32;
