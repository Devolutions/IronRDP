mod display;
mod gfx;

use std::collections::HashMap;
use std::{cmp, io};

use bytes::{BufMut as _, Bytes, BytesMut};
use ironrdp_core::dvc::FieldType;
use ironrdp_core::rdp::vc::{self, dvc};
use ironrdp_core::rdp::{ErrorInfo, ProtocolIndependentCode, ServerSetErrorInfoPdu};
use ironrdp_core::ShareDataPdu;

pub use self::gfx::GfxHandler;
use crate::frame::{
    DynamicVirtualChannelClientFrame, DynamicVirtualChannelServerFrame, Frame as _, SendDataInfoFrame, ShareDataFrame,
};
use crate::{ChannelIdentificators, GraphicsConfig, RdpError};

pub const RDP8_GRAPHICS_PIPELINE_NAME: &str = "Microsoft::Windows::RDS::Graphics";
pub const RDP8_DISPLAY_PIPELINE_NAME: &str = "Microsoft::Windows::RDS::DisplayControl";

pub struct Processor {
    channel_map: HashMap<String, u32>,
    dynamic_channels: HashMap<u32, DynamicChannel>,
    global_initiatior_id: u16,
    global_channel_id: u16,
    drdynvc_channel_id: Option<u16>,
    graphics_config: Option<GraphicsConfig>,
    graphics_handler: Option<Box<dyn GfxHandler + Send>>,
}

impl Processor {
    pub fn new(
        static_channels: HashMap<u16, String>,
        global_initiatior_id: u16,
        global_channel_id: u16,
        graphics_config: Option<GraphicsConfig>,
        graphics_handler: Option<Box<dyn GfxHandler + Send>>,
    ) -> Self {
        let drdynvc_channel_id = static_channels.iter().find_map(|(id, name)| {
            if name == vc::DRDYNVC_CHANNEL_NAME {
                Some(*id)
            } else {
                None
            }
        });

        Self {
            dynamic_channels: HashMap::new(),
            channel_map: HashMap::new(),
            global_initiatior_id,
            global_channel_id,
            drdynvc_channel_id,
            graphics_config,
            graphics_handler,
        }
    }

    pub fn process(&mut self, frame: Bytes) -> Result<BytesMut, RdpError> {
        // FIXME(perf): there is decoding work done multiple times.
        // I believe the `Frame` trait is not appropriate here and its use should be replaced.

        let info_frame = SendDataInfoFrame::decode(&frame[..])?;

        let channel_id = info_frame.channel_ids.channel_id;

        if channel_id == self.global_channel_id {
            // NOTE: my understanding is that this so-called global channel is actually called the I/O channel in official doc
            self.process_global_channel_pdu(frame)?;
            Ok(BytesMut::new())
        } else {
            match self.drdynvc_channel_id {
                Some(dyvc_id) if channel_id == dyvc_id => self.process_dvc_message(frame),
                _ => Err(RdpError::UnexpectedChannel(channel_id)),
            }
        }
    }

    /// Sends a PDU on the dynamic channel. The upper layers are responsible for encoding the PDU and converting them to message
    pub fn send_dynamic(
        &mut self,
        stream: impl io::Write,
        channel_name: &str,
        extra_data: Bytes,
    ) -> Result<(), RdpError> {
        let drdynvc_channel_id = self
            .drdynvc_channel_id
            .ok_or_else(|| RdpError::DynamicVirtualChannelNotConnected)?;

        let dvc_channel_id = self
            .channel_map
            .get(channel_name)
            .ok_or_else(|| RdpError::AccessToNonExistingChannelName(channel_name.to_string()))?;

        let dvc_channel = self
            .dynamic_channels
            .get_mut(dvc_channel_id)
            .ok_or(RdpError::AccessToNonExistingChannel(*dvc_channel_id))?;

        let dvc_client_data = dvc::ClientPdu::Data(dvc::DataPdu {
            channel_id_type: dvc_channel.channel_id_type,
            channel_id: dvc_channel.channel_id,
            data_size: extra_data.len(),
        });

        DynamicVirtualChannelClientFrame {
            channel_ids: ChannelIdentificators {
                initiator_id: self.global_initiatior_id,
                channel_id: drdynvc_channel_id,
            },
            dvc_pdu: dvc_client_data,
            extra_data,
        }
        .encode(stream)?;

        Ok(())
    }

    /// Send a pdu on the static global channel. Typically used to send input events
    pub fn send_static(&self, stream: impl io::Write, message: ShareDataPdu) -> Result<(), RdpError> {
        ShareDataFrame {
            channel_ids: ChannelIdentificators {
                initiator_id: self.global_initiatior_id,
                channel_id: self.global_channel_id,
            },
            share_id: 0,
            pdu_source: self.global_initiatior_id,
            pdu: message,
        }
        .encode(stream)
    }

    fn process_global_channel_pdu(&self, frame: Bytes) -> Result<(), RdpError> {
        let share_data_frame = ShareDataFrame::decode(&frame[..])?;

        if share_data_frame.channel_ids.channel_id != self.global_channel_id {
            return Err(RdpError::InvalidResponse(format!(
                "Unexpected Share Data channel ID ({})",
                share_data_frame.channel_ids.channel_id,
            )));
        }

        match share_data_frame.pdu {
            ShareDataPdu::SaveSessionInfo(session_info) => {
                debug!("Got Session Save Info PDU: {:?}", session_info);
                Ok(())
            }
            ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(ErrorInfo::ProtocolIndependentCode(
                ProtocolIndependentCode::None,
            ))) => {
                debug!("Received None server error");
                Ok(())
            }
            ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(e)) => Err(RdpError::Server(e.description())),
            _ => Err(RdpError::UnexpectedPdu(format!(
                "Expected Session Save Info PDU, got: {:?}",
                share_data_frame.pdu.as_short_name()
            ))),
        }
    }

    fn process_dvc_message(&mut self, frame: Bytes) -> Result<BytesMut, RdpError> {
        let drdynvc_frame = DynamicVirtualChannelServerFrame::decode(&frame[..])?;
        let mut buf_writer = BytesMut::new().writer();

        match drdynvc_frame.dvc_pdu {
            dvc::ServerPdu::CapabilitiesRequest(caps_request) => {
                debug!("Got DVC Capabilities Request PDU: {:?}", caps_request);
                let caps_response = dvc::ClientPdu::CapabilitiesResponse(dvc::CapabilitiesResponsePdu {
                    version: dvc::CapsVersion::V1,
                });

                debug!("Send DVC Capabilities Response PDU: {:?}", caps_response);
                DynamicVirtualChannelClientFrame {
                    channel_ids: drdynvc_frame.channel_ids,
                    dvc_pdu: caps_response,
                    extra_data: Bytes::new(),
                }
                .encode(&mut buf_writer)?;
            }
            dvc::ServerPdu::CreateRequest(create_request) => {
                debug!("Got DVC Create Request PDU: {:?}", create_request);

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

                debug!("Send DVC Create Response PDU: {:?}", create_response);
                DynamicVirtualChannelClientFrame {
                    channel_ids: drdynvc_frame.channel_ids,
                    dvc_pdu: create_response,
                    extra_data: Bytes::new(),
                }
                .encode(&mut buf_writer)?;

                negotiate_dvc(
                    &create_request,
                    drdynvc_frame.channel_ids,
                    &mut buf_writer,
                    &self.graphics_config,
                )?;
            }
            dvc::ServerPdu::CloseRequest(close_request) => {
                debug!("Got DVC Close Request PDU: {:?}", close_request);

                let close_response = dvc::ClientPdu::CloseResponse(dvc::ClosePdu {
                    channel_id_type: close_request.channel_id_type,
                    channel_id: close_request.channel_id,
                });

                debug!("Send DVC Close Response PDU: {:?}", close_response);
                DynamicVirtualChannelClientFrame {
                    channel_ids: drdynvc_frame.channel_ids,
                    dvc_pdu: close_response,
                    extra_data: Bytes::new(),
                }
                .encode(&mut buf_writer)?;

                self.dynamic_channels.remove(&close_request.channel_id);
            }
            dvc::ServerPdu::DataFirst(data) => {
                let channel_id_type = data.channel_id_type;
                let channel_id = data.channel_id;

                let mut data_buf = drdynvc_frame.extra_data;
                if data_buf.len() > data.data_size {
                    let _ = data_buf.split_off(data.data_size);
                }

                // FIXME(perf): copy with data_buf.to_vec()
                if let Some(dvc_data) = self
                    .dynamic_channels
                    .get_mut(&data.channel_id)
                    .ok_or(RdpError::AccessToNonExistingChannel(data.channel_id))?
                    .process_data_first_pdu(data.total_data_size as usize, data_buf.to_vec())?
                {
                    let client_data = dvc::ClientPdu::Data(dvc::DataPdu {
                        channel_id_type,
                        channel_id,
                        data_size: dvc_data.len(),
                    });

                    DynamicVirtualChannelClientFrame {
                        channel_ids: drdynvc_frame.channel_ids,
                        dvc_pdu: client_data,
                        extra_data: Bytes::copy_from_slice(&dvc_data), // FIXME(perf): copy
                    }
                    .encode(&mut buf_writer)?;
                }
            }
            dvc::ServerPdu::Data(data) => {
                let channel_id_type = data.channel_id_type;
                let channel_id = data.channel_id;

                let mut data_buf = drdynvc_frame.extra_data;
                if data_buf.len() > data.data_size {
                    let _ = data_buf.split_off(data.data_size);
                }

                // FIXME(perf): copy with data_buf.to_vec()
                if let Some(dvc_data) = self
                    .dynamic_channels
                    .get_mut(&data.channel_id)
                    .ok_or(RdpError::AccessToNonExistingChannel(data.channel_id))?
                    .process_data_pdu(data_buf.to_vec())?
                {
                    let client_data = dvc::ClientPdu::Data(dvc::DataPdu {
                        channel_id_type,
                        channel_id,
                        data_size: dvc_data.len(),
                    });

                    DynamicVirtualChannelClientFrame {
                        channel_ids: drdynvc_frame.channel_ids,
                        dvc_pdu: client_data,
                        extra_data: Bytes::copy_from_slice(&dvc_data),
                    }
                    .encode(&mut buf_writer)?;
                }
            }
        }

        Ok(buf_writer.into_inner())
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
            Box::new(display::Handler::new()),
            channel_id,
            channel_id_type,
        )),
        _ => {
            error!("Unknown channel name: {}", channel_name);
            None
        }
    }
}

fn negotiate_dvc(
    create_request: &dvc::CreateRequestPdu,
    channel_ids: ChannelIdentificators,
    stream: impl io::Write,
    graphics_config: &Option<GraphicsConfig>,
) -> Result<(), RdpError> {
    if create_request.channel_name == RDP8_GRAPHICS_PIPELINE_NAME {
        let dvc_data = gfx::create_capabilities_advertise(graphics_config)?;
        let client_data = dvc::ClientPdu::Data(dvc::DataPdu {
            channel_id_type: create_request.channel_id_type,
            channel_id: create_request.channel_id,
            data_size: dvc_data.len(),
        });

        debug!("Send GFX Capabilities Advertise PDU");
        DynamicVirtualChannelClientFrame {
            channel_ids,
            dvc_pdu: client_data,
            extra_data: Bytes::copy_from_slice(&dvc_data),
        }
        .encode(stream)?;
    }

    Ok(())
}

trait DynamicChannelDataHandler {
    fn process_complete_data(&mut self, complete_data: Vec<u8>) -> Result<Option<Vec<u8>>, RdpError>;
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

    fn process_data_first_pdu(&mut self, total_data_size: usize, data: Vec<u8>) -> Result<Option<Vec<u8>>, RdpError> {
        if let Some(complete_data) = self.data.process_data_first_pdu(total_data_size, data) {
            self.handler.process_complete_data(complete_data)
        } else {
            Ok(None)
        }
    }

    fn process_data_pdu(&mut self, data: Vec<u8>) -> Result<Option<Vec<u8>>, RdpError> {
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
