use std::io;

use ironrdp::{
    rdp::vc::{self, dvc},
    ShareDataPdu,
};
use log::{debug, warn};

use crate::{
    transport::{
        Decoder, DynamicVirtualChannelTransport, Encoder, SendDataContextTransport,
        ShareControlHeaderTransport, ShareDataHeaderTransport, StaticVirtualChannelTransport,
    },
    utils, RdpError, RdpResult, StaticChannels, GLOBAL_CHANNEL_NAME,
};

const RDP8_GRAPHICS_PIPELINE_NAME: &str = "Graphics";

pub fn process_active_connection_messages(
    mut stream: impl io::BufRead + io::Write,
    static_channels: StaticChannels,
) -> RdpResult<()> {
    let static_channels = utils::swap_hashmap_kv(static_channels);
    let mut transport = SendDataContextTransport::default();

    loop {
        let (channel_ids, pdu) = transport.decode(&mut stream)?;
        let mut transport = transport.clone();
        transport.set_decoded_context(channel_ids, pdu);

        match static_channels.get(&channel_ids.channel_id) {
            Some(channel_name) => {
                if vc::DRDYNVC_CHANNEL_NAME == channel_name {
                    let transport = DynamicVirtualChannelTransport::new(
                        StaticVirtualChannelTransport::new(transport),
                        channel_ids.channel_id,
                    );

                    process_dvc_message(&mut stream, transport)?;
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

    Ok(())
}

fn process_dvc_message(
    mut stream: impl io::BufRead + io::Write,
    mut transport: DynamicVirtualChannelTransport,
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

            let create_response = dvc::ClientPdu::CreateResponse(dvc::CreateResponsePdu {
                channel_id_type: create_request.channel_id_type,
                channel_id: create_request.channel_id,
                creation_status: dvc::DVC_CREATION_STATUS_NO_LISTENER,
            });

            debug!("Send DVC Create Response PDU: {:?}", create_response);
            transport.encode(create_response, &mut stream)?;

            if create_request
                .channel_name
                .contains(RDP8_GRAPHICS_PIPELINE_NAME)
            {
                // TODO: send caps
            }
        }
        dvc::ServerPdu::CloseRequest(close_request) => {
            debug!("Got DVC Close Request PDU: {:?}", close_request);

            let close_response = dvc::ClientPdu::CloseResponse(dvc::ClosePdu {
                channel_id_type: close_request.channel_id_type,
                channel_id: close_request.channel_id,
            });

            debug!("Send DVC Close Response PDU: {:?}", close_response);
            transport.encode(close_response, &mut stream)?;
        }
        dvc::ServerPdu::Data(dvc::DataPdu { dvc_data: data, .. })
        | dvc::ServerPdu::DataFirst(dvc::DataFirstPdu { dvc_data: data, .. }) => {
            debug!("Got DVC Data PDU with {} size", data.len());
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
