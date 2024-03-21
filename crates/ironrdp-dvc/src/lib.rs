#![cfg_attr(not(feature = "std"), no_std)]

use crate::alloc::borrow::ToOwned;
#[macro_use]
extern crate tracing;

extern crate alloc;

use alloc::string::String;
use core::any::TypeId;
use pdu::DrdynvcDataPdu;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
// Re-export ironrdp_pdu crate for convenience
#[rustfmt::skip] // do not re-order this pub use
pub use ironrdp_pdu;
use ironrdp_pdu::{assert_obj_safe, cast_length, encode_vec, other_err, PduEncode, PduResult};
use ironrdp_svc::{self, AsAny, SvcMessage};

mod complete_data;
use complete_data::CompleteData;

mod client;
pub use client::*;

mod server;
pub use server::*;

pub mod pdu;

/// Represents a message that, when encoded, forms a complete PDU for a given dynamic virtual channel.
/// This means a message that is ready to be wrapped in [`dvc::CommonPdu::DataFirst`] and [`dvc::CommonPdu::Data`] PDUs
/// (being split into multiple of such PDUs if necessary).
pub trait DvcPduEncode: PduEncode + Send {}
pub type DvcMessage = Box<dyn DvcPduEncode>;

/// We implement `DvcPduEncode` for `Vec<u8>` for legacy reasons.
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
    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>>;

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>>;

    fn close(&mut self, _channel_id: u32) {}
}

assert_obj_safe!(DvcProcessor);

pub fn encode_dvc_messages(
    channel_id: u32,
    messages: Vec<DvcMessage>,
    flags: ironrdp_svc::ChannelFlags,
) -> PduResult<Vec<SvcMessage>> {
    let mut res = Vec::new();
    for msg in messages {
        let total_length = msg.size();
        let needs_splitting = total_length >= DrdynvcDataPdu::MAX_DATA_SIZE;

        let msg = encode_vec(msg.as_ref())?;
        let mut off = 0;

        while off < total_length {
            let first = off == 0;
            let rem = total_length.checked_sub(off).unwrap();
            let size = core::cmp::min(rem, DrdynvcDataPdu::MAX_DATA_SIZE);
            let end = off
                .checked_add(size)
                .ok_or_else(|| other_err!("encode_dvc_messages", "overflow occurred"))?;

            let pdu = if needs_splitting && first {
                pdu::DrdynvcDataPdu::DataFirst(pdu::DataFirstPdu::new(
                    channel_id,
                    cast_length!("total_length", total_length)?,
                    msg[off..end].to_vec(),
                ))
            } else {
                pdu::DrdynvcDataPdu::Data(pdu::DataPdu::new(channel_id, msg[off..end].to_vec()))
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
}

impl DynamicVirtualChannel {
    fn new<T: DvcProcessor + 'static>(handler: T) -> Self {
        Self {
            channel_processor: Box::new(handler),
            complete_data: CompleteData::new(),
        }
    }

    fn start(&mut self, channel_id: DynamicChannelId) -> PduResult<Vec<DvcMessage>> {
        self.channel_processor.start(channel_id)
    }

    fn process(&mut self, pdu: DrdynvcDataPdu) -> PduResult<Vec<DvcMessage>> {
        let channel_id = pdu.channel_id();
        let complete_data = self.complete_data.process_data(pdu)?;
        if let Some(complete_data) = complete_data {
            self.channel_processor.process(channel_id, &complete_data)
        } else {
            Ok(Vec::new())
        }
    }

    fn channel_name(&self) -> &str {
        self.channel_processor.channel_name()
    }

    fn channel_processor_downcast_ref<T: DvcProcessor>(&self) -> Option<&T> {
        self.channel_processor.as_any().downcast_ref()
    }
}

struct DynamicChannelSet {
    channels: BTreeMap<DynamicChannelName, DynamicVirtualChannel>,
    name_to_channel_id: BTreeMap<DynamicChannelName, DynamicChannelId>,
    channel_id_to_name: BTreeMap<DynamicChannelId, DynamicChannelName>,
    type_id_to_name: BTreeMap<TypeId, DynamicChannelName>,
}

impl DynamicChannelSet {
    #[inline]
    fn new() -> Self {
        Self {
            channels: BTreeMap::new(),
            name_to_channel_id: BTreeMap::new(),
            channel_id_to_name: BTreeMap::new(),
            type_id_to_name: BTreeMap::new(),
        }
    }

    fn insert<T: DvcProcessor + 'static>(&mut self, channel: T) -> Option<DynamicVirtualChannel> {
        let name = channel.channel_name().to_owned();
        self.type_id_to_name.insert(TypeId::of::<T>(), name.clone());
        self.channels.insert(name, DynamicVirtualChannel::new(channel))
    }

    pub fn attach_channel_id(&mut self, name: DynamicChannelName, id: DynamicChannelId) -> Option<DynamicChannelId> {
        self.channel_id_to_name.insert(id, name.clone());
        self.name_to_channel_id.insert(name, id)
    }

    pub fn get_by_type_id(&self, type_id: TypeId) -> Option<(&DynamicVirtualChannel, Option<DynamicChannelId>)> {
        self.type_id_to_name.get(&type_id).and_then(|name| {
            self.channels
                .get(name)
                .map(|channel| (channel, self.name_to_channel_id.get(name).copied()))
        })
    }

    pub fn get_by_channel_name(&self, name: &DynamicChannelName) -> Option<&DynamicVirtualChannel> {
        self.channels.get(name)
    }

    pub fn get_by_channel_name_mut(&mut self, name: &DynamicChannelName) -> Option<&mut DynamicVirtualChannel> {
        self.channels.get_mut(name)
    }

    pub fn get_by_channel_id_mut(&mut self, id: &DynamicChannelId) -> Option<&mut DynamicVirtualChannel> {
        self.channel_id_to_name
            .get(id)
            .and_then(|name| self.channels.get_mut(name))
    }

    pub fn remove_by_channel_id(&mut self, id: &DynamicChannelId) -> Option<DynamicChannelId> {
        if let Some(name) = self.channel_id_to_name.remove(id) {
            return self.name_to_channel_id.remove(&name);
            // Channels are retained in the `self.channels` and `self.type_id_to_name` map to allow potential
            // dynamic re-addition by the server.
        }
        None
    }

    #[inline]
    pub fn values(&self) -> impl Iterator<Item = &DynamicVirtualChannel> {
        self.channels.values()
    }
}

pub type DynamicChannelName = String;
pub type DynamicChannelId = u32;
