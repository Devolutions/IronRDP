mod user_info;

use std::collections::HashMap;
use std::future::Future;
use std::io::{self};
use std::iter;
use std::net::SocketAddr;

use dns_lookup::lookup_addr;
use futures::{SinkExt, StreamExt};
use ironrdp::rdp::capability_sets::CapabilitySet;
use ironrdp::rdp::server_license::{
    ClientNewLicenseRequest, ClientPlatformChallengeResponse, InitialMessageType, InitialServerLicenseMessage,
    ServerPlatformChallenge, ServerUpgradeLicense, PREMASTER_SECRET_SIZE, RANDOM_NUMBER_SIZE,
};
use ironrdp::rdp::{ErrorInfo, ProtocolIndependentCode, ServerSetErrorInfoPdu, SERVER_CHANNEL_ID};
use ironrdp::{nego, rdp, ConnectInitial, ConnectResponse, McsPdu, PduParsing};
use log::{debug, info, trace, warn};
use ring::rand::SecureRandom;
use sspi::internal::credssp;
use sspi::NegotiateConfig;
use tokio::net::TcpStream;
use tokio_util::codec::Framed;

use crate::codecs::{RdpFrameCodec, TrasnportCodec};
use crate::{transport::*, TlsStreamType};
use crate::{InputConfig, RdpError};

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

pub struct UpgradedStream {
    pub stream: TlsStreamType,
    pub server_public_key: Vec<u8>,
}

pub async fn process_connection_sequence<'a, F, FnRes>(
    mut stream: TcpStream,
    routing_addr: &SocketAddr,
    config: &InputConfig,
    upgrade_stream: F,
) -> Result<(ConnectionSequenceResult, Framed<TlsStreamType, RdpFrameCodec>), RdpError>
where
    F: FnOnce(TcpStream) -> FnRes,
    FnRes: Future<Output = Result<UpgradedStream, RdpError>>,
{
    let selected_protocol = connect(
        &mut stream,
        config.security_protocol,
        config.credentials.username.clone(),
    )
    .await?;

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

    let stream = Framed::new(stream, RdpFrameCodec::default());

    let (static_channels, stream) = process_mcs_connect(stream, config, selected_protocol).await?;

    let (joined_static_channels, stream) = process_mcs(stream, static_channels, config).await?;
    debug!("Joined static active_session: {:?}", joined_static_channels);

    let global_channel_id = *joined_static_channels
        .get(config.global_channel_name.as_str())
        .expect("global channel must be added");
    let initiator_id = *joined_static_channels
        .get(config.user_channel_name.as_str())
        .expect("user channel must be added");

    let transport =
        SendDataContextTransport::new(McsTransport::new(DataTransport::new()), initiator_id, global_channel_id);
    let stream = send_client_info(stream, transport, config, routing_addr).await?;

    let stream = process_server_license_exchange(stream, config, global_channel_id).await?;

    let transport =
        SendDataContextTransport::new(McsTransport::new(DataTransport::new()), initiator_id, global_channel_id);
    let transport = ShareControlHeaderTransport::new(transport, initiator_id, global_channel_id);
    let (desktop_sizes, stream) = process_capability_sets(stream, transport, config).await?;

    let transport =
        SendDataContextTransport::new(McsTransport::new(DataTransport::new()), initiator_id, global_channel_id);
    let transport = ShareControlHeaderTransport::new(transport, initiator_id, global_channel_id);
    let transport = ShareDataHeaderTransport::new(transport);
    let stream = process_finalization(stream, transport, initiator_id).await?;

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

pub async fn process_cred_ssp(
    mut tls_stream: &mut TlsStreamType,
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
                transport.encode(ts_request, &mut tls_stream).await?;
                next_ts_request = transport.decode(&mut tls_stream).await?;
            }
            credssp::ClientState::FinalMessage(ts_request) => {
                debug!("Send CredSSP TSRequest (final): {:x?}", ts_request);
                transport.encode(ts_request, &mut tls_stream).await?;
                break;
            }
        }
    }

    Ok(())
}

pub async fn process_mcs_connect<C>(
    stream: Framed<TlsStreamType, C>,
    config: &InputConfig,
    selected_protocol: nego::SecurityProtocol,
) -> Result<
    (
        StaticChannels,
        Framed<TlsStreamType, TrasnportCodec<X224DataTransport<ConnectInitial, ConnectResponse>>>,
    ),
    RdpError,
> {
    let connect_initial =
        ironrdp::ConnectInitial::with_gcc_blocks(user_info::create_gcc_blocks(config, selected_protocol)?);
    debug!("Send MCS Connect Initial PDU: {:?}", connect_initial);
    let mut stream = stream.map_codec(|_| TrasnportCodec::new(X224DataTransport::default()));
    stream.send(connect_initial.clone()).await?;
    let connect_response: ironrdp::ConnectResponse =
        stream.next().await.ok_or(RdpError::UnexpectedStreamTermination)??;
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

    Ok((static_channels, stream))
}

pub async fn process_mcs<C>(
    stream: Framed<TlsStreamType, C>,
    mut static_channels: StaticChannels,
    config: &InputConfig,
) -> Result<
    (
        StaticChannels,
        Framed<TlsStreamType, TrasnportCodec<X224DataTransport<McsPdu>>>,
    ),
    RdpError,
> {
    let erect_domain_request = ironrdp::mcs::ErectDomainPdu {
        sub_height: 0,
        sub_interval: 0,
    };

    let mut stream = stream.map_codec(|_| TrasnportCodec::new(X224DataTransport::default()));
    debug!("Send MCS Erect Domain Request PDU: {:?}", erect_domain_request);
    stream
        .send(ironrdp::McsPdu::ErectDomainRequest(erect_domain_request))
        .await?;

    debug!("Send MCS Attach User Request PDU");

    stream.send(ironrdp::McsPdu::AttachUserRequest).await?;

    let mcs_pdu = stream.next().await.ok_or(RdpError::UnexpectedStreamTermination)??;
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

        stream
            .send(ironrdp::McsPdu::ChannelJoinRequest(channel_join_request))
            .await?;

        let mcs_pdu = stream.next().await.ok_or(RdpError::UnexpectedStreamTermination)??;
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

    Ok((static_channels, stream))
}

pub async fn send_client_info<C>(
    stream: Framed<TlsStreamType, C>,
    transport: SendDataContextTransport,
    config: &InputConfig,
    routing_addr: &SocketAddr,
) -> Result<Framed<TlsStreamType, TrasnportCodec<SendDataContextTransport>>, RdpError> {
    let mut stream = stream.map_codec(|_| TrasnportCodec::new(transport));

    let client_info_pdu = user_info::create_client_info_pdu(config, routing_addr)?;
    debug!("Send Client Info PDU: {:?}", client_info_pdu);
    let mut pdu = Vec::with_capacity(client_info_pdu.buffer_length());
    client_info_pdu
        .to_buffer(&mut pdu)
        .map_err(RdpError::ServerLicenseError)?;
    stream.send(pdu).await?;
    Ok(stream)
}

pub async fn process_server_license_exchange<C>(
    stream: Framed<TlsStreamType, C>,
    config: &InputConfig,
    global_channel_id: u16,
) -> Result<Framed<TlsStreamType, RdpFrameCodec>, RdpError> {
    let transp = SendPduDataContextTransport::<ClientNewLicenseRequest, InitialServerLicenseMessage>::default();
    let codec = TrasnportCodec::new(transp);
    let mut stream = stream.map_codec(|_| codec);
    let (channel_ids, initial_license_message) = stream.next().await.ok_or(RdpError::UnexpectedStreamTermination)??;

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

            return Ok(stream.map_codec(|_| RdpFrameCodec::default()));
        }
    };

    debug!("Successfully generated Client New License Request");
    trace!("{:?}", new_license_request);
    trace!("{:?}", encryption_data);

    stream.send(new_license_request).await?;

    let transp = SendPduDataContextTransport::<ClientPlatformChallengeResponse, ServerPlatformChallenge>::default();
    let codec = TrasnportCodec::new(transp);
    let mut stream = stream.map_codec(|_| codec);
    let (channel_ids, challenge) = stream.next().await.ok_or(RdpError::UnexpectedStreamTermination)??;
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
    stream.send(challenge_response).await?;

    let transp = SendPduDataContextTransport::<ServerUpgradeLicense>::default();
    let codec = TrasnportCodec::new(transp);
    let mut stream = stream.map_codec(|_| codec);
    let (channel_ids, upgrade_license) = match stream.next().await.ok_or(RdpError::UnexpectedStreamTermination)? {
        Err(RdpError::ServerLicenseError(rdp::RdpError::ServerLicenseError(
            rdp::server_license::ServerLicenseError::UnexpectedValidClientError(_),
        ))) => {
            warn!("The server has returned STATUS_VALID_CLIENT unexpectedly");
            return Ok(stream.map_codec(|_| RdpFrameCodec::default()));
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

    Ok(stream.map_codec(|_| RdpFrameCodec::default()))
}

pub async fn process_capability_sets<C>(
    stream: Framed<TlsStreamType, C>,
    transport: ShareControlHeaderTransport,
    config: &InputConfig,
) -> Result<(DesktopSizes, Framed<TlsStreamType, RdpFrameCodec>), RdpError> {
    let mut stream = stream.map_codec(|_| TrasnportCodec::new(transport));
    let share_control_pdu = stream.next().await.ok_or(RdpError::UnexpectedStreamTermination)??;
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
    stream.send(client_confirm_active).await?;
    Ok((desktop_sizes, stream.map_codec(|_| RdpFrameCodec::default())))
}

pub async fn process_finalization<C>(
    stream: Framed<TlsStreamType, C>,
    transport: ShareDataHeaderTransport,
    initiator_id: u16,
) -> Result<Framed<TlsStreamType, RdpFrameCodec>, RdpError> {
    use ironrdp::rdp::{ControlAction, ControlPdu, FontPdu, SequenceFlags, ShareDataPdu, SynchronizePdu};

    #[derive(Copy, Clone, PartialEq, Debug)]
    enum FinalizationOrder {
        Synchronize,
        ControlCooperate,
        RequestControl,
        Font,
        Finished,
    }

    let mut stream = stream.map_codec(|_| TrasnportCodec::new(transport));
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
        stream.send(share_data_pdu).await?;
        let share_data_pdu = stream.next().await.ok_or(RdpError::UnexpectedStreamTermination)??;
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

    let mapped = stream.map_codec(|_| RdpFrameCodec::default());
    Ok(mapped)
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
