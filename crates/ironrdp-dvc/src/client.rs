use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;
use core::{cmp, fmt};

use ironrdp_pdu as pdu;

use ironrdp_svc::{impl_as_any, CompressionCondition, SvcClientProcessor, SvcMessage, SvcProcessor};
use pdu::cursor::WriteCursor;
use pdu::gcc::ChannelName;
use pdu::rdp::vc;
use pdu::{dvc, PduResult};
use pdu::{other_err, PduEncode};

use crate::complete_data::CompleteData;
use crate::{encode_dvc_data, DvcMessages, DvcProcessor};

pub trait DvcClientProcessor: DvcProcessor {}

/// DRDYNVC Static Virtual Channel (the Remote Desktop Protocol: Dynamic Virtual Channel Extension)
///
/// It adds support for dynamic virtual channels (DVC).
pub struct DrdynvcClient {
    dynamic_channels: DynamicChannelSet,
    /// Indicates whether the capability request/response handshake has been completed.
    cap_handshake_done: bool,
}

impl fmt::Debug for DrdynvcClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DrdynvcClient([")?;

        for (i, channel) in self.dynamic_channels.values().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", channel.channel_name())?;
        }

        write!(f, "])")
    }
}

impl DrdynvcClient {
    pub const NAME: ChannelName = ChannelName::from_static(b"drdynvc\0");

    pub fn new() -> Self {
        Self {
            dynamic_channels: DynamicChannelSet::new(),
            cap_handshake_done: false,
        }
    }

    // FIXME(#61): it’s likely we want to enable adding dynamic channels at any point during the session (message passing? other approach?)

    #[must_use]
    pub fn with_dynamic_channel<T>(mut self, channel: T) -> Self
    where
        T: DvcClientProcessor + 'static,
    {
        self.dynamic_channels.insert(channel);
        self
    }

    fn create_capabilities_response(&mut self) -> SvcMessage {
        let caps_response = dvc::ClientPdu::CapabilitiesResponse(dvc::CapabilitiesResponsePdu {
            version: dvc::CapsVersion::V1,
        });
        debug!("Send DVC Capabilities Response PDU: {caps_response:?}");
        self.cap_handshake_done = true;
        SvcMessage::from(DvcMessage {
            dvc_pdu: caps_response,
            dvc_data: &[],
        })
    }
}

impl_as_any!(DrdynvcClient);

impl Default for DrdynvcClient {
    fn default() -> Self {
        Self::new()
    }
}

impl SvcProcessor for DrdynvcClient {
    fn channel_name(&self) -> ChannelName {
        DrdynvcClient::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::WhenRdpDataIsCompressed
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let dvc_ctx = decode_dvc_message(payload)?;
        let mut responses = Vec::new();

        match dvc_ctx.dvc_pdu {
            dvc::ServerPdu::CapabilitiesRequest(caps_request) => {
                debug!("Got DVC Capabilities Request PDU: {caps_request:?}");
                responses.push(self.create_capabilities_response());
            }
            dvc::ServerPdu::CreateRequest(create_request) => {
                debug!("Got DVC Create Request PDU: {create_request:?}");
                let channel_name = create_request.channel_name.clone();
                let channel_id = create_request.channel_id;

                if !self.cap_handshake_done {
                    debug!(
                        "Got DVC Create Request PDU before a Capabilities Request PDU. \
                        Sending Capabilities Response PDU before the Create Response PDU."
                    );
                    responses.push(self.create_capabilities_response());
                }

                let channel_exists = self.dynamic_channels.get_by_channel_name(&channel_name).is_some();
                let (creation_status, start_messages) = if channel_exists {
                    // If we have a handler for this channel, attach the channel ID
                    // and get any start messages.
                    self.dynamic_channels
                        .attach_channel_id(channel_name.clone(), channel_id);
                    let dynamic_channel = self.dynamic_channels.get_by_channel_name_mut(&channel_name).unwrap();
                    (dvc::DVC_CREATION_STATUS_OK, dynamic_channel.start(channel_id)?)
                } else {
                    (dvc::DVC_CREATION_STATUS_NO_LISTENER, vec![])
                };

                // Send the Create Response PDU.
                let create_response = dvc::ClientPdu::CreateResponse(dvc::CreateResponsePdu {
                    channel_id_type: create_request.channel_id_type,
                    channel_id,
                    creation_status,
                });
                debug!("Send DVC Create Response PDU: {create_response:?}");
                responses.push(SvcMessage::from(DvcMessage::new(create_response, &[])));

                // If this DVC has start messages, send them.
                if !start_messages.is_empty() {
                    responses.extend(encode_dvc_data(channel_id, start_messages)?);
                }
            }
            dvc::ServerPdu::CloseRequest(close_request) => {
                debug!("Got DVC Close Request PDU: {close_request:?}");

                let close_response = dvc::ClientPdu::CloseResponse(dvc::ClosePdu {
                    channel_id_type: close_request.channel_id_type,
                    channel_id: close_request.channel_id,
                });

                debug!("Send DVC Close Response PDU: {close_response:?}");
                responses.push(SvcMessage::from(DvcMessage::new(close_response, &[])));
                self.dynamic_channels.remove_by_channel_id(&close_request.channel_id);
            }
            dvc::ServerPdu::Common(dvc::CommonPdu::DataFirst(data)) => {
                let channel_id = data.channel_id;
                let dvc_data = dvc_ctx.dvc_data;

                let messages = self
                    .dynamic_channels
                    .get_by_channel_id_mut(&channel_id)
                    .ok_or_else(|| other_err!("DVC", "access to non existing channel"))?
                    .process(channel_id, dvc_data)?;

                responses.extend(encode_dvc_data(channel_id, messages)?);
            }
            dvc::ServerPdu::Common(dvc::CommonPdu::Data(data)) => {
                // TODO: identical to DataFirst, create a helper function
                let channel_id = data.channel_id;
                let dvc_data = dvc_ctx.dvc_data;

                let messages = self
                    .dynamic_channels
                    .get_by_channel_id_mut(&channel_id)
                    .ok_or_else(|| other_err!("DVC", "access to non existing channel"))?
                    .process(channel_id, dvc_data)?;

                responses.extend(encode_dvc_data(channel_id, messages)?);
            }
        }

        Ok(responses)
    }

    fn is_drdynvc(&self) -> bool {
        true
    }
}

impl SvcClientProcessor for DrdynvcClient {}

struct DynamicChannelCtx<'a> {
    dvc_pdu: vc::dvc::ServerPdu,
    dvc_data: &'a [u8],
}

fn decode_dvc_message(user_data: &[u8]) -> PduResult<DynamicChannelCtx<'_>> {
    use ironrdp_pdu::{custom_err, PduParsing as _};

    let mut user_data = user_data;
    let user_data_len = user_data.len();

    // … | dvc::ServerPdu | …
    let dvc_pdu =
        vc::dvc::ServerPdu::from_buffer(&mut user_data, user_data_len).map_err(|e| custom_err!("DVC server PDU", e))?;

    // … | DvcData ]
    let dvc_data = user_data;

    Ok(DynamicChannelCtx { dvc_pdu, dvc_data })
}

/// TODO: this is the same as server.rs's `DynamicChannelCtx`, can we unify them?
struct DvcMessage<'a> {
    dvc_pdu: vc::dvc::ClientPdu,
    dvc_data: &'a [u8],
}

impl<'a> DvcMessage<'a> {
    fn new(dvc_pdu: vc::dvc::ClientPdu, dvc_data: &'a [u8]) -> Self {
        Self { dvc_pdu, dvc_data }
    }
}

impl PduEncode for DvcMessage<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        self.dvc_pdu.to_buffer(dst)?;
        dst.write_slice(self.dvc_data);
        Ok(())
    }

    fn name(&self) -> &'static str {
        self.dvc_pdu.as_short_name()
    }

    fn size(&self) -> usize {
        self.dvc_pdu.buffer_length() + self.dvc_data.len()
    }
}

pub struct DynamicVirtualChannel {
    handler: Box<dyn DvcProcessor + Send>,
}

impl DynamicVirtualChannel {
    fn new<T: DvcProcessor + 'static>(handler: T) -> Self {
        Self {
            handler: Box::new(handler),
        }
    }

    fn start(&mut self, channel_id: DynamicChannelId) -> PduResult<DvcMessages> {
        self.handler.start(channel_id)
    }

    fn process(&mut self, channel_id: DynamicChannelId, data: &[u8]) -> PduResult<DvcMessages> {
        self.handler.process(channel_id, data)
    }

    fn channel_name(&self) -> &str {
        self.handler.channel_name()
    }
}

struct DynamicChannelSet {
    channels: BTreeMap<DynamicChannelName, DynamicVirtualChannel>,
    name_to_id: BTreeMap<DynamicChannelName, DynamicChannelId>,
    id_to_name: BTreeMap<DynamicChannelId, DynamicChannelName>,
}

impl DynamicChannelSet {
    #[inline]
    fn new() -> Self {
        Self {
            channels: BTreeMap::new(),
            name_to_id: BTreeMap::new(),
            id_to_name: BTreeMap::new(),
        }
    }

    pub fn insert<T: DvcProcessor + 'static>(&mut self, channel: T) -> Option<DynamicVirtualChannel> {
        let name = channel.channel_name().to_owned();
        self.channels.insert(name, DynamicVirtualChannel::new(channel))
    }

    pub fn get_by_channel_name(&self, name: &DynamicChannelName) -> Option<&DynamicVirtualChannel> {
        self.channels.get(name)
    }

    pub fn get_by_channel_name_mut(&mut self, name: &DynamicChannelName) -> Option<&mut DynamicVirtualChannel> {
        self.channels.get_mut(name)
    }

    pub fn get_by_channel_id(&self, id: &DynamicChannelId) -> Option<&DynamicVirtualChannel> {
        self.id_to_name.get(id).and_then(|name| self.channels.get(name))
    }

    pub fn get_by_channel_id_mut(&mut self, id: &DynamicChannelId) -> Option<&mut DynamicVirtualChannel> {
        self.id_to_name.get(id).and_then(|name| self.channels.get_mut(name))
    }

    pub fn attach_channel_id(&mut self, name: DynamicChannelName, id: DynamicChannelId) -> Option<DynamicChannelId> {
        let channel = self.get_by_channel_name_mut(&name)?;
        self.id_to_name.insert(id, name.clone());
        self.name_to_id.insert(name, id)
    }

    pub fn remove_by_channel_id(&mut self, id: &DynamicChannelId) -> Option<DynamicChannelId> {
        if let Some(name) = self.id_to_name.remove(id) {
            return self.name_to_id.remove(&name);
            // Channels are retained in the `self.channels` map to allow potential re-addition by the server.
        }
        None
    }

    #[inline]
    pub fn values(&self) -> impl Iterator<Item = &DynamicVirtualChannel> {
        self.channels.values()
    }
}

type DynamicChannelName = String;
type DynamicChannelId = u32;
