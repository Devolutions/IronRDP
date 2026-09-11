use core::any::TypeId;
use core::mem;

use ironrdp_connector::{
    ConnectorError, ConnectorErrorExt as _, ConnectorResult, DesktopSize, MonotonicInstant, Sequence, State, Written,
    encode_x224_packet, general_err, reason_err,
};
use ironrdp_core::{WriteBuf, decode};
use ironrdp_pdu as pdu;
use ironrdp_pdu::nego::SecurityProtocol;
use ironrdp_pdu::x224::X224;
use ironrdp_svc::{MAX_STATIC_CHANNELS, StaticChannelKey, StaticChannelSet, SvcServerProcessor};
use pdu::rdp::capability_sets::CapabilitySet;
use pdu::rdp::client_info::{ClientAutoReconnect, Credentials};
use pdu::rdp::headers::ShareControlPdu;
use pdu::rdp::server_error_info::{ErrorInfo, ProtocolIndependentCode, ServerSetErrorInfoPdu};
use pdu::rdp::server_license::{LicensePdu, LicensingErrorMessage};
use pdu::{gcc, mcs, nego, rdp};
use rand::RngCore as _;
use tracing::{debug, warn};

use super::channel_connection::ChannelConnectionSequence;
use super::finalization::FinalizationSequence;
use crate::util::{self, wrap_share_data};

const IO_CHANNEL_ID: u16 = 1003;
const USER_CHANNEL_ID: u16 = 1002;

pub struct Acceptor {
    pub(crate) state: AcceptorState,
    security: SecurityProtocol,
    io_channel_id: u16,
    user_channel_id: u16,
    message_channel_id: Option<u16>,
    desktop_size: DesktopSize,
    keyboard_layout: u32,
    keyboard_type: gcc::KeyboardType,
    ime_file_name: String,
    multitransport_flags: gcc::MultiTransportFlags,
    /// Whether the client sent a Client MultiTransportChannelData block at all
    /// (MS-RDPBCGR 2.2.1.3.8), independent of what flags it carried. The
    /// server's own block MUST be omitted when the client did not populate
    /// this field (2.2.1.4), which `multitransport_flags` alone can't express
    /// since it collapses "absent" and "present but empty" together.
    client_offered_multitransport: bool,
    early_capability_flags: gcc::ClientEarlyCapabilityFlags,
    server_capabilities: Vec<CapabilitySet>,
    static_channels: StaticChannelSet,
    saved_for_reactivation: AcceptorState,
    pub(crate) creds: Option<Credentials>,
    received_credentials: Option<Credentials>,
    received_auto_reconnect: Option<ClientAutoReconnect>,
    reactivation: bool,
    honor_client_desktop_size: Option<DesktopSize>,
    /// UDP multitransport flags to advertise to the client and, if it
    /// reciprocates, to offer. `None` disables the feature entirely: no
    /// Server MultiTransportChannelData block is sent, and no Initiate
    /// Multitransport Request follows. See `set_multitransport_offer()`.
    offer_multitransport: Option<gcc::MultiTransportFlags>,
    /// The Initiate Multitransport Request sent to the client, once
    /// `MultitransportBootstrapping` has run. See `multitransport_request()`.
    sent_multitransport_request: Option<rdp::multitransport::MultitransportRequestPdu>,
}

/// Minimum and maximum desktop dimension honored from a client.
///
/// A desktop dimension in RDP is a `u16`; [MS-RDPBCGR] caps it at 8192, and
/// 200 is a conservative floor. A client-requested dimension outside this
/// range is not honored: the acceptor keeps the server-provided desktop size
/// rather than treating the request as an error.
const MIN_DESKTOP_DIM: u16 = 200;
const MAX_DESKTOP_DIM: u16 = 8192;

/// Returns the client-requested desktop size if both dimensions are within the
/// protocol-legal range, otherwise `None`.
fn validate_desktop_size(width: u16, height: u16) -> Option<DesktopSize> {
    if (MIN_DESKTOP_DIM..=MAX_DESKTOP_DIM).contains(&width) && (MIN_DESKTOP_DIM..=MAX_DESKTOP_DIM).contains(&height) {
        Some(DesktopSize { width, height })
    } else {
        None
    }
}

/// Writes `size` into every Bitmap capability set in `capabilities`.
///
/// The server advertises its desktop size in the Bitmap capability set of the
/// Demand Active PDU; this keeps that advertisement in sync with `size`.
fn set_bitmap_desktop_size(capabilities: &mut [CapabilitySet], size: DesktopSize) {
    for cap in capabilities.iter_mut() {
        if let CapabilitySet::Bitmap(cap) = cap {
            cap.desktop_width = size.width;
            cap.desktop_height = size.height;
        }
    }
}

#[derive(Debug)]
pub struct AcceptorResult {
    pub static_channels: StaticChannelSet,
    pub capabilities: Vec<CapabilitySet>,
    pub input_events: Vec<Vec<u8>>,
    pub user_channel_id: u16,
    pub io_channel_id: u16,
    /// MCS channel ID of the message channel, present when the client requested
    /// one via Client Message Channel Data (section 2.2.1.3.7).
    ///
    /// Server-initiated PDUs that ride the message channel (network auto-detect
    /// per section 2.2.14, multitransport bootstrap, heartbeat) are sent on this
    /// channel. `None` when the client did not request it.
    pub message_channel_id: Option<u16>,
    pub reactivation: bool,
    /// Keyboard layout identifier (KLID) announced by the client in its GCC
    /// Client Core Data (section 2.2.1.3.2, `keyboardLayout`).
    ///
    /// This is the low word of a Windows locale identifier (e.g. `0x0000_0409`
    /// for US English, `0x0000_040C` for French). `0` when the client did not
    /// announce one. Servers can use it to pick a server-side keyboard layout
    /// matching the client without changing any local input state.
    pub keyboard_layout: u32,
    /// Keyboard type announced by the client in its GCC Client Core Data
    /// (section 2.2.1.3.2, `keyboardType`).
    ///
    /// `KeyboardType(0)` when the client did not announce one (0 is not among the
    /// documented values, so it doubles as "unset" the same way `keyboard_layout`'s
    /// `0` does).
    pub keyboard_type: gcc::KeyboardType,
    /// Input method editor file name announced by the client in its GCC Client Core
    /// Data (section 2.2.1.3.2, `imeFileName`).
    ///
    /// Populated for East Asian IME-based input locales; empty otherwise.
    pub ime_file_name: String,
    /// Early capability flags announced by the client in its GCC Client Core
    /// Data (section 2.2.1.3.2, `earlyCapabilityFlags`).
    ///
    /// Empty when the client did not send the optional field. Servers use it
    /// to gate optional features the client has to opt into, e.g. Server
    /// Heartbeat PDUs (`RNS_UD_CS_SUPPORT_HEARTBEAT_PDU`, section 2.2.16.1).
    pub client_early_capability_flags: gcc::ClientEarlyCapabilityFlags,
    /// Multitransport (MS-RDPEMT) capability flags announced by the client in
    /// its GCC `MultiTransportChannelData` block (section 2.2.1.3.8).
    ///
    /// Empty when the client did not send a multitransport block. Servers that
    /// implement UDP multitransport can use it to decide whether to send a
    /// Server Initiate Multitransport Request.
    pub multitransport_flags: gcc::MultiTransportFlags,
    /// Credentials received from the client during SecureSettingsExchange.
    ///
    /// Present for TLS-mode connections where the client sends credentials
    /// in the ClientInfoPdu. `None` for CredSSP/Hybrid connections (where
    /// authentication happens during the CredSSP exchange instead).
    ///
    /// Servers that need to validate credentials (e.g., via PAM or LDAP)
    /// can use this field for post-handshake validation.
    pub credentials: Option<Credentials>,
    /// Client Auto-Reconnect Packet received in the Client Info PDU.
    ///
    /// This is present when the client resumes a session using an
    /// `ARC_CS_PRIVATE_PACKET`. The packet has already passed wire-format
    /// validation, but the server must still verify its security verifier
    /// against the reconnect random for the target session.
    pub auto_reconnect: Option<ClientAutoReconnect>,
}

impl Acceptor {
    pub fn new(
        security: SecurityProtocol,
        desktop_size: DesktopSize,
        capabilities: Vec<CapabilitySet>,
        creds: Option<Credentials>,
    ) -> Self {
        Self {
            security,
            state: AcceptorState::InitiationWaitRequest,
            user_channel_id: USER_CHANNEL_ID,
            io_channel_id: IO_CHANNEL_ID,
            message_channel_id: None,
            desktop_size,
            keyboard_layout: 0,
            keyboard_type: gcc::KeyboardType(0),
            ime_file_name: String::new(),
            multitransport_flags: gcc::MultiTransportFlags::empty(),
            client_offered_multitransport: false,
            early_capability_flags: gcc::ClientEarlyCapabilityFlags::empty(),
            server_capabilities: capabilities,
            static_channels: StaticChannelSet::new(),
            saved_for_reactivation: Default::default(),
            creds,
            received_credentials: None,
            received_auto_reconnect: None,
            reactivation: false,
            honor_client_desktop_size: None,
            offer_multitransport: None,
            sent_multitransport_request: None,
        }
    }

    /// Adopt the desktop size requested by the client in its Client Core Data
    /// instead of the size this acceptor was constructed with, clamped to an
    /// operator-configured maximum.
    ///
    /// The client's requested resolution is only carried in the GCC Client
    /// Core Data of the MCS Connect Initial PDU; the desktop size echoed back
    /// later in the client's Confirm Active is, per [MS-RDPBCGR] 2.2.1.13.2,
    /// the value the client copied from the *server's* Demand Active, so it
    /// cannot be used to discover what the client originally asked for. When
    /// this is enabled, the client's request is first clamped per dimension to
    /// the operator maximum and then validated against the protocol-legal range;
    /// if the clamped size is legal the acceptor negotiates it from the start (it
    /// is written into the server's Bitmap capability set before Demand Active is
    /// sent), avoiding a Deactivation-Reactivation resize round trip.
    ///
    /// Pass `Some(max)` to honor the client's request, clamped per dimension to
    /// `max`: the client can ask for a *smaller* desktop than `max`, but never a
    /// larger one. The desktop size is a client-controlled `u16` bounded only by
    /// the protocol ([200, 8192]); without a ceiling, a client could request
    /// e.g. 8192×8192 and drive the server's framebuffer/encoder allocation off
    /// that untrusted number (~256 MiB per frame buffer). `max` is that ceiling
    /// — set it to what the server is actually willing to render (for instance
    /// the host display's native resolution). Pass `None` to disable honoring
    /// entirely and always enforce the server-provided size.
    ///
    /// `None` is the default, preserving the previous behavior of always
    /// enforcing the server-provided size.
    ///
    /// # Precondition
    ///
    /// Enabling this only makes sense together with a display handler
    /// ([`RdpServerDisplay`]) whose `request_initial_size` actually adopts (or
    /// at least intersects) the size it is given. The acceptor negotiates the
    /// client's size, but the server still builds its framebuffer/encoder from
    /// the size the display handler reports; if that handler ignores the
    /// requested size and returns a fixed, smaller framebuffer, the resulting
    /// mismatch can cause the client to be dropped. With a fixed-size display
    /// handler, leave this disabled.
    ///
    /// [`RdpServerDisplay`]: <https://docs.rs/ironrdp-server/latest/ironrdp_server/trait.RdpServerDisplay.html>
    pub fn set_honor_client_desktop_size(&mut self, max: Option<DesktopSize>) {
        self.honor_client_desktop_size = max;
    }

    /// Advertise UDP multitransport support (MS-RDPBCGR 2.2.1.4.6) and offer
    /// it to clients that reciprocate.
    ///
    /// Pass `Some(flags)` to send a Server MultiTransportChannelData block
    /// with these flags during Basic Settings Exchange, and, once licensing
    /// completes, an Initiate Multitransport Request for reliable UDP
    /// (`TRANSPORT_TYPE_UDP_FECR`) if `flags` includes it and the client's
    /// own Client MultiTransportChannelData reciprocated. Lossy UDP
    /// (`TRANSPORT_TYPE_UDP_FECL`) is accepted in `flags` for advertisement
    /// purposes but this acceptor never requests it; only the reliable
    /// transport is implemented. Include `SOFT_SYNC_TCP_TO_UDP` to also
    /// support switching dynamic virtual channels from TCP to UDP after the
    /// sideband transport is up; see
    /// [`multitransport_soft_sync_negotiated()`](Self::multitransport_soft_sync_negotiated).
    ///
    /// Offering requires the client to have requested an MCS message
    /// channel (MS-RDPBCGR 2.2.1.3.7): both the request and, when owed, the
    /// client's response travel on it. If the client never requests one, no
    /// request is sent regardless of this setting.
    ///
    /// `None` is the default: no multitransport block is advertised and no
    /// request is ever sent.
    pub fn set_multitransport_offer(&mut self, flags: Option<gcc::MultiTransportFlags>) {
        self.offer_multitransport = flags;
    }

    /// Returns the Initiate Multitransport Request sent to the client, if
    /// [`MultitransportBootstrapping`](AcceptorState::MultitransportBootstrapping)
    /// has run and decided to offer UDP multitransport.
    ///
    /// The caller should treat a `Some` here as the signal to begin
    /// establishing the sideband UDP transport (RDPEUDP2 + TLS + RDPEMT)
    /// using `request_id` and `security_cookie`, in parallel with (not
    /// blocking) the rest of the acceptor sequence: this acceptor does not
    /// wait for the client's Initiate Multitransport Response before
    /// continuing on to capability negotiation, since MS-RDPBCGR 3.2.5.15.1
    /// only obliges the client to send one when Soft-Sync is negotiated or
    /// the attempt failed, never on a plain successful bootstrap.
    ///
    /// `None` before `MultitransportBootstrapping` has run, or when
    /// multitransport was not offered
    /// ([`set_multitransport_offer()`](Self::set_multitransport_offer)
    /// disabled or the client did not reciprocate). Bootstrapping does not
    /// run again on reactivation, so no new request is sent then, but a
    /// request from before reactivation carries forward and is still
    /// returned here: `CapabilitiesWaitConfirm`'s late-response tolerance
    /// needs it to remain visible across reactivation too.
    pub fn multitransport_request(&self) -> Option<&rdp::multitransport::MultitransportRequestPdu> {
        self.sent_multitransport_request.as_ref()
    }

    /// Whether both peers advertised Soft-Sync support for multitransport.
    ///
    /// Only meaningful once [`multitransport_request()`](Self::multitransport_request)
    /// returns `Some`.
    pub fn multitransport_soft_sync_negotiated(&self) -> bool {
        self.offer_multitransport
            .is_some_and(|offer| offer.contains(gcc::MultiTransportFlags::SOFT_SYNC_TCP_TO_UDP))
            && self
                .multitransport_flags
                .contains(gcc::MultiTransportFlags::SOFT_SYNC_TCP_TO_UDP)
    }

    /// If `data` (an MCS SendDataRequest already decoded from the wire) is on
    /// the message channel while a multitransport request is outstanding AND
    /// its payload strictly decodes as an Initiate Multitransport Response,
    /// returns it. MS-RDPBCGR 3.2.5.15.1 gives this response no fixed
    /// position relative to the rest of the handshake: it depends on when
    /// the client resolves its own bootstrapping and whether the sideband
    /// attempt failed, so both `CapabilitiesWaitConfirm` and
    /// `ConnectionFinalization` tolerate it landing wherever it actually
    /// shows up rather than only where `MultitransportBootstrapping`'s own
    /// comment describes as typical.
    ///
    /// The message channel also carries Auto-Detect Response and Heartbeat
    /// PDUs (2.2.1.4.5, 2.2.8.1.1.2.1), so a channel-and-outstanding-request
    /// check alone would misclassify that traffic too; requiring the decode
    /// to actually succeed here lets callers fall through to their own
    /// handling for anything that isn't really a response, mirroring how
    /// `ClientConnectorState::ConnectTimeAutoDetection` demuxes the same
    /// channel client-side.
    fn late_multitransport_response(
        &self,
        data: &mcs::SendDataRequest<'_>,
    ) -> Option<rdp::multitransport::MultitransportResponsePdu> {
        if !(self.sent_multitransport_request.is_some() && Some(data.channel_id) == self.message_channel_id) {
            return None;
        }
        decode::<rdp::multitransport::MultitransportResponsePdu>(data.user_data.as_ref()).ok()
    }

    /// Logs a received Initiate Multitransport Response against the
    /// outstanding request, matching request IDs. Shared by the two call
    /// sites `late_multitransport_response` gates; both only call this once
    /// that method has confirmed a request is outstanding, so
    /// `sent_multitransport_request` is always `Some` here.
    fn log_multitransport_response(&self, response: &rdp::multitransport::MultitransportResponsePdu) {
        let expected_request_id = self
            .sent_multitransport_request
            .as_ref()
            .expect("late_multitransport_response only returns Some when a request is outstanding")
            .request_id;
        if response.request_id == expected_request_id {
            debug!(
                request_id = response.request_id,
                success = response.is_success(),
                "Received Initiate Multitransport Response"
            );
        } else {
            warn!(
                response.request_id,
                expected_request_id, "Initiate Multitransport Response request ID does not match the sent request"
            );
        }
    }

    pub fn new_deactivation_reactivation(
        mut consumed: Acceptor,
        static_channels: StaticChannelSet,
        desktop_size: DesktopSize,
    ) -> ConnectorResult<Self> {
        let AcceptorState::CapabilitiesSendServer {
            early_capability,
            channels,
        } = consumed.saved_for_reactivation
        else {
            return Err(general_err!("invalid acceptor state"));
        };

        set_bitmap_desktop_size(&mut consumed.server_capabilities, desktop_size);
        let state = AcceptorState::CapabilitiesSendServer {
            early_capability,
            channels: channels.clone(),
        };
        let saved_for_reactivation = AcceptorState::CapabilitiesSendServer {
            early_capability,
            channels,
        };
        Ok(Self {
            security: consumed.security,
            state,
            user_channel_id: consumed.user_channel_id,
            io_channel_id: consumed.io_channel_id,
            message_channel_id: consumed.message_channel_id,
            desktop_size,
            keyboard_layout: consumed.keyboard_layout,
            keyboard_type: consumed.keyboard_type,
            ime_file_name: consumed.ime_file_name,
            multitransport_flags: consumed.multitransport_flags,
            client_offered_multitransport: consumed.client_offered_multitransport,
            early_capability_flags: consumed.early_capability_flags,
            server_capabilities: consumed.server_capabilities,
            static_channels,
            saved_for_reactivation,
            creds: consumed.creds,
            received_credentials: consumed.received_credentials,
            received_auto_reconnect: consumed.received_auto_reconnect,
            reactivation: true,
            honor_client_desktop_size: consumed.honor_client_desktop_size,
            offer_multitransport: consumed.offer_multitransport,
            sent_multitransport_request: consumed.sent_multitransport_request,
        })
    }

    pub fn attach_static_channel<T>(&mut self, channel: T)
    where
        T: SvcServerProcessor + 'static,
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
        T: SvcServerProcessor + 'static,
    {
        let channel_name = channel.channel_name();
        if self.static_channels.len() >= MAX_STATIC_CHANNELS
            || self.static_channels.get_by_channel_name_key(&channel_name).is_some()
        {
            return false;
        }

        self.static_channels.insert_dynamic(channel).is_some()
    }

    pub fn reached_security_upgrade(&self) -> Option<SecurityProtocol> {
        match self.state {
            AcceptorState::SecurityUpgrade { .. } => Some(self.security),
            _ => None,
        }
    }

    /// # Panics
    ///
    /// Panics if state is not [AcceptorState::SecurityUpgrade].
    pub fn mark_security_upgrade_as_done(&mut self) {
        assert!(self.reached_security_upgrade().is_some());
        self.step(&[], None, &mut WriteBuf::new())
            .expect("transition to next state");
        debug_assert!(self.reached_security_upgrade().is_none());
    }

    pub fn should_perform_credssp(&self) -> bool {
        matches!(self.state, AcceptorState::Credssp { .. })
    }

    /// # Panics
    ///
    /// Panics if state is not [AcceptorState::Credssp].
    pub fn mark_credssp_as_done(&mut self) {
        assert!(self.should_perform_credssp());
        let res = self
            .step(&[], None, &mut WriteBuf::new())
            .expect("transition to next state");
        debug_assert!(!self.should_perform_credssp());
        assert_eq!(res, Written::Nothing);
    }

    pub fn get_result(&mut self) -> Option<AcceptorResult> {
        match mem::take(&mut self.state) {
            AcceptorState::Accepted {
                channels: _channels, // TODO: what about ChannelDef?
                client_capabilities,
                input_events,
            } => Some(AcceptorResult {
                static_channels: mem::take(&mut self.static_channels),
                capabilities: client_capabilities,
                input_events,
                user_channel_id: self.user_channel_id,
                io_channel_id: self.io_channel_id,
                message_channel_id: self.message_channel_id,
                keyboard_layout: self.keyboard_layout,
                keyboard_type: self.keyboard_type,
                ime_file_name: self.ime_file_name.clone(),
                multitransport_flags: self.multitransport_flags,
                client_early_capability_flags: self.early_capability_flags,
                reactivation: self.reactivation,
                credentials: self.received_credentials.take(),
                auto_reconnect: self.received_auto_reconnect.take(),
            }),
            previous_state => {
                self.state = previous_state;
                None
            }
        }
    }
}

#[derive(Default, Debug)]
pub enum AcceptorState {
    #[default]
    Consumed,

    InitiationWaitRequest,
    InitiationSendConfirm {
        requested_protocol: SecurityProtocol,
    },
    SecurityUpgrade {
        requested_protocol: SecurityProtocol,
        protocol: SecurityProtocol,
    },
    Credssp {
        requested_protocol: SecurityProtocol,
        protocol: SecurityProtocol,
    },
    BasicSettingsWaitInitial {
        requested_protocol: SecurityProtocol,
        protocol: SecurityProtocol,
    },
    BasicSettingsSendResponse {
        requested_protocol: SecurityProtocol,
        protocol: SecurityProtocol,
        early_capability: Option<gcc::ClientEarlyCapabilityFlags>,
        channels: Vec<(u16, Option<gcc::ChannelDef>)>,
    },
    ChannelConnection {
        protocol: SecurityProtocol,
        early_capability: Option<gcc::ClientEarlyCapabilityFlags>,
        channels: Vec<(u16, gcc::ChannelDef)>,
        connection: ChannelConnectionSequence,
    },
    RdpSecurityCommencement {
        protocol: SecurityProtocol,
        early_capability: Option<gcc::ClientEarlyCapabilityFlags>,
        channels: Vec<(u16, gcc::ChannelDef)>,
    },
    SecureSettingsExchange {
        protocol: SecurityProtocol,
        early_capability: Option<gcc::ClientEarlyCapabilityFlags>,
        channels: Vec<(u16, gcc::ChannelDef)>,
    },
    LicensingExchange {
        early_capability: Option<gcc::ClientEarlyCapabilityFlags>,
        channels: Vec<(u16, gcc::ChannelDef)>,
    },
    /// After licensing, decide whether to offer UDP multitransport
    /// (MS-RDPBCGR 2.2.15.1) and, if so, send the Initiate Multitransport
    /// Request.
    ///
    /// Unlike the client's `MultitransportBootstrapping`, which is purely
    /// reactive (it waits to read whatever the server sends), this state is
    /// where the server actively decides and writes: it is entered with
    /// nothing to read, decides based on the client's advertised
    /// `multitransport_flags` and the acceptor's own configured offer, and
    /// either sends the request or skips it, either way moving straight on
    /// to `CapabilitiesSendServer` in the same step.
    ///
    /// There is deliberately no state mirroring the client's
    /// `MultitransportPending`: MS-RDPBCGR 3.2.5.15.1 only obliges the client
    /// to send an Initiate Multitransport Response when Soft-Sync is
    /// negotiated or the sideband attempt failed, so on the common
    /// successful, non-Soft-Sync path no response is ever sent. Blocking
    /// here to read one would stall the handshake forever in exactly that
    /// case. Establishing the actual UDP transport (RDPEUDP2 + TLS + RDPEMT)
    /// is the caller's responsibility, driven out of band from this request:
    /// see [`Acceptor::multitransport_request()`]. Because the client sends
    /// its response, if any, before it ever reads the server's Demand
    /// Active, IronRDP's own client always sends one (if at all) before the
    /// Confirm Active on the wire. That is a client behavior, not a
    /// protocol guarantee 3.2.5.15.1 makes: a conforming third-party client
    /// could just as legitimately send it later, during finalization. Both
    /// `CapabilitiesWaitConfirm` and `ConnectionFinalization` tolerate and
    /// consume it wherever it actually lands, rather than this state
    /// waiting for it.
    MultitransportBootstrapping {
        early_capability: Option<gcc::ClientEarlyCapabilityFlags>,
        channels: Vec<(u16, gcc::ChannelDef)>,
    },
    CapabilitiesSendServer {
        early_capability: Option<gcc::ClientEarlyCapabilityFlags>,
        channels: Vec<(u16, gcc::ChannelDef)>,
    },
    MonitorLayoutSend {
        channels: Vec<(u16, gcc::ChannelDef)>,
    },
    CapabilitiesWaitConfirm {
        channels: Vec<(u16, gcc::ChannelDef)>,
    },
    ConnectionFinalization {
        finalization: FinalizationSequence,
        channels: Vec<(u16, gcc::ChannelDef)>,
        client_capabilities: Vec<CapabilitySet>,
    },
    Accepted {
        channels: Vec<(u16, gcc::ChannelDef)>,
        client_capabilities: Vec<CapabilitySet>,
        input_events: Vec<Vec<u8>>,
    },
}

impl State for AcceptorState {
    fn name(&self) -> &'static str {
        match self {
            Self::Consumed => "Consumed",
            Self::InitiationWaitRequest => "InitiationWaitRequest",
            Self::InitiationSendConfirm { .. } => "InitiationSendConfirm",
            Self::SecurityUpgrade { .. } => "SecurityUpgrade",
            Self::Credssp { .. } => "Credssp",
            Self::BasicSettingsWaitInitial { .. } => "BasicSettingsWaitInitial",
            Self::BasicSettingsSendResponse { .. } => "BasicSettingsSendResponse",
            Self::ChannelConnection { .. } => "ChannelConnection",
            Self::RdpSecurityCommencement { .. } => "RdpSecurityCommencement",
            Self::SecureSettingsExchange { .. } => "SecureSettingsExchange",
            Self::LicensingExchange { .. } => "LicensingExchange",
            Self::MultitransportBootstrapping { .. } => "MultitransportBootstrapping",
            Self::CapabilitiesSendServer { .. } => "CapabilitiesSendServer",
            Self::MonitorLayoutSend { .. } => "MonitorLayoutSend",
            Self::CapabilitiesWaitConfirm { .. } => "CapabilitiesWaitConfirm",
            Self::ConnectionFinalization { .. } => "ConnectionFinalization",
            Self::Accepted { .. } => "Connected",
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(self, Self::Accepted { .. })
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

impl Sequence for Acceptor {
    fn next_pdu_hint(&self) -> Option<&dyn pdu::PduHint> {
        match &self.state {
            AcceptorState::Consumed => None,
            AcceptorState::InitiationWaitRequest => Some(&pdu::X224_HINT),
            AcceptorState::InitiationSendConfirm { .. } => None,
            AcceptorState::SecurityUpgrade { .. } => None,
            AcceptorState::Credssp { .. } => None,
            AcceptorState::BasicSettingsWaitInitial { .. } => Some(&pdu::X224_HINT),
            AcceptorState::BasicSettingsSendResponse { .. } => None,
            AcceptorState::ChannelConnection { connection, .. } => connection.next_pdu_hint(),
            AcceptorState::RdpSecurityCommencement { .. } => None,
            AcceptorState::SecureSettingsExchange { .. } => Some(&pdu::X224_HINT),
            AcceptorState::LicensingExchange { .. } => None,
            // Nothing to read: this state decides whether to send a request,
            // then moves straight on to CapabilitiesSendServer.
            AcceptorState::MultitransportBootstrapping { .. } => None,
            AcceptorState::CapabilitiesSendServer { .. } => None,
            AcceptorState::MonitorLayoutSend { .. } => None,
            AcceptorState::CapabilitiesWaitConfirm { .. } => Some(&pdu::X224_HINT),
            AcceptorState::ConnectionFinalization { finalization, .. } => finalization.next_pdu_hint(),
            AcceptorState::Accepted { .. } => None,
        }
    }

    fn state(&self) -> &dyn State {
        &self.state
    }

    fn step(
        &mut self,
        input: &[u8],
        received_at: Option<MonotonicInstant>,
        output: &mut WriteBuf,
    ) -> ConnectorResult<Written> {
        let prev_state = mem::take(&mut self.state);

        let (written, next_state) = match prev_state {
            AcceptorState::InitiationWaitRequest => {
                let connection_request = decode::<X224<nego::ConnectionRequest>>(input)
                    .map_err(ConnectorError::decode)
                    .map(|p| p.0)?;

                debug!(message = ?connection_request, "Received");

                (
                    Written::Nothing,
                    AcceptorState::InitiationSendConfirm {
                        requested_protocol: connection_request.protocol,
                    },
                )
            }

            AcceptorState::InitiationSendConfirm { requested_protocol } => {
                let protocols = requested_protocol & self.security;
                let protocol = if protocols.intersects(SecurityProtocol::HYBRID_EX) {
                    SecurityProtocol::HYBRID_EX
                } else if protocols.intersects(SecurityProtocol::HYBRID) {
                    SecurityProtocol::HYBRID
                } else if protocols.intersects(SecurityProtocol::SSL) {
                    SecurityProtocol::SSL
                } else if self.security.is_empty() {
                    SecurityProtocol::empty()
                } else {
                    // No common security protocol. Send RDP_NEG_FAILURE so the client
                    // gets a well-formed response instead of a TCP reset (MS-RDPBCGR 2.2.1.2.2).
                    let failure_code = if self.security.intersects(SecurityProtocol::SSL) {
                        nego::FailureCode::SSL_REQUIRED_BY_SERVER
                    } else if self
                        .security
                        .intersects(SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX)
                    {
                        nego::FailureCode::HYBRID_REQUIRED_BY_SERVER
                    } else {
                        nego::FailureCode::SSL_REQUIRED_BY_SERVER
                    };

                    let failure = nego::ConnectionConfirm::Failure { code: failure_code };

                    debug!(message = ?failure, "Send");

                    ironrdp_core::encode_buf(&X224(failure), output).map_err(ConnectorError::encode)?;

                    return Err(reason_err!(
                        "security protocol mismatch",
                        "server requires {:?} but client only offered {:?}",
                        self.security,
                        requested_protocol,
                    ));
                };
                let connection_confirm = nego::ConnectionConfirm::Response {
                    flags: nego::ResponseFlags::EXTENDED_CLIENT_DATA_SUPPORTED,
                    protocol,
                };

                debug!(message = ?connection_confirm, "Send");

                let written =
                    ironrdp_core::encode_buf(&X224(connection_confirm), output).map_err(ConnectorError::encode)?;

                (
                    Written::from_size(written)?,
                    AcceptorState::SecurityUpgrade {
                        requested_protocol,
                        protocol,
                    },
                )
            }

            AcceptorState::SecurityUpgrade {
                requested_protocol,
                protocol,
            } => {
                debug!(?requested_protocol);
                let next_state = if protocol.intersects(SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX) {
                    AcceptorState::Credssp {
                        requested_protocol,
                        protocol,
                    }
                } else {
                    AcceptorState::BasicSettingsWaitInitial {
                        requested_protocol,
                        protocol,
                    }
                };
                (Written::Nothing, next_state)
            }

            AcceptorState::Credssp {
                requested_protocol,
                protocol,
            } => (
                Written::Nothing,
                AcceptorState::BasicSettingsWaitInitial {
                    requested_protocol,
                    protocol,
                },
            ),

            AcceptorState::BasicSettingsWaitInitial {
                requested_protocol,
                protocol,
            } => {
                let x224_payload = decode::<X224<pdu::x224::X224Data<'_>>>(input)
                    .map_err(ConnectorError::decode)
                    .map(|p| p.0)?;
                let settings_initial =
                    decode::<mcs::ConnectInitial>(x224_payload.data.as_ref()).map_err(ConnectorError::decode)?;

                debug!(message = ?settings_initial, "Received");

                let gcc_blocks = settings_initial.conference_create_request.into_gcc_blocks();
                let early_capability = gcc_blocks.core.optional_data.early_capability_flags;
                self.early_capability_flags = early_capability.unwrap_or(gcc::ClientEarlyCapabilityFlags::empty());
                let client_wants_message_channel = gcc_blocks.message_channel.is_some();
                self.keyboard_layout = gcc_blocks.core.keyboard_layout;
                self.keyboard_type = gcc_blocks.core.keyboard_type;
                self.ime_file_name.clone_from(&gcc_blocks.core.ime_file_name);
                self.client_offered_multitransport = gcc_blocks.multi_transport_channel.is_some();
                self.multitransport_flags = gcc_blocks
                    .multi_transport_channel
                    .as_ref()
                    .map(|m| m.flags)
                    .unwrap_or_else(gcc::MultiTransportFlags::empty);

                // Adopt the client's requested desktop size (from its Client
                // Core Data) before Demand Active is sent, so the session is
                // negotiated at that size without a Deactivation-Reactivation
                // resize. The request is clamped to the operator-configured
                // maximum first, so an untrusted client can't drive the
                // framebuffer/encoder allocation past that ceiling. See
                // `set_honor_client_desktop_size`.
                if let Some(max) = self.honor_client_desktop_size {
                    let requested_width = gcc_blocks.core.desktop_width;
                    let requested_height = gcc_blocks.core.desktop_height;
                    let clamped_width = requested_width.min(max.width);
                    let clamped_height = requested_height.min(max.height);
                    if let Some(client_size) = validate_desktop_size(clamped_width, clamped_height) {
                        if client_size != self.desktop_size {
                            debug!(
                                requested = ?DesktopSize { width: requested_width, height: requested_height },
                                max = ?max,
                                adopted = ?client_size,
                                previous = ?self.desktop_size,
                                "Honoring client-requested desktop size (clamped to operator maximum)"
                            );
                            self.desktop_size = client_size;
                            set_bitmap_desktop_size(&mut self.server_capabilities, client_size);
                        }
                    } else {
                        debug!(
                            requested = ?DesktopSize { width: requested_width, height: requested_height },
                            clamped = ?DesktopSize { width: clamped_width, height: clamped_height },
                            max = ?max,
                            "Client-requested desktop size is out of protocol range after clamping to the operator maximum; keeping the server-provided size"
                        );
                    }
                }

                let joined: Vec<_> = gcc_blocks
                    .network
                    .map(|network| {
                        network
                            .channels
                            .into_iter()
                            .map(|c| {
                                self.static_channels
                                    .get_by_channel_name_key(&c.name)
                                    .map(|(key, _)| (key, c))
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                #[expect(clippy::arithmetic_side_effects)] // IO channel ID is not big enough for overflowing.
                let channels: Vec<_> = joined
                    .into_iter()
                    .enumerate()
                    .map(|(i, channel)| {
                        let channel_id = u16::try_from(i).expect("always in the range") + self.io_channel_id + 1;
                        if let Some((key, c)) = channel {
                            self.static_channels.attach_channel_id_by_key(key, channel_id);
                            (channel_id, Some(c))
                        } else {
                            (channel_id, None)
                        }
                    })
                    .collect();

                if client_wants_message_channel {
                    // Allocate the message channel ID after the I/O channel and
                    // any static virtual channels. It is advertised in Server
                    // Message Channel Data and joined alongside the others.
                    #[expect(clippy::arithmetic_side_effects)] // IO channel ID is not big enough for overflowing.
                    let channel_id =
                        u16::try_from(channels.len()).expect("always in the range") + self.io_channel_id + 1;
                    self.message_channel_id = Some(channel_id);
                }

                (
                    Written::Nothing,
                    AcceptorState::BasicSettingsSendResponse {
                        requested_protocol,
                        protocol,
                        early_capability,
                        channels,
                    },
                )
            }

            AcceptorState::BasicSettingsSendResponse {
                requested_protocol,
                protocol,
                early_capability,
                channels,
            } => {
                let channel_ids: Vec<u16> = channels.iter().map(|&(i, _)| i).collect();

                let skip_channel_join = early_capability
                    .is_some_and(|client| client.contains(gcc::ClientEarlyCapabilityFlags::SUPPORT_SKIP_CHANNELJOIN));

                let server_blocks = create_gcc_blocks(
                    self.io_channel_id,
                    channel_ids.clone(),
                    requested_protocol,
                    skip_channel_join,
                    self.message_channel_id,
                    self.offer_multitransport.filter(|_| self.client_offered_multitransport),
                );

                let settings_response = mcs::ConnectResponse {
                    conference_create_response: gcc::ConferenceCreateResponse::new(self.user_channel_id, server_blocks)
                        .map_err(ConnectorError::decode)?,
                    called_connect_id: 1,
                    domain_parameters: mcs::DomainParameters::target(),
                };

                debug!(message = ?settings_response, "Send");

                let written = encode_x224_packet(&settings_response, output)?;
                let channels = channels.into_iter().filter_map(|(i, c)| c.map(|c| (i, c))).collect();

                (
                    Written::from_size(written)?,
                    AcceptorState::ChannelConnection {
                        protocol,
                        early_capability,
                        channels,
                        connection: if skip_channel_join {
                            ChannelConnectionSequence::skip_channel_join(self.user_channel_id)
                        } else {
                            let mut join_channel_ids = channel_ids;
                            join_channel_ids.extend(self.message_channel_id);
                            ChannelConnectionSequence::new(self.user_channel_id, self.io_channel_id, join_channel_ids)
                        },
                    },
                )
            }

            AcceptorState::ChannelConnection {
                protocol,
                early_capability,
                channels,
                mut connection,
            } => {
                let written = connection.step(input, received_at, output)?;
                let state = if connection.is_done() {
                    AcceptorState::RdpSecurityCommencement {
                        protocol,
                        early_capability,
                        channels,
                    }
                } else {
                    AcceptorState::ChannelConnection {
                        protocol,
                        early_capability,
                        channels,
                        connection,
                    }
                };

                (written, state)
            }

            AcceptorState::RdpSecurityCommencement {
                protocol,
                early_capability,
                channels,
                ..
            } => (
                Written::Nothing,
                AcceptorState::SecureSettingsExchange {
                    protocol,
                    early_capability,
                    channels,
                },
            ),

            AcceptorState::SecureSettingsExchange {
                protocol,
                early_capability,
                channels,
            } => {
                let data: X224<mcs::SendDataRequest<'_>> = decode(input).map_err(ConnectorError::decode)?;
                let data = data.0;
                let client_info: rdp::ClientInfoPdu =
                    decode(data.user_data.as_ref()).map_err(ConnectorError::decode)?;

                let auto_reconnect = client_info
                    .client_info
                    .extra_info
                    .optional_data
                    .auto_reconnect()
                    .cloned();
                debug!(
                    has_auto_reconnect = auto_reconnect.is_some(),
                    "Received Client Info PDU"
                );
                self.received_auto_reconnect = auto_reconnect;

                if !protocol.intersects(SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX) {
                    let creds = client_info.client_info.credentials;

                    if let Some(expected) = &self.creds {
                        if expected != &creds {
                            // FIXME: How authorization should be denied with standard RDP security?
                            // Since standard RDP security is not a priority, we just send a ServerDeniedConnection ServerSetErrorInfo PDU.
                            let info = ServerSetErrorInfoPdu(ErrorInfo::ProtocolIndependentCode(
                                ProtocolIndependentCode::ServerDeniedConnection,
                            ));

                            debug!(message = ?info, "Send");

                            util::encode_send_data_indication(self.user_channel_id, self.io_channel_id, &info, output)?;

                            return Err(ConnectorError::general("invalid credentials"));
                        }
                    }

                    // Store credentials for later retrieval via AcceptorResult.
                    self.received_credentials = Some(creds);
                }

                (
                    Written::Nothing,
                    AcceptorState::LicensingExchange {
                        early_capability,
                        channels,
                    },
                )
            }

            AcceptorState::LicensingExchange {
                early_capability,
                channels,
            } => {
                let license: LicensePdu = LicensingErrorMessage::new_valid_client()
                    .map_err(ConnectorError::encode)?
                    .into();

                debug!(message = ?license, "Send");

                let written =
                    util::encode_send_data_indication(self.user_channel_id, self.io_channel_id, &license, output)?;

                // Reactivation (Deactivation-Reactivation Sequence, e.g. a
                // display resize) re-enters capability negotiation directly:
                // the sideband UDP transport, if any, was already bootstrapped
                // once for this connection and is not torn down or re-offered.
                self.saved_for_reactivation = AcceptorState::CapabilitiesSendServer {
                    early_capability,
                    channels: channels.clone(),
                };

                (
                    Written::from_size(written)?,
                    AcceptorState::MultitransportBootstrapping {
                        early_capability,
                        channels,
                    },
                )
            }

            AcceptorState::MultitransportBootstrapping {
                early_capability,
                channels,
            } => {
                let next_state = AcceptorState::CapabilitiesSendServer {
                    early_capability,
                    channels,
                };

                let offer_udp_fecr = self
                    .offer_multitransport
                    .is_some_and(|offer| offer.contains(gcc::MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));
                let client_supports_udp_fecr = self
                    .multitransport_flags
                    .contains(gcc::MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR);
                // 2.2.15.1 requires the request to travel on the MCS message
                // channel. A client can in principle advertise UDP support
                // without also requesting a message channel; rather than
                // failing the whole connection over a mismatch in an optional
                // feature's negotiation, that is treated the same as not
                // offering.
                let message_channel_id = self
                    .message_channel_id
                    .filter(|_| offer_udp_fecr && client_supports_udp_fecr);

                if let Some(message_channel_id) = message_channel_id {
                    let mut security_cookie = [0u8; 16];
                    let mut rng = rand::rng();
                    rng.fill_bytes(&mut security_cookie);
                    let request_id = rng.next_u32();

                    let request = rdp::multitransport::MultitransportRequestPdu {
                        security_header: rdp::headers::BasicSecurityHeader {
                            flags: rdp::headers::BasicSecurityHeaderFlags::TRANSPORT_REQ,
                        },
                        request_id,
                        requested_protocol: rdp::multitransport::RequestedProtocol::UdpFecR,
                        security_cookie,
                    };

                    debug!(message = ?request, "Send");

                    let written =
                        util::encode_send_data_indication(self.user_channel_id, message_channel_id, &request, output)?;

                    self.sent_multitransport_request = Some(request);

                    (Written::from_size(written)?, next_state)
                } else {
                    debug!(
                        offer_udp_fecr,
                        client_supports_udp_fecr, "Not offering UDP multitransport"
                    );
                    (Written::Nothing, next_state)
                }
            }

            AcceptorState::CapabilitiesSendServer {
                early_capability,
                channels,
            } => {
                let demand_active = rdp::headers::ShareControlHeader {
                    share_id: 0,
                    pdu_source: self.io_channel_id,
                    share_control_pdu: ShareControlPdu::ServerDemandActive(rdp::capability_sets::ServerDemandActive {
                        pdu: rdp::capability_sets::DemandActive {
                            source_descriptor: "".into(),
                            capability_sets: self.server_capabilities.clone(),
                        },
                    }),
                };

                debug!(message = ?demand_active, "Send");

                let written = util::encode_send_data_indication(
                    self.user_channel_id,
                    self.io_channel_id,
                    &demand_active,
                    output,
                )?;

                let layout_flag = gcc::ClientEarlyCapabilityFlags::SUPPORT_MONITOR_LAYOUT_PDU;
                let next_state = if early_capability.is_some_and(|c| c.contains(layout_flag)) {
                    AcceptorState::MonitorLayoutSend { channels }
                } else {
                    AcceptorState::CapabilitiesWaitConfirm { channels }
                };

                (Written::from_size(written)?, next_state)
            }

            AcceptorState::MonitorLayoutSend { channels } => {
                let monitor_layout =
                    rdp::headers::ShareDataPdu::MonitorLayout(rdp::finalization_messages::MonitorLayoutPdu {
                        monitors: vec![gcc::Monitor {
                            left: 0,
                            top: 0,
                            right: i32::from(self.desktop_size.width) - 1,
                            bottom: i32::from(self.desktop_size.height) - 1,
                            flags: gcc::MonitorFlags::PRIMARY,
                        }],
                    });

                debug!(message = ?monitor_layout, "Send");

                let share_data = wrap_share_data(monitor_layout, self.io_channel_id);

                let written =
                    util::encode_send_data_indication(self.user_channel_id, self.io_channel_id, &share_data, output)?;

                (
                    Written::from_size(written)?,
                    AcceptorState::CapabilitiesWaitConfirm { channels },
                )
            }

            AcceptorState::CapabilitiesWaitConfirm { ref channels } => {
                let message = decode::<X224<mcs::McsMessage<'_>>>(input)
                    .map_err(ConnectorError::decode)
                    .map(|p| p.0);
                let message = match message {
                    Ok(msg) => msg,
                    Err(e) => {
                        if self.reactivation {
                            debug!("Dropping unexpected PDU during reactivation");
                            self.state = prev_state;
                            return Ok(Written::Nothing);
                        } else {
                            return Err(e);
                        }
                    }
                };
                // An Initiate Multitransport Response can legitimately land here:
                // it travels on the message channel, and the client sends it
                // (when it sends one at all) while resolving its own multitransport
                // bootstrapping, strictly before it ever reads the Demand Active
                // that leads to Confirm Active. So it is checked for by channel
                // and a successful strict decode before assuming the payload is a
                // Confirm Active, and simply logged and dropped: this acceptor
                // does not gate on it, per the note on
                // `AcceptorState::MultitransportBootstrapping`. A decode failure
                // here means the message-channel traffic isn't a response at all
                // (Auto-Detect Response, Heartbeat), so it falls through to the
                // Confirm Active handling below instead.
                let late_multitransport_response = match &message {
                    mcs::McsMessage::SendDataRequest(data) => self.late_multitransport_response(data),
                    _ => None,
                };

                if let Some(response) = late_multitransport_response {
                    self.log_multitransport_response(&response);
                    self.state = prev_state;
                    return Ok(Written::Nothing);
                }

                match message {
                    mcs::McsMessage::SendDataRequest(data) => {
                        let capabilities_confirm = decode::<rdp::headers::ShareControlHeader>(data.user_data.as_ref())
                            .map_err(ConnectorError::decode);
                        let capabilities_confirm = match capabilities_confirm {
                            Ok(capabilities_confirm) => capabilities_confirm,
                            Err(e) => {
                                if self.reactivation {
                                    debug!("Dropping unexpected PDU during reactivation");
                                    self.state = prev_state;
                                    return Ok(Written::Nothing);
                                } else {
                                    return Err(e);
                                }
                            }
                        };

                        debug!(message = ?capabilities_confirm, "Received");

                        let ShareControlPdu::ClientConfirmActive(confirm) = capabilities_confirm.share_control_pdu
                        else {
                            return Err(ConnectorError::general("expected client confirm active"));
                        };

                        (
                            Written::Nothing,
                            AcceptorState::ConnectionFinalization {
                                channels: channels.clone(),
                                finalization: FinalizationSequence::new(self.user_channel_id, self.io_channel_id),
                                client_capabilities: confirm.pdu.capability_sets,
                            },
                        )
                    }

                    mcs::McsMessage::DisconnectProviderUltimatum(ultimatum) => {
                        return Err(reason_err!("received disconnect ultimatum", "{:?}", ultimatum.reason));
                    }

                    _ => {
                        warn!(?message, "Unexpected MCS message received");

                        (Written::Nothing, prev_state)
                    }
                }
            }

            AcceptorState::ConnectionFinalization {
                mut finalization,
                channels,
                client_capabilities,
            } => {
                // A late Initiate Multitransport Response can land in any
                // finalization sub-state (see `late_multitransport_response`);
                // none of FinalizationSequence's own PDU decoders expect it, and
                // depending which sub-state is active it would otherwise be
                // silently swallowed while advancing a state, propagated as a
                // connection-ending decode error, or surfaced to the embedding
                // application as a raw input event. Check for it here, before
                // finalization ever sees the bytes, mirroring
                // `CapabilitiesWaitConfirm`'s handling.
                let late_multitransport_response = match decode::<X224<mcs::McsMessage<'_>>>(input) {
                    Ok(X224(mcs::McsMessage::SendDataRequest(data))) => self.late_multitransport_response(&data),
                    _ => None,
                };

                if let Some(response) = late_multitransport_response {
                    self.log_multitransport_response(&response);

                    (
                        Written::Nothing,
                        AcceptorState::ConnectionFinalization {
                            finalization,
                            channels,
                            client_capabilities,
                        },
                    )
                } else {
                    let written = finalization.step(input, received_at, output)?;

                    let state = if finalization.is_done() {
                        AcceptorState::Accepted {
                            channels,
                            client_capabilities,
                            input_events: finalization.into_input_events(),
                        }
                    } else {
                        AcceptorState::ConnectionFinalization {
                            finalization,
                            channels,
                            client_capabilities,
                        }
                    };

                    (written, state)
                }
            }

            _ => unreachable!(),
        };

        self.state = next_state;
        Ok(written)
    }
}

fn create_gcc_blocks(
    io_channel: u16,
    channel_ids: Vec<u16>,
    requested: SecurityProtocol,
    skip_channel_join: bool,
    message_channel_id: Option<u16>,
    offer_multitransport: Option<gcc::MultiTransportFlags>,
) -> gcc::ServerGccBlocks {
    gcc::ServerGccBlocks {
        core: gcc::ServerCoreData {
            version: gcc::RdpVersion::V5_PLUS,
            optional_data: gcc::ServerCoreOptionalData {
                client_requested_protocols: Some(requested),
                early_capability_flags: skip_channel_join
                    .then_some(gcc::ServerEarlyCapabilityFlags::SKIP_CHANNELJOIN_SUPPORTED),
            },
        },
        security: gcc::ServerSecurityData::no_security(),
        network: gcc::ServerNetworkData {
            channel_ids,
            io_channel,
        },
        message_channel: message_channel_id.map(|id| gcc::ServerMessageChannelData {
            mcs_message_channel_id: id,
        }),
        // Only meaningful alongside a message channel: the request and any
        // response it draws both travel there (MS-RDPBCGR 2.2.15.1, 2.2.15.2).
        // The caller has already filtered offer_multitransport to None when
        // the client did not populate its own MultiTransportChannelData
        // block, per 2.2.1.4's requirement that this block be omitted then.
        multi_transport_channel: message_channel_id
            .and(offer_multitransport)
            .map(|flags| gcc::MultiTransportChannelData { flags }),
    }
}
