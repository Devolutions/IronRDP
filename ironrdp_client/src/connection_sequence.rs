mod transport;
mod user_info;

pub use transport::{
    ConnectionConfirm, ConnectionRequest, EarlyUserAuthResult, McsConnectInitial,
    McsConnectResponse, McsTransport, SendDataContextTransport, ShareControlHeaderTransport,
    ShareDataHeaderTransport, TsRequestTransport,
};

use std::{collections::HashMap, io, iter};

use ironrdp::PduParsing;
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

pub fn process_negotiation<S>(
    mut stream: &mut S,
    cookie: String,
    request_protocols: ironrdp::SecurityProtocol,
    request_flags: ironrdp::NegotiationRequestFlags,
) -> RdpResult<ironrdp::SecurityProtocol>
where
    S: io::Read + io::Write,
{
    let connection_request = ConnectionRequest::new(
        Some(ironrdp::NegoData::Cookie(cookie)),
        request_protocols,
        request_flags,
    );
    debug!(
        "Send X.224 Connection Request PDU: {:?}",
        connection_request
    );
    connection_request.write(&mut stream)?;

    let connection_confirm = ConnectionConfirm::read(&mut stream)?;
    debug!("Got X.224 Connection Confirm PDU: {:?}", connection_confirm);
    let selected_protocol = connection_confirm.protocol;

    if request_protocols.contains(selected_protocol) {
        Ok(selected_protocol)
    } else {
        Err(RdpError::InvalidResponse(format!(
            "Got unexpected security protocol: {:?} while was expected one of {:?}",
            selected_protocol, request_protocols
        )))
    }
}

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
    config: &Config,
    selected_protocol: ironrdp::SecurityProtocol,
) -> RdpResult<StaticChannels>
where
    S: io::Read + io::Write,
{
    let connect_initial = McsConnectInitial(ironrdp::ConnectInitial::with_gcc_blocks(
        user_info::create_gcc_blocks(&config, selected_protocol)?,
    ));
    debug!("Send MCS Connect Initial PDU: {:?}", connect_initial.0);
    connect_initial.write(&mut stream)?;

    let connect_response = McsConnectResponse::read(&mut stream)?;
    debug!("Got MCS Connect Response PDU: {:?}", connect_response.0);

    let gcc_blocks = connect_response.0.conference_create_response.gcc_blocks;
    if connect_initial
        .0
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
        .0
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
    mut static_channels: StaticChannels,
) -> RdpResult<StaticChannels>
where
    S: io::Read + io::Write,
{
    let mut transport = McsTransport::default();

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
