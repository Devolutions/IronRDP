use core::fmt;
use core::net::{IpAddr, SocketAddr};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::time::Duration;
#[cfg(feature = "usb")]
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::LazyLock;
use std::time::Instant;

use ironrdp_acceptor::{Acceptor, AcceptorResult, BeginResult, DesktopSize};
use ironrdp_async::Framed;
use ironrdp_cliprdr::CliprdrServer;
use ironrdp_cliprdr::backend::ClipboardMessage;
use ironrdp_core::{decode, encode_vec, impl_as_any};
use ironrdp_displaycontrol::pdu::DisplayControlMonitorLayout;
use ironrdp_displaycontrol::server::{DisplayControlHandler, DisplayControlServer};
use ironrdp_dvc as dvc;
#[cfg(feature = "usb")]
use ironrdp_dvc::DynamicChannelId;
use ironrdp_error::ResultExt as _;
#[cfg(feature = "usb")]
use ironrdp_pdu::PduError;
use ironrdp_pdu::codecs::rfx::Quant;
use ironrdp_pdu::input::InputEventPdu;
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent};
use ironrdp_pdu::mcs::{SendDataIndication, SendDataRequest};
use ironrdp_pdu::rdp::capability_sets::{
    BitmapCodecs, CapabilitySet, CmdFlags, CodecProperty, EntropyBits, GeneralExtraFlags,
};
pub use ironrdp_pdu::rdp::client_info::Credentials;
use ironrdp_pdu::rdp::headers::{ServerDeactivateAll, ShareControlPdu};
use ironrdp_pdu::rdp::server_error_info::{ErrorInfo, ProtocolIndependentCode, ServerSetErrorInfoPdu};
use ironrdp_pdu::x224::X224;
use ironrdp_pdu::{Action, PduResult, decode_err, mcs, nego, rdp};
#[cfg(feature = "usb")]
use ironrdp_rdpeusb::io::RequestId;
use ironrdp_rdpsnd as rdpsnd;
use ironrdp_svc::{ChannelFlags, StaticChannelId, StaticChannelSet, SvcProcessor, server_encode_svc_messages};
use ironrdp_tokio::{FramedRead, FramedWrite, TokioFramed, split_tokio_framed, unsplit_tokio_framed};
use rand::RngCore as _;
use rdpsnd::server::{RdpsndServer, RdpsndServerMessage};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};
use tokio::net::{TcpSocket, TcpStream};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task;
use tokio_rustls::TlsAcceptor;
use tracing::{debug, error, info, trace, warn};

use crate::autodetect::{AutoDetectManager, RttSnapshot};
use crate::clipboard::CliprdrServerFactory;
use crate::display::{DisplayUpdate, RdpServerDisplay};
use crate::echo::{EchoDvcBridge, EchoServerHandle, EchoServerMessage, build_echo_request};
use crate::encoder::{UpdateEncoder, UpdateEncoderCodecs};
use crate::error::{ServerError, ServerErrorExt as _, ServerErrorKind, ServerResult, from_anyhow_with_context};
#[cfg(feature = "egfx")]
use crate::gfx::{EgfxServerMessage, GfxServerFactory};
use crate::handler::RdpServerInputHandler;
use crate::rdpei::RdpeiServerFactory;
#[cfg(feature = "usb")]
use crate::urbdrc::{
    DeviceFactory, RawPending, ServerDeviceIoReq, UrbdrcDeviceServerMessage, UrbdrcServerMessage, UsbControlHandle,
    UsbDeviceHandle, UsbDeviceLifecycle,
};
use crate::{SoundServerFactory, builder, capabilities};
#[cfg(feature = "usb")]
use ironrdp_rdpeusb::{InterfaceAlloc, io::CompletionData, server::UrbdrcControlServer, server::UrbdrcDeviceServer};

/// TCP listen backlog size for the RDP server socket.
const LISTENER_BACKLOG: u32 = 1024;
const AUTO_RECONNECT_COOKIE_UPDATE_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Monotonic milliseconds since first use, for feeding the auto-detect state machine.
///
/// The clock lives here, in the I/O driver, rather than inside [`AutoDetectManager`]:
/// the state machine takes timestamps as arguments so it stays free of ambient time.
/// Only differences are meaningful, so the epoch is arbitrary.
fn monotonic_now_ms() -> u64 {
    static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);
    u64::try_from(EPOCH.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Action to take after a client disconnects.
///
/// Returned by [`ConnectionHandler::on_disconnected`] to control whether
/// the server continues accepting new connections or shuts down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostConnectionAction {
    /// Continue accepting new connections.
    Continue,
    /// Stop the accept loop and return from [`RdpServer::run`].
    Stop,
}

/// Per-connection metadata captured during connection setup, made available to
/// [`ConnectionHandler::on_connection_info`] once the connection is established.
///
/// These are GCC Client Core Data fields (MS-RDPBCGR 2.2.1.3.2) that the acceptor
/// captures but has no use for itself; embedders that want to act on them (for
/// example, selecting a server-side keyboard layout matching the client) can do
/// so here without reaching into the acceptor's internals.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ConnectionInfo {
    /// See [`ironrdp_acceptor::AcceptorResult::keyboard_layout`].
    pub keyboard_layout: u32,
    /// See [`ironrdp_acceptor::AcceptorResult::keyboard_type`].
    pub keyboard_type: ironrdp_pdu::gcc::KeyboardType,
    /// See [`ironrdp_acceptor::AcceptorResult::ime_file_name`].
    pub ime_file_name: String,
}

impl ConnectionInfo {
    /// Builds a `ConnectionInfo` directly, for downstream `ConnectionHandler` implementations
    /// that want to exercise [`ConnectionHandler::on_connection_info`] in their own unit tests
    /// without going through a live connection. `#[non_exhaustive]` blocks struct-literal
    /// construction outside this crate, so a constructor is the only way to do that.
    pub fn new(keyboard_layout: u32, keyboard_type: ironrdp_pdu::gcc::KeyboardType, ime_file_name: String) -> Self {
        Self {
            keyboard_layout,
            keyboard_type,
            ime_file_name,
        }
    }
}

/// Hooks for connection lifecycle events.
///
/// Implement this trait to add pre-accept filtering (rate limiting,
/// IP allowlists), post-disconnect logic (cleanup, session validity
/// checks, metrics), and to observe per-connection metadata once a
/// connection is established.
///
/// All methods have default implementations that accept all connections
/// and continue unconditionally.
///
/// [`Self::on_accept`] and [`Self::on_disconnected`] are called only from
/// [`RdpServer::run`]'s own accept loop. [`Self::on_connection_info`] is
/// called from every code path that completes connection setup, including
/// [`RdpServer::run_connection`] and [`RdpServer::run_connection_with`], so it
/// is the hook to use for embedders (such as those with their own
/// multi-transport accept loop) that do not call `run`.
pub trait ConnectionHandler: Send {
    /// Called after `accept()` returns but before `run_connection()`.
    ///
    /// Return `false` to reject the connection (the TCP stream is dropped).
    fn on_accept(&mut self, peer: SocketAddr) -> bool {
        let _ = peer;
        true
    }

    /// Called once per connection, after credential and auto-reconnect
    /// validation succeed and before the session loop starts.
    fn on_connection_info(&mut self, info: &ConnectionInfo) {
        let _ = info;
    }

    /// Called after `run_connection()` completes (successfully or with error).
    ///
    /// `duration` is the wall-clock time the connection was active.
    /// `error` is `Some` if the connection ended with an error.
    fn on_disconnected(
        &mut self,
        peer: SocketAddr,
        duration: Duration,
        error: Option<&ServerError>,
    ) -> PostConnectionAction {
        let _ = (peer, duration, error);
        PostConnectionAction::Continue
    }
}

/// Outcome of a successful [`CredentialValidator::validate`] call.
///
/// A rejection from a working validator is not an error: the validator did
/// its job and decided the credentials do not authenticate. Backend failures
/// (LDAP unreachable, PAM transport broken, database connection lost) are
/// reported via [`CredentialValidationError`] instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialDecision {
    /// Credentials accepted; the connection proceeds.
    Accept,
    /// Credentials rejected; the connection is closed.
    Reject,
}

/// Error returned by a [`CredentialValidator`] when the validator backend
/// itself fails (rather than the credentials being invalid).
///
/// Wraps any [`core::error::Error`] from the backend (LDAP/PAM/DB/etc.) so
/// the trait does not require a particular error library in implementors or
/// consumers.
#[derive(Debug)]
pub struct CredentialValidationError {
    source: Box<dyn core::error::Error + Send + Sync>,
}

impl CredentialValidationError {
    /// Wrap a backend error as a credential-validation failure.
    pub fn new<E>(source: E) -> Self
    where
        E: core::error::Error + Send + Sync + 'static,
    {
        Self {
            source: Box::new(source),
        }
    }
}

impl fmt::Display for CredentialValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("credential validator backend failure")
    }
}

impl core::error::Error for CredentialValidationError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&*self.source)
    }
}

/// Server-side credential validator for TLS-mode connections.
///
/// Called during connection setup when the server receives client credentials
/// via `ClientInfoPdu`. Not used for CredSSP/Hybrid connections (those use
/// pre-loaded credentials for NTLM challenge-response).
///
/// Implement this trait to validate credentials against external systems
/// (PAM, LDAP, database, etc.). For blocking backends, wrap the call in
/// `tokio::task::spawn_blocking` to avoid stalling the async runtime.
///
/// # Example
///
/// ```ignore
/// use ironrdp_server::{CredentialDecision, CredentialValidationError, CredentialValidator, Credentials};
///
/// struct StaticValidator {
///     expected_user: String,
///     expected_password: String,
/// }
///
/// #[async_trait::async_trait]
/// impl CredentialValidator for StaticValidator {
///     async fn validate(
///         &self,
///         creds: &Credentials,
///     ) -> Result<CredentialDecision, CredentialValidationError> {
///         if creds.username == self.expected_user && creds.password == self.expected_password {
///             Ok(CredentialDecision::Accept)
///         } else {
///             Ok(CredentialDecision::Reject)
///         }
///     }
/// }
/// ```
#[async_trait::async_trait]
pub trait CredentialValidator: Send + Sync {
    /// Validate credentials received from the client.
    ///
    /// Return `Ok(CredentialDecision::Accept)` to permit the connection,
    /// `Ok(CredentialDecision::Reject)` to refuse it. Return
    /// `Err(CredentialValidationError::new(_))` only when the validator
    /// itself could not produce a decision (backend system error).
    ///
    /// Implementors backed by blocking systems (PAM, libldap, a synchronous
    /// database driver) should offload the work, for example with
    /// `tokio::task::spawn_blocking`, so the returned future does not stall the
    /// caller's executor. Native-async backends can simply `.await`.
    async fn validate(&self, credentials: &Credentials) -> Result<CredentialDecision, CredentialValidationError>;
}

/// A built-in [`CredentialValidator`] that accepts exactly one fixed set of credentials.
///
/// This is the validation-policy equivalent of the acceptor's pre-loaded
/// exact-match: it keeps the common "one known account" case a one-liner while
/// going through the same hook as PAM, LDAP, or database-backed validators.
pub struct ExactMatchCredentialValidator {
    expected: Credentials,
}

impl ExactMatchCredentialValidator {
    /// Build a validator that accepts only `expected` and rejects everything else.
    pub fn new(expected: Credentials) -> Self {
        Self { expected }
    }
}

#[async_trait::async_trait]
impl CredentialValidator for ExactMatchCredentialValidator {
    async fn validate(&self, credentials: &Credentials) -> Result<CredentialDecision, CredentialValidationError> {
        if credentials == &self.expected {
            Ok(CredentialDecision::Accept)
        } else {
            Ok(CredentialDecision::Reject)
        }
    }
}

#[derive(Clone)]
#[non_exhaustive]
pub struct RdpServerOptions {
    pub addr: SocketAddr,
    pub security: RdpServerSecurity,
    pub codecs: BitmapCodecs,
    pub max_request_size: u32,
    /// When `Some(max)`, each connection's acceptor adopts the desktop size the
    /// client requests in its Client Core Data (instead of the size reported by
    /// the display handler), negotiating that size from the start without a
    /// Deactivation-Reactivation resize. The request is clamped per dimension to
    /// `max` so an untrusted client can't drive the framebuffer/encoder
    /// allocation past that ceiling. `None` (the default) always enforces the
    /// server-provided size. Set via
    /// [`RdpServerBuilder::with_honor_client_desktop_size`](crate::RdpServerBuilder::with_honor_client_desktop_size).
    pub honor_client_desktop_size: Option<DesktopSize>,
    /// When `true`, a new connection accepted while [`RdpServer::run`] is
    /// already serving another one PREEMPTS it: once the newcomer has
    /// **completed authentication**, the existing connection is told why it is
    /// going away and dropped, and the newcomer is served in its place.
    ///
    /// [`RdpServer`] serves one connection at a time. By default a second
    /// connection accepted while one is live is left unserved in the OS listen
    /// backlog — from that client's point of view, a silent hang until the
    /// first session ends. That queue-behind behaviour suits a server expecting
    /// many short-lived connections, but not one backing a single specific
    /// session (e.g. mirroring one desktop), where a newly connecting client
    /// should replace a stale or abandoned one.
    ///
    /// # Security — what a candidate must clear, per mode
    ///
    /// Evicting a live session is disruptive, so a candidate runs the FULL
    /// negotiation — and, under [`RdpServerSecurity::Hybrid`], CredSSP/NLA —
    /// before the live session is touched at all. A candidate that fails at any
    /// step leaves the live session untouched.
    ///
    /// How strong that bar actually is depends entirely on the security mode,
    /// because only `Hybrid` authenticates the *client* before the point a
    /// candidate reaches. **Read this table before enabling the option:**
    ///
    /// | Security mode | Bar to preempt | Guarantee |
    /// |---|---|---|
    /// | [`Hybrid`](RdpServerSecurity::Hybrid) | CredSSP/NLA succeeds | an unauthenticated peer can never evict |
    /// | [`Tls`](RdpServerSecurity::Tls) | a TLS handshake — which authenticates the *server* to the client, not the reverse | **none against an unauthenticated peer**: any peer that can reach the port clears it, and a [`CredentialValidator`] does not run until finalization |
    /// | [`None`](RdpServerSecurity::None) | a well-formed X.224 Connection Request | **none**: this mode authenticates nothing |
    ///
    /// So under `Tls` and `None` an unauthenticated peer CAN evict an
    /// authenticated session, repeatedly — the anti-storm cooldown bars the
    /// victim, never the attacker. A warning is logged at startup in that case.
    /// If you need takeover to be authentication-gated, use `Hybrid`; if you
    /// must enable it under another mode, restrict who may attempt one with
    /// [`ConnectionHandler::on_accept`].
    ///
    /// Candidates are additionally gated through
    /// [`ConnectionHandler::on_accept`] *before* they are allowed to
    /// negotiate, so an IP allowlist or rate limiter bounds who may even
    /// attempt a takeover. It does NOT bound how long one admitted candidate
    /// can occupy the (single) negotiation slot before another is even
    /// considered — see the limitation documented on
    /// `CANDIDATE_NEGOTIATION_TIMEOUT`.
    ///
    /// Defaults to `false` (queue-behind, the pre-existing behaviour). Set via
    /// [`RdpServerBuilder::with_preempt_existing_session`](crate::RdpServerBuilder::with_preempt_existing_session).
    pub preempt_existing_session: bool,
    /// Quantization values the RemoteFX encoder uses once selected. Defaults
    /// to [`Quant::default`], the same values Windows RDP servers send. Set
    /// via
    /// [`RdpServerBuilder::with_remotefx_quant`](crate::RdpServerBuilder::with_remotefx_quant).
    pub remotefx_quant: Quant,
    /// Preferred RemoteFX entropy coder. If the client's advertised
    /// TS_RFX_ICAP array includes it, the server uses it; otherwise the
    /// server falls back to whichever coder the client offered first.
    /// `None` (the default) always uses whichever coder is offered first,
    /// since [MS-RDPRFX] 3.1.5.1 has the server arbitrarily pick one
    /// supported TS_RFX_ICAP element rather than rank the array as a
    /// preference order. Set via
    /// [`RdpServerBuilder::with_remotefx_entropy_coder`](crate::RdpServerBuilder::with_remotefx_entropy_coder).
    ///
    /// [MS-RDPRFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdprfx/
    pub remotefx_entropy_coder: Option<EntropyBits>,
}

impl RdpServerOptions {
    /// Default [MultifragmentUpdate] max reassembly buffer size (8 MB).
    ///
    /// Advertised to the client during capability exchange as the largest
    /// reassembled Fast-Path Update the server can accept.
    /// Values that are too large cause certain clients (notably mstsc)
    /// to reject the connection.
    ///
    /// [MultifragmentUpdate]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/01717954-716a-424d-af35-28fb2b86df89
    pub(crate) const DEFAULT_MAX_REQUEST_SIZE: u32 = 8 * 1024 * 1024;

    fn has_image_remote_fx(&self) -> bool {
        self.codecs
            .0
            .iter()
            .any(|codec| matches!(codec.property, CodecProperty::ImageRemoteFx(_)))
    }

    fn has_remote_fx(&self) -> bool {
        self.codecs
            .0
            .iter()
            .any(|codec| matches!(codec.property, CodecProperty::RemoteFx(_)))
    }

    #[cfg(feature = "qoi")]
    fn has_qoi(&self) -> bool {
        self.codecs
            .0
            .iter()
            .any(|codec| matches!(codec.property, CodecProperty::Qoi))
    }

    #[cfg(feature = "qoiz")]
    fn has_qoiz(&self) -> bool {
        self.codecs
            .0
            .iter()
            .any(|codec| matches!(codec.property, CodecProperty::QoiZ))
    }

    #[cfg(feature = "nscodec")]
    fn has_nscodec(&self) -> bool {
        self.codecs
            .0
            .iter()
            .any(|codec| matches!(codec.property, CodecProperty::NsCodec(_)))
    }
}

/// Picks a RemoteFX entropy coder out of the client's advertised TS_RFX_ICAP
/// array. Returns `preferred` if the client offered it, otherwise the first
/// coder the client offered. Returns `None` if `offered` is empty.
pub fn pick_remotefx_entropy_coder(
    preferred: Option<EntropyBits>,
    offered: impl Iterator<Item = EntropyBits>,
) -> Option<EntropyBits> {
    let mut first = None;

    for entropy_bits in offered {
        if first.is_none() {
            first = Some(entropy_bits);
        }

        if preferred == Some(entropy_bits) {
            return Some(entropy_bits);
        }
    }

    first
}

#[derive(Clone)]
pub enum RdpServerSecurity {
    None,
    Tls(TlsAcceptor),
    /// Used for both hybrid + hybrid-ex.
    Hybrid((TlsAcceptor, Vec<u8>)),
}

impl RdpServerSecurity {
    pub fn flag(&self) -> nego::SecurityProtocol {
        match self {
            RdpServerSecurity::None => nego::SecurityProtocol::empty(),
            RdpServerSecurity::Tls(_) => nego::SecurityProtocol::SSL,
            RdpServerSecurity::Hybrid(_) => nego::SecurityProtocol::HYBRID | nego::SecurityProtocol::HYBRID_EX,
        }
    }
}

struct AInputHandler {
    handler: Arc<Mutex<Box<dyn RdpServerInputHandler>>>,
}

impl_as_any!(AInputHandler);

impl dvc::DvcProcessor for AInputHandler {
    fn channel_name(&self) -> &str {
        ironrdp_ainput::CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<dvc::DvcMessage>> {
        use ironrdp_ainput::{ServerPdu, VersionPdu};

        let pdu = ServerPdu::Version(VersionPdu::default());

        Ok(vec![Box::new(pdu)])
    }

    fn close(&mut self, _channel_id: u32) {}

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<dvc::DvcMessage>> {
        use ironrdp_ainput::ClientPdu;

        match decode(payload).map_err(|e| decode_err!(e))? {
            ClientPdu::Mouse(pdu) => {
                let handler = Arc::clone(&self.handler);
                task::spawn_blocking(move || {
                    handler.blocking_lock().mouse(pdu.into());
                });
            }
        }

        Ok(Vec::new())
    }
}

impl dvc::DvcServerProcessor for AInputHandler {}

struct DisplayControlBackend {
    display: Arc<Mutex<Box<dyn RdpServerDisplay>>>,
}

impl DisplayControlBackend {
    fn new(display: Arc<Mutex<Box<dyn RdpServerDisplay>>>) -> Self {
        Self { display }
    }
}

impl DisplayControlHandler for DisplayControlBackend {
    fn monitor_layout(&self, layout: DisplayControlMonitorLayout) {
        let display = Arc::clone(&self.display);
        task::spawn_blocking(move || display.blocking_lock().request_layout(layout));
    }
}

#[cfg(feature = "usb")]
struct ServerUsbManager {
    factory: Box<dyn DeviceFactory>,
    comp_iface_alloc: InterfaceAlloc,
    router: HashMap<DynamicChannelId, ServerUsbDevice>,
}

#[cfg(feature = "usb")]
struct ServerUsbDevice {
    lifecycle: Arc<UsbDeviceLifecycle>,
    /// Handle attached to a submitted request, so dropping the request cancels
    /// it. Kept per device rather than per message to keep [`ServerEvent`] small.
    handle: UsbDeviceHandle,
    pending: HashMap<RequestId, oneshot::Sender<CompletionData>>,
}

#[cfg(feature = "usb")]
impl ServerUsbManager {
    fn new(inner: Box<dyn DeviceFactory>) -> Self {
        Self {
            factory: inner,
            comp_iface_alloc: InterfaceAlloc::default(),
            router: HashMap::new(),
        }
    }
}

/// Selects who performs the TLS handshake for a connection accepted via
/// [`RdpServer::run_connection_with`].
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum TransportTls {
    /// IronRDP performs the TLS accept on the stream (standard TCP+TLS).
    Managed,
    /// The stream is already past TLS, terminated by a lower layer (e.g. a WSS
    /// terminator). IronRDP skips the TLS handshake. The caller MUST guarantee
    /// the transport is already encrypted; see the preconditions on
    /// [`RdpServer::run_connection_with`].
    AlreadyDone,
}

/// RDP Server
///
/// A server is created to listen for connections.
/// After the connection sequence is finalized using the provided security mechanism, the server can:
///  - receive display updates from a [`RdpServerDisplay`] and forward them to the client
///  - receive input events from a client and forward them to an [`RdpServerInputHandler`]
///
/// # Example
///
/// ```
/// use ironrdp_server::{RdpServer, RdpServerInputHandler, RdpServerDisplay, RdpServerDisplayUpdates};
///
///# use anyhow::Result;
///# use ironrdp_server::{DisplayUpdate, DesktopSize, KeyboardEvent, MouseEvent, ServerResult};
///# use tokio_rustls::TlsAcceptor;
///# struct NoopInputHandler;
///# impl RdpServerInputHandler for NoopInputHandler {
///#     fn keyboard(&mut self, _: KeyboardEvent) {}
///#     fn mouse(&mut self, _: MouseEvent) {}
///# }
///# struct NoopDisplay;
///# #[async_trait::async_trait]
///# impl RdpServerDisplay for NoopDisplay {
///#     async fn size(&mut self) -> DesktopSize {
///#         todo!()
///#     }
///#     async fn updates(&mut self) -> Result<Box<dyn RdpServerDisplayUpdates>> {
///#         todo!()
///#     }
///# }
///# async fn stub() -> ServerResult<()> {
/// fn make_tls_acceptor() -> TlsAcceptor {
///    /* snip */
///#    todo!()
/// }
///
/// fn make_input_handler() -> impl RdpServerInputHandler {
///    /* snip */
///#    NoopInputHandler
/// }
///
/// fn make_display_handler() -> impl RdpServerDisplay {
///    /* snip */
///#    NoopDisplay
/// }
///
/// let tls_acceptor = make_tls_acceptor();
/// let input_handler = make_input_handler();
/// let display_handler = make_display_handler();
///
/// let mut server = RdpServer::builder()
///     .with_addr(([127, 0, 0, 1], 3389))
///     .with_tls(tls_acceptor)
///     .with_input_handler(input_handler)
///     .with_display_handler(display_handler)
///     .build();
///
/// server.run().await;
/// Ok(())
///# }
/// ```
pub struct RdpServer {
    opts: RdpServerOptions,
    // FIXME: replace with a channel and poll/process the handler?
    handler: Arc<Mutex<Box<dyn RdpServerInputHandler>>>,
    display: Arc<Mutex<Box<dyn RdpServerDisplay>>>,
    static_channels: StaticChannelSet,
    static_channel_factories: Vec<Box<dyn StaticChannelFactory>>,
    sound_factory: Option<Box<dyn SoundServerFactory>>,
    cliprdr_factory: Option<Box<dyn CliprdrServerFactory>>,
    rdpei_factory: Option<Box<dyn RdpeiServerFactory>>,
    echo_handle: EchoServerHandle,
    #[cfg(feature = "egfx")]
    gfx_factory: Option<Box<dyn GfxServerFactory>>,
    #[cfg(feature = "egfx")]
    gfx_handle: Option<crate::gfx::GfxServerHandle>,
    #[cfg(feature = "usb")]
    usb_man: Option<ServerUsbManager>,
    ev_sender: mpsc::UnboundedSender<ServerEvent>,
    ev_receiver: Arc<Mutex<mpsc::UnboundedReceiver<ServerEvent>>>,
    creds: Option<Credentials>,
    credential_validator: Option<Arc<dyn CredentialValidator>>,
    local_addr: Option<SocketAddr>,
    autodetect: Option<AutoDetectManager>,
    connection_handler: Option<Box<dyn ConnectionHandler>>,
    /// Anti-storm net for [`RdpServerOptions::preempt_existing_session`]: the
    /// peer most recently EVICTED by a takeover, and when it last tried to
    /// come back.
    ///
    /// Telling the loser why it was evicted (see
    /// [`ServerEvent::EvictedByOtherConnection`]) is the real fix for the
    /// eviction loop, but whether a client honours it is client-dependent.
    /// This bounds the damage if one doesn't: a just-evicted peer may not
    /// immediately re-preempt, and each refused attempt RE-ARMS the window, so
    /// an automatic reconnect storm can never win the session back, while a
    /// human who closes the client and reconnects still can. Keyed on source
    /// IP, since the source port changes on every reconnect. Cleared once a
    /// session ends on its own terms rather than being replaced.
    recently_evicted: Option<EvictedPeer>,
    /// True while the client has sent `SuppressOutput { desktop_rect: None }`
    /// — the standard RDP "I don't need display updates right now" signal
    /// (mstsc raises it on window minimize). Cleared on
    /// `SuppressOutput { Some(rect) }` or `RefreshRectangle` (sent on
    /// refocus). Exposed via [`Self::display_suppressed_handle`] so display
    /// backends can hold a clone and skip frame emission while it's set —
    /// without this, a server keeps streaming high-bitrate
    /// EGFX/H.264 frames into a minimized client, which accumulates them
    /// and locks up its input dispatch for seconds on refocus while it
    /// chews through the backlog.
    display_suppressed: Arc<AtomicBool>,

    /// Latest NetworkAutoDetect round-trip time in milliseconds, or `u32::MAX`
    /// until the first measurement (and while auto-detect is disabled). Updated
    /// on each RTT Measure Response when auto-detect is enabled (see
    /// [`Self::enable_autodetect`]). Exposed via [`Self::autodetect_rtt_handle`]
    /// so display backends can read a fresh, frame-traffic-independent network
    /// RTT for flow control.
    autodetect_rtt: Arc<AtomicU32>,

    /// Session-lifetime lowest RTT in milliseconds (`baseRTT` per MS-RDPBCGR
    /// 2.2.14.1.5), or `u32::MAX` until the first measurement. Unlike
    /// [`Self::autodetect_rtt`], this never rises: it is the floor over the
    /// whole session, not a sliding-window figure, which is what makes
    /// `averageRTT - baseRTT` a queueing-delay signal rather than two
    /// unrelated latency numbers. Updated at the same point as
    /// [`Self::autodetect_rtt`]. Exposed via
    /// [`Self::autodetect_baseline_rtt_handle`].
    autodetect_baseline_rtt: Arc<AtomicU32>,

    /// Optional Server Auto-Reconnect Cookie (MS-RDPBCGR 2.2.4.2
    /// `ARC_SC_PRIVATE_PACKET`). When `Some`, the server validates a returning
    /// `ARC_CS_PRIVATE_PACKET`, replaces its random after every connection, and
    /// sends hourly updates to the active client. This requires TLS or Hybrid
    /// security, which provides the all-zero client random required for Enhanced
    /// RDP Security. `None` (the default) disables automatic reconnection.
    /// Configure it on the builder
    /// ([`RdpServer::builder`]) via `with_auto_reconnect_cookie`, or after
    /// construction via [`Self::set_auto_reconnect_cookie`].
    auto_reconnect_cookie: Option<rdp::session_info::ServerAutoReconnect>,
    /// The cookie replaced by the current one, accepted until the next rotation.
    ///
    /// A successful socket write does not prove the client received the
    /// replacement. Retaining one previous value lets a client that disconnects
    /// during that window reconnect with the last cookie it knows.
    previous_auto_reconnect_cookie: Option<rdp::session_info::ServerAutoReconnect>,
    /// Tracks whether the current cookie has reached a client. Subsequent
    /// connections and hourly updates replace it with a new random.
    auto_reconnect_sent: bool,
}

/// Cloneable handle for updating the Server Auto-Reconnect Cookie while
/// [`RdpServer::run`] owns the server.
#[derive(Clone)]
pub struct AutoReconnectCookieHandle {
    sender: mpsc::UnboundedSender<ServerEvent>,
}

impl AutoReconnectCookieHandle {
    /// Queue a replacement cookie for the active client or the next connection.
    ///
    /// The change takes effect only after the server handles this event. `None`
    /// then disables auto-reconnect and invalidates every cookie currently held
    /// by the server.
    pub fn set(
        &self,
        cookie: Option<rdp::session_info::ServerAutoReconnect>,
    ) -> Result<(), mpsc::error::SendError<ServerEvent>> {
        self.sender.send(ServerEvent::SetAutoReconnectCookie(cookie))
    }
}

pub enum ServerEvent {
    Quit(String),
    /// End this connection because an authenticated candidate is taking the
    /// session over — a preemption, not a plain quit.
    ///
    /// Unlike [`Self::Quit`], this sends a Server Set Error Info PDU carrying
    /// `ERRINFO_DISCONNECTED_BY_OTHERCONNECTION` (MS-RDPBCGR 2.2.5.1.1 — the
    /// code real Windows RDS uses for a session takeover) before
    /// disconnecting. That distinction is load-bearing rather than cosmetic
    /// whenever a Server Auto-Reconnect Cookie is in play: a client dropped
    /// with no explanation auto-reconnects a second later, re-preempts the
    /// client that replaced it, and the two ping-pong indefinitely. Telling
    /// the loser WHY it was disconnected is what makes it stay away.
    EvictedByOtherConnection,
    Clipboard(ClipboardMessage),
    Rdpsnd(RdpsndServerMessage),
    Echo(EchoServerMessage),
    SetCredentials(Credentials),
    /// Replace or clear the Server Auto-Reconnect Cookie.
    SetAutoReconnectCookie(Option<rdp::session_info::ServerAutoReconnect>),
    GetLocalAddr(oneshot::Sender<Option<SocketAddr>>),
    #[cfg(feature = "egfx")]
    Egfx(EgfxServerMessage),
    /// Trigger an RTT measurement probe (requires auto-detect enabled).
    AutoDetectRttRequest,
    #[cfg(feature = "usb")]
    Usb(UrbdrcServerMessage),
}

/// Creates a fresh static-channel processor for each accepted RDP connection.
///
/// Factories are invoked before the Basic Settings Exchange so their channels
/// participate in GCC static-channel negotiation.
pub trait StaticChannelFactory: Send {
    /// Attaches the connection-local static-channel processor to `acceptor`.
    fn attach(&self, acceptor: &mut Acceptor);
}

impl fmt::Debug for ServerEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Quit(reason) => f.debug_tuple("Quit").field(reason).finish(),
            Self::EvictedByOtherConnection => f.write_str("EvictedByOtherConnection"),
            Self::Clipboard(..) => f.write_str("Clipboard(..)"),
            Self::Rdpsnd(..) => f.write_str("Rdpsnd(..)"),
            Self::Echo(..) => f.write_str("Echo(..)"),
            Self::SetCredentials(..) => f.write_str("SetCredentials(..)"),
            Self::SetAutoReconnectCookie(Some(..)) => f.write_str("SetAutoReconnectCookie(Some(..))"),
            Self::SetAutoReconnectCookie(None) => f.write_str("SetAutoReconnectCookie(None)"),
            Self::GetLocalAddr(..) => f.write_str("GetLocalAddr(..)"),
            #[cfg(feature = "egfx")]
            Self::Egfx(..) => f.write_str("Egfx(..)"),
            #[cfg(feature = "usb")]
            Self::Usb(..) => f.write_str("Usb(..)"),
            Self::AutoDetectRttRequest => f.write_str("AutoDetectRttRequest"),
        }
    }
}

pub trait ServerEventSender {
    fn set_sender(&mut self, sender: mpsc::UnboundedSender<ServerEvent>);
}

impl ServerEvent {
    pub fn create_channel() -> (mpsc::UnboundedSender<Self>, mpsc::UnboundedReceiver<Self>) {
        mpsc::unbounded_channel()
    }
}

/// The in-flight [`negotiate_candidate`] call for a candidate connection, or a
/// never-resolving placeholder while none is being negotiated. Borrows the
/// [`NegotiationContext`] the race built.
type PreemptProbe<'ctx> =
    core::pin::Pin<Box<dyn Future<Output = Option<(Box<NegotiatedCandidate>, SocketAddr)>> + 'ctx>>;

/// What resolved first while a session was live, under
/// [`RdpServerOptions::preempt_existing_session`]: the session itself ending, a
/// new inbound connection, or the verdict on a [`PreemptProbe`] being
/// negotiated. The race's `select!` yields one of these and must not
/// otherwise mutate the probe slot, whose futures it still borrows.
enum PreemptRace {
    Ended(ServerResult<()>),
    Accepted(std::io::Result<(TcpStream, SocketAddr)>),
    Probed(Option<(Box<NegotiatedCandidate>, SocketAddr)>),
}

#[derive(Debug, PartialEq)]
enum RunState {
    Continue,
    Disconnect,
    DeactivationReactivation { desktop_size: DesktopSize },
}

/// Which transport a connection ended up on after
/// [`negotiate_and_authenticate`], carrying the framed stream in the shape the
/// finalize step needs. The three variants exist to preserve the three
/// pre-existing finalize behaviours exactly.
enum NegotiatedTransport<S> {
    /// Never upgraded ([`RdpServerSecurity::None`]): finalize without a
    /// stream shutdown, matching the old `BeginResult::Continue` arm.
    Continued(TokioFramed<S>),
    /// Upgraded in-band by us ([`TransportTls::Managed`]).
    Tls(Box<TokioFramed<tokio_rustls::server::TlsStream<S>>>),
    /// Already past TLS at a lower layer ([`TransportTls::AlreadyDone`]).
    Offloaded(TokioFramed<S>),
}

/// A freshly built [`Acceptor`], paired with the exact [`RdpServerSecurity`]
/// it was constructed from.
///
/// Negotiation needs both together: [`Acceptor::new`] takes `security.flag()`
/// up front, and the TLS-upgrade step later needs the full `security` value
/// again (for the [`TlsAcceptor`] and, under Hybrid, the CredSSP public key).
/// Threading `security` and `acceptor` as independent parameters — which an
/// earlier revision of this refactor did — turns that pairing into a
/// caller-enforced precondition: nothing stops a future caller from passing
/// an `Acceptor` built from a *different* `RdpServerSecurity`, and the
/// failure mode is a panic on a server connection path
/// (`RdpServerSecurity::None => unreachable!()`, below). The single
/// constructor here makes the pairing a construction-time guarantee instead:
/// there is no way to reach [`Self::negotiate_and_authenticate`] with a
/// mismatched pair, because there is no way to build a `PendingConnection`
/// without going through [`Self::new`], which ties them together atomically.
///
/// Owns a cloned `RdpServerSecurity` rather than borrowing `&self.opts.security`
/// — `RdpServerSecurity` is a cheap `Clone` (its `TlsAcceptor` is an `Arc`
/// underneath) — deliberately: a caller in `run_connection_with` needs `&mut
/// self` (for `attach_channels`) while a live `PendingConnection` is still in
/// scope, which a borrowed `security` would conflict with for as long as the
/// pending connection exists.
struct PendingConnection {
    security: RdpServerSecurity,
    acceptor: Acceptor,
}

impl PendingConnection {
    fn new(
        security: RdpServerSecurity,
        desktop_size: DesktopSize,
        capabilities: Vec<CapabilitySet>,
        creds: Option<Credentials>,
        honor_client_desktop_size: Option<DesktopSize>,
    ) -> Self {
        let mut acceptor = Acceptor::new(security.flag(), desktop_size, capabilities, creds);
        acceptor.set_honor_client_desktop_size(honor_client_desktop_size);
        Self { security, acceptor }
    }

    /// Mutable access to the acceptor for the one thing that must happen
    /// before negotiation: attaching static/dynamic channels. Negotiation
    /// itself (`negotiate_and_authenticate`) owns the acceptor from here on,
    /// so this is only available pre-negotiation.
    fn acceptor_mut(&mut self) -> &mut Acceptor {
        &mut self.acceptor
    }

    /// Negotiate `stream` and, where the security mode provides it,
    /// AUTHENTICATE it — everything up to (but not including)
    /// `accept_finalize`.
    ///
    /// Consumes `self` rather than taking `&mut self`, so it can be driven
    /// without holding a mutable borrow of the whole server for the
    /// duration — a future caller (a preempting connection negotiating
    /// concurrently with the live one) needs exactly that.
    ///
    /// `Ok(None)` means the TLS handshake failed and was already logged — the
    /// caller should abandon the connection quietly rather than treat it as a
    /// connection error (preserving the pre-existing `return Ok(())` behaviour).
    async fn negotiate_and_authenticate<S>(
        self,
        stream: S,
        tls: TransportTls,
    ) -> ServerResult<Option<NegotiatedConnection<S>>>
    where
        S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
    {
        let PendingConnection { security, mut acceptor } = self;
        let security = &security;
        let framed = TokioFramed::new(stream);

        let res = ironrdp_acceptor::accept_begin(framed, &mut acceptor)
            .await
            .map_err_kind("accept_begin failed", ServerErrorKind::Connector)?;

        match res {
            // The only thing that varies between the two modes is who performs
            // the TLS handshake; everything past it is `complete_security_upgrade`.
            BeginResult::ShouldUpgrade(stream) => match tls {
                TransportTls::Managed => {
                    // `RdpServerSecurity::None` can never reach this arm: `Self::new`
                    // built `acceptor` from THIS `security` via `security.flag()`,
                    // which is empty only for `None`, and `accept_begin` yields
                    // `ShouldUpgrade` only when the negotiated flags are non-empty
                    // -- `None` always yields `Continue` instead (the arm below).
                    let tls_acceptor = match security {
                        RdpServerSecurity::Tls(acceptor) => acceptor,
                        RdpServerSecurity::Hybrid((acceptor, _)) => acceptor,
                        RdpServerSecurity::None => unreachable!(),
                    };
                    let accept = match tls_acceptor.accept(stream).await {
                        Ok(accept) => accept,
                        Err(e) => {
                            warn!("Failed to TLS accept: {}", e);
                            return Ok(None);
                        }
                    };
                    let mut framed = TokioFramed::new(accept);
                    complete_security_upgrade(security, &mut framed, &mut acceptor).await?;
                    Ok(Some(NegotiatedConnection {
                        transport: NegotiatedTransport::Tls(Box::new(framed)),
                        acceptor,
                    }))
                }
                // The stream is already past TLS (terminated at a lower
                // layer, e.g. a WSS terminator); do NOT call
                // tls_acceptor.accept on it.
                TransportTls::AlreadyDone => {
                    let mut framed = TokioFramed::new(stream);
                    complete_security_upgrade(security, &mut framed, &mut acceptor).await?;
                    Ok(Some(NegotiatedConnection {
                        transport: NegotiatedTransport::Offloaded(framed),
                        acceptor,
                    }))
                }
            },

            BeginResult::Continue(framed) => Ok(Some(NegotiatedConnection {
                transport: NegotiatedTransport::Continued(framed),
                acceptor,
            })),
        }
    }
}

/// The result of [`PendingConnection::negotiate_and_authenticate`]: the
/// [`NegotiatedTransport`] it landed on, bundled with the same [`Acceptor`]
/// that negotiated it (rather than the caller tracking the two as separate
/// values, which is how this looked before `PendingConnection` existed).
struct NegotiatedConnection<S> {
    transport: NegotiatedTransport<S>,
    acceptor: Acceptor,
}

/// Advance a stream that is now past the security upgrade: mark the acceptor
/// accordingly and, under [`RdpServerSecurity::Hybrid`], run the CredSSP
/// exchange.
///
/// Generic over the stream so both [`TransportTls`] modes can call this one
/// definition of the exchange (the two differ only in what the framed stream
/// wraps) — restoring, not introducing, the single-call-site property the
/// pre-existing `finalize_after_upgrade` already had for the same two arms
/// before this refactor split negotiation out of it. The actual reason this
/// exists as its own function is [`negotiate_and_authenticate`]'s: a future
/// caller (a preempting connection negotiating without holding `&mut self`)
/// needs the CredSSP step available from a plain function it can drive
/// itself, not bundled into a `&mut self` method.
async fn complete_security_upgrade<S>(
    security: &RdpServerSecurity,
    framed: &mut TokioFramed<S>,
    acceptor: &mut Acceptor,
) -> ServerResult<()>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    acceptor.mark_security_upgrade_as_done();

    if let RdpServerSecurity::Hybrid((_, pub_key)) = security {
        // Generic streams don't expose peer address. Use a neutral
        // placeholder; it's unclear whether CredSSP/NTLM actually
        // uses this value in practice.
        let client_name = "rdp-client".to_owned();

        ironrdp_acceptor::accept_credssp(
            framed,
            acceptor,
            &mut ironrdp_tokio::reqwest::ReqwestNetworkClient::new(),
            client_name.into(),
            pub_key.clone(),
            None,
        )
        .await
        .map_err_kind("accept_credssp", ServerErrorKind::Connector)?;
    }

    Ok(())
}

/// How long an evicted session gets to send its
/// `ERRINFO_DISCONNECTED_BY_OTHERCONNECTION` and wind down on its own before
/// it is cancelled outright. Short, because the preempting client is already
/// authenticated and waiting and a half-dead peer must not stall the takeover;
/// exceeding it is not an error, it just degrades to an abrupt drop.
const EVICTION_GRACE: Duration = Duration::from_millis(750);

/// How long a candidate gets to complete negotiation and authentication before
/// it is abandoned.
///
/// This bound is load-bearing, not a tidiness measure: `negotiate_candidate`
/// blocks on socket reads, so without it a peer that completes the TCP
/// handshake and then sends NOTHING parks the probe forever — which stalls
/// accepts for the rest of the session (the accept arm is gated on `!probing`)
/// and, once the live session ends, hangs the whole accept loop on the handoff
/// await with no way left to observe [`ServerEvent::Quit`]. Generous enough for
/// TLS + CredSSP over a slow link, which is sub-second on a healthy one.
///
/// # Known limitation: one candidate is negotiated at a time
///
/// `run()`'s race holds a SINGLE probe slot (`probe`/`probing`), so a peer
/// that has already cleared [`ConnectionHandler::on_accept`] and then merely
/// STALLS its handshake (a well-formed X.224 Connection Request, then
/// silence before TLS — `on_accept` cannot see this in advance, since it
/// already returned `true` for this peer) occupies the slot for up to this
/// whole timeout, during which the accept arm is gated off and no OTHER
/// candidate — including a legitimate one — can even begin negotiating. The
/// live session itself is unaffected either way (this only withholds
/// PREEMPTION, never breaks it), and it is not a regression against master's
/// queue-behind default. But it means `on_accept` bounds who may ATTEMPT a
/// takeover, not how long one attempt can hold up every other. Closing this
/// properly needs a small pool of concurrent probe slots rather than one;
/// not done here.
const CANDIDATE_NEGOTIATION_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the accept loop will wait, AFTER the live session has ended, for a
/// candidate that is still mid-negotiation.
///
/// Deliberately short and separate from [`CANDIDATE_NEGOTIATION_TIMEOUT`]:
/// while this wait is in progress the loop is servicing nothing — no accepts,
/// no [`ServerEvent`]s, not even [`ServerEvent::Quit`] — so it is the window in
/// which an unauthenticated peer can make the server look hung. A candidate
/// that cannot finish within it is dropped and simply reconnects; holding the
/// whole listener for it is the worse trade.
const CANDIDATE_HANDOFF_GRACE: Duration = Duration::from_millis(750);

/// How long a just-evicted peer is barred from preempting its way back in —
/// see [`RdpServer::recently_evicted`].
const REPREEMPT_COOLDOWN: Duration = Duration::from_secs(5);

/// Absolute cap on that bar, measured from the eviction itself.
///
/// [`refuse_reconnect_from_evicted`] re-arms its window on every refused
/// attempt, which is what stops an auto-reconnect storm from winning the
/// session back. Left uncapped that also permanently locks out the feature's
/// own headline case — a client whose link dropped, whose stale session is
/// still live, and which is auto-reconnecting to reclaim it. Past this cap the
/// bar lifts even under a continuing storm; by then the evicted peer has had
/// its `ERRINFO_DISCONNECTED_BY_OTHERCONNECTION` (the real fix for the loop),
/// and this heuristic has served its purpose as a backstop.
const REPREEMPT_MAX_LOCKOUT: Duration = Duration::from_secs(30);

/// Does this security mode authenticate the CLIENT before a candidate reaches
/// the point where it could evict the live session?
///
/// Only [`RdpServerSecurity::Hybrid`] does: CredSSP/NLA runs inside
/// [`negotiate_candidate`]. A `Tls` handshake authenticates the *server* to the
/// client, not the reverse, and any peer that can reach the port completes one;
/// `None` authenticates nothing. Under those two, preemption's bar is therefore
/// NOT authentication — see the security section on
/// [`RdpServerOptions::preempt_existing_session`], and the startup warning in
/// [`RdpServer::run`].
fn authenticates_before_eviction(security: &RdpServerSecurity) -> bool {
    matches!(security, RdpServerSecurity::Hybrid(_))
}

/// A peer barred from preempting straight back after being evicted, and when
/// that bar started — see [`refuse_reconnect_from_evicted`].
#[derive(Debug, Clone, Copy)]
struct EvictedPeer {
    ip: IpAddr,
    /// When the eviction happened; bounds the lockout via [`REPREEMPT_MAX_LOCKOUT`].
    evicted_at: Instant,
    /// Most recent refused attempt; re-armed to throttle a reconnect storm.
    last_try: Instant,
}

/// Should this candidate be refused because it is the peer that was just
/// evicted, bouncing straight back to retake the session?
///
/// Each refused attempt RE-ARMS the window, so a client auto-reconnecting on a
/// ~1 s cadence keeps resetting its own cooldown and cannot immediately win the
/// session back, while a human who closes the client and reconnects — a gap
/// beyond `cooldown` — still can.
///
/// The re-arm is bounded by `max_lockout` from the eviction, so a peer that
/// keeps retrying is eventually let back in rather than barred forever. Without
/// that cap this locks out exactly the case the feature exists for: a client
/// whose network dropped, auto-reconnecting to reclaim its own stale session.
///
/// Keyed on source IP, because the source port changes on every reconnect. The
/// cost is that two clients behind one NAT briefly share a bar; the cap bounds
/// how long that lasts.
fn refuse_reconnect_from_evicted(
    recently_evicted: &mut Option<EvictedPeer>,
    peer: IpAddr,
    now: Instant,
    cooldown: Duration,
    max_lockout: Duration,
) -> bool {
    match recently_evicted {
        Some(evicted) if evicted.ip == peer => {
            let within_cooldown = now.duration_since(evicted.last_try) < cooldown;
            let within_cap = now.duration_since(evicted.evicted_at) < max_lockout;
            let refuse = within_cooldown && within_cap;
            if refuse {
                evicted.last_try = now;
            }
            refuse
        }
        _ => false,
    }
}

/// A cheap, cloned snapshot of everything a preempting candidate needs to
/// negotiate, so it can do so WITHOUT `&mut self` and therefore concurrently
/// with the live connection's borrow. Built per race via
/// [`RdpServer::negotiation_context`].
///
/// Deliberately does NOT carry the channel factories: a candidate builds no
/// backends, because it may never be served (see the note in
/// [`negotiate_candidate`]). Only the winner does, in
/// [`RdpServer::serve_negotiated`], from `self`.
struct NegotiationContext {
    opts: RdpServerOptions,
    creds: Option<Credentials>,
    display: Arc<Mutex<Box<dyn RdpServerDisplay>>>,
}

/// A candidate that has negotiated AND authenticated, and so has earned the
/// right to evict the live session. Everything needed to resume at
/// finalization, which [`RdpServer::serve_negotiated`] does once it wins --
/// just [`PendingConnection::negotiate_and_authenticate`]'s own result type,
/// named for what it means in this context.
type NegotiatedCandidate = NegotiatedConnection<TcpStream>;

/// Negotiate and authenticate a candidate connection against a cloned `ctx`,
/// touching no `&mut self` — which is what lets this run inside
/// [`RdpServer::run`]'s preemption race, concurrently with the live
/// connection.
///
/// Returns `Some` only once the candidate has genuinely earned the session:
/// negotiation completed and, where the security mode provides it,
/// authentication succeeded (see the table on
/// [`RdpServerOptions::preempt_existing_session`]). On any failure — a
/// malformed or non-RDP handshake, TLS rejected, CredSSP rejected — returns
/// `None` and the live session is left completely undisturbed.
async fn negotiate_candidate(
    ctx: &NegotiationContext,
    stream: TcpStream,
    peer: SocketAddr,
) -> Option<(Box<NegotiatedCandidate>, SocketAddr)> {
    let size = ctx.display.lock().await.size().await;
    let capabilities = capabilities::capabilities(&ctx.opts, size);
    let pending = PendingConnection::new(
        ctx.opts.security.clone(),
        size,
        capabilities,
        ctx.creds.clone(),
        ctx.opts.honor_client_desktop_size,
    );

    // NOTE: deliberately NO channel attachment here. Building the cliprdr /
    // sound / gfx backends means running user-supplied factories for a peer
    // that has not authenticated yet — a port scan would construct and tear
    // down backends alongside the live session's, and those factories may claim
    // exclusive OS resources (an audio capture device, clipboard ownership).
    // The winner attaches its own channels in `RdpServer::serve_negotiated`,
    // which is still before `accept_finalize` — the acceptor does not consume
    // the static channel set until it processes the MCS Connect Initial, which
    // happens there, not in `accept_begin`.

    match pending.negotiate_and_authenticate(stream, TransportTls::Managed).await {
        Ok(Some(negotiated)) => {
            debug!(?peer, "candidate authenticated -- eligible to preempt the live session");
            Some((Box::new(negotiated), peer))
        }
        Ok(None) => {
            debug!(
                ?peer,
                "candidate TLS handshake failed -- not preempting the live session"
            );
            None
        }
        Err(error) => {
            debug!(
                ?peer,
                %error,
                "candidate did not negotiate/authenticate -- not preempting the live session"
            );
            None
        }
    }
}

/// [`negotiate_candidate`] under a hard deadline.
///
/// The negotiation blocks on socket reads from an as-yet-unauthenticated peer,
/// so it MUST NOT be awaited unbounded anywhere in the accept loop: a peer that
/// connects and then says nothing would otherwise stall accepts for the rest of
/// the session and hang the loop outright once the session ended. Timing out is
/// treated exactly like a failed negotiation — the candidate is dropped and the
/// live session is untouched.
async fn negotiate_candidate_bounded(
    ctx: &NegotiationContext,
    stream: TcpStream,
    peer: SocketAddr,
) -> Option<(Box<NegotiatedCandidate>, SocketAddr)> {
    match tokio::time::timeout(CANDIDATE_NEGOTIATION_TIMEOUT, negotiate_candidate(ctx, stream, peer)).await {
        Ok(candidate) => candidate,
        Err(_) => {
            debug!(
                ?peer,
                timeout = ?CANDIDATE_NEGOTIATION_TIMEOUT,
                "candidate did not finish negotiating in time -- abandoning it, the live session is untouched"
            );
            None
        }
    }
}

impl RdpServer {
    #[expect(
        clippy::too_many_arguments,
        reason = "called via the builder; positional parameters are an internal detail"
    )]
    pub(crate) fn new(
        opts: RdpServerOptions,
        handler: Box<dyn RdpServerInputHandler>,
        display: Box<dyn RdpServerDisplay>,
        static_channel_factories: Vec<Box<dyn StaticChannelFactory>>,
        mut sound_factory: Option<Box<dyn SoundServerFactory>>,
        mut cliprdr_factory: Option<Box<dyn CliprdrServerFactory>>,
        mut rdpei_factory: Option<Box<dyn RdpeiServerFactory>>,
        connection_handler: Option<Box<dyn ConnectionHandler>>,
        #[cfg(feature = "egfx")] mut gfx_factory: Option<Box<dyn GfxServerFactory>>,
        display_suppressed: Option<Arc<AtomicBool>>,
        #[cfg(feature = "usb")] usb_factory: Option<Box<dyn DeviceFactory>>,
        autodetect_rtt: Option<Arc<AtomicU32>>,
        autodetect_baseline_rtt: Option<Arc<AtomicU32>>,
    ) -> Self {
        let (ev_sender, ev_receiver) = ServerEvent::create_channel();
        if let Some(cliprdr) = cliprdr_factory.as_mut() {
            cliprdr.set_sender(ev_sender.clone());
        }
        if let Some(snd) = sound_factory.as_mut() {
            snd.set_sender(ev_sender.clone());
        }
        if let Some(rdpei) = rdpei_factory.as_mut() {
            rdpei.set_sender(ev_sender.clone());
        }
        #[cfg(feature = "egfx")]
        if let Some(gfx) = gfx_factory.as_mut() {
            gfx.set_sender(ev_sender.clone());
        }

        Self {
            opts,
            handler: Arc::new(Mutex::new(handler)),
            display: Arc::new(Mutex::new(display)),
            static_channels: StaticChannelSet::new(),
            static_channel_factories,
            sound_factory,
            cliprdr_factory,
            rdpei_factory,
            echo_handle: EchoServerHandle::new(ev_sender.clone()),
            #[cfg(feature = "egfx")]
            gfx_factory,
            #[cfg(feature = "egfx")]
            gfx_handle: None,
            #[cfg(feature = "usb")]
            usb_man: usb_factory.map(ServerUsbManager::new),
            ev_sender,
            ev_receiver: Arc::new(Mutex::new(ev_receiver)),
            creds: None,
            credential_validator: None,
            local_addr: None,
            autodetect: None,
            connection_handler,
            recently_evicted: None,
            display_suppressed: display_suppressed.unwrap_or_else(|| Arc::new(AtomicBool::new(false))),
            autodetect_rtt: {
                // Reset to the sentinel: an injected handle must not expose a stale value before the first measurement.
                let handle = autodetect_rtt.unwrap_or_else(|| Arc::new(AtomicU32::new(u32::MAX)));
                handle.store(u32::MAX, Ordering::Relaxed);
                handle
            },
            autodetect_baseline_rtt: {
                let handle = autodetect_baseline_rtt.unwrap_or_else(|| Arc::new(AtomicU32::new(u32::MAX)));
                handle.store(u32::MAX, Ordering::Relaxed);
                handle
            },
            auto_reconnect_cookie: None,
            previous_auto_reconnect_cookie: None,
            auto_reconnect_sent: false,
        }
    }

    pub fn builder() -> builder::RdpServerBuilder<builder::WantsAddr> {
        builder::RdpServerBuilder::new()
    }

    /// Set or clear the credential validator for TLS-mode connections.
    ///
    /// When set, credentials received from the client during
    /// `SecureSettingsExchange` are validated through this callback before
    /// the session is established. If the validator returns
    /// [`CredentialDecision::Reject`] (or a [`CredentialValidationError`]),
    /// the connection is rejected. Passing `None` clears any previously
    /// configured validator.
    ///
    /// A valid Server Auto-Reconnect Cookie bypasses this validator. Applications
    /// that must validate every connection should leave automatic reconnection
    /// disabled.
    ///
    /// Most callers should configure the validator at construction time via
    /// the builder's `with_credential_validator` method
    /// ([`RdpServer::builder`]); this setter exists for dynamic
    /// post-construction reconfiguration.
    ///
    /// Not used for CredSSP/Hybrid connections (those use pre-loaded credentials).
    pub fn set_credential_validator(&mut self, validator: Option<Arc<dyn CredentialValidator>>) {
        self.credential_validator = validator;
    }

    /// Set or clear the Server Auto-Reconnect Cookie (MS-RDPBCGR 2.2.4.2
    /// `ARC_SC_PRIVATE_PACKET`) handed to the client during logon.
    ///
    /// When set to `Some`, the server sends a Save Session Info PDU carrying the
    /// cookie right after activation. It verifies the returned
    /// `ARC_CS_PRIVATE_PACKET` using the HMAC-MD5 verifier required by
    /// MS-RDPBCGR 5.5, replaces the random after every accepted connection, and
    /// sends an update every hour. Automatic reconnection requires TLS or Hybrid
    /// security, which provides the all-zero client random required for Enhanced
    /// RDP Security. The [`ServerAutoReconnect`] `logon_id` identifies the
    /// session; the server generates replacement randoms with a CSPRNG.
    ///
    /// Pass `None` (the default) to send no cookie.
    ///
    /// Most callers should configure this at construction time via the builder
    /// ([`RdpServer::builder`])'s `with_auto_reconnect_cookie`. To replace a
    /// cookie while [`Self::run`] owns the server, use
    /// [`Self::auto_reconnect_cookie_handle`].
    ///
    /// [`ServerAutoReconnect`]: ironrdp_pdu::rdp::session_info::ServerAutoReconnect
    pub fn set_auto_reconnect_cookie(&mut self, cookie: Option<rdp::session_info::ServerAutoReconnect>) {
        self.auto_reconnect_cookie = cookie;
        self.previous_auto_reconnect_cookie = None;
        self.auto_reconnect_sent = false;
    }

    /// Returns a handle for replacing the cookie while [`Self::run`] owns this
    /// server.
    pub fn auto_reconnect_cookie_handle(&self) -> AutoReconnectCookieHandle {
        AutoReconnectCookieHandle {
            sender: self.ev_sender.clone(),
        }
    }

    fn supports_auto_reconnect(&self) -> bool {
        matches!(
            &self.opts.security,
            RdpServerSecurity::Tls(_) | RdpServerSecurity::Hybrid(_)
        )
    }

    fn verify_auto_reconnect_cookie(&self, reconnect: &rdp::client_info::ClientAutoReconnect) -> bool {
        if !self.supports_auto_reconnect() {
            return false;
        }

        [&self.auto_reconnect_cookie, &self.previous_auto_reconnect_cookie]
            .into_iter()
            .flatten()
            .any(|cookie| reconnect.verify(cookie))
    }

    fn generate_auto_reconnect_cookie(logon_id: u32) -> rdp::session_info::ServerAutoReconnect {
        let mut random_bits = [0; 16];
        rand::rng().fill_bytes(&mut random_bits);

        rdp::session_info::ServerAutoReconnect { logon_id, random_bits }
    }

    fn next_auto_reconnect_cookie(&self) -> Option<rdp::session_info::ServerAutoReconnect> {
        if !self.supports_auto_reconnect() {
            return None;
        }

        let cookie = self.auto_reconnect_cookie.as_ref()?;

        if self.auto_reconnect_sent {
            Some(Self::generate_auto_reconnect_cookie(cookie.logon_id))
        } else {
            Some(cookie.clone())
        }
    }

    fn commit_auto_reconnect_rotation(&mut self, cookie: rdp::session_info::ServerAutoReconnect) {
        if self.auto_reconnect_sent {
            self.previous_auto_reconnect_cookie = self.auto_reconnect_cookie.replace(cookie);
        } else {
            self.auto_reconnect_cookie = Some(cookie);
        }
        self.auto_reconnect_sent = true;
    }

    /// (vendored, divergence 23) Invalidate whatever ARC cookie belongs to
    /// the session that is about to be EVICTED, without disabling
    /// auto-reconnect for the server going forward.
    ///
    /// MS-RDPBCGR 5.5 requires a session's auto-reconnect cookie to be
    /// invalidated once a different client's session begins. This crate
    /// already demotes the outgoing cookie into `previous_auto_reconnect_
    /// cookie` on every normal rotation (`commit_auto_reconnect_rotation`) --
    /// a network-timing tolerance for the common case of a lost Save Session
    /// Info PDU -- which means an EVICTED peer's cookie stays valid for one
    /// more rotation. Since `verify_auto_reconnect_cookie` accepts either
    /// slot, and a client presenting a valid ARC cookie skips
    /// `credential_validator` (see `client_accepted`), an evicted peer could
    /// silently resume without a real re-authorization check -- and, because
    /// re-authenticating via ARC needs no user interaction, do so reliably
    /// the moment `REPREEMPT_MAX_LOCKOUT` lifts.
    ///
    /// Rotates to a FRESH cookie under the SAME `logon_id` (so the server
    /// keeps issuing cookies to whoever connects next -- a normal rotation
    /// does the same) but with new random bits, which is what actually
    /// invalidates the old one: `ClientAutoReconnect::verify` HMACs against
    /// `random_bits`, not `logon_id` alone. `previous_auto_reconnect_cookie`
    /// is discarded outright here rather than demoted into, since the whole
    /// point is that the evicted party's cookie must not remain valid even
    /// for one more attempt.
    ///
    /// MUST NOT set `auto_reconnect_cookie` to `None`:
    /// `next_auto_reconnect_cookie` treats `None` as "auto-reconnect is not
    /// configured" and stops issuing cookies to EVERY future connection, not
    /// just this one -- silently disabling the feature server-wide for the
    /// rest of the process (see `next_auto_reconnect_cookie`'s early
    /// `self.auto_reconnect_cookie.as_ref()?`). No-op if auto-reconnect isn't
    /// configured at all.
    fn invalidate_auto_reconnect_cookie_on_eviction(&mut self) {
        if let Some(current) = self.auto_reconnect_cookie.as_ref() {
            self.auto_reconnect_cookie = Some(Self::generate_auto_reconnect_cookie(current.logon_id));
        }
        self.previous_auto_reconnect_cookie = None;
    }
    async fn send_auto_reconnect_cookie(
        cookie: rdp::session_info::ServerAutoReconnect,
        writer: &mut impl FramedWrite,
        io_channel_id: u16,
        user_channel_id: u16,
    ) -> ServerResult<()> {
        let pdu = rdp::headers::ShareDataPdu::SaveSessionInfo(rdp::session_info::SaveSessionInfoPdu {
            info_type: rdp::session_info::InfoType::LogonExtended,
            info_data: rdp::session_info::InfoData::LogonExtended(rdp::session_info::LogonInfoExtended {
                present_fields_flags: rdp::session_info::LogonExFlags::AUTO_RECONNECT_COOKIE,
                auto_reconnect: Some(cookie),
                errors_info: None,
            }),
        });
        let data = encode_share_data_pdu(pdu, user_channel_id, io_channel_id, user_channel_id)?;
        writer
            .write_all(&data)
            .await
            .map_err(|e| ServerError::io("send auto-reconnect cookie", e))?;
        debug!("Sent Server Auto-Reconnect Cookie (Save Session Info PDU)");

        Ok(())
    }

    async fn send_next_auto_reconnect_cookie(
        &mut self,
        writer: &mut impl FramedWrite,
        io_channel_id: u16,
        user_channel_id: u16,
    ) -> ServerResult<()> {
        let Some(cookie) = self.next_auto_reconnect_cookie() else {
            return Ok(());
        };

        Self::send_auto_reconnect_cookie(cookie.clone(), writer, io_channel_id, user_channel_id).await?;
        self.commit_auto_reconnect_rotation(cookie);

        Ok(())
    }

    async fn rotate_auto_reconnect_cookie(
        &mut self,
        writer: &mut impl FramedWrite,
        io_channel_id: u16,
        user_channel_id: u16,
    ) -> ServerResult<()> {
        if !self.supports_auto_reconnect() {
            return Ok(());
        }

        let Some(cookie) = self.auto_reconnect_cookie.as_ref() else {
            return Ok(());
        };
        let cookie = Self::generate_auto_reconnect_cookie(cookie.logon_id);

        Self::send_auto_reconnect_cookie(cookie.clone(), writer, io_channel_id, user_channel_id).await?;
        self.commit_auto_reconnect_rotation(cookie);

        Ok(())
    }

    async fn update_auto_reconnect_cookie(
        &mut self,
        cookie: Option<rdp::session_info::ServerAutoReconnect>,
        writer: &mut impl FramedWrite,
        io_channel_id: u16,
        user_channel_id: u16,
    ) -> ServerResult<()> {
        let Some(cookie) = cookie else {
            self.set_auto_reconnect_cookie(None);
            return Ok(());
        };

        if !self.supports_auto_reconnect() {
            self.set_auto_reconnect_cookie(Some(cookie));
            return Ok(());
        }

        Self::send_auto_reconnect_cookie(cookie.clone(), writer, io_channel_id, user_channel_id).await?;
        self.auto_reconnect_cookie = Some(cookie);
        self.previous_auto_reconnect_cookie = None;
        self.auto_reconnect_sent = true;

        Ok(())
    }

    pub fn event_sender(&self) -> &mpsc::UnboundedSender<ServerEvent> {
        &self.ev_sender
    }

    #[cfg(feature = "usb")]
    fn remove_usb_device(&mut self, dvc_id: DynamicChannelId) {
        let Some(usb_man) = self.usb_man.as_mut() else {
            warn!("Missing USB device factory");
            return;
        };

        if let Some(device) = usb_man.router.remove(&dvc_id) {
            // Set the terminal state before dropping completion senders. A woken
            // PendingRequest must not enqueue CANCEL_REQUEST for a removed DVC.
            device.lifecycle.mark_closed();
            debug!(
                dvc_id,
                pending_requests = device.pending.len(),
                "Removed closed USB device from request router"
            );
        } else {
            trace!(dvc_id, "Closed USB device is absent from request router");
        }
    }

    /// Returns the shared "display suppressed" flag — `true` while the
    /// connected client has sent `SuppressOutput { desktop_rect: None }`
    /// (e.g., mstsc minimized).
    ///
    /// Display backends should hold a clone of this `Arc` and skip frame
    /// emission while it's set, so the client doesn't accumulate a backlog
    /// of frames it can't present until refocus. Cleared by the per-
    /// connection PDU handler on `SuppressOutput { Some(rect) }` or
    /// `RefreshRectangle`.
    ///
    /// **Caveat:** some clients (notably mstsc) send
    /// `SuppressOutput { desktop_rect: None }` during their connect
    /// handshake *before* their display surface is fully initialized; a
    /// backend that honors the flag blindly will block that first frame
    /// and leave the client with a half-initialized surface that doesn't
    /// recover on un-suppress (visible as a frozen desktop on first
    /// connect). Backends are advised to defer acting on the flag until
    /// after the first frame has been delivered to the client, and to
    /// debounce transient flaps (some clients pulse this PDU under wire
    /// pressure on heavy CPU/IO loads) — e.g., only engage the gate once
    /// the flag has been steady-`true` for ~1 s.
    ///
    /// The display backend typically needs to share this flag with the
    /// server before any client connects (so the same `Arc` is read by
    /// the backend's polling thread and written by the per-connection
    /// PDU handler). To inject the shared instance at construction time,
    /// use [`RdpServerBuilder::with_display_suppressed_handle`](crate::RdpServerBuilder::with_display_suppressed_handle).
    ///
    /// [crate::RdpServerBuilder]: crate::RdpServerBuilder
    pub fn display_suppressed_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.display_suppressed)
    }

    /// Returns a handle to the latest NetworkAutoDetect RTT in milliseconds
    /// (`u32::MAX` until the first measurement, and while auto-detect is
    /// disabled). The server updates it on each RTT Measure Response; backends
    /// clone the handle to read a fresh network RTT for flow control. Inject a
    /// shared instance at construction with
    /// [`RdpServerBuilder::with_autodetect_rtt_handle`](crate::RdpServerBuilder::with_autodetect_rtt_handle).
    pub fn autodetect_rtt_handle(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.autodetect_rtt)
    }

    /// Returns a handle to the session-lifetime lowest RTT in milliseconds
    /// (`baseRTT` per MS-RDPBCGR 2.2.14.1.5; `u32::MAX` until the first
    /// measurement, and while auto-detect is disabled). Unlike
    /// [`Self::autodetect_rtt_handle`], this figure never rises: pair it with
    /// that handle's average to derive queueing delay
    /// (`averageRTT - baseRTT`), which `autodetect_rtt_handle` alone cannot
    /// give since its figure is a sliding-window value that rises as low
    /// samples age out. Inject a shared instance at construction with
    /// [`RdpServerBuilder::with_autodetect_baseline_rtt_handle`](crate::RdpServerBuilder::with_autodetect_baseline_rtt_handle).
    pub fn autodetect_baseline_rtt_handle(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.autodetect_baseline_rtt)
    }

    /// Returns the shared ECHO server handle for runtime probe requests and RTT measurements.
    pub fn echo_handle(&self) -> &EchoServerHandle {
        &self.echo_handle
    }

    /// Enable protocol-level auto-detect ([MS-RDPBCGR 2.2.14]).
    ///
    /// Auto-detect uses lightweight Share Data PDUs on the IO channel,
    /// separate from the ECHO DVC. It supports bandwidth measurement
    /// in addition to RTT and works even when DVC is unavailable.
    ///
    /// Send probes via [`ServerEvent::AutoDetectRttRequest`] and
    /// query results with [`rtt_snapshot()`](Self::rtt_snapshot).
    pub fn enable_autodetect(&mut self) {
        self.autodetect = Some(AutoDetectManager::new());
    }

    /// Get the latest auto-detect RTT snapshot.
    ///
    /// Returns `None` if auto-detect is not enabled or no measurements
    /// have been received yet.
    pub fn rtt_snapshot(&self) -> Option<RttSnapshot> {
        self.autodetect.as_ref().and_then(|ad| ad.snapshot())
    }

    /// Returns the shared EGFX server handle for proactive frame submission.
    ///
    /// Available after `build_server_with_handle()` returns `Some` during
    /// channel setup. Display handlers use this to call
    /// `send_avc420_frame()` / `send_avc444_frame()` and then signal the
    /// event loop via `ServerEvent::Egfx`.
    #[cfg(feature = "egfx")]
    pub fn gfx_handle(&self) -> Option<&crate::gfx::GfxServerHandle> {
        self.gfx_handle.as_ref()
    }

    fn attach_channels(&mut self, acceptor: &mut Acceptor) {
        if let Some(cliprdr_factory) = self.cliprdr_factory.as_deref() {
            let backend = cliprdr_factory.build_cliprdr_backend();

            let cliprdr = CliprdrServer::new(backend);

            acceptor.attach_static_channel(cliprdr);
        }

        if let Some(factory) = self.sound_factory.as_deref() {
            let backend = factory.build_backend();

            acceptor.attach_static_channel(RdpsndServer::new(backend));
        }

        let dcs_backend = DisplayControlBackend::new(Arc::clone(&self.display));
        let dvc = dvc::DrdynvcServer::new()
            .with_dynamic_channel(AInputHandler {
                handler: Arc::clone(&self.handler),
            })
            .with_dynamic_channel(DisplayControlServer::new(Box::new(dcs_backend)));

        let dvc = {
            let echo_handle = self.echo_handle.clone();
            dvc.with_dynamic_channel(EchoDvcBridge::new(echo_handle))
        };

        let dvc = if let Some(factory) = self.rdpei_factory.as_deref() {
            dvc.with_dynamic_channel(factory.build_server())
        } else {
            dvc
        };

        #[cfg(feature = "egfx")]
        let dvc = {
            let mut dvc = dvc;
            if let Some(gfx_factory) = self.gfx_factory.as_deref() {
                if let Some((bridge, handle)) = gfx_factory.build_server_with_handle() {
                    self.gfx_handle = Some(handle);
                    dvc = dvc.with_dynamic_channel(bridge);
                } else {
                    let handler = gfx_factory.build_gfx_handler();
                    let gfx_server = ironrdp_egfx::server::GraphicsPipelineServer::new(handler);
                    dvc = dvc.with_dynamic_channel(gfx_server);
                }
            }
            dvc
        };

        #[cfg(feature = "usb")]
        let dvc = {
            let mut dvc = dvc;
            if self.usb_man.is_some() {
                dvc = dvc.with_dynamic_channel(UrbdrcControlServer::new(Box::new(UsbControlHandle::new(
                    self.ev_sender.clone(),
                ))));
            }
            dvc
        };

        acceptor.attach_static_channel(dvc);

        for factory in &self.static_channel_factories {
            factory.attach(acceptor);
        }
    }

    /// Drop every event still queued on the server-global channel that
    /// belongs to the SESSION just replaced, keeping only the small set of
    /// lifecycle/control events meant to survive across connections.
    ///
    /// Called immediately before serving a preemption winner. The channel is
    /// shared across every connection the server ever serves and is read by
    /// whichever connection drains it next -- so any event the outgoing
    /// session produced but never got around to consuming (its OWN eviction
    /// notice, a queued clipboard message, an RDPSND wave, an EGFX frame)
    /// would otherwise be delivered to its replacement. That is at best stale
    /// (an audio wave from a session that no longer exists) and at worst a
    /// real leak (the previous peer's clipboard content, handed unprompted to
    /// the client that just replaced it).
    ///
    /// An ALLOWLIST of what to KEEP, not a denylist of `EvictedByOtherConnection`
    /// alone, and deliberately so: this mirrors what `run()`'s own top-level
    /// select already does when NOTHING is being served (only `Quit` /
    /// `GetLocalAddr` / `SetCredentials` / `SetAutoReconnectCookie` are
    /// meaningful there; everything else falls into its `ev => debug!("Unexpected
    /// event")` catch-all and is discarded). A new per-session `ServerEvent`
    /// variant is excluded here by default, instead of silently leaking across
    /// a takeover boundary until someone remembers to add it to a denylist.
    async fn discard_stale_session_events(&mut self) {
        use tokio::sync::mpsc::error::TryRecvError;

        let ev_receiver = Arc::clone(&self.ev_receiver);
        let mut ev_receiver = ev_receiver.lock().await;

        // Collect first, re-send after: re-sending during the drain would push
        // events onto the back of the same queue we are draining.
        let mut keep = Vec::new();
        let mut discarded = 0usize;
        loop {
            match ev_receiver.try_recv() {
                Ok(
                    event @ (ServerEvent::Quit(_)
                    | ServerEvent::GetLocalAddr(_)
                    | ServerEvent::SetCredentials(_)
                    | ServerEvent::SetAutoReconnectCookie(_)),
                ) => keep.push(event),
                Ok(_other) => discarded += 1,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }

        if discarded > 0 {
            debug!(
                discarded,
                "dropped per-session events the replaced session never consumed -- they must not reach its replacement"
            );
        }

        for event in keep {
            let _ = self.ev_sender.send(event);
        }
    }

    /// Build the cheap, cloned snapshot a preempting candidate negotiates
    /// against — see [`NegotiationContext`].
    fn negotiation_context(&self) -> NegotiationContext {
        NegotiationContext {
            opts: self.opts.clone(),
            creds: self.creds.clone(),
            display: Arc::clone(&self.display),
        }
    }

    /// Serve a candidate that already won the preemption race: negotiation and
    /// authentication are done, so this is where its channel backends are
    /// finally built (see the body comment below for why only now) before
    /// handing off to the same finalization the normal path uses. From here
    /// on, a preemption winner is indistinguishable from a normally-accepted
    /// connection.
    async fn serve_negotiated(&mut self, candidate: Box<NegotiatedCandidate>) -> ServerResult<()> {
        self.display_suppressed.store(false, Ordering::Relaxed);

        let mut candidate = candidate;
        // Only NOW build the channel backends: this connection has
        // authenticated and is about to be served, so the factories run
        // exactly once per served session, as they always have. Still ahead of
        // `accept_finalize`, which is where the acceptor first consumes the
        // static channel set (the MCS Connect Initial); `accept_begin`, already
        // done, stops at the security-upgrade gate before that.
        self.attach_channels(&mut candidate.acceptor);

        self.finalize_negotiated(*candidate).await
    }

    /// Run a single RDP connection over `stream`, performing the
    /// IronRDP-managed TLS handshake on `ShouldUpgrade` (standard TCP+TLS).
    ///
    /// Equivalent to [`run_connection_with`](Self::run_connection_with) with
    /// [`TransportTls::Managed`].
    pub async fn run_connection<S>(&mut self, stream: S) -> ServerResult<()>
    where
        S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
    {
        self.run_connection_with(stream, TransportTls::Managed).await
    }

    /// Run a single RDP connection over `stream`, choosing who performs the TLS
    /// handshake with `tls`.
    ///
    /// With [`TransportTls::Managed`], IronRDP performs the TLS accept on
    /// `ShouldUpgrade`, exactly as [`run_connection`](Self::run_connection).
    ///
    /// With [`TransportTls::AlreadyDone`], the caller's `stream` has ALREADY
    /// been transport-encrypted at a lower layer that the embedder owns
    /// (typically a WSS terminator in the same process, or a TLS stream the
    /// embedder accepted up front), so IronRDP skips the TLS handshake and
    /// advances the state machine via [`Acceptor::mark_security_upgrade_as_done`].
    /// Everything past the handshake, including the optional Hybrid CredSSP
    /// exchange and finalization, is identical to the managed path.
    ///
    /// # Use case for [`TransportTls::AlreadyDone`]
    ///
    /// This mode decouples transport encryption from the RDP security-upgrade
    /// step. It is for ironrdp-server endpoints that terminate transport
    /// encryption themselves before the RDP state machine runs — for example a
    /// server that accepts WSS directly, or one fronted by an in-process TLS
    /// terminator — and therefore must not perform a second, inner TLS
    /// handshake when the X.224 negotiation selects `PROTOCOL_SSL`.
    ///
    /// This is distinct from a [RDCleanPath] proxy deployment (e.g.
    /// Devolutions Gateway), where the proxy performs a real TLS handshake with
    /// a *separate* backend RDP server and relays that server's certificate
    /// chain to the client. In that topology the backend server owns its own
    /// TLS and uses [`TransportTls::Managed`]; this mode does not apply to it.
    /// RDCleanPath is relevant here only as one client-side mechanism (see
    /// precondition 2) for telling a client not to expect an inner handshake.
    ///
    /// # Preconditions for [`TransportTls::AlreadyDone`] (caller MUST guarantee)
    ///
    /// 1. The `stream` is already transport-encrypted by another layer
    ///    (WSS, in-process, etc.). Passing a plain TCP stream here exposes
    ///    RDP traffic in plaintext on the wire.
    ///
    /// 2. The connecting client must not expect an inner TLS handshake on this
    ///    stream. Vanilla RDP clients (mstsc, xfreerdp) negotiate TLS from the
    ///    X.224 `selectedProtocol` and have no concept of "TLS already done at a
    ///    lower layer": they will hang or fail, and must use
    ///    [`TransportTls::Managed`]. Arranging for a client to skip the inner
    ///    handshake is the embedder's responsibility; RDCleanPath is one such
    ///    mechanism, but this method does not depend on it.
    ///
    /// 3. If `self.opts.security` is [`RdpServerSecurity::Hybrid`], two things
    ///    must hold. First, the client must support CredSSP over this
    ///    transport; the SPNEGO exchange itself is transport-independent
    ///    (CredSSP carries its own crypto via TSRequest), so it runs the same
    ///    as on the managed path. Second, and less obvious: the CredSSP
    ///    server-public-key confirmation (`pubKeyAuth`, per MS-CSSP) binds to
    ///    the certificate the client validated at the lower transport layer,
    ///    not to anything IronRDP does here. So the public key configured in
    ///    [`RdpServerSecurity::Hybrid`] MUST be the public key of the
    ///    certificate that lower layer (e.g. the WSS terminator) presented to
    ///    the client, otherwise the client's `pubKeyAuth` check fails and
    ///    Hybrid is rejected. This is the embedder's responsibility; it does
    ///    not hold automatically. In practice it means terminating transport
    ///    TLS with the same certificate configured for Hybrid.
    ///
    /// [RDCleanPath]: https://docs.rs/ironrdp-rdcleanpath
    ///
    /// # Wire-level invariant
    ///
    /// This method does NOT alter the X.224 negotiation. The acceptor still
    /// advertises whatever `SecurityProtocol` it was constructed with, and the
    /// connecting client still negotiates as normal. The only behaviour change
    /// under [`TransportTls::AlreadyDone`] is that after the negotiation reaches
    /// the security-upgrade gate, no TLS handshake is performed on the byte
    /// stream, because the caller's stream is already past TLS at a lower layer.
    pub async fn run_connection_with<S>(&mut self, stream: S, tls: TransportTls) -> ServerResult<()>
    where
        S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
    {
        let result = self.run_connection_inner(stream, tls).await;

        // The static channels belong to the connection that negotiated them,
        // and their backends own real resources: an rdpsnd handler is stopped
        // through `Drop`, so an audio backend keeps capturing until the set is
        // replaced. `run` cleared the set itself, which left embedders driving
        // connections through this method with the previous session's backends
        // still live until the next client attached new ones.
        self.static_channels = StaticChannelSet::new();

        result
    }

    async fn run_connection_inner<S>(&mut self, stream: S, tls: TransportTls) -> ServerResult<()>
    where
        S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
    {
        // Per-connection state must start fresh: if the previous client
        // disconnected while it had sent `SuppressOutput { None }` (e.g.,
        // closed the mstsc window while minimized so the matching resume
        // PDU never arrived), the flag would still read `true` here and the
        // display backend would silently drop frames for the entire new
        // session until/unless the new client happens to send a
        // `RefreshRectangle` or `SuppressOutput { Some(rect) }`. Resetting
        // here also covers backends that share an externally-created Arc via
        // `set_display_suppressed_handle()`.
        self.display_suppressed.store(false, Ordering::Relaxed);

        let size = self.display.lock().await.size().await;
        let capabilities = capabilities::capabilities(&self.opts, size);
        let mut pending = PendingConnection::new(
            self.opts.security.clone(),
            size,
            capabilities,
            self.creds.clone(),
            self.opts.honor_client_desktop_size,
        );

        self.attach_channels(pending.acceptor_mut());

        let Some(negotiated) = pending.negotiate_and_authenticate(stream, tls).await? else {
            return Ok(());
        };

        self.finalize_negotiated(negotiated).await
    }

    /// Finalize a connection that has already negotiated (and, under Hybrid,
    /// authenticated) via [`PendingConnection::negotiate_and_authenticate`].
    /// Dispatches on which [`NegotiatedTransport`] variant it got: `Continued`
    /// (no security upgrade happened, [`RdpServerSecurity::None`]) goes
    /// straight to `accept_finalize` with no stream to shut down, while `Tls`
    /// / `Offloaded` route through [`Self::finalize_and_shutdown`] for the
    /// extra shutdown step — these three paths are NOT structurally identical
    /// past this point, only past the handshake `negotiate_and_authenticate`
    /// itself covers.
    async fn finalize_negotiated<S>(&mut self, negotiated: NegotiatedConnection<S>) -> ServerResult<()>
    where
        S: AsyncRead + AsyncWrite + Sync + Send + Unpin,
    {
        let NegotiatedConnection { transport, acceptor } = negotiated;
        match transport {
            // No security upgrade happened, so there is no TLS session to shut
            // down — matches the pre-existing `BeginResult::Continue` arm.
            NegotiatedTransport::Continued(framed) => {
                self.accept_finalize(framed, acceptor).await?;
            }
            NegotiatedTransport::Tls(framed) => {
                self.finalize_and_shutdown(*framed, acceptor, "TLS connection").await?;
            }
            NegotiatedTransport::Offloaded(framed) => {
                self.finalize_and_shutdown(framed, acceptor, "TLS-offloaded stream")
                    .await?;
            }
        }

        Ok(())
    }

    /// Finalize an upgraded stream and shut it down afterwards. The
    /// negotiation and authentication that used to precede this now live in
    /// [`negotiate_and_authenticate`], which the preemption candidate path
    /// shares.
    async fn finalize_and_shutdown<S>(
        &mut self,
        framed: TokioFramed<S>,
        acceptor: Acceptor,
        shutdown_label: &str,
    ) -> ServerResult<()>
    where
        S: AsyncRead + AsyncWrite + Sync + Send + Unpin,
    {
        // No mark_security_upgrade_as_done / CredSSP here: this refactor moved
        // both into `complete_security_upgrade`, called from
        // `PendingConnection::negotiate_and_authenticate` before this function
        // ever runs -- upstream's un-refactored equivalent still does that
        // work at this point, since it has no separate negotiation step.
        let framed = self.accept_finalize(framed, acceptor).await?;
        debug!("Shutting down {}", shutdown_label);
        let (mut inner, _) = framed.into_inner();
        if let Err(e) = inner.shutdown().await {
            debug!(?e, "{} shutdown error", shutdown_label);
        }

        Ok(())
    }

    pub async fn run(&mut self) -> ServerResult<()> {
        // Create socket with control over options before binding.
        // Using TcpSocket instead of TcpListener::bind() allows setting
        // SO_REUSEADDR and IPv6 dual-stack mode.
        let socket = match self.opts.addr {
            SocketAddr::V4(_) => TcpSocket::new_v4().map_err(|e| ServerError::io("create IPv4 socket", e))?,
            SocketAddr::V6(_) => {
                // IPv6 socket: on Linux, dual-stack is the default
                // (net.ipv6.bindv6only=0), so IPv4 clients connect as
                // IPv4-mapped addresses (::ffff:x.x.x.x). On platforms
                // where IPV6_V6ONLY defaults to 1 (Windows, some BSDs),
                // only IPv6 clients will be accepted and a separate IPv4
                // listener would be needed.
                TcpSocket::new_v6().map_err(|e| ServerError::io("create IPv6 socket", e))?
            }
        };

        // SO_REUSEADDR prevents EADDRINUSE when restarting the server while
        // the previous socket is still in TIME_WAIT. Only set on Unix;
        // on Windows SO_REUSEADDR has different semantics that allow a
        // second process to bind the same port, which is a security risk.
        #[cfg(unix)]
        socket
            .set_reuseaddr(true)
            .map_err(|e| ServerError::io("set SO_REUSEADDR", e))?;

        socket
            .bind(self.opts.addr)
            .map_err(|e| ServerError::io("bind listen address", e))?;

        let listener = socket
            .listen(LISTENER_BACKLOG)
            .map_err(|e| ServerError::io("start listener", e))?;
        let local_addr = listener.local_addr().map_err(|e| ServerError::io("local_addr", e))?;

        debug!("Listening for connections on {local_addr}");
        self.local_addr = Some(local_addr);

        // A candidate that wins a preemption race has ALREADY cleared
        // `on_accept` and fully authenticated by the time it lands here, so it
        // carries a negotiated candidate rather than a raw stream: the next
        // iteration resumes it at finalization, with no second `on_accept`
        // call (that hook is stateful for rate limiters) and no renegotiation.
        let mut pending: Option<(Box<NegotiatedCandidate>, SocketAddr)> = None;

        let preempt_enabled = self.opts.preempt_existing_session;
        // Say so out loud: under these modes the bar to evict a live session is
        // NOT authentication, whatever the option's name suggests. Restrict who
        // may even attempt a takeover with `ConnectionHandler::on_accept`.
        if preempt_enabled && !authenticates_before_eviction(&self.opts.security) {
            warn!(
                "preempt_existing_session is enabled under a security mode that does not authenticate the client \
                 before it could evict the live session: any peer able to complete the handshake can take the \
                 session over. Use RdpServerSecurity::Hybrid (CredSSP/NLA) for an authentication-gated takeover, or \
                 gate candidates with ConnectionHandler::on_accept."
            );
        }

        loop {
            enum Entry {
                Fresh(TcpStream, SocketAddr),
                Negotiated(Box<NegotiatedCandidate>, SocketAddr),
            }

            let entry = match pending.take() {
                Some((candidate, peer)) => {
                    // The eviction event is queued on the server-global channel
                    // but consumed by whichever connection happens to drain it.
                    // If the incumbent was too wedged to take it within
                    // `EVICTION_GRACE` (very plausibly the case — being wedged
                    // is why it was evicted), it is still queued now, and the
                    // WINNER's `client_loop` would drain it and disconnect
                    // itself, reporting a bogus
                    // `ERRINFO_DISCONNECTED_BY_OTHERCONNECTION` to the client
                    // that just took the session over. Discard any such
                    // leftover before serving it; every other event is put back
                    // in order.
                    self.discard_stale_session_events().await;
                    Entry::Negotiated(candidate, peer)
                }
                None => {
                    let ev_receiver = Arc::clone(&self.ev_receiver);
                    let mut ev_receiver = ev_receiver.lock().await;
                    let accepted = tokio::select! {
                        Some(event) = ev_receiver.recv() => {
                            match event {
                                ServerEvent::Quit(reason) => {
                                    debug!("Got quit event {reason}");
                                    break;
                                }
                                ServerEvent::GetLocalAddr(tx) => {
                                    let _ = tx.send(self.local_addr);
                                }
                                ServerEvent::SetCredentials(creds) => {
                                    self.set_credentials(Some(creds));
                                }
                                ServerEvent::SetAutoReconnectCookie(cookie) => {
                                    self.set_auto_reconnect_cookie(cookie);
                                }
                                ev => {
                                    debug!("Unexpected event {:?}", ev);
                                }
                            }
                            continue;
                        },
                        Ok((stream, peer)) = listener.accept() => {
                            drop(ev_receiver);
                            (stream, peer)
                        },
                        else => break,
                    };
                    Entry::Fresh(accepted.0, accepted.1)
                }
            };

            let peer = match &entry {
                Entry::Fresh(_, peer) | Entry::Negotiated(_, peer) => *peer,
            };
            debug!(?peer, "Received connection");

            // A `Negotiated` winner already passed `on_accept` as a candidate,
            // inside the race below — its negotiation would not even have
            // started otherwise. Re-running it here would double-count for a
            // stateful handler (a rate limiter's window, an audit record).
            let accepted = matches!(entry, Entry::Negotiated(..))
                || self.connection_handler.as_mut().is_none_or(|h| h.on_accept(peer));

            if !accepted {
                debug!(?peer, "Connection rejected by handler");
                if let Entry::Fresh(stream, _) = entry {
                    drop(stream);
                }
                continue;
            }

            let started = tokio::time::Instant::now();

            let (result, preempted_by) = if preempt_enabled {
                // Serve this connection while still accepting: a newcomer that
                // clears `on_accept` AND fully authenticates
                // (`negotiate_candidate`) takes over, instead of queuing behind
                // the live session. Cancelling `conn` runs the same
                // per-connection teardown a client-side disconnect does.
                //
                // `conn` borrows `self` for the whole race, so the candidate's
                // `on_accept` and its negotiation work from clones taken here.
                let handler = self.connection_handler.take();
                let ctx = self.negotiation_context();
                let ev_sender = self.ev_sender.clone();
                let mut recently_evicted = self.recently_evicted.take();

                let outcome = {
                    // Uses the anyhow-returning inner method, not the public
                    // `run_connection` (`ServerResult`-returning as of
                    // upstream's typed-error migration, #1242): `conn`'s
                    // declared `Result<()>` (anyhow) must match
                    // `serve_negotiated`'s return type across both match
                    // arms, and `on_disconnected` below still expects
                    // `Option<&anyhow::Error>` -- the same reason upstream's
                    // own accept loop bypasses the public wrapper too.
                    let mut conn: core::pin::Pin<Box<dyn Future<Output = ServerResult<()>> + '_>> = match entry {
                        Entry::Fresh(stream, _) => Box::pin(self.run_connection_inner(stream, TransportTls::Managed)),
                        Entry::Negotiated(candidate, _) => Box::pin(self.serve_negotiated(candidate)),
                    };
                    let mut probe: PreemptProbe<'_> = Box::pin(core::future::pending());
                    let mut handler = handler;
                    let mut probing = false;

                    loop {
                        // This `select!` must only YIELD — never mutate
                        // `probe`, whose futures it still borrows.
                        let race = tokio::select! {
                            res = &mut conn => PreemptRace::Ended(res),
                            accepted = listener.accept(), if !probing => PreemptRace::Accepted(accepted),
                            candidate = &mut probe => PreemptRace::Probed(candidate),
                        };

                        match race {
                            // The session ended on its own. A candidate still
                            // negotiating is NOT discarded — that would reset a
                            // legitimate client that happened to connect just
                            // as the old session ended; finish it and serve it
                            // next if it authenticates.
                            PreemptRace::Ended(res) => {
                                if probing {
                                    // BOUNDED: nothing else is being serviced
                                    // during this await, so a candidate that
                                    // is not nearly done is dropped rather
                                    // than allowed to stall the listener.
                                    pending = match tokio::time::timeout(CANDIDATE_HANDOFF_GRACE, &mut probe).await {
                                        Ok(candidate) => candidate,
                                        Err(_) => {
                                            debug!(
                                                "a candidate was still negotiating when the session ended -- \
                                                 dropping it rather than stalling the accept loop; it can reconnect"
                                            );
                                            None
                                        }
                                    };
                                }
                                break (res, None, handler, recently_evicted);
                            }
                            PreemptRace::Accepted(Ok((next_stream, next_peer))) => {
                                // A peer evicted moments ago may not bounce
                                // straight back and retake the session; each
                                // attempt re-arms the window, so a reconnect
                                // storm can never win. See `recently_evicted`.
                                let bounced_back = refuse_reconnect_from_evicted(
                                    &mut recently_evicted,
                                    next_peer.ip(),
                                    Instant::now(),
                                    REPREEMPT_COOLDOWN,
                                    REPREEMPT_MAX_LOCKOUT,
                                );
                                // Gate the candidate through `on_accept` BEFORE
                                // it may negotiate, and so before it can
                                // preempt anything: otherwise a candidate the
                                // rate limiter would reject could still evict
                                // the live session and only be rejected
                                // afterwards, once the damage was done.
                                let candidate_accepted =
                                    !bounced_back && handler.as_mut().is_none_or(|h| h.on_accept(next_peer));

                                if candidate_accepted {
                                    probing = true;
                                    // BOUNDED: see `CANDIDATE_NEGOTIATION_TIMEOUT`.
                                    // An unbounded probe is a remote hang of
                                    // the whole accept loop.
                                    probe = Box::pin(negotiate_candidate_bounded(&ctx, next_stream, next_peer));
                                } else if bounced_back {
                                    info!(
                                        ?next_peer,
                                        "ignoring a reconnect from the peer just evicted -- it is \
                                         auto-reconnecting into the session that replaced it"
                                    );
                                    drop(next_stream);
                                } else {
                                    debug!(?next_peer, "candidate rejected by handler while a session was live");
                                    drop(next_stream);
                                }
                            }
                            PreemptRace::Accepted(Err(error)) => {
                                warn!(?error, "accept failed while a session was live");
                            }
                            PreemptRace::Probed(candidate) => {
                                probing = false;
                                probe = Box::pin(core::future::pending());
                                // `negotiate_candidate` already logged the
                                // reason when it declines, so there is nothing
                                // to do in the `None` case.
                                if let Some((candidate, new_peer)) = candidate {
                                    info!(
                                        old_peer = ?peer,
                                        ?new_peer,
                                        "an authenticated client connected -- evicting the existing session"
                                    );
                                    let _ = ev_sender.send(ServerEvent::EvictedByOtherConnection);
                                    let now = Instant::now();
                                    recently_evicted = Some(EvictedPeer {
                                        ip: peer.ip(),
                                        evicted_at: now,
                                        last_try: now,
                                    });
                                    // Let the incumbent observe the event and
                                    // put the reason on the wire before it
                                    // goes; bounded, so a wedged peer cannot
                                    // stall the takeover.
                                    match tokio::time::timeout(EVICTION_GRACE, &mut conn).await {
                                        Ok(res) => {
                                            break (res, Some((candidate, new_peer)), handler, recently_evicted);
                                        }
                                        Err(_) => {
                                            debug!(old_peer = ?peer, "evicted session did not wind down in time");
                                            break (Ok(()), Some((candidate, new_peer)), handler, recently_evicted);
                                        }
                                    }
                                }
                            }
                        }
                    }
                };

                let (result, preempted_by, handler, evicted) = outcome;
                self.connection_handler = handler;
                // Only remember an eviction that actually replaced this
                // session; a session that ended on its own terms leaves nobody
                // barred from connecting.
                self.recently_evicted = if preempted_by.is_some() { evicted } else { None };
                if preempted_by.is_some() {
                    // Can't do this INSIDE the race above: `conn` (built from
                    // `self.run_connection`/`self.serve_negotiated`) borrows
                    // `self` mutably for the whole race, so no other &mut self
                    // call is possible there. `self` is free again here, and
                    // the ~750ms EVICTION_GRACE this waited through is
                    // immaterial to what this closes -- a real ARC reconnect
                    // takes far longer than that to occur.
                    self.invalidate_auto_reconnect_cookie_on_eviction();
                }
                (result, preempted_by)
            } else {
                let result = match entry {
                    // Same anyhow-vs-ServerResult reasoning as the preemption
                    // branch above.
                    Entry::Fresh(stream, _) => self.run_connection_inner(stream, TransportTls::Managed).await,
                    // Unreachable in practice: `pending` is only ever populated
                    // by the preemption branch above.
                    Entry::Negotiated(candidate, _) => self.serve_negotiated(candidate).await,
                };
                (result, None)
            };
            let duration = started.elapsed();

            if let Some((candidate, new_peer)) = preempted_by {
                pending = Some((candidate, new_peer));
            }

            if let Err(ref error) = result {
                error!(?error, "Connection error");
            }

            // NOT redundant with `run_connection_with`'s own reset (added
            // upstream, #1721) despite resetting the same field: a preemption
            // winner reaches this point via `serve_negotiated`, which never
            // calls `run_connection`/`run_connection_with` at all -- so this
            // is the only reset that path gets. Removing this because
            // `run_connection_with` "already handles it" would silently
            // reintroduce #1721's leak (channel backends, e.g. rdpsnd's audio
            // capture, held open until the next client) for every preemption
            // takeover.
            self.static_channels = StaticChannelSet::new();

            if let Some(ref mut handler) = self.connection_handler {
                let action = handler.on_disconnected(peer, duration, result.as_ref().err());
                if action == PostConnectionAction::Stop {
                    debug!(?peer, "Handler requested stop after disconnect");
                    break;
                }
            }
        }

        Ok(())
    }

    pub fn get_svc_processor<T: SvcProcessor + 'static>(&mut self) -> Option<&mut T> {
        self.static_channels
            .get_by_type_mut::<T>()
            .and_then(|svc| svc.channel_processor_downcast_mut())
    }

    pub fn get_channel_id_by_type<T: SvcProcessor + 'static>(&self) -> Option<StaticChannelId> {
        self.static_channels.get_channel_id_by_type::<T>()
    }

    async fn dispatch_pdu(
        &mut self,
        action: Action,
        bytes: bytes::BytesMut,
        writer: &mut impl FramedWrite,
        io_channel_id: u16,
        user_channel_id: u16,
        message_channel_id: Option<u16>,
    ) -> ServerResult<RunState> {
        match action {
            Action::FastPath => {
                let input = decode(&bytes).map_err(ServerError::decode)?;
                self.handle_fastpath(input).await;
            }

            Action::X224 => {
                if self
                    .handle_x224(writer, io_channel_id, user_channel_id, message_channel_id, &bytes)
                    .await?
                {
                    debug!("Got disconnect request");
                    return Ok(RunState::Disconnect);
                }
            }
        }

        Ok(RunState::Continue)
    }

    async fn dispatch_display_update(
        update: DisplayUpdate,
        writer: &mut impl FramedWrite,
        user_channel_id: u16,
        io_channel_id: u16,
        buffer: &mut Vec<u8>,
        mut encoder: UpdateEncoder,
    ) -> ServerResult<(RunState, UpdateEncoder)> {
        if let DisplayUpdate::Resize(desktop_size) = update {
            debug!(?desktop_size, "Display resize");
            encoder.set_desktop_size(desktop_size);
            deactivate_all(io_channel_id, user_channel_id, writer).await?;
            return Ok((RunState::DeactivationReactivation { desktop_size }, encoder));
        }

        let mut encoder_iter = encoder.update(update);
        loop {
            let Some(fragmenter) = encoder_iter.next().await else {
                break;
            };

            let mut fragmenter = fragmenter?;
            if fragmenter.size_hint() > buffer.len() {
                buffer.resize(fragmenter.size_hint(), 0);
            }

            while let Some(len) = fragmenter.next(buffer) {
                writer
                    .write_all(&buffer[..len])
                    .await
                    .map_err(|e| ServerError::custom("failed to write display update", e))?;
            }
        }

        Ok((RunState::Continue, encoder))
    }

    async fn dispatch_server_events(
        &mut self,
        events: &mut Vec<ServerEvent>,
        writer: &mut impl FramedWrite,
        io_channel_id: u16,
        user_channel_id: u16,
        message_channel_id: Option<u16>,
    ) -> ServerResult<RunState> {
        // Avoid wave messages queuing up and causing extra delay. When a
        // batch carries more than `WAVE_KEEP` waves, drop the OLDEST ones
        // and keep the most recent — playing stale audio just bakes the
        // latency in permanently, so a one-time dispatch stall (e.g. a video
        // encode holding the server lock) would otherwise become a permanent
        // audio offset.
        //
        // This is still a naive solution; better long-term: compute the
        // actual delay, add IO priority, encode audio, use UDP, etc. 4 frames
        // is roughly low hundreds of ms in regular setups.
        const WAVE_KEEP: usize = 4;
        let wave_total = events
            .iter()
            .filter(|e| matches!(e, ServerEvent::Rdpsnd(RdpsndServerMessage::Wave(..))))
            .count();
        let mut wave_skip = wave_total.saturating_sub(WAVE_KEEP);
        for event in events.drain(..) {
            trace!(?event, "Dispatching");
            match event {
                ServerEvent::Quit(reason) => {
                    debug!("Got quit event: {reason}");
                    return Ok(RunState::Disconnect);
                }
                // Session takeover: tell the client WHY it is being
                // disconnected before dropping it, so it does not read this as
                // an unexpected drop and auto-reconnect (which ping-pongs
                // against the preempting client — see the variant's docs).
                ServerEvent::EvictedByOtherConnection => {
                    debug!("evicting this connection -- another client took the session over");
                    // KNOWN GAP: MS-RDPBCGR 3.3.5.7.1 says the Set Error Info
                    // PDU MUST NOT be sent to a client that did not set
                    // RNS_UD_CS_SUPPORT_ERRINFO_PDU in its Client Core Data
                    // `earlyCapabilityFlags`, and this sends it unconditionally.
                    // `AcceptorResult` exposes no early-capability field today,
                    // so the check is not currently expressible here; the
                    // pre-existing `send_access_denied` has the identical gap.
                    // Closing it needs an ironrdp-acceptor API addition.
                    let pdu = rdp::headers::ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(
                        ErrorInfo::ProtocolIndependentCode(ProtocolIndependentCode::DisconnectedByOtherconnection),
                    ));
                    // Best-effort: if the evicted peer's socket is already
                    // half-dead the write fails, which is fine — it is leaving
                    // either way, and the caller falls back to cancelling it.
                    // pduSource=0, not user_channel_id -- MS-RDPBCGR 2.2.5.1.1
                    // requires it for TS_SET_ERROR_INFO_PDU specifically.
                    match encode_share_data_pdu(pdu, 0, io_channel_id, user_channel_id) {
                        Ok(bytes) => {
                            if let Err(error) = writer.write_all(&bytes).await {
                                debug!(%error, "could not send the eviction reason; disconnecting anyway");
                            }
                        }
                        Err(error) => {
                            warn!(%error, "could not encode the eviction reason; disconnecting anyway");
                        }
                    }
                    return Ok(RunState::Disconnect);
                }
                ServerEvent::GetLocalAddr(tx) => {
                    let _ = tx.send(self.local_addr);
                }
                ServerEvent::SetCredentials(creds) => {
                    self.set_credentials(Some(creds));
                }
                ServerEvent::SetAutoReconnectCookie(cookie) => {
                    self.update_auto_reconnect_cookie(cookie, writer, io_channel_id, user_channel_id)
                        .await?;
                }
                ServerEvent::Rdpsnd(s) => {
                    let Some(rdpsnd) = self.get_svc_processor::<RdpsndServer>() else {
                        warn!("No rdpsnd channel, dropping event");
                        continue;
                    };
                    let msgs = match s {
                        RdpsndServerMessage::Wave(data, ts) => {
                            if wave_skip > 0 {
                                wave_skip -= 1;
                                debug!("Dropping stale wave");
                                continue;
                            }
                            rdpsnd.wave(data, ts)
                        }
                        RdpsndServerMessage::SetVolume { left, right } => rdpsnd.set_volume(left, right),
                        RdpsndServerMessage::Close => rdpsnd.close(),
                        RdpsndServerMessage::Error(error) => {
                            error!(?error, "Handling rdpsnd event");
                            continue;
                        }
                    }
                    .map_err_kind("failed to send rdpsnd event", ServerErrorKind::Pdu)?;
                    let channel_id = self
                        .get_channel_id_by_type::<RdpsndServer>()
                        .ok_or_else(|| ServerError::channel("SVC channel not found"))?;
                    let data = server_encode_svc_messages(msgs.into(), channel_id, user_channel_id)
                        .map_err(ServerError::encode)?;
                    writer
                        .write_all(&data)
                        .await
                        .map_err(|e| ServerError::io("write_all", e))?;
                }
                ServerEvent::Clipboard(c) => {
                    let Some(cliprdr) = self.get_svc_processor::<CliprdrServer>() else {
                        warn!("No clipboard channel, dropping event");
                        continue;
                    };
                    let msgs = match c {
                        ClipboardMessage::SendInitiateCopy(formats) => cliprdr.initiate_copy(&formats),
                        ClipboardMessage::SendInitiateFileCopy(files) => cliprdr.initiate_file_copy(files),
                        ClipboardMessage::SendFormatData(data) => cliprdr.submit_format_data(data),
                        ClipboardMessage::SendInitiatePaste(format) => cliprdr.initiate_paste(format),
                        ClipboardMessage::SendFileContentsRequest(request) => cliprdr.request_file_contents(request),
                        ClipboardMessage::SendFileContentsResponse(response) => cliprdr.submit_file_contents(response),
                        ClipboardMessage::Error(error) => {
                            error!(?error, "Handling clipboard event");
                            continue;
                        }
                    }
                    .map_err_kind("failed to send clipboard event", ServerErrorKind::Pdu)?;
                    let channel_id = self
                        .get_channel_id_by_type::<CliprdrServer>()
                        .ok_or_else(|| ServerError::channel("SVC channel not found"))?;
                    let data = server_encode_svc_messages(msgs.into(), channel_id, user_channel_id)
                        .map_err(ServerError::encode)?;
                    writer
                        .write_all(&data)
                        .await
                        .map_err(|e| ServerError::io("write_all", e))?;
                }
                ServerEvent::Echo(msg) => match msg {
                    EchoServerMessage::SendRequest { payload } => {
                        let Some(drdynvc) = self.get_svc_processor::<dvc::DrdynvcServer>() else {
                            warn!("No drdynvc channel, dropping ECHO request");
                            continue;
                        };

                        let Some(echo_channel_id) = drdynvc.get_channel_id_by_type::<EchoDvcBridge>() else {
                            warn!("No ECHO dynamic channel, dropping ECHO request");
                            continue;
                        };

                        if !drdynvc.is_channel_opened(echo_channel_id) {
                            warn!("ECHO dynamic channel not yet opened, dropping ECHO request");
                            continue;
                        }

                        self.echo_handle.on_request_sent(&payload);

                        let request = build_echo_request(payload)?;
                        let messages =
                            dvc::encode_dvc_messages(echo_channel_id, vec![request], ChannelFlags::SHOW_PROTOCOL)
                                .map_err(ServerError::encode)?;

                        let drdynvc_channel_id = self
                            .get_channel_id_by_type::<dvc::DrdynvcServer>()
                            .ok_or_else(|| ServerError::channel("DRDYNVC channel not found"))?;

                        let data = server_encode_svc_messages(messages, drdynvc_channel_id, user_channel_id)
                            .map_err(ServerError::encode)?;
                        writer
                            .write_all(&data)
                            .await
                            .map_err(|e| ServerError::io("write_all", e))?;
                    }
                },
                #[cfg(feature = "usb")]
                ServerEvent::Usb(msg) => match msg {
                    UrbdrcServerMessage::AddChan => {
                        let create_dvc_msg = {
                            use crate::urbdrc::UsbRedirServer;

                            let Some(usb_man) = self.usb_man.as_mut() else {
                                warn!("Missing USB device factory");
                                continue;
                            };
                            let Some(drdynvc) = self
                                .static_channels
                                .get_by_type_mut::<dvc::DrdynvcServer>()
                                .and_then(|svc| svc.channel_processor_downcast_mut::<dvc::DrdynvcServer>())
                            else {
                                warn!("No drdynvc channel, dropping URBDRC request");
                                continue;
                            };

                            let Some(comp_iface) = usb_man.comp_iface_alloc.alloc() else {
                                warn!("Run out of URBDRC interface IDs");
                                continue;
                            };

                            let Some(device_backend) = usb_man.factory.create_device() else {
                                warn!("Failed to create USB device backend");
                                continue;
                            };

                            drdynvc
                                .create_channel_with(|dvc_id| {
                                    let lifecycle = Arc::new(UsbDeviceLifecycle::new());
                                    let handle =
                                        UsbDeviceHandle::new(self.ev_sender.clone(), dvc_id, Arc::clone(&lifecycle));
                                    let device = ServerUsbDevice {
                                        lifecycle,
                                        handle: handle.clone(),
                                        pending: HashMap::new(),
                                    };
                                    if usb_man.router.insert(dvc_id, device).is_some() {
                                        warn!(dvc_id = dvc_id, "Replacing USB device pending-request map");
                                    }
                                    Ok::<_, PduError>(
                                        UrbdrcDeviceServer::new(
                                            Box::new(UsbRedirServer::new(device_backend, handle)),
                                            comp_iface,
                                        )
                                        .expect("interface ID allocated by InterfaceAlloc must be valid"),
                                    )
                                })
                                .map_err_kind("create URBDRC device channel", ServerErrorKind::Pdu)?
                        };

                        let drdynvc_channel_id = self
                            .get_channel_id_by_type::<dvc::DrdynvcServer>()
                            .ok_or_else(|| ServerError::channel("DRDYNVC channel not found"))?;
                        let data =
                            server_encode_svc_messages(vec![create_dvc_msg], drdynvc_channel_id, user_channel_id)
                                .map_err(ServerError::encode)?;

                        writer
                            .write_all(&data)
                            .await
                            .map_err(|e| ServerError::io("write_all", e))?;
                    }
                    UrbdrcServerMessage::Device { dvc_id, dev_msg } => {
                        let Some(lifecycle) = self
                            .usb_man
                            .as_ref()
                            .and_then(|usb_man| usb_man.router.get(&dvc_id))
                            .map(|device| Arc::clone(&device.lifecycle))
                        else {
                            warn!(dvc_id, "Missing USB device state");
                            continue;
                        };

                        // Handle checks are an early rejection for callers. This event-loop check
                        // is authoritative because a request may already be queued when retract or
                        // channel close changes the shared lifecycle state.
                        if !lifecycle.is_open() {
                            trace!(dvc_id, "Dropping request for closing or closed USB device");
                            continue;
                        }

                        let (dvc_msgs, io_reply, close_dev) = {
                            let Some(drdynvc) = self.get_svc_processor::<dvc::DrdynvcServer>() else {
                                warn!("No drdynvc channel, dropping URBDRC request");
                                continue;
                            };

                            let Some(mut dvc) = drdynvc.dvc_by_id_mut::<UrbdrcDeviceServer>(dvc_id) else {
                                warn!(dvc_id, "USB dynamic channel ID mismatch");
                                continue;
                            };
                            let processor = dvc.processor_mut();

                            match dev_msg {
                                UrbdrcDeviceServerMessage::QueryDeviceText { text_type, locale_id } => {
                                    let text = processor
                                        .query_device_text(text_type, locale_id)
                                        .map_err_kind("query USB device text", ServerErrorKind::Pdu)?;
                                    (vec![text], None, false)
                                }
                                UrbdrcDeviceServerMessage::IoComp { request_id, completion } => {
                                    let Some(usb_man) = self.usb_man.as_mut() else {
                                        warn!("Missing USB device factory");
                                        continue;
                                    };
                                    let Some(device) = usb_man.router.get_mut(&dvc_id) else {
                                        warn!(dvc_id, "Missing USB device state");
                                        continue;
                                    };
                                    let Some(sender) = device.pending.remove(&request_id) else {
                                        warn!(dvc_id, request_id, "Missing pending USB I/O request");
                                        continue;
                                    };

                                    if sender.send(completion).is_err() {
                                        trace!(dvc_id, request_id, "USB I/O completion receiver dropped");
                                    }
                                    (Vec::new(), None, false)
                                }
                                UrbdrcDeviceServerMessage::IoReq { data, tx } => {
                                    if tx.is_closed() {
                                        continue;
                                    }

                                    let request = match data {
                                        ServerDeviceIoReq::IoControl(packet) => processor.io_control(packet),
                                        ServerDeviceIoReq::InternalIoControl(packet) => {
                                            processor.internal_io_control(packet)
                                        }
                                        ServerDeviceIoReq::TransferOut(packet) => processor.transfer_out(packet),
                                        ServerDeviceIoReq::TransferIn(packet) => processor.transfer_in(packet),
                                    }
                                    .map_err_kind("USB I/O request", ServerErrorKind::Pdu)?;

                                    let pending = if request.expects_completion {
                                        let Some(usb_man) = self.usb_man.as_mut() else {
                                            warn!("Missing USB device factory");
                                            continue;
                                        };
                                        let Some(device) = usb_man.router.get_mut(&dvc_id) else {
                                            error!(dvc_id, "Missing USB device state");
                                            continue;
                                        };

                                        let (comp_tx, comp_rx) = oneshot::channel();
                                        if device.pending.insert(request.request_id, comp_tx).is_some() {
                                            warn!(
                                                dvc_id,
                                                request_id = request.request_id,
                                                "Replacing pending USB I/O request"
                                            );
                                        }

                                        Some(RawPending {
                                            rx: comp_rx,
                                            id: request.request_id,
                                            handle: device.handle.clone(),
                                        })
                                    } else {
                                        None
                                    };

                                    (vec![request.message], Some((tx, pending)), false)
                                }
                                UrbdrcDeviceServerMessage::Retract(reason) => {
                                    let request = processor
                                        .retract_device(reason)
                                        .map_err_kind("retract USB device", ServerErrorKind::Pdu)?;
                                    lifecycle.mark_retracting();
                                    (vec![request], None, true)
                                }
                                UrbdrcDeviceServerMessage::CancelRequest(request_id) => {
                                    let request = processor
                                        .cancel_request(request_id)
                                        .map_err_kind("cancel USB I/O request", ServerErrorKind::Pdu)?;
                                    let Some(usb_man) = self.usb_man.as_mut() else {
                                        warn!("Missing USB device factory");
                                        continue;
                                    };
                                    let Some(device) = usb_man.router.get_mut(&dvc_id) else {
                                        warn!(dvc_id, "Missing USB device state");
                                        continue;
                                    };

                                    // A completion may have won the race with PendingRequest::drop.
                                    // Only emit CANCEL_REQUEST while the request is still pending.
                                    if !device.pending.contains_key(&request_id) {
                                        trace!(dvc_id, request_id, "USB I/O request is no longer pending");
                                        continue;
                                    }

                                    (vec![request], None, false)
                                }
                            }
                        };

                        let mut messages = dvc::encode_dvc_messages(dvc_id, dvc_msgs, ChannelFlags::SHOW_PROTOCOL)
                            .map_err(ServerError::encode)?;

                        if close_dev {
                            let close_message = self
                                .get_svc_processor::<dvc::DrdynvcServer>()
                                .and_then(|drdynvc| drdynvc.close_channel(dvc_id))
                                .ok_or_else(|| {
                                    ServerError::channel("URBDRC dynamic channel disappeared before close")
                                })?;
                            self.remove_usb_device(dvc_id);
                            messages.push(close_message);
                        }

                        let drdynvc_channel_id = self
                            .get_channel_id_by_type::<dvc::DrdynvcServer>()
                            .ok_or_else(|| ServerError::channel("DRDYNVC channel not found"))?;

                        let data = server_encode_svc_messages(messages, drdynvc_channel_id, user_channel_id)
                            .map_err(ServerError::encode)?;
                        writer
                            .write_all(&data)
                            .await
                            .map_err(|e| ServerError::io("write_all", e))?;

                        if let Some((tx, pending)) = io_reply {
                            if let Err(pending) = tx.send(pending) {
                                trace!(dvc_id, "USB I/O request receiver dropped");
                                drop(pending);
                            }
                        }
                    }
                    UrbdrcServerMessage::DeviceClosed { dvc_id } => {
                        self.remove_usb_device(dvc_id);
                    }
                },
                #[cfg(feature = "egfx")]
                ServerEvent::Egfx(msg) => match msg {
                    EgfxServerMessage::SendMessages { messages } => {
                        let drdynvc_channel_id = self
                            .get_channel_id_by_type::<dvc::DrdynvcServer>()
                            .ok_or_else(|| ServerError::channel("DRDYNVC channel not found"))?;
                        let data = server_encode_svc_messages(messages, drdynvc_channel_id, user_channel_id)
                            .map_err(ServerError::encode)?;
                        writer
                            .write_all(&data)
                            .await
                            .map_err(|e| ServerError::io("write_all", e))?;
                    }
                },
                ServerEvent::AutoDetectRttRequest => {
                    // Auto-detect requests ride the MCS message channel
                    // ([MS-RDPBCGR] 2.2.14.3). With none negotiated (the client
                    // did not request it), there is nowhere to send them.
                    if let (Some(ad), Some(message_channel_id)) = (self.autodetect.as_mut(), message_channel_id) {
                        let now_ms = monotonic_now_ms();
                        ad.expire_stale_probes(now_ms, crate::autodetect::RTT_PROBE_MAX_AGE_MS);
                        let request = ad.send_rtt_request(now_ms);
                        let data = encode_autodetect_request(request, message_channel_id, user_channel_id)?;
                        writer
                            .write_all(&data)
                            .await
                            .map_err(|e| ServerError::io("write_all", e))?;

                        // Report the measured characteristics to the client
                        // ([MS-RDPBCGR] 2.2.14.1.5). The client does not reply. Sent only
                        // once both RTT and bandwidth are known, and paced independently
                        // of this probe cadence, so a fast caller does not turn into a
                        // fast stream of unsolicited PDUs.
                        if let Some(result) = ad.build_netchar_result(now_ms) {
                            let data = encode_autodetect_request(result, message_channel_id, user_channel_id)?;
                            writer
                                .write_all(&data)
                                .await
                                .map_err(|e| ServerError::io("write_all", e))?;
                        }

                        // Periodically measure bandwidth: Start on one tick, Stop several
                        // ticks later, with ordinary traffic in between counted by the
                        // client, then a Bandwidth Measure Results PDU in reply. Until one
                        // has completed there is no characteristics result to send at all.
                        if let Some(pdu) = ad.build_bandwidth_measure() {
                            let data = encode_autodetect_request(pdu, message_channel_id, user_channel_id)?;
                            writer
                                .write_all(&data)
                                .await
                                .map_err(|e| ServerError::io("write_all", e))?;
                        }
                    }
                }
            }
        }

        Ok(RunState::Continue)
    }

    async fn client_loop<R, W>(
        &mut self,
        reader: &mut Framed<R>,
        writer: &mut Framed<W>,
        io_channel_id: u16,
        user_channel_id: u16,
        message_channel_id: Option<u16>,
        mut encoder: UpdateEncoder,
    ) -> ServerResult<RunState>
    where
        R: FramedRead,
        W: FramedWrite,
    {
        debug!("Starting client loop");
        let mut display_updates = self
            .display
            .lock()
            .await
            .updates()
            .await
            .map_err(|e| from_anyhow_with_context(e, "getting display updates"))?;
        let mut writer = SharedWriter::new(writer);
        let mut display_writer = writer.clone();
        let mut event_writer = writer.clone();
        let mut auto_reconnect_writer = writer.clone();
        let ev_receiver = Arc::clone(&self.ev_receiver);
        let s = Rc::new(Mutex::new(self));

        let this = Rc::clone(&s);
        let dispatch_pdu = async move {
            loop {
                let (action, bytes) = reader.read_pdu().await.map_err(|e| ServerError::io("read pdu", e))?;
                let mut this = this.lock().await;
                match this
                    .dispatch_pdu(
                        action,
                        bytes,
                        &mut writer,
                        io_channel_id,
                        user_channel_id,
                        message_channel_id,
                    )
                    .await?
                {
                    RunState::Continue => continue,
                    state => break Ok(state),
                }
            }
        };

        let dispatch_display = async move {
            let mut buffer = vec![0u8; 4096];

            loop {
                match display_updates.next_update().await {
                    Ok(Some(update)) => {
                        match Self::dispatch_display_update(
                            update,
                            &mut display_writer,
                            user_channel_id,
                            io_channel_id,
                            &mut buffer,
                            encoder,
                        )
                        .await?
                        {
                            (RunState::Continue, enc) => {
                                encoder = enc;
                                continue;
                            }
                            (state, _) => {
                                break Ok(state);
                            }
                        }
                    }
                    Ok(None) => {
                        break Ok(RunState::Disconnect);
                    }
                    Err(error) => {
                        warn!(error = format!("{error:#}"), "next_updated failed");
                    }
                }
            }
        };

        let this = Rc::clone(&s);
        let mut ev_receiver = ev_receiver.lock().await;
        let dispatch_events = async move {
            let mut events = Vec::with_capacity(100);
            loop {
                let nevents = ev_receiver.recv_many(&mut events, 100).await;
                if nevents == 0 {
                    debug!("No sever events.. stopping");
                    break Ok(RunState::Disconnect);
                }
                while let Ok(ev) = ev_receiver.try_recv() {
                    events.push(ev);
                }
                let mut this = this.lock().await;
                match this
                    .dispatch_server_events(
                        &mut events,
                        &mut event_writer,
                        io_channel_id,
                        user_channel_id,
                        message_channel_id,
                    )
                    .await?
                {
                    RunState::Continue => continue,
                    state => break Ok(state),
                }
            }
        };

        let this = Rc::clone(&s);
        let refresh_auto_reconnect_cookie = async move {
            let mut interval = tokio::time::interval(AUTO_RECONNECT_COOKIE_UPDATE_INTERVAL);
            interval.tick().await;

            loop {
                interval.tick().await;
                let mut this = this.lock().await;
                this.rotate_auto_reconnect_cookie(&mut auto_reconnect_writer, io_channel_id, user_channel_id)
                    .await?;
            }
        };

        let state = tokio::select!(
            state = dispatch_pdu => state,
            state = dispatch_display => state,
            state = dispatch_events => state,
            state = refresh_auto_reconnect_cookie => state,
        );

        debug!("End of client loop: {state:?}");
        state
    }

    async fn client_accepted<R, W>(
        &mut self,
        reader: &mut Framed<R>,
        writer: &mut Framed<W>,
        result: AcceptorResult,
    ) -> ServerResult<RunState>
    where
        R: FramedRead,
        W: FramedWrite,
    {
        debug!("Client accepted");

        let is_auto_reconnect = if let Some(reconnect) = result.auto_reconnect.as_ref() {
            if !self.verify_auto_reconnect_cookie(reconnect) {
                warn!("Auto-reconnect cookie validation rejected");
                send_access_denied(result.io_channel_id, result.user_channel_id, writer).await?;
                return Err(ServerError::reason("auto-reconnect validation", "cookie rejected"));
            }

            debug!("Auto-reconnect cookie validation accepted");
            true
        } else {
            false
        };

        // Validate credentials if a validator is configured. The validator runs here, in the
        // async server layer, rather than in the sans-I/O acceptor, because real validators
        // (PAM/LDAP/DB) are I/O-bound. On rejection, deny with a ServerSetErrorInfoPdu before
        // closing, matching the acceptor's exact-match denial path.
        if !is_auto_reconnect && let Some(validator) = self.credential_validator.clone() {
            if let Some(creds) = &result.credentials {
                match validator.validate(creds).await {
                    Ok(CredentialDecision::Accept) => {
                        debug!("Credential validation accepted");
                    }
                    Ok(CredentialDecision::Reject) => {
                        warn!("Credential validation rejected");
                        send_access_denied(result.io_channel_id, result.user_channel_id, writer).await?;
                        return Err(ServerError::reason("credential validation", "rejected by validator"));
                    }
                    Err(e) => {
                        error!(error = %e, "Credential validator backend error");
                        send_access_denied(result.io_channel_id, result.user_channel_id, writer).await?;
                        return Err(ServerError::custom("credential validation", e));
                    }
                }
            } else {
                debug!("Skipping credential validation (no credentials in AcceptorResult)");
            }
        }

        if !result.reactivation
            && let Some(ref mut handler) = self.connection_handler
        {
            handler.on_connection_info(&ConnectionInfo {
                keyboard_layout: result.keyboard_layout,
                keyboard_type: result.keyboard_type,
                ime_file_name: result.ime_file_name.clone(),
            });
        }

        if !result.input_events.is_empty() {
            debug!("Handling input event backlog from acceptor sequence");
            self.handle_input_backlog(
                writer,
                result.io_channel_id,
                result.user_channel_id,
                result.message_channel_id,
                result.input_events,
            )
            .await?;
        }

        self.static_channels = result.static_channels;
        if !result.reactivation {
            for (_channel_key, channel, channel_id) in self.static_channels.iter_by_key_mut() {
                debug!(?channel, ?channel_id, "Start");
                let Some(channel_id) = channel_id else {
                    continue;
                };
                let svc_responses = channel.start().map_err_kind("svc start", ServerErrorKind::Pdu)?;
                let response = server_encode_svc_messages(svc_responses, channel_id, result.user_channel_id)
                    .map_err(ServerError::encode)?;
                writer
                    .write_all(&response)
                    .await
                    .map_err(|e| ServerError::io("write svc response", e))?;
            }
        }

        let mut update_codecs = UpdateEncoderCodecs::new();
        let mut surface_flags = CmdFlags::empty();
        for c in result.capabilities {
            match c {
                CapabilitySet::General(c) => {
                    let fastpath = c.extra_flags.contains(GeneralExtraFlags::FASTPATH_OUTPUT_SUPPORTED);
                    if !fastpath {
                        return Err(ServerError::unsupported("Fastpath output"));
                    }
                }
                CapabilitySet::Bitmap(b) => {
                    if !b.desktop_resize_flag {
                        debug!("Desktop resize is not supported by the client");
                        continue;
                    }

                    let client_size = DesktopSize {
                        width: b.desktop_width,
                        height: b.desktop_height,
                    };
                    let display_size = self.display.lock().await.request_initial_size(client_size).await;

                    // It's problematic when the client didn't resize, as we send bitmap updates that don't fit.
                    // The client will likely drop the connection.
                    if client_size.width < display_size.width || client_size.height < display_size.height {
                        // TODO: we may have different behaviour instead, such as clipping or scaling?
                        warn!(
                            "Client size doesn't fit the server size: {:?} < {:?}",
                            client_size, display_size
                        );
                    }
                }
                CapabilitySet::SurfaceCommands(c) => {
                    surface_flags = c.flags;
                }
                CapabilitySet::BitmapCodecs(BitmapCodecs(codecs)) => {
                    for codec in codecs {
                        match codec.property {
                            // FIXME: The encoder operates in image mode only.
                            //
                            // See [MS-RDPRFX] 3.1.1.1 "State Machine" for
                            // implementation of the video mode. which allows to
                            // skip sending Header for each image.
                            //
                            // We should distinguish parameters for both modes.
                            CodecProperty::RemoteFx(rdp::capability_sets::RemoteFxContainer::ClientContainer(c))
                                if self.opts.has_remote_fx() =>
                            {
                                let offered = c.caps_data.0.0.iter().map(|caps| caps.entropy_bits);
                                let preferred = self.opts.remotefx_entropy_coder;
                                if let Some(entropy_bits) = pick_remotefx_entropy_coder(preferred, offered) {
                                    update_codecs.set_remotefx(Some((entropy_bits, codec.id)));
                                    update_codecs.set_remotefx_quant(self.opts.remotefx_quant.clone());
                                }
                            }
                            CodecProperty::ImageRemoteFx(rdp::capability_sets::RemoteFxContainer::ClientContainer(
                                c,
                            )) if self.opts.has_image_remote_fx() => {
                                let offered = c.caps_data.0.0.iter().map(|caps| caps.entropy_bits);
                                let preferred = self.opts.remotefx_entropy_coder;
                                if let Some(entropy_bits) = pick_remotefx_entropy_coder(preferred, offered) {
                                    update_codecs.set_remotefx(Some((entropy_bits, codec.id)));
                                    update_codecs.set_remotefx_quant(self.opts.remotefx_quant.clone());
                                }
                            }
                            #[cfg(feature = "nscodec")]
                            CodecProperty::NsCodec(client_ns) if self.opts.has_nscodec() => {
                                // Re-use the client's confirmed color-loss
                                // level so the server encodes at the same
                                // shift the client decodes against.
                                update_codecs.set_nscodec(Some((codec.id, client_ns.color_loss_level)));
                            }
                            CodecProperty::NsCodec(_) => (),
                            #[cfg(feature = "qoi")]
                            CodecProperty::Qoi if self.opts.has_qoi() => {
                                update_codecs.set_qoi(Some(codec.id));
                            }
                            #[cfg(feature = "qoiz")]
                            CodecProperty::QoiZ if self.opts.has_qoiz() => {
                                update_codecs.set_qoiz(Some(codec.id));
                            }
                            _ => (),
                        }
                    }
                }
                _ => {}
            }
        }

        let desktop_size = self.display.lock().await.size().await;
        let encoder = UpdateEncoder::new(desktop_size, surface_flags, update_codecs, self.opts.max_request_size)?;

        self.send_next_auto_reconnect_cookie(writer, result.io_channel_id, result.user_channel_id)
            .await?;

        let state = self
            .client_loop(
                reader,
                writer,
                result.io_channel_id,
                result.user_channel_id,
                result.message_channel_id,
                encoder,
            )
            .await?;

        Ok(state)
    }

    async fn handle_input_backlog(
        &mut self,
        writer: &mut impl FramedWrite,
        io_channel_id: u16,
        user_channel_id: u16,
        message_channel_id: Option<u16>,
        frames: Vec<Vec<u8>>,
    ) -> ServerResult<()> {
        for frame in frames {
            match Action::from_fp_output_header(frame[0]) {
                Ok(Action::FastPath) => {
                    let input = decode(&frame).map_err(ServerError::decode)?;
                    self.handle_fastpath(input).await;
                }

                Ok(Action::X224) => {
                    let _ = self
                        .handle_x224(writer, io_channel_id, user_channel_id, message_channel_id, &frame)
                        .await;
                }

                // the frame here is always valid, because otherwise it would
                // have failed during the acceptor loop
                Err(_) => unreachable!(),
            }
        }

        Ok(())
    }

    async fn handle_fastpath(&mut self, input: FastPathInput) {
        for event in input.input_events().iter().copied() {
            let mut handler = self.handler.lock().await;
            match event {
                FastPathInputEvent::KeyboardEvent(flags, key) => {
                    handler.keyboard((key, flags).into());
                }

                FastPathInputEvent::UnicodeKeyboardEvent(flags, key) => {
                    handler.keyboard((key, flags).into());
                }

                FastPathInputEvent::SyncEvent(flags) => {
                    handler.keyboard(flags.into());
                }

                FastPathInputEvent::MouseEvent(mouse) => {
                    handler.mouse(mouse.into());
                }

                FastPathInputEvent::MouseEventEx(mouse) => {
                    handler.mouse(mouse.into());
                }

                FastPathInputEvent::MouseEventRel(mouse) => {
                    handler.mouse(mouse.into());
                }

                FastPathInputEvent::QoeEvent(quality) => {
                    warn!("Received QoE: {}", quality);
                }
            }
        }
    }

    async fn handle_io_channel_data(&mut self, data: SendDataRequest<'_>) -> ServerResult<bool> {
        let control: rdp::headers::ShareControlHeader = decode(data.user_data.as_ref()).map_err(ServerError::decode)?;

        match control.share_control_pdu {
            ShareControlPdu::Data(header) => match header.share_data_pdu {
                rdp::headers::ShareDataPdu::Input(pdu) => {
                    self.handle_input_event(pdu).await;
                }

                rdp::headers::ShareDataPdu::ShutdownRequest => {
                    return Ok(true);
                }

                // Client requests the server stop or resume sending display
                // updates. mstsc sends `desktop_rect: None` on minimize and
                // `desktop_rect: Some(rect)` on refocus. Without honoring
                // this, the server keeps streaming high-bitrate EGFX/H.264
                // frames into a minimized client; on refocus the client
                // must chew through the accumulated backlog before it can
                // present the current frame, locking up its input dispatch
                // for seconds. Flagging the shared `display_suppressed`
                // lets the display backend skip frame emission while it's
                // set.
                rdp::headers::ShareDataPdu::SuppressOutput(pdu) => {
                    let suppress = pdu.desktop_rect.is_none();
                    self.display_suppressed.store(suppress, Ordering::Relaxed);
                    debug!(suppress, "client suppress-output state changed");
                }

                // Client asks the server to redraw a rectangle — typical on
                // refocus after a minimize. Clear the suppress flag so the
                // backend resumes emission and treat this as "client wants
                // updates again." (The flag would also be cleared by the
                // `SuppressOutput { Some(rect) }` that usually accompanies
                // this; clearing here is belt-and-braces against clients
                // that send only one of the two.)
                rdp::headers::ShareDataPdu::RefreshRectangle(_) => {
                    if self.display_suppressed.swap(false, Ordering::Relaxed) {
                        debug!("client RefreshRectangle cleared suppress-output state");
                    }
                }

                unexpected => {
                    warn!(?unexpected, "Unexpected share data pdu");
                }
            },

            unexpected => {
                warn!(?unexpected, "Unexpected share control");
            }
        }

        Ok(false)
    }

    fn handle_message_channel_data(&mut self, data: SendDataRequest<'_>) {
        // The MCS message channel currently carries only the auto-detect
        // response. It is framed by a Basic Security Header (SEC_AUTODETECT_RSP),
        // not a Share Control header.
        match decode::<rdp::autodetect::AutoDetectRspPdu>(data.user_data.as_ref()) {
            Ok(pdu) => {
                if let Some(ref mut ad) = self.autodetect {
                    if let Some(rtt_ms) = ad.handle_response(&pdu.response, monotonic_now_ms()) {
                        self.autodetect_rtt.store(rtt_ms, Ordering::Relaxed);
                        // A matched RTT sample always updates the session-lifetime low in the
                        // same call (see `handle_response`'s RttResponse arm), so it is available
                        // unconditionally here, not just on a new low.
                        let baseline_rtt_ms = ad
                            .baseline_rtt_ms()
                            .expect("handle_response just recorded a sample above");
                        self.autodetect_baseline_rtt.store(baseline_rtt_ms, Ordering::Relaxed);
                        debug!(
                            rtt_ms,
                            baseline_rtt_ms,
                            seq = pdu.response.sequence_number(),
                            "RTT measured"
                        );
                    } else {
                        trace!(seq = pdu.response.sequence_number(), "Unmatched auto-detect response");
                    }
                }
            }
            Err(error) => {
                warn!(error = format!("{error:#}"), "Unhandled MCS message channel PDU");
            }
        }
    }

    async fn handle_x224(
        &mut self,
        writer: &mut impl FramedWrite,
        io_channel_id: u16,
        user_channel_id: u16,
        message_channel_id: Option<u16>,
        frame: &[u8],
    ) -> ServerResult<bool> {
        let message = decode::<X224<mcs::McsMessage<'_>>>(frame).map_err(ServerError::decode)?;
        match message.0 {
            mcs::McsMessage::SendDataRequest(data) => {
                debug!(
                    initiator_id = data.initiator_id,
                    channel_id = data.channel_id,
                    user_data_len = data.user_data.len(),
                    "McsMessage::SendDataRequest"
                );
                if data.channel_id == io_channel_id {
                    return self.handle_io_channel_data(data).await;
                }

                if message_channel_id == Some(data.channel_id) {
                    self.handle_message_channel_data(data);
                    return Ok(false);
                }

                if let Some(svc) = self.static_channels.get_by_channel_id_mut(data.channel_id) {
                    let response_pdus = svc
                        .process(&data.user_data)
                        .map_err_kind("svc process", ServerErrorKind::Pdu)?;
                    let response = server_encode_svc_messages(response_pdus, data.channel_id, user_channel_id)
                        .map_err(ServerError::encode)?;
                    writer
                        .write_all(&response)
                        .await
                        .map_err(|e| ServerError::io("write svc response", e))?;
                } else {
                    warn!(channel_id = data.channel_id, "Unexpected channel received: ID",);
                }
            }

            mcs::McsMessage::DisconnectProviderUltimatum(disconnect) => {
                if disconnect.reason == mcs::DisconnectReason::UserRequested {
                    return Ok(true);
                }
            }

            _ => {
                warn!(name = ironrdp_core::name(&message), "Unexpected mcs message");
            }
        }

        Ok(false)
    }

    async fn handle_input_event(&mut self, input: InputEventPdu) {
        for event in input.0 {
            let mut handler = self.handler.lock().await;
            match event {
                ironrdp_pdu::input::InputEvent::ScanCode(key) => {
                    handler.keyboard((key.key_code, key.flags).into());
                }

                ironrdp_pdu::input::InputEvent::Unicode(key) => {
                    handler.keyboard((key.unicode_code, key.flags).into());
                }

                ironrdp_pdu::input::InputEvent::Sync(sync) => {
                    handler.keyboard(sync.flags.into());
                }

                ironrdp_pdu::input::InputEvent::Mouse(mouse) => {
                    handler.mouse(mouse.into());
                }

                ironrdp_pdu::input::InputEvent::MouseX(mouse) => {
                    handler.mouse(mouse.into());
                }

                ironrdp_pdu::input::InputEvent::MouseRel(mouse) => {
                    handler.mouse(mouse.into());
                }

                ironrdp_pdu::input::InputEvent::Unused(_) => {}
            }
        }
    }

    async fn accept_finalize<S>(
        &mut self,
        mut framed: TokioFramed<S>,
        mut acceptor: Acceptor,
    ) -> ServerResult<TokioFramed<S>>
    where
        S: AsyncRead + AsyncWrite + Sync + Send + Unpin,
    {
        loop {
            let (new_framed, result) = ironrdp_acceptor::accept_finalize(framed, &mut acceptor)
                .await
                .map_err_kind("failed to accept client during finalize", ServerErrorKind::Connector)?;

            let (mut reader, mut writer) = split_tokio_framed(new_framed);

            match self.client_accepted(&mut reader, &mut writer, result).await? {
                RunState::Continue => {
                    unreachable!();
                }
                RunState::DeactivationReactivation { desktop_size } => {
                    // No description of such behavior was found in the
                    // specification, but apparently, we must keep the channel
                    // state as they were during reactivation. This fixes
                    // various state issues during client resize.
                    acceptor = Acceptor::new_deactivation_reactivation(
                        acceptor,
                        core::mem::take(&mut self.static_channels),
                        desktop_size,
                    )
                    .map_err_kind("deactivation-reactivation acceptor", ServerErrorKind::Connector)?;
                    framed = unsplit_tokio_framed(reader, writer);
                    continue;
                }
                RunState::Disconnect => {
                    let final_framed = unsplit_tokio_framed(reader, writer);
                    return Ok(final_framed);
                }
            }
        }
    }

    pub fn set_credentials(&mut self, creds: Option<Credentials>) {
        debug!(?creds, "Changing credentials");
        self.creds = creds
    }
}

/// Encode a server-initiated Auto-Detect Request PDU for the MCS message channel.
///
/// The request is framed by a Basic Security Header (SEC_AUTODETECT_REQ) per
/// [MS-RDPBCGR] 2.2.14.3 and carried in an MCS Send Data Indication on the
/// negotiated message channel, not as a Share Data PDU on the I/O channel.
fn encode_autodetect_request(
    request: rdp::autodetect::AutoDetectRequest,
    message_channel_id: u16,
    user_channel_id: u16,
) -> ServerResult<Vec<u8>> {
    // Auto-detect rides the MCS message channel framed by a Basic Security
    // Header (SEC_AUTODETECT_REQ), not a Share Control / Share Data header.
    let pdu = rdp::autodetect::AutoDetectReqPdu::new(request);
    let user_data = encode_vec(&pdu).map_err(ServerError::encode)?.into();
    let mcs_pdu = SendDataIndication {
        initiator_id: user_channel_id,
        channel_id: message_channel_id,
        user_data,
    };
    encode_vec(&X224(mcs_pdu)).map_err(ServerError::encode)
}

/// Encode a Share Data PDU wrapped in a Share Control header and carried in an
/// MCS Send Data Indication on the I/O channel.
///
/// A general `encode_share_data_pdu` helper previously lived here for the
/// auto-detect path; #1348 rerouted auto-detect onto the message channel (see
/// [`encode_autodetect_request`]), leaving the Save Session Info sender as its
/// only user until the eviction notice (`ServerEvent::EvictedByOtherConnection`)
/// became a second one, so the encoder now lives with the former.
///
/// `pdu_source` is a parameter, not hardcoded, because the two callers
/// disagree: MS-RDPBCGR 2.2.1.19 has the server echo the client's own MCS user
/// channel ID here for a normal Share Data PDU (what Save Session Info sends),
/// but 2.2.5.1.1 requires `pduSource` to be zero specifically for
/// TS_SET_ERROR_INFO_PDU (what the eviction notice sends).
fn encode_share_data_pdu(
    share_data_pdu: rdp::headers::ShareDataPdu,
    pdu_source: u16,
    io_channel_id: u16,
    user_channel_id: u16,
) -> ServerResult<Vec<u8>> {
    let header = rdp::headers::ShareDataHeader {
        share_data_pdu,
        stream_priority: rdp::headers::StreamPriority::Medium,
        compression_flags: rdp::headers::CompressionFlags::empty(),
        compression_type: rdp::client_info::CompressionType::K8,
    };
    let pdu = rdp::headers::ShareControlHeader {
        share_id: 0,
        pdu_source,
        share_control_pdu: ShareControlPdu::Data(header),
    };
    let user_data = encode_vec(&pdu).map_err(ServerError::encode)?.into();
    let mcs_pdu = SendDataIndication {
        initiator_id: user_channel_id,
        channel_id: io_channel_id,
        user_data,
    };
    encode_vec(&X224(mcs_pdu)).map_err(ServerError::encode)
}

async fn deactivate_all(io_channel_id: u16, user_channel_id: u16, writer: &mut impl FramedWrite) -> ServerResult<()> {
    let pdu = ShareControlPdu::ServerDeactivateAll(ServerDeactivateAll);
    let pdu = rdp::headers::ShareControlHeader {
        share_id: 0,
        pdu_source: io_channel_id,
        share_control_pdu: pdu,
    };
    let user_data = encode_vec(&pdu).map_err(ServerError::encode)?.into();
    let pdu = SendDataIndication {
        initiator_id: user_channel_id,
        channel_id: io_channel_id,
        user_data,
    };
    let msg = encode_vec(&X224(pdu)).map_err(ServerError::encode)?;
    writer
        .write_all(&msg)
        .await
        .map_err(|e| ServerError::io("write deactivate_all", e))?;
    Ok(())
}

/// Send a `ServerSetErrorInfoPdu(ServerDeniedConnection)` to the client, then return.
///
/// Used to deny a connection after credential validation rejects it, mirroring the
/// acceptor's exact-match denial so both paths refuse the same spec-defined way.
async fn send_access_denied(
    io_channel_id: u16,
    user_channel_id: u16,
    writer: &mut impl FramedWrite,
) -> ServerResult<()> {
    let info = ServerSetErrorInfoPdu(ErrorInfo::ProtocolIndependentCode(
        ProtocolIndependentCode::ServerDeniedConnection,
    ));
    let user_data = encode_vec(&info).map_err(ServerError::encode)?.into();
    let pdu = SendDataIndication {
        initiator_id: user_channel_id,
        channel_id: io_channel_id,
        user_data,
    };
    let msg = encode_vec(&X224(pdu)).map_err(ServerError::encode)?;
    writer
        .write_all(&msg)
        .await
        .map_err(|e| ServerError::io("write access_denied", e))?;
    Ok(())
}

struct SharedWriter<'w, W: FramedWrite> {
    writer: Rc<Mutex<&'w mut W>>,
}

impl<W: FramedWrite> Clone for SharedWriter<'_, W> {
    fn clone(&self) -> Self {
        Self {
            writer: Rc::clone(&self.writer),
        }
    }
}

impl<W> FramedWrite for SharedWriter<'_, W>
where
    W: FramedWrite,
{
    type WriteAllFut<'write>
        = core::pin::Pin<Box<dyn Future<Output = std::io::Result<()>> + 'write>>
    where
        Self: 'write;

    fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> Self::WriteAllFut<'a> {
        Box::pin(async {
            let mut writer = self.writer.lock().await;

            writer.write_all(buf).await?;
            Ok(())
        })
    }
}

impl<'a, W: FramedWrite> SharedWriter<'a, W> {
    fn new(writer: &'a mut W) -> Self {
        Self {
            writer: Rc::new(Mutex::new(writer)),
        }
    }
}

#[cfg(test)]
mod preempt_tests {
    use core::net::Ipv4Addr;

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;

    use super::*;

    fn ctx_with_no_security() -> NegotiationContext {
        struct NoDisplay;
        #[async_trait::async_trait]
        impl RdpServerDisplay for NoDisplay {
            async fn size(&mut self) -> DesktopSize {
                DesktopSize {
                    width: 1024,
                    height: 768,
                }
            }
            async fn updates(&mut self) -> anyhow::Result<Box<dyn crate::RdpServerDisplayUpdates>> {
                // `RdpServerDisplay::updates()` still returns `anyhow::Result`
                // (its conversion to `ServerResult` is #1245, not yet landed
                // upstream) -- qualified explicitly since this module's own
                // `Result` alias is `ServerResult` now, not anyhow's.
                unreachable!("negotiation never asks for updates")
            }
        }

        NegotiationContext {
            opts: RdpServerOptions {
                addr: (Ipv4Addr::LOCALHOST, 0).into(),
                security: RdpServerSecurity::None,
                codecs: BitmapCodecs(Vec::new()),
                max_request_size: 8 * 1024 * 1024,
                honor_client_desktop_size: None,
                preempt_existing_session: true,
                remotefx_quant: Quant::default(),
                remotefx_entropy_coder: None,
            },
            creds: None,
            display: Arc::new(Mutex::new(Box::new(NoDisplay))),
        }
    }

    /// The invariant behind this feature: a connection that never completes a
    /// real RDP negotiation must NOT be treated as an eligible candidate, and
    /// so can never evict a live session. This is the case the earlier
    /// two-byte `03 00` peek got wrong — it accepted anything whose first
    /// bytes merely *looked* like a TPKT header.
    #[tokio::test]
    async fn a_candidate_sending_garbage_never_authenticates() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            // Starts with a plausible TPKT prefix — enough to fool a
            // header peek — but is not a valid X.224 Connection Request.
            stream.write_all(&[0x03, 0x00, 0xff, 0xff, 0x41, 0x41]).await.unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
        });

        let (stream, peer) = listener.accept().await.unwrap();
        let ctx = ctx_with_no_security();
        assert!(
            negotiate_candidate(&ctx, stream, peer).await.is_none(),
            "traffic that only looks like RDP must not become an eligible candidate"
        );

        client.await.unwrap();
    }

    /// The other half: a bare connect that sends nothing (a port scan, a
    /// half-open probe) must not qualify either.
    #[tokio::test]
    async fn a_candidate_that_closes_immediately_never_authenticates() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = tokio::spawn(async move {
            let stream = TcpStream::connect(addr).await.unwrap();
            drop(stream);
        });

        let (stream, peer) = listener.accept().await.unwrap();
        let ctx = ctx_with_no_security();
        assert!(
            negotiate_candidate(&ctx, stream, peer).await.is_none(),
            "a connect-and-close must not become an eligible candidate"
        );

        client.await.unwrap();
    }

    /// The eviction notice must be a properly framed Share Data PDU carrying
    /// `ERRINFO_DISCONNECTED_BY_OTHERCONNECTION` (MS-RDPBCGR 2.2.5.1.1) — that
    /// exact code is what tells a client it was replaced rather than dropped,
    /// and so what stops it auto-reconnecting into a ping-pong. Round-trips
    /// the encoding a real eviction sends.
    #[test]
    fn the_eviction_notice_is_a_share_data_pdu_with_the_takeover_code() {
        let pdu = rdp::headers::ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(
            ErrorInfo::ProtocolIndependentCode(ProtocolIndependentCode::DisconnectedByOtherconnection),
        ));
        // pdu_source=0 here: unlike the Save Session Info sender (which
        // echoes the client's own user_channel_id), MS-RDPBCGR 2.2.5.1.1
        // requires pduSource to be zero for TS_SET_ERROR_INFO_PDU.
        let bytes = encode_share_data_pdu(pdu, 0, 1003, 1002).expect("encode eviction notice");

        let x224: X224<mcs::McsMessage<'_>> = decode(&bytes).expect("decode X.224/MCS");
        let mcs::McsMessage::SendDataIndication(data) = x224.0 else {
            panic!("eviction notice must ride an MCS Send Data Indication");
        };
        let control: rdp::headers::ShareControlHeader =
            decode(data.user_data.as_ref()).expect("decode Share Control header");
        assert_eq!(
            control.pdu_source, 0,
            "MS-RDPBCGR 2.2.5.1.1 requires pduSource=0 for TS_SET_ERROR_INFO_PDU"
        );
        let ShareControlPdu::Data(header) = control.share_control_pdu else {
            panic!("eviction notice must be a Share Data PDU");
        };
        match header.share_data_pdu {
            rdp::headers::ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(
                ErrorInfo::ProtocolIndependentCode(code),
            )) => {
                assert_eq!(code, ProtocolIndependentCode::DisconnectedByOtherconnection);
            }
            other => panic!("unexpected share data pdu: {other:?}"),
        }
    }

    /// The anti-storm net: a just-evicted peer may not bounce straight back,
    /// and every refused attempt RE-ARMS the window, so an automatic reconnect
    /// loop cannot immediately outlast it. A peer that goes quiet for the
    /// cooldown (a human closing the client and reconnecting) is let back in.
    #[test]
    fn a_just_evicted_peer_cannot_bounce_back_but_a_quiet_one_can() {
        let cooldown = Duration::from_secs(5);
        let max_lockout = Duration::from_secs(30);
        let evicted: IpAddr = Ipv4Addr::new(192, 168, 4, 46).into();
        let other: IpAddr = Ipv4Addr::new(192, 168, 4, 44).into();

        let t0 = Instant::now();
        let mut state = Some(EvictedPeer {
            ip: evicted,
            evicted_at: t0,
            last_try: t0,
        });

        // An unrelated peer is never affected by someone else's eviction.
        assert!(!refuse_reconnect_from_evicted(
            &mut state,
            other,
            t0 + Duration::from_millis(500),
            cooldown,
            max_lockout,
        ));

        // The evicted peer auto-reconnecting ~1 s later is refused, and each
        // attempt pushes the window out.
        let mut at = t0;
        for _ in 0..5 {
            at += Duration::from_secs(1);
            assert!(
                refuse_reconnect_from_evicted(&mut state, evicted, at, cooldown, max_lockout),
                "an auto-reconnect storm must not immediately win the session back"
            );
        }

        // ...but once it stops hammering for the full cooldown, a deliberate
        // reconnect is allowed through.
        let quiet = at + cooldown + Duration::from_millis(1);
        assert!(
            !refuse_reconnect_from_evicted(&mut state, evicted, quiet, cooldown, max_lockout),
            "a peer that waited out the cooldown must be able to connect again"
        );
    }

    /// The re-arming window MUST NOT lock a peer out forever, or it defeats
    /// the feature's own headline case: a client whose link dropped, whose
    /// stale session is still live, auto-reconnecting to reclaim it. Past
    /// `max_lockout` from the eviction the bar lifts even under a storm that
    /// never pauses.
    #[test]
    fn the_reconnect_bar_lifts_once_the_absolute_cap_passes() {
        let cooldown = Duration::from_secs(5);
        let max_lockout = Duration::from_secs(30);
        let evicted: IpAddr = Ipv4Addr::new(192, 168, 4, 46).into();

        let t0 = Instant::now();
        let mut state = Some(EvictedPeer {
            ip: evicted,
            evicted_at: t0,
            last_try: t0,
        });

        // A relentless 1 s auto-reconnect cadence: refused while inside the
        // cap, even though every attempt re-arms the cooldown...
        let mut at = t0;
        let mut refused_while_capped = 0;
        while at < t0 + max_lockout {
            at += Duration::from_secs(1);
            if at < t0 + max_lockout {
                assert!(
                    refuse_reconnect_from_evicted(&mut state, evicted, at, cooldown, max_lockout),
                    "still inside the cap, so the storm is throttled"
                );
                refused_while_capped += 1;
            }
        }
        assert!(refused_while_capped > 0, "the test must exercise the throttled window");

        // ...and let through the moment the cap passes, WITHOUT the peer ever
        // having paused. Before the cap was added this returned true forever.
        let past_cap = t0 + max_lockout + Duration::from_millis(1);
        assert!(
            !refuse_reconnect_from_evicted(&mut state, evicted, past_cap, cooldown, max_lockout),
            "a peer must not be barred forever just for retrying; that locks out the case the feature exists for"
        );
    }

    /// A silent candidate MUST NOT be able to hang the accept loop.
    ///
    /// `negotiate_candidate` blocks on socket reads from a peer that has not
    /// authenticated. Before `CANDIDATE_NEGOTIATION_TIMEOUT` the probe was
    /// awaited unbounded: a peer that connected and then sent NOTHING parked it
    /// forever, and once the live session ended `run()` blocked on the handoff
    /// await with no `select!` left — no further accepts, no event drain, so
    /// not even `ServerEvent::Quit` could stop the server. An unauthenticated
    /// remote could wedge the listener.
    ///
    /// Drive exactly that: a live session, a silent candidate, then end the
    /// session and require the server to still respond and still shut down.
    #[tokio::test]
    async fn a_silent_candidate_cannot_wedge_the_accept_loop() {
        let local = task::LocalSet::new();
        local
            .run_until(async move {
                let mut server = RdpServer::builder()
                    .with_addr((Ipv4Addr::LOCALHOST, 0))
                    .with_no_security()
                    .with_no_input()
                    .with_no_display()
                    .with_preempt_existing_session(true)
                    .build();

                let event_sender = server.event_sender().clone();
                let run_task = task::spawn_local(async move {
                    let _ = Box::pin(server.run()).await;
                });

                let addr = loop {
                    let (tx, rx) = oneshot::channel();
                    let _ = event_sender.send(ServerEvent::GetLocalAddr(tx));
                    if let Ok(Some(addr)) = rx.await {
                        break addr;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                };

                // Client A: the live session (connect, stay silent).
                let client_a = TcpStream::connect(addr).await.unwrap();
                tokio::time::sleep(Duration::from_millis(100)).await;

                // Client B: the candidate — completes the TCP handshake, then
                // says nothing at all, parking the probe mid-`accept_begin`.
                let _client_b = TcpStream::connect(addr).await.unwrap();
                tokio::time::sleep(Duration::from_millis(100)).await;

                // The live session ends. This is the branch that used to do a
                // bare `probe.await` and never come back.
                drop(client_a);

                // The server must still be answering its event channel. With
                // the unbounded await this never resolves.
                let responsive = tokio::time::timeout(Duration::from_secs(3), async {
                    loop {
                        let (tx, rx) = oneshot::channel();
                        if event_sender.send(ServerEvent::GetLocalAddr(tx)).is_err() {
                            return;
                        }
                        if rx.await.is_ok() {
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await;
                assert!(
                    responsive.is_ok(),
                    "the accept loop stopped servicing events while a silent candidate was in flight"
                );

                // ...and must still be stoppable.
                let _ = event_sender.send(ServerEvent::Quit("test over".to_owned()));
                let stopped = tokio::time::timeout(Duration::from_secs(3), run_task).await;
                assert!(
                    stopped.is_ok(),
                    "the server ignored Quit -- the accept loop was wedged by an unauthenticated silent peer"
                );
            })
            .await;
    }

    /// A `ConnectionHandler` that accepts the first connection and rejects
    /// every one after, recording every peer it was consulted about.
    struct RejectAfterFirst {
        seen: Arc<std::sync::Mutex<Vec<SocketAddr>>>,
        accepted_once: bool,
    }

    impl ConnectionHandler for RejectAfterFirst {
        fn on_accept(&mut self, peer: SocketAddr) -> bool {
            self.seen.lock().unwrap().push(peer);
            !core::mem::replace(&mut self.accepted_once, true)
        }
    }

    /// Drives a real `RdpServer::run()` accept loop over TCP with preemption
    /// on. A candidate must be gated through `ConnectionHandler::on_accept`
    /// *before* it is allowed to negotiate — so a rate limiter or IP allowlist
    /// can stop a takeover, rather than only learning about it after the live
    /// session was already evicted.
    #[tokio::test]
    async fn a_candidate_is_gated_through_on_accept_before_it_can_preempt() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_for_handler = Arc::clone(&seen);

        let local = task::LocalSet::new();
        local
            .run_until(async move {
                let mut server = RdpServer::builder()
                    .with_addr((Ipv4Addr::LOCALHOST, 0))
                    .with_no_security()
                    .with_no_input()
                    .with_no_display()
                    .with_connection_handler(Some(Box::new(RejectAfterFirst {
                        seen: seen_for_handler,
                        accepted_once: false,
                    })))
                    .with_preempt_existing_session(true)
                    .build();

                let event_sender = server.event_sender().clone();
                let run_task = task::spawn_local(async move {
                    let _ = Box::pin(server.run()).await;
                });

                // Learn the ephemeral port (retrying while `run()` binds).
                let addr = loop {
                    let (tx, rx) = oneshot::channel();
                    let _ = event_sender.send(ServerEvent::GetLocalAddr(tx));
                    if let Ok(Some(addr)) = rx.await {
                        break addr;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                };

                // Client A: connect and go silent. `run_connection` parks
                // reading the first PDU, which is all "a live session" needs
                // to be here.
                let mut client_a = TcpStream::connect(addr).await.unwrap();
                tokio::time::sleep(Duration::from_millis(100)).await;

                // Client B: the candidate. The handler rejects it, so it must
                // never reach negotiation, let alone evict client A.
                let mut client_b = TcpStream::connect(addr).await.unwrap();
                client_b.write_all(&[0x03, 0x00]).await.unwrap();

                let mut buf = [0u8; 1];
                let client_b_read = tokio::time::timeout(Duration::from_secs(2), client_b.read(&mut buf)).await;
                assert!(
                    matches!(client_b_read, Ok(Ok(0)) | Ok(Err(_))),
                    "the rejected candidate's connection should be closed, got {client_b_read:?}"
                );

                // Client A is untouched: a read TIMES OUT (no EOF, no data)
                // rather than showing the server dropped it for client B.
                let client_a_still_alive =
                    tokio::time::timeout(Duration::from_millis(300), client_a.read(&mut buf)).await;
                assert!(
                    client_a_still_alive.is_err(),
                    "the live session must survive a rejected preemption attempt, got {client_a_still_alive:?}"
                );

                // The handler was actually consulted about the candidate —
                // the gate ran on B, not merely on A.
                let seen = seen.lock().unwrap().clone();
                assert_eq!(
                    seen.len(),
                    2,
                    "on_accept should have been consulted for both peers: {seen:?}"
                );

                run_task.abort();
            })
            .await;
    }

    /// Regression guard for the blocking review finding: invalidating an
    /// evicted peer's ARC cookie must NOT permanently disable auto-reconnect
    /// for the server. `next_auto_reconnect_cookie` treats
    /// `self.auto_reconnect_cookie == None` as "unconfigured" and stops
    /// issuing cookies to EVERY future connection -- a naive
    /// `self.auto_reconnect_cookie = None` on eviction would have silently
    /// killed the feature server-wide the moment the first eviction ever
    /// happened.
    #[test]
    fn invalidating_the_evicted_peers_cookie_does_not_disable_auto_reconnect() {
        let mut server = RdpServer::builder()
            .with_addr((Ipv4Addr::LOCALHOST, 0))
            .with_no_security()
            .with_no_input()
            .with_no_display()
            .build();

        let seed = RdpServer::generate_auto_reconnect_cookie(4242);
        server.set_auto_reconnect_cookie(Some(seed.clone()));
        // A normal rotation would have run once by the time a real session
        // reaches an eviction; simulate that so `previous_auto_reconnect_
        // cookie` starts populated, which is the slot the fix must ALSO clear.
        server.commit_auto_reconnect_rotation(RdpServer::generate_auto_reconnect_cookie(seed.logon_id));

        server.invalidate_auto_reconnect_cookie_on_eviction();

        let after = server
            .auto_reconnect_cookie
            .as_ref()
            .expect("invalidation must not clear the cookie to None -- that permanently disables auto-reconnect");
        assert_eq!(after.logon_id, seed.logon_id, "the logon_id lineage must be preserved");
        assert_ne!(
            after.random_bits, seed.random_bits,
            "the random bits must actually change, or the evicted peer's OLD cookie would still verify"
        );
        assert!(
            server.previous_auto_reconnect_cookie.is_none(),
            "the outgoing cookie(s) must be discarded, not demoted into previous_ -- demoting would keep an \
             evicted peer's cookie valid for one more attempt, which is exactly the gap this closes"
        );
    }

    /// `discard_stale_session_events` must be an ALLOWLIST of lifecycle
    /// events, not a denylist of `EvictedByOtherConnection` alone -- otherwise
    /// a preemption winner is handed whatever per-session events (audio
    /// waves, clipboard messages, EGFX frames) the session it replaced left
    /// queued but never consumed.
    #[tokio::test]
    async fn stale_session_events_are_dropped_but_lifecycle_events_survive() {
        let mut server = RdpServer::builder()
            .with_addr((Ipv4Addr::LOCALHOST, 0))
            .with_no_security()
            .with_no_input()
            .with_no_display()
            .build();

        let sender = server.event_sender().clone();
        // A representative per-session event that must NOT survive a
        // takeover -- it belonged to the session just replaced.
        let _ = sender.send(ServerEvent::AutoDetectRttRequest);
        // A lifecycle event that MUST survive.
        let _ = sender.send(ServerEvent::Quit("keep me".to_owned()));

        server.discard_stale_session_events().await;

        let remaining = {
            let mut rx = server.ev_receiver.lock().await;
            let mut events = Vec::new();
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
            events
        };

        assert_eq!(
            remaining.len(),
            1,
            "exactly the lifecycle event should have survived the drain: {remaining:?}"
        );
        assert!(
            matches!(&remaining[0], ServerEvent::Quit(reason) if reason == "keep me"),
            "the surviving event should be the Quit, not the discarded per-session event: {remaining:?}"
        );
    }

    /// Regression guard for the "no happy-path test" review finding, and the
    /// load-bearing claim it's actually worried about: that deferring
    /// `attach_channels` to `serve_negotiated` does NOT silently drop a
    /// preemption winner's channels. That claim rests entirely on
    /// `accept_begin` stopping at `AcceptorState::SecurityUpgrade` -- before
    /// `BasicSettingsWaitInitial` consumes `static_channels` -- for
    /// [`RdpServerSecurity::None`]. If a future `ironrdp-acceptor` change
    /// moved that stop point, a preemption winner would negotiate ZERO static
    /// channels (no clipboard, no sound, no DVC) and every OTHER test in this
    /// module would still pass, since none of them drive a candidate all the
    /// way to `serve_negotiated`.
    ///
    /// Drives a REAL candidate through `negotiate_candidate` (a genuine X.224
    /// Connection Request, matching `RdpServerSecurity::None`'s empty
    /// protocol flags, is enough to reach `BeginResult::Continue` -- no TLS,
    /// no MCS needed) and then `serve_negotiated`, and asserts the sound
    /// factory's backend was actually built. `serve_negotiated` can't finish
    /// without a full MCS/GCC handshake this test doesn't drive, so it's
    /// bounded by a short timeout that is EXPECTED to fire -- the assertion
    /// that matters is the side effect that happens before that point.
    #[tokio::test]
    async fn a_winning_candidate_actually_gets_its_channels_attached() {
        let backend_built = Arc::new(AtomicBool::new(false));

        #[derive(Debug)]
        struct RecordingSoundHandler;
        impl ironrdp_rdpsnd::server::RdpsndServerHandler for RecordingSoundHandler {
            fn get_formats(&self) -> &[ironrdp_rdpsnd::pdu::AudioFormat] {
                &[]
            }
            fn choose_format<'a>(
                &mut self,
                _common: &'a [ironrdp_rdpsnd::server::NegotiatedFormat],
            ) -> Option<&'a ironrdp_rdpsnd::server::NegotiatedFormat> {
                None
            }
            fn start(
                &mut self,
                _format: &ironrdp_rdpsnd::server::NegotiatedFormat,
            ) -> Result<(), Box<dyn ironrdp_rdpsnd::server::RdpsndError>> {
                Ok(())
            }
            fn stop(&mut self) {}
        }

        #[derive(Debug)]
        struct RecordingSoundFactory(Arc<AtomicBool>);
        impl ServerEventSender for RecordingSoundFactory {
            fn set_sender(&mut self, _sender: mpsc::UnboundedSender<ServerEvent>) {}
        }
        impl SoundServerFactory for RecordingSoundFactory {
            fn build_backend(&self) -> Box<dyn ironrdp_rdpsnd::server::RdpsndServerHandler> {
                self.0.store(true, Ordering::SeqCst);
                Box::new(RecordingSoundHandler)
            }
        }

        let local = task::LocalSet::new();
        local
            .run_until(Box::pin(async move {
                let mut server = RdpServer::builder()
                    .with_addr((Ipv4Addr::LOCALHOST, 0))
                    .with_no_security()
                    .with_no_input()
                    .with_no_display()
                    .with_sound_factory(Some(Box::new(RecordingSoundFactory(Arc::clone(&backend_built)))))
                    .build();

                let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
                let addr = listener.local_addr().unwrap();

                let client = tokio::spawn(async move {
                    let mut stream = TcpStream::connect(addr).await.unwrap();
                    let cr = nego::ConnectionRequest {
                        nego_data: None,
                        flags: nego::RequestFlags::empty(),
                        // Matches `RdpServerSecurity::None`'s
                        // `RdpServerSecurity::flag()` exactly -- this is what
                        // makes `accept_begin` reach `BeginResult::Continue`.
                        protocol: nego::SecurityProtocol::empty(),
                        correlation_info: None,
                    };
                    let bytes = encode_vec(&X224(cr)).unwrap();
                    stream.write_all(&bytes).await.unwrap();
                    // Hold the connection open; `serve_negotiated` will try
                    // to read the next (MCS) PDU, which this test never
                    // sends, so the read simply blocks until the test ends.
                    tokio::time::sleep(Duration::from_secs(5)).await;
                });

                let (stream, peer) = listener.accept().await.unwrap();
                let (candidate, _peer) = negotiate_candidate(&server.negotiation_context(), stream, peer)
                    .await
                    .expect("a well-formed X.224 Connection Request under RdpServerSecurity::None must authenticate");

                // Bounded: `serve_negotiated` cannot finish without a full
                // MCS/GCC handshake this test doesn't drive, so timing out is
                // the EXPECTED outcome here -- `attach_channels`'s
                // synchronous side effect (below) already happened before
                // `serve_negotiated` reached its first blocking read.
                let _ = tokio::time::timeout(Duration::from_millis(300), server.serve_negotiated(candidate)).await;

                assert!(
                    backend_built.load(Ordering::SeqCst),
                    "the winning candidate's sound backend was never built -- attach_channels was not called, \
                     meaning this preemption winner would have gotten NO static channels at all"
                );

                client.abort();
            }))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_core::impl_as_any;
    use ironrdp_pdu::gcc::ChannelName;
    use ironrdp_svc::{SvcMessage, SvcServerProcessor};

    use super::*;

    /// A channel backend that owns a resource, released on drop the way
    /// `RdpsndServer` stops its handler.
    #[derive(Debug)]
    struct ResourceChannel(Arc<AtomicBool>);

    impl Drop for ResourceChannel {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    impl_as_any!(ResourceChannel);

    impl SvcProcessor for ResourceChannel {
        fn channel_name(&self) -> ChannelName {
            ChannelName::from_static(b"testchan")
        }

        fn process(&mut self, _payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
            Ok(Vec::new())
        }
    }

    impl SvcServerProcessor for ResourceChannel {}

    #[tokio::test]
    async fn run_connection_releases_the_static_channels() {
        let mut server = RdpServer::builder()
            .with_addr(([127, 0, 0, 1], 0))
            .with_no_security()
            .with_no_input()
            .with_no_display()
            .build();

        let released = Arc::new(AtomicBool::new(false));
        server.static_channels.insert(ResourceChannel(Arc::clone(&released)));

        // A stream that is already at EOF: the connection ends early, which
        // is the path an embedder's accept loop sees when a client vanishes.
        let (client, server_side) = tokio::io::duplex(64);
        drop(client);
        let _ = server.run_connection(server_side).await;

        assert!(
            released.load(Ordering::Relaxed),
            "the channel backends of a finished connection must be released, not held until the next client"
        );
    }
}
