use core::mem;
use core::net::SocketAddr;
use std::borrow::Cow;
use std::sync::Arc;

use ironrdp_core::{Encode, WriteBuf, decode, encode_vec};
use ironrdp_pdu::x224::X224;
use ironrdp_pdu::{PduHint, gcc, mcs, nego, pcb, rdp};
use ironrdp_svc::{StaticChannelSet, StaticVirtualChannel, SvcClientProcessor};
use tracing::{debug, error, info, warn};

use crate::channel_connection::{ChannelConnectionSequence, ChannelConnectionState};
use crate::connection_activation::{
    ConnectionActivationFactory, ConnectionActivationSequence, ConnectionActivationState,
};
use crate::credssp::DEFAULT_SPN_SERVICE_CLASS;
use crate::license_exchange::{LicenseExchangeSequence, NoopLicenseCache};
use crate::{
    Config, ConnectorError, ConnectorErrorExt as _, ConnectorErrorKind, ConnectorResult, DesktopSize,
    NegotiationFailure, Sequence, State, Written, encode_x224_packet, general_err, reason_err,
};

/// Security protocols advertised in the Hyper-V X.224 connection request.
///
/// Matches what mstsc offers for a Hyper-V console; a Direct Approach host answers with plain
/// `HYBRID`. Wider than [`HYPERV_REQUIRED_PROTOCOL`], which is the only answer we accept.
///
/// [2.2.1.1.1] RDP Negotiation Request (RDP_NEG_REQ)
///
/// [2.2.1.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/902b090b-9cb3-4efc-92bf-ee13373371e3
const HYPERV_SECURITY_PROTOCOL: nego::SecurityProtocol = nego::SecurityProtocol::HYBRID_EX
    .union(nego::SecurityProtocol::SSL)
    .union(nego::SecurityProtocol::HYBRID);

/// The one protocol a Hyper-V console connection actually performs: NLA runs with it, and the host
/// must confirm exactly it. We advertise `HYBRID_EX` but refuse it if selected, because it would
/// have the host answer CredSSP with an Early User Authorization Result PDU that nothing reads —
/// by the time the negotiation happens here, CredSSP is long finished.
const HYPERV_REQUIRED_PROTOCOL: nego::SecurityProtocol = nego::SecurityProtocol::HYBRID;

/// SPN service class for a Hyper-V console. The console is a separate service from the Remote
/// Desktop host and registers its own SPN, so [`DEFAULT_SPN_SERVICE_CLASS`] fails Kerberos here.
///
/// See the SPNs `setspn` registers for VMConnect.exe: [Missing or incorrect service principal
/// names](https://learn.microsoft.com/en-us/troubleshoot/system-center/vmm/authentication-authorization-problems).
const HYPERV_SPN_SERVICE_CLASS: &str = "Microsoft Virtual Console Service";

/// How the beginning of the connection is ordered.
///
/// Standard RDP and a Hyper-V console run the same steps; they only disagree about when the X.224
/// negotiation happens. Everything from the Basic Settings Exchange onward is shared, so this is the
/// whole difference between the two — see the transition table in the [`Front`] methods below.
#[derive(Debug)]
enum Front {
    /// Negotiate X.224 first, then upgrade security and authenticate.
    Standard,
    /// Route to the VM console with a Preconnection Blob, upgrade security and authenticate, and
    /// negotiate X.224 last (the Direct Approach).
    HyperV { vm_id: String },
}

impl Front {
    /// Where the connection starts.
    fn initial_state(&self) -> ClientConnectorState {
        match self {
            Self::Standard => ClientConnectorState::ConnectionInitiationSendRequest,
            Self::HyperV { .. } => ClientConnectorState::PreconnectionBlobSendRequest,
        }
    }

    /// Where the X.224 negotiation leads.
    fn after_initiation(&self, selected_protocol: nego::SecurityProtocol) -> ClientConnectorState {
        match self {
            Self::Standard => ClientConnectorState::EnhancedSecurityUpgrade { selected_protocol },
            Self::HyperV { .. } => ClientConnectorState::BasicSettingsExchangeSendInitial { selected_protocol },
        }
    }

    /// Where authentication leads. Mirrors [`Front::after_initiation`]: whichever of the two ran
    /// last hands over to the Basic Settings Exchange.
    fn after_credssp(&self, selected_protocol: nego::SecurityProtocol) -> ClientConnectorState {
        match self {
            Self::Standard => ClientConnectorState::BasicSettingsExchangeSendInitial { selected_protocol },
            Self::HyperV { .. } => ClientConnectorState::ConnectionInitiationSendRequest,
        }
    }

    /// The service we authenticate against.
    fn spn_service_class(&self) -> &'static str {
        match self {
            Self::Standard => DEFAULT_SPN_SERVICE_CLASS,
            Self::HyperV { .. } => HYPERV_SPN_SERVICE_CLASS,
        }
    }

    /// The security protocols to advertise in the X.224 connection request.
    fn advertised_protocol(&self, config: &Config) -> ConnectorResult<nego::SecurityProtocol> {
        match self {
            Self::HyperV { .. } => Ok(HYPERV_SECURITY_PROTOCOL),
            Self::Standard => {
                let mut security_protocol = nego::SecurityProtocol::empty();

                if config.enable_tls {
                    security_protocol.insert(nego::SecurityProtocol::SSL);
                }

                if config.enable_credssp {
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

                Ok(security_protocol)
            }
        }
    }

    /// Checks the protocol the server selected against what this ordering can accept.
    fn accept_selected_protocol(
        &self,
        selected_protocol: nego::SecurityProtocol,
        requested_protocol: nego::SecurityProtocol,
    ) -> ConnectorResult<()> {
        match self {
            Self::Standard => {
                if !selected_protocol.intersects(requested_protocol) {
                    return Err(reason_err!(
                        "Initiation",
                        "client advertised {requested_protocol}, but server selected {selected_protocol}",
                    ));
                }
            }
            // Accepting anything else would echo a protocol we never performed into Client Core
            // Data: NLA already ran with plain HYBRID by this point.
            Self::HyperV { .. } => {
                if selected_protocol != HYPERV_REQUIRED_PROTOCOL {
                    return Err(reason_err!(
                        "Initiation",
                        "server must select {HYPERV_REQUIRED_PROTOCOL} for a Hyper-V console, but it selected {selected_protocol}",
                    ));
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct ConnectionResult {
    pub io_channel_id: u16,
    pub user_channel_id: u16,
    /// MCS channel ID of the message channel, when one was negotiated.
    pub message_channel_id: Option<u16>,
    pub share_id: u32,
    pub static_channels: StaticChannelSet,
    pub desktop_size: DesktopSize,
    /// The server's Input capability flags from the Server Demand Active PDU.
    ///
    /// Per [MS-RDPBCGR] 2.2.8.1.2, fast-path input events may only be sent when
    /// `INPUT_FLAG_FASTPATH_INPUT` or `INPUT_FLAG_FASTPATH_INPUT2` is present; some
    /// servers (e.g. VirtualBox VRDP) close the connection on unsolicited fast-path
    /// input, so clients should fall back to slow-path input PDUs when neither flag
    /// is set.
    ///
    /// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/b8e7c588-51cb-455b-bb73-92d480903133
    pub input_flags: rdp::capability_sets::InputFlags,
    pub enable_server_pointer: bool,
    pub pointer_software_rendering: bool,
    /// Factory for producing connection activation sequences.
    ///
    /// Used to drive the [Deactivation-Reactivation Sequence] when a Server Deactivate All PDU is
    /// received: produce a fresh sequence, drive it to completion, then drop it.
    ///
    /// [Deactivation-Reactivation Sequence]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dfc234ce-481a-4674-9a5d-2a7bafb14432
    pub activation_factory: ConnectionActivationFactory,
    /// The bulk compression type that was negotiated, if any.
    pub compression_type: Option<rdp::client_info::CompressionType>,
}

#[derive(Default, Debug)]
#[non_exhaustive]
pub enum ClientConnectorState {
    #[default]
    Consumed,

    ConnectionInitiationSendRequest,
    ConnectionInitiationWaitConfirm {
        requested_protocol: nego::SecurityProtocol,
    },
    PreconnectionBlobSendRequest,
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
            Self::PreconnectionBlobSendRequest => "PreconnectionBlobSendRequest",
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
#[expect(clippy::partial_pub_fields, reason = "`Front` is an implementation detail")]
pub struct ClientConnector {
    pub config: Config,
    pub state: ClientConnectorState,
    /// The client address to be used in the Client Info PDU.
    pub client_addr: SocketAddr,
    pub static_channels: StaticChannelSet,
    /// MCS message channel ID assigned by the server, once negotiated.
    pub message_channel_id: Option<u16>,
    /// How the front of this connection is ordered.
    front: Front,
}

impl ClientConnector {
    pub fn new(config: Config, client_addr: SocketAddr) -> Self {
        Self::with_front(config, client_addr, Front::Standard)
    }

    /// Creates a connector for a Hyper-V console session, identified by `vm_id` (a VM GUID).
    ///
    /// The connection runs the Hyper-V ordering: a Preconnection Blob carrying `vm_id`, then TLS,
    /// then CredSSP, then the X.224 negotiation — after which it rejoins the shared sequence at the
    /// Basic Settings Exchange. The async and blocking drivers run it exactly like a standard
    /// connection.
    ///
    /// A Hyper-V console only accepts TLS and NLA, so [`Config::enable_tls`] and
    /// [`Config::enable_credssp`] are not consulted: both always happen.
    ///
    /// The transport underneath must be plain bytes to the host: a socket, or a tunnel such as an
    /// RDS gateway. A proxy that negotiates X.224 for you, the way RDCleanPath does, expects that
    /// to happen first, but here it happens last — `skip_connect_begin` panics on the mismatch.
    pub fn new_vmconnect(config: Config, client_addr: SocketAddr, vm_id: String) -> Self {
        Self::with_front(config, client_addr, Front::HyperV { vm_id })
    }

    fn with_front(config: Config, client_addr: SocketAddr, front: Front) -> Self {
        Self {
            config,
            state: front.initial_state(),
            client_addr,
            static_channels: StaticChannelSet::new(),
            message_channel_id: None,
            front,
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

    /// The SPN service class to authenticate against, for [`CredsspSequence::init`]. Only the
    /// connector knows whether this connection targets a Hyper-V console or a Remote Desktop host.
    ///
    /// [`CredsspSequence::init`]: crate::credssp::CredsspSequence::init
    pub fn spn_service_class(&self) -> &'static str {
        self.front.spn_service_class()
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

fn advance_licensing_exchange(
    mut license_exchange: LicenseExchangeSequence,
    io_channel_id: u16,
    user_channel_id: u16,
    input: &[u8],
    output: &mut WriteBuf,
) -> ConnectorResult<(Written, ClientConnectorState)> {
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

    Ok((written, next_state))
}

/// Decodes an X.224 Connection Confirm and returns the protocol the server selected, mapping a
/// negotiation Failure to an error.
fn decode_connection_confirm(input: &[u8]) -> ConnectorResult<nego::SecurityProtocol> {
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

    Ok(selected_protocol)
}

impl Sequence for ClientConnector {
    fn next_pdu_hint(&self) -> Option<&dyn PduHint> {
        match &self.state {
            ClientConnectorState::Consumed => None,
            ClientConnectorState::ConnectionInitiationSendRequest => None,
            ClientConnectorState::ConnectionInitiationWaitConfirm { .. } => Some(&ironrdp_pdu::X224_HINT),
            ClientConnectorState::PreconnectionBlobSendRequest => None,
            ClientConnectorState::EnhancedSecurityUpgrade { .. } => None,
            ClientConnectorState::Credssp { .. } => None,
            ClientConnectorState::BasicSettingsExchangeSendInitial { .. } => None,
            ClientConnectorState::BasicSettingsExchangeWaitResponse { .. } => Some(&ironrdp_pdu::X224_HINT),
            ClientConnectorState::ChannelConnection { channel_connection, .. } => channel_connection.next_pdu_hint(),
            ClientConnectorState::SecureSettingsExchange { .. } => None,
            ClientConnectorState::ConnectTimeAutoDetection { .. } => {
                // Wait for input only when a message channel was negotiated, so
                // we can receive connect-time auto-detect requests there. With a
                // message channel the server always sends a PDU next in this phase
                // (a connect-time Auto-Detect Request on the message channel, or
                // the first licensing PDU on the I/O channel), so waiting here
                // cannot stall. Without one, this state reads nothing and
                // transitions straight to licensing.
                if self.message_channel_id.is_some() {
                    Some(&ironrdp_pdu::X224_HINT)
                } else {
                    None
                }
            }
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
                return Err(general_err!("connector sequence state is consumed (this is a bug)",));
            }

            //== Connection Initiation ==//
            // Exchange supported security protocols and a few other connection flags. Runs first for
            // a standard connection, and last in the front for a Hyper-V console.
            ClientConnectorState::ConnectionInitiationSendRequest => {
                debug!("Connection Initiation");

                let security_protocol = self.front.advertised_protocol(&self.config)?;

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
                let selected_protocol = decode_connection_confirm(input)?;

                self.front
                    .accept_selected_protocol(selected_protocol, requested_protocol)?;

                (Written::Nothing, self.front.after_initiation(selected_protocol))
            }

            //== Preconnection Blob ==//
            // The host needs this to route the connection to the VM console before anything else is
            // exchanged.
            ClientConnectorState::PreconnectionBlobSendRequest => {
                // `Front::initial_state` only reaches this state for `Front::HyperV`.
                let Front::HyperV { vm_id } = &self.front else {
                    return Err(general_err!(
                        "preconnection blob is only sent for a Hyper-V connection (this is a bug)"
                    ));
                };

                debug!("Send Preconnection Blob");

                let pcb = pcb::PreconnectionBlob {
                    id: 0,
                    version: pcb::PcbVersion::V2,
                    v2_payload: Some(vm_id.clone()),
                };

                let written = ironrdp_core::encode_buf(&pcb, output).map_err(ConnectorError::encode)?;

                // Nothing is negotiated yet, so this is the protocol we are going to perform rather
                // than one the host picked; the X.224 exchange later holds the host to it.
                (
                    Written::from_size(written)?,
                    ClientConnectorState::EnhancedSecurityUpgrade {
                        selected_protocol: HYPERV_REQUIRED_PROTOCOL,
                    },
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
            ClientConnectorState::Credssp { selected_protocol } => {
                (Written::Nothing, self.front.after_credssp(selected_protocol))
            }

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

                self.message_channel_id = server_gcc_blocks
                    .message_channel
                    .as_ref()
                    .map(|data| data.mcs_message_channel_id);

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
                            let mut join_channel_ids = static_channel_ids;
                            join_channel_ids.extend(self.message_channel_id);
                            ChannelConnectionSequence::new(io_channel_id, join_channel_ids)
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
            } => {
                // The server may run Optional Connect-Time Auto-Detection on the
                // message channel before licensing ([MS-RDPBCGR] 1.3.8). When a
                // message channel was negotiated we wait for a PDU here and demux
                // by MCS channel: a PDU on the message channel is never a licensing
                // PDU, so it must not be handed to the licensing sequence. An
                // auto-detect request is answered and we keep listening; any other
                // message-channel PDU is not ours to act on in this phase and is
                // ignored. The first PDU that is not on the message channel (the
                // licensing PDU on the I/O channel) ends the phase. Without a
                // message channel nothing is read and we go straight to licensing,
                // as before.
                // Decode the inbound PDU once and demux on the MCS channel.
                let message_channel_pdu = self.message_channel_id.and_then(|message_channel_id| {
                    let mcs = decode::<X224<mcs::McsMessage<'_>>>(input).ok()?;
                    match mcs.0 {
                        mcs::McsMessage::SendDataIndication(data) if data.channel_id == message_channel_id => {
                            Some((message_channel_id, data))
                        }
                        _ => None,
                    }
                });

                if let Some((message_channel_id, data)) = message_channel_pdu {
                    if let Ok(autodetect) = decode::<rdp::autodetect::AutoDetectReqPdu>(&data.user_data) {
                        let written = respond_to_connect_time_autodetect(
                            autodetect.request,
                            message_channel_id,
                            user_channel_id,
                            output,
                        )?;
                        (
                            written,
                            ClientConnectorState::ConnectTimeAutoDetection {
                                io_channel_id,
                                user_channel_id,
                            },
                        )
                    } else {
                        // A message-channel PDU we do not handle in this phase (per the
                        // canonical sequence multitransport bootstrap is Phase 8 and
                        // heartbeat is post-connection, both after licensing). Ignore it
                        // and keep listening rather than decoding it as a licensing PDU.
                        (
                            Written::Nothing,
                            ClientConnectorState::ConnectTimeAutoDetection {
                                io_channel_id,
                                user_channel_id,
                            },
                        )
                    }
                } else {
                    let license_exchange = LicenseExchangeSequence::new(
                        io_channel_id,
                        self.config.credentials.username().unwrap_or("").to_owned(),
                        self.config.domain.clone(),
                        self.config.hardware_id.unwrap_or_default(),
                        self.config
                            .license_cache
                            .clone()
                            .unwrap_or_else(|| Arc::new(NoopLicenseCache)),
                    );
                    // If a PDU was read (message channel present) it is the first
                    // licensing PDU; advance the licensing sequence with it now,
                    // through the same helper the LicensingExchange state uses, so
                    // the terminal-state transition lives in one place. Otherwise
                    // nothing was read and the licensing sequence runs from its
                    // first step when the next PDU arrives.
                    if self.message_channel_id.is_some() {
                        advance_licensing_exchange(license_exchange, io_channel_id, user_channel_id, input, output)?
                    } else {
                        (
                            Written::Nothing,
                            ClientConnectorState::LicensingExchange {
                                io_channel_id,
                                user_channel_id,
                                license_exchange,
                            },
                        )
                    }
                }
            }

            //== Licensing ==//
            // Server is sending information regarding licensing.
            // Typically useful when support for more than two simultaneous connections is required (terminal server).
            ClientConnectorState::LicensingExchange {
                io_channel_id,
                user_channel_id,
                license_exchange,
            } => {
                debug!("Licensing Exchange");

                advance_licensing_exchange(license_exchange, io_channel_id, user_channel_id, input, output)?
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
                    // The inner sequence stays in CapabilitiesExchange when it receives a
                    // Server Deactivate All PDU before the Server Demand Active PDU (sent
                    // by e.g. Windows Server and gnome-remote-desktop); mirror it here and
                    // wait for the next input.
                    ConnectionActivationState::CapabilitiesExchange => (
                        written,
                        ClientConnectorState::CapabilitiesExchange { connection_activation },
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
                            desktop_size,
                            share_id,
                            input_flags,
                            enable_server_pointer,
                            pointer_software_rendering,
                        } => ClientConnectorState::Connected {
                            result: ConnectionResult {
                                io_channel_id: connection_activation.io_channel_id(),
                                user_channel_id: connection_activation.user_channel_id(),
                                message_channel_id: self.message_channel_id,
                                share_id,
                                static_channels: mem::take(&mut self.static_channels),
                                desktop_size,
                                input_flags,
                                enable_server_pointer,
                                pointer_software_rendering,
                                activation_factory: ConnectionActivationFactory::new(
                                    self.config.clone(),
                                    connection_activation.io_channel_id(),
                                    connection_activation.user_channel_id(),
                                ),
                                compression_type: self.config.compression_type,
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

fn respond_to_connect_time_autodetect(
    request: rdp::autodetect::AutoDetectRequest,
    message_channel_id: u16,
    user_channel_id: u16,
    output: &mut WriteBuf,
) -> ConnectorResult<Written> {
    use ironrdp_pdu::rdp::autodetect::{AutoDetectRequest, AutoDetectResponse, AutoDetectRspPdu};

    match request {
        AutoDetectRequest::RttRequest { sequence_number, .. } => {
            let response = AutoDetectRspPdu::new(AutoDetectResponse::RttResponse { sequence_number });
            let written = encode_send_data_request(user_channel_id, message_channel_id, &response, output)?;
            Written::from_size(written)
        }
        // Only RTT is answered at connect time. A connect-time Bandwidth Measure
        // Stop ([MS-RDPBCGR] 2.2.14.1.4) is defined to warrant a Bandwidth Measure
        // Results reply, and the Network Characteristics Result is informational.
        // We deliberately send neither: connect-time auto-detect is informational
        // and the server proceeds to licensing whether or not it receives them, so
        // skipping them does not stall the sequence. Full connect-time bandwidth
        // measurement (replying to Bandwidth Measure Stop with Bandwidth Measure
        // Results) is left for a follow-up.
        _ => Ok(Written::Nothing),
    }
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

    // Derive the preferred depth indicator. 32bpp has no highColorDepth value; it is
    // expressed via WANT_32_BPP_SESSION in earlyCapabilityFlags instead.
    let high_color_depth = match max_color_depth {
        15 => HighColorDepth::Rgb555Bpp16,
        16 => HighColorDepth::Rgb565Bpp16,
        24 | 32 => HighColorDepth::Bpp24,
        _ => {
            return Err(reason_err!(
                "create gcc blocks",
                "unsupported color depth: {max_color_depth}"
            ));
        }
    };

    // Advertise all colour depth capabilities unconditionally. The preferred depth is
    // expressed via highColorDepth and WANT_32_BPP_SESSION, not by restricting this
    // bitmask. This lets servers negotiate down without resetting the connection.
    let supported_color_depths = SupportedColorDepths::BPP32
        | SupportedColorDepths::BPP24
        | SupportedColorDepths::BPP16
        | SupportedColorDepths::BPP15;

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
                high_color_depth: Some(high_color_depth),
                supported_color_depths: Some(supported_color_depths),
                early_capability_flags: {
                    let mut early_capability_flags = ClientEarlyCapabilityFlags::VALID_CONNECTION_TYPE
                        | ClientEarlyCapabilityFlags::SUPPORT_ERR_INFO_PDU
                        | ClientEarlyCapabilityFlags::STRONG_ASYMMETRIC_KEYS
                        | ClientEarlyCapabilityFlags::SUPPORT_NET_CHAR_AUTODETECT
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
                    Some(MonitorOrientation::Landscape.as_u16())
                } else {
                    Some(MonitorOrientation::Portrait.as_u16())
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
        // Request the MCS message channel, which carries network auto-detect
        // ([MS-RDPBCGR] 2.2.14) and the multitransport / heartbeat PDUs. The
        // server assigns its ID in Server Message Channel Data.
        message_channel: Some(gcc::ClientMessageChannelData),
        multi_transport_channel: config
            .multitransport_flags
            .map(|flags| gcc::MultiTransportChannelData { flags }),
        monitor_extended: None,
    })
}

fn create_client_info_pdu(config: &Config, client_addr: &SocketAddr) -> rdp::ClientInfoPdu {
    use ironrdp_pdu::rdp::ClientInfoPdu;
    use ironrdp_pdu::rdp::client_info::{
        AddressFamily, ClientInfo, ClientInfoFlags, CompressionType, Credentials, ExtendedClientInfo,
        ExtendedClientOptionalInfo,
    };
    use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};

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

    // Advertise bulk compression support if configured
    let compression_type = if let Some(ct) = config.compression_type {
        flags |= ClientInfoFlags::COMPRESSION;
        info!(compression_type = ?ct, "Advertising bulk compression in Client Info PDU");
        ct
    } else {
        CompressionType::K8 // ignored if ClientInfoFlags::COMPRESSION is not set
    };

    let client_info = ClientInfo {
        credentials: Credentials {
            username: config.credentials.username().unwrap_or("").to_owned(),
            password: config.credentials.secret().to_owned(),
            domain: config.domain.clone(),
        },
        code_page: 0, // ignored if the keyboardLayout field of the Client Core Data is set to zero
        flags,
        compression_type,
        alternate_shell: config.alternate_shell.clone(),
        work_dir: config.work_dir.clone(),
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
