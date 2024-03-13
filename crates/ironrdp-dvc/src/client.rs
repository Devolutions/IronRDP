use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;
use core::fmt;

use ironrdp_pdu as pdu;

use ironrdp_svc::{impl_as_any, CompressionCondition, SvcClientProcessor, SvcMessage, SvcProcessor};
use pdu::cursor::WriteCursor;
use pdu::gcc::ChannelName;
use pdu::rdp::vc;
use pdu::PduEncode;
use pdu::{dvc, PduResult};

use crate::DvcProcessor;

pub trait DvcClientProcessor: DvcProcessor {}

/// DRDYNVC Static Virtual Channel (the Remote Desktop Protocol: Dynamic Virtual Channel Extension)
///
/// It adds support for dynamic virtual channels (DVC).
pub struct DrdynvcClient {
    dynamic_channels: BTreeMap<String, Box<dyn DvcClientProcessor>>,
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
            dynamic_channels: BTreeMap::new(),
            cap_handshake_done: false,
        }
    }

    // FIXME(#61): it’s likely we want to enable adding dynamic channels at any point during the session (message passing? other approach?)

    #[must_use]
    pub fn with_dynamic_channel<T>(mut self, channel: T) -> Self
    where
        T: DvcClientProcessor + 'static,
    {
        let channel_name = channel.channel_name().to_owned();
        self.dynamic_channels.insert(channel_name, Box::new(channel));
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

                if !self.cap_handshake_done {
                    debug!(
                        "Got DVC Create Request PDU before a Capabilities Request PDU. \
                        Sending Capabilities Response PDU before the Create Response PDU."
                    );
                    responses.push(self.create_capabilities_response());
                }

                // let creation_status = if let Some(dynamic_channel) = create_dvc(
                //     create_request.channel_name.as_str(),
                //     create_request.channel_id,
                //     create_request.channel_id_type,
                //     &mut self.graphics_handler,
                // ) {
                //     self.dynamic_channels.insert(create_request.channel_id, dynamic_channel);
                //     self.channel_map
                //         .insert(create_request.channel_name.clone(), create_request.channel_id);

                //     dvc::DVC_CREATION_STATUS_OK
                // } else {
                //     dvc::DVC_CREATION_STATUS_NO_LISTENER
                // };

                // let create_response = dvc::ClientPdu::CreateResponse(dvc::CreateResponsePdu {
                //     channel_id_type: create_request.channel_id_type,
                //     channel_id: create_request.channel_id,
                //     creation_status,
                // });

                // debug!("Send DVC Create Response PDU: {create_response:?}");
                // crate::legacy::encode_dvc_message(
                //     data_ctx.initiator_id,
                //     data_ctx.channel_id,
                //     create_response,
                //     &[],
                //     &mut buf,
                // )?;

                // negotiate_dvc(
                //     &create_request,
                //     data_ctx.initiator_id,
                //     data_ctx.channel_id,
                //     &mut buf,
                //     &self.graphics_config,
                // )?;
            }
            dvc::ServerPdu::CloseRequest(close_request) => {
                debug!("Got DVC Close Request PDU: {close_request:?}");

                let close_response = dvc::ClientPdu::CloseResponse(dvc::ClosePdu {
                    channel_id_type: close_request.channel_id_type,
                    channel_id: close_request.channel_id,
                });

                // debug!("Send DVC Close Response PDU: {close_response:?}");
                // crate::legacy::encode_dvc_message(
                //     data_ctx.initiator_id,
                //     data_ctx.channel_id,
                //     close_response,
                //     &[],
                //     &mut buf,
                // )?;

                // self.dynamic_channels.remove(&close_request.channel_id);
            }
            dvc::ServerPdu::DataFirst(data) => {
                let channel_id_type = data.channel_id_type;
                let channel_id = data.channel_id;

                let dvc_data = dvc_ctx.dvc_data;

                // // FIXME(perf): copy with data_buf.to_vec()
                // if let Some(dvc_data) = self
                //     .dynamic_channels
                //     .get_mut(&data.channel_id)
                //     .ok_or_else(|| reason_err!("DVC", "access to non existing channel: {}", data.channel_id))?
                //     .process_data_first_pdu(data.total_data_size as usize, dvc_data.to_vec())?
                // {
                //     let client_data = dvc::ClientPdu::Data(dvc::DataPdu {
                //         channel_id_type,
                //         channel_id,
                //         data_size: dvc_data.len(),
                //     });

                //     crate::legacy::encode_dvc_message(
                //         data_ctx.initiator_id,
                //         data_ctx.channel_id,
                //         client_data,
                //         &dvc_data,
                //         &mut buf,
                //     )?;
                // }
            }
            dvc::ServerPdu::Data(data) => {
                let channel_id_type = data.channel_id_type;
                let channel_id = data.channel_id;

                let dvc_data = dvc_ctx.dvc_data;

                // // FIXME(perf): copy with data_buf.to_vec()
                // if let Some(dvc_data) = self
                //     .dynamic_channels
                //     .get_mut(&data.channel_id)
                //     .ok_or_else(|| reason_err!("DVC", "access to non existing channel: {}", data.channel_id))?
                //     .process_data_pdu(dvc_data.to_vec())?
                // {
                //     let client_data = dvc::ClientPdu::Data(dvc::DataPdu {
                //         channel_id_type,
                //         channel_id,
                //         data_size: dvc_data.len(),
                //     });

                //     crate::legacy::encode_dvc_message(
                //         data_ctx.initiator_id,
                //         data_ctx.channel_id,
                //         client_data,
                //         &dvc_data,
                //         &mut buf,
                //     )?;
                // }
            }
        }

        if !responses.is_empty() {
            Ok(responses)
        } else {
            Err(ironrdp_pdu::other_err!(
                "DRDYNVC",
                "ironrdp-dvc::DrdynvcClient implementation is not yet ready"
            ))
        }
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

struct DvcMessage<'a> {
    dvc_pdu: vc::dvc::ClientPdu,
    dvc_data: &'a [u8],
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
