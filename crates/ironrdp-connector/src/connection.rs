use std::mem;
use std::net::SocketAddr;

use ironrdp_pdu::rdp::capability_sets::CapabilitySet;
use ironrdp_pdu::{gcc, mcs, nego, rdp, PduHint};
use sspi::credssp;

use crate::channel_connection::{ChannelConnectionSequence, ChannelConnectionState};
use crate::connection_finalization::ConnectionFinalizationSequence;
use crate::license_exchange::LicenseExchangeSequence;
use crate::{
    legacy, Config, ConnectorError, ConnectorErrorExt as _, ConnectorErrorKind, ConnectorResult, DesktopSize, Sequence,
    ServerName, State, StaticChannels, Written,
};

#[derive(Clone, Copy, Debug)]
pub struct CredsspTsRequestHint;

pub const CREDSSP_TS_REQUEST_HINT: CredsspTsRequestHint = CredsspTsRequestHint;

impl PduHint for CredsspTsRequestHint {
    fn find_size(&self, bytes: &[u8]) -> ironrdp_pdu::PduResult<Option<usize>> {
        match sspi::credssp::TsRequest::read_length(bytes) {
            Ok(length) => Ok(Some(length)),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(e) => Err(ironrdp_pdu::custom_err!("CredsspTsRequestHint", e)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CredsspEarlyUserAuthResultHint;

pub const CREDSSP_EARLY_USER_AUTH_RESULT_HINT: CredsspEarlyUserAuthResultHint = CredsspEarlyUserAuthResultHint;

impl PduHint for CredsspEarlyUserAuthResultHint {
    fn find_size(&self, _: &[u8]) -> ironrdp_pdu::PduResult<Option<usize>> {
        Ok(Some(sspi::credssp::EARLY_USER_AUTH_RESULT_PDU_SIZE))
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct ConnectionResult {
    pub io_channel_id: u16,
    pub user_channel_id: u16,
    pub static_channels: StaticChannels,
    pub desktop_size: DesktopSize,
    pub graphics_config: Option<crate::GraphicsConfig>,
}

#[derive(Default, Debug)]
#[non_exhaustive]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub enum ClientConnectorState {
    #[default]
    Consumed,

    ConnectionInitiationSendRequest,
    ConnectionInitiationWaitConfirm,
    EnhancedSecurityUpgrade {
        selected_protocol: nego::SecurityProtocol,
    },
    CredsspInitial {
        selected_protocol: nego::SecurityProtocol,
    },
    /// At this point, user code may use `sspi::credssp::TsRequest::read_length` to find the correct frame length.
    CredsspReplyNeeded {
        selected_protocol: nego::SecurityProtocol,
        credssp_client: Box<credssp::CredSspClient>,
    },
    CredsspEarlyUserAuthResult {
        selected_protocol: nego::SecurityProtocol,
    },
    BasicSettingsExchangeSendInitial {
        selected_protocol: nego::SecurityProtocol,
    },
    BasicSettingsExchangeWaitResponse {
        connect_initial: mcs::ConnectInitial,
        selected_protocol: nego::SecurityProtocol,
    },
    ChannelConnection {
        selected_protocol: nego::SecurityProtocol,
        io_channel_id: u16,
        static_channels: StaticChannels,
        channel_connection: ChannelConnectionSequence,
    },
    RdpSecurityCommencement {
        selected_protocol: nego::SecurityProtocol,
        io_channel_id: u16,
        user_channel_id: u16,
        static_channels: StaticChannels,
    },
    SecureSettingsExchange {
        io_channel_id: u16,
        user_channel_id: u16,
        static_channels: StaticChannels,
    },
    ConnectTimeAutoDetection {
        io_channel_id: u16,
        user_channel_id: u16,
        static_channels: StaticChannels,
    },
    LicensingExchange {
        io_channel_id: u16,
        user_channel_id: u16,
        static_channels: StaticChannels,
        license_exchange: LicenseExchangeSequence,
    },
    MultitransportBootstrapping {
        io_channel_id: u16,
        user_channel_id: u16,
        static_channels: StaticChannels,
    },
    CapabilitiesExchange {
        io_channel_id: u16,
        user_channel_id: u16,
        static_channels: StaticChannels,
    },
    ConnectionFinalization {
        io_channel_id: u16,
        user_channel_id: u16,
        static_channels: StaticChannels,
        desktop_size: DesktopSize,
        connection_finalization: ConnectionFinalizationSequence,
    },
    Connected {
        result: ConnectionResult,
    },
}

impl State for ClientConnectorState {
    fn name(&self) -> &'static str {
        match self {
            Self::Consumed => "Consumed",
            Self::ConnectionInitiationSendRequest => "ConnectionInitiationSendRequest",
            Self::ConnectionInitiationWaitConfirm => "ConnectionInitiationWaitResponse",
            Self::EnhancedSecurityUpgrade { .. } => "EnhancedSecurityUpgrade",
            Self::CredsspInitial { .. } => "CredsspInitial",
            Self::CredsspReplyNeeded { .. } => "CredsspReplyNeeded",
            Self::CredsspEarlyUserAuthResult { .. } => "CredsspEarlyUserAuthResult",
            Self::BasicSettingsExchangeSendInitial { .. } => "BasicSettingsExchangeSendInitial",
            Self::BasicSettingsExchangeWaitResponse { .. } => "BasicSettingsExchangeWaitResponse",
            Self::ChannelConnection { .. } => "ChannelConnection",
            Self::RdpSecurityCommencement { .. } => "RdpSecurityCommencement",
            Self::SecureSettingsExchange { .. } => "SecureSettingsExchange",
            Self::ConnectTimeAutoDetection { .. } => "ConnectTimeAutoDetection",
            Self::LicensingExchange { .. } => "LicensingExchange",
            Self::MultitransportBootstrapping { .. } => "MultitransportBootstrapping",
            Self::CapabilitiesExchange { .. } => "CapabilitiesExchange",
            Self::ConnectionFinalization { .. } => "ConnectionFinalization",
            Self::Connected { .. } => "Connected",
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct ClientConnector {
    pub config: Config,
    pub state: ClientConnectorState,
    pub server_addr: Option<std::net::SocketAddr>,
    pub server_name: Option<ServerName>,
    pub network_client_factory: Option<Box<dyn sspi::network_client::NetworkClientFactory>>,
    pub server_public_key: Option<Vec<u8>>,
}

impl ClientConnector {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            state: ClientConnectorState::ConnectionInitiationSendRequest,
            server_addr: None,
            server_name: None,
            network_client_factory: None,
            server_public_key: None,
        }
    }

    /// Must be set to the actual target server address (as opposed to the proxy)
    pub fn with_server_addr(mut self, addr: std::net::SocketAddr) -> Self {
        self.server_addr = Some(addr);
        self
    }

    /// Must be set to the actual target server address (as opposed to the proxy)
    pub fn attach_server_addr(&mut self, addr: std::net::SocketAddr) {
        self.server_addr = Some(addr);
    }

    /// Must be set to the actual target server hostname (as opposed to the proxy)
    pub fn with_server_name(mut self, name: impl Into<ServerName>) -> Self {
        self.server_name = Some(name.into());
        self
    }

    /// Must be set to the actual target server hostname (as opposed to the proxy)
    pub fn attach_server_name(&mut self, name: impl Into<ServerName>) {
        self.server_name = Some(name.into());
    }

    pub fn with_credssp_client_factory(
        mut self,
        network_client_factory: Box<dyn sspi::network_client::NetworkClientFactory>,
    ) -> Self {
        self.network_client_factory = Some(network_client_factory);
        self
    }

    pub fn attach_credssp_client_factory(
        &mut self,
        network_client_factory: Box<dyn sspi::network_client::NetworkClientFactory>,
    ) {
        self.network_client_factory = Some(network_client_factory);
    }

    pub fn attach_server_public_key(&mut self, server_public_key: Vec<u8>) {
        self.server_public_key = Some(server_public_key);
    }

    pub fn should_perform_security_upgrade(&self) -> bool {
        matches!(self.state, ClientConnectorState::EnhancedSecurityUpgrade { .. })
    }

    pub fn mark_security_upgrade_as_done(&mut self) {
        assert!(self.should_perform_security_upgrade());
        self.step(&[], &mut Vec::new()).expect("transition to next state");
        debug_assert!(!self.should_perform_security_upgrade());
    }

    pub fn is_credssp_step(&self) -> bool {
        matches!(
            &self.state,
            ClientConnectorState::CredsspInitial { .. }
                | ClientConnectorState::CredsspReplyNeeded { .. }
                | ClientConnectorState::CredsspEarlyUserAuthResult { .. }
        )
    }
}

impl Sequence for ClientConnector {
    fn next_pdu_hint(&self) -> Option<&dyn PduHint> {
        match &self.state {
            ClientConnectorState::Consumed => None,
            ClientConnectorState::ConnectionInitiationSendRequest => None,
            ClientConnectorState::ConnectionInitiationWaitConfirm => Some(&ironrdp_pdu::X224_HINT),
            ClientConnectorState::EnhancedSecurityUpgrade { .. } => None,
            ClientConnectorState::CredsspInitial { .. } => None,
            ClientConnectorState::CredsspReplyNeeded { .. } => Some(&CREDSSP_TS_REQUEST_HINT),
            ClientConnectorState::CredsspEarlyUserAuthResult { .. } => Some(&CREDSSP_EARLY_USER_AUTH_RESULT_HINT),
            ClientConnectorState::BasicSettingsExchangeSendInitial { .. } => None,
            ClientConnectorState::BasicSettingsExchangeWaitResponse { .. } => Some(&ironrdp_pdu::X224_HINT),
            ClientConnectorState::ChannelConnection { channel_connection, .. } => channel_connection.next_pdu_hint(),
            ClientConnectorState::RdpSecurityCommencement { .. } => None,
            ClientConnectorState::SecureSettingsExchange { .. } => None,
            ClientConnectorState::ConnectTimeAutoDetection { .. } => None,
            ClientConnectorState::LicensingExchange { license_exchange, .. } => license_exchange.next_pdu_hint(),
            ClientConnectorState::MultitransportBootstrapping { .. } => None,
            ClientConnectorState::CapabilitiesExchange { .. } => Some(&ironrdp_pdu::X224_HINT),
            ClientConnectorState::ConnectionFinalization {
                connection_finalization,
                ..
            } => connection_finalization.next_pdu_hint(),
            ClientConnectorState::Connected { .. } => None,
        }
    }

    fn state(&self) -> &dyn State {
        &self.state
    }

    fn step(&mut self, input: &[u8], output: &mut Vec<u8>) -> ConnectorResult<Written> {
        let (written, next_state) = match mem::take(&mut self.state) {
            // Invalid state
            ClientConnectorState::Consumed => {
                return Err(general_err!("connector sequence state is consumed (this is a bug)",))
            }

            //== Connection Initiation ==//
            // Exchange supported security protocols and a few other connection flags.
            ClientConnectorState::ConnectionInitiationSendRequest => {
                let connection_request = nego::ConnectionRequest {
                    nego_data: Some(nego::NegoRequestData::cookie(self.config.username.clone())),
                    flags: nego::RequestFlags::empty(),
                    protocol: self.config.security_protocol,
                };

                debug!(message = ?connection_request, "Send");

                let written = ironrdp_pdu::encode_buf(&connection_request, output).map_err(ConnectorError::pdu)?;

                (
                    Written::from_size(written)?,
                    ClientConnectorState::ConnectionInitiationWaitConfirm,
                )
            }
            ClientConnectorState::ConnectionInitiationWaitConfirm => {
                let connection_confirm =
                    ironrdp_pdu::decode::<nego::ConnectionConfirm>(input).map_err(ConnectorError::pdu)?;

                debug!(message = ?connection_confirm, "Received");

                let (flags, selected_protocol) = match connection_confirm {
                    nego::ConnectionConfirm::Response { flags, protocol } => (flags, protocol),
                    nego::ConnectionConfirm::Failure { code } => {
                        error!(?code, "Received connection failure code");
                        return Err(general_err!("connection failed"));
                    }
                };

                info!(?selected_protocol, ?flags, "Server confirmed connection");

                if !self.config.security_protocol.contains(selected_protocol) {
                    return Err(general_err!(
                        "server selected a security protocol that is unsupported by this client",
                    ));
                }

                (
                    Written::Nothing,
                    ClientConnectorState::EnhancedSecurityUpgrade { selected_protocol },
                )
            }

            //== Upgrade to Enhanced RDP Security ==//
            // NOTE: we assume the selected protocol is never the standard RDP security (RC4).
            // User code should match this variant and perform the appropriate upgrade (TLS handshake, etc).
            ClientConnectorState::EnhancedSecurityUpgrade { selected_protocol } => {
                let next_state = if selected_protocol.contains(nego::SecurityProtocol::HYBRID)
                    || selected_protocol.contains(nego::SecurityProtocol::HYBRID_EX)
                {
                    ClientConnectorState::CredsspInitial { selected_protocol }
                } else {
                    ClientConnectorState::BasicSettingsExchangeSendInitial { selected_protocol }
                };

                (Written::Nothing, next_state)
            }

            //== CredSSP ==//
            ClientConnectorState::CredsspInitial { selected_protocol } => {
                let credentials = sspi::AuthIdentity {
                    username: self.config.username.clone(),
                    password: self.config.password.clone().into(),
                    domain: self.config.domain.clone(),
                };

                let server_public_key = self
                    .server_public_key
                    .take()
                    .ok_or_else(|| general_err!("server public key is missing"))?;

                let network_client_factory = self
                    .network_client_factory
                    .take()
                    .ok_or_else(|| general_err!("CredSSP network client factory is missing"))?;

                let server_name = self
                    .server_name
                    .take()
                    .ok_or_else(|| general_err!("server name is missing"))?
                    .into_inner();

                let service_principal_name = format!("TERMSRV/{server_name}");

                let mut credssp_client = credssp::CredSspClient::new(
                    server_public_key,
                    credentials,
                    credssp::CredSspMode::WithCredentials,
                    credssp::ClientMode::Negotiate(sspi::NegotiateConfig {
                        protocol_config: Box::<sspi::ntlm::NtlmConfig>::default(),
                        package_list: None,
                        hostname: server_name,
                        network_client_factory,
                    }),
                    service_principal_name,
                )
                .map_err(|e| ConnectorError::new("CredSSP", ConnectorErrorKind::Credssp(e)))?;

                let initial_ts_request = credssp::TsRequest::default();

                let result = credssp_client
                    .process(initial_ts_request)
                    .map_err(|e| ConnectorError::new("CredSSP", ConnectorErrorKind::Credssp(e)))?;

                let (ts_request_from_client, next_state) = match result {
                    credssp::ClientState::ReplyNeeded(ts_request) => (
                        ts_request,
                        ClientConnectorState::CredsspReplyNeeded {
                            selected_protocol,
                            credssp_client: Box::new(credssp_client),
                        },
                    ),
                    credssp::ClientState::FinalMessage(ts_request) => (
                        ts_request,
                        ClientConnectorState::BasicSettingsExchangeSendInitial { selected_protocol },
                    ),
                };

                debug!(message = ?ts_request_from_client, "Send");

                let written = write_credssp_request(ts_request_from_client, output)?;

                (Written::from_size(written)?, next_state)
            }
            ClientConnectorState::CredsspReplyNeeded {
                selected_protocol,
                mut credssp_client,
            } => {
                let ts_request_from_server = credssp::TsRequest::from_buffer(input)
                    .map_err(|e| reason_err!("CredSSP", "TsRequest decode: {e}"))?;

                let result = credssp_client
                    .process(ts_request_from_server)
                    .map_err(|e| ConnectorError::new("CredSSP", ConnectorErrorKind::Credssp(e)))?;

                let (ts_request_from_client, next_state) = match result {
                    credssp::ClientState::ReplyNeeded(ts_request) => (
                        ts_request,
                        ClientConnectorState::CredsspReplyNeeded {
                            selected_protocol,
                            credssp_client,
                        },
                    ),
                    credssp::ClientState::FinalMessage(ts_request) => (
                        ts_request,
                        if selected_protocol.contains(nego::SecurityProtocol::HYBRID_EX) {
                            ClientConnectorState::CredsspEarlyUserAuthResult { selected_protocol }
                        } else {
                            ClientConnectorState::BasicSettingsExchangeSendInitial { selected_protocol }
                        },
                    ),
                };

                debug!(message = ?ts_request_from_client, "Send");

                let written = write_credssp_request(ts_request_from_client, output)?;

                (Written::from_size(written)?, next_state)
            }
            ClientConnectorState::CredsspEarlyUserAuthResult { selected_protocol } => {
                let early_user_auth_result = credssp::EarlyUserAuthResult::from_buffer(input)
                    .map_err(|e| custom_err!("credssp::EarlyUserAuthResult", e))?;

                let credssp::EarlyUserAuthResult::Success = early_user_auth_result else {
                    return Err(ConnectorError::new("CredSSP", ConnectorErrorKind::AccessDenied));
                };

                (
                    Written::Nothing,
                    ClientConnectorState::BasicSettingsExchangeSendInitial { selected_protocol },
                )
            }

            //== Basic Settings Exchange ==//
            // Exchange basic settings including Core Data, Security Data and Network Data.
            ClientConnectorState::BasicSettingsExchangeSendInitial { selected_protocol } => {
                let client_gcc_blocks = create_gcc_blocks(&self.config, selected_protocol);
                let connect_initial = mcs::ConnectInitial::with_gcc_blocks(client_gcc_blocks);

                debug!(message = ?connect_initial, "Send");

                let written = legacy::encode_x224_packet(&connect_initial, output)?;

                (
                    Written::from_size(written)?,
                    ClientConnectorState::BasicSettingsExchangeWaitResponse {
                        connect_initial,
                        selected_protocol,
                    },
                )
            }
            ClientConnectorState::BasicSettingsExchangeWaitResponse {
                connect_initial,
                selected_protocol,
            } => {
                let connect_response = legacy::decode_x224_packet::<mcs::ConnectResponse>(input)?;

                debug!(message = ?connect_response, "Received");

                let client_gcc_blocks = &connect_initial.conference_create_request.gcc_blocks;

                let server_gcc_blocks = connect_response.conference_create_response.gcc_blocks;

                if client_gcc_blocks.security == gcc::ClientSecurityData::no_security()
                    && server_gcc_blocks.security != gcc::ServerSecurityData::no_security()
                {
                    return Err(general_err!("can’t satisfy server security settings"));
                }

                if server_gcc_blocks.message_channel.is_some() {
                    warn!("Unexpected server message channel data received");
                }

                if server_gcc_blocks.multi_transport_channel.is_some() {
                    warn!("Unexpected multitransport channel data received");
                }

                let static_channel_ids = server_gcc_blocks.network.channel_ids;
                let io_channel_id = server_gcc_blocks.network.io_channel;

                debug!(?static_channel_ids, io_channel_id);

                let static_channels = connect_initial
                    .channel_names()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|channel| channel.name)
                    .zip(static_channel_ids.iter().copied())
                    .collect::<StaticChannels>();

                (
                    Written::Nothing,
                    ClientConnectorState::ChannelConnection {
                        selected_protocol,
                        io_channel_id,
                        static_channels,
                        channel_connection: ChannelConnectionSequence::new(io_channel_id, static_channel_ids),
                    },
                )
            }

            //== Channel Connection ==//
            // Connect every individual channel.
            ClientConnectorState::ChannelConnection {
                selected_protocol,
                io_channel_id,
                static_channels,
                mut channel_connection,
            } => {
                let written = channel_connection.step(input, output)?;

                let next_state = if let ChannelConnectionState::AllJoined { user_channel_id } = channel_connection.state
                {
                    debug_assert!(channel_connection.state.is_terminal());

                    ClientConnectorState::RdpSecurityCommencement {
                        selected_protocol,
                        io_channel_id,
                        user_channel_id,
                        static_channels,
                    }
                } else {
                    ClientConnectorState::ChannelConnection {
                        selected_protocol,
                        io_channel_id,
                        static_channels,
                        channel_connection,
                    }
                };

                (written, next_state)
            }

            //== RDP Security Commencement ==//
            // When using standard RDP security (RC4), a Security Exchange PDU is sent at this point.
            // NOTE: IronRDP does not support RC4 security.
            ClientConnectorState::RdpSecurityCommencement {
                selected_protocol,
                io_channel_id,
                user_channel_id,
                static_channels,
            } => {
                if selected_protocol == nego::SecurityProtocol::RDP {
                    return Err(general_err!("standard RDP Security (RC4 encryption) is not supported"));
                }

                (
                    Written::Nothing,
                    ClientConnectorState::SecureSettingsExchange {
                        io_channel_id,
                        user_channel_id,
                        static_channels,
                    },
                )
            }

            //== Secure Settings Exchange ==//
            // Send Client Info PDU (information about supported types of compression, username, password, etc).
            ClientConnectorState::SecureSettingsExchange {
                io_channel_id,
                user_channel_id,
                static_channels,
            } => {
                let routing_addr = self
                    .server_addr
                    .as_ref()
                    .ok_or_else(|| general_err!("server address is missing"))?;

                let client_info = create_client_info_pdu(&self.config, routing_addr);

                debug!(message = ?client_info, "Send");

                let written = legacy::encode_send_data_request(user_channel_id, io_channel_id, &client_info, output)?;

                (
                    Written::from_size(written)?,
                    ClientConnectorState::ConnectTimeAutoDetection {
                        io_channel_id,
                        user_channel_id,
                        static_channels,
                    },
                )
            }

            //== Optional Connect-Time Auto-Detection ==//
            // NOTE: IronRDP is not expecting the Auto-Detect Request PDU from server.
            ClientConnectorState::ConnectTimeAutoDetection {
                io_channel_id,
                user_channel_id,
                static_channels,
            } => (
                Written::Nothing,
                ClientConnectorState::LicensingExchange {
                    io_channel_id,
                    user_channel_id,
                    static_channels,
                    license_exchange: LicenseExchangeSequence::new(
                        io_channel_id,
                        self.config.username.clone(),
                        self.config.domain.clone(),
                    ),
                },
            ),

            //== Licensing ==//
            // Server is sending information regarding licensing.
            // Typically useful when support for more than two simultaneous connections is required (terminal server).
            ClientConnectorState::LicensingExchange {
                io_channel_id,
                user_channel_id,
                static_channels,
                mut license_exchange,
            } => {
                let written = license_exchange.step(input, output)?;

                let next_state = if license_exchange.state.is_terminal() {
                    ClientConnectorState::MultitransportBootstrapping {
                        io_channel_id,
                        user_channel_id,
                        static_channels,
                    }
                } else {
                    ClientConnectorState::LicensingExchange {
                        io_channel_id,
                        user_channel_id,
                        static_channels,
                        license_exchange,
                    }
                };

                (written, next_state)
            }

            //== Optional Multitransport Bootstrapping ==//
            // NOTE: our implementation is not expecting the Auto-Detect Request PDU from server
            ClientConnectorState::MultitransportBootstrapping {
                io_channel_id,
                user_channel_id,
                static_channels,
            } => (
                Written::Nothing,
                ClientConnectorState::CapabilitiesExchange {
                    io_channel_id,
                    user_channel_id,
                    static_channels,
                },
            ),

            //== Capabilities Exchange ==/
            // The server sends the set of capabilities it supports to the client.
            ClientConnectorState::CapabilitiesExchange {
                io_channel_id,
                user_channel_id,
                static_channels,
            } => {
                let send_data_indication_ctx = legacy::decode_send_data_indication(input)?;
                let share_control_ctx = legacy::decode_share_control(send_data_indication_ctx)?;

                debug!(message = ?share_control_ctx.pdu, "Received");

                if share_control_ctx.channel_id != io_channel_id {
                    warn!(
                        io_channel_id,
                        share_control_ctx.channel_id, "Unexpected channel ID for received Share Control Pdu"
                    );
                }

                let capability_sets = if let rdp::headers::ShareControlPdu::ServerDemandActive(server_demand_active) =
                    share_control_ctx.pdu
                {
                    server_demand_active.pdu.capability_sets
                } else {
                    return Err(general_err!(
                        "unexpected Share Control Pdu (expected ServerDemandActive)",
                    ));
                };

                let desktop_size = capability_sets
                    .iter()
                    .find_map(|c| match c {
                        rdp::capability_sets::CapabilitySet::Bitmap(b) => Some(DesktopSize {
                            width: b.desktop_width,
                            height: b.desktop_height,
                        }),
                        _ => None,
                    })
                    .unwrap_or(DesktopSize {
                        width: self.config.desktop_size.width,
                        height: self.config.desktop_size.height,
                    });

                let client_confirm_active = rdp::headers::ShareControlPdu::ClientConfirmActive(
                    create_client_confirm_active(&self.config, capability_sets),
                );

                debug!(message = ?client_confirm_active, "Send");

                let written = legacy::encode_share_control(
                    user_channel_id,
                    io_channel_id,
                    share_control_ctx.share_id,
                    client_confirm_active,
                    output,
                )?;

                (
                    Written::from_size(written)?,
                    ClientConnectorState::ConnectionFinalization {
                        io_channel_id,
                        user_channel_id,
                        static_channels,
                        desktop_size,
                        connection_finalization: ConnectionFinalizationSequence::new(io_channel_id, user_channel_id),
                    },
                )
            }

            //== Connection Finalization ==//
            // Client and server exchange a few PDUs in order to finalize the connection.
            // Client may send PDUs one after the other without waiting for a response in order to speed up the process.
            ClientConnectorState::ConnectionFinalization {
                io_channel_id,
                user_channel_id,
                static_channels,
                desktop_size,
                mut connection_finalization,
            } => {
                let written = connection_finalization.step(input, output)?;

                let next_state = if connection_finalization.state.is_terminal() {
                    ClientConnectorState::Connected {
                        result: ConnectionResult {
                            io_channel_id,
                            user_channel_id,
                            static_channels,
                            desktop_size,
                            graphics_config: self.config.graphics.clone(),
                        },
                    }
                } else {
                    ClientConnectorState::ConnectionFinalization {
                        io_channel_id,
                        user_channel_id,
                        static_channels,
                        desktop_size,
                        connection_finalization,
                    }
                };

                (written, next_state)
            }

            //== Connected ==//
            // The client connector job is done.
            ClientConnectorState::Connected { .. } => return Err(general_err!("already connected")),
        };

        self.state = next_state;

        Ok(written)
    }
}

fn create_gcc_blocks(config: &Config, selected_protocol: nego::SecurityProtocol) -> gcc::ClientGccBlocks {
    use ironrdp_pdu::gcc::*;

    let max_color_depth = config.bitmap.as_ref().map(|bitmap| bitmap.color_depth).unwrap_or(32);

    let supported_color_depths = match max_color_depth {
        15 => SupportedColorDepths::BPP15,
        16 => SupportedColorDepths::BPP16,
        24 => SupportedColorDepths::BPP24,
        32 => SupportedColorDepths::BPP32 | SupportedColorDepths::BPP16,
        _ => panic!("Unsupported color depth: {}", max_color_depth),
    };

    ClientGccBlocks {
        core: ClientCoreData {
            version: RdpVersion::V5_PLUS,
            desktop_width: config.desktop_size.width,
            desktop_height: config.desktop_size.height,
            color_depth: ColorDepth::Bpp8, // ignored because we use the optional core data below
            sec_access_sequence: SecureAccessSequence::Del,
            keyboard_layout: 0, // the server SHOULD use the default active input locale identifier
            client_build: config.client_build,
            client_name: config.client_name.clone(),
            keyboard_type: config.keyboard_type,
            keyboard_subtype: config.keyboard_subtype,
            keyboard_functional_keys_count: config.keyboard_functional_keys_count,
            ime_file_name: config.ime_file_name.clone(),
            optional_data: ClientCoreOptionalData {
                post_beta2_color_depth: Some(ColorDepth::Bpp8), // ignored because we set high_color_depth
                client_product_id: Some(1),
                serial_number: Some(0),
                high_color_depth: Some(HighColorDepth::Bpp24),
                supported_color_depths: Some(supported_color_depths),
                early_capability_flags: {
                    let mut early_capability_flags = ClientEarlyCapabilityFlags::VALID_CONNECTION_TYPE
                        | ClientEarlyCapabilityFlags::SUPPORT_ERR_INFO_PDU
                        | ClientEarlyCapabilityFlags::SUPPORT_STATUS_INFO_PDU
                        | ClientEarlyCapabilityFlags::STRONG_ASYMMETRIC_KEYS;

                    if config.graphics.is_some() {
                        early_capability_flags |= ClientEarlyCapabilityFlags::SUPPORT_DYN_VC_GFX_PROTOCOL;
                    }

                    if max_color_depth == 32 {
                        early_capability_flags |= ClientEarlyCapabilityFlags::WANT_32_BPP_SESSION;
                    }

                    Some(early_capability_flags)
                },
                dig_product_id: Some(config.dig_product_id.clone()),
                connection_type: Some(ConnectionType::Lan),
                server_selected_protocol: Some(selected_protocol),
                desktop_physical_width: None,
                desktop_physical_height: None,
                desktop_orientation: None,
                desktop_scale_factor: None,
                device_scale_factor: None,
            },
        },
        security: ClientSecurityData {
            encryption_methods: EncryptionMethod::empty(),
            ext_encryption_methods: 0,
        },
        network: if config.graphics.is_some() {
            Some(ClientNetworkData {
                channels: vec![Channel {
                    name: "drdynvc".to_owned(),
                    options: ChannelOptions::COMPRESS_RDP,
                }],
            })
        } else {
            Some(ClientNetworkData { channels: Vec::new() })
        },
        cluster: Some(ClientClusterData {
            flags: RedirectionFlags::REDIRECTION_SUPPORTED,
            redirection_version: RedirectionVersion::V4,
            redirected_session_id: 0,
        }),
        monitor: None,
        message_channel: Some(ClientMessageChannelData {}),
        multi_transport_channel: Some(MultiTransportChannelData {
            flags: MultiTransportFlags::empty(),
        }),
        monitor_extended: None,
    }
}

fn create_client_info_pdu(config: &Config, routing_addr: &SocketAddr) -> rdp::ClientInfoPdu {
    use ironrdp_pdu::rdp::client_info::{
        AddressFamily, ClientInfo, ClientInfoFlags, CompressionType, Credentials, ExtendedClientInfo,
        ExtendedClientOptionalInfo,
    };
    use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
    use ironrdp_pdu::rdp::ClientInfoPdu;

    let security_header = BasicSecurityHeader {
        flags: BasicSecurityHeaderFlags::INFO_PKT,
    };

    let client_info = ClientInfo {
        credentials: Credentials {
            username: config.username.clone(),
            password: config.password.clone(),
            domain: config.domain.clone(),
        },
        code_page: 0, // ignored if the keyboardLayout field of the Client Core Data is set to zero
        flags: ClientInfoFlags::UNICODE
            | ClientInfoFlags::DISABLE_CTRL_ALT_DEL
            | ClientInfoFlags::LOGON_NOTIFY
            | ClientInfoFlags::LOGON_ERRORS
            | ClientInfoFlags::NO_AUDIO_PLAYBACK
            | ClientInfoFlags::VIDEO_DISABLE,
        compression_type: CompressionType::K8, // ignored if ClientInfoFlags::COMPRESSION is not set
        alternate_shell: String::new(),
        work_dir: String::new(),
        extra_info: ExtendedClientInfo {
            address_family: match routing_addr {
                SocketAddr::V4(_) => AddressFamily::INet,
                SocketAddr::V6(_) => AddressFamily::INet6,
            },
            address: routing_addr.ip().to_string(),
            dir: config.client_dir.clone(),
            optional_data: ExtendedClientOptionalInfo::default(),
        },
    };

    ClientInfoPdu {
        security_header,
        client_info,
    }
}

fn create_client_confirm_active(
    config: &Config,
    mut server_capability_sets: Vec<CapabilitySet>,
) -> rdp::capability_sets::ClientConfirmActive {
    use ironrdp_pdu::rdp::capability_sets::*;

    server_capability_sets.retain(|capability_set| matches!(capability_set, CapabilitySet::MultiFragmentUpdate(_)));

    let lossy_bitmap_compression = config
        .bitmap
        .as_ref()
        .map(|bitmap| bitmap.lossy_compression)
        .unwrap_or(false);

    let drawing_flags = if lossy_bitmap_compression {
        BitmapDrawingFlags::ALLOW_SKIP_ALPHA
            | BitmapDrawingFlags::ALLOW_DYNAMIC_COLOR_FIDELITY
            | BitmapDrawingFlags::ALLOW_COLOR_SUBSAMPLING
    } else {
        BitmapDrawingFlags::ALLOW_SKIP_ALPHA
    };

    server_capability_sets.extend_from_slice(&[
        CapabilitySet::General(General {
            major_platform_type: config.platform,
            minor_platform_type: MinorPlatformType::Unspecified,
            extra_flags: GeneralExtraFlags::FASTPATH_OUTPUT_SUPPORTED | GeneralExtraFlags::NO_BITMAP_COMPRESSION_HDR,
            refresh_rect_support: false,
            suppress_output_support: false,
        }),
        CapabilitySet::Bitmap(Bitmap {
            pref_bits_per_pix: 32,
            desktop_width: config.desktop_size.width,
            desktop_height: config.desktop_size.height,
            desktop_resize_flag: false,
            drawing_flags,
        }),
        CapabilitySet::Order(Order::new(
            OrderFlags::NEGOTIATE_ORDER_SUPPORT | OrderFlags::ZERO_BOUNDS_DELTAS_SUPPORT,
            OrderSupportExFlags::empty(),
            0,
            0,
        )),
        CapabilitySet::BitmapCache(BitmapCache {
            caches: [CacheEntry {
                entries: 0,
                max_cell_size: 0,
            }; BITMAP_CACHE_ENTRIES_NUM],
        }),
        CapabilitySet::Input(Input {
            input_flags: InputFlags::all(),
            keyboard_layout: 0,
            keyboard_type: Some(config.keyboard_type),
            keyboard_subtype: config.keyboard_subtype,
            keyboard_function_key: config.keyboard_functional_keys_count,
            keyboard_ime_filename: config.ime_file_name.clone(),
        }),
        CapabilitySet::Pointer(Pointer {
            color_pointer_cache_size: 0,
            pointer_cache_size: 0,
        }),
        CapabilitySet::Brush(Brush {
            support_level: SupportLevel::Default,
        }),
        CapabilitySet::GlyphCache(GlyphCache {
            glyph_cache: [CacheDefinition {
                entries: 0,
                max_cell_size: 0,
            }; GLYPH_CACHE_NUM],
            frag_cache: CacheDefinition {
                entries: 0,
                max_cell_size: 0,
            },
            glyph_support_level: GlyphSupportLevel::None,
        }),
        CapabilitySet::OffscreenBitmapCache(OffscreenBitmapCache {
            is_supported: false,
            cache_size: 0,
            cache_entries: 0,
        }),
        CapabilitySet::VirtualChannel(VirtualChannel {
            flags: VirtualChannelFlags::NO_COMPRESSION,
            chunk_size: Some(0), // ignored
        }),
        CapabilitySet::Sound(Sound {
            flags: SoundFlags::empty(),
        }),
        CapabilitySet::LargePointer(LargePointer {
            flags: LargePointerSupportFlags::UP_TO_96X96_PIXELS,
        }),
        CapabilitySet::SurfaceCommands(SurfaceCommands {
            flags: CmdFlags::SET_SURFACE_BITS | CmdFlags::STREAM_SURFACE_BITS | CmdFlags::FRAME_MARKER,
        }),
        CapabilitySet::BitmapCodecs(BitmapCodecs(vec![Codec {
            id: 0x03, // RemoteFX
            property: CodecProperty::RemoteFx(RemoteFxContainer::ClientContainer(RfxClientCapsContainer {
                capture_flags: CaptureFlags::empty(),
                caps_data: RfxCaps(RfxCapset(vec![RfxICap {
                    flags: RfxICapFlags::empty(),
                    entropy_bits: EntropyBits::Rlgr3,
                }])),
            })),
        }])),
        CapabilitySet::FrameAcknowledge(FrameAcknowledge {
            max_unacknowledged_frame_count: 2,
        }),
    ]);

    if !server_capability_sets
        .iter()
        .any(|c| matches!(&c, CapabilitySet::MultiFragmentUpdate(_)))
    {
        server_capability_sets.push(CapabilitySet::MultiFragmentUpdate(MultifragmentUpdate {
            max_request_size: 1024,
        }));
    }

    ClientConfirmActive {
        originator_id: SERVER_CHANNEL_ID,
        pdu: DemandActive {
            source_descriptor: "IRONRDP".to_owned(),
            capability_sets: server_capability_sets,
        },
    }
}

fn write_credssp_request(ts_request: credssp::TsRequest, output: &mut Vec<u8>) -> crate::ConnectorResult<usize> {
    let length = usize::from(ts_request.buffer_len());

    if output.len() < length {
        output.resize(length, 0);
    }

    ts_request
        .encode_ts_request(output.as_mut_slice())
        .map_err(|e| reason_err!("CredSSP", "TsRequest encode: {e}"))?;

    Ok(length)
}
