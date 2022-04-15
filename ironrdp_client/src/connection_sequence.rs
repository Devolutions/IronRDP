mod user_info;

use std::collections::HashMap;
use std::io::{self};
use std::iter;
use std::net::SocketAddr;

use bufstream::BufStream;
use bytes::BytesMut;
use ironrdp::rdp::capability_sets::CapabilitySet;
use ironrdp::rdp::server_license::{
    ClientNewLicenseRequest, ClientPlatformChallengeResponse, InitialMessageType, InitialServerLicenseMessage,
    ServerPlatformChallenge, ServerUpgradeLicense, PREMASTER_SECRET_SIZE, RANDOM_NUMBER_SIZE,
};
use ironrdp::rdp::{ErrorInfo, ProtocolIndependentCode, ServerSetErrorInfoPdu, SERVER_CHANNEL_ID};
use ironrdp::{nego, rdp, PduParsing};
use log::{debug, info, trace, warn};
use ring::rand::SecureRandom;
use sspi::internal::credssp;

use crate::transport::*;
use crate::{InputConfig, RdpError, BUF_STREAM_SIZE};

pub type StaticChannels = HashMap<String, u16>;

pub struct DesktopSizes {
    pub width: u16,
    pub height: u16,
}

pub struct ConnectionSequenceResult {
    pub desktop_sizes: DesktopSizes,
    pub joined_static_channels: StaticChannels,
    pub global_channel_id: u16,
    pub initiator_id: u16,
}

pub struct UpgradedStream<S>
where
    S: io::Read + io::Write,
{
    pub stream: S,
    pub server_public_key: Vec<u8>,
}

pub fn process_connection_sequence<F, S, US>(
    stream: S,
    routing_addr: &SocketAddr,
    config: &InputConfig,
    upgrade_stream: F,
) -> Result<(ConnectionSequenceResult, BufStream<US>), RdpError>
where
    S: io::Read + io::Write,
    US: io::Read + io::Write,
    F: FnOnce(S) -> Result<UpgradedStream<US>, RdpError>,
{
    let mut stream = BufStream::new(stream);

    let (mut transport, selected_protocol) = connect(
        &mut stream,
        config.security_protocol,
        config.credentials.username.clone(),
    )?;

    let stream = stream.into_inner().map_err(io::Error::from)?;
    let UpgradedStream {
        stream,
        server_public_key,
    } = upgrade_stream(stream)?;
    let mut stream = BufStream::with_capacities(BUF_STREAM_SIZE, BUF_STREAM_SIZE, stream);

    if selected_protocol.contains(nego::SecurityProtocol::HYBRID)
        || selected_protocol.contains(nego::SecurityProtocol::HYBRID_EX)
    {
        process_cred_ssp(&mut stream, config.credentials.clone(), server_public_key)?;

        if selected_protocol.contains(nego::SecurityProtocol::HYBRID_EX) {
            if let credssp::EarlyUserAuthResult::AccessDenied = EarlyUserAuthResult::read(&mut stream)? {
                return Err(RdpError::AccessDenied);
            }
        }
    }

    let static_channels = process_mcs_connect(&mut stream, &mut transport, config, selected_protocol)?;

    let mut transport = McsTransport::new(transport);
    let joined_static_channels = process_mcs(&mut stream, &mut transport, static_channels, config)?;
    debug!("Joined static active_session: {:?}", joined_static_channels);

    let global_channel_id = *joined_static_channels
        .get(config.global_channel_name.as_str())
        .expect("global channel must be added");
    let initiator_id = *joined_static_channels
        .get(config.user_channel_name.as_str())
        .expect("user channel must be added");

    let mut transport = SendDataContextTransport::new(transport, initiator_id, global_channel_id);
    send_client_info(&mut stream, &mut transport, config, routing_addr)?;

    match process_server_license_exchange(&mut stream, &mut transport, config, global_channel_id) {
        Err(RdpError::ServerLicenseError(rdp::RdpError::ServerLicenseError(
            rdp::server_license::ServerLicenseError::UnexpectedValidClientError(_),
        ))) => {
            warn!("The server has returned STATUS_VALID_CLIENT unexpectedly");
        }
        Err(error) => return Err(error),
        Ok(_) => (),
    }

    let mut transport = ShareControlHeaderTransport::new(transport, initiator_id, global_channel_id);
    let desktop_sizes = process_capability_sets(&mut stream, &mut transport, config)?;

    let mut transport = ShareDataHeaderTransport::new(transport);
    process_finalization(&mut stream, &mut transport, initiator_id)?;

    Ok((
        ConnectionSequenceResult {
            desktop_sizes,
            joined_static_channels,
            global_channel_id,
            initiator_id,
        },
        stream,
    ))
}

pub fn process_cred_ssp(
    mut tls_stream: impl io::Read + io::Write,
    credentials: sspi::AuthIdentity,
    server_public_key: Vec<u8>,
) -> Result<(), RdpError> {
    let mut transport = TsRequestTransport::default();

    let mut cred_ssp_client =
        credssp::CredSspClient::new(server_public_key, credentials, credssp::CredSspMode::WithCredentials)
            .map_err(RdpError::CredSspError)?;
    let mut next_ts_request = credssp::TsRequest::default();

    loop {
        let result = cred_ssp_client
            .process(next_ts_request)
            .map_err(RdpError::CredSspError)?;
        debug!("Got CredSSP TSRequest: {:x?}", result);

        match result {
            credssp::ClientState::ReplyNeeded(ts_request) => {
                debug!("Send CredSSP TSRequest: {:x?}", ts_request);
                transport.encode(ts_request, &mut tls_stream)?;
                next_ts_request = transport.decode(&mut tls_stream)?;
            }
            credssp::ClientState::FinalMessage(ts_request) => {
                debug!("Send CredSSP TSRequest: {:x?}", ts_request);
                transport.encode(ts_request, &mut tls_stream)?;
                break;
            }
        }
    }

    Ok(())
}

pub fn process_mcs_connect(
    mut stream: impl io::Read + io::Write,
    transport: &mut DataTransport,
    config: &InputConfig,
    selected_protocol: nego::SecurityProtocol,
) -> Result<StaticChannels, RdpError> {
    let connect_initial =
        ironrdp::ConnectInitial::with_gcc_blocks(user_info::create_gcc_blocks(config, selected_protocol)?);
    debug!("Send MCS Connect Initial PDU: {:?}", connect_initial);
    let mut connect_initial_buf = BytesMut::with_capacity(connect_initial.buffer_length());
    connect_initial_buf.resize(connect_initial.buffer_length(), 0x00);
    connect_initial.to_buffer(connect_initial_buf.as_mut())?;
    transport.encode(connect_initial_buf, &mut stream)?;

    let data_length = transport.decode(&mut stream)?;
    let mut data = BytesMut::with_capacity(data_length);
    data.resize(data_length, 0x00);
    stream.read_exact(&mut data)?;

    let connect_response = ironrdp::ConnectResponse::from_buffer(data.as_ref()).map_err(RdpError::McsConnectError)?;
    debug!("Got MCS Connect Response PDU: {:?}", connect_response);

    let gcc_blocks = connect_response.conference_create_response.gcc_blocks;
    if connect_initial.conference_create_request.gcc_blocks.security == ironrdp::gcc::ClientSecurityData::no_security()
        && gcc_blocks.security != ironrdp::gcc::ServerSecurityData::no_security()
    {
        return Err(RdpError::InvalidResponse(String::from(
            "The server demands a security, while the client requested 'no security'",
        )));
    }

    if gcc_blocks.message_channel.is_some() || gcc_blocks.multi_transport_channel.is_some() {
        return Err(RdpError::InvalidResponse(String::from(
            "The server demands additional active_session",
        )));
    }

    let static_channel_ids = gcc_blocks.network.channel_ids;
    let global_channel_id = gcc_blocks.network.io_channel;

    let static_channels = connect_initial
        .channel_names()
        .unwrap_or_default()
        .into_iter()
        .map(|channel| channel.name)
        .zip(static_channel_ids.into_iter())
        .chain(iter::once((config.global_channel_name.clone(), global_channel_id)))
        .collect::<StaticChannels>();

    Ok(static_channels)
}

pub fn process_mcs(
    mut stream: impl io::Read + io::Write,
    transport: &mut McsTransport,
    mut static_channels: StaticChannels,
    config: &InputConfig,
) -> Result<StaticChannels, RdpError> {
    let erect_domain_request = ironrdp::mcs::ErectDomainPdu {
        sub_height: 0,
        sub_interval: 0,
    };

    debug!("Send MCS Erect Domain Request PDU: {:?}", erect_domain_request);

    transport.encode(
        McsTransport::prepare_data_to_encode(ironrdp::McsPdu::ErectDomainRequest(erect_domain_request), None)?,
        &mut stream,
    )?;

    debug!("Send MCS Attach User Request PDU");
    transport.encode(
        McsTransport::prepare_data_to_encode(ironrdp::McsPdu::AttachUserRequest, None)?,
        &mut stream,
    )?;

    let mcs_pdu = transport.decode(&mut stream)?;
    let initiator_id = if let ironrdp::McsPdu::AttachUserConfirm(attach_user_confirm) = mcs_pdu {
        debug!("Got MCS Attach User Confirm PDU: {:?}", attach_user_confirm);

        static_channels.insert(config.user_channel_name.clone(), attach_user_confirm.initiator_id);

        attach_user_confirm.initiator_id
    } else {
        return Err(RdpError::UnexpectedPdu(format!(
            "Expected MCS Attach User Confirm, got: {:?}",
            mcs_pdu.as_short_name()
        )));
    };

    for (_, id) in static_channels.iter() {
        let channel_join_request = ironrdp::mcs::ChannelJoinRequestPdu {
            initiator_id,
            channel_id: *id,
        };
        debug!("Send MCS Channel Join Request PDU: {:?}", channel_join_request);
        transport.encode(
            McsTransport::prepare_data_to_encode(ironrdp::McsPdu::ChannelJoinRequest(channel_join_request), None)?,
            &mut stream,
        )?;

        let mcs_pdu = transport.decode(&mut stream)?;
        if let ironrdp::McsPdu::ChannelJoinConfirm(channel_join_confirm) = mcs_pdu {
            debug!("Got MCS Channel Join Confirm PDU: {:?}", channel_join_confirm);

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

pub fn send_client_info(
    stream: impl io::Read + io::Write,
    transport: &mut SendDataContextTransport,
    config: &InputConfig,
    routing_addr: &SocketAddr,
) -> Result<(), RdpError> {
    let client_info_pdu = user_info::create_client_info_pdu(config, routing_addr)?;
    debug!("Send Client Info PDU: {:?}", client_info_pdu);
    let mut pdu = Vec::with_capacity(client_info_pdu.buffer_length());
    client_info_pdu
        .to_buffer(&mut pdu)
        .map_err(RdpError::ServerLicenseError)?;
    transport.encode(pdu, stream)?;

    Ok(())
}

pub fn process_server_license_exchange(
    mut stream: impl io::Read + io::Write,
    transport: &mut SendDataContextTransport,
    config: &InputConfig,
    global_channel_id: u16,
) -> Result<(), RdpError> {
    let channel_ids = transport.decode(&mut stream)?;
    check_global_id(channel_ids, global_channel_id)?;

    let initial_license_message = InitialServerLicenseMessage::from_buffer(&mut stream)
        .map_err(|err| RdpError::ServerLicenseError(rdp::RdpError::ServerLicenseError(err)))?;

    debug!("Received Initial License Message PDU");
    trace!("{:?}", initial_license_message);

    let (new_license_request, encryption_data) = match initial_license_message.message_type {
        InitialMessageType::LicenseRequest(license_request) => {
            let mut client_random = vec![0u8; RANDOM_NUMBER_SIZE];

            let rand = ring::rand::SystemRandom::new();
            rand.fill(&mut client_random)
                .map_err(|err| RdpError::IOError(io::Error::new(io::ErrorKind::InvalidData, format!("{}", err))))?;

            let mut premaster_secret = vec![0u8; PREMASTER_SECRET_SIZE];
            rand.fill(&mut premaster_secret)
                .map_err(|err| RdpError::IOError(io::Error::new(io::ErrorKind::InvalidData, format!("{}", err))))?;

            ClientNewLicenseRequest::from_server_license_request(
                &license_request,
                client_random.as_slice(),
                premaster_secret.as_slice(),
                &config.credentials.username,
                config.credentials.domain.as_deref().unwrap_or(""),
            )
            .map_err(|err| {
                RdpError::IOError(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Unable to generate Client New License Request from Server License Request: {}",
                        err
                    ),
                ))
            })?
        }
        InitialMessageType::StatusValidClient(_) => {
            info!("The server has not initiated license exchange");

            return Ok(());
        }
    };

    debug!("Successfully generated Client New License Request");
    trace!("{:?}", new_license_request);
    trace!("{:?}", encryption_data);

    let mut new_pdu_buffer = Vec::with_capacity(new_license_request.buffer_length());
    new_license_request.to_buffer(&mut new_pdu_buffer).map_err(|err| {
        RdpError::IOError(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Unable to write to buffer: {}", err),
        ))
    })?;
    transport.encode(new_pdu_buffer, &mut stream)?;

    let channel_ids = transport.decode(&mut stream)?;
    check_global_id(channel_ids, global_channel_id)?;

    let challenge = ServerPlatformChallenge::from_buffer(&mut stream)
        .map_err(|err| RdpError::ServerLicenseError(rdp::RdpError::ServerLicenseError(err)))?;

    debug!("Received Server Platform Challenge PDU");
    trace!("{:?}", challenge);

    let challenge_response = ClientPlatformChallengeResponse::from_server_platform_challenge(
        &challenge,
        config.credentials.domain.as_deref().unwrap_or(""),
        &encryption_data,
    )
    .map_err(|err| {
        RdpError::ServerLicenseError(rdp::RdpError::IOError(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Unable to generate Client Platform Challenge Response {}", err),
        )))
    })?;

    debug!("Successfully generated Client Platform Challenge Response");
    trace!("{:?}", challenge_response);

    let mut new_pdu_buffer = Vec::with_capacity(challenge_response.buffer_length());
    challenge_response.to_buffer(&mut new_pdu_buffer).map_err(|err| {
        RdpError::IOError(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Unable to write to buffer: {}", err),
        ))
    })?;
    transport.encode(new_pdu_buffer, &mut stream)?;

    let channel_ids = transport.decode(&mut stream)?;
    check_global_id(channel_ids, global_channel_id)?;

    let upgrade_license = ServerUpgradeLicense::from_buffer(&mut stream)
        .map_err(|err| RdpError::ServerLicenseError(rdp::RdpError::ServerLicenseError(err)))?;

    debug!("Received Server Upgrade License PDU");
    trace!("{:?}", upgrade_license);

    upgrade_license.verify_server_license(&encryption_data).map_err(|err| {
        RdpError::ServerLicenseError(rdp::RdpError::IOError(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("License verification failed: {:?}", err),
        )))
    })?;

    debug!("Successfully verified the license");

    Ok(())
}

pub fn process_capability_sets(
    mut stream: impl io::Read + io::Write,
    transport: &mut ShareControlHeaderTransport,
    config: &InputConfig,
) -> Result<DesktopSizes, RdpError> {
    let share_control_pdu = transport.decode(&mut stream)?;
    let capability_sets = if let ironrdp::ShareControlPdu::ServerDemandActive(server_demand_active) = share_control_pdu
    {
        debug!("Got Server Demand Active PDU: {:?}", server_demand_active.pdu);

        server_demand_active.pdu.capability_sets
    } else {
        return Err(RdpError::UnexpectedPdu(format!(
            "Expected Server Demand Active PDU, got: {:?}",
            share_control_pdu.as_short_name()
        )));
    };
    let desktop_sizes = capability_sets
        .iter()
        .find(|c| matches!(c, CapabilitySet::Bitmap(_)))
        .map(|c| match c {
            CapabilitySet::Bitmap(b) => DesktopSizes {
                width: b.desktop_width,
                height: b.desktop_height,
            },
            _ => unreachable!(),
        })
        .unwrap_or(DesktopSizes {
            width: config.width,
            height: config.height,
        });

    let client_confirm_active = ironrdp::ShareControlPdu::ClientConfirmActive(user_info::create_client_confirm_active(
        config,
        capability_sets,
    )?);
    debug!("Send Client Confirm Active PDU: {:?}", client_confirm_active);
    transport.encode(client_confirm_active, &mut stream)?;

    Ok(desktop_sizes)
}

pub fn process_finalization(
    mut stream: impl io::Read + io::Write,
    transport: &mut ShareDataHeaderTransport,
    initiator_id: u16,
) -> Result<(), RdpError> {
    use ironrdp::rdp::{ControlAction, ControlPdu, FontPdu, SequenceFlags, ShareDataPdu, SynchronizePdu};

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
            FinalizationOrder::Synchronize => ShareDataPdu::Synchronize(SynchronizePdu {
                target_user_id: initiator_id,
            }),
            FinalizationOrder::ControlCooperate => ShareDataPdu::Control(ControlPdu {
                action: ControlAction::Cooperate,
                grant_id: 0,
                control_id: 0,
            }),
            FinalizationOrder::RequestControl => ShareDataPdu::Control(ControlPdu {
                action: ControlAction::RequestControl,
                grant_id: 0,
                control_id: 0,
            }),
            FinalizationOrder::Font => ShareDataPdu::FontList(FontPdu {
                number: 0,
                total_number: 0,
                flags: SequenceFlags::FIRST | SequenceFlags::LAST,
                entry_size: 0x0032,
            }),
            FinalizationOrder::Finished => unreachable!(),
        };
        debug!("Send Finalization PDU: {:?}", share_data_pdu);
        transport.encode(share_data_pdu, &mut stream)?;

        let share_data_pdu = transport.decode(&mut stream)?;
        debug!("Got Finalization PDU: {:?}", share_data_pdu);

        finalization_order = match (finalization_order, share_data_pdu) {
            (FinalizationOrder::Synchronize, ShareDataPdu::Synchronize(_)) => FinalizationOrder::ControlCooperate,
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
            ) if grant_id == initiator_id && control_id == u32::from(SERVER_CHANNEL_ID) => FinalizationOrder::Font,
            (FinalizationOrder::Font, ShareDataPdu::FontMap(_)) => FinalizationOrder::Finished,
            (
                order,
                ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(ErrorInfo::ProtocolIndependentCode(
                    ProtocolIndependentCode::None,
                ))),
            ) => order,
            (_, ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(e))) => {
                return Err(RdpError::ServerError(e.description()));
            }
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

fn check_global_id(channel_ids: ChannelIdentificators, id: u16) -> Result<(), RdpError> {
    if channel_ids.channel_id != id {
        Err(RdpError::InvalidResponse(format!(
            "Unexpected Send Data Context channel ID ({})",
            channel_ids.channel_id,
        )))
    } else {
        Ok(())
    }
}
