use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;
use core::fmt;
use pdu::dvc::{CreateRequestPdu, DataFirstPdu, DataPdu};
use slab::Slab;

use ironrdp_pdu as pdu;

use ironrdp_svc::{impl_as_any, ChannelFlags, CompressionCondition, SvcMessage, SvcProcessor, SvcServerProcessor};
use pdu::cursor::WriteCursor;
use pdu::gcc::ChannelName;
use pdu::rdp::vc;
use pdu::write_buf::WriteBuf;
use pdu::{cast_length, custom_err, encode_vec, invalid_message_err, other_err, PduEncode, PduParsing};
use pdu::{dvc, PduResult};

use crate::{CompleteData, DvcMessages, DvcProcessor};

const DATA_MAX_SIZE: usize = 1590;

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

    fn channel_by_id(&mut self, id: u32) -> PduResult<&mut DynamicChannel> {
        let id = cast_length!("DRDYNVC", "", id)?;
        self.dynamic_channels
            .get_mut(id)
            .ok_or_else(|| invalid_message_err!("DRDYNVC", "", "invalid channel id"))
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
        let cap = dvc::CapabilitiesRequestPdu::V1;
        let req = dvc::ServerPdu::CapabilitiesRequest(cap);
        let msg = encode_dvc_message(req)?;
        Ok(alloc::vec![msg])
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let dvc_ctx = decode_dvc_message(payload)?;
        let mut resp = Vec::new();

        match dvc_ctx.dvc_pdu {
            dvc::ClientPdu::CapabilitiesResponse(caps_resp) => {
                debug!("Got DVC Capabilities Response PDU: {caps_resp:?}");
                for (id, c) in self.dynamic_channels.iter_mut() {
                    if c.state != ChannelState::Closed {
                        continue;
                    }
                    let req = dvc::ServerPdu::CreateRequest(CreateRequestPdu::new(
                        id.try_into().map_err(|e| custom_err!("invalid channel id", e))?,
                        c.processor.channel_name().into(),
                    ));
                    c.state = ChannelState::Creation;
                    resp.push(encode_dvc_message(req)?);
                }
            }
            dvc::ClientPdu::CreateResponse(create_resp) => {
                debug!("Got DVC Create Response PDU: {create_resp:?}");
                let id = create_resp.channel_id;
                let c = self.channel_by_id(id)?;
                if c.state != ChannelState::Creation {
                    return Err(invalid_message_err!("DRDYNVC", "", "invalid channel state"));
                }
                if create_resp.creation_status != dvc::DVC_CREATION_STATUS_OK {
                    c.state = ChannelState::CreationFailed(create_resp.creation_status);
                    return Ok(resp);
                }
                c.state = ChannelState::Opened;
                let msg = c.processor.start(create_resp.channel_id)?;
                resp.extend(encode_dvc_data(id, msg)?);
            }
            dvc::ClientPdu::CloseResponse(close_resp) => {
                debug!("Got DVC Close Response PDU: {close_resp:?}");
                let c = self.channel_by_id(close_resp.channel_id)?;
                if c.state != ChannelState::Opened {
                    return Err(invalid_message_err!("DRDYNVC", "", "invalid channel state"));
                }
                c.state = ChannelState::Closed;
            }
            dvc::ClientPdu::Common(dvc::CommonPdu::DataFirst(data)) => {
                let channel_id = data.channel_id;
                let c = self.channel_by_id(channel_id)?;
                if c.state != ChannelState::Opened {
                    return Err(invalid_message_err!("DRDYNVC", "", "invalid channel state"));
                }
                if let Some(complete) = c.complete_data.process_data(data.into(), dvc_ctx.dvc_data.into()) {
                    let msg = c.processor.process(channel_id, &complete)?;
                    resp.extend(encode_dvc_data(channel_id, msg)?);
                }
            }
            dvc::ClientPdu::Common(dvc::CommonPdu::Data(data)) => {
                let channel_id = data.channel_id;
                let c = self.channel_by_id(channel_id)?;
                if c.state != ChannelState::Opened {
                    return Err(invalid_message_err!("DRDYNVC", "", "invalid channel state"));
                }
                if let Some(complete) = c.complete_data.process_data(data.into(), dvc_ctx.dvc_data.into()) {
                    let msg = c.processor.process(channel_id, &complete)?;
                    resp.extend(encode_dvc_data(channel_id, msg)?);
                }
            }
        }

        Ok(resp)
    }
}

impl SvcServerProcessor for DrdynvcServer {}

struct DynamicChannelCtx<'a> {
    dvc_pdu: vc::dvc::ClientPdu,
    dvc_data: &'a [u8],
}

fn decode_dvc_message(user_data: &[u8]) -> PduResult<DynamicChannelCtx<'_>> {
    let mut user_data = user_data;
    let user_data_len = user_data.len();

    // … | dvc::ClientPdu | …
    let dvc_pdu =
        vc::dvc::ClientPdu::from_buffer(&mut user_data, user_data_len).map_err(|e| custom_err!("DVC client PDU", e))?;

    // … | DvcData ]
    let dvc_data = user_data;

    Ok(DynamicChannelCtx { dvc_pdu, dvc_data })
}

fn encode_dvc_message(pdu: vc::dvc::ServerPdu) -> PduResult<SvcMessage> {
    // FIXME: use PduEncode instead
    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).map_err(|e| custom_err!("DVC server pdu", e))?;
    Ok(SvcMessage::from(buf).with_flags(ChannelFlags::SHOW_PROTOCOL))
}

// TODO: This is used by both client and server, so it should be moved to a common place
pub fn encode_dvc_data(channel_id: u32, messages: DvcMessages) -> PduResult<Vec<SvcMessage>> {
    let mut res = Vec::new();
    for msg in messages {
        let total_size = msg.size();

        let msg = encode_vec(msg.as_ref())?;
        let mut off = 0;

        while off < total_size {
            let rem = total_size.checked_sub(off).unwrap();
            let size = core::cmp::min(rem, DATA_MAX_SIZE);

            let pdu = if off == 0 && total_size >= DATA_MAX_SIZE {
                let total_size = cast_length!("encode_dvc_data", "totalDataSize", total_size)?;
                dvc::CommonPdu::DataFirst(DataFirstPdu::new(channel_id, total_size, DATA_MAX_SIZE))
            } else {
                dvc::CommonPdu::Data(DataPdu::new(channel_id, size))
            };

            let end = off
                .checked_add(size)
                .ok_or_else(|| other_err!("encode_dvc_data", "overflow occurred"))?;
            let mut data = Vec::new();
            pdu.to_buffer(&mut data).map_err(|e| custom_err!("DVC server pdu", e))?;
            data.extend_from_slice(&msg[off..end]);
            res.push(SvcMessage::from(data).with_flags(ChannelFlags::SHOW_PROTOCOL));
            off = end;
        }
    }

    Ok(res)
}
