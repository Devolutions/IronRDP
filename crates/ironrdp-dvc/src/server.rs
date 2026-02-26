use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::any::TypeId;
use core::fmt;

use ironrdp_core::{cast_length, impl_as_any, invalid_field_err, Decode as _, DecodeResult, ReadCursor};
use ironrdp_pdu::{self as pdu, decode_err, encode_err, pdu_other_err};
use ironrdp_svc::{ChannelFlags, CompressionCondition, SvcMessage, SvcProcessor, SvcServerProcessor};
use pdu::gcc::ChannelName;
use pdu::PduResult;
use slab::Slab;
use tracing::debug;

use crate::pdu::{
    CapabilitiesRequestPdu, CapsVersion, CreateRequestPdu, CreationStatus, DrdynvcClientPdu, DrdynvcServerPdu,
};
use crate::{encode_dvc_messages, CompleteData, DvcProcessor};

pub trait DvcServerProcessor: DvcProcessor {}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ChannelState {
    Closed,
    Creation,
    Opened,
    CreationFailed(u32),
}

struct DynamicChannel {
    state: ChannelState,
    processor: Box<dyn DvcProcessor>,
    complete_data: CompleteData,
}

impl DynamicChannel {
    fn new<T>(processor: T) -> Self
    where
        T: DvcServerProcessor + 'static,
    {
        Self {
            state: ChannelState::Closed,
            processor: Box::new(processor),
            complete_data: CompleteData::new(),
        }
    }
}
/// DRDYNVC Static Virtual Channel (the Remote Desktop Protocol: Dynamic Virtual Channel Extension)
///
/// It adds support for dynamic virtual channels (DVC).
pub struct DrdynvcServer {
    dynamic_channels: Slab<DynamicChannel>,
    type_id_to_channel_id: BTreeMap<TypeId, u32>,
}

impl fmt::Debug for DrdynvcServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DrdynvcServer([")?;

        for (i, (id, channel)) in self.dynamic_channels.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}:{} ({:?})", id, channel.processor.channel_name(), channel.state)?;
        }

        write!(f, "])")
    }
}

impl DrdynvcServer {
    pub const NAME: ChannelName = ChannelName::from_static(b"drdynvc\0");

    pub fn new() -> Self {
        Self {
            dynamic_channels: Slab::new(),
            type_id_to_channel_id: BTreeMap::new(),
        }
    }

    pub fn get_channel_id_by_type<T>(&self) -> Option<u32>
    where
        T: DvcServerProcessor + 'static,
    {
        self.type_id_to_channel_id.get(&TypeId::of::<T>()).copied()
    }

    /// Returns `true` if the DVC channel with the given ID has completed
    /// its creation handshake and is in the `Opened` state.
    pub fn is_channel_opened(&self, channel_id: u32) -> bool {
        let Ok(id) = usize::try_from(channel_id) else {
            return false;
        };
        self.dynamic_channels
            .get(id)
            .is_some_and(|c| c.state == ChannelState::Opened)
    }

    // FIXME(#61): it's likely we want to enable adding dynamic channels at any point during the session (message passing? other approach?)

    /// Registers a dynamic channel with the server.
    ///
    /// # Panics
    ///
    /// Panics if the number of registered dynamic channels exceeds `u32::MAX`.
    #[must_use]
    pub fn with_dynamic_channel<T>(mut self, channel: T) -> Self
    where
        T: DvcServerProcessor + 'static,
    {
        let id = self.dynamic_channels.insert(DynamicChannel::new(channel));
        // The slab index is used as the DVC channel ID (a u32).
        let channel_id = u32::try_from(id).expect("DVC channel count should not exceed u32::MAX");
        self.type_id_to_channel_id.insert(TypeId::of::<T>(), channel_id);
        self
    }

    fn channel_by_id(&mut self, id: u32) -> DecodeResult<&mut DynamicChannel> {
        let id = cast_length!("DRDYNVC", "", id)?;
        self.dynamic_channels
            .get_mut(id)
            .ok_or_else(|| invalid_field_err!("DRDYNVC", "", "invalid channel id"))
    }
}

impl_as_any!(DrdynvcServer);

impl Default for DrdynvcServer {
    fn default() -> Self {
        Self::new()
    }
}

impl SvcProcessor for DrdynvcServer {
    fn channel_name(&self) -> ChannelName {
        DrdynvcServer::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::WhenRdpDataIsCompressed
    }

    fn start(&mut self) -> PduResult<Vec<SvcMessage>> {
        let cap = CapabilitiesRequestPdu::new(CapsVersion::V2, None);
        let req = DrdynvcServerPdu::Capabilities(cap);
        let msg = as_svc_msg_with_flag(req)?;
        Ok(alloc::vec![msg])
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let pdu = decode_dvc_message(payload).map_err(|e| decode_err!(e))?;
        let mut resp = Vec::new();

        match pdu {
            DrdynvcClientPdu::Capabilities(caps_resp) => {
                debug!("Got DVC Capabilities Response PDU: {caps_resp:?}");
                for (id, c) in self.dynamic_channels.iter_mut() {
                    if c.state != ChannelState::Closed {
                        continue;
                    }
                    let req = DrdynvcServerPdu::Create(CreateRequestPdu::new(
                        id.try_into()
                            .map_err(|e| pdu_other_err!("invalid channel id", source: e))?,
                        c.processor.channel_name().into(),
                    ));
                    c.state = ChannelState::Creation;
                    resp.push(as_svc_msg_with_flag(req)?);
                }
            }
            DrdynvcClientPdu::Create(create_resp) => {
                debug!("Got DVC Create Response PDU: {create_resp:?}");
                let id = create_resp.channel_id();
                let c = self.channel_by_id(id).map_err(|e| decode_err!(e))?;
                if c.state != ChannelState::Creation {
                    return Err(pdu_other_err!("invalid channel state"));
                }
                if create_resp.creation_status() != CreationStatus::OK {
                    c.state = ChannelState::CreationFailed(create_resp.creation_status().into());
                    return Ok(resp);
                }
                c.state = ChannelState::Opened;
                let msg = c.processor.start(create_resp.channel_id())?;
                resp.extend(encode_dvc_messages(id, msg, ChannelFlags::SHOW_PROTOCOL).map_err(|e| encode_err!(e))?);
            }
            DrdynvcClientPdu::Close(close_resp) => {
                debug!("Got DVC Close Response PDU: {close_resp:?}");
                let c = self
                    .channel_by_id(close_resp.channel_id())
                    .map_err(|e| decode_err!(e))?;
                if c.state != ChannelState::Opened {
                    return Err(pdu_other_err!("invalid channel state"));
                }
                c.state = ChannelState::Closed;
            }
            DrdynvcClientPdu::Data(data) => {
                let channel_id = data.channel_id();
                let c = self.channel_by_id(channel_id).map_err(|e| decode_err!(e))?;
                if c.state != ChannelState::Opened {
                    debug!(?channel_id, ?c.state, "Invalid channel state");
                    return Err(pdu_other_err!("invalid channel state"));
                }
                if let Some(complete) = c.complete_data.process_data(data).map_err(|e| decode_err!(e))? {
                    let msg = c.processor.process(channel_id, &complete)?;
                    resp.extend(
                        encode_dvc_messages(channel_id, msg, ChannelFlags::SHOW_PROTOCOL)
                            .map_err(|e| encode_err!(e))?,
                    );
                }
            }
        }

        Ok(resp)
    }
}

impl SvcServerProcessor for DrdynvcServer {}

fn decode_dvc_message(user_data: &[u8]) -> DecodeResult<DrdynvcClientPdu> {
    DrdynvcClientPdu::decode(&mut ReadCursor::new(user_data))
}

fn as_svc_msg_with_flag(pdu: DrdynvcServerPdu) -> PduResult<SvcMessage> {
    Ok(SvcMessage::from(pdu).with_flags(ChannelFlags::SHOW_PROTOCOL))
}
