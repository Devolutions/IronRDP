#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use pdu::DrdynvcDataPdu;

// Re-export ironrdp_pdu crate for convenience
#[rustfmt::skip] // do not re-order this pub use
pub use ironrdp_pdu;
use ironrdp_core::{
    AsAny, Decode as _, Encode, EncodeResult, ReadCursor, assert_obj_safe, cast_length, encode_vec, other_err,
};
use ironrdp_pdu::{PduResult, decode_err, encode_err, pdu_other_err};
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

/// A batch of outgoing DRDYNVC data messages, all addressed to the same dynamic channel.
///
/// Carrying the channel ID alongside the messages lets a caller route the batch onto the
/// multitransport tunnel (or the main connection) selected for that channel by Soft-Sync,
/// without re-parsing the encoded messages.
#[derive(Debug)]
pub struct DvcMessageBatch {
    channel_id: DynamicChannelId,
    messages: Vec<SvcMessage>,
}

impl DvcMessageBatch {
    /// Creates a batch, trusting the caller that every message already carries `channel_id`.
    ///
    /// Crate-private: callers outside this crate cannot vouch for that invariant and must go
    /// through [`Self::try_new`] instead, which verifies it.
    pub(crate) fn new(channel_id: DynamicChannelId, messages: Vec<SvcMessage>) -> Self {
        Self { channel_id, messages }
    }

    /// Creates a batch after verifying that every message is DVC data addressed to `channel_id`.
    ///
    /// Use this for messages obtained from outside this crate, where the channel ID has not
    /// already been established by construction (e.g. [`Self::new`]).
    pub fn try_new(channel_id: DynamicChannelId, messages: Vec<SvcMessage>) -> PduResult<Self> {
        for message in &messages {
            let encoded = message.encode_unframed_pdu().map_err(|error| encode_err!(error))?;
            let pdu =
                pdu::DrdynvcClientPdu::decode(&mut ReadCursor::new(&encoded)).map_err(|error| decode_err!(error))?;
            match pdu {
                pdu::DrdynvcClientPdu::Data(data) if data.channel_id() == channel_id => {}
                pdu::DrdynvcClientPdu::Data(_) => {
                    return Err(pdu_other_err!("DVC message channel ID does not match its batch"));
                }
                _ => return Err(pdu_other_err!("DVC message batch contains a non-data PDU")),
            }
        }
        Ok(Self::new(channel_id, messages))
    }

    pub fn channel_id(&self) -> DynamicChannelId {
        self.channel_id
    }

    pub fn messages(&self) -> &[SvcMessage] {
        &self.messages
    }

    pub fn into_messages(self) -> Vec<SvcMessage> {
        self.messages
    }
}

/// A type that is a Dynamic Virtual Channel (DVC)
///
/// Dynamic virtual channels may be created at any point during the RDP session.
/// The Dynamic Virtual Channel APIs exist to address limitations of Static Virtual Channels:
///   - Limited number of channels
///   - Packet reconstruction
pub trait DvcProcessor: AsAny + Send {
    /// The name of the channel, e.g. "Microsoft::Windows::RDS::DisplayControl"
    fn channel_name(&self) -> &str;

    /// Returns the maximum complete, reassembled message size accepted by this channel.
    ///
    /// This limit is checked before DVC fragments are retained for reassembly.
    /// `None` preserves the historical behavior without a message-size limit.
    fn max_message_size(&self) -> Option<usize> {
        None
    }

    /// Returns any messages that should be sent immediately
    /// upon the channel being created.
    ///
    /// A pre-registered channel the server closes is started again when the server
    /// opens it anew, so this may follow [`DvcProcessor::close`] on the same processor.
    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>>;

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>>;

    fn close(&mut self, _channel_id: u32) {}
}

assert_obj_safe!(DvcProcessor);

/// Encodes complete messages as DVC data PDUs, preserving message boundaries.
///
/// Empty messages produce a header-only `DATA` PDU ([2.2.3.2] DVC Data PDU (`DYNVC_DATA`)).
/// Messages of at most 1,590 bytes use `DATA`; larger messages start with `DATA_FIRST` ([2.2.3.1] DVC Data First PDU (`DYNVC_DATA_FIRST`)).
///
/// [2.2.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/69377767-56a6-4ab8-996b-7758676e9261
/// [2.2.3.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/15b59886-db44-47f1-8da3-47c8fcd82803
pub fn encode_dvc_messages(
    channel_id: u32,
    messages: Vec<DvcMessage>,
    flags: ironrdp_svc::ChannelFlags,
) -> EncodeResult<Vec<SvcMessage>> {
    let mut res = Vec::new();
    for msg in messages {
        let total_length = msg.size();
        let needs_splitting = DrdynvcDataPdu::MAX_DATA_SIZE < total_length;

        let msg = encode_vec(msg.as_ref())?;
        let mut off = 0;

        loop {
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
            if off == total_length {
                break;
            }
        }
    }

    Ok(res)
}

/// Borrowed typed view of a dynamic virtual channel.
#[derive(Debug, Clone, Copy)]
pub struct DynamicChannelRef<'a, T> {
    channel_id: DynamicChannelId,
    processor: &'a T,
}

impl<T> DynamicChannelRef<'_, T> {
    pub fn channel_id(&self) -> DynamicChannelId {
        self.channel_id
    }
}

impl<'a, T: DvcProcessor> DynamicChannelRef<'a, T> {
    fn new(channel_id: u32, processor: &'a T) -> Self {
        Self { channel_id, processor }
    }

    pub fn processor(&self) -> &'a T {
        self.processor
    }
}

/// Mutable borrowed typed view of a dynamic virtual channel.
#[derive(Debug)]
pub struct DynamicChannelMut<'a, T> {
    channel_id: DynamicChannelId,
    processor: &'a mut T,
}

impl<T> DynamicChannelMut<'_, T> {
    pub fn channel_id(&self) -> DynamicChannelId {
        self.channel_id
    }
}

impl<'a, T: DvcProcessor> DynamicChannelMut<'a, T> {
    fn new(channel_id: u32, processor: &'a mut T) -> Self {
        Self { channel_id, processor }
    }

    pub fn processor(&self) -> &T {
        self.processor
    }

    pub fn processor_mut(&mut self) -> &mut T {
        self.processor
    }
}

pub type DynamicChannelName = String;
pub type DynamicChannelId = u32;
