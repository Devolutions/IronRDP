mod display;
mod gfx;

use std::borrow::Cow;
use std::cmp;
use std::collections::HashMap;

use ironrdp_connector::legacy::SendDataIndicationCtx;
use ironrdp_connector::GraphicsConfig;
use ironrdp_pdu::dvc::FieldType;
use ironrdp_pdu::rdp::headers::ShareDataPdu;
use ironrdp_pdu::rdp::server_error_info::{ErrorInfo, ProtocolIndependentCode, ServerSetErrorInfoPdu};
use ironrdp_pdu::rdp::vc::dvc;
use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::{encode_buf, mcs};
use ironrdp_svc::{StaticChannelSet, StaticVirtualChannelProcessor, SvcMessage, SvcRequest};

pub use self::gfx::GfxHandler;
use crate::{SessionErrorExt as _, SessionResult};

pub const RDP8_GRAPHICS_PIPELINE_NAME: &str = "Microsoft::Windows::RDS::Graphics";
pub const RDP8_DISPLAY_PIPELINE_NAME: &str = "Microsoft::Windows::RDS::DisplayControl";

pub struct Processor {
    channel_map: HashMap<String, u32>,
    static_channels: StaticChannelSet,
    dynamic_channels: HashMap<u32, DynamicChannel>,
    user_channel_id: u16,
    io_channel_id: u16,
    drdynvc_channel_id: Option<u16>,
    graphics_config: Option<GraphicsConfig>,
    graphics_handler: Option<Box<dyn GfxHandler + Send>>,
}

impl Processor {
    pub fn new(
        static_channels: StaticChannelSet,
        user_channel_id: u16,
        io_channel_id: u16,
        graphics_config: Option<GraphicsConfig>,
        graphics_handler: Option<Box<dyn GfxHandler + Send>>,
    ) -> Self {
        let drdynvc_channel_id = static_channels.iter().find_map(|(type_id, channel)| {
            if channel.is_drdynvc() {
                static_channels.get_channel_id_by_type_id(type_id)
            } else {
                None
            }
        });

        Self {
            static_channels,
            dynamic_channels: HashMap::new(),
            channel_map: HashMap::new(),
            user_channel_id,
            io_channel_id,
            drdynvc_channel_id,
            graphics_config,
            graphics_handler,
        }
    }

    pub fn get_svc_processor_downcast_ref<T: StaticVirtualChannelProcessor + 'static>(&mut self) -> Option<&T> {
        self.static_channels
            .get_by_type::<T>()
            .and_then(|svc| svc.channel_processor_downcast_ref())
    }

    pub fn get_svc_processor_downcast_mut<T: StaticVirtualChannelProcessor + 'static>(&mut self) -> Option<&mut T> {
        self.static_channels
            .get_by_type_mut::<T>()
            .and_then(|svc| svc.channel_processor_downcast_mut())
    }

    /// Completes user's SVC request with data, required to sent it over the network and returns
    /// a buffer with encoded data.
    pub fn process_user_svc_request<C: StaticVirtualChannelProcessor + 'static>(
        &self,
        request: SvcRequest<C>,
    ) -> SessionResult<Vec<u8>> {
        let channel_id = self
            .static_channels
            .get_channel_id_by_type::<C>()
            .ok_or_else(|| reason_err!("SVC", "channel not found"))?;

        self.process_svc_request(request.into(), channel_id, self.user_channel_id)
    }

    pub fn process(&mut self, frame: &[u8]) -> SessionResult<Vec<u8>> {
        let data_ctx: SendDataIndicationCtx<'_> =
            ironrdp_connector::legacy::decode_send_data_indication(frame).map_err(crate::legacy::map_error)?;
        let channel_id = data_ctx.channel_id;

        if channel_id == self.io_channel_id {
            self.process_io_channel(data_ctx)?;
            Ok(Vec::new())
        } else {
            match self.drdynvc_channel_id {
                Some(drdynvc_id) if channel_id == drdynvc_id => self.process_dyvc(data_ctx),
                _ => {
                    if let Some(svc) = self.static_channels.get_by_channel_id_mut(channel_id) {
                        let response_pdus = svc.process(data_ctx.user_data).map_err(crate::SessionError::pdu)?;
                        self.process_svc_request(response_pdus, channel_id, data_ctx.initiator_id)
                    } else {
                        Err(reason_err!("X224", "unexpected channel received: ID {channel_id}"))
                    }
                }
            }
        }
    }

    fn process_io_channel(&self, data_ctx: SendDataIndicationCtx<'_>) -> SessionResult<()> {
        debug_assert_eq!(data_ctx.channel_id, self.io_channel_id);

        let ctx = ironrdp_connector::legacy::decode_share_data(data_ctx).map_err(crate::legacy::map_error)?;

        match ctx.pdu {
            ShareDataPdu::SaveSessionInfo(session_info) => {
                debug!("Got Session Save Info PDU: {session_info:?}");
                Ok(())
            }
            ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(ErrorInfo::ProtocolIndependentCode(
                ProtocolIndependentCode::None,
            ))) => {
                debug!("Received None server error");
                Ok(())
            }
            ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(e)) => {
                Err(reason_err!("ServerSetErrorInfo", "{}", e.description()))
            }
            _ => Err(reason_err!(
                "IO channel",
                "unexpected PDU: expected Session Save Info PDU, got: {:?}",
                ctx.pdu.as_short_name()
            )),
        }
    }

    fn process_dyvc(&mut self, data_ctx: SendDataIndicationCtx<'_>) -> SessionResult<Vec<u8>> {
        debug_assert_eq!(Some(data_ctx.channel_id), self.drdynvc_channel_id);

        let dvc_ctx = crate::legacy::decode_dvc_message(data_ctx)?;

        let mut buf = WriteBuf::new();

        match dvc_ctx.dvc_pdu {
            dvc::ServerPdu::CapabilitiesRequest(caps_request) => {
                debug!("Got DVC Capabilities Request PDU: {caps_request:?}");
                let caps_response = dvc::ClientPdu::CapabilitiesResponse(dvc::CapabilitiesResponsePdu {
                    version: dvc::CapsVersion::V1,
                });

                debug!("Send DVC Capabilities Response PDU: {caps_response:?}");
                crate::legacy::encode_dvc_message(
                    data_ctx.initiator_id,
                    data_ctx.channel_id,
                    caps_response,
                    &[],
                    &mut buf,
                )?;
            }
            dvc::ServerPdu::CreateRequest(create_request) => {
                debug!("Got DVC Create Request PDU: {create_request:?}");

                let creation_status = if let Some(dynamic_channel) = create_dvc(
                    create_request.channel_name.as_str(),
                    create_request.channel_id,
                    create_request.channel_id_type,
                    &mut self.graphics_handler,
                ) {
                    self.dynamic_channels.insert(create_request.channel_id, dynamic_channel);
                    self.channel_map
                        .insert(create_request.channel_name.clone(), create_request.channel_id);

                    dvc::DVC_CREATION_STATUS_OK
                } else {
                    dvc::DVC_CREATION_STATUS_NO_LISTENER
                };

                let create_response = dvc::ClientPdu::CreateResponse(dvc::CreateResponsePdu {
                    channel_id_type: create_request.channel_id_type,
                    channel_id: create_request.channel_id,
                    creation_status,
                });

                debug!("Send DVC Create Response PDU: {create_response:?}");
                crate::legacy::encode_dvc_message(
                    data_ctx.initiator_id,
                    data_ctx.channel_id,
                    create_response,
                    &[],
                    &mut buf,
                )?;

                negotiate_dvc(
                    &create_request,
                    data_ctx.initiator_id,
                    data_ctx.channel_id,
                    &mut buf,
                    &self.graphics_config,
                )?;
            }
            dvc::ServerPdu::CloseRequest(close_request) => {
                debug!("Got DVC Close Request PDU: {close_request:?}");

                let close_response = dvc::ClientPdu::CloseResponse(dvc::ClosePdu {
                    channel_id_type: close_request.channel_id_type,
                    channel_id: close_request.channel_id,
                });

                debug!("Send DVC Close Response PDU: {close_response:?}");
                crate::legacy::encode_dvc_message(
                    data_ctx.initiator_id,
                    data_ctx.channel_id,
                    close_response,
                    &[],
                    &mut buf,
                )?;

                self.dynamic_channels.remove(&close_request.channel_id);
            }
            dvc::ServerPdu::DataFirst(data) => {
                let channel_id_type = data.channel_id_type;
                let channel_id = data.channel_id;

                let dvc_data = dvc_ctx.dvc_data;

                // FIXME(perf): copy with data_buf.to_vec()
                if let Some(dvc_data) = self
                    .dynamic_channels
                    .get_mut(&data.channel_id)
                    .ok_or_else(|| reason_err!("DVC", "access to non existing channel: {}", data.channel_id))?
                    .process_data_first_pdu(data.total_data_size as usize, dvc_data.to_vec())?
                {
                    let client_data = dvc::ClientPdu::Data(dvc::DataPdu {
                        channel_id_type,
                        channel_id,
                        data_size: dvc_data.len(),
                    });

                    crate::legacy::encode_dvc_message(
                        data_ctx.initiator_id,
                        data_ctx.channel_id,
                        client_data,
                        &dvc_data,
                        &mut buf,
                    )?;
                }
            }
            dvc::ServerPdu::Data(data) => {
                let channel_id_type = data.channel_id_type;
                let channel_id = data.channel_id;

                let dvc_data = dvc_ctx.dvc_data;

                // FIXME(perf): copy with data_buf.to_vec()
                if let Some(dvc_data) = self
                    .dynamic_channels
                    .get_mut(&data.channel_id)
                    .ok_or_else(|| reason_err!("DVC", "access to non existing channel: {}", data.channel_id))?
                    .process_data_pdu(dvc_data.to_vec())?
                {
                    let client_data = dvc::ClientPdu::Data(dvc::DataPdu {
                        channel_id_type,
                        channel_id,
                        data_size: dvc_data.len(),
                    });

                    crate::legacy::encode_dvc_message(
                        data_ctx.initiator_id,
                        data_ctx.channel_id,
                        client_data,
                        &dvc_data,
                        &mut buf,
                    )?;
                }
            }
        }

        Ok(buf.into_inner())
    }

    /// Sends a PDU on the dynamic channel.
    pub fn encode_dynamic(&self, output: &mut WriteBuf, channel_name: &str, dvc_data: &[u8]) -> SessionResult<()> {
        let drdynvc_channel_id = self
            .drdynvc_channel_id
            .ok_or_else(|| general_err!("dynamic virtual channel not connected"))?;

        let dvc_channel_id = self
            .channel_map
            .get(channel_name)
            .ok_or_else(|| reason_err!("DVC", "access to non existing channel name: {}", channel_name))?;

        let dvc_channel = self
            .dynamic_channels
            .get(dvc_channel_id)
            .ok_or_else(|| reason_err!("DVC", "access to non existing channel: {}", dvc_channel_id))?;

        let dvc_client_data = dvc::ClientPdu::Data(dvc::DataPdu {
            channel_id_type: dvc_channel.channel_id_type,
            channel_id: dvc_channel.channel_id,
            data_size: dvc_data.len(),
        });

        crate::legacy::encode_dvc_message(
            self.user_channel_id,
            drdynvc_channel_id,
            dvc_client_data,
            dvc_data,
            output,
        )?;

        Ok(())
    }

    /// Send a pdu on the static global channel. Typically used to send input events
    pub fn encode_static(&self, output: &mut WriteBuf, pdu: ShareDataPdu) -> SessionResult<usize> {
        let written =
            ironrdp_connector::legacy::encode_share_data(self.user_channel_id, self.io_channel_id, 0, pdu, output)
                .map_err(crate::legacy::map_error)?;
        Ok(written)
    }

    fn process_svc_request(
        &self,
        messages: Vec<SvcMessage>,
        channel_id: u16,
        initiator_id: u16,
    ) -> SessionResult<Vec<u8>> {
        let svc = self.static_channels.get_by_channel_id(channel_id).ok_or_else(|| {
            reason_err!(
                "SVC",
                "access to non existing channel: ID {channel_id}",
                channel_id = channel_id
            )
        })?;

        // For each response PDU, chunkify it and add appropriate static channel headers.
        let chunks = svc.chunkify(messages).map_err(crate::SessionError::pdu)?;

        // Place each chunk into a SendDataRequest
        let mcs_pdus = chunks
            .into_iter()
            .map(|buf| mcs::SendDataRequest {
                initiator_id,
                channel_id,
                user_data: Cow::Owned(buf.into_inner()),
            })
            .collect::<Vec<mcs::SendDataRequest>>();

        // SendDataRequest is [`McsPdu`], which is [`x224Pdu`], which is [`PduEncode`]. [`PduEncode`] for [`x224Pdu`]
        // also takes care of adding the Tpkt header, so therefore we can just call `encode_buf` on each of these and
        // we will create a buffer of fully encoded PDUs ready to send to the server.
        //
        // For example, if we had 2 chunks, our fully_encoded_responses buffer would look like:
        //
        // [ | tpkt | x224 | mcs::SendDataRequest | chunk 1 | tpkt | x224 | mcs::SendDataRequest | chunk 2 | ]
        //   |<------------------- PDU 1 ------------------>|<------------------- PDU 2 ------------------>|
        let mut fully_encoded_responses = WriteBuf::new(); // TODO(perf): reuse this buffer using `clear` and `filled` as appropriate
        for pdu in mcs_pdus {
            encode_buf(&pdu, &mut fully_encoded_responses).map_err(crate::SessionError::pdu)?;
        }

        Ok(fully_encoded_responses.into_inner())
    }
}

fn create_dvc(
    channel_name: &str,
    channel_id: u32,
    channel_id_type: FieldType,
    graphics_handler: &mut Option<Box<dyn GfxHandler + Send>>,
) -> Option<DynamicChannel> {
    match channel_name {
        RDP8_GRAPHICS_PIPELINE_NAME => {
            let handler = graphics_handler.take();
            Some(DynamicChannel::new(
                Box::new(gfx::Handler::new(handler)),
                channel_id,
                channel_id_type,
            ))
        }
        RDP8_DISPLAY_PIPELINE_NAME => Some(DynamicChannel::new(
            Box::new(display::Handler),
            channel_id,
            channel_id_type,
        )),
        _ => {
            warn!(channel_name, "Unsupported dynamic virtual channel");
            None
        }
    }
}

fn negotiate_dvc(
    create_request: &dvc::CreateRequestPdu,
    initiator_id: u16,
    channel_id: u16,
    buf: &mut WriteBuf,
    graphics_config: &Option<GraphicsConfig>,
) -> SessionResult<()> {
    if create_request.channel_name == RDP8_GRAPHICS_PIPELINE_NAME {
        let dvc_data = gfx::create_capabilities_advertise(graphics_config)?;
        let dvc_pdu = dvc::ClientPdu::Data(dvc::DataPdu {
            channel_id_type: create_request.channel_id_type,
            channel_id: create_request.channel_id,
            data_size: dvc_data.len(),
        });

        debug!("Send GFX Capabilities Advertise PDU");
        crate::legacy::encode_dvc_message(initiator_id, channel_id, dvc_pdu, &dvc_data, buf)?;
    }

    Ok(())
}

trait DynamicChannelDataHandler {
    fn process_complete_data(&mut self, complete_data: Vec<u8>) -> SessionResult<Option<Vec<u8>>>;
}

pub struct DynamicChannel {
    data: CompleteData,
    channel_id_type: FieldType,
    channel_id: u32,
    handler: Box<dyn DynamicChannelDataHandler + Send>,
}

impl DynamicChannel {
    fn new(handler: Box<dyn DynamicChannelDataHandler + Send>, channel_id: u32, channel_id_type: FieldType) -> Self {
        Self {
            data: CompleteData::new(),
            handler,
            channel_id_type,
            channel_id,
        }
    }

    fn process_data_first_pdu(&mut self, total_data_size: usize, data: Vec<u8>) -> SessionResult<Option<Vec<u8>>> {
        if let Some(complete_data) = self.data.process_data_first_pdu(total_data_size, data) {
            self.handler.process_complete_data(complete_data)
        } else {
            Ok(None)
        }
    }

    fn process_data_pdu(&mut self, data: Vec<u8>) -> SessionResult<Option<Vec<u8>>> {
        if let Some(complete_data) = self.data.process_data_pdu(data) {
            self.handler.process_complete_data(complete_data)
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug, PartialEq)]
struct CompleteData {
    total_size: usize,
    data: Vec<u8>,
}

impl CompleteData {
    fn new() -> Self {
        Self {
            total_size: 0,
            data: Vec::new(),
        }
    }

    fn process_data_first_pdu(&mut self, total_data_size: usize, data: Vec<u8>) -> Option<Vec<u8>> {
        if self.total_size != 0 || !self.data.is_empty() {
            error!("Incomplete DVC message, it will be skipped");

            self.data.clear();
        }

        if total_data_size == data.len() {
            Some(data)
        } else {
            self.total_size = total_data_size;
            self.data = data;

            None
        }
    }

    fn process_data_pdu(&mut self, mut data: Vec<u8>) -> Option<Vec<u8>> {
        if self.total_size == 0 && self.data.is_empty() {
            // message is not fragmented
            Some(data)
        } else {
            // message is fragmented so need to reassemble it
            let actual_data_length = self.data.len() + data.len();

            match actual_data_length.cmp(&(self.total_size)) {
                cmp::Ordering::Less => {
                    // this is one of the fragmented messages, just append it
                    self.data.append(&mut data);
                    None
                }
                cmp::Ordering::Equal => {
                    // this is the last fragmented message, need to return the whole reassembled message
                    self.total_size = 0;
                    self.data.append(&mut data);
                    Some(self.data.drain(..).collect())
                }
                cmp::Ordering::Greater => {
                    error!("Actual DVC message size is grater than expected total DVC message size");
                    self.total_size = 0;
                    self.data.clear();

                    None
                }
            }
        }
    }
}
