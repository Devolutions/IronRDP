mod user_info;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::{io, iter};

use bytes::{BufMut as _, Bytes, BytesMut};
use futures_util::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use ironrdp_pdu::rdp::capability_sets::CapabilitySet;
use ironrdp_pdu::rdp::server_license::{
    ClientNewLicenseRequest, ClientPlatformChallengeResponse, InitialMessageType, InitialServerLicenseMessage,
    ServerPlatformChallenge, ServerUpgradeLicense, PREMASTER_SECRET_SIZE, RANDOM_NUMBER_SIZE,
};
use ironrdp_pdu::rdp::{ErrorInfo, ProtocolIndependentCode, ServerSetErrorInfoPdu, SERVER_CHANNEL_ID};
use ironrdp_pdu::{rdp, PduParsing};
use rand_core::{OsRng, RngCore};
use sspi::network_client::NetworkClientFactory;
use sspi::{credssp, NegotiateConfig};

use crate::frame::{SendDataInfoFrame, SendPduDataFrame, ShareControlFrame, ShareDataFrame, X224Frame};
use crate::framed::{encode_next_frame, ErasedWriter, FramedReader};
use crate::{ChannelIdentificators, InputConfig, RdpError};

pub type StaticChannels = HashMap<String, u16>;

#[derive(Clone)]
pub struct DesktopSize {
    pub width: u16,
    pub height: u16,
}

#[derive(Clone)]
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

pub struct Address {
    pub hostname: String,
    pub sock: SocketAddr,
}

impl Address {
    // TODO: use a custom type holding a hostname and a port explicitely similar to a (String, u16) tuple
    pub fn lookup_addr(addr: &str) -> io::Result<Self> {
        use std::net::ToSocketAddrs as _;
        let sockaddr = addr.to_socket_addrs()?.next().unwrap();
        let port_segment_idx = addr.rfind(':').unwrap(); // there must be a port
        let hostname = addr[..port_segment_idx].to_owned();
        Ok(Self {
            hostname,
            sock: sockaddr,
        })
    }
}

// TODO: convert to a state machine and wrap into a future in ironrdp-session-async
// The ultimate goal is to remove the `dgw_ext` feature flag.

#[cfg(not(feature = "dgw_ext"))]
pub async fn process_connection_sequence<S, UpgradeFn, FnRes, UpgradedS>(
    stream: S,
    addr: &Address,
    config: &InputConfig,
    upgrade_stream: UpgradeFn,
    network_client_factory: Box<dyn NetworkClientFactory>,
) -> Result<(ConnectionSequenceResult, FramedReader, ErasedWriter), RdpError>
where
    S: AsyncRead + AsyncWrite + Unpin + 'static,
    UpgradeFn: FnOnce(S) -> FnRes,
    FnRes: std::future::Future<Output = Result<UpgradedStream<UpgradedS>, RdpError>>,
    UpgradedS: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (reader, mut writer) = stream.split();
    let mut reader = FramedReader::new(reader);

    //== Connection Initiation ==//
    // Exchange supported security protocols and a few other connection flags.

    trace!("Connection Initiation");

    let selected_protocol = connection_initiation(
        &mut reader,
        &mut writer,
        config.security_protocol,
        config.credentials.username.clone(),
    )
    .await?;

    info!("Selected security protocol: {selected_protocol:?}");

    //== Upgrade to Enhanced RDP Security ==//
    // NOTE: we assume the selected protocol is never the standard RDP security (RC4).

    let reader = reader.into_inner_no_leftover();
    let stream = reader.reunite(writer).unwrap();

    let UpgradedStream {
        mut stream,
        server_public_key,
    } = upgrade_stream(stream).await?;

    if selected_protocol.contains(ironrdp_pdu::SecurityProtocol::HYBRID)
        || selected_protocol.contains(ironrdp_pdu::SecurityProtocol::HYBRID_EX)
    {
        process_cred_ssp(
            &mut stream,
            config.credentials.clone(),
            server_public_key,
            addr.hostname.clone(),
            network_client_factory,
        )
        .await?;

        if selected_protocol.contains(ironrdp_pdu::SecurityProtocol::HYBRID_EX) {
            let data = read_early_user_auth_result(&mut stream).await?;
            if let credssp::EarlyUserAuthResult::AccessDenied = data {
                return Err(RdpError::AccessDenied);
            }
        }
    }

    let (reader, writer) = stream.split();
    let mut reader = FramedReader::new(reader).into_erased();
    let mut writer = Box::pin(writer) as ErasedWriter;

    //== Basic Settings Exchange ==//
    // Exchange basic settings including Core Data, Security Data and Network Data.

    trace!("Basic Settings Exchange");

    let static_channels = basic_settings_exchange(&mut reader, &mut writer, config, selected_protocol).await?;

    //== Channel Connection ==//
    // Connect every individual channel.

    trace!("Channel Connection");

    let joined_static_channels = channel_connection(&mut reader, &mut writer, static_channels, config).await?;
    debug!("Joined static active_session: {:?}", joined_static_channels);

    let global_channel_id = *joined_static_channels
        .get(config.global_channel_name.as_str())
        .expect("global channel must be added");
    let initiator_id = *joined_static_channels
        .get(config.user_channel_name.as_str())
        .expect("user channel must be added");

    //== RDP Security Commencement ==//
    // When using standard RDP security (RC4), a Security Exchange PDU is sent at this point.
    // NOTE: IronRDP is only supporting extended security (TLS…).

    if selected_protocol == ironrdp_pdu::SecurityProtocol::RDP {
        return Err(RdpError::Connection(io::Error::new(
            io::ErrorKind::Other,
            "Standard RDP Security (RC4 encryption) is not supported",
        )));
    }

    //== Secure Settings Exchange ==//
    // Send Client Info PDU (information about supported types of compression, username, password, etc).

    trace!("Secure Settings Exchange");

    let global_channel_ids = ChannelIdentificators {
        initiator_id,
        channel_id: global_channel_id,
    };
    settings_exchange(&mut writer, global_channel_ids, config, &addr.sock).await?;

    //== Optional Connect-Time Auto-Detection ==//
    // NOTE: IronRDP is not expecting the Auto-Detect Request PDU from server.

    //== Licensing ==//
    // Server is sending information regarding licensing.
    // Typically useful when support for more than two simultaneous connections is required (terminal server).

    trace!("Licensing");

    server_licensing_exchange(&mut reader, &mut writer, config, global_channel_id).await?;

    //== Optional Multitransport Bootstrapping ==//
    // NOTE: our implemention is not expecting the Auto-Detect Request PDU from server

    //== Capabilities Exchange ==/
    // The server sends the set of capabilities it supports to the client.

    trace!("Capabilities Exchange");

    let desktop_size = capabilities_exchange(&mut reader, &mut writer, global_channel_ids, config).await?;

    //== Connection Finalization ==//
    // Client and server exchange a few PDUs in order to finalize the connection.
    // Client may send PDUs one after the other without waiting for a response in order to speed up the process.

    trace!("Connection finalization");

    connection_finalization(&mut reader, &mut writer, global_channel_ids).await?;

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

#[cfg(feature = "dgw_ext")]
pub async fn process_connection_sequence<S>(
    stream: S,
    target_hostname: String,
    proxy_auth_token: String,
    config: &InputConfig,
    network_client_factory: Box<dyn NetworkClientFactory>,
) -> Result<(ConnectionSequenceResult, FramedReader, ErasedWriter), RdpError>
where
    S: AsyncRead + AsyncWrite + Unpin + 'static,
{
    let (reader, mut writer) = stream.split();
    let mut reader = FramedReader::new(reader);

    let (selected_protocol, server_public_key, server_addr) = connection_initiation(
        &mut reader,
        &mut writer,
        proxy_auth_token,
        config.security_protocol,
        config.credentials.username.clone(),
        target_hostname.clone(),
    )
    .await?;

    let (reader, leftover) = reader.into_inner();
    debug_assert_eq!(leftover.len(), 0, "no leftover is expected after connection initiation");

    let mut stream = reader.reunite(writer).unwrap();

    if selected_protocol.contains(ironrdp_pdu::SecurityProtocol::HYBRID)
        || selected_protocol.contains(ironrdp_pdu::SecurityProtocol::HYBRID_EX)
    {
        process_cred_ssp(
            &mut stream,
            config.credentials.clone(),
            server_public_key,
            target_hostname,
            network_client_factory,
        )
        .await?;

        if selected_protocol.contains(ironrdp_pdu::SecurityProtocol::HYBRID_EX) {
            let data = read_early_user_auth_result(&mut stream).await?;
            if let credssp::EarlyUserAuthResult::AccessDenied = data {
                return Err(RdpError::AccessDenied);
            }
        }
    }

    let (reader, writer) = stream.split();
    let mut reader = FramedReader::new(reader).into_erased();
    let mut writer = Box::pin(writer) as ErasedWriter;

    //== Basic Settings Exchange ==//
    // Exchange basic settings including Core Data, Security Data and Network Data.

    trace!("Basic Settings Exchange");

    let static_channels = basic_settings_exchange(&mut reader, &mut writer, config, selected_protocol).await?;

    //== Channel Connection ==//
    // Connect every individual channel.

    trace!("Channel Connection");

    let joined_static_channels = channel_connection(&mut reader, &mut writer, static_channels, config).await?;
    debug!("Joined static active_session: {:?}", joined_static_channels);

    let global_channel_id = *joined_static_channels
        .get(config.global_channel_name.as_str())
        .expect("global channel must be added");
    let initiator_id = *joined_static_channels
        .get(config.user_channel_name.as_str())
        .expect("user channel must be added");

    //== RDP Security Commencement ==//
    // When using standard RDP security (RC4), a Security Exchange PDU is sent at this point.
    // NOTE: IronRDP is only supporting extended security (TLS…).

    if selected_protocol == ironrdp_pdu::SecurityProtocol::RDP {
        return Err(RdpError::Connection(io::Error::new(
            io::ErrorKind::Other,
            "Standard RDP Security (RC4 encryption) is not supported",
        )));
    }

    //== Secure Settings Exchange ==//
    // Send Client Info PDU (information about supported types of compression, username, password, etc).

    trace!("Secure Settings Exchange");

    let global_channel_ids = ChannelIdentificators {
        initiator_id,
        channel_id: global_channel_id,
    };
    settings_exchange(&mut writer, global_channel_ids, config, &server_addr).await?;

    //== Optional Connect-Time Auto-Detection ==//
    // NOTE: IronRDP is not expecting the Auto-Detect Request PDU from server.

    //== Licensing ==//
    // Server is sending information regarding licensing.
    // Typically useful when support for more than two simultaneous connections is required (terminal server).

    trace!("Licensing");

    server_licensing_exchange(&mut reader, &mut writer, config, global_channel_id).await?;

    //== Optional Multitransport Bootstrapping ==//
    // NOTE: our implemention is not expecting the Auto-Detect Request PDU from server

    //== Capabilities Exchange ==/
    // The server sends the set of capabilities it supports to the client.

    trace!("Capabilities Exchange");

    let desktop_size = capabilities_exchange(&mut reader, &mut writer, global_channel_ids, config).await?;

    //== Connection Finalization ==//
    // Client and server exchange a few PDUs in order to finalize the connection.
    // Client may send PDUs one after the other without waiting for a response in order to speed up the process.

    trace!("Connection finalization");

    connection_finalization(&mut reader, &mut writer, global_channel_ids).await?;

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
    hostname: String,
    network_client_factory: Box<dyn NetworkClientFactory>,
) -> Result<(), RdpError> {
    use sspi::ntlm::NtlmConfig;

    let service_principal_name = format!("TERMSRV/{hostname}");

    let mut cred_ssp_client = credssp::CredSspClient::new(
        server_public_key,
        credentials,
        credssp::CredSspMode::WithCredentials,
        credssp::ClientMode::Negotiate(NegotiateConfig {
            protocol_config: Box::<NtlmConfig>::default(),
            package_list: None,
            hostname,
            network_client_factory,
        }),
        service_principal_name,
    )
    .map_err(RdpError::CredSsp)?;

    let mut next_ts_request = credssp::TsRequest::default();

    loop {
        let result = cred_ssp_client.process(next_ts_request).map_err(RdpError::CredSsp)?;
        debug!("Got CredSSP TSRequest: {:x?}", result);

        match result {
            credssp::ClientState::ReplyNeeded(ts_request) => {
                debug!("Send CredSSP TSRequest (reply needed): {:x?}", ts_request);
                write_credssp_ts_request(ts_request, &mut stream).await?;
                next_ts_request = read_credssp_ts_request(&mut stream).await?;
            }
            credssp::ClientState::FinalMessage(ts_request) => {
                debug!("Send CredSSP TSRequest (final): {:x?}", ts_request);
                write_credssp_ts_request(ts_request, &mut stream).await?;
                break;
            }
        }
    }

    Ok(())
}

pub async fn basic_settings_exchange(
    reader: &mut FramedReader,
    writer: &mut ErasedWriter,
    config: &InputConfig,
    selected_protocol: ironrdp_pdu::SecurityProtocol,
) -> Result<StaticChannels, RdpError> {
    let connect_initial =
        ironrdp_pdu::ConnectInitial::with_gcc_blocks(user_info::create_gcc_blocks(config, selected_protocol)?);
    debug!("Send MCS Connect Initial PDU: {:?}", connect_initial);
    encode_next_frame(writer, X224Frame(&connect_initial)).await?;
    let connect_response: ironrdp_pdu::ConnectResponse = reader.decode_next_frame::<X224Frame<_>>().await?.0;
    debug!("Got MCS Connect Response PDU: {:?}", connect_response);

    let gcc_blocks = connect_response.conference_create_response.gcc_blocks;
    if connect_initial.conference_create_request.gcc_blocks.security
        == ironrdp_pdu::gcc::ClientSecurityData::no_security()
        && gcc_blocks.security != ironrdp_pdu::gcc::ServerSecurityData::no_security()
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

pub async fn channel_connection(
    stream: &mut FramedReader,
    writer: &mut ErasedWriter,
    mut static_channels: StaticChannels,
    config: &InputConfig,
) -> Result<StaticChannels, RdpError> {
    let erect_domain_request = ironrdp_pdu::mcs::ErectDomainPdu {
        sub_height: 0,
        sub_interval: 0,
    };

    debug!("Send MCS Erect Domain Request PDU: {:?}", erect_domain_request);
    encode_next_frame(
        writer,
        X224Frame(ironrdp_pdu::McsPdu::ErectDomainRequest(erect_domain_request)),
    )
    .await?;

    debug!("Send MCS Attach User Request PDU");
    encode_next_frame(writer, X224Frame(ironrdp_pdu::McsPdu::AttachUserRequest)).await?;

    let mcs_pdu = stream.decode_next_frame::<X224Frame<_>>().await?.0;
    let initiator_id = if let ironrdp_pdu::McsPdu::AttachUserConfirm(attach_user_confirm) = mcs_pdu {
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
        let channel_join_request = ironrdp_pdu::mcs::ChannelJoinRequestPdu {
            initiator_id,
            channel_id: *id,
        };
        debug!("Send MCS Channel Join Request PDU: {:?}", channel_join_request);
        encode_next_frame(
            writer,
            X224Frame(ironrdp_pdu::McsPdu::ChannelJoinRequest(channel_join_request)),
        )
        .await?;

        let mcs_pdu = stream.decode_next_frame::<X224Frame<_>>().await?.0;
        if let ironrdp_pdu::McsPdu::ChannelJoinConfirm(channel_join_confirm) = mcs_pdu {
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

pub async fn settings_exchange(
    writer: &mut ErasedWriter,
    channel_ids: ChannelIdentificators,
    config: &InputConfig,
    routing_addr: &SocketAddr,
) -> Result<(), RdpError> {
    let client_info_pdu = user_info::create_client_info_pdu(config, routing_addr)?;
    debug!("Send Client Info PDU: {:?}", client_info_pdu);

    let mut buf_writer = BytesMut::with_capacity(client_info_pdu.buffer_length()).writer();
    client_info_pdu
        .to_buffer(&mut buf_writer)
        .map_err(RdpError::ServerLicense)?;
    let buf = buf_writer.into_inner();

    encode_next_frame(
        writer,
        SendDataInfoFrame {
            channel_ids,
            data: buf.freeze(),
        },
    )
    .await?;

    Ok(())
}

pub async fn server_licensing_exchange(
    reader: &mut FramedReader,
    writer: &mut ErasedWriter,
    config: &InputConfig,
    global_channel_id: u16,
) -> Result<(), RdpError> {
    let initial_server_license_frame = reader
        .decode_next_frame::<SendPduDataFrame<InitialServerLicenseMessage>>()
        .await?;
    let channel_ids = initial_server_license_frame.channel_ids;
    let initial_license_message = initial_server_license_frame.pdu;

    check_global_id(channel_ids, global_channel_id)?;

    debug!("Received Initial License Message PDU");
    trace!("{:?}", initial_license_message);

    let (new_license_request, encryption_data) = match initial_license_message.message_type {
        InitialMessageType::LicenseRequest(license_request) => {
            let mut client_random = vec![0u8; RANDOM_NUMBER_SIZE];
            OsRng.fill_bytes(&mut client_random);

            let mut premaster_secret = vec![0u8; PREMASTER_SECRET_SIZE];
            OsRng.fill_bytes(&mut premaster_secret);

            ClientNewLicenseRequest::from_server_license_request(
                &license_request,
                client_random.as_slice(),
                premaster_secret.as_slice(),
                &config.credentials.username,
                config.credentials.domain.as_deref().unwrap_or(""),
            )
            .map_err(|err| {
                RdpError::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Unable to generate Client New License Request from Server License Request: {err}"),
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

    encode_next_frame(
        writer,
        SendPduDataFrame {
            channel_ids,
            pdu: new_license_request,
        },
    )
    .await?;

    let server_platform_challenge_frame = reader
        .decode_next_frame::<SendPduDataFrame<ServerPlatformChallenge>>()
        .await?;
    let channel_ids = server_platform_challenge_frame.channel_ids;
    let challenge = server_platform_challenge_frame.pdu;
    check_global_id(channel_ids, global_channel_id)?;

    let challenge_response = ClientPlatformChallengeResponse::from_server_platform_challenge(
        &challenge,
        config.credentials.domain.as_deref().unwrap_or(""),
        &encryption_data,
    )
    .map_err(|err| {
        RdpError::ServerLicense(rdp::RdpError::IOError(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Unable to generate Client Platform Challenge Response {err}"),
        )))
    })?;

    debug!("Successfully generated Client Platform Challenge Response");
    trace!("{:?}", challenge_response);
    encode_next_frame(
        writer,
        SendPduDataFrame {
            channel_ids,
            pdu: challenge_response,
        },
    )
    .await?;

    let SendPduDataFrame {
        channel_ids,
        pdu: upgrade_license,
    } = match reader
        .decode_next_frame::<SendPduDataFrame<ServerUpgradeLicense>>()
        .await
    {
        Err(RdpError::ServerLicense(rdp::RdpError::ServerLicenseError(
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
        RdpError::ServerLicense(rdp::RdpError::IOError(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("License verification failed: {err:?}"),
        )))
    })?;

    debug!("Successfully verified the license");

    Ok(())
}

pub async fn capabilities_exchange(
    reader: &mut FramedReader,
    writer: &mut ErasedWriter,
    global_channel_ids: ChannelIdentificators,
    config: &InputConfig,
) -> Result<DesktopSize, RdpError> {
    let share_control_frame = reader.decode_next_frame::<ShareControlFrame>().await?;

    if share_control_frame.channel_ids.channel_id != global_channel_ids.channel_id {
        return Err(RdpError::InvalidResponse(format!(
            "Unexpected Send Data Context channel ID ({})",
            global_channel_ids.channel_id,
        )));
    }

    let capability_sets =
        if let ironrdp_pdu::ShareControlPdu::ServerDemandActive(server_demand_active) = share_control_frame.pdu {
            debug!("Got Server Demand Active PDU: {:?}", server_demand_active.pdu);
            server_demand_active.pdu.capability_sets
        } else {
            return Err(RdpError::UnexpectedPdu(format!(
                "Expected Server Demand Active PDU, got: {:?}",
                share_control_frame.pdu.as_short_name()
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

    let client_confirm_active = ironrdp_pdu::ShareControlPdu::ClientConfirmActive(
        user_info::create_client_confirm_active(config, capability_sets)?,
    );
    debug!("Send Client Confirm Active PDU: {:?}", client_confirm_active);
    encode_next_frame(
        writer,
        ShareControlFrame {
            channel_ids: global_channel_ids,
            share_id: share_control_frame.share_id,
            pdu_source: global_channel_ids.initiator_id,
            data: Bytes::new(),
            pdu: client_confirm_active,
        },
    )
    .await?;
    Ok(desktop_size)
}

pub async fn connection_finalization(
    reader: &mut FramedReader,
    writer: &mut ErasedWriter,
    global_channel_ids: ChannelIdentificators,
) -> Result<(), RdpError> {
    use ironrdp_pdu::rdp::{ControlAction, ControlPdu, FontPdu, SequenceFlags, ShareDataPdu, SynchronizePdu};

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
                target_user_id: global_channel_ids.initiator_id,
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
        encode_next_frame(
            writer,
            ShareDataFrame {
                channel_ids: global_channel_ids,
                share_id: 0,
                pdu_source: global_channel_ids.initiator_id,
                pdu: share_data_pdu,
            },
        )
        .await?;

        let share_data_frame = reader.decode_next_frame::<ShareDataFrame>().await?;
        let share_data_pdu = share_data_frame.pdu;
        debug!("Got Finalization PDU: {:?}", share_data_pdu);

        finalization_order = match (finalization_order, share_data_pdu) {
            (FinalizationOrder::Synchronize, ShareDataPdu::Synchronize(_)) => FinalizationOrder::ControlCooperate,
            (
                FinalizationOrder::ControlCooperate,
                ShareDataPdu::Control(ControlPdu {
                    action: ironrdp_pdu::ControlAction::Cooperate,
                    grant_id: 0,
                    control_id: 0,
                }),
            ) => FinalizationOrder::RequestControl,
            (
                FinalizationOrder::RequestControl,
                ShareDataPdu::Control(ControlPdu {
                    action: ironrdp_pdu::ControlAction::GrantedControl,
                    grant_id,
                    control_id,
                }),
            ) if grant_id == global_channel_ids.initiator_id && control_id == u32::from(SERVER_CHANNEL_ID) => {
                FinalizationOrder::Font
            }
            (FinalizationOrder::Font, ShareDataPdu::FontMap(_)) => FinalizationOrder::Finished,
            (
                order,
                ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(ErrorInfo::ProtocolIndependentCode(
                    ProtocolIndependentCode::None,
                ))),
            ) => order,
            (_, ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(e))) => {
                return Err(RdpError::Server(e.description()));
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

pub async fn write_credssp_ts_request(
    ts_request: credssp::TsRequest,
    mut stream: impl AsyncWrite + Unpin,
) -> Result<(), RdpError> {
    let mut buf = BytesMut::with_capacity(ts_request.buffer_len() as usize);
    buf.resize(ts_request.buffer_len() as usize, 0x00);

    ts_request
        .encode_ts_request(buf.as_mut())
        .map_err(RdpError::TsRequest)?;

    stream.write_all(buf.as_ref()).await?;
    stream.flush().await?;

    Ok(())
}

pub async fn read_credssp_ts_request(mut stream: impl AsyncRead + Unpin) -> Result<credssp::TsRequest, RdpError> {
    const MAX_TS_REQUEST_LENGTH_BUFFER_SIZE: usize = 4;

    let mut buf = BytesMut::with_capacity(MAX_TS_REQUEST_LENGTH_BUFFER_SIZE);
    buf.resize(MAX_TS_REQUEST_LENGTH_BUFFER_SIZE, 0x00);
    stream.read_exact(&mut buf).await?;

    let ts_request_buffer_length = credssp::TsRequest::read_length(buf.as_ref())?;
    buf.resize(ts_request_buffer_length, 0x00);
    stream.read_exact(&mut buf[MAX_TS_REQUEST_LENGTH_BUFFER_SIZE..]).await?;

    let ts_request = credssp::TsRequest::from_buffer(buf.as_ref()).map_err(RdpError::TsRequest)?;

    Ok(ts_request)
}

pub async fn read_early_user_auth_result(
    mut stream: impl AsyncRead + Unpin,
) -> Result<credssp::EarlyUserAuthResult, RdpError> {
    let mut buf = BytesMut::with_capacity(credssp::EARLY_USER_AUTH_RESULT_PDU_SIZE);
    buf.resize(credssp::EARLY_USER_AUTH_RESULT_PDU_SIZE, 0x00);
    stream.read_exact(&mut buf).await?;
    let early_user_auth_result =
        credssp::EarlyUserAuthResult::from_buffer(buf.as_ref()).map_err(RdpError::EarlyUserAuthResult)?;

    Ok(early_user_auth_result)
}

#[cfg(not(feature = "dgw_ext"))]
pub async fn connection_initiation<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    reader: &mut FramedReader<R>,
    mut writer: W,
    security_protocol: ironrdp_pdu::SecurityProtocol,
    username: String,
) -> Result<ironrdp_pdu::SecurityProtocol, RdpError> {
    let connection_request = build_connection_request_with_username(security_protocol, username);
    let mut buffer = Vec::new();
    connection_request.to_buffer(&mut buffer)?;
    debug!("Send X.224 Connection Request PDU: {:?}", connection_request);
    writer.write_all(&buffer).await?;
    writer.flush().await?;

    let frame = reader.read_frame().await?.ok_or(RdpError::AccessDenied)?;
    let connection_response = ironrdp_pdu::Response::from_buffer(frame.as_ref())?;
    if let Some(ironrdp_pdu::ResponseData::Confirm {
        flags,
        protocol: selected_protocol,
    }) = connection_response.response
    {
        debug!(
            "Got X.224 Connection Confirm PDU: selected protocol ({:?}), response flags ({:?})",
            selected_protocol, flags
        );

        if security_protocol.contains(selected_protocol) {
            Ok(selected_protocol)
        } else {
            Err(RdpError::InvalidResponse(format!(
                "Got unexpected security protocol: {selected_protocol:?} while was expected one of {security_protocol:?}"
            )))
        }
    } else {
        Err(RdpError::InvalidResponse(format!(
            "Got unexpected X.224 Connection Response: {:?}",
            connection_response.response
        )))
    }
}

// FIXME: extract this function into another crate later
// TODO: clarify output type (currently the Vec<u8> is the server public key only)
// TODO: returns the whole certification chain in final version
#[cfg(feature = "dgw_ext")]
pub async fn connection_initiation<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    reader: &mut FramedReader<R>,
    mut writer: W,
    proxy_auth_token: String,
    security_protocol: ironrdp_pdu::SecurityProtocol,
    username: String,
    hostname: String,
) -> Result<(ironrdp_pdu::SecurityProtocol, Vec<u8>, std::net::SocketAddr), RdpError> {
    use ironrdp_rdcleanpath::{RDCleanPath, RDCleanPathPdu};
    use x509_cert::der::Decode as _;

    let connection_request = build_connection_request_with_username(security_protocol, username);

    let mut x224_pdu = Vec::new();
    connection_request.to_buffer(&mut x224_pdu)?;

    let rdp_clean_path = RDCleanPathPdu::new_request(x224_pdu, hostname, proxy_auth_token, None)
        .map_err(|e| RdpError::Server(e.to_string()))?;
    debug!("Send RDCleanPath PDU request: {:?}", connection_request);

    let request_bytes = rdp_clean_path.to_der().map_err(|e| {
        RdpError::Connection(io::Error::new(
            io::ErrorKind::Other,
            format!("couldn’t encode cleanpath request into der: {e}"),
        ))
    })?;
    writer.write_all(&request_bytes).await?;

    writer.flush().await?;

    let (reader, buf) = reader.get_inner_mut();

    let cleanpath_pdu = loop {
        if let Some(pdu) = RDCleanPathPdu::decode(buf).map_err(|e| RdpError::Server(e.to_string()))? {
            break pdu;
        }

        let mut read_bytes = [0u8; 1024];
        let len = reader.read(&mut read_bytes[..]).await?;
        buf.extend_from_slice(&read_bytes[..len]);

        if len == 0 {
            return Err(RdpError::InvalidResponse("EOF when reading RDCleanPath PDU".to_owned()));
        }
    };

    debug!("Received RDCleanPath PDU: {:?}", cleanpath_pdu);

    let (x224_connection_response, server_cert_chain, server_addr) = match cleanpath_pdu
        .into_enum()
        .map_err(|e| RdpError::InvalidResponse(format!("Invalid RDCleanPath: {e}")))?
    {
        RDCleanPath::Request { .. } => {
            return Err(RdpError::InvalidResponse(
                "Received an unexpected RDCleanPath request".to_owned(),
            ));
        }
        RDCleanPath::Response {
            x224_connection_response,
            server_cert_chain,
            server_addr,
        } => (x224_connection_response, server_cert_chain, server_addr),
        RDCleanPath::Err(error) => {
            return Err(RdpError::InvalidResponse(format!(
                "Received an RDCleanPath error: {error}"
            )));
        }
    };

    let connection_response = ironrdp_pdu::Response::from_buffer(x224_connection_response.as_bytes())?;

    if let Some(ironrdp_pdu::ResponseData::Confirm {
        flags,
        protocol: selected_protocol,
    }) = connection_response.response
    {
        debug!(
            "Got X.224 Connection Confirm PDU: selected protocol ({:?}), response flags ({:?})",
            selected_protocol, flags
        );

        if security_protocol.contains(selected_protocol) {
            let server_cert = server_cert_chain.into_iter().next().ok_or_else(|| {
                RdpError::Connection(io::Error::new(
                    io::ErrorKind::Other,
                    "cleanpath response is missing the server cert chain",
                ))
            })?;

            let cert = x509_cert::Certificate::from_der(server_cert.as_bytes()).map_err(|e| {
                RdpError::Connection(io::Error::new(
                    io::ErrorKind::Other,
                    format!("couldn’t decode x509 certificate sent by Devolutions Gateway: {e}"),
                ))
            })?;

            let server_public_key = cert.tbs_certificate.subject_public_key_info.subject_public_key.to_vec();

            let server_addr = server_addr.parse().map_err(|e| {
                RdpError::Connection(io::Error::new(
                    io::ErrorKind::Other,
                    format!("couldn’t parse server address sent by Devolutions Gateway: {e}"),
                ))
            })?;

            Ok((selected_protocol, server_public_key, server_addr))
        } else {
            Err(RdpError::InvalidResponse(format!(
                "Got unexpected security protocol: {selected_protocol:?} while was expected one of {security_protocol:?}"
            )))
        }
    } else {
        Err(RdpError::InvalidResponse(format!(
            "Got unexpected X.224 Connection Response: {:?}",
            connection_response.response
        )))
    }
}

fn build_connection_request_with_username(
    security_protocol: ironrdp_pdu::SecurityProtocol,
    username: String,
) -> ironrdp_pdu::Request {
    ironrdp_pdu::Request {
        nego_data: Some(ironrdp_pdu::NegoData::Cookie(username)),
        flags: ironrdp_pdu::RequestFlags::empty(),
        protocol: security_protocol,
        src_ref: 0,
    }
}
