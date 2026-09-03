use core::fmt;
use core::net::SocketAddr;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
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
    BitmapCodecs, CapabilitySet, CmdFlags, CodecProperty, EntropyBits, GeneralExtraFlags, LargePointerSupportFlags,
};
pub use ironrdp_pdu::rdp::client_info::Credentials;
use ironrdp_pdu::rdp::headers::{ServerDeactivateAll, ShareControlPdu};
use ironrdp_pdu::rdp::server_error_info::{ErrorInfo, ProtocolIndependentCode, ServerSetErrorInfoPdu};
use ironrdp_pdu::x224::X224;
use ironrdp_pdu::{Action, PduResult, decode_err, mcs, nego, rdp};
use ironrdp_rdpdr as rdpdr;
use ironrdp_rdpsnd as rdpsnd;
use ironrdp_svc::{ChannelFlags, StaticChannelId, StaticChannelSet, SvcProcessor, server_encode_svc_messages};
use ironrdp_tokio::{FramedRead, FramedWrite, TokioFramed, split_tokio_framed, unsplit_tokio_framed};
use rand::RngCore as _;
use rdpdr::server::{RdpdrServer, RdpdrServerMessage};
use rdpsnd::server::{RdpsndServer, RdpsndServerMessage};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};
use tokio::net::TcpSocket;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task;
use tokio_rustls::TlsAcceptor;
use tracing::{debug, error, trace, warn};

use crate::autodetect::{AutoDetectManager, AutoDetectOutcome, RttSnapshot};
use crate::clipboard::CliprdrServerFactory;
use crate::display::{DisplayUpdate, RdpServerDisplay};
use crate::echo::{EchoDvcBridge, EchoServerHandle, EchoServerMessage, build_echo_request};
use crate::encoder::{UpdateEncoder, UpdateEncoderCodecs};
use crate::error::{ServerError, ServerErrorExt as _, ServerErrorKind, ServerResult};
#[cfg(feature = "egfx")]
use crate::gfx::{EgfxServerMessage, GfxServerFactory};
use crate::handler::RdpServerInputHandler;
use crate::heartbeat::HeartbeatConfig;
use crate::rdpei::RdpeiServerFactory;
#[cfg(feature = "usb")]
use crate::urbdrc::{
    DeviceFactory, ServerDeviceIoReq, ServerUsbDevice, UrbdrcDeviceServerMessage, UrbdrcServerMessage,
    UsbControlHandle, UsbDeviceHandle,
};
use crate::{RdpdrServerFactory, SoundServerFactory, builder, capabilities};
#[cfg(feature = "usb")]
use ironrdp_rdpeusb::{InterfaceAlloc, server::UrbdrcControlServer, server::UrbdrcDeviceServer};

/// TCP listen backlog size for the RDP server socket.
const LISTENER_BACKLOG: u32 = 1024;
const AUTO_RECONNECT_COOKIE_UPDATE_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// How long a single [`ironrdp_acceptor::accept_finalize`] pass may take before
/// the connection is dropped.
///
/// A client can complete the whole security handshake — X.224, TLS, and CredSSP
/// where applicable — and then stop producing PDUs, leaving the server blocked
/// on a socket read with no timeout. Because `RdpServer` serves one connection
/// at a time, that connection then holds the server indefinitely: the socket
/// stays ESTABLISHED with both TCP queues empty, and nothing tears it down.
/// This has been observed in the field against a real client, which completed
/// MCS Connect, Erect Domain, Attach User and every channel join and then never
/// sent its Client Info PDU; the connection sat there for 45 minutes.
///
/// Generous on purpose: a healthy finalize is sub-second, and even over a slow
/// mobile link with heavy retransmits the whole pass stays within a few
/// seconds. A false timeout is cheap — the connection is dropped and the client
/// reconnects (immediately, if it holds an auto-reconnect cookie).
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(30);

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
    router: HashMap<DynamicChannelId, Arc<ServerUsbDevice>>,
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
///#     async fn updates(&mut self) -> ServerResult<Box<dyn RdpServerDisplayUpdates>> {
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
    rdpdr_factory: Option<Box<dyn RdpdrServerFactory>>,
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
    heartbeat: Option<HeartbeatConfig>,
    connection_handler: Option<Box<dyn ConnectionHandler>>,
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

    /// Latest NetworkAutoDetect measured bandwidth in kilobits per second, or
    /// `u32::MAX` until the first measurement completes (and while auto-detect
    /// is disabled). Updated whenever a Bandwidth Measure Results response is
    /// processed, same trigger point as [`Self::autodetect_rtt`]. Exposed via
    /// [`Self::autodetect_bandwidth_handle`]: without it, the server can tell
    /// the *client* its measured bandwidth over the wire but has no way to
    /// tell the embedder, which the connect-time figure carried to the client
    /// alone does not fix.
    autodetect_bandwidth: Arc<AtomicU32>,

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
    #[expect(
        clippy::result_large_err,
        reason = "SendError<ServerEvent> hands the whole event back on a closed channel; ServerEvent's size is \
                  driven by its largest per-channel payload (RdpdrServerMessage), not by anything this method does"
    )]
    pub fn set(
        &self,
        cookie: Option<rdp::session_info::ServerAutoReconnect>,
    ) -> Result<(), mpsc::error::SendError<ServerEvent>> {
        self.sender.send(ServerEvent::SetAutoReconnectCookie(cookie))
    }
}

/// Cloneable handle for gracefully disconnecting the active client with a
/// `ServerSetErrorInfo` PDU (MS-RDPBCGR 2.2.5.1) while [`RdpServer::run`]
/// owns the server.
#[derive(Clone)]
pub struct ErrorInfoDisconnectHandle {
    sender: mpsc::UnboundedSender<ServerEvent>,
}

impl ErrorInfoDisconnectHandle {
    /// Send `error` to the client via a `ServerSetErrorInfoPdu`, then close the
    /// connection.
    ///
    /// The disconnect takes effect only after the server handles this event.
    /// Unlike [`ServerEvent::Quit`], the client is told why: it decodes the
    /// PDU and can surface `error` to the user before the connection drops.
    #[expect(
        clippy::result_large_err,
        reason = "SendError<ServerEvent> hands the whole event back on a closed channel; ServerEvent's size is \
                  driven by its largest per-channel payload (RdpdrServerMessage), not by anything this method does"
    )]
    pub fn disconnect(&self, error: ErrorInfo) -> Result<(), mpsc::error::SendError<ServerEvent>> {
        self.sender.send(ServerEvent::Disconnect(error))
    }
}

pub enum ServerEvent {
    Quit(String),
    /// Disconnect the active client with a `ServerSetErrorInfoPdu` carrying
    /// the given reason. See [`ErrorInfoDisconnectHandle::disconnect`].
    Disconnect(ErrorInfo),
    Clipboard(ClipboardMessage),
    Rdpsnd(RdpsndServerMessage),
    Rdpdr(RdpdrServerMessage),
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
            Self::Disconnect(error) => f.debug_tuple("Disconnect").field(error).finish(),
            Self::Clipboard(..) => f.write_str("Clipboard(..)"),
            Self::Rdpsnd(..) => f.write_str("Rdpsnd(..)"),
            Self::Rdpdr(..) => f.write_str("Rdpdr(..)"),
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

#[derive(Debug, PartialEq)]
enum RunState {
    Continue,
    Disconnect,
    DeactivationReactivation { desktop_size: DesktopSize },
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
        mut rdpdr_factory: Option<Box<dyn RdpdrServerFactory>>,
        connection_handler: Option<Box<dyn ConnectionHandler>>,
        #[cfg(feature = "egfx")] mut gfx_factory: Option<Box<dyn GfxServerFactory>>,
        display_suppressed: Option<Arc<AtomicBool>>,
        #[cfg(feature = "usb")] usb_factory: Option<Box<dyn DeviceFactory>>,
        autodetect_rtt: Option<Arc<AtomicU32>>,
        autodetect_baseline_rtt: Option<Arc<AtomicU32>>,
        autodetect_bandwidth: Option<Arc<AtomicU32>>,
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
        if let Some(rdpdr) = rdpdr_factory.as_mut() {
            rdpdr.set_sender(ev_sender.clone());
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
            rdpdr_factory,
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
            heartbeat: None,
            connection_handler,
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
            autodetect_bandwidth: {
                let handle = autodetect_bandwidth.unwrap_or_else(|| Arc::new(AtomicU32::new(u32::MAX)));
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

    /// Returns a handle for gracefully disconnecting the active client with a
    /// `ServerSetErrorInfo` PDU while [`Self::run`] owns this server.
    pub fn error_info_disconnect_handle(&self) -> ErrorInfoDisconnectHandle {
        ErrorInfoDisconnectHandle {
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
        let data = encode_share_data_pdu(pdu, io_channel_id, user_channel_id)?;
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

        let Some(device) = usb_man.router.remove(&dvc_id) else {
            trace!(dvc_id, "Closed USB device is absent from request router");
            return;
        };

        // Set the terminal state before failing waiters: a woken PendingRequest
        // must not enqueue CANCEL_REQUEST for a removed DVC. The pending map is
        // shared, so dropping the router entry no longer drops it.
        device.mark_closed();
        let pending_requests = device.drain_pending();
        debug!(
            dvc_id,
            pending_requests, "Removed closed USB device from request router"
        );
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

    /// Returns a handle to the latest NetworkAutoDetect measured bandwidth in
    /// kilobits per second (`u32::MAX` until the first measurement completes,
    /// and while auto-detect is disabled). The server updates it whenever a
    /// Bandwidth Measure Results response completes a measurement; backends
    /// clone the handle to read the figure the server also reports to the
    /// client on the wire. Inject a shared instance at construction with
    /// [`RdpServerBuilder::with_autodetect_bandwidth_handle`](crate::RdpServerBuilder::with_autodetect_bandwidth_handle).
    pub fn autodetect_bandwidth_handle(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.autodetect_bandwidth)
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

    /// Enable periodic Server Heartbeat PDUs (MS-RDPBCGR 2.2.16.1).
    ///
    /// Heartbeats ride the MCS message channel, so they are only emitted
    /// when the client requested one AND advertised
    /// `RNS_UD_CS_SUPPORT_HEARTBEAT_PDU` in its early capability flags, and,
    /// per the spec's idle-only SHOULD, only when no other PDU went out
    /// during the previous heartbeat interval.
    pub fn enable_heartbeat(&mut self, config: HeartbeatConfig) {
        self.heartbeat = Some(config);
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

        if let Some(factory) = self.rdpdr_factory.as_deref() {
            let backend = factory.build_backend();

            acceptor.attach_static_channel(RdpdrServer::new(backend));
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

    /// Run a single RDP connection over `stream`, performing the
    /// IronRDP-managed TLS handshake on `ShouldUpgrade` (standard TCP+TLS).
    ///
    /// Socket options on `stream` are the caller's to set. In particular RDP
    /// is a stream of small, latency-sensitive writes, so a TCP stream should
    /// have `TCP_NODELAY` set; [`RdpServer::run`] does that for the
    /// connections it accepts itself.
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
    /// Socket options on `stream` are the caller's to set; see
    /// [`run_connection`](Self::run_connection).
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

        let framed = TokioFramed::new(stream);

        let size = self.display.lock().await.size().await;
        let capabilities = capabilities::capabilities(&self.opts, size);
        let mut acceptor = Acceptor::new(self.opts.security.flag(), size, capabilities, self.creds.clone());
        acceptor.set_honor_client_desktop_size(self.opts.honor_client_desktop_size);

        self.attach_channels(&mut acceptor);

        let res = ironrdp_acceptor::accept_begin(framed, &mut acceptor)
            .await
            .map_err_kind("accept_begin failed", ServerErrorKind::Connector)?;

        match res {
            // The only thing that varies between the two modes is who performs
            // the TLS handshake; everything past it is `finalize_after_upgrade`.
            BeginResult::ShouldUpgrade(stream) => match tls {
                TransportTls::Managed => {
                    let tls_acceptor = match &self.opts.security {
                        RdpServerSecurity::Tls(acceptor) => acceptor,
                        RdpServerSecurity::Hybrid((acceptor, _)) => acceptor,
                        RdpServerSecurity::None => unreachable!(),
                    };
                    let accept = match tls_acceptor.accept(stream).await {
                        Ok(accept) => accept,
                        Err(e) => {
                            warn!("Failed to TLS accept: {}", e);
                            return Ok(());
                        }
                    };
                    self.finalize_after_upgrade(TokioFramed::new(accept), acceptor, "TLS connection")
                        .await?;
                }
                TransportTls::AlreadyDone => {
                    // The stream is already past TLS (terminated at a lower
                    // layer, e.g. a WSS terminator); do NOT call
                    // tls_acceptor.accept on it.
                    self.finalize_after_upgrade(TokioFramed::new(stream), acceptor, "TLS-offloaded stream")
                        .await?;
                }
            },

            BeginResult::Continue(framed) => {
                self.accept_finalize(framed, acceptor).await?;
            }
        };

        Ok(())
    }

    /// Shared post-handshake tail for both [`TransportTls`] modes: mark the
    /// security upgrade complete, run the optional Hybrid CredSSP exchange,
    /// finalize, and shut the stream down. Single-sourcing this is what keeps
    /// the managed and TLS-offloaded paths structurally identical past the
    /// handshake, so per-connection state handling cannot drift between them.
    async fn finalize_after_upgrade<S>(
        &mut self,
        mut framed: TokioFramed<S>,
        mut acceptor: Acceptor,
        shutdown_label: &str,
    ) -> ServerResult<()>
    where
        S: AsyncRead + AsyncWrite + Sync + Send + Unpin,
    {
        acceptor.mark_security_upgrade_as_done();

        if let RdpServerSecurity::Hybrid((_, pub_key)) = &self.opts.security {
            // Generic streams don't expose peer address. Use a neutral
            // placeholder; it's unclear whether CredSSP/NTLM actually
            // uses this value in practice.
            let client_name = "rdp-client".to_owned();

            ironrdp_acceptor::accept_credssp(
                &mut framed,
                &mut acceptor,
                &mut ironrdp_tokio::reqwest::ReqwestNetworkClient::new(),
                client_name.into(),
                pub_key.clone(),
                None,
            )
            .await
            .map_err_kind("accept_credssp", ServerErrorKind::Connector)?;
        }

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

        loop {
            let ev_receiver = Arc::clone(&self.ev_receiver);
            let mut ev_receiver = ev_receiver.lock().await;
            tokio::select! {
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
                },
                Ok((stream, peer)) = listener.accept() => {
                    debug!(?peer, "Received connection");
                    drop(ev_receiver);

                    let accepted = self.connection_handler
                        .as_mut()
                        .is_none_or(|h| h.on_accept(peer));

                    if !accepted {
                        debug!(?peer, "Connection rejected by handler");
                        drop(stream);
                    } else {
                        // RDP output is small writes the peer is waiting on: a
                        // frame, a pointer update, a channel PDU. Nagle holds
                        // the trailing partial segment of each until the
                        // previous is acknowledged, which against a peer using
                        // delayed acknowledgements is dead time on every one.
                        // Not worth refusing a connection over, though.
                        if let Err(error) = stream.set_nodelay(true) {
                            warn!(?peer, %error, "Failed to set TCP_NODELAY; interactive latency may suffer");
                        }
                        let started = tokio::time::Instant::now();
                        let result = self.run_connection(stream).await;
                        let duration = started.elapsed();

                        if let Err(ref error) = result {
                            error!(?error, "Connection error");
                        }

                        if let Some(ref mut handler) = self.connection_handler {
                            let action = handler.on_disconnected(
                                peer,
                                duration,
                                result.as_ref().err(),
                            );
                            if action == PostConnectionAction::Stop {
                                debug!(?peer, "Handler requested stop after disconnect");
                                break;
                            }
                        }
                    }
                }
                else => break,
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
                    .map_err(|e| ServerError::io("failed to write display update", e))?;
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
                ServerEvent::Disconnect(error) => {
                    debug!(?error, "Got disconnect event");
                    let pdu = rdp::headers::ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(error));
                    let data = encode_share_data_pdu(pdu, io_channel_id, user_channel_id)?;
                    writer
                        .write_all(&data)
                        .await
                        .map_err(|e| ServerError::io("send server set error info", e))?;
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
                ServerEvent::Rdpdr(msg) => {
                    let Some(rdpdr) = self.get_svc_processor::<RdpdrServer>() else {
                        warn!("No rdpdr channel, dropping event");
                        continue;
                    };
                    let msgs = match msg {
                        RdpdrServerMessage::Create {
                            device_id,
                            path,
                            desired_access,
                            create_disposition,
                            create_options,
                        } => rdpdr.drive_create(device_id, path, desired_access, create_disposition, create_options),
                        RdpdrServerMessage::Read {
                            device_id,
                            file_id,
                            length,
                            offset,
                        } => rdpdr.drive_read(device_id, file_id, length, offset),
                        RdpdrServerMessage::Write {
                            device_id,
                            file_id,
                            data,
                            offset,
                        } => rdpdr.drive_write(device_id, file_id, data, offset),
                        RdpdrServerMessage::Close { device_id, file_id } => rdpdr.drive_close(device_id, file_id),
                        RdpdrServerMessage::FlushBuffers { device_id, file_id } => {
                            rdpdr.drive_flush_buffers(device_id, file_id)
                        }
                        RdpdrServerMessage::QueryInformation {
                            device_id,
                            file_id,
                            info_class,
                        } => rdpdr.drive_query_information(device_id, file_id, info_class),
                        RdpdrServerMessage::SetInformation {
                            device_id,
                            file_id,
                            set_buffer,
                        } => rdpdr.drive_set_information(device_id, file_id, set_buffer),
                        RdpdrServerMessage::QueryDirectory {
                            device_id,
                            file_id,
                            info_class,
                            path,
                            initial_query,
                        } => rdpdr.drive_query_directory(device_id, file_id, info_class, path, initial_query),
                        RdpdrServerMessage::NotifyChangeDirectory {
                            device_id,
                            file_id,
                            watch_tree,
                            completion_filter,
                        } => rdpdr.drive_notify_change_directory(device_id, file_id, watch_tree, completion_filter),
                        RdpdrServerMessage::QueryVolumeInformation {
                            device_id,
                            file_id,
                            fs_info_class,
                        } => rdpdr.drive_query_volume_information(device_id, file_id, fs_info_class),
                        RdpdrServerMessage::LockControl {
                            device_id,
                            file_id,
                            operation,
                            wait,
                            locks,
                        } => rdpdr.drive_lock_control(device_id, file_id, operation, wait, locks),
                        RdpdrServerMessage::QuerySecurity {
                            device_id,
                            file_id,
                            security_information,
                        } => rdpdr.drive_query_security(device_id, file_id, security_information),
                        RdpdrServerMessage::SetSecurity {
                            device_id,
                            file_id,
                            security_information,
                            security_descriptor,
                        } => rdpdr.drive_set_security(device_id, file_id, security_information, security_descriptor),
                        RdpdrServerMessage::DeviceControl {
                            device_id,
                            file_id,
                            io_control_code,
                            input_buffer,
                            output_buffer_length,
                        } => rdpdr.drive_device_control(
                            device_id,
                            file_id,
                            io_control_code,
                            input_buffer,
                            output_buffer_length,
                        ),
                    }
                    .map_err_kind("failed to send rdpdr event", ServerErrorKind::Pdu)?;
                    let channel_id = self
                        .get_channel_id_by_type::<RdpdrServer>()
                        .ok_or_else(|| ServerError::channel("SVC channel not found"))?;
                    let data =
                        server_encode_svc_messages(msgs, channel_id, user_channel_id).map_err(ServerError::encode)?;
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
                                    let handle = UsbDeviceHandle::new(self.ev_sender.clone(), dvc_id);
                                    if usb_man.router.insert(dvc_id, handle.device()).is_some() {
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
                        let Some(device) = self
                            .usb_man
                            .as_ref()
                            .and_then(|usb_man| usb_man.router.get(&dvc_id))
                            .map(Arc::clone)
                        else {
                            warn!(dvc_id, "Missing USB device state");
                            continue;
                        };

                        // Handle checks are an early rejection for callers. This event-loop check
                        // is authoritative because a request may already be queued when retract or
                        // channel close changes the shared lifecycle state.
                        if !device.is_open() {
                            trace!(dvc_id, "Dropping request for closing or closed USB device");
                            continue;
                        }

                        let Some(drdynvc) = self.get_svc_processor::<dvc::DrdynvcServer>() else {
                            warn!("No drdynvc channel, dropping URBDRC request");
                            continue;
                        };

                        let Some(mut dvc) = drdynvc.dvc_by_id_mut::<UrbdrcDeviceServer>(dvc_id) else {
                            warn!(dvc_id, "USB dynamic channel ID mismatch");
                            continue;
                        };
                        let processor = dvc.processor_mut();

                        let (dvc_msgs, close_dev) = match dev_msg {
                            UrbdrcDeviceServerMessage::QueryDeviceText { text_type, locale_id } => {
                                let text = processor
                                    .query_device_text(text_type, locale_id)
                                    .map_err_kind("query USB device text", ServerErrorKind::Pdu)?;
                                (vec![text], false)
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

                                let pending = request
                                    .expects_completion
                                    .then(|| device.register_pending(request.request_id));

                                // Reply before the write so the caller owns cancel-on-drop as early
                                // as possible. A CANCEL_REQUEST it enqueues in response lands in a
                                // later batch, so it cannot overtake this request on the wire.
                                if tx.send(pending).is_err() && request.expects_completion {
                                    trace!(dvc_id, "USB I/O request receiver dropped");
                                    device.forget_pending(request.request_id);
                                    processor.abandon_unsent(request);
                                    (Vec::new(), false)
                                } else {
                                    (vec![request.message], false)
                                }
                            }
                            UrbdrcDeviceServerMessage::Retract(reason) => {
                                let request = processor
                                    .retract_device(reason)
                                    .map_err_kind("retract USB device", ServerErrorKind::Pdu)?;
                                device.mark_retracting();
                                (vec![request], true)
                            }
                            UrbdrcDeviceServerMessage::CancelRequest(request_id) => {
                                if !device.is_pending(request_id) {
                                    trace!(dvc_id, request_id, "USB I/O request is no longer pending");
                                    continue;
                                }

                                let request = processor
                                    .cancel_request(request_id)
                                    .map_err_kind("cancel USB I/O request", ServerErrorKind::Pdu)?;

                                (vec![request], false)
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

    #[expect(
        clippy::too_many_arguments,
        reason = "private per-connection entry point; the parameters are the connection's negotiated identifiers"
    )]
    async fn client_loop<R, W>(
        &mut self,
        reader: &mut Framed<R>,
        writer: &mut Framed<W>,
        io_channel_id: u16,
        user_channel_id: u16,
        message_channel_id: Option<u16>,
        client_supports_heartbeat: bool,
        mut encoder: UpdateEncoder,
    ) -> ServerResult<RunState>
    where
        R: FramedRead,
        W: FramedWrite,
    {
        debug!("Starting client loop");
        let heartbeat = if client_supports_heartbeat {
            self.heartbeat
        } else {
            None
        };
        let mut display_updates = self.display.lock().await.updates().await?;
        let mut writer = SharedWriter::new(writer);
        let mut display_writer = writer.clone();
        let mut event_writer = writer.clone();
        let mut auto_reconnect_writer = writer.clone();
        let mut heartbeat_writer = writer.clone();
        let write_counter = writer.write_counter();
        let ev_receiver = Arc::clone(&self.ev_receiver);
        let s = Rc::new(Mutex::new(self));

        let this = Rc::clone(&s);
        let dispatch_pdu = async move {
            loop {
                let (action, bytes) = reader.read_pdu().await.map_err(|e| ServerError::io("read pdu", e))?;
                // D8: per-PDU lock-acquisition + dispatch timing. The `this`
                // mutex is shared with dispatch_events; when an outbound
                // event batch is in flight, this lock wait is the latency
                // that a FrameAcknowledge sees before it reaches its
                // handler. Log when the dispatch itself or the lock wait
                // exceeds 50ms.
                let pdu_len = bytes.len();
                let lock_start = Instant::now();
                let mut this = this.lock().await;
                let lock_wait_ms = u64::try_from(lock_start.elapsed().as_millis()).unwrap_or(u64::MAX);

                let dispatch_start = Instant::now();
                let result = this
                    .dispatch_pdu(
                        action,
                        bytes,
                        &mut writer,
                        io_channel_id,
                        user_channel_id,
                        message_channel_id,
                    )
                    .await?;
                let dispatch_ms = u64::try_from(dispatch_start.elapsed().as_millis()).unwrap_or(u64::MAX);

                if lock_wait_ms >= 50 {
                    tracing::warn!(
                        pdu_len,
                        lock_wait_ms,
                        dispatch_ms,
                        "dispatch_pdu delayed acquiring this.lock, contended with outbound batch (dispatch_events/dispatch_display)"
                    );
                } else if dispatch_ms >= 50 {
                    tracing::warn!(
                        pdu_len,
                        lock_wait_ms,
                        dispatch_ms,
                        "dispatch_pdu ran long after acquiring this.lock immediately, handler or runtime stall, not lock contention"
                    );
                } else {
                    tracing::debug!(pdu_len, lock_wait_ms, dispatch_ms, "dispatch_pdu");
                }

                match result {
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

                // D7: per-batch dispatch_events timing. The events Vec can
                // grow up to 100+ entries; dispatch_server_events holds the
                // `this` mutex AND the SharedWriter mutex for the full
                // batch. Log batch size + total dispatch time so operators
                // can see when an event batch ties up both locks.
                let batch_size = events.len();
                let lock_start = Instant::now();
                let mut this = this.lock().await;
                let lock_wait_ms = u64::try_from(lock_start.elapsed().as_millis()).unwrap_or(u64::MAX);

                let dispatch_start = Instant::now();
                let result = this
                    .dispatch_server_events(
                        &mut events,
                        &mut event_writer,
                        io_channel_id,
                        user_channel_id,
                        message_channel_id,
                    )
                    .await?;
                let dispatch_ms = u64::try_from(dispatch_start.elapsed().as_millis()).unwrap_or(u64::MAX);

                if lock_wait_ms >= 50 || dispatch_ms >= 100 {
                    tracing::warn!(
                        batch_size,
                        lock_wait_ms,
                        dispatch_ms,
                        "dispatch_events batch stalled, long write or lock contention"
                    );
                } else if batch_size > 1 {
                    tracing::debug!(batch_size, lock_wait_ms, dispatch_ms, "dispatch_events batch");
                }

                match result {
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

        let send_heartbeats = async move {
            let (Some(config), Some(message_channel_id)) = (heartbeat, message_channel_id) else {
                return core::future::pending::<ServerResult<RunState>>().await;
            };
            // 2.2.16.1: `period` is in seconds. A zero period is meaningless
            // (and would panic tokio's interval), so it is bumped to one.
            let period = Duration::from_secs(u64::from(config.period_secs.max(1)));
            let mut interval = tokio::time::interval(period);
            // A stalled write (TCP back-pressure) can hold this future past
            // several tick deadlines; Burst (the default) would then fire the
            // missed ticks back-to-back and emit a run of consecutive
            // heartbeats on an otherwise idle link. Skip fires at the next
            // period boundary instead.
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await; // first tick completes immediately; real waits start below

            let mut writes_at_last_tick = write_counter.load(Ordering::Relaxed);
            loop {
                interval.tick().await;
                let writes_now = write_counter.load(Ordering::Relaxed);
                if writes_now != writes_at_last_tick {
                    // 2.2.16.1: heartbeats SHOULD only be sent when no other
                    // PDU went out in the interval; ordinary traffic doubles
                    // as the liveness signal.
                    writes_at_last_tick = writes_now;
                    continue;
                }
                let data = encode_heartbeat(&config, message_channel_id, user_channel_id)?;
                heartbeat_writer
                    .write_all(&data)
                    .await
                    .map_err(|e| ServerError::io("send heartbeat", e))?;
                // Re-read so the heartbeat's own write does not read as
                // foreign traffic on the next tick.
                writes_at_last_tick = write_counter.load(Ordering::Relaxed);
            }
        };

        let state = tokio::select!(
            state = dispatch_pdu => state,
            state = dispatch_display => state,
            state = dispatch_events => state,
            state = refresh_auto_reconnect_cookie => state,
            state = send_heartbeats => state,
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
        let mut pointer_cache_size: u16 = 0;
        // Absence means the client did not send a Large Pointer Capability Set at all,
        // which per MS-RDPBCGR 2.2.7.2.7 leaves the pointer size ceiling at 32x32 (the
        // base Color/New Pointer Update limit with no large-pointer flags set).
        let mut large_pointer_flags = LargePointerSupportFlags::empty();
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
                CapabilitySet::Pointer(p) => {
                    // MS-RDPBCGR 2.2.7.1.5: pointerCacheSize is the client's advertised cache
                    // size for the New Pointer Update specifically (colorPointerCacheSize is
                    // the separate, always-supported Color Pointer Update cache). A zero or
                    // absent pointerCacheSize means the client did not advertise New Pointer
                    // Update support at all, so `UpdateEncoder` must not emit RGBAPointer, and
                    // must not reference a cache slot via CachedPointer either, since nothing
                    // else in this crate populates that cache via the Color Pointer Update.
                    pointer_cache_size = p.pointer_cache_size;
                }
                CapabilitySet::LargePointer(lp) => {
                    // MS-RDPBCGR 2.2.7.2.7: LARGE_POINTER_FLAG_96x96 raises the Color/New
                    // Pointer Update ceiling from 32x32 to 96x96; LARGE_POINTER_FLAG_384x384
                    // additionally unlocks the dedicated Fast-Path Large Pointer Update, up to
                    // 384x384. `UpdateEncoder` uses these flags to decide which pointer
                    // updates it can send at all, and at what size.
                    large_pointer_flags = lp.flags;
                }
                _ => {}
            }
        }

        let desktop_size = self.display.lock().await.size().await;
        let encoder = UpdateEncoder::new(
            desktop_size,
            surface_flags,
            update_codecs,
            self.opts.max_request_size,
            pointer_cache_size,
            large_pointer_flags,
        )?;

        self.send_next_auto_reconnect_cookie(writer, result.io_channel_id, result.user_channel_id)
            .await?;

        let state = self
            .client_loop(
                reader,
                writer,
                result.io_channel_id,
                result.user_channel_id,
                result.message_channel_id,
                result
                    .client_early_capability_flags
                    .contains(ironrdp_pdu::gcc::ClientEarlyCapabilityFlags::SUPPORT_HEART_BEAT_PDU),
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
                    match ad.handle_response(&pdu.response, monotonic_now_ms()) {
                        AutoDetectOutcome::Rtt(rtt_ms) => {
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
                        }
                        AutoDetectOutcome::Bandwidth(Some(bandwidth_kbps)) => {
                            self.autodetect_bandwidth.store(bandwidth_kbps, Ordering::Relaxed);
                            debug!(
                                bandwidth_kbps,
                                seq = pdu.response.sequence_number(),
                                "Bandwidth measured"
                            );
                        }
                        AutoDetectOutcome::Bandwidth(None) => {
                            // The manager just cleared its own figure rather than keep
                            // reporting a stale one (see `handle_response`'s doc comment);
                            // mirror that here so the exposed handle does not disagree.
                            self.autodetect_bandwidth.store(u32::MAX, Ordering::Relaxed);
                            trace!(
                                seq = pdu.response.sequence_number(),
                                "Bandwidth measurement completed without a usable figure"
                            );
                        }
                        AutoDetectOutcome::Unmatched => {
                            trace!(seq = pdu.response.sequence_number(), "Unmatched auto-detect response");
                        }
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
            // Bounded: see `FINALIZE_TIMEOUT`. The bound belongs on THIS call
            // and not on `accept_finalize` itself or its callers — the loop
            // below also runs `client_accepted`, which drives the entire live
            // session, so a timeout hoisted any higher would cap session
            // length. Applying it per pass also gives a
            // deactivation-reactivation its own budget rather than sharing one
            // with the initial handshake.
            let finalize = ironrdp_acceptor::accept_finalize(framed, &mut acceptor);
            let (new_framed, result) = match tokio::time::timeout(FINALIZE_TIMEOUT, finalize).await {
                Ok(res) => res.map_err_kind("failed to accept client during finalize", ServerErrorKind::Connector)?,
                Err(_) => {
                    warn!(
                        timeout = ?FINALIZE_TIMEOUT,
                        "Client stopped responding during the finalize handshake — dropping the connection"
                    );
                    return Err(ServerError::io(
                        "timed out waiting for the client during finalize",
                        std::io::Error::from(std::io::ErrorKind::TimedOut),
                    ));
                }
            };

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

/// Encode a server-initiated Heartbeat PDU for the MCS message channel.
///
/// Like auto-detect (see [`encode_autodetect_request`]), heartbeats are framed
/// by a Basic Security Header (SEC_HEARTBEAT) and ride the message channel,
/// not a Share Control / Share Data header on the I/O channel
/// (MS-RDPBCGR 2.2.16.1).
fn encode_heartbeat(config: &HeartbeatConfig, message_channel_id: u16, user_channel_id: u16) -> ServerResult<Vec<u8>> {
    let pdu = rdp::heartbeat::HeartbeatPdu {
        security_header: rdp::headers::BasicSecurityHeader {
            flags: rdp::headers::BasicSecurityHeaderFlags::HEARTBEAT,
        },
        period: config.period_secs,
        count1: config.warning_count,
        count2: config.reconnect_count,
    };
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
/// [`encode_autodetect_request`]), leaving this Save Session Info sender as the
/// sole user, so the encoder now lives with it.
fn encode_share_data_pdu(
    share_data_pdu: rdp::headers::ShareDataPdu,
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
        pdu_source: user_channel_id,
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
    /// Count of successful `write_all` calls across all clones. The heartbeat
    /// loop compares it across ticks to honor 2.2.16.1's idle-only SHOULD: a
    /// changed count means ordinary traffic already served as the liveness
    /// signal for that interval.
    writes: Arc<AtomicU64>,
}

impl<W: FramedWrite> Clone for SharedWriter<'_, W> {
    fn clone(&self) -> Self {
        Self {
            writer: Rc::clone(&self.writer),
            writes: Arc::clone(&self.writes),
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
        Box::pin(async move {
            // D1: time both the lock acquisition and the actual write.
            // Three concurrent tasks (dispatch_pdu, dispatch_display,
            // dispatch_events) share this Rc<Mutex<W>>. When the kernel TCP
            // send buffer fills (slow client), write_all blocks while still
            // holding the mutex — starving the other two tasks. Logging
            // both phases tells us whether a stall is "waiting in line for
            // the writer" (lock-wait) or "TLS write held up by TCP back-
            // pressure" (write-time).
            let len = buf.len();
            let wait_start = Instant::now();
            let mut writer = self.writer.lock().await;
            let wait_ms = u64::try_from(wait_start.elapsed().as_millis()).unwrap_or(u64::MAX);

            let write_start = Instant::now();
            let res = writer.write_all(buf).await;
            let write_ms = u64::try_from(write_start.elapsed().as_millis()).unwrap_or(u64::MAX);

            // Threshold: 50ms total budget for one write_all. Anything above
            // is operationally interesting on a healthy LAN. Logged at WARN
            // when stalled, DEBUG when fast (so wire-time samples are still
            // visible during routine debugging).
            if wait_ms + write_ms >= 50 {
                tracing::warn!(
                    bytes = len,
                    lock_wait_ms = wait_ms,
                    write_ms,
                    "SharedWriter.write_all stalled, possible TCP back-pressure or writer-mutex contention"
                );
            } else {
                tracing::debug!(bytes = len, lock_wait_ms = wait_ms, write_ms, "SharedWriter.write_all");
            }
            if res.is_ok() {
                self.writes.fetch_add(1, Ordering::Relaxed);
            }
            res
        })
    }
}

impl<'a, W: FramedWrite> SharedWriter<'a, W> {
    fn new(writer: &'a mut W) -> Self {
        Self {
            writer: Rc::new(Mutex::new(writer)),
            writes: Arc::new(AtomicU64::new(0)),
        }
    }

    fn write_counter(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.writes)
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
