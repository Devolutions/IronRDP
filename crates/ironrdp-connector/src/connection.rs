use core::mem;
use core::net::SocketAddr;
use std::borrow::Cow;
use std::sync::Arc;

use ironrdp_core::{decode, encode_vec, Encode, WriteBuf};
use ironrdp_pdu::x224::X224;
use ironrdp_pdu::{gcc, mcs, nego, rdp, PduHint};
use ironrdp_svc::{StaticChannelSet, StaticVirtualChannel, SvcClientProcessor};
use tracing::{debug, error, info, warn};

use crate::channel_connection::{ChannelConnectionSequence, ChannelConnectionState};
use crate::connection_activation::{ConnectionActivationSequence, ConnectionActivationState};
use crate::license_exchange::{LicenseExchangeSequence, NoopLicenseCache};
use crate::{
    encode_x224_packet, general_err, reason_err, Config, ConnectorError, ConnectorErrorExt as _, ConnectorErrorKind,
    ConnectorResult, DesktopSize, NegotiationFailure, Sequence, State, Written,
};

#[derive(Debug)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct ConnectionResult {
    pub io_channel_id: u16,
    pub user_channel_id: u16,
    pub static_channels: StaticChannelSet,
    pub desktop_size: DesktopSize,
    pub enable_server_pointer: bool,
    pub pointer_software_rendering: bool,
    pub connection_activation: ConnectionActivationSequence,
}

#[derive(Default, Debug)]
#[non_exhaustive]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub enum ClientConnectorState {
    #[default]
    Consumed,

    ConnectionInitiationSendRequest,
    ConnectionInitiationWaitConfirm {
        requested_protocol: nego::SecurityProtocol,
    },
    EnhancedSecurityUpgrade {
        selected_protocol: nego::SecurityProtocol,
    },
    Credssp {
        selected_protocol: nego::SecurityProtocol,
    },
    BasicSettingsExchangeSendInitial {
        selected_protocol: nego::SecurityProtocol,
    },
    BasicSettingsExchangeWaitResponse {
        connect_initial: mcs::ConnectInitial,
    },
    ChannelConnection {
        io_channel_id: u16,
        channel_connection: ChannelConnectionSequence,
    },
    SecureSettingsExchange {
        io_channel_id: u16,
        user_channel_id: u16,
    },
    ConnectTimeAutoDetection {
        io_channel_id: u16,
        user_channel_id: u16,
    },
    LicensingExchange {
        io_channel_id: u16,
        user_channel_id: u16,
        license_exchange: LicenseExchangeSequence,
    },
    MultitransportBootstrapping {
        io_channel_id: u16,
        user_channel_id: u16,
    },
    CapabilitiesExchange {
        connection_activation: ConnectionActivationSequence,
    },
    ConnectionFinalization {
        connection_activation: ConnectionActivationSequence,
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
            Self::ConnectionInitiationWaitConfirm { .. } => "ConnectionInitiationWaitResponse",
            Self::EnhancedSecurityUpgrade { .. } => "EnhancedSecurityUpgrade",
            Self::Credssp { .. } => "Credssp",
            Self::BasicSettingsExchangeSendInitial { .. } => "BasicSettingsExchangeSendInitial",
            Self::BasicSettingsExchangeWaitResponse { .. } => "BasicSettingsExchangeWaitResponse",
            Self::ChannelConnection { .. } => "ChannelConnection",
            Self::SecureSettingsExchange { .. } => "SecureSettingsExchange",
            Self::ConnectTimeAutoDetection { .. } => "ConnectTimeAutoDetection",
            Self::LicensingExchange { .. } => "LicensingExchange",
            Self::MultitransportBootstrapping { .. } => "MultitransportBootstrapping",
            Self::CapabilitiesExchange {
                connection_activation, ..
            } => connection_activation.state().name(),
            Self::ConnectionFinalization {
                connection_activation, ..
            } => connection_activation.state().name(),
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
    /// The client address to be used in the Client Info PDU.
    pub client_addr: SocketAddr,
    pub static_channels: StaticChannelSet,
}

impl ClientConnector {
    pub fn new(config: Config, client_addr: SocketAddr) -> Self {
        Self {
            config,
            state: ClientConnectorState::ConnectionInitiationSendRequest,
            client_addr,
            static_channels: StaticChannelSet::new(),
        }
    }

    #[must_use]
    pub fn with_static_channel<T>(mut self, channel: T) -> Self
    where
        T: SvcClientProcessor + 'static,
    {
        self.static_channels.insert(channel);
        self
    }

    pub fn attach_static_channel<T>(&mut self, channel: T)
    where
        T: SvcClientProcessor + 'static,
    {
        self.static_channels.insert(channel);
    }

    pub fn get_static_channel_processor<T>(&mut self) -> Option<&T>
    where
        T: SvcClientProcessor + 'static,
    {
        self.static_channels
            .get_by_type::<T>()
            .and_then(|channel| channel.channel_processor_downcast_ref())
    }

    pub fn get_static_channel_processor_mut<T>(&mut self) -> Option<&mut T>
    where
        T: SvcClientProcessor + 'static,
    {
        self.static_channels
            .get_by_type_mut::<T>()
            .and_then(|channel| channel.channel_processor_downcast_mut())
    }

    pub fn should_perform_security_upgrade(&self) -> bool {
        matches!(self.state, ClientConnectorState::EnhancedSecurityUpgrade { .. })
    }

    /// # Panics
    ///
    /// Panics if state is not [ClientConnectorState::EnhancedSecurityUpgrade].
    pub fn mark_security_upgrade_as_done(&mut self) {
        assert!(self.should_perform_security_upgrade());
        self.step(&[], &mut WriteBuf::new()).expect("transition to next state");
        debug_assert!(!self.should_perform_security_upgrade());
    }

    pub fn should_perform_credssp(&self) -> bool {
        matches!(self.state, ClientConnectorState::Credssp { .. })
    }

    /// # Panics
    ///
    /// Panics if state is not [ClientConnectorState::Credssp].
    pub fn mark_credssp_as_done(&mut self) {
        assert!(self.should_perform_credssp());
        let res = self.step(&[], &mut WriteBuf::new()).expect("transition to next state");
        debug_assert!(!self.should_perform_credssp());
        assert_eq!(res, Written::Nothing);
    }
}

impl Sequence for ClientConnector {
    fn next_pdu_hint(&self) -> Option<&dyn PduHint> {
        match &self.state {
            ClientConnectorState::Consumed => None,
            ClientConnectorState::ConnectionInitiationSendRequest => None,
            ClientConnectorState::ConnectionInitiationWaitConfirm { .. } => Some(&ironrdp_pdu::X224_HINT),
            ClientConnectorState::EnhancedSecurityUpgrade { .. } => None,
            ClientConnectorState::Credssp { .. } => None,
            ClientConnectorState::BasicSettingsExchangeSendInitial { .. } => None,
            ClientConnectorState::BasicSettingsExchangeWaitResponse { .. } => Some(&ironrdp_pdu::X224_HINT),
            ClientConnectorState::ChannelConnection { channel_connection, .. } => channel_connection.next_pdu_hint(),
            ClientConnectorState::SecureSettingsExchange { .. } => None,
            ClientConnectorState::ConnectTimeAutoDetection { .. } => None,
            ClientConnectorState::LicensingExchange { license_exchange, .. } => license_exchange.next_pdu_hint(),
            ClientConnectorState::MultitransportBootstrapping { .. } => None,
            ClientConnectorState::CapabilitiesExchange {
                connection_activation, ..
            } => connection_activation.next_pdu_hint(),
            ClientConnectorState::ConnectionFinalization {
                connection_activation, ..
            } => connection_activation.next_pdu_hint(),
            ClientConnectorState::Connected { .. } => None,
        }
    }

    fn state(&self) -> &dyn State {
        &self.state
    }

    fn step(&mut self, input: &[u8], output: &mut WriteBuf) -> ConnectorResult<Written> {
        let (written, next_state) = match mem::take(&mut self.state) {
            // Invalid state
            ClientConnectorState::Consumed => {
                return Err(general_err!("connector sequence state is consumed (this is a bug)",))
            }

            //== Connection Initiation ==//
            // Exchange supported security protocols and a few other connection flags.
            ClientConnectorState::ConnectionInitiationSendRequest => {
                debug!("Connection Initiation");

                let mut security_protocol = nego::SecurityProtocol::empty();

                if self.config.enable_tls {
                    security_protocol.insert(nego::SecurityProtocol::SSL);
                }

                if self.config.enable_credssp {
                    // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/902b090b-9cb3-4efc-92bf-ee13373371e3
                    // The spec is stating that `PROTOCOL_SSL` "SHOULD" also be set when using `PROTOCOL_HYBRID`.
                    // > PROTOCOL_HYBRID (0x00000002)
                    // > Credential Security Support Provider protocol (CredSSP) (section 5.4.5.2).
                    // > If this flag is set, then the PROTOCOL_SSL (0x00000001) flag SHOULD also be set
                    // > because Transport Layer Security (TLS) is a subset of CredSSP.
                    // However, crucially, it’s not strictly required (not "MUST").
                    // In fact, we purposefully choose to not set `PROTOCOL_SSL` unless `enable_winlogon` is `true`.
                    // This tells the server that we are not going to accept downgrading NLA to TLS security.
                    security_protocol.insert(nego::SecurityProtocol::HYBRID | nego::SecurityProtocol::HYBRID_EX);
                }

                if security_protocol.is_standard_rdp_security() {
                    return Err(reason_err!("Initiation", "standard RDP security is not supported",));
                }

                let connection_request = nego::ConnectionRequest {
                    nego_data: self.config.request_data.clone().or_else(|| {
                        self.config
                            .credentials
                            .username()
                            .map(|username| nego::NegoRequestData::cookie(username.to_owned()))
                    }),
                    flags: nego::RequestFlags::empty(),
                    protocol: security_protocol,
                };

                debug!(message = ?connection_request, "Send");

                let written =
                    ironrdp_core::encode_buf(&X224(connection_request), output).map_err(ConnectorError::encode)?;

                (
                    Written::from_size(written)?,
                    ClientConnectorState::ConnectionInitiationWaitConfirm {
                        requested_protocol: security_protocol,
                    },
                )
            }
            ClientConnectorState::ConnectionInitiationWaitConfirm { requested_protocol } => {
                let connection_confirm = decode::<X224<nego::ConnectionConfirm>>(input)
                    .map_err(ConnectorError::decode)
                    .map(|p| p.0)?;

                debug!(message = ?connection_confirm, "Received");

                let (flags, selected_protocol) = match connection_confirm {
                    nego::ConnectionConfirm::Response { flags, protocol } => (flags, protocol),
                    nego::ConnectionConfirm::Failure { code } => {
                        error!(?code, "Received connection failure code");
                        return Err(ConnectorError::new(
                            "negotiation failure",
                            ConnectorErrorKind::Negotiation(NegotiationFailure::from(code)),
                        ));
                    }
                };

                info!(?selected_protocol, ?flags, "Server confirmed connection");

                if !selected_protocol.intersects(requested_protocol) {
                    return Err(reason_err!(
                        "Initiation",
                        "client advertised {requested_protocol}, but server selected {selected_protocol}",
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
                let next_state = if selected_protocol
                    .intersects(nego::SecurityProtocol::HYBRID | nego::SecurityProtocol::HYBRID_EX)
                {
                    debug!("Begin NLA using CredSSP");
                    ClientConnectorState::Credssp { selected_protocol }
                } else {
                    debug!("CredSSP is disabled, skipping NLA");
                    ClientConnectorState::BasicSettingsExchangeSendInitial { selected_protocol }
                };

                (Written::Nothing, next_state)
            }

            //== CredSSP ==//
            ClientConnectorState::Credssp { selected_protocol } => (
                Written::Nothing,
                ClientConnectorState::BasicSettingsExchangeSendInitial { selected_protocol },
            ),

            //== Basic Settings Exchange ==//
            // Exchange basic settings including Core Data, Security Data and Network Data.
            ClientConnectorState::BasicSettingsExchangeSendInitial { selected_protocol } => {
                debug!("Basic Settings Exchange");

                let client_gcc_blocks =
                    create_gcc_blocks(&self.config, selected_protocol, self.static_channels.values())?;

                let connect_initial =
                    mcs::ConnectInitial::with_gcc_blocks(client_gcc_blocks).map_err(ConnectorError::decode)?;

                debug!(message = ?connect_initial, "Send");

                let written = encode_x224_packet(&connect_initial, output)?;

                (
                    Written::from_size(written)?,
                    ClientConnectorState::BasicSettingsExchangeWaitResponse { connect_initial },
                )
            }
            ClientConnectorState::BasicSettingsExchangeWaitResponse { connect_initial } => {
                let x224_payload = decode::<X224<crate::x224::X224Data<'_>>>(input)
                    .map_err(ConnectorError::decode)
                    .map(|p| p.0)?;
                let connect_response =
                    decode::<mcs::ConnectResponse>(x224_payload.data.as_ref()).map_err(ConnectorError::decode)?;

                debug!(message = ?connect_response, "Received");

                let client_gcc_blocks = connect_initial.conference_create_request.gcc_blocks();

                let server_gcc_blocks = connect_response.conference_create_response.into_gcc_blocks();

                if client_gcc_blocks.security == gcc::ClientSecurityData::no_security()
                    && server_gcc_blocks.security != gcc::ServerSecurityData::no_security()
                {
                    return Err(general_err!("can't satisfy server security settings"));
                }

                if server_gcc_blocks.message_channel.is_some() {
                    warn!("Unexpected ServerMessageChannelData GCC block (not supported)");
                }

                if server_gcc_blocks.multi_transport_channel.is_some() {
                    warn!("Unexpected MultiTransportChannelData GCC block (not supported)");
                }

                let static_channel_ids = server_gcc_blocks.network.channel_ids;
                let io_channel_id = server_gcc_blocks.network.io_channel;

                debug!(?static_channel_ids, io_channel_id);

                let zipped: Vec<_> = self
                    .static_channels
                    .type_ids()
                    .zip(static_channel_ids.iter().copied())
                    .collect();

                zipped.into_iter().for_each(|(channel, channel_id)| {
                    self.static_channels.attach_channel_id(channel, channel_id);
                });

                let skip_channel_join = server_gcc_blocks
                    .core
                    .optional_data
                    .early_capability_flags
                    .is_some_and(|c| c.contains(gcc::ServerEarlyCapabilityFlags::SKIP_CHANNELJOIN_SUPPORTED));

                (
                    Written::Nothing,
                    ClientConnectorState::ChannelConnection {
                        io_channel_id,
                        channel_connection: if skip_channel_join {
                            ChannelConnectionSequence::skip_channel_join()
                        } else {
                            ChannelConnectionSequence::new(io_channel_id, static_channel_ids)
                        },
                    },
                )
            }

            //== Channel Connection ==//
            // Connect every individual channel.
            ClientConnectorState::ChannelConnection {
                io_channel_id,
                mut channel_connection,
            } => {
                debug!("Channel Connection");
                let written = channel_connection.step(input, output)?;

                let next_state = if let ChannelConnectionState::AllJoined { user_channel_id } = channel_connection.state
                {
                    debug_assert!(channel_connection.state.is_terminal());

                    ClientConnectorState::SecureSettingsExchange {
                        io_channel_id,
                        user_channel_id,
                    }
                } else {
                    ClientConnectorState::ChannelConnection {
                        io_channel_id,
                        channel_connection,
                    }
                };

                (written, next_state)
            }

            //== RDP Security Commencement ==//
            // When using standard RDP security (RC4), a Security Exchange PDU is sent at this point.
            // However, IronRDP does not support this unsecure security protocol (purposefully) and
            // this part of the sequence is not implemented.
            //==============================//

            //== Secure Settings Exchange ==//
            // Send Client Info PDU (information about supported types of compression, username, password, etc).
            ClientConnectorState::SecureSettingsExchange {
                io_channel_id,
                user_channel_id,
            } => {
                debug!("Secure Settings Exchange");

                let client_info = create_client_info_pdu(&self.config, &self.client_addr);

                debug!(message = ?client_info, "Send");

                let written = encode_send_data_request(user_channel_id, io_channel_id, &client_info, output)?;

                (
                    Written::from_size(written)?,
                    ClientConnectorState::ConnectTimeAutoDetection {
                        io_channel_id,
                        user_channel_id,
                    },
                )
            }

            //== Optional Connect-Time Auto-Detection ==//
            // NOTE: IronRDP is not expecting the Auto-Detect Request PDU from server.
            ClientConnectorState::ConnectTimeAutoDetection {
                io_channel_id,
                user_channel_id,
            } => (
                Written::Nothing,
                ClientConnectorState::LicensingExchange {
                    io_channel_id,
                    user_channel_id,
                    license_exchange: LicenseExchangeSequence::new(
                        io_channel_id,
                        self.config.credentials.username().unwrap_or("").to_owned(),
                        self.config.domain.clone(),
                        self.config.hardware_id.unwrap_or_default(),
                        self.config
                            .license_cache
                            .clone()
                            .unwrap_or_else(|| Arc::new(NoopLicenseCache)),
                    ),
                },
            ),

            //== Licensing ==//
            // Server is sending information regarding licensing.
            // Typically useful when support for more than two simultaneous connections is required (terminal server).
            ClientConnectorState::LicensingExchange {
                io_channel_id,
                user_channel_id,
                mut license_exchange,
            } => {
                debug!("Licensing Exchange");

                let written = license_exchange.step(input, output)?;

                let next_state = if license_exchange.state.is_terminal() {
                    ClientConnectorState::MultitransportBootstrapping {
                        io_channel_id,
                        user_channel_id,
                    }
                } else {
                    ClientConnectorState::LicensingExchange {
                        io_channel_id,
                        user_channel_id,
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
            } => (
                Written::Nothing,
                ClientConnectorState::CapabilitiesExchange {
                    connection_activation: ConnectionActivationSequence::new(
                        self.config.clone(),
                        io_channel_id,
                        user_channel_id,
                    ),
                },
            ),

            //== Capabilities Exchange ==/
            // The server sends the set of capabilities it supports to the client.
            ClientConnectorState::CapabilitiesExchange {
                mut connection_activation,
            } => {
                let written = connection_activation.step(input, output)?;
                match connection_activation.connection_activation_state() {
                    ConnectionActivationState::ConnectionFinalization { .. } => (
                        written,
                        ClientConnectorState::ConnectionFinalization { connection_activation },
                    ),
                    _ => return Err(general_err!("invalid state (this is a bug)")),
                }
            }

            //== Connection Finalization ==//
            // Client and server exchange a few PDUs in order to finalize the connection.
            // Client may send PDUs one after the other without waiting for a response in order to speed up the process.
            ClientConnectorState::ConnectionFinalization {
                mut connection_activation,
            } => {
                let written = connection_activation.step(input, output)?;

                let next_state = if !connection_activation.connection_activation_state().is_terminal() {
                    ClientConnectorState::ConnectionFinalization { connection_activation }
                } else {
                    match connection_activation.connection_activation_state() {
                        ConnectionActivationState::Finalized {
                            io_channel_id,
                            user_channel_id,
                            desktop_size,
                            enable_server_pointer,
                            pointer_software_rendering,
                        } => ClientConnectorState::Connected {
                            result: ConnectionResult {
                                io_channel_id,
                                user_channel_id,
                                static_channels: mem::take(&mut self.static_channels),
                                desktop_size,
                                enable_server_pointer,
                                pointer_software_rendering,
                                connection_activation,
                            },
                        },
                        _ => return Err(general_err!("invalid state (this is a bug)")),
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

pub fn encode_send_data_request<T: Encode>(
    initiator_id: u16,
    channel_id: u16,
    user_msg: &T,
    buf: &mut WriteBuf,
) -> ConnectorResult<usize> {
    let user_data = encode_vec(user_msg).map_err(ConnectorError::encode)?;

    let pdu = mcs::SendDataRequest {
        initiator_id,
        channel_id,
        user_data: Cow::Owned(user_data),
    };

    let written = ironrdp_core::encode_buf(&X224(pdu), buf).map_err(ConnectorError::encode)?;

    Ok(written)
}

#[expect(single_use_lifetimes)] // anonymous lifetimes in `impl Trait` are unstable
fn create_gcc_blocks<'a>(
    config: &Config,
    selected_protocol: nego::SecurityProtocol,
    static_channels: impl Iterator<Item = &'a StaticVirtualChannel>,
) -> ConnectorResult<gcc::ClientGccBlocks> {
    use ironrdp_pdu::gcc::{
        ClientCoreData, ClientCoreOptionalData, ClientEarlyCapabilityFlags, ClientGccBlocks, ClientNetworkData,
        ClientSecurityData, ColorDepth, ConnectionType, EncryptionMethod, HighColorDepth, MonitorOrientation,
        RdpVersion, SecureAccessSequence, SupportedColorDepths,
    };

    let max_color_depth = config.bitmap.as_ref().map(|bitmap| bitmap.color_depth).unwrap_or(32);

    let supported_color_depths = match max_color_depth {
        15 => SupportedColorDepths::BPP15,
        16 => SupportedColorDepths::BPP16,
        24 => SupportedColorDepths::BPP24,
        32 => SupportedColorDepths::BPP32 | SupportedColorDepths::BPP16,
        _ => {
            return Err(reason_err!(
                "create gcc blocks",
                "unsupported color depth: {max_color_depth}"
            ))
        }
    };

    let channels = static_channels
        .map(ironrdp_svc::make_channel_definition)
        .collect::<Vec<_>>();

    Ok(ClientGccBlocks {
        core: ClientCoreData {
            version: RdpVersion::V5_PLUS,
            desktop_width: config.desktop_size.width,
            desktop_height: config.desktop_size.height,
            color_depth: ColorDepth::Bpp8, // ignored because we use the optional core data below
            sec_access_sequence: SecureAccessSequence::Del,
            keyboard_layout: config.keyboard_layout,
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
                        | ClientEarlyCapabilityFlags::STRONG_ASYMMETRIC_KEYS
                        | ClientEarlyCapabilityFlags::SUPPORT_SKIP_CHANNELJOIN;

                    // TODO(#136): support for ClientEarlyCapabilityFlags::SUPPORT_STATUS_INFO_PDU

                    if max_color_depth == 32 {
                        early_capability_flags |= ClientEarlyCapabilityFlags::WANT_32_BPP_SESSION;
                    }

                    Some(early_capability_flags)
                },
                dig_product_id: Some(config.dig_product_id.clone()),
                connection_type: Some(ConnectionType::Lan),
                server_selected_protocol: Some(selected_protocol),
                desktop_physical_width: Some(0),  // 0 per FreeRDP
                desktop_physical_height: Some(0), // 0 per FreeRDP
                desktop_orientation: if config.desktop_size.width > config.desktop_size.height {
                    Some(MonitorOrientation::Landscape as u16)
                } else {
                    Some(MonitorOrientation::Portrait as u16)
                },
                desktop_scale_factor: Some(config.desktop_scale_factor),
                device_scale_factor: if config.desktop_scale_factor >= 100 && config.desktop_scale_factor <= 500 {
                    Some(100)
                } else {
                    Some(0)
                },
            },
        },
        security: ClientSecurityData {
            encryption_methods: EncryptionMethod::empty(),
            ext_encryption_methods: 0,
        },
        network: if channels.is_empty() {
            None
        } else {
            Some(ClientNetworkData { channels })
        },
        // TODO(#139): support for Some(ClientClusterData { flags: RedirectionFlags::REDIRECTION_SUPPORTED, redirection_version: RedirectionVersion::V4, redirected_session_id: 0, }),
        cluster: None,
        monitor: None,
        // TODO(#140): support for Client Message Channel Data (https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/f50e791c-de03-4b25-b17e-e914c9020bc3)
        message_channel: None,
        // TODO(#140): support for Some(MultiTransportChannelData { flags: MultiTransportFlags::empty(), })
        multi_transport_channel: None,
        monitor_extended: None,
    })
}

fn create_client_info_pdu(config: &Config, client_addr: &SocketAddr) -> rdp::ClientInfoPdu {
    use ironrdp_pdu::rdp::client_info::{
        AddressFamily, ClientInfo, ClientInfoFlags, CompressionType, Credentials, ExtendedClientInfo,
        ExtendedClientOptionalInfo,
    };
    use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
    use ironrdp_pdu::rdp::ClientInfoPdu;

    let security_header = BasicSecurityHeader {
        flags: BasicSecurityHeaderFlags::INFO_PKT,
    };

    // Default flags for all sessions
    let mut flags = ClientInfoFlags::MOUSE
        | ClientInfoFlags::MOUSE_HAS_WHEEL
        | ClientInfoFlags::UNICODE
        | ClientInfoFlags::DISABLE_CTRL_ALT_DEL
        | ClientInfoFlags::LOGON_NOTIFY
        | ClientInfoFlags::LOGON_ERRORS
        | ClientInfoFlags::VIDEO_DISABLE
        | ClientInfoFlags::ENABLE_WINDOWS_KEY
        | ClientInfoFlags::MAXIMIZE_SHELL;

    if config.autologon {
        flags |= ClientInfoFlags::AUTOLOGON;
    }

    if let crate::Credentials::SmartCard { .. } = &config.credentials {
        flags |= ClientInfoFlags::PASSWORD_IS_SC_PIN;
    }

    if !config.enable_audio_playback {
        flags |= ClientInfoFlags::NO_AUDIO_PLAYBACK;
    }

    let client_info = ClientInfo {
        credentials: Credentials {
            username: config.credentials.username().unwrap_or("").to_owned(),
            password: config.credentials.secret().to_owned(),
            domain: config.domain.clone(),
        },
        code_page: 0, // ignored if the keyboardLayout field of the Client Core Data is set to zero
        flags,
        compression_type: CompressionType::K8, // ignored if ClientInfoFlags::COMPRESSION is not set
        alternate_shell: String::new(),
        work_dir: String::new(),
        extra_info: ExtendedClientInfo {
            address_family: match client_addr {
                SocketAddr::V4(_) => AddressFamily::INET,
                SocketAddr::V6(_) => AddressFamily::INET_6,
            },
            address: client_addr.ip().to_string(),
            dir: config.client_dir.clone(),
            optional_data: ExtendedClientOptionalInfo::builder()
                .timezone(config.timezone_info.clone())
                .session_id(0)
                .performance_flags(config.performance_flags)
                .build(),
        },
    };

    ClientInfoPdu {
        security_header,
        client_info,
    }
}
