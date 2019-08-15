mod transport;
mod user_info;

pub use transport::{
    DataTransport, EarlyUserAuthResult, McsTransport, SendDataContextTransport,
    ShareControlHeaderTransport, ShareDataHeaderTransport, TsRequestTransport,
};

use std::{collections::HashMap, io, iter};

use bytes::BytesMut;
use ironrdp::{nego, PduParsing};
use lazy_static::lazy_static;
use log::debug;
use native_tls::TlsStream;
use sspi::CredSsp;

use crate::connection_sequence::transport::{Decoder, Encoder};
use crate::{config::Config, utils, RdpError, RdpResult};

pub type StaticChannels = HashMap<String, u16>;

lazy_static! {
    pub static ref GLOBAL_CHANNEL_NAME: String = String::from("GLOBAL");
    pub static ref USER_CHANNEL_NAME: String = String::from("USER");
}

const SERVER_CHANNEL_ID: u16 = 0x03ea;

pub fn process_cred_ssp<S>(
    mut tls_stream: &mut TlsStream<S>,
    credentials: sspi::Credentials,
) -> RdpResult<()>
where
    S: io::Read + io::Write,
{
    let server_tls_pubkey = utils::get_tls_peer_pubkey(&tls_stream)?;
    let mut transport = TsRequestTransport::default();

    let mut cred_ssp_client = sspi::CredSspClient::with_default_version(
        server_tls_pubkey,
        credentials,
        sspi::CredSspMode::WithCredentials,
    )
    .map_err(RdpError::CredSspError)?;
    let mut next_ts_request = sspi::TsRequest::default();

    loop {
        let result = cred_ssp_client
            .process(next_ts_request.clone())
            .map_err(RdpError::CredSspError)?;
        debug!("Got CredSSP TSRequest: {:x?}", result);

        match result {
            sspi::CredSspResult::ReplyNeeded(ts_request) => {
                debug!("Send CredSSP TSRequest: {:x?}", ts_request);
                transport.encode(ts_request, &mut tls_stream)?;

                next_ts_request = transport.decode(&mut tls_stream)?;
            }
            sspi::CredSspResult::FinalMessage(ts_request) => {
                debug!("Send CredSSP TSRequest: {:x?}", ts_request);
                transport.encode(ts_request, &mut tls_stream)?;

                break;
            }
            _ => unreachable!(),
        }
    }

    Ok(())
}

pub fn process_mcs_connect<S>(
    mut stream: &mut S,
    transport: &mut DataTransport,
    config: &Config,
    selected_protocol: nego::SecurityProtocol,
) -> RdpResult<StaticChannels>
where
    S: io::Read + io::Write,
{
    let connect_initial = ironrdp::ConnectInitial::with_gcc_blocks(user_info::create_gcc_blocks(
        &config,
        selected_protocol,
    )?);
    debug!("Send MCS Connect Initial PDU: {:?}", connect_initial);
    let mut connect_initial_buf = BytesMut::with_capacity(connect_initial.buffer_length());
    connect_initial_buf.resize(connect_initial.buffer_length(), 0x00);
    connect_initial.to_buffer(connect_initial_buf.as_mut())?;
    transport.encode(connect_initial_buf, &mut stream)?;

    let data = transport.decode(&mut stream)?;
    let connect_response =
        ironrdp::ConnectResponse::from_buffer(data.as_ref()).map_err(RdpError::McsConnectError)?;
    debug!("Got MCS Connect Response PDU: {:?}", connect_response);

    let gcc_blocks = connect_response.conference_create_response.gcc_blocks;
    if connect_initial
        .conference_create_request
        .gcc_blocks
        .security
        == ironrdp::gcc::ClientSecurityData::no_security()
        && gcc_blocks.security != ironrdp::gcc::ServerSecurityData::no_security()
    {
        return Err(RdpError::InvalidResponse(String::from(
            "The server demands a security, while the client requested 'no security'",
        )));
    }

    if gcc_blocks.message_channel.is_some() || gcc_blocks.multi_transport_channel.is_some() {
        return Err(RdpError::InvalidResponse(String::from(
            "The server demands additional channels",
        )));
    }

    let static_channel_ids = gcc_blocks.network.channel_ids;
    let global_channel_id = gcc_blocks.network.io_channel;

    let static_channels = connect_initial
        .channel_names()
        .into_iter()
        .map(|channel| channel.name)
        .zip(static_channel_ids.into_iter())
        .chain(iter::once((
            GLOBAL_CHANNEL_NAME.to_string(),
            global_channel_id,
        )))
        .collect::<StaticChannels>();

    Ok(static_channels)
}

pub fn process_mcs<S>(
    mut stream: &mut S,
    transport: &mut McsTransport,
    mut static_channels: StaticChannels,
) -> RdpResult<StaticChannels>
where
    S: io::Read + io::Write,
{
    let erect_domain_request = ironrdp::mcs::ErectDomainPdu::new(0, 0);

    debug!(
        "Send MCS Erect Domain Request PDU: {:?}",
        erect_domain_request
    );
    transport.encode(
        ironrdp::McsPdu::ErectDomainRequest(erect_domain_request),
        &mut stream,
    )?;
    debug!("Send MCS Attach User Request PDU");
    transport.encode(ironrdp::McsPdu::AttachUserRequest, &mut stream)?;

    let mcs_pdu = transport.decode(&mut stream)?;
    let initiator_id = if let ironrdp::McsPdu::AttachUserConfirm(attach_user_confirm) = mcs_pdu {
        debug!("Got MCS Attach User Confirm PDU: {:?}", attach_user_confirm);

        static_channels.insert(
            USER_CHANNEL_NAME.to_string(),
            attach_user_confirm.initiator_id,
        );

        attach_user_confirm.initiator_id
    } else {
        return Err(RdpError::UnexpectedPdu(format!(
            "Expected MCS Attach User Confirm, got: {:?}",
            mcs_pdu.as_short_name()
        )));
    };

    for (_, id) in static_channels.iter() {
        let channel_join_request = ironrdp::mcs::ChannelJoinRequestPdu::new(initiator_id, *id);
        debug!(
            "Send MCS Channel Join Request PDU: {:?}",
            channel_join_request
        );
        transport.encode(
            ironrdp::McsPdu::ChannelJoinRequest(channel_join_request),
            &mut stream,
        )?;

        let mcs_pdu = transport.decode(&mut stream)?;
        if let ironrdp::McsPdu::ChannelJoinConfirm(channel_join_confirm) = mcs_pdu {
            debug!(
                "Got MCS Channel Join Confirm PDU: {:?}",
                channel_join_confirm
            );

            if channel_join_confirm.initiator_id != initiator_id
                || channel_join_confirm.channel_id != channel_join_confirm.requested_channel_id
                || channel_join_confirm.channel_id != *id
            {
                return Err(RdpError::InvalidResponse(String::from(
                    "Invalid MCS Channel Join Confirm",
                )));
            }
        } else {
            return Err(RdpError::UnexpectedPdu(format!(
                "Expected MCS Channel Join Confirm, got: {:?}",
                mcs_pdu.as_short_name()
            )));
        }
    }

    Ok(static_channels)
}

pub fn send_client_info<S>(
    transport: &mut SendDataContextTransport,
    mut stream: &mut S,
    config: &Config,
) -> RdpResult<()>
where
    S: io::Read + io::Write,
{
    let client_info_pdu = user_info::create_client_info_pdu(config)?;
    debug!("Send Client Info PDU: {:?}", client_info_pdu);
    let mut pdu = Vec::with_capacity(client_info_pdu.buffer_length());
    client_info_pdu
        .to_buffer(&mut pdu)
        .map_err(RdpError::ServerLicenseError)?;

    transport.encode(pdu, &mut stream)?;

    Ok(())
}

pub fn process_server_license<S>(
    transport: &mut SendDataContextTransport,
    mut stream: &mut S,
) -> RdpResult<()>
where
    S: io::Read + io::Write,
{
    let pdu = transport.decode(&mut stream)?;
    let server_license_pdu = ironrdp::ServerLicensePdu::from_buffer(pdu.as_slice())
        .map_err(RdpError::ServerLicenseError)?;
    debug!("Got Server License PDU: {:?}", server_license_pdu);

    let server_license = server_license_pdu.server_license;
    if server_license.preamble.message_type == ironrdp::rdp::PreambleType::ErrorAlert
        && server_license.error_message.state_transition
            == ironrdp::rdp::LicensingStateTransition::NoTransition
        && server_license.error_message.error_info.blob_type == ironrdp::rdp::BlobType::Error
        && server_license.error_message.error_info.data.is_empty()
    {
        Ok(())
    } else {
        Err(RdpError::InvalidResponse(String::from(
            "Invalid Server License PDU",
        )))
    }
}

pub fn process_capability_sets<S>(
    transport: &mut ShareControlHeaderTransport,
    mut stream: &mut S,
    config: &Config,
) -> RdpResult<()>
where
    S: io::Read + io::Write,
{
    let share_control_pdu = transport.decode(&mut stream)?;
    let capability_sets =
        if let ironrdp::ShareControlPdu::ServerDemandActive(server_demand_active) =
            share_control_pdu
        {
            debug!(
                "Got Server Demand Active PDU: {:?}",
                server_demand_active.pdu
            );

            server_demand_active.pdu.capability_sets
        } else {
            return Err(RdpError::UnexpectedPdu(format!(
                "Expected Server Demand Active PDU, got: {:?}",
                share_control_pdu.as_short_name()
            )));
        };

    let client_confirm_active = ironrdp::ShareControlPdu::ClientConfirmActive(
        user_info::create_client_confirm_active(config, capability_sets)?,
    );
    debug!(
        "Send Client Confirm Active PDU: {:?}",
        client_confirm_active
    );
    transport.encode(client_confirm_active, &mut stream)?;

    Ok(())
}

pub fn process_finalization<S>(
    transport: &mut ShareDataHeaderTransport,
    mut stream: &mut S,
    initiator_id: u16,
) -> RdpResult<()>
where
    S: io::Read + io::Write,
{
    use ironrdp::rdp::{
        ControlAction, ControlPdu, FontPdu, SequenceFlags, ShareDataPdu, SynchronizePdu,
    };

    #[derive(Copy, Clone, PartialEq, Debug)]
    enum FinalizationOrder {
        Synchronize,
        ControlCooperate,
        RequestControl,
        Font,
        Finished,
    }

    let mut finalization_order = FinalizationOrder::Synchronize;
    while finalization_order != FinalizationOrder::Finished {
        let share_data_pdu = match finalization_order {
            FinalizationOrder::Synchronize => {
                ShareDataPdu::Synchronize(SynchronizePdu::new(initiator_id))
            }
            FinalizationOrder::ControlCooperate => {
                ShareDataPdu::Control(ControlPdu::new(ControlAction::Cooperate, 0, 0))
            }
            FinalizationOrder::RequestControl => {
                ShareDataPdu::Control(ControlPdu::new(ControlAction::RequestControl, 0, 0))
            }
            FinalizationOrder::Font => ShareDataPdu::FontList(FontPdu::new(
                0,
                0,
                SequenceFlags::FIRST | SequenceFlags::LAST,
                0x0032,
            )),
            FinalizationOrder::Finished => unreachable!(),
        };
        debug!("Send Finalization PDU: {:?}", share_data_pdu);
        transport.encode(share_data_pdu, &mut stream)?;

        let share_data_pdu = transport.decode(&mut stream)?;
        debug!("Got Finalization PDU: {:?}", share_data_pdu);

        finalization_order = match (finalization_order, share_data_pdu) {
            (FinalizationOrder::Synchronize, ShareDataPdu::Synchronize(_)) => {
                FinalizationOrder::ControlCooperate
            }
            (
                FinalizationOrder::ControlCooperate,
                ShareDataPdu::Control(ControlPdu {
                    action: ironrdp::ControlAction::Cooperate,
                    grant_id: 0,
                    control_id: 0,
                }),
            ) => FinalizationOrder::RequestControl,
            (
                FinalizationOrder::RequestControl,
                ShareDataPdu::Control(ControlPdu {
                    action: ironrdp::ControlAction::GrantedControl,
                    grant_id,
                    control_id,
                }),
            ) if grant_id == initiator_id && control_id == u32::from(SERVER_CHANNEL_ID) => {
                FinalizationOrder::Font
            }
            (FinalizationOrder::Font, ShareDataPdu::FontMap(_)) => FinalizationOrder::Finished,
            (order, pdu) => {
                return Err(RdpError::UnexpectedPdu(format!(
                    "Expected Server {:?} PDU, got invalid PDU: {:?}",
                    order,
                    pdu.as_short_name()
                )));
            }
        };
    }

    Ok(())
}
