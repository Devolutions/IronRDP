use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::any::{Any, TypeId};
use core::{cmp, fmt};

use ironrdp_pdu as pdu;

use ironrdp_svc::{impl_as_any, CompressionCondition, SvcClientProcessor, SvcMessage, SvcPduEncode, SvcProcessor};
use pdu::cursor::WriteCursor;
use pdu::gcc::ChannelName;
use pdu::rdp::vc;
use pdu::{dvc, PduResult};
use pdu::{other_err, PduEncode};

use crate::complete_data::CompleteData;
use crate::pdu::{CapabilitiesResponsePdu, CapsVersion, ClosePdu, CreateResponsePdu, CreationStatus, DrdynvcPdu};
use crate::{encode_dvc_messages, DvcMessages, DvcProcessor, DynamicChannelId, DynamicChannelSet};

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
        T: DvcProcessor + 'static,
    {
        self.dynamic_channels.insert(channel);
        self
    }

    pub fn get_dynamic_channel_by_type_id<T>(&self) -> Option<(&T, Option<DynamicChannelId>)>
    where
        T: DvcProcessor,
    {
        self.dynamic_channels
            .get_by_type_id(TypeId::of::<T>())
            .and_then(|(channel, channel_id)| {
                channel
                    .channel_processor_downcast_ref()
                    .map(|channel| (channel as &T, channel_id))
            })
    }

    fn create_capabilities_response(&mut self) -> SvcMessage {
        let caps_response = DrdynvcPdu::CapabilitiesResponse(CapabilitiesResponsePdu::new(CapsVersion::V1));
        debug!("Send DVC Capabilities Response PDU: {caps_response:?}");
        self.cap_handshake_done = true;
        SvcMessage::from(caps_response)
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
                    (CreationStatus::OK, dynamic_channel.start(channel_id)?)
                } else {
                    (CreationStatus::NO_LISTENER, vec![])
                };

                let create_response = DrdynvcPdu::CreateResponse(CreateResponsePdu::new(channel_id, creation_status));
                debug!("Send DVC Create Response PDU: {create_response:?}");
                responses.push(SvcMessage::from(create_response));

                // If this DVC has start messages, send them.
                if !start_messages.is_empty() {
                    responses.extend(encode_dvc_messages(channel_id, start_messages, None)?);
                }
            }
            dvc::ServerPdu::CloseRequest(close_request) => {
                debug!("Got DVC Close Request PDU: {close_request:?}");
                self.dynamic_channels.remove_by_channel_id(&close_request.channel_id);

                let close_response = DrdynvcPdu::Close(ClosePdu::new(close_request.channel_id));

                debug!("Send DVC Close Response PDU: {close_response:?}");
                responses.push(SvcMessage::from(close_response));
            }
            dvc::ServerPdu::Common(common) => {
                let channel_id = common.channel_id();
                let dvc_data = dvc_ctx.dvc_data;

                let messages = self
                    .dynamic_channels
                    .get_by_channel_id_mut(&channel_id)
                    .ok_or_else(|| other_err!("DVC", "access to non existing channel"))?
                    .process(common, dvc_data)?;

                responses.extend(encode_dvc_messages(channel_id, messages, None)?);
            }
        }

        Ok(responses)
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
