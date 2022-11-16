mod display;
mod gfx;

use std::collections::HashMap;
use std::{cmp, io};

use ironrdp_core::dvc::FieldType;
use ironrdp_core::rdp::vc::{self, dvc};
use ironrdp_core::rdp::{ErrorInfo, ProtocolIndependentCode, ServerSetErrorInfoPdu};
use ironrdp_core::{Data, ShareDataPdu};

pub use self::gfx::GfxHandler;
use crate::transport::{
    Decoder, DynamicVirtualChannelTransport, Encoder, SendDataContextTransport, ShareControlHeaderTransport,
    ShareDataHeaderTransport, StaticVirtualChannelTransport,
};
use crate::{GraphicsConfig, RdpError};

pub const RDP8_GRAPHICS_PIPELINE_NAME: &str = "Microsoft::Windows::RDS::Graphics";
pub const RDP8_DISPLAY_PIPELINE_NAME: &str = "Microsoft::Windows::RDS::DisplayControl";

pub struct Processor {
    static_channels: HashMap<u16, String>,
    channel_map: HashMap<String, u32>,
    dynamic_channels: HashMap<u32, DynamicChannel>,
    global_channel_name: String,
    drdynvc_transport: Option<DynamicVirtualChannelTransport>,
    static_transport: Option<ShareDataHeaderTransport>,
    graphics_config: Option<GraphicsConfig>,
    graphics_handler: Option<Box<dyn GfxHandler + Send>>,
}

impl Processor {
    pub fn new(
        static_channels: HashMap<u16, String>,
        global_channel_name: String,
        graphics_config: Option<GraphicsConfig>,
        graphics_handler: Option<Box<dyn GfxHandler + Send>>,
    ) -> Self {
        Self {
            static_channels,
            dynamic_channels: HashMap::new(),
            channel_map: HashMap::new(),
            global_channel_name,
            drdynvc_transport: None,
            static_transport: None,
            graphics_config,
            graphics_handler,
        }
    }

    pub fn process(
        &mut self,
        mut stream: impl io::Read,
        mut output: impl io::Write,
        data: Data,
    ) -> Result<(), RdpError> {
        let mut transport = SendDataContextTransport::default();
        transport.mcs_transport.0.set_decoded_context(data.data_length);

        let channel_ids = transport.decode(&mut stream)?;
        transport.set_decoded_context(channel_ids);

        let channel_id = channel_ids.channel_id;
        let initiator_id = channel_ids.initiator_id;
        match self.static_channels.get(&channel_id).map(String::as_str) {
            Some(vc::DRDYNVC_CHANNEL_NAME) => self.process_dvc_message(&mut stream, &mut output, transport, channel_id),
            Some(name) if name == self.global_channel_name => {
                if self.static_transport.is_none() {
                    self.static_transport = Some(ShareDataHeaderTransport::new(ShareControlHeaderTransport::new(
                        transport,
                        initiator_id,
                        channel_id,
                    )));
                }
                let transport = self.static_transport.as_mut().unwrap();

                process_global_channel_pdu(&mut stream, transport)
            }
            Some(_) => Err(RdpError::UnexpectedChannel(channel_id)),
            None => panic!("Channel with {} ID must be added", channel_id),
        }
    }

    /// Sends a PDU on the dynamic channel. The upper layers are responsible for encoding the PDU and converting them to message
    #[allow(dead_code)]
    pub fn send_dynamic(
        &mut self,
        mut stream: impl io::Write,
        channel_name: &str,
        message: Vec<u8>,
    ) -> Result<(), RdpError> {
        if let Some(transport) = self.drdynvc_transport.as_mut() {
            let channel_id = self
                .channel_map
                .get(channel_name)
                .ok_or_else(|| RdpError::AccessToNonExistingChannelName(channel_name.to_string()))?;
            let channel = self
                .dynamic_channels
                .get_mut(channel_id)
                .ok_or(RdpError::AccessToNonExistingChannel(*channel_id))?;
            let client_data = dvc::ClientPdu::Data(dvc::DataPdu {
                channel_id_type: channel.channel_id_type,
                channel_id: channel.channel_id,
                data_size: message.len(),
            });

            transport.encode(
                DynamicVirtualChannelTransport::prepare_data_to_encode(client_data, Some(message))?,
                &mut stream,
            )?;
        } else {
            return Err(RdpError::DynamicVirtualChannelNotConnected);
        };

        Ok(())
    }

    /// Send a pdu on the static global channel. Typically used to send input events
    #[allow(dead_code)]
    pub fn send_static(&mut self, mut stream: impl io::Write, message: ShareDataPdu) -> Result<(), RdpError> {
        if let Some(transport) = self.static_transport.as_mut() {
            transport.encode(message, &mut stream)?;
        } else {
            return Err(RdpError::StaticChannelNotConnected);
        };

        Ok(())
    }

    fn process_dvc_message(
        &mut self,
        mut stream: impl io::Read,
        mut output: impl io::Write,
        transport: SendDataContextTransport,
        channel_id: u16,
    ) -> Result<(), RdpError> {
        if self.drdynvc_transport.is_none() {
            self.drdynvc_transport = Some(DynamicVirtualChannelTransport::new(
                StaticVirtualChannelTransport::new(transport),
                channel_id,
            ));
        }

        let transport = self.drdynvc_transport.as_mut().unwrap();

        match transport.decode(&mut stream)? {
            dvc::ServerPdu::CapabilitiesRequest(caps_request) => {
                debug!("Got DVC Capabilities Request PDU: {:?}", caps_request);
                let caps_response = dvc::ClientPdu::CapabilitiesResponse(dvc::CapabilitiesResponsePdu {
                    version: dvc::CapsVersion::V1,
                });

                debug!("Send DVC Capabilities Response PDU: {:?}", caps_response);
                transport.encode(
                    DynamicVirtualChannelTransport::prepare_data_to_encode(caps_response, None)?,
                    &mut output,
                )?;
            }
            dvc::ServerPdu::CreateRequest(create_request) => {
                debug!("Got DVC Create Request PDU: {:?}", create_request);

                let creation_status = if let Some(dyncamic_channel) = create_dvc(
                    create_request.channel_name.as_str(),
                    create_request.channel_id,
                    create_request.channel_id_type,
                    &mut self.graphics_handler,
                ) {
                    self.dynamic_channels
                        .insert(create_request.channel_id, dyncamic_channel);
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
                transport.encode(
                    DynamicVirtualChannelTransport::prepare_data_to_encode(create_response, None)?,
                    &mut output,
                )?;

                negotiate_dvc(&create_request, transport, &mut output, &self.graphics_config)?;
            }
            dvc::ServerPdu::CloseRequest(close_request) => {
                debug!("Got DVC Close Request PDU: {:?}", close_request);

                let close_response = dvc::ClientPdu::CloseResponse(dvc::ClosePdu {
                    channel_id_type: close_request.channel_id_type,
                    channel_id: close_request.channel_id,
                });

                debug!("Send DVC Close Response PDU: {:?}", close_response);
                transport.encode(
                    DynamicVirtualChannelTransport::prepare_data_to_encode(close_response, None)?,
                    &mut output,
                )?;

                self.dynamic_channels.remove(&close_request.channel_id);
            }
            dvc::ServerPdu::DataFirst(data) => {
                let channel_id_type = data.channel_id_type;
                let channel_id = data.channel_id;
                let mut data_buff = vec![0; data.data_size];
                stream.read_exact(&mut data_buff)?;

                if let Some(dvc_data) = self
                    .dynamic_channels
                    .get_mut(&data.channel_id)
                    .ok_or(RdpError::AccessToNonExistingChannel(data.channel_id))?
                    .process_data_first_pdu(data.total_data_size as usize, data_buff)?
                {
                    let client_data = dvc::ClientPdu::Data(dvc::DataPdu {
                        channel_id_type,
                        channel_id,
                        data_size: dvc_data.len(),
                    });

                    transport.encode(
                        DynamicVirtualChannelTransport::prepare_data_to_encode(client_data, Some(dvc_data))?,
                        &mut output,
                    )?;
                }
            }
            dvc::ServerPdu::Data(data) => {
                let channel_id_type = data.channel_id_type;
                let channel_id = data.channel_id;
                let mut data_buff = vec![0; data.data_size];
                stream.read_exact(&mut data_buff)?;

                if let Some(dvc_data) = self
                    .dynamic_channels
                    .get_mut(&data.channel_id)
                    .ok_or(RdpError::AccessToNonExistingChannel(data.channel_id))?
                    .process_data_pdu(data_buff)?
                {
                    let client_data = dvc::ClientPdu::Data(dvc::DataPdu {
                        channel_id_type,
                        channel_id,
                        data_size: dvc_data.len(),
                    });

                    transport.encode(
                        DynamicVirtualChannelTransport::prepare_data_to_encode(client_data, Some(dvc_data))?,
                        &mut output,
                    )?;
                }
            }
        }

        Ok(())
    }
}

fn process_global_channel_pdu(
    mut stream: impl io::Read,
    transport: &mut ShareDataHeaderTransport,
) -> Result<(), RdpError> {
    let share_data_pdu = transport.decode(&mut stream)?;

    match share_data_pdu {
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
        ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(e)) => Err(RdpError::ServerError(e.description())),
        _ => Err(RdpError::UnexpectedPdu(format!(
            "Expected Session Save Info PDU, got: {:?}",
            share_data_pdu.as_short_name()
        ))),
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
    transport: &mut DynamicVirtualChannelTransport,
    mut stream: impl io::Write,
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
        transport.encode(
            DynamicVirtualChannelTransport::prepare_data_to_encode(client_data, Some(dvc_data))?,
            &mut stream,
        )?;
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
