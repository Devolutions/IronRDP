use crate::pdu::{
    CapabilitiesRequestPdu, CapsVersion, CreateRequestPdu, CreationStatus, DrdynvcClientPdu, DrdynvcServerPdu,
};
use crate::{encode_dvc_messages, CompleteData, DvcProcessor};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt;
use ironrdp_core::impl_as_any;
use ironrdp_core::ReadCursor;
use ironrdp_pdu::{self as pdu, decode_err, encode_err, other_err, DecodeResult};
use ironrdp_svc::{ChannelFlags, CompressionCondition, SvcMessage, SvcProcessor, SvcServerProcessor};
use pdu::gcc::ChannelName;
use pdu::Decode as _;
use pdu::PduResult;
use pdu::{cast_length, invalid_field_err};
use slab::Slab;

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
        }
    }

    // FIXME(#61): it’s likely we want to enable adding dynamic channels at any point during the session (message passing? other approach?)

    #[must_use]
    pub fn with_dynamic_channel<T>(mut self, channel: T) -> Self
    where
        T: DvcServerProcessor + 'static,
    {
        self.dynamic_channels.insert(DynamicChannel::new(channel));
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
        let cap = CapabilitiesRequestPdu::new(CapsVersion::V1, None);
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
                        id.try_into().map_err(|e| other_err!("invalid channel id", source: e))?,
                        c.processor.channel_name().into(),
                    ));
                    c.state = ChannelState::Creation;
                    resp.push(as_svc_msg_with_flag(req)?);
                }
            }
            DrdynvcClientPdu::Create(create_resp) => {
                debug!("Got DVC Create Response PDU: {create_resp:?}");
                let id = create_resp.channel_id;
                let c = self.channel_by_id(id).map_err(|e| decode_err!(e))?;
                if c.state != ChannelState::Creation {
                    return Err(other_err!("invalid channel state"));
                }
                if create_resp.creation_status != CreationStatus::OK {
                    c.state = ChannelState::CreationFailed(create_resp.creation_status.into());
                    return Ok(resp);
                }
                c.state = ChannelState::Opened;
                let msg = c.processor.start(create_resp.channel_id)?;
                resp.extend(encode_dvc_messages(id, msg, ChannelFlags::SHOW_PROTOCOL).map_err(|e| encode_err!(e))?);
            }
            DrdynvcClientPdu::Close(close_resp) => {
                debug!("Got DVC Close Response PDU: {close_resp:?}");
                let c = self.channel_by_id(close_resp.channel_id).map_err(|e| decode_err!(e))?;
                if c.state != ChannelState::Opened {
                    return Err(other_err!("invalid channel state"));
                }
                c.state = ChannelState::Closed;
            }
            DrdynvcClientPdu::Data(data) => {
                let channel_id = data.channel_id();
                let c = self.channel_by_id(channel_id).map_err(|e| decode_err!(e))?;
                if c.state != ChannelState::Opened {
                    return Err(other_err!("invalid channel state"));
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
