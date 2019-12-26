mod gfx;

use std::{cmp, collections::HashMap, io};

use ironrdp::{
    rdp::vc::{self, dvc},
    ShareDataPdu,
};
use log::{debug, error, warn};

use crate::{
    transport::{
        Decoder, DynamicVirtualChannelTransport, Encoder, SendDataContextTransport,
        ShareControlHeaderTransport, ShareDataHeaderTransport, StaticVirtualChannelTransport,
    },
    utils, RdpError, RdpResult, StaticChannels, GLOBAL_CHANNEL_NAME,
};

const RDP8_GRAPHICS_PIPELINE_NAME: &str = "Microsoft::Windows::RDS::Graphics";

pub fn process_active_connection_messages(
    mut stream: impl io::BufRead + io::Write,
    static_channels: StaticChannels,
) -> RdpResult<()> {
    let static_channels = utils::swap_hashmap_kv(static_channels);
    let mut dynamic_channels: HashMap<u32, DynamicChannel> = HashMap::new();
    let mut transport = SendDataContextTransport::default();

    loop {
        match transport.decode(&mut stream) {
            Ok((channel_ids, pdu)) => {
                let mut transport = transport.clone();
                transport.set_decoded_context(channel_ids, pdu);

                match static_channels.get(&channel_ids.channel_id) {
                    Some(channel_name) => {
                        if vc::DRDYNVC_CHANNEL_NAME == channel_name {
                            let transport = DynamicVirtualChannelTransport::new(
                                StaticVirtualChannelTransport::new(transport),
                                channel_ids.channel_id,
                            );

                            process_dvc_message(&mut stream, transport, &mut dynamic_channels)?;
                        } else if GLOBAL_CHANNEL_NAME == channel_name {
                            let transport =
                                ShareDataHeaderTransport::new(ShareControlHeaderTransport::new(
                                    transport,
                                    channel_ids.initiator_id,
                                    channel_ids.channel_id,
                                ));

                            process_session_info(&mut stream, transport)?;
                        } else {
                            warn!(
                                "Got message on a channel with {} ID",
                                channel_ids.channel_id
                            );
                            break;
                        }
                    }
                    None => panic!("Channel with {} ID must be added", channel_ids.channel_id),
                }
            }
            Err(error) => match error {
                RdpError::UnexpectedDisconnection(message) => {
                    warn!("User-Initiated disconnection on Server: {}", message);
                    break;
                }
                err => {
                    return Err(err);
                }
            },
        }
    }

    Ok(())
}

fn process_dvc_message(
    mut stream: impl io::BufRead + io::Write,
    mut transport: DynamicVirtualChannelTransport,
    dynamic_channels: &mut HashMap<u32, DynamicChannel>,
) -> RdpResult<()> {
    match transport.decode(&mut stream)? {
        dvc::ServerPdu::CapabilitiesRequest(caps_request) => {
            debug!("Got DVC Capabilities Request PDU: {:?}", caps_request);
            let caps_response =
                dvc::ClientPdu::CapabilitiesResponse(dvc::CapabilitiesResponsePdu {
                    version: dvc::CapsVersion::V1,
                });

            debug!("Send DVC Capabiities Response PDU: {:?}", caps_response);
            transport.encode(caps_response, &mut stream)?;
        }
        dvc::ServerPdu::CreateRequest(create_request) => {
            debug!("Got DVC Create Request PDU: {:?}", create_request);

            let creation_status =
                if let Some(dyncamic_channel) = create_dvc(create_request.channel_name.as_str()) {
                    dynamic_channels.insert(create_request.channel_id, dyncamic_channel);

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
            transport.encode(create_response, &mut stream)?;

            negotiate_dvc(&create_request, transport, &mut stream)?;
        }
        dvc::ServerPdu::CloseRequest(close_request) => {
            debug!("Got DVC Close Request PDU: {:?}", close_request);

            let close_response = dvc::ClientPdu::CloseResponse(dvc::ClosePdu {
                channel_id_type: close_request.channel_id_type,
                channel_id: close_request.channel_id,
            });

            debug!("Send DVC Close Response PDU: {:?}", close_response);
            transport.encode(close_response, &mut stream)?;

            dynamic_channels.remove(&close_request.channel_id);
        }
        dvc::ServerPdu::DataFirst(data) => {
            let channel_id_type = data.channel_id_type;
            let channel_id = data.channel_id;
            if let Some(dvc_data) = dynamic_channels
                .get_mut(&data.channel_id)
                .ok_or_else(|| RdpError::AccessToNonExistingChannel(data.channel_id))?
                .process_data_first_pdu(data)?
            {
                let client_data = dvc::ClientPdu::Data(dvc::DataPdu {
                    channel_id_type,
                    channel_id,
                    dvc_data,
                });

                transport.encode(client_data, &mut stream)?;
            }
        }
        dvc::ServerPdu::Data(data) => {
            let channel_id_type = data.channel_id_type;
            let channel_id = data.channel_id;
            if let Some(dvc_data) = dynamic_channels
                .get_mut(&data.channel_id)
                .ok_or_else(|| RdpError::AccessToNonExistingChannel(data.channel_id))?
                .process_data_pdu(data)?
            {
                let client_data = dvc::ClientPdu::Data(dvc::DataPdu {
                    channel_id_type,
                    channel_id,
                    dvc_data,
                });

                transport.encode(client_data, &mut stream)?;
            }
        }
    }

    Ok(())
}

fn process_session_info(
    mut stream: impl io::BufRead + io::Write,
    mut transport: ShareDataHeaderTransport,
) -> RdpResult<()> {
    let share_data_pdu = transport.decode(&mut stream)?;

    if let ShareDataPdu::SaveSessionInfo(session_info) = share_data_pdu {
        debug!("Got Session Save Info PDU: {:?}", session_info);
        Ok(())
    } else {
        Err(RdpError::UnexpectedPdu(format!(
            "Expected Session Save Info PDU, got: {:?}",
            share_data_pdu.as_short_name()
        )))
    }
}

fn create_dvc(channel_name: &str) -> Option<DynamicChannel> {
    match channel_name {
        RDP8_GRAPHICS_PIPELINE_NAME => Some(DynamicChannel::new(Box::new(gfx::Handler::new()))),
        _ => None,
    }
}

fn negotiate_dvc(
    create_request: &dvc::CreateRequestPdu,
    mut transport: DynamicVirtualChannelTransport,
    mut stream: impl io::Write,
) -> Result<(), RdpError> {
    if create_request.channel_name == RDP8_GRAPHICS_PIPELINE_NAME {
        let client_data = dvc::ClientPdu::Data(dvc::DataPdu {
            channel_id_type: create_request.channel_id_type,
            channel_id: create_request.channel_id,
            dvc_data: gfx::create_capabilities_advertise()?,
        });
        debug!("Send GFX Capabilities Advertise PDU");
        transport.encode(client_data, &mut stream)?;
    }

    Ok(())
}

trait DynamicChannelDataHandler {
    fn process_complete_data(
        &mut self,
        complete_data: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, RdpError>;
}

struct DynamicChannel {
    data: CompleteData,
    handler: Box<dyn DynamicChannelDataHandler>,
}

impl DynamicChannel {
    pub fn new(handler: Box<dyn DynamicChannelDataHandler>) -> Self {
        Self {
            data: CompleteData::new(),
            handler,
        }
    }

    pub fn process_data_first_pdu(
        &mut self,
        data_first: dvc::DataFirstPdu,
    ) -> Result<Option<Vec<u8>>, RdpError> {
        if let Some(complete_data) = self.data.process_data_first_pdu(data_first) {
            self.handler.process_complete_data(complete_data)
        } else {
            Ok(None)
        }
    }

    pub fn process_data_pdu(&mut self, data: dvc::DataPdu) -> Result<Option<Vec<u8>>, RdpError> {
        if let Some(complete_data) = self.data.process_data_pdu(data) {
            self.handler.process_complete_data(complete_data)
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug, PartialEq)]
struct CompleteData {
    total_length: u32,
    data: Vec<u8>,
}

impl CompleteData {
    fn new() -> Self {
        Self {
            total_length: 0,
            data: Vec::new(),
        }
    }

    fn process_data_first_pdu(&mut self, data_first: dvc::DataFirstPdu) -> Option<Vec<u8>> {
        if self.total_length != 0 || !self.data.is_empty() {
            error!("Incomplete DVC message, it will be skipped");
            self.data.clear();
        }
        self.total_length = data_first.data_length;
        self.data = data_first.dvc_data;

        None
    }

    fn process_data_pdu(&mut self, mut data: dvc::DataPdu) -> Option<Vec<u8>> {
        if self.total_length == 0 && self.data.is_empty() {
            // message is not fragmented
            Some(data.dvc_data)
        } else {
            // message is fragmented so need to reassemble it
            let actual_data_length = self.data.len() + data.dvc_data.len();

            match actual_data_length.cmp(&(self.total_length as usize)) {
                cmp::Ordering::Less => {
                    // this is one of the fragmented messages, just append it
                    self.data.append(&mut data.dvc_data);
                    None
                }
                cmp::Ordering::Equal => {
                    // this is the last fragmented message, need to return the whole reassembled message
                    self.total_length = 0;
                    self.data.append(&mut data.dvc_data);
                    Some(self.data.drain(..).collect())
                }
                cmp::Ordering::Greater => {
                    error!(
                        "Actual DVC message size is grater than expected total DVC message size"
                    );
                    self.total_length = 0;
                    self.data.clear();

                    None
                }
            }
        }
    }
}
