mod user_info;

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::iter;
use std::net::SocketAddr;

use dns_lookup::lookup_addr;
use futures_util::AsyncRead;
use futures_util::AsyncReadExt as _;
use futures_util::AsyncWrite;
use ironrdp::rdp::capability_sets::CapabilitySet;
use ironrdp::rdp::server_license::{
    ClientNewLicenseRequest, ClientPlatformChallengeResponse, InitialMessageType, InitialServerLicenseMessage,
    ServerPlatformChallenge, ServerUpgradeLicense, PREMASTER_SECRET_SIZE, RANDOM_NUMBER_SIZE,
};
use ironrdp::rdp::{ErrorInfo, ProtocolIndependentCode, ServerSetErrorInfoPdu, SERVER_CHANNEL_ID};
use ironrdp::{nego, rdp, PduParsing};
use ring::rand::SecureRandom as _;
use sspi::internal::credssp;
use sspi::NegotiateConfig;

use crate::codecs::encode_next_frame;
use crate::codecs::ErasedWriter;
use crate::codecs::FramedReader;
use crate::transport::ChannelIdentificators;
use crate::transport::SendPduDataContextTransport;
use crate::transport::ShareControlHeaderTransport;
use crate::transport::TsRequestTransport;
use crate::transport::{
    connect, DataTransport, EarlyUserAuthResult, McsTransport, SendDataContextTransport, ShareDataHeaderTransport,
    X224DataTransport,
};
use crate::{InputConfig, RdpError};

pub type StaticChannels = HashMap<String, u16>;

pub struct DesktopSize {
    pub width: u16,
    pub height: u16,
}

pub struct ConnectionSequenceResult {
    pub desktop_size: DesktopSize,
    pub joined_static_channels: StaticChannels,
    pub global_channel_id: u16,
    pub initiator_id: u16,
}

pub struct UpgradedStream<S> {
    pub stream: S,
    pub server_public_key: Vec<u8>,
}

pub async fn process_connection_sequence<S, UpgradeFn, FnRes, UpgradedS>(
    stream: S,
    routing_addr: &SocketAddr,
    config: &InputConfig,
    upgrade_stream: UpgradeFn,
) -> Result<(ConnectionSequenceResult, FramedReader, ErasedWriter), RdpError>
where
    S: AsyncRead + AsyncWrite + Unpin + 'static,
    UpgradeFn: FnOnce(S) -> FnRes,
    FnRes: Future<Output = Result<UpgradedStream<UpgradedS>, RdpError>>,
    UpgradedS: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (reader, mut writer) = stream.split();

    let mut reader = FramedReader::new(reader);

    let selected_protocol = connect(
        &mut reader,
        &mut writer,
        config.security_protocol,
        config.credentials.username.clone(),
    )
    .await?;

    let (reader, leftover) = reader.into_inner();

    let stream = reader.reunite(writer).unwrap();

    debug_assert_eq!(leftover.len(), 0, "no leftover is expected after initial negotiation");

    let UpgradedStream {
        mut stream,
        server_public_key,
    } = upgrade_stream(stream).await?;

    if selected_protocol.contains(nego::SecurityProtocol::HYBRID)
        || selected_protocol.contains(nego::SecurityProtocol::HYBRID_EX)
    {
        process_cred_ssp(&mut stream, config.credentials.clone(), server_public_key, routing_addr).await?;

        if selected_protocol.contains(nego::SecurityProtocol::HYBRID_EX) {
            let data = EarlyUserAuthResult::read(&mut stream).await?;
            if let credssp::EarlyUserAuthResult::AccessDenied = data {
                return Err(RdpError::AccessDenied);
            }
        }
    }

    let (reader, writer) = stream.split();
    let mut reader = FramedReader::new(reader).into_erased();
    let mut writer = Box::pin(writer) as ErasedWriter;

    let static_channels = process_mcs_connect(&mut reader, &mut writer, config, selected_protocol).await?;
    let joined_static_channels = process_mcs(&mut reader, &mut writer, static_channels, config).await?;
    debug!("Joined static active_session: {:?}", joined_static_channels);

    let global_channel_id = *joined_static_channels
        .get(config.global_channel_name.as_str())
        .expect("global channel must be added");
    let initiator_id = *joined_static_channels
        .get(config.user_channel_name.as_str())
        .expect("user channel must be added");

    let transport =
        SendDataContextTransport::new(McsTransport::new(DataTransport::new()), initiator_id, global_channel_id);
    send_client_info(&mut writer, transport, config, routing_addr).await?;

    process_server_license_exchange(&mut reader, &mut writer, config, global_channel_id).await?;

    let transport =
        SendDataContextTransport::new(McsTransport::new(DataTransport::new()), initiator_id, global_channel_id);
    let transport = ShareControlHeaderTransport::new(transport, initiator_id, global_channel_id);
    let desktop_size = process_capability_sets(&mut reader, &mut writer, transport, config).await?;

    let transport =
        SendDataContextTransport::new(McsTransport::new(DataTransport::new()), initiator_id, global_channel_id);
    let transport = ShareControlHeaderTransport::new(transport, initiator_id, global_channel_id);
    let transport = ShareDataHeaderTransport::new(transport);
    process_finalization(&mut reader, &mut writer, transport, initiator_id).await?;

    Ok((
        ConnectionSequenceResult {
            desktop_size,
            joined_static_channels,
            global_channel_id,
            initiator_id,
        },
        reader,
        writer,
    ))
}

pub async fn process_cred_ssp(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    credentials: sspi::AuthIdentity,
    server_public_key: Vec<u8>,
    routing_addr: &SocketAddr,
) -> Result<(), RdpError> {
    let mut transport = TsRequestTransport::default();

    let destination_host = lookup_addr(&routing_addr.ip())
        .map_err(|err| RdpError::UserInfoError(format!("unable to query destination host name: {:?}", err)))?;
    let service_principal_name = format!("TERMSRV/{}", destination_host);

    let mut cred_ssp_client = credssp::CredSspClient::new(
        server_public_key,
        credentials,
        credssp::CredSspMode::WithCredentials,
        credssp::ClientMode::Negotiate(NegotiateConfig::default()),
        service_principal_name,
    )
    .map_err(RdpError::CredSspError)?;
    let mut next_ts_request = credssp::TsRequest::default();

    loop {
        let result = cred_ssp_client
            .process(next_ts_request)
            .map_err(RdpError::CredSspError)?;
        debug!("Got CredSSP TSRequest: {:x?}", result);

        match result {
            credssp::ClientState::ReplyNeeded(ts_request) => {
                debug!("Send CredSSP TSRequest (reply needed): {:x?}", ts_request);
                transport.encode(ts_request, &mut stream).await?;
                next_ts_request = transport.decode(&mut stream).await?;
            }
            credssp::ClientState::FinalMessage(ts_request) => {
                debug!("Send CredSSP TSRequest (final): {:x?}", ts_request);
                transport.encode(ts_request, &mut stream).await?;
                break;
            }
        }
    }

    Ok(())
}

pub async fn process_mcs_connect(
    reader: &mut FramedReader,
    writer: &mut ErasedWriter,
    config: &InputConfig,
    selected_protocol: nego::SecurityProtocol,
) -> Result<StaticChannels, RdpError> {
    let connect_initial =
        ironrdp::ConnectInitial::with_gcc_blocks(user_info::create_gcc_blocks(config, selected_protocol)?);
    debug!("Send MCS Connect Initial PDU: {:?}", connect_initial);
    let mut codec = X224DataTransport::default();
    encode_next_frame(writer, &mut codec, connect_initial.clone()).await?;
    let connect_response: ironrdp::ConnectResponse = reader.decode_next_frame(&mut codec).await?;
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

pub async fn process_mcs(
    stream: &mut FramedReader,
    writer: &mut ErasedWriter,
    mut static_channels: StaticChannels,
    config: &InputConfig,
) -> Result<StaticChannels, RdpError> {
    let erect_domain_request = ironrdp::mcs::ErectDomainPdu {
        sub_height: 0,
        sub_interval: 0,
    };

    let mut codec = X224DataTransport::default();

    debug!("Send MCS Erect Domain Request PDU: {:?}", erect_domain_request);
    encode_next_frame(
        writer,
        &mut codec,
        ironrdp::McsPdu::ErectDomainRequest(erect_domain_request),
    )
    .await?;

    debug!("Send MCS Attach User Request PDU");
    encode_next_frame(writer, &mut codec, ironrdp::McsPdu::AttachUserRequest).await?;

    let mcs_pdu = stream.decode_next_frame(&mut codec).await?;
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
        encode_next_frame(
            writer,
            &mut codec,
            ironrdp::McsPdu::ChannelJoinRequest(channel_join_request),
        )
        .await?;

        let mcs_pdu = stream.decode_next_frame(&mut codec).await?;
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

pub async fn send_client_info(
    writer: &mut ErasedWriter,
    mut codec: SendDataContextTransport,
    config: &InputConfig,
    routing_addr: &SocketAddr,
) -> Result<(), RdpError> {
    let client_info_pdu = user_info::create_client_info_pdu(config, routing_addr)?;
    debug!("Send Client Info PDU: {:?}", client_info_pdu);
    let mut pdu = Vec::with_capacity(client_info_pdu.buffer_length());
    client_info_pdu
        .to_buffer(&mut pdu)
        .map_err(RdpError::ServerLicenseError)?;
    encode_next_frame(writer, &mut codec, pdu).await?;
    Ok(())
}

pub async fn process_server_license_exchange(
    reader: &mut FramedReader,
    writer: &mut ErasedWriter,
    config: &InputConfig,
    global_channel_id: u16,
) -> Result<(), RdpError> {
    let mut codec = SendPduDataContextTransport::<ClientNewLicenseRequest, InitialServerLicenseMessage>::default();
    let (channel_ids, initial_license_message) = reader.decode_next_frame(&mut codec).await?;

    check_global_id(channel_ids, global_channel_id)?;

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

    encode_next_frame(writer, &mut codec, new_license_request).await?;

    let mut codec = SendPduDataContextTransport::<ClientPlatformChallengeResponse, ServerPlatformChallenge>::default();
    let (channel_ids, challenge) = reader.decode_next_frame(&mut codec).await?;
    check_global_id(channel_ids, global_channel_id)?;

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
    encode_next_frame(writer, &mut codec, challenge_response).await?;

    let mut codec = SendPduDataContextTransport::<ServerUpgradeLicense>::default();
    let (channel_ids, upgrade_license) = match reader.decode_next_frame(&mut codec).await {
        Err(RdpError::ServerLicenseError(rdp::RdpError::ServerLicenseError(
            rdp::server_license::ServerLicenseError::UnexpectedValidClientError(_),
        ))) => {
            warn!("The server has returned STATUS_VALID_CLIENT unexpectedly");
            return Ok(());
        }
        Ok(data) => data,
        Err(err) => {
            return Err(err);
        }
    };
    check_global_id(channel_ids, global_channel_id)?;

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

pub async fn process_capability_sets(
    reader: &mut FramedReader,
    writer: &mut ErasedWriter,
    mut codec: ShareControlHeaderTransport,
    config: &InputConfig,
) -> Result<DesktopSize, RdpError> {
    let share_control_pdu = reader.decode_next_frame(&mut codec).await?;
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
    let desktop_size = capability_sets
        .iter()
        .find(|c| matches!(c, CapabilitySet::Bitmap(_)))
        .map(|c| match c {
            CapabilitySet::Bitmap(b) => DesktopSize {
                width: b.desktop_width,
                height: b.desktop_height,
            },
            _ => unreachable!(),
        })
        .unwrap_or(DesktopSize {
            width: config.width,
            height: config.height,
        });

    let client_confirm_active = ironrdp::ShareControlPdu::ClientConfirmActive(user_info::create_client_confirm_active(
        config,
        capability_sets,
    )?);
    debug!("Send Client Confirm Active PDU: {:?}", client_confirm_active);
    encode_next_frame(writer, &mut codec, client_confirm_active).await?;
    Ok(desktop_size)
}

pub async fn process_finalization(
    reader: &mut FramedReader,
    writer: &mut ErasedWriter,
    mut codec: ShareDataHeaderTransport,
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
        encode_next_frame(writer, &mut codec, share_data_pdu).await?;
        let share_data_pdu = reader.decode_next_frame(&mut codec).await?;
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
