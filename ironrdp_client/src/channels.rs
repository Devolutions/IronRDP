use std::io;

use ironrdp::{rdp::vc, PduParsing};
use log::debug;

use crate::{
    transport::{Decoder, DynamicVirtualChannelTransport, Encoder, StaticVirtualChannelTransport},
    RdpResult, StaticChannels,
};

pub fn process_dvc_messages_exchange(
    mut stream: impl io::BufRead + io::Write,
    static_channels: StaticChannels,
) -> RdpResult<()> {
    let drdynvc_id = *static_channels
        .get(vc::DRDYNVC_CHANNEL_NAME)
        .expect("drdynvc channel must be added");

    let mut dvc_messages_exchange = false;
    let mut svc_transport = StaticVirtualChannelTransport::new();
    let mut dynamic_channels = vc::dvc::DynamicChannels::new();

    loop {
        let (channel_id, channel_data) = svc_transport.decode(&mut stream)?;
        if drdynvc_id != channel_id {
            debug!("Got virtual channel PDU on channel with {} id", channel_id);

            if dvc_messages_exchange {
                // during dvc messages exchange server can send some messages through static channels and wait the response
                // so if dvc messages exchange was at least once need to finish connection to avoid client hanging
                break;
            } else {
                continue;
            }
        }

        let mut dvc_transport = DynamicVirtualChannelTransport::new(svc_transport);
        dvc_messages_exchange = true;

        match vc::dvc::ServerPdu::from_buffer(channel_data.as_slice())? {
            vc::dvc::ServerPdu::CapabilitiesRequest(caps_request) => {
                debug!("Got DVC Capabilities Request PDU: {:?}", caps_request);
                let caps_response =
                    vc::dvc::ClientPdu::CapabilitiesResponse(vc::dvc::CapabilitiesResponsePdu {
                        version: vc::dvc::CapsVersion::V1,
                    });

                debug!("Send DVC Capabiities Response PDU: {:?}", caps_response);
                dvc_transport.encode(caps_response, &mut stream)?;
            }
            vc::dvc::ServerPdu::CreateRequest(create_request) => {
                debug!("Got DVC Create Request PDU: {:?}", create_request);
                dynamic_channels.insert(create_request.channel_id, create_request.channel_name);

                let create_response =
                    vc::dvc::ClientPdu::CreateResponse(vc::dvc::CreateResponsePdu {
                        channel_id_type: create_request.channel_id_type,
                        channel_id: create_request.channel_id,
                        creation_status: vc::dvc::DVC_CREATION_STATUS_OK,
                    });

                debug!("Send DVC Create Response PDU: {:?}", create_response);
                dvc_transport.encode(create_response, &mut stream)?;
            }
            vc::dvc::ServerPdu::CloseRequest(close_request) => {
                debug!("Got DVC Close Request PDU: {:?}", close_request);
                let channel_name = dynamic_channels
                    .remove(&close_request.channel_id)
                    .map_or("unknown".to_string(), |channel| channel);

                let close_response = vc::dvc::ClientPdu::CloseResponse(vc::dvc::ClosePdu {
                    channel_id_type: close_request.channel_id_type,
                    channel_id: close_request.channel_id,
                });

                debug!("Send DVC Close Response PDU: {:?}", close_response);
                dvc_transport.encode(close_response, &mut stream)?;

                debug!("DVC {} was closed", channel_name);
            }
            _ => break,
        }
    }

    Ok(())
}
