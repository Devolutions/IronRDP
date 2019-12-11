use std::io;

use ironrdp::{nego::NegotiationError, rdp::vc};
use log::{debug, warn};

use crate::{
    transport::{Decoder, DynamicVirtualChannelTransport, Encoder, StaticVirtualChannelTransport},
    RdpError, RdpResult, StaticChannels,
};

const RDP8_GRAPHICS_PIPELINE_NAME: &str = "Graphics";

pub fn process_dvc_messages_exchange(
    mut stream: impl io::BufRead + io::Write,
    static_channels: StaticChannels,
) -> RdpResult<()> {
    let drdynvc_id = *static_channels
        .get(vc::DRDYNVC_CHANNEL_NAME)
        .expect("drdynvc channel must be added");

    let mut dvc_transport =
        DynamicVirtualChannelTransport::new(StaticVirtualChannelTransport::new(), drdynvc_id);

    loop {
        match dvc_transport.decode(&mut stream) {
            Ok(request) => match request {
                vc::dvc::ServerPdu::CapabilitiesRequest(caps_request) => {
                    debug!("Got DVC Capabilities Request PDU: {:?}", caps_request);
                    let caps_response = vc::dvc::ClientPdu::CapabilitiesResponse(
                        vc::dvc::CapabilitiesResponsePdu {
                            version: vc::dvc::CapsVersion::V1,
                        },
                    );

                    debug!("Send DVC Capabiities Response PDU: {:?}", caps_response);
                    dvc_transport.encode(caps_response, &mut stream)?;
                }
                vc::dvc::ServerPdu::CreateRequest(create_request) => {
                    debug!("Got DVC Create Request PDU: {:?}", create_request);

                    let create_response =
                        vc::dvc::ClientPdu::CreateResponse(vc::dvc::CreateResponsePdu {
                            channel_id_type: create_request.channel_id_type,
                            channel_id: create_request.channel_id,
                            creation_status: vc::dvc::DVC_CREATION_STATUS_OK,
                        });

                    debug!("Send DVC Create Response PDU: {:?}", create_response);
                    dvc_transport.encode(create_response, &mut stream)?;

                    let close_request = vc::dvc::ClientPdu::CloseResponse(vc::dvc::ClosePdu {
                        channel_id_type: create_request.channel_id_type,
                        channel_id: create_request.channel_id,
                    });

                    debug!("Send DVC Close Request PDU: {:?}", close_request);
                    dvc_transport.encode(close_request, &mut stream)?;

                    if create_request
                        .channel_name
                        .contains(RDP8_GRAPHICS_PIPELINE_NAME)
                    {
                        break;
                    }
                }
                vc::dvc::ServerPdu::CloseRequest(close_request) => {
                    debug!("Got DVC Close Request PDU: {:?}", close_request);

                    let close_response = vc::dvc::ClientPdu::CloseResponse(vc::dvc::ClosePdu {
                        channel_id_type: close_request.channel_id_type,
                        channel_id: close_request.channel_id,
                    });

                    debug!("Send DVC Close Response PDU: {:?}", close_response);
                    dvc_transport.encode(close_response, &mut stream)?;
                }
                vc::dvc::ServerPdu::Data(_) | vc::dvc::ServerPdu::DataFirst(_) => break,
            },
            Err(RdpError::InvalidChannelIdError(err_message)) => {
                warn!("{}", err_message);
                break;
            }
            Err(RdpError::NegotiationError(error)) => match error {
                NegotiationError::TpktVersionError => {
                    warn!("Got fast-path message");
                    break;
                }
                error => {
                    return Err(RdpError::NegotiationError(error));
                }
            },
            Err(err) => {
                return Err(err);
            }
        }
    }

    Ok(())
}
