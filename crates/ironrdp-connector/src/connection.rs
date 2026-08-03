use core::any::TypeId;
use core::mem;
use core::net::SocketAddr;
use std::borrow::Cow;
use std::sync::Arc;

use ironrdp_core::{Encode, WriteBuf, decode, encode_vec};
use ironrdp_pdu::rdp::capability_sets::WindowSupportLevel;
use ironrdp_pdu::rdp::session_info::ServerAutoReconnect;
use ironrdp_pdu::x224::X224;
use ironrdp_pdu::{PduHint, gcc, mcs, nego, rdp};
use ironrdp_svc::{MAX_STATIC_CHANNELS, StaticChannelKey, StaticChannelSet, StaticVirtualChannel, SvcClientProcessor};
use tracing::{debug, error, info, warn};

use crate::channel_connection::{ChannelConnectionSequence, ChannelConnectionState};
use crate::connection_activation::{
    ConnectionActivationFactory, ConnectionActivationSequence, ConnectionActivationState,
};
use crate::license_exchange::{LicenseExchangeSequence, NoopLicenseCache};
use crate::{
    Config, ConnectorError, ConnectorErrorExt as _, ConnectorErrorKind, ConnectorResult, DesktopSize, MonotonicInstant,
    NegotiationFailure, Sequence, State, Written, encode_x224_packet, general_err, reason_err,
};

/// Maximum number of `Initiate Multitransport Request` PDUs the server is
/// permitted to send during bootstrapping, per MS-RDPBCGR 2.2.15.1 (one per
/// transport protocol: reliable + lossy UDP).
const MAX_MULTITRANSPORT_REQUESTS: usize = 2;

/// Reported as `timeDelta` when a connect-time bandwidth window cannot be timed.
///
/// One millisecond rather than zero, because a server computing
/// `byteCount * 8 / timeDelta` divides by it. The result understates a fast link,
/// which is the safe direction: the figure is an informational QoS hint, and the
/// server proceeds on receipt either way.
const UNMEASURABLE_INTERVAL_MS: u32 = 1;

/// Outcome of a single multitransport bootstrapping request, passed to
/// [`ClientConnector::complete_multitransport()`].
///
/// The connector uses this to build the response PDU internally, carrying the
/// request ID from the server's original request. The request's 16-byte
/// security cookie is not echoed here: it binds the UDP transport itself, not
/// the main-channel response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultitransportResult {
    /// UDP transport was established successfully (`S_OK`).
    Success,
    /// UDP transport failed. The `u32` is the HRESULT error code (typically
    /// [`MultitransportResponsePdu::E_ABORT`](rdp::multitransport::MultitransportResponsePdu::E_ABORT)).
    Failure(u32),
}

/// Why a runtime-defined static virtual channel could not be registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicStaticChannelAttachError {
    /// The negotiated static-channel key space has no remaining entries.
    ChannelLimitReached,
    /// Another static channel already uses the requested channel name.
    DuplicateChannelName,
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
    /// Whether the server permits client Refresh Rect PDUs for visual recovery.
    pub refresh_rect_support: bool,
    /// Whether the server permits Suppress Output PDUs for visual recovery.
    pub suppress_output_support: bool,
    /// The Window List support level negotiated with the server.
    ///
    /// `None` means Window List was absent or disabled, so windowing orders
    /// retain ordinary desktop-session handling.
    pub window_support_level: Option<WindowSupportLevel>,
    /// The monitor layout reported by the server during connection finalization.
    pub monitor_layout: Option<rdp::finalization_messages::MonitorLayoutPdu>,
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
    /// Waiting for either an Initiate Multitransport Request or the Demand
    /// Active that ends the optional bootstrapping phase.
    ///
    /// The server may send 0, 1, or 2 requests, and there is no end marker for
    /// the set: it is not obliged to send Demand Active before the client acts
    /// on a request it has already sent. So each request is surfaced to the
    /// caller the moment it decodes, and the connector comes back here for the
    /// next PDU rather than trying to collect a batch it cannot know the size
    /// of. A PDU on the I/O channel is the Demand Active and ends the phase.
    MultitransportBootstrapping {
        io_channel_id: u16,
        user_channel_id: u16,
        /// MCS message channel negotiated during GCC, when the server offered
        /// one. Multitransport travels on it in both directions per
        /// MS-RDPBCGR 2.2.15.1 and 2.2.15.2, so it is both the channel inbound
        /// requests arrive on and the channel responses go out on. `None` means
        /// the server never offered one, in which case no request can be valid.
        message_channel_id: Option<u16>,
        /// How many requests have been surfaced so far, to enforce the cap in
        /// MS-RDPBCGR 2.2.15.1 now that they are handled one at a time.
        requests_seen: usize,
    },
    /// A single Initiate Multitransport Request has been surfaced and the
    /// connector is waiting for the caller to establish that UDP transport
    /// (RDPEUDP2 + TLS + RDPEMT) or decline it.
    ///
    /// Call [`ClientConnector::complete_multitransport()`] or
    /// [`ClientConnector::skip_multitransport()`] to advance. Either returns
    /// the connector to [`Self::MultitransportBootstrapping`] to read whatever
    /// the server sends next, which may be a second request or the Demand
    /// Active.
    ///
    /// On the wire, TCP and UDP negotiation happen in parallel: the UDP
    /// transport is established alongside the ongoing TCP handshake, and
    /// its completion is a signal to the dynamic-channel layer that
    /// subsequent channels may migrate to UDP. The connector's suspension
    /// here is a Rust-API affordance, not a spec-mandated TCP pause.
    MultitransportPending {
        io_channel_id: u16,
        user_channel_id: u16,
        /// MCS message channel the Initiate Multitransport Response must go out
        /// on; see the same field on [`Self::MultitransportBootstrapping`].
        message_channel_id: Option<u16>,
        /// The request awaiting the caller's outcome.
        request: rdp::multitransport::MultitransportRequestPdu,
        /// Carried through so the cap survives the round trip through the
        /// caller.
        requests_seen: usize,
        /// Whether both peers advertised Soft-Sync, captured when this state was
        /// entered. Combined with the reported outcome this decides whether the
        /// completion and skip paths emit an Initiate Multitransport Response:
        /// a failure is always reported, while success is withheld unless
        /// Soft-Sync was negotiated.
        soft_sync: bool,
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
            Self::MultitransportPending { .. } => "MultitransportPending",
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

#[expect(
    clippy::partial_pub_fields,
    reason = "server response flags are negotiated internally and must not expand the public connector construction API; the connect-time bandwidth accumulators are likewise internal to the measurement, and exposing them would let a caller break the Start/Payload/Stop invariant"
)]
#[derive(Debug)]
pub struct ClientConnector {
    pub config: Config,
    pub state: ClientConnectorState,
    /// The client address to be used in the Client Info PDU.
    pub client_addr: SocketAddr,
    pub static_channels: StaticChannelSet,
    /// MCS message channel ID assigned by the server, once negotiated.
    pub message_channel_id: Option<u16>,
    /// X.224 negotiation flags supplied by the server.
    response_flags: nego::ResponseFlags,
    /// Multitransport flags the server advertised in its GCC
    /// `MultiTransportChannelData` block, if it sent one. Retained because
    /// MS-RDPBCGR 2.2.15.2 permits an `S_OK` response only to a server that
    /// advertised `SOFTSYNC_TCP_TO_UDP`, so the outcome reported back depends on
    /// what both peers advertised.
    pub server_multitransport_flags: Option<gcc::MultiTransportFlags>,
    /// Auto-reconnect cookie from a previous session, when reconnecting.
    ///
    /// Set via [`ClientConnector::with_auto_reconnect_cookie`].
    pub auto_reconnect_cookie: Option<ServerAutoReconnect>,
    /// Start of the in-flight connect-time bandwidth measurement window.
    ///
    /// Set when the server's Bandwidth Measure Start arrives and cleared when the
    /// matching Stop is answered. `None` means no window is open, in which case the
    /// Stop is answered with [`UNMEASURABLE_INTERVAL_MS`] rather than a measurement.
    connect_time_bw_started_at: Option<MonotonicInstant>,
    /// Bytes seen in the open window, accumulated across Payload messages.
    connect_time_bw_bytes: u32,
}

impl ClientConnector {
    pub fn new(config: Config, client_addr: SocketAddr) -> Self {
        Self {
            config,
            state: ClientConnectorState::ConnectionInitiationSendRequest,
            client_addr,
            static_channels: StaticChannelSet::new(),
            message_channel_id: None,
            response_flags: nego::ResponseFlags::empty(),
            server_multitransport_flags: None,
            auto_reconnect_cookie: None,
            connect_time_bw_started_at: None,
            connect_time_bw_bytes: 0,
        }
    }

    /// Attempt to resume a previous session using its auto-reconnect cookie.
    ///
    /// The cookie is handed to the client by the server in a Save Session Info
    /// PDU ([MS-RDPBCGR] 2.2.10.1) and is bound to one session. Supplying it here
    /// makes the connector send the derived Client Auto-Reconnect Packet
    /// ([MS-RDPBCGR] 2.2.4.3) in the Client Info PDU, which lets the server
    /// reattach the session without prompting for credentials again
    /// ([MS-RDPBCGR] 1.3.1.5).
    ///
    /// The server regenerates the cookie whenever a client connects and again at
    /// hourly intervals ([MS-RDPBCGR] 3.3.6.2), so pass the most recent one
    /// received. A stale or absent cookie is not an error: the server falls back
    /// to a normal logon.
    #[must_use]
    pub fn with_auto_reconnect_cookie(mut self, cookie: ServerAutoReconnect) -> Self {
        self.auto_reconnect_cookie = Some(cookie);
        self
    }

    /// Send the initial X.224 request with an explicit security protocol set.
    pub fn initiate_with_security_protocol(
        &mut self,
        security_protocol: nego::SecurityProtocol,
        output: &mut WriteBuf,
    ) -> ConnectorResult<Written> {
        if !matches!(self.state, ClientConnectorState::ConnectionInitiationSendRequest) {
            return Err(reason_err!("Initiation", "connection initiation has already started"));
        }

        let enabled_protocols = self.enabled_security_protocols()?;
        if !enabled_protocols.contains(security_protocol) {
            return Err(reason_err!(
                "Initiation",
                "requested security protocols {security_protocol} are not enabled by connector configuration",
            ));
        }

        self.state = ClientConnectorState::Consumed;
        self.encode_connection_request(security_protocol, output)
    }

    fn enabled_security_protocols(&self) -> ConnectorResult<nego::SecurityProtocol> {
        let mut security_protocol = nego::SecurityProtocol::empty();

        if self.config.enable_tls {
            security_protocol.insert(nego::SecurityProtocol::SSL);
        }
        if self.config.enable_credssp {
            // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/902b090b-9cb3-4efc-92bf-ee13373371e3
            // PROTOCOL_HYBRID "SHOULD" also set PROTOCOL_SSL, but it is not a MUST.
            // IronRDP intentionally omits SSL unless enable_tls is set so the server
            // cannot silently downgrade NLA to TLS-only.
            security_protocol.insert(nego::SecurityProtocol::HYBRID | nego::SecurityProtocol::HYBRID_EX);
        }

        // PROTOCOL_RDP (empty flags) is standard RDP security. IronRDP only supports the
        // ENCRYPTION_LEVEL_NONE variant (no RC4 Security Exchange). Keep it opt-in so
        // `enable_tls = false` + `enable_credssp = false` cannot silently open a plaintext
        // TCP session; local named-pipe transports set `enable_standard_rdp_security`.
        if security_protocol.is_standard_rdp_security() {
            if !self.config.enable_standard_rdp_security {
                return Err(reason_err!("Initiation", "standard RDP security is not supported"));
            }
            debug!("Advertising standard RDP security (PROTOCOL_RDP / no enhanced protocols)");
        }

        Ok(security_protocol)
    }

    fn encode_connection_request(
        &mut self,
        security_protocol: nego::SecurityProtocol,
        output: &mut WriteBuf,
    ) -> ConnectorResult<Written> {
        let connection_request = nego::ConnectionRequest {
            nego_data: self.config.request_data.clone().or_else(|| {
                self.config
                    .credentials
                    .username()
                    .map(|username| nego::NegoRequestData::cookie(username.to_owned()))
            }),
            flags: nego::RequestFlags::empty(),
            protocol: security_protocol,
            correlation_info: None,
        };

        debug!(message = ?connection_request, "Send");

        let written = ironrdp_core::encode_buf(&X224(connection_request), output).map_err(ConnectorError::encode)?;
        self.state = ClientConnectorState::ConnectionInitiationWaitConfirm {
            requested_protocol: security_protocol,
        };
        Written::from_size(written)
    }

    /// Whether Soft-Sync (`SOFTSYNC_TCP_TO_UDP`) was mutually advertised, meaning
    /// both peers set the flag in their GCC `MultiTransportChannelData` block.
    ///
    /// This does not by itself decide whether a response is sent. It gates
    /// success only: per [\[MS-RDPBCGR\] 2.2.15.2] `S_OK` "MUST only be sent to a
    /// server that advertises the SOFTSYNC_TCP_TO_UDP flag", while a failure is
    /// reported either way. See [`Self::multitransport_response_channel`].
    ///
    /// [\[MS-RDPBCGR\] 2.2.15.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/44044233-e498-46f8-8e16-1ffa595a8e8b
    fn soft_sync_negotiated(&self) -> bool {
        fn advertised(flags: Option<gcc::MultiTransportFlags>) -> bool {
            flags.is_some_and(|flags| flags.contains(gcc::MultiTransportFlags::SOFT_SYNC_TCP_TO_UDP))
        }

        advertised(self.config.multitransport_flags) && advertised(self.server_multitransport_flags)
    }

    #[must_use]
    pub fn with_static_channel<T>(mut self, channel: T) -> Self
    where
        T: SvcClientProcessor + 'static,
    {
        self.attach_static_channel(channel);
        self
    }

    pub fn attach_static_channel<T>(&mut self, channel: T)
    where
        T: SvcClientProcessor + 'static,
    {
        let channel_name = channel.channel_name();
        let channel_key = StaticChannelKey::Typed(TypeId::of::<T>());
        if self.static_channels.get_by_type::<T>().is_none() && self.static_channels.len() >= MAX_STATIC_CHANNELS {
            warn!(max_channels = MAX_STATIC_CHANNELS, "Static channel limit reached");
            return;
        }
        if let Some((existing_key, _)) = self.static_channels.get_by_channel_name_key(&channel_name)
            && existing_key != channel_key
        {
            warn!(?channel_name, "Static channel name is already registered");
            return;
        }
        self.static_channels.insert(channel);
    }

    /// Attaches a runtime-defined static virtual channel.
    ///
    /// This permits multiple instances of the same processor type, each with its own negotiated
    /// channel name. `false` means the static-channel limit was reached or the name is already
    /// registered.
    pub fn attach_dynamic_static_channel<T>(&mut self, channel: T) -> bool
    where
        T: SvcClientProcessor + 'static,
    {
        self.try_attach_dynamic_static_channel(channel).is_ok()
    }

    /// Attaches a runtime-defined static virtual channel, returning the reason when it cannot be
    /// registered.
    pub fn try_attach_dynamic_static_channel<T>(&mut self, channel: T) -> Result<(), DynamicStaticChannelAttachError>
    where
        T: SvcClientProcessor + 'static,
    {
        let channel_name = channel.channel_name();
        if self.static_channels.len() >= MAX_STATIC_CHANNELS {
            return Err(DynamicStaticChannelAttachError::ChannelLimitReached);
        }
        if self.static_channels.get_by_channel_name_key(&channel_name).is_some() {
            return Err(DynamicStaticChannelAttachError::DuplicateChannelName);
        }
        self.static_channels
            .insert_dynamic(channel)
            .map(|_| ())
            .ok_or(DynamicStaticChannelAttachError::ChannelLimitReached)
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

    /// Advance past [`ClientConnectorState::EnhancedSecurityUpgrade`] after TLS is already done.
    ///
    /// # Panics
    ///
    /// Panics if state is not [ClientConnectorState::EnhancedSecurityUpgrade].
    pub fn mark_security_upgrade_as_done(&mut self) {
        assert!(self.should_perform_security_upgrade());
        self.step(&[], MonotonicInstant::ZERO, &mut WriteBuf::new())
            .expect("transition to next state");
        debug_assert!(!self.should_perform_security_upgrade());
    }

    pub fn should_perform_credssp(&self) -> bool {
        matches!(self.state, ClientConnectorState::Credssp { .. })
    }

    /// Advance past [`ClientConnectorState::Credssp`] after NLA is already done.
    ///
    /// # Panics
    ///
    /// Panics if state is not [ClientConnectorState::Credssp].
    pub fn mark_credssp_as_done(&mut self) {
        assert!(self.should_perform_credssp());
        let res = self
            .step(&[], MonotonicInstant::ZERO, &mut WriteBuf::new())
            .expect("transition to next state");
        debug_assert!(!self.should_perform_credssp());
        assert_eq!(res, Written::Nothing);
    }

    /// Returns `true` when the server has sent an Initiate Multitransport
    /// Request and the connector is waiting for the application to either
    /// establish that UDP transport or decline it.
    ///
    /// The application should:
    ///
    /// 1. Call [`multitransport_request()`](Self::multitransport_request) to
    ///    get the server's request
    /// 2. Establish UDP transport (RDPEUDP2 + TLS + RDPEMT), or decide not to
    /// 3. Call [`complete_multitransport()`](Self::complete_multitransport) with
    ///    the [`MultitransportResult`], or
    ///    [`skip_multitransport()`](Self::skip_multitransport) to decline
    ///
    /// This can come round more than once. MS-RDPBCGR 2.2.15.1 permits two
    /// requests, one per transport protocol, and each is surfaced on its own
    /// as soon as it decodes rather than batched, because the server is not
    /// obliged to announce that it has finished sending them.
    pub fn should_perform_multitransport(&self) -> bool {
        matches!(self.state, ClientConnectorState::MultitransportPending { .. })
    }

    /// Returns the multitransport request PDU awaiting an outcome.
    ///
    /// `None` unless
    /// [`should_perform_multitransport()`](Self::should_perform_multitransport)
    /// returns `true`.
    pub fn multitransport_request(&self) -> Option<&rdp::multitransport::MultitransportRequestPdu> {
        match &self.state {
            ClientConnectorState::MultitransportPending { request, .. } => Some(request),
            _ => None,
        }
    }

    /// Report the outcome of the multitransport request currently surfaced by
    /// [`multitransport_request()`](Self::multitransport_request).
    ///
    /// The connector builds the response PDU internally from the stored request
    /// ID, sends it on the MCS message channel when one is owed for this outcome,
    /// and returns to reading. A failure is always reported; success is withheld
    /// unless Soft-Sync was negotiated, per MS-RDPBCGR 2.2.15.2. The next PDU may be a second request
    /// or the Demand Active that ends bootstrapping; either way the caller does
    /// not have to know which.
    ///
    /// Returns an error if the connector is not in `MultitransportPending`
    /// state.
    pub fn complete_multitransport(
        &mut self,
        result: MultitransportResult,
        output: &mut WriteBuf,
    ) -> ConnectorResult<Written> {
        self.respond_to_multitransport("complete_multitransport", result, output)
    }

    /// Decline the multitransport request currently surfaced.
    ///
    /// Use this when the application doesn't support or doesn't want UDP
    /// transport. This reports `E_ABORT`, which MS-RDPBCGR 3.2.5.15.1 asks for
    /// whenever the client cannot initiate the sideband channel, with no
    /// Soft-Sync condition attached; the server then continues TCP-only.
    ///
    /// Returns an error if the connector is not in `MultitransportPending`
    /// state.
    pub fn skip_multitransport(&mut self, output: &mut WriteBuf) -> ConnectorResult<Written> {
        self.respond_to_multitransport(
            "skip_multitransport",
            MultitransportResult::Failure(rdp::multitransport::MultitransportResponsePdu::E_ABORT),
            output,
        )
    }

    /// The channel an Initiate Multitransport Response goes out on, or `None`
    /// when no response is owed.
    ///
    /// Which of the two applies is decided by the outcome as well as the mode.
    /// MS-RDPBCGR 3.2.5.15.1 asks for a response whenever the client could not
    /// initiate the sideband channel, with no Soft-Sync condition, and requires
    /// one either way once Soft-Sync is negotiated. 2.2.15.2 restricts only
    /// `S_OK`, which "MUST only be sent to a server that advertises the
    /// SOFTSYNC_TCP_TO_UDP flag". So a failure is reported whenever there is a
    /// channel to report it on, and only success is withheld.
    ///
    /// Under Soft-Sync the message channel is presupposed, and falling back to
    /// the I/O channel would put the response somewhere the server is not
    /// reading, so its absence is an error rather than a reason to improvise.
    ///
    /// Borrows rather than consuming, so the caller can resolve this while the
    /// connector is still in a state it can act on.
    fn multitransport_response_channel(&self, result: &MultitransportResult) -> ConnectorResult<Option<u16>> {
        let ClientConnectorState::MultitransportPending {
            message_channel_id,
            soft_sync,
            ..
        } = &self.state
        else {
            return Ok(None);
        };

        match (*soft_sync, result) {
            // Soft-Sync obliges a response either way, and presupposes the
            // message channel. Falling back to the I/O channel would put the
            // response somewhere the server is not reading, so its absence is an
            // error rather than a reason to improvise.
            (true, _) => message_channel_id
                .ok_or_else(|| {
                    general_err!("Soft-Sync was negotiated but the server never offered an MCS message channel")
                })
                .map(Some),
            // `S_OK` is the one value 2.2.15.2 forbids here, so success and only
            // success is withheld. The outcome is still consumed and the
            // handshake proceeds, so a caller that established the transport does
            // not have to know which mode is in play.
            (false, MultitransportResult::Success) => Ok(None),
            // A failure is still reported: 3.2.5.15.1 asks for it whenever the
            // client could not initiate the channel, with no Soft-Sync condition.
            // It is a SHOULD, so a missing message channel means staying silent
            // rather than failing a connection over an optional report. In
            // practice one exists, since 2.2.15.1 puts the request on it.
            (false, MultitransportResult::Failure(_)) => Ok(*message_channel_id),
        }
    }

    /// Shared body of [`Self::complete_multitransport`] and
    /// [`Self::skip_multitransport`]: declining is just an `E_ABORT` outcome,
    /// so the two differ only in the HRESULT they report.
    fn respond_to_multitransport(
        &mut self,
        caller: &str,
        result: MultitransportResult,
        output: &mut WriteBuf,
    ) -> ConnectorResult<Written> {
        // The state is read, never taken. Everything this needs is a scalar, so
        // nothing has to be moved out of `self.state`, and the transition at the
        // end is the only mutation. That makes state preservation structural: an
        // error anywhere above it leaves the connector exactly as it was, so the
        // caller can still see what failed and decline. Taking the state up front
        // and failing afterwards would leave it `Consumed`, with the error having
        // destroyed the state needed to act on it.
        let ClientConnectorState::MultitransportPending {
            io_channel_id,
            user_channel_id,
            message_channel_id,
            request,
            requests_seen,
            ..
        } = &self.state
        else {
            return Err(reason_err!(
                "MultitransportPending",
                "{caller} called outside MultitransportPending state",
            ));
        };
        let (io_channel_id, user_channel_id, message_channel_id, requests_seen) =
            (*io_channel_id, *user_channel_id, *message_channel_id, *requests_seen);
        let request_id = request.request_id;

        let response_channel = self.multitransport_response_channel(&result)?;

        // Whether a response is owed at all is decided by
        // `multitransport_response_channel`, which weighs the outcome against
        // Soft-Sync. Either way the outcome is consumed and the handshake
        // proceeds.
        let total_written = if let Some(response_channel) = response_channel {
            let response = match result {
                MultitransportResult::Success => rdp::multitransport::MultitransportResponsePdu::success(request_id),
                MultitransportResult::Failure(hr) => multitransport_response(request_id, hr),
            };

            encode_send_data_request(user_channel_id, response_channel, &response, output)?
        } else {
            0
        };

        // Back to reading. The server may send another request or the Demand
        // Active; nothing here needs to predict which.
        self.state = ClientConnectorState::MultitransportBootstrapping {
            io_channel_id,
            user_channel_id,
            message_channel_id,
            requests_seen,
        };

        // Nothing goes on the wire when no response is owed, and `from_size`
        // rejects a zero length.
        if total_written == 0 {
            Ok(Written::Nothing)
        } else {
            Written::from_size(total_written)
        }
    }

    fn respond_to_connect_time_autodetect(
        &mut self,
        request: rdp::autodetect::AutoDetectRequest,
        received_at: MonotonicInstant,
        message_channel_id: u16,
        user_channel_id: u16,
        output: &mut WriteBuf,
    ) -> ConnectorResult<Written> {
        use ironrdp_pdu::rdp::autodetect::{
            AutoDetectRequest, AutoDetectResponse, AutoDetectRspPdu, BW_RESULTS_CONNECT_TIME,
        };

        match request {
            AutoDetectRequest::RttRequest { sequence_number, .. } => {
                let response = AutoDetectRspPdu::new(AutoDetectResponse::RttResponse { sequence_number });
                let written = encode_send_data_request(user_channel_id, message_channel_id, &response, output)?;
                Written::from_size(written)
            }
            // Start opens the measurement window ([MS-RDPBCGR] 2.2.14.1.2). No reply is
            // due; we only note when it arrived.
            AutoDetectRequest::BandwidthMeasureStart { .. } => {
                self.connect_time_bw_started_at = Some(received_at);
                self.connect_time_bw_bytes = 0;
                Ok(Written::Nothing)
            }
            // Payload carries the bytes whose transfer is being timed ([MS-RDPBCGR]
            // 2.2.14.1.3). No reply is due; accumulate so Stop can report the total.
            AutoDetectRequest::BandwidthMeasurePayload { payload, .. } => {
                let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
                self.connect_time_bw_bytes = self.connect_time_bw_bytes.saturating_add(len);
                Ok(Written::Nothing)
            }
            // A connect-time Bandwidth Measure Stop ([MS-RDPBCGR] 2.2.14.1.4) warrants a
            // Bandwidth Measure Results reply ([MS-RDPBCGR] 2.2.14.2.2). This reply must
            // be sent: FreeRDP-based servers (for example GNOME Remote Desktop) block in
            // their AWAIT_BW_RESULT state until they receive it and never proceed to
            // licensing without it, so omitting it stalls the whole connection.
            //
            // The interval is measured from the Start that opened the window to the
            // arrival of this Stop, using the times the I/O driver observed rather than
            // any clock this sequence could read for itself.
            //
            // Two cases yield no interval to report. Either no Start was seen, or the
            // whole Start/Payload/Stop sequence arrived in a single socket read, which
            // is the common case on a fast link because the payload is small enough to
            // coalesce. Both are unmeasurable at read granularity rather than
            // instantaneous, and the distinction matters on the wire: `timeDelta` of 0
            // divides out to an unbounded bandwidth for a server that computes
            // `byteCount * 8 / timeDelta`. Report the floor instead, which is the
            // slowest rate consistent with what was observed.
            AutoDetectRequest::BandwidthMeasureStop {
                sequence_number,
                payload,
                ..
            } => {
                let stop_bytes = payload
                    .as_ref()
                    .map_or(0, |p| u32::try_from(p.len()).unwrap_or(u32::MAX));
                let byte_count = self.connect_time_bw_bytes.saturating_add(stop_bytes);

                let measured_ms = self
                    .connect_time_bw_started_at
                    .map(|started_at| {
                        u32::try_from(received_at.duration_since(started_at).as_millis()).unwrap_or(u32::MAX)
                    })
                    .unwrap_or(0);
                let time_delta_ms = measured_ms.max(UNMEASURABLE_INTERVAL_MS);

                self.connect_time_bw_started_at = None;
                self.connect_time_bw_bytes = 0;

                let response = AutoDetectRspPdu::new(AutoDetectResponse::BandwidthMeasureResults {
                    sequence_number,
                    response_type: BW_RESULTS_CONNECT_TIME,
                    time_delta_ms,
                    byte_count,
                });
                let written = encode_send_data_request(user_channel_id, message_channel_id, &response, output)?;
                Written::from_size(written)
            }
            // The Network Characteristics Result is informational; nothing to send.
            _ => Ok(Written::Nothing),
        }
    }
}

/// Build an Initiate Multitransport Response carrying `hr_response`.
///
/// [`MultitransportResponsePdu::success`] covers `S_OK`; every other HRESULT,
/// whether a caller-supplied failure or the `E_ABORT` the skip path sends,
/// comes through here so the security-header framing lives in one place.
fn multitransport_response(request_id: u32, hr_response: u32) -> rdp::multitransport::MultitransportResponsePdu {
    rdp::multitransport::MultitransportResponsePdu {
        security_header: rdp::headers::BasicSecurityHeader {
            flags: rdp::headers::BasicSecurityHeaderFlags::TRANSPORT_RSP,
        },
        request_id,
        hr_response,
    }
}

fn advance_licensing_exchange(
    mut license_exchange: LicenseExchangeSequence,
    io_channel_id: u16,
    user_channel_id: u16,
    message_channel_id: Option<u16>,
    input: &[u8],
    received_at: MonotonicInstant,
    output: &mut WriteBuf,
) -> ConnectorResult<(Written, ClientConnectorState)> {
    let written = license_exchange.step(input, received_at, output)?;

    let next_state = if license_exchange.state.is_terminal() {
        ClientConnectorState::MultitransportBootstrapping {
            io_channel_id,
            user_channel_id,
            message_channel_id,
            requests_seen: 0,
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
            ClientConnectorState::MultitransportBootstrapping { .. } => Some(&ironrdp_pdu::X224_HINT),
            ClientConnectorState::MultitransportPending { .. } => None,
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

    fn step(&mut self, input: &[u8], received_at: MonotonicInstant, output: &mut WriteBuf) -> ConnectorResult<Written> {
        let (written, next_state) = match mem::take(&mut self.state) {
            // Invalid state
            ClientConnectorState::Consumed => {
                return Err(general_err!("connector sequence state is consumed (this is a bug)",));
            }

            //== Connection Initiation ==//
            // Exchange supported security protocols and a few other connection flags.
            ClientConnectorState::ConnectionInitiationSendRequest => {
                debug!("Connection Initiation");
                let security_protocol = self.enabled_security_protocols()?;
                let written = self.encode_connection_request(security_protocol, output)?;
                (written, mem::take(&mut self.state))
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

                // PROTOCOL_RDP is encoded as an empty bitset. `intersects` is false for two empty
                // sets, so treat standard RDP security as a special case.
                let protocol_ok = if selected_protocol.is_standard_rdp_security() {
                    requested_protocol.is_standard_rdp_security()
                } else {
                    selected_protocol.intersects(requested_protocol)
                };
                if !protocol_ok {
                    return Err(reason_err!(
                        "Initiation",
                        "client advertised {requested_protocol}, but server selected {selected_protocol}",
                    ));
                }

                self.response_flags = flags;

                (
                    Written::Nothing,
                    ClientConnectorState::EnhancedSecurityUpgrade { selected_protocol },
                )
            }

            //== Upgrade to Enhanced RDP Security ==//
            // When PROTOCOL_RDP is selected there is no TLS/CredSSP front-end: the caller should
            // still invoke mark_security_upgrade_as_done() (a no-op upgrade) before continuing.
            // When SSL/HYBRID is selected, user code must perform the TLS handshake first.
            ClientConnectorState::EnhancedSecurityUpgrade { selected_protocol } => {
                let next_state = if selected_protocol.is_standard_rdp_security() {
                    debug!("Standard RDP security selected; skipping TLS and CredSSP");
                    ClientConnectorState::BasicSettingsExchangeSendInitial { selected_protocol }
                } else if selected_protocol
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

                let client_gcc_blocks = create_gcc_blocks(
                    &self.config,
                    selected_protocol,
                    self.response_flags
                        .contains(nego::ResponseFlags::EXTENDED_CLIENT_DATA_SUPPORTED),
                    self.static_channels.values(),
                )?;

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

                self.server_multitransport_flags = server_gcc_blocks
                    .multi_transport_channel
                    .as_ref()
                    .map(|data| data.flags);

                let static_channel_ids = server_gcc_blocks.network.channel_ids;
                let io_channel_id = server_gcc_blocks.network.io_channel;

                debug!(?static_channel_ids, io_channel_id);

                let zipped: Vec<_> = self
                    .static_channels
                    .keys()
                    .zip(static_channel_ids.iter().copied())
                    .collect();

                zipped.into_iter().for_each(|(channel, channel_id)| {
                    self.static_channels.attach_channel_id_by_key(channel, channel_id);
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
                let written = channel_connection.step(input, received_at, output)?;

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
            // With standard RDP security and ENCRYPTION_LEVEL_NONE, no Security Exchange PDU is
            // sent (MS-RDPBCGR 5.3.2). IronRDP only supports that no-encryption path; RC4/FIPS
            // would require a Security Exchange here and is rejected earlier when the server's
            // SC_SECURITY block is not ServerSecurityData::no_security().
            //==============================//

            //== Secure Settings Exchange ==//
            // Send Client Info PDU (information about supported types of compression, username, password, etc).
            ClientConnectorState::SecureSettingsExchange {
                io_channel_id,
                user_channel_id,
            } => {
                debug!("Secure Settings Exchange");

                let client_info =
                    create_client_info_pdu(&self.config, &self.client_addr, self.auto_reconnect_cookie.as_ref());

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
                        let written = self.respond_to_connect_time_autodetect(
                            autodetect.request,
                            received_at,
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
                        advance_licensing_exchange(
                            license_exchange,
                            io_channel_id,
                            user_channel_id,
                            self.message_channel_id,
                            input,
                            received_at,
                            output,
                        )?
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

                advance_licensing_exchange(
                    license_exchange,
                    io_channel_id,
                    user_channel_id,
                    self.message_channel_id,
                    input,
                    received_at,
                    output,
                )?
            }

            //== Optional Multitransport Bootstrapping ==//
            //
            // After licensing the server may send 0, 1, or 2 Initiate Multitransport
            // Request PDUs (MS-RDPBCGR 2.2.15.1), and it is under no obligation to
            // send the Demand Active before the client acts on one it has already
            // sent. There is therefore no end marker for the set, and waiting for a
            // following PDU to decide what to do with the current one can stall the
            // handshake outright. Each request is surfaced the moment it decodes,
            // per MS-RDPBCGR 3.2.5.15.1.
            //
            // Routing is by channel. Requests travel on the MCS message channel; the
            // Demand Active is on the I/O channel and ends the phase. The message
            // channel also carries auto-detect PDUs, so a decode still confirms what
            // arrived there, but the I/O channel is never speculatively decoded as
            // multitransport any more.
            ClientConnectorState::MultitransportBootstrapping {
                io_channel_id,
                user_channel_id,
                message_channel_id,
                requests_seen,
            } => {
                let ctx = mcs::decode_send_data_indication(input).map_err(ConnectorError::decode)?;

                if Some(ctx.channel_id) == message_channel_id {
                    let pdu = decode::<rdp::multitransport::MultitransportRequestPdu>(ctx.user_data)
                        .map_err(ConnectorError::decode)?;

                    if requests_seen >= MAX_MULTITRANSPORT_REQUESTS {
                        return Err(reason_err!(
                            "MultitransportBootstrapping",
                            "server sent more than {} multitransport requests (MS-RDPBCGR 2.2.15.1 caps the count at {})",
                            MAX_MULTITRANSPORT_REQUESTS,
                            MAX_MULTITRANSPORT_REQUESTS,
                        ));
                    }

                    debug!(
                        request_id = pdu.request_id,
                        protocol = ?pdu.requested_protocol,
                        "Received Initiate Multitransport Request"
                    );

                    // Captured on entry rather than read back at completion time: the
                    // GCC exchange carrying both peers' flags is long finished, and
                    // freezing it here keeps the response paths from depending on
                    // connector fields that could move in between.
                    let soft_sync = self.soft_sync_negotiated();

                    (
                        Written::Nothing,
                        ClientConnectorState::MultitransportPending {
                            io_channel_id,
                            user_channel_id,
                            message_channel_id,
                            request: pdu,
                            requests_seen: requests_seen + 1,
                            soft_sync,
                        },
                    )
                } else if ctx.channel_id == io_channel_id {
                    // Demand Active: bootstrapping is over, hand off to capabilities
                    // exchange with the PDU intact.
                    let mut connection_activation =
                        ConnectionActivationSequence::new(self.config.clone(), io_channel_id, user_channel_id);
                    let written = connection_activation.step(input, received_at, output)?;

                    (
                        written,
                        match connection_activation.connection_activation_state() {
                            ConnectionActivationState::ConnectionFinalization { .. } => {
                                ClientConnectorState::ConnectionFinalization { connection_activation }
                            }
                            _ => ClientConnectorState::CapabilitiesExchange { connection_activation },
                        },
                    )
                } else {
                    return Err(reason_err!(
                        "MultitransportBootstrapping",
                        "PDU on unexpected channel {} (message channel is {}, I/O channel is {})",
                        ctx.channel_id,
                        message_channel_id
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| "not negotiated".to_owned()),
                        io_channel_id,
                    ));
                }
            }

            // MultitransportPending: application should call complete_multitransport()
            // or skip_multitransport() instead of step()
            ClientConnectorState::MultitransportPending { .. } => {
                return Err(general_err!(
                    "multitransport pending: call complete_multitransport() or skip_multitransport()"
                ));
            }

            //== Capabilities Exchange ==/
            // The server sends the set of capabilities it supports to the client.
            ClientConnectorState::CapabilitiesExchange {
                mut connection_activation,
            } => {
                let written = connection_activation.step(input, received_at, output)?;
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
                let written = connection_activation.step(input, received_at, output)?;

                let next_state = if !connection_activation.connection_activation_state().is_terminal() {
                    ClientConnectorState::ConnectionFinalization { connection_activation }
                } else {
                    match connection_activation.connection_activation_state() {
                        ConnectionActivationState::Finalized {
                            desktop_size,
                            share_id,
                            input_flags,
                            static_channel_chunk_size,
                            enable_server_pointer,
                            pointer_software_rendering,
                            refresh_rect_support,
                            suppress_output_support,
                            window_support_level,
                        } => {
                            let mut static_channels = mem::take(&mut self.static_channels);
                            if !static_channels.set_maximum_chunk_size(static_channel_chunk_size) {
                                return Err(general_err!("invalid static channel chunk size"));
                            }

                            ClientConnectorState::Connected {
                                result: ConnectionResult {
                                    io_channel_id: connection_activation.io_channel_id(),
                                    user_channel_id: connection_activation.user_channel_id(),
                                    message_channel_id: self.message_channel_id,
                                    share_id,
                                    static_channels,
                                    desktop_size,
                                    input_flags,
                                    enable_server_pointer,
                                    pointer_software_rendering,
                                    refresh_rect_support,
                                    suppress_output_support,
                                    window_support_level,
                                    monitor_layout: connection_activation.monitor_layout(),
                                    activation_factory: ConnectionActivationFactory::new(
                                        self.config.clone(),
                                        connection_activation.io_channel_id(),
                                        connection_activation.user_channel_id(),
                                    ),
                                    compression_type: self.config.compression_type,
                                },
                            }
                        }
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
    extended_client_data_supported: bool,
    static_channels: impl Iterator<Item = &'a StaticVirtualChannel>,
) -> ConnectorResult<gcc::ClientGccBlocks> {
    use ironrdp_pdu::gcc::{
        ClientCoreData, ClientCoreOptionalData, ClientEarlyCapabilityFlags, ClientGccBlocks, ClientNetworkData,
        ClientSecurityData, ColorDepth, EncryptionMethod, HighColorDepth, MonitorOrientation, RdpVersion,
        SecureAccessSequence, SupportedColorDepths,
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
                    if extended_client_data_supported {
                        early_capability_flags |= ClientEarlyCapabilityFlags::SUPPORT_MONITOR_LAYOUT_PDU;
                    }

                    Some(early_capability_flags)
                },
                dig_product_id: Some(config.dig_product_id.clone()),
                connection_type: Some(config.connection_type),
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
        monitor: extended_client_data_supported
            .then(|| config.monitor_layout.clone())
            .flatten(),
        // Request the MCS message channel, which carries network auto-detect
        // ([MS-RDPBCGR] 2.2.14) and the multitransport / heartbeat PDUs. The
        // server assigns its ID in Server Message Channel Data.
        message_channel: extended_client_data_supported.then_some(gcc::ClientMessageChannelData),
        multi_transport_channel: extended_client_data_supported
            .then(|| {
                config
                    .multitransport_flags
                    .map(|flags| gcc::MultiTransportChannelData { flags })
            })
            .flatten(),
        monitor_extended: None,
    })
}

fn create_client_info_pdu(
    config: &Config,
    client_addr: &SocketAddr,
    auto_reconnect_cookie: Option<&ServerAutoReconnect>,
) -> rdp::ClientInfoPdu {
    use ironrdp_pdu::rdp::ClientInfoPdu;
    use ironrdp_pdu::rdp::client_info::{
        AddressFamily, ClientAutoReconnect, ClientInfo, ClientInfoFlags, CompressionType, Credentials,
        ExtendedClientInfo, ExtendedClientOptionalInfo,
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

    if config.enable_audio_capture {
        flags |= ClientInfoFlags::AUDIO_CAPTURE;
    }

    if config.remote_application_mode {
        flags |= ClientInfoFlags::RAIL;
    }

    // Advertise bulk compression support if configured
    let compression_type = if let Some(ct) = config.compression_type {
        flags |= ClientInfoFlags::COMPRESSION;
        info!(compression_type = ?ct, "Advertising bulk compression in Client Info PDU");
        ct
    } else {
        CompressionType::K8 // ignored if ClientInfoFlags::COMPRESSION is not set
    };

    // MS-RDPERP requires RemoteApp launch data on the RAIL channel.
    let (alternate_shell, work_dir) = if config.remote_application_mode {
        (String::new(), String::new())
    } else {
        (config.alternate_shell.clone(), config.work_dir.clone())
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
        alternate_shell,
        work_dir,
        extra_info: ExtendedClientInfo {
            address_family: match client_addr {
                SocketAddr::V4(_) => AddressFamily::INET,
                SocketAddr::V6(_) => AddressFamily::INET_6,
            },
            address: client_addr.ip().to_string(),
            dir: config.client_dir.clone(),
            optional_data: {
                let builder = ExtendedClientOptionalInfo::builder()
                    .timezone(config.timezone_info.clone())
                    .session_id(0)
                    .performance_flags(config.performance_flags);

                // Resuming a session: prove we held the cookie the server issued
                // for it ([MS-RDPBCGR] 2.2.4.3, derived per 5.5). Absent on a
                // fresh connection, which is an ordinary logon.
                match auto_reconnect_cookie {
                    Some(cookie) => builder
                        .reconnect_cookie(ClientAutoReconnect::from_server_cookie(cookie).to_bytes())
                        .build(),
                    None => builder.build(),
                }
            },
        },
    };

    ClientInfoPdu {
        security_header,
        client_info,
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_pdu::rdp::capability_sets::{MajorPlatformType, RailSupportLevel};
    use ironrdp_pdu::rdp::client_info::ClientInfoFlags;
    use ironrdp_pdu::{gcc, nego};

    use super::{create_client_info_pdu, create_gcc_blocks};
    use crate::{Config, Credentials, DesktopSize};

    #[test]
    fn remote_application_client_info_uses_rail_launch_data() {
        let config = Config {
            desktop_size: DesktopSize {
                width: 1024,
                height: 768,
            },
            monitor_layout: None,
            desktop_scale_factor: 0,
            enable_tls: true,
            enable_credssp: false,
            enable_standard_rdp_security: false,
            credentials: Credentials::UsernamePassword {
                username: "test".into(),
                password: "test".into(),
            },
            domain: None,
            client_build: 0,
            client_name: "test".into(),
            keyboard_type: gcc::KeyboardType::IBM_ENHANCED,
            keyboard_subtype: 0,
            keyboard_functional_keys_count: 12,
            keyboard_layout: 0,
            connection_type: gcc::ConnectionType::Lan,
            ime_file_name: String::new(),
            bitmap: None,
            dig_product_id: String::new(),
            client_dir: String::new(),
            alternate_shell: "app.exe".into(),
            work_dir: "C:\\apps".into(),
            remote_application_mode: true,
            rail_support_level: RailSupportLevel::SUPPORTED,
            platform: MajorPlatformType::UNIX,
            hardware_id: None,
            request_data: None,
            autologon: false,
            enable_audio_playback: false,
            enable_audio_capture: false,
            performance_flags: Default::default(),
            license_cache: None,
            timezone_info: Default::default(),
            compression_type: None,
            enable_server_pointer: false,
            pointer_software_rendering: false,
            multitransport_flags: None,
        };

        let client_info = create_client_info_pdu(&config, &"127.0.0.1:3389".parse().unwrap(), None).client_info;

        assert!(client_info.flags.contains(ClientInfoFlags::RAIL));
        assert!(client_info.alternate_shell.is_empty());
        assert!(client_info.work_dir.is_empty());
        assert!(!client_info.flags.contains(ClientInfoFlags::AUDIO_CAPTURE));
    }

    #[test]
    fn audio_capture_flag_is_set_when_enabled() {
        let mut config = Config {
            desktop_size: DesktopSize {
                width: 1024,
                height: 768,
            },
            monitor_layout: None,
            desktop_scale_factor: 0,
            enable_tls: true,
            enable_credssp: false,
            enable_standard_rdp_security: false,
            credentials: Credentials::UsernamePassword {
                username: "test".into(),
                password: "test".into(),
            },
            domain: None,
            client_build: 0,
            client_name: "test".into(),
            keyboard_type: gcc::KeyboardType::IBM_ENHANCED,
            keyboard_subtype: 0,
            keyboard_functional_keys_count: 12,
            keyboard_layout: 0,
            connection_type: gcc::ConnectionType::Lan,
            ime_file_name: String::new(),
            bitmap: None,
            dig_product_id: String::new(),
            client_dir: String::new(),
            alternate_shell: String::new(),
            work_dir: String::new(),
            remote_application_mode: false,
            rail_support_level: RailSupportLevel::empty(),
            platform: MajorPlatformType::UNIX,
            hardware_id: None,
            request_data: None,
            autologon: false,
            enable_audio_playback: true,
            enable_audio_capture: true,
            performance_flags: Default::default(),
            license_cache: None,
            timezone_info: Default::default(),
            compression_type: None,
            enable_server_pointer: false,
            pointer_software_rendering: false,
            multitransport_flags: None,
        };

        let client_info = create_client_info_pdu(&config, &"127.0.0.1:3389".parse().unwrap(), None).client_info;
        assert!(client_info.flags.contains(ClientInfoFlags::AUDIO_CAPTURE));
        assert!(!client_info.flags.contains(ClientInfoFlags::NO_AUDIO_PLAYBACK));

        config.enable_audio_capture = false;
        let client_info = create_client_info_pdu(&config, &"127.0.0.1:3389".parse().unwrap(), None).client_info;
        assert!(!client_info.flags.contains(ClientInfoFlags::AUDIO_CAPTURE));
    }

    #[test]
    fn gcc_blocks_advertise_monitor_layout_when_supported() {
        let mut config = Config {
            desktop_size: DesktopSize {
                width: 1_920,
                height: 1_080,
            },
            monitor_layout: Some(gcc::ClientMonitorData {
                monitors: vec![gcc::Monitor {
                    left: 0,
                    top: 0,
                    right: 1_919,
                    bottom: 1_079,
                    flags: gcc::MonitorFlags::PRIMARY,
                }],
            }),
            desktop_scale_factor: 0,
            enable_tls: true,
            enable_credssp: false,
            enable_standard_rdp_security: false,
            credentials: Credentials::UsernamePassword {
                username: "test".into(),
                password: "test".into(),
            },
            domain: None,
            client_build: 0,
            client_name: "test".into(),
            keyboard_type: gcc::KeyboardType::IBM_ENHANCED,
            keyboard_subtype: 0,
            keyboard_functional_keys_count: 12,
            keyboard_layout: 0,
            connection_type: gcc::ConnectionType::Lan,
            ime_file_name: String::new(),
            bitmap: None,
            dig_product_id: String::new(),
            client_dir: String::new(),
            alternate_shell: String::new(),
            work_dir: String::new(),
            remote_application_mode: false,
            rail_support_level: RailSupportLevel::empty(),
            platform: MajorPlatformType::UNIX,
            hardware_id: None,
            request_data: None,
            autologon: false,
            enable_audio_playback: false,
            enable_audio_capture: false,
            performance_flags: Default::default(),
            license_cache: None,
            timezone_info: Default::default(),
            compression_type: None,
            enable_server_pointer: false,
            pointer_software_rendering: false,
            multitransport_flags: None,
        };

        let blocks = create_gcc_blocks(&config, nego::SecurityProtocol::empty(), true, core::iter::empty())
            .expect("valid GCC Client Monitor Data");

        assert_eq!(blocks.monitor, config.monitor_layout);
        assert!(
            blocks
                .core
                .optional_data
                .early_capability_flags
                .expect("early capability flags are present")
                .contains(gcc::ClientEarlyCapabilityFlags::SUPPORT_MONITOR_LAYOUT_PDU)
        );

        config.monitor_layout = None;
        let blocks = create_gcc_blocks(&config, nego::SecurityProtocol::empty(), true, core::iter::empty())
            .expect("valid GCC Client Monitor Data");

        assert!(blocks.monitor.is_none());
        assert!(
            blocks
                .core
                .optional_data
                .early_capability_flags
                .expect("early capability flags are present")
                .contains(gcc::ClientEarlyCapabilityFlags::SUPPORT_MONITOR_LAYOUT_PDU)
        );

        let blocks = create_gcc_blocks(&config, nego::SecurityProtocol::empty(), false, core::iter::empty())
            .expect("valid GCC Client Monitor Data");

        assert!(blocks.monitor.is_none());
        assert!(blocks.message_channel.is_none());
        assert!(blocks.multi_transport_channel.is_none());
        assert!(
            !blocks
                .core
                .optional_data
                .early_capability_flags
                .expect("early capability flags are present")
                .contains(gcc::ClientEarlyCapabilityFlags::SUPPORT_MONITOR_LAYOUT_PDU)
        );
    }
}
