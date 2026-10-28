use core::fmt;
use core::str::FromStr;
use core::time::Duration;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use ironrdp_cfg::PropertySetExt as _;
pub use ironrdp_cfg::RemoteGuardMode;
use ironrdp_connector::credssp::KdcResolution;
use ironrdp_propertyset::PropertySet;
use ironrdp_rail::pdu::ExecutePdu;
use url::Url;

/// Local drive selection, as configured with [`ConfigBuilder::with_drive_redirection`].
#[cfg(feature = "rdpdr")]
pub use ironrdp_cfg::DriveRedirection;

/// RDP-UDP protocol version, as offered with [`ConfigBuilder::with_udp_offer_version`].
#[cfg(feature = "udp")]
pub use ironrdp_rdpeudp::pdu::UdpVersion;
#[cfg(feature = "vmconnect")]
pub use ironrdp_vmconnect::Mode as VmConnectMode;
// ── Extension registry ────────────────────────────────────────────────────────

type StaticChannelFn = Arc<dyn Fn(&mut ironrdp_connector::ClientConnector, &PropertySet) + Send + Sync>;
type DvcChannelFn = Arc<dyn Fn(&mut ironrdp_dvc::DrdynvcClient, &PropertySet) + Send + Sync>;

/// Private registry of user-supplied static and dynamic virtual channel factories.
///
/// Cloneable via `Arc`; the factory closures are shared across reconnects.
#[derive(Default)]
pub(crate) struct ExtensionRegistry {
    pub(crate) static_channels: Vec<StaticChannelFn>,
    pub(crate) dvc_channels: Vec<DvcChannelFn>,
}

impl Clone for ExtensionRegistry {
    fn clone(&self) -> Self {
        Self {
            static_channels: self.static_channels.clone(),
            dvc_channels: self.dvc_channels.clone(),
        }
    }
}

impl fmt::Debug for ExtensionRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtensionRegistry")
            .field("static_channels", &self.static_channels.len())
            .field("dvc_channels", &self.dvc_channels.len())
            .finish()
    }
}

// ── Public configuration types ────────────────────────────────────────────────

/// RDPSND quality policy sent to servers that support audio protocol version 6 or newer.
#[cfg(any(feature = "sound", feature = "rdpdr"))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AudioQualityMode {
    Dynamic,
    Medium,
    #[default]
    High,
}

#[cfg(any(feature = "sound", feature = "rdpdr"))]
impl AudioQualityMode {
    pub(crate) fn into_rdpsnd(self) -> ironrdp_rdpsnd::pdu::QualityMode {
        match self {
            Self::Dynamic => ironrdp_rdpsnd::pdu::QualityMode::Dynamic,
            Self::Medium => ironrdp_rdpsnd::pdu::QualityMode::Medium,
            Self::High => ironrdp_rdpsnd::pdu::QualityMode::High,
        }
    }

    fn as_u32(self) -> u32 {
        match self {
            Self::Dynamic => 0,
            Self::Medium => 1,
            Self::High => 2,
        }
    }
}

/// Fully resolved client configuration.
///
/// This is the typed surface consumed by [`crate::rdp::RdpClient`]. Build it with
/// [`ConfigBuilder`]; producing a `Config` from CLI arguments, `.rdp` files, or interactive
/// prompts is the consumer's responsibility (see `ironrdp-viewer` for a reference front-end).
///
/// The struct is opaque: fields are read-only via accessors so a built `Config` cannot drift into
/// an inconsistent state (e.g. mutating the connector without updating the originating
/// [`PropertySet`]).
#[derive(Clone)]
pub struct Config {
    pub(crate) connector: ironrdp_connector::Config,
    pub(crate) destination: Destination,
    pub(crate) transport: Transport,
    #[cfg(feature = "udp")]
    pub(crate) udp_transport_enabled: bool,
    #[cfg(feature = "udp")]
    pub(crate) udp_offer_version: UdpVersion,
    pub(crate) certificate_validation: ironrdp_tls::CertificateValidation,
    pub(crate) certificate_validation_callback: Option<ironrdp_tls::CertificateValidationCallback>,
    #[cfg(feature = "entra-auth")]
    pub(crate) entra_auth: Option<Arc<crate::entra::EntraAuthSession>>,
    #[cfg(all(feature = "gateway", feature = "entra-auth"))]
    pub(crate) avd_token_provider: Option<Arc<dyn crate::avd::AvdTokenProvider>>,
    #[cfg(all(feature = "gateway", feature = "entra-auth"))]
    pub(crate) avd_gateway_token: Option<zeroize::Zeroizing<String>>,
    #[cfg(all(feature = "gateway", feature = "entra-auth"))]
    pub(crate) avd_sso_logon_token: Option<zeroize::Zeroizing<String>>,

    #[cfg(feature = "vmconnect")]
    /// Hyper-V VM ID when connecting to a VM console (port [`ironrdp_vmconnect::PORT`]).
    pub(crate) vm_id: Option<String>,
    #[cfg(feature = "vmconnect")]
    pub(crate) vmconnect_mode: VmConnectMode,
    #[cfg(all(windows, feature = "vmconnect"))]
    pub(crate) vmconnect_current_user: bool,
    pub(crate) kerberos_config: Option<ironrdp_connector::credssp::KerberosConfig>,
    pub(crate) remote_guard_mode: RemoteGuardMode,
    pub(crate) input_send_interval: Option<Duration>,
    pub(crate) input_keepalive_interval: Option<Duration>,
    pub(crate) fake_events_interval: Option<Duration>,
    pub(crate) tcp_connect_attempt_timeout: Option<Duration>,
    pub(crate) channels: ChannelConfig,
    pub(crate) administrative_session: bool,
    pub(crate) load_balance_info: Option<String>,
    /// Whether a server redirection connects to the target by name rather than by address.
    pub(crate) use_redirection_server_name: bool,
    /// What the last Server Redirection PDU changed for the next connection attempts.
    pub(crate) redirection: Option<crate::redirection::RedirectionHop>,
    #[cfg(any(feature = "sound", feature = "rdpdr"))]
    pub(crate) audio_quality_mode: AudioQualityMode,
    pub(crate) rail_client_status_flags: Option<u32>,
    pub(crate) rail_system_parameters: Option<Vec<ironrdp_rail::pdu::ClientSystemParameter>>,
    /// Initial RemoteApp launch queued after the RAIL handshake.
    pub(crate) rail_initial_execute: Option<ExecutePdu>,

    /// DVC channel ↔ named-pipe proxy configuration.
    ///
    /// Each entry causes IronRDP to forward that DVC channel's traffic to/from the
    /// named pipe, allowing out-of-process DVC logic.
    #[cfg(feature = "dvc-pipe-proxy")]
    pub(crate) dvc_pipe_proxies: Vec<DvcProxyInfo>,

    /// Paths to DVC client plugin DLLs to load (Windows only).
    ///
    /// Each DLL is loaded via `LoadLibraryW` and its `VirtualChannelGetInstance` export is
    /// called to obtain DVC plugin COM objects.  Example: `C:\Windows\System32\webauthn.dll`.
    #[cfg(all(windows, feature = "dvc-com-plugin"))]
    pub(crate) dvc_plugins: Vec<PathBuf>,

    /// Parent HWND for WebAuthn UI prompts (Windows only).
    #[cfg(all(windows, feature = "webauthn"))]
    pub(crate) webauthn_parent_hwnd: Option<isize>,

    /// The merged PropertySet that produced this config, shared (read-only) with channel factories.
    ///
    /// Well-known secret properties are stripped when calling [`ConfigBuilder::build`].
    pub(crate) properties: PropertySet,

    pub(crate) extensions: ExtensionRegistry,
}

impl Config {
    /// Connector configuration handed to the RDP connection sequence.
    pub fn connector(&self) -> &ironrdp_connector::Config {
        &self.connector
    }

    /// Resolved RDP target (host + port).
    pub fn destination(&self) -> &Destination {
        &self.destination
    }

    /// Selected transport (Direct, Gateway, or RDCleanPath).
    pub fn transport(&self) -> &Transport {
        &self.transport
    }

    /// Whether reliable RDP-UDP2 is enabled for eligible direct connections.
    #[cfg(feature = "udp")]
    pub fn udp_transport_enabled(&self) -> bool {
        self.udp_transport_enabled
    }

    /// Highest RDP-UDP protocol version offered in the SYN of the UDP handshake.
    #[cfg(feature = "udp")]
    pub fn udp_offer_version(&self) -> UdpVersion {
        self.udp_offer_version
    }

    /// TLS peer-certificate validation policy.
    pub fn certificate_validation(&self) -> ironrdp_tls::CertificateValidation {
        self.certificate_validation
    }

    /// Optional exception handler for an otherwise-invalid server certificate.
    pub fn certificate_validation_callback(&self) -> Option<&ironrdp_tls::CertificateValidationCallback> {
        self.certificate_validation_callback.as_ref()
    }

    #[cfg(feature = "vmconnect")]
    /// Hyper-V VM ID for a vmconnect session, if any.
    pub fn vm_id(&self) -> Option<&str> {
        self.vm_id.as_deref()
    }

    #[cfg(feature = "vmconnect")]
    /// Hyper-V console mode for a vmconnect session, if any.
    pub fn vmconnect_mode(&self) -> Option<VmConnectMode> {
        self.vm_id.as_ref().map(|_| self.vmconnect_mode)
    }

    #[cfg(all(windows, feature = "vmconnect"))]
    /// Whether VMConnect host authentication uses the caller's current Windows logon token.
    pub fn vmconnect_current_user(&self) -> bool {
        self.vm_id.is_some() && self.vmconnect_current_user
    }

    #[cfg(all(windows, feature = "vmconnect"))]
    /// Whether this local VMConnect session accepts Hyper-V frame-buffer redirection.
    pub fn vmconnect_framebuffer_redirection(&self) -> bool {
        self.vm_id.is_some()
            && matches!(&self.transport, Transport::Direct)
            && is_loopback_host(self.destination.name())
    }

    /// Optional Kerberos/KDC proxy configuration.
    pub fn kerberos_config(&self) -> Option<&ironrdp_connector::credssp::KerberosConfig> {
        self.kerberos_config.as_ref()
    }

    /// Required Remote Credential Guard policy.
    pub fn remote_guard_mode(&self) -> RemoteGuardMode {
        self.remote_guard_mode
    }

    #[cfg(feature = "__test")]
    #[doc(hidden)]
    pub fn __test_set_remote_guard_mode(&mut self, mode: RemoteGuardMode) {
        self.remote_guard_mode = mode;
    }

    /// Minimum interval between non-forced input batches.
    pub fn input_send_interval(&self) -> Option<Duration> {
        self.input_send_interval
    }

    /// Interval between synthetic input keepalive events.
    pub fn input_keepalive_interval(&self) -> Option<Duration> {
        self.input_keepalive_interval
    }

    /// Idle anti-lock fake-events interval, if enabled.
    pub fn fake_events_interval(&self) -> Option<Duration> {
        self.fake_events_interval
    }

    /// Maximum time a direct connection waits for one resolved server address to accept TCP.
    pub fn tcp_connect_attempt_timeout(&self) -> Option<Duration> {
        self.tcp_connect_attempt_timeout
    }

    /// Channel/codec runtime toggles.
    pub fn channels(&self) -> &ChannelConfig {
        &self.channels
    }

    /// Whether GCC requests the server's administrative session.
    pub fn administrative_session(&self) -> bool {
        self.administrative_session
    }

    /// Opaque load-balancing data sent in the initial X.224 Connection Request.
    pub fn load_balance_info(&self) -> Option<&str> {
        self.load_balance_info.as_deref()
    }

    /// Whether a server redirection connects to the target by name rather than by address
    /// (`use redirection server name`).
    pub fn use_redirection_server_name(&self) -> bool {
        self.use_redirection_server_name
    }

    /// RDPSND quality policy sent during audio format negotiation.
    #[cfg(any(feature = "sound", feature = "rdpdr"))]
    pub fn audio_quality_mode(&self) -> AudioQualityMode {
        self.audio_quality_mode
    }

    /// DVC named-pipe proxy mappings.
    #[cfg(feature = "dvc-pipe-proxy")]
    pub fn dvc_pipe_proxies(&self) -> &[DvcProxyInfo] {
        &self.dvc_pipe_proxies
    }

    /// DVC client plugin DLL paths (Windows only).
    #[cfg(all(windows, feature = "dvc-com-plugin"))]
    pub fn dvc_plugins(&self) -> &[PathBuf] {
        &self.dvc_plugins
    }

    /// Parent HWND for native WebAuthn redirection prompts (Windows only).
    #[cfg(all(windows, feature = "webauthn"))]
    pub fn webauthn_parent_hwnd(&self) -> Option<isize> {
        self.webauthn_parent_hwnd
    }

    /// Merged `.rdp` PropertySet that produced this config.
    pub fn properties(&self) -> &PropertySet {
        &self.properties
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("Config");
        s.field("connector", &self.connector);
        s.field("destination", &self.destination);
        s.field("transport", &self.transport);
        #[cfg(feature = "udp")]
        s.field("udp_transport_enabled", &self.udp_transport_enabled);
        #[cfg(feature = "udp")]
        s.field("udp_offer_version", &self.udp_offer_version);
        s.field("certificate_validation", &self.certificate_validation);
        s.field(
            "certificate_validation_callback",
            &self.certificate_validation_callback.as_ref().map(|_| "<configured>"),
        );
        #[cfg(feature = "vmconnect")]
        {
            s.field("vm_id", &self.vm_id);
            s.field("vmconnect_mode", &self.vmconnect_mode);
            #[cfg(windows)]
            s.field("vmconnect_current_user", &self.vmconnect_current_user);
            #[cfg(windows)]
            s.field(
                "vmconnect_framebuffer_redirection",
                &self.vmconnect_framebuffer_redirection(),
            );
        }
        s.field("kerberos_config", &self.kerberos_config);
        s.field("remote_guard_mode", &self.remote_guard_mode);
        s.field("input_send_interval", &self.input_send_interval);
        s.field("input_keepalive_interval", &self.input_keepalive_interval);
        s.field("fake_events_interval", &self.fake_events_interval);
        s.field("tcp_connect_attempt_timeout", &self.tcp_connect_attempt_timeout);
        s.field("channels", &self.channels);
        s.field("administrative_session", &self.administrative_session);
        s.field("load_balance_info", &self.load_balance_info);
        s.field("use_redirection_server_name", &self.use_redirection_server_name);
        s.field("redirection", &self.redirection);
        #[cfg(any(feature = "sound", feature = "rdpdr"))]
        s.field("audio_quality_mode", &self.audio_quality_mode);
        s.field("rail_client_status_flags", &self.rail_client_status_flags);
        s.field("rail_system_parameters", &self.rail_system_parameters);
        #[cfg(feature = "dvc-pipe-proxy")]
        s.field("dvc_pipe_proxies", &self.dvc_pipe_proxies);
        #[cfg(all(windows, feature = "dvc-com-plugin"))]
        s.field("dvc_plugins", &self.dvc_plugins);
        #[cfg(all(windows, feature = "webauthn"))]
        s.field("webauthn_parent_hwnd", &self.webauthn_parent_hwnd);
        s.field("extensions", &self.extensions);
        s.finish()
    }
}

/// Resolved clipboard backend selection.
///
/// Platform-specific details (e.g., which native clipboard backend to use) are handled
/// internally by the library when [`Enable`](ClipboardType::Enable) is selected.
#[cfg(feature = "clipboard")]
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClipboardType {
    /// Enable clipboard redirection (use the best available backend).
    Enable,
    /// Disable clipboard redirection entirely.
    Disable,
    /// Use a stub clipboard backend (for testing or headless usage).
    // FIXME: the `Stub` concept arguably shouldn't live in ironrdp-client. Investigate whether it
    // can move out via the extension/backend API, so the stub backend stays in ironrdp-viewer as a
    // debugging tool. Note that other consumers (e.g. ironrdp-agent) may need their own custom
    // backend that is not integrated with the host system's clipboard either; the design should
    // accommodate plugging in arbitrary CliprdrBackendFactory implementations rather than baking
    // specific variants into the client.
    Stub,
}

/// Channel and codec runtime toggles.
///
/// Each field is only present when the corresponding Cargo feature is enabled.
/// The defaults for all optional fields are `true` (enabled) when the feature is on.
// TODO: Also add flags for all the channels that are not behind Cargo feature flags.
// Examples: ECHO and Display Control virtual channels.
#[derive(Clone, Debug)]
pub struct ChannelConfig {
    /// Enable the RDPSND (audio) virtual channel.
    #[cfg(feature = "sound")]
    pub sound: bool,

    /// Enable the AUDIO_INPUT (microphone capture) dynamic virtual channel.
    #[cfg(feature = "sound")]
    pub audio_capture: bool,

    /// Audio devices and playback buffering.
    #[cfg(feature = "sound")]
    pub audio: crate::audio::AudioConfig,

    /// Clipboard redirection mode.
    #[cfg(feature = "clipboard")]
    pub clipboard: ClipboardType,

    /// Device-redirection (RDPDR) configuration.
    #[cfg(feature = "rdpdr")]
    pub rdpdr: RdpdrConfig,

    /// Enable QOI bitmap codec.
    ///
    /// When `false`, the QOI codec is removed from `connector.bitmap.codecs` before connecting
    /// even if the `qoi` feature is compiled in.
    #[cfg(feature = "qoi")]
    pub qoi: bool,

    /// Enable QOIZ (QOI with zlib) bitmap codec.
    #[cfg(feature = "qoiz")]
    pub qoiz: bool,

    /// Enable native MS-RDPEWA WebAuthn redirection.
    #[cfg(feature = "webauthn")]
    pub webauthn: bool,

    /// Enable the MS-RDPEL location dynamic virtual channel.
    #[cfg(feature = "location")]
    pub location: bool,

    /// Maximum number of simultaneous touch contacts advertised in the RDPEI `CS_READY` PDU.
    ///
    /// Defaults to [`RDPEI_MAX_TOUCH_CONTACTS`](crate::rdp::RDPEI_MAX_TOUCH_CONTACTS).
    pub max_touch_contacts: u16,

    /// MS-RDPECAM camera redirection (`camerastoredirect`). No camera is redirected by default.
    #[cfg(feature = "camera")]
    pub cameras: crate::camera::CameraConfig,

    /// MS-RDPEVOR video optimized remoting, enabled when an H.264 decoder is available unless
    /// `videoplaybackmode:i:0` turns it off.
    #[cfg(feature = "video")]
    pub video: crate::video::VideoConfig,

    /// Local USB devices to redirect over MS-RDPEUSB (URBDRC), or `None` to disable the channel.
    #[cfg(feature = "usb")]
    pub usb: Option<ironrdp_rdpeusb_native::UsbRedirectionConfig>,

    /// H.264 decoder for the graphics pipeline and video optimized remoting.
    ///
    /// Defaults to [`crate::h264::H264DecoderFactory::platform_default`]: Media Foundation's on
    /// Windows, or OpenH264 with the `openh264` feature.
    pub h264_decoder: Option<crate::h264::H264DecoderFactory>,
    /// File keeping the graphics pipeline's bitmap cache across sessions ([MS-RDPEGFX] 3.3.1.5),
    /// so a reconnect paints from cached tiles instead of waiting for the server to resend them.
    /// `None` keeps the cache in memory only.
    ///
    /// Defaults to `None`, unlike mstsc, which persists the cache unless told not to
    /// (`BitmapPersistenceEnabled` defaults to on there); [`crate::egfx_cache::default_cache_path`]
    /// gives the per-user location to opt in with.
    ///
    /// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
    pub egfx_persistent_cache: Option<PathBuf>,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            #[cfg(feature = "sound")]
            sound: true,
            #[cfg(feature = "sound")]
            audio_capture: false,
            #[cfg(feature = "sound")]
            audio: crate::audio::AudioConfig::default(),
            #[cfg(feature = "clipboard")]
            clipboard: ClipboardType::Enable,
            #[cfg(feature = "rdpdr")]
            rdpdr: RdpdrConfig::default(),
            #[cfg(feature = "qoi")]
            qoi: true,
            #[cfg(feature = "qoiz")]
            qoiz: true,
            #[cfg(feature = "webauthn")]
            webauthn: true,
            #[cfg(feature = "location")]
            location: false,
            max_touch_contacts: crate::rdp::RDPEI_MAX_TOUCH_CONTACTS,
            #[cfg(feature = "camera")]
            cameras: crate::camera::CameraConfig::default(),
            #[cfg(feature = "video")]
            video: crate::video::VideoConfig::default(),
            #[cfg(feature = "usb")]
            usb: None,
            h264_decoder: crate::h264::H264DecoderFactory::platform_default(),
            egfx_persistent_cache: None,
        }
    }
}

/// RDPDR (device redirection) runtime configuration.
#[cfg(feature = "rdpdr")]
#[derive(Clone, Debug)]
pub struct RdpdrConfig {
    /// Enable device redirection at all.
    pub enabled: bool,

    /// Enable smart-card redirection within RDPDR.
    #[cfg(feature = "smartcard")]
    pub smartcard: bool,

    /// Local drives requested for redirection.
    ///
    /// The client does not enumerate drives itself: frontends read this selection to configure
    /// their native RDPDR backend factory.
    pub drives: DriveRedirection,

    /// Announce the serial and parallel ports the RDPDR backend factory provides.
    ///
    /// Mirrors the `.rdp` `redirectcomports` property. Which local ports exist and are
    /// selected is up to the factory; this only gates their announcement.
    pub ports: bool,

    /// Allow printers from the RDPDR backend product to be announced.
    ///
    /// Mirrors the `.rdp` `redirectprinters` property. The backend factory still decides which
    /// printers exist; this only lets the connection drop them.
    pub printers: bool,
}

#[cfg(feature = "rdpdr")]
impl Default for RdpdrConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            #[cfg(feature = "smartcard")]
            smartcard: true,
            drives: DriveRedirection::default(),
            ports: true,
            printers: true,
        }
    }
}

/// Fully-resolved transport selection for an established RDP connection.
///
/// This is the form stored in [`Config`] and consumed by the connection code: every variant
/// carries all the data the transport needs, including any credentials. To *configure* a transport
/// on a [`ConfigBuilder`], use the granular [`TransportKind`] instead — it carries only the
/// addressing, leaving secrets to be supplied (or prompted for) separately.
#[derive(Clone, Debug, Default)]
pub enum Transport {
    /// Plain TCP → TLS direct connection to the RDP server.
    #[default]
    Direct,

    /// Connect via an RDS gateway (MS-TSGU / MSTSGU).
    ///
    /// The target RDP server host and port are taken from [`Config::destination`] and
    /// forwarded in the MS-TSGU channel-create packet. The gateway only needs its own
    /// endpoint and credentials.
    #[cfg(feature = "gateway")]
    Gateway(GatewayConfig),

    /// Connect via an RDS gateway using authenticated RPC over HTTP (MS-RPCH).
    ///
    /// This opt-in path does not fall back to unsigned RPC. IN and OUT channels are recycled
    /// before their finite lifetimes end.
    #[cfg(feature = "gateway")]
    GatewayRpch(GatewayConfig),

    /// Connect via an RDCleanPath proxy (WebSocket-based).
    RDCleanPath(RDCleanPathConfig),

    /// Windows named-pipe byte stream carrying TPKT/X.224 (e.g. Windows Sandbox `\\.\pipe\{VMId}`).
    ///
    /// Typically used with standard RDP security (`enable_tls`/`enable_credssp` both false).
    #[cfg(windows)]
    NamedPipe {
        /// Full pipe path (`\\.\pipe\…`) or bare pipe name.
        path: String,
    },
}

/// Transport selection used to configure a [`ConfigBuilder`].
///
/// Only the *addressing* of the transport is provided here (the gateway endpoint, the RDCleanPath
/// URL). The associated secrets — gateway username/password and the RDCleanPath authentication
/// token — are supplied through their own dedicated `with_*` methods
/// ([`with_gateway_username`](ConfigBuilder::with_gateway_username),
/// [`with_gateway_password`](ConfigBuilder::with_gateway_password),
/// [`with_rdcleanpath_token`](ConfigBuilder::with_rdcleanpath_token)).
///
/// Decoupling addressing from secrets means the latter can be tracked as [`MissingField`]s and
/// resolved independently (e.g. prompted interactively) instead of having to be known up-front when
/// the transport is selected. The builder assembles the resolved [`Transport`] from this selection
/// and the collected credentials in [`build`](ConfigBuilder::build).
#[derive(Clone, Debug, Default)]
pub enum TransportKind {
    /// Plain TCP → TLS direct connection to the RDP server.
    #[default]
    Direct,

    /// Connect via an RDS gateway (MS-TSGU / MSTSGU).
    ///
    /// Gateway credentials are supplied separately via
    /// [`with_gateway_username`](ConfigBuilder::with_gateway_username) /
    /// [`with_gateway_password`](ConfigBuilder::with_gateway_password).
    #[cfg(feature = "gateway")]
    Gateway {
        /// Gateway endpoint address (e.g., `"rdg.contoso.com:443"`).
        endpoint: String,
        /// When `true` (`GatewayUsageMethod::Detect`), try a direct TCP connection first and fall
        /// back to the gateway only if that fails.
        prefer_direct: bool,
    },

    /// Connect through an RDS gateway using authenticated RPC over HTTP (MS-RPCH).
    ///
    /// This path is selected only explicitly and does not fall back to unsigned RPC. IN and OUT
    /// channels are recycled before their finite lifetimes end.
    #[cfg(feature = "gateway")]
    GatewayRpch {
        /// Gateway endpoint address (e.g., `"rdg.contoso.com:443"`).
        endpoint: String,
        /// When `true` (`GatewayUsageMethod::Detect`), try a direct TCP connection first and fall
        /// back to RPCH only if that fails.
        prefer_direct: bool,
    },

    /// Connect via an RDCleanPath proxy (WebSocket-based).
    ///
    /// The authentication token is supplied separately via
    /// [`with_rdcleanpath_token`](ConfigBuilder::with_rdcleanpath_token).
    RDCleanPath {
        /// RDCleanPath proxy URL.
        url: Url,
    },

    /// Windows named-pipe RDP stream (e.g. Windows Sandbox).
    #[cfg(windows)]
    NamedPipe {
        /// Full pipe path (`\\.\pipe\…`) or bare pipe name.
        path: String,
    },
}

/// Endpoint and credentials for a fully-resolved RDS gateway connection.
#[cfg(feature = "gateway")]
#[derive(Clone, Debug)]
pub struct GatewayConfig {
    /// Gateway endpoint address (e.g., `"rdg.contoso.com:443"`).
    pub endpoint: String,
    /// Gateway username.
    pub username: String,
    /// Gateway password.
    pub password: String,
    /// When `true` (`GatewayUsageMethod::Detect`), try direct TCP first and fall back to the
    /// gateway on connection failure.
    pub prefer_direct: bool,
}

// ── Destination ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Destination {
    name: String,
    port: Option<u16>,
}

impl Destination {
    pub fn new(addr: impl Into<String>) -> anyhow::Result<Self> {
        let addr = addr.into();

        if let Some(addr_split) = addr.rsplit_once(':') {
            if let Ok(sock_addr) = addr.parse::<core::net::SocketAddr>() {
                Ok(Self {
                    name: sock_addr.ip().to_string(),
                    port: Some(sock_addr.port()),
                })
            } else if addr.parse::<core::net::Ipv6Addr>().is_ok() {
                Ok(Self { name: addr, port: None })
            } else {
                Ok(Self {
                    name: addr_split.0.to_owned(),
                    port: Some(addr_split.1.parse().context("invalid port")?),
                })
            }
        } else {
            Ok(Self { name: addr, port: None })
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn port(&self) -> u16 {
        self.port.unwrap_or(RDP_DEFAULT_PORT)
    }

    /// Construct a `Destination` from already-validated components.
    ///
    /// Intended for front-ends that have already resolved the host and port from their own
    /// configuration sources (CLI flags, `.rdp` files, IPC schemas).
    pub fn from_parts(name: impl Into<String>, port: u16) -> Self {
        Self {
            name: name.into(),
            port: Some(port),
        }
    }
}

impl PartialEq for Destination {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.port() == other.port()
    }
}

impl Eq for Destination {}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // IPv6 addresses must be bracketed in host:port notation.
        if self.name.parse::<core::net::Ipv6Addr>().is_ok() {
            write!(f, "[{}]:{}", self.name, self.port())
        } else {
            write!(f, "{}:{}", self.name, self.port())
        }
    }
}

impl FromStr for Destination {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl From<Destination> for ironrdp_connector::ServerName {
    fn from(value: Destination) -> Self {
        Self::new(value.name)
    }
}

impl From<&Destination> for ironrdp_connector::ServerName {
    fn from(value: &Destination) -> Self {
        Self::new(&value.name)
    }
}

// ── RDCleanPath & DVC proxy ───────────────────────────────────────────────────

/// URL and authentication token for a fully-resolved RDCleanPath connection.
#[derive(Clone, Debug)]
pub struct RDCleanPathConfig {
    /// RDCleanPath proxy URL.
    pub url: Url,
    /// RDCleanPath authentication token (secret).
    pub auth_token: String,
}

/// Name-to-pipe mapping for a single DVC proxy channel.
#[derive(Clone, Debug)]
pub struct DvcProxyInfo {
    pub channel_name: String,
    pub pipe_name: String,
}

impl FromStr for DvcProxyInfo {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (channel_name, pipe_name) = s
            .split_once('=')
            .context("missing '=' delimiter in DVC proxy specification")?;
        Ok(Self {
            channel_name: channel_name.to_owned(),
            pipe_name: pipe_name.to_owned(),
        })
    }
}

// ── ConfigBuilder ─────────────────────────────────────────────────────────────

const RDP_DEFAULT_PORT: u16 = 3389;
const DEFAULT_WIDTH: u16 = 1280;
const DEFAULT_HEIGHT: u16 = 720;

/// A configuration value that the consumer must supply before [`ConfigBuilder::build`] can succeed.
///
/// Query the outstanding ones with [`ConfigBuilder::missing`], resolve each (prompt the user, or
/// derive a value), set it via the matching `with_*` method, then build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingField {
    /// Target server address (host[:port]).
    ServerAddress,
    /// RDP account user name.
    Username,
    /// RDP account password.
    Password,
    /// Gateway user name (only when a gateway transport is selected).
    GatewayUsername,
    /// Gateway password (only when a gateway transport is selected).
    GatewayPassword,
    /// RDCleanPath authentication token (only when an RDCleanPath transport is selected).
    RDCleanPathToken,
    /// Client build number (frontend-derived).
    ClientBuild,
    /// Client directory path (frontend-derived).
    ClientDir,
    /// Client platform (frontend-derived).
    Platform,
    /// Client computer name (frontend-derived).
    ClientName,
}

impl fmt::Display for MissingField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::ServerAddress => "server address",
            Self::Username => "username",
            Self::Password => "password",
            Self::GatewayUsername => "gateway username",
            Self::GatewayPassword => "gateway password",
            Self::RDCleanPathToken => "RDCleanPath token",
            Self::ClientBuild => "client build",
            Self::ClientDir => "client dir",
            Self::Platform => "platform",
            Self::ClientName => "client name",
        };
        f.write_str(s)
    }
}

/// Builder for [`Config`].
///
/// No defaults are created up-front for required values; they are tracked as unset until provided.
/// Truly optional settings receive sensible defaults inside [`build`](ConfigBuilder::build). Use
/// [`missing`](ConfigBuilder::missing) to discover which required fields still need a value.
///
/// # Duplicate-channel behaviour
///
/// * **Static channels** are keyed by the concrete processor `TypeId`; registering two factories
///   with the same concrete type silently shadows the earlier one via
///   [`ironrdp_connector::ClientConnector::attach_static_channel`].
/// * **DVC channels** are keyed by channel name; duplicate names follow
///   [`ironrdp_dvc::DrdynvcClient`]'s overwrite semantics.
///
/// # Custom-channel configuration keys
///
/// Factory closures registered with [`with_static_channel`](Self::with_static_channel) and
/// [`with_dvc`](Self::with_dvc) receive the merged [`PropertySet`], so a custom channel can read
/// its own settings (enabled/disabled, endpoints, flags) straight from the `.rdp` file. Which keys
/// to read is entirely up to the channel: there is no enforced naming scheme. By convention,
/// IronRDP's own extension keys use an `ironrdp_` prefix to avoid colliding with standard Microsoft
/// keys, and custom channels are encouraged (but not required) to namespace their keys similarly
/// (e.g. `mycorp_mychannel_enabled`). A channel may equally reuse a standard MS key when that fits,
/// or adopt a completely different pattern if warranted — these are only conventions.
#[derive(Default)]
pub struct ConfigBuilder {
    // Required (no default).
    destination: Option<Destination>,
    username: Option<String>,
    password: Option<String>,
    client_build: Option<u32>,
    client_dir: Option<String>,
    client_name: Option<String>,
    platform: Option<ironrdp_pdu::rdp::capability_sets::MajorPlatformType>,
    gateway_username: Option<String>,
    gateway_password: Option<String>,

    // Optional (defaulted at build time).
    domain: Option<String>,
    enable_tls: Option<bool>,
    enable_credssp: Option<bool>,
    certificate_validation: Option<ironrdp_tls::CertificateValidation>,
    certificate_validation_callback: Option<ironrdp_tls::CertificateValidationCallback>,
    keyboard_type: Option<ironrdp_pdu::gcc::KeyboardType>,
    keyboard_subtype: Option<u32>,
    keyboard_functional_keys_count: Option<u32>,
    keyboard_layout: Option<u32>,
    connection_type: Option<ironrdp_pdu::gcc::ConnectionType>,
    rdp_version: Option<ironrdp_pdu::gcc::RdpVersion>,
    ime_file_name: Option<String>,
    dig_product_id: Option<String>,
    desktop_width: Option<u16>,
    desktop_height: Option<u16>,
    monitor_layout: Option<ironrdp_pdu::gcc::ClientMonitorData>,
    monitor_extended_layout: Option<ironrdp_pdu::gcc::ClientMonitorExtendedData>,
    relative_mouse_input: bool,
    desktop_scale_factor: Option<u32>,
    device_scale_factor: Option<u32>,
    desktop_physical_size: Option<(u32, u32)>,
    desktop_orientation: Option<ironrdp_pdu::gcc::MonitorOrientation>,
    color_depth: Option<u32>,
    lossy_compression: Option<bool>,
    codecs: Vec<String>,
    autologon: Option<bool>,
    enable_server_pointer: Option<bool>,
    pointer_software_rendering: Option<bool>,
    performance_flags: Option<ironrdp_pdu::rdp::client_info::PerformanceFlags>,
    enable_audio_playback: Option<bool>,
    enable_audio_capture: Option<bool>,
    compression_type: Option<ironrdp_pdu::rdp::client_info::CompressionType>,
    compression_enabled: Option<bool>,
    load_balance_info: Option<String>,
    use_redirection_server_name: bool,
    administrative_session: bool,
    alternate_shell: Option<String>,
    work_dir: Option<String>,
    remote_application_mode: Option<bool>,
    remote_application_program: Option<String>,
    rail_support_level: Option<ironrdp_pdu::rdp::capability_sets::RailSupportLevel>,
    rail_client_status_flags: Option<u32>,
    rail_system_parameters: Option<Vec<ironrdp_rail::pdu::ClientSystemParameter>>,

    transport: TransportKind,
    #[cfg(feature = "udp")]
    udp_transport_enabled: bool,
    #[cfg(feature = "udp")]
    udp_offer_version: Option<UdpVersion>,
    graphics_pipeline: Option<bool>,
    #[cfg(feature = "vmconnect")]
    vm_id: Option<String>,
    #[cfg(feature = "vmconnect")]
    vmconnect_mode: VmConnectMode,
    #[cfg(all(windows, feature = "vmconnect"))]
    vmconnect_current_user: bool,
    rdcleanpath_token: Option<String>,
    #[cfg(feature = "entra-auth")]
    entra_auth: Option<Arc<crate::entra::EntraAuthSession>>,
    #[cfg(all(feature = "gateway", feature = "entra-auth"))]
    avd_token_provider: Option<Arc<dyn crate::avd::AvdTokenProvider>>,
    #[cfg(all(feature = "gateway", feature = "entra-auth"))]
    avd_gateway_token: Option<zeroize::Zeroizing<String>>,
    #[cfg(all(feature = "gateway", feature = "entra-auth"))]
    avd_sso_logon_token: Option<zeroize::Zeroizing<String>>,
    kerberos_config: Option<ironrdp_connector::credssp::KerberosConfig>,
    remote_guard_mode: RemoteGuardMode,
    input_send_interval: Option<Duration>,
    input_keepalive_interval: Option<Duration>,
    fake_events_interval: Option<Duration>,
    tcp_connect_attempt_timeout: Option<Duration>,
    channels: ChannelConfig,
    #[cfg(any(feature = "sound", feature = "rdpdr"))]
    audio_quality_mode: Option<AudioQualityMode>,
    #[cfg(feature = "dvc-pipe-proxy")]
    dvc_pipe_proxies: Vec<DvcProxyInfo>,
    #[cfg(all(windows, feature = "dvc-com-plugin"))]
    dvc_plugins: Vec<PathBuf>,
    #[cfg(all(windows, feature = "webauthn"))]
    webauthn_parent_hwnd: Option<isize>,
    properties: PropertySet,
    extensions: ExtensionRegistry,
}

impl ConfigBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_destination(mut self, destination: Destination) -> Self {
        // Classify the host so the persisted `full address` follows TargetAddr's formatting rules
        // (notably, IPv6 addresses must be bracketed). A bare `TargetHost::Domain` would drop the
        // brackets and desynchronize the PropertySet from its own canonical formatting.
        let host = match destination.name.parse::<core::net::IpAddr>() {
            Ok(ip) => ironrdp_cfg::TargetHost::Ip(ip),
            Err(_) => ironrdp_cfg::TargetHost::Domain(destination.name.clone()),
        };
        self.properties
            .set_full_address(&ironrdp_cfg::TargetAddr { host, port: None });
        self.properties.set_server_port(destination.port());
        self.properties.clear_alternate_full_address();
        self.destination = Some(destination);
        self
    }

    #[must_use]
    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        let username = username.into();
        self.username = Some(username.clone());
        self.properties.set_username(username);
        self
    }

    /// Set the domain used by the RDP account credentials. Upserts the `domain` property.
    #[must_use]
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        let domain = domain.into();
        self.properties.set_domain(domain.clone());
        self.domain = Some(domain);
        self
    }

    #[must_use]
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    #[must_use]
    pub fn with_gateway_username(mut self, username: impl Into<String>) -> Self {
        let username = username.into();
        self.gateway_username = Some(username.clone());
        self.properties.set_gateway_username(username);
        self
    }

    #[must_use]
    pub fn with_gateway_password(mut self, password: impl Into<String>) -> Self {
        self.gateway_password = Some(password.into());
        self
    }

    #[must_use]
    pub fn with_client_build(mut self, build: u32) -> Self {
        self.client_build = Some(build);
        self
    }

    #[must_use]
    pub fn with_client_dir(mut self, dir: impl Into<String>) -> Self {
        self.client_dir = Some(dir.into());
        self
    }

    #[must_use]
    pub fn with_client_name(mut self, name: impl Into<String>) -> Self {
        self.client_name = Some(name.into());
        self
    }

    #[must_use]
    pub fn with_platform(mut self, platform: ironrdp_pdu::rdp::capability_sets::MajorPlatformType) -> Self {
        self.platform = Some(platform);
        self
    }

    #[must_use]
    pub fn with_keyboard_type(mut self, ty: ironrdp_pdu::gcc::KeyboardType) -> Self {
        self.keyboard_type = Some(ty);
        self
    }

    #[must_use]
    pub fn with_keyboard_subtype(mut self, subtype: u32) -> Self {
        self.keyboard_subtype = Some(subtype);
        self
    }

    #[must_use]
    pub fn with_keyboard_functional_keys_count(mut self, count: u32) -> Self {
        self.keyboard_functional_keys_count = Some(count);
        self
    }

    /// Set the keyboard layout (HKL) advertised in the GCC Client Core Data block.
    #[must_use]
    pub fn with_keyboard_layout(mut self, keyboard_layout: u32) -> Self {
        self.keyboard_layout = Some(keyboard_layout);
        self
    }

    /// Set the network profile advertised in the GCC Client Core Data block.
    #[must_use]
    pub fn with_connection_type(mut self, connection_type: ironrdp_pdu::gcc::ConnectionType) -> Self {
        self.connection_type = Some(connection_type);
        self
    }

    /// Set the RDP version announced in the GCC Client Core Data block.
    ///
    /// The default is [`RdpVersion::V5_PLUS`](ironrdp_pdu::gcc::RdpVersion::V5_PLUS), and it should not be lower.
    /// See [`ironrdp_connector::Config::rdp_version`].
    #[must_use]
    pub fn with_rdp_version(mut self, rdp_version: ironrdp_pdu::gcc::RdpVersion) -> Self {
        self.rdp_version = Some(rdp_version);
        self
    }

    #[must_use]
    pub fn with_ime_file_name(mut self, name: impl Into<String>) -> Self {
        self.ime_file_name = Some(name.into());
        self
    }

    #[must_use]
    pub fn with_dig_product_id(mut self, id: impl Into<String>) -> Self {
        self.dig_product_id = Some(id.into());
        self
    }

    #[must_use]
    pub fn with_color_depth(mut self, depth: u32) -> Self {
        self.color_depth = Some(depth);
        self.properties.set_color_depth(depth);
        self
    }

    /// Permit dynamic color fidelity and chroma subsampling in RDP 6.0 bitmap updates.
    #[must_use]
    pub fn with_lossy_compression(mut self, enabled: bool) -> Self {
        self.lossy_compression = Some(enabled);
        self
    }

    /// Set the desktop width (in pixels) to request. Upserts the `desktopwidth` property.
    ///
    /// Together with [`with_desktop_height`](Self::with_desktop_height) this becomes the initial
    /// [`DesktopSize`](ironrdp_connector::DesktopSize) advertised to the server.
    #[must_use]
    pub fn with_desktop_width(mut self, width: u16) -> Self {
        self.desktop_width = Some(width);
        self.properties.set_desktop_width(width);
        self
    }

    /// Set the desktop height (in pixels) to request. Upserts the `desktopheight` property.
    ///
    /// Together with [`with_desktop_width`](Self::with_desktop_width) this becomes the initial
    /// [`DesktopSize`](ironrdp_connector::DesktopSize) advertised to the server.
    #[must_use]
    pub fn with_desktop_height(mut self, height: u16) -> Self {
        self.desktop_height = Some(height);
        self.properties.set_desktop_height(height);
        self
    }

    /// Advertise the client monitor topology in the GCC Client Monitor Data block.
    ///
    /// The caller must set [`Self::with_desktop_width`] and [`Self::with_desktop_height`] to
    /// the virtual desktop dimensions containing every monitor.
    #[must_use]
    pub fn with_monitor_layout(mut self, monitor_layout: ironrdp_pdu::gcc::ClientMonitorData) -> Self {
        self.monitor_layout = Some(monitor_layout);
        self
    }

    /// Set the desktop scale factor (percentage, typically 100–500) to request. Upserts the
    /// `desktopscalefactor` property.
    ///
    /// This becomes the `desktop_scale_factor` in the `TS_UD_CS_CORE` GCC structure.
    #[must_use]
    pub fn with_desktop_scale_factor(mut self, scale: u32) -> Self {
        self.desktop_scale_factor = Some(scale);
        self.properties.set_desktop_scale_factor(scale);
        self
    }

    /// Advertise per-monitor physical size, orientation and scale in the GCC Client Monitor
    /// Extended Data block.
    ///
    /// The block is sent only with a [monitor layout](Self::with_monitor_layout), and must list
    /// the same monitors in the same order.
    #[must_use]
    pub fn with_monitor_extended_layout(
        mut self,
        monitor_extended_layout: ironrdp_pdu::gcc::ClientMonitorExtendedData,
    ) -> Self {
        self.monitor_extended_layout = Some(monitor_extended_layout);
        self
    }

    /// Set the device scale factor (100, 140 or 180) to request. Upserts the `devicescalefactor`
    /// property.
    ///
    /// This becomes the `deviceScaleFactor` in the `TS_UD_CS_CORE` GCC structure, which servers
    /// honor only alongside a valid [desktop scale factor](Self::with_desktop_scale_factor).
    /// [`build`](Self::build) rejects any other value.
    #[must_use]
    pub fn with_device_scale_factor(mut self, scale: u32) -> Self {
        self.device_scale_factor = Some(scale);
        self.properties.insert("devicescalefactor", scale);
        self
    }

    /// Set the physical size of the desktop in millimeters.
    ///
    /// This becomes `desktopPhysicalWidth` and `desktopPhysicalHeight` in the `TS_UD_CS_CORE` GCC
    /// structure. Servers ignore sizes outside 10 to 10,000 millimeters.
    #[must_use]
    pub fn with_desktop_physical_size(mut self, width: u32, height: u32) -> Self {
        self.desktop_physical_size = Some((width, height));
        self
    }

    /// Set the desktop orientation, instead of deriving landscape or portrait from the desktop size.
    ///
    /// This becomes `desktopOrientation` in the `TS_UD_CS_CORE` GCC structure.
    #[must_use]
    pub fn with_desktop_orientation(mut self, orientation: ironrdp_pdu::gcc::MonitorOrientation) -> Self {
        self.desktop_orientation = Some(orientation);
        self
    }

    /// Enable or disable TLS + Network Level Authentication (NLA) using CredSSP. Upserts the
    /// `enablecredsspsupport` property.
    ///
    /// NLA allows authentication to be performed before session establishment, considerably
    /// reducing the attack surface compared to the legacy TLS security protocol. When connecting to
    /// NLA-capable servers it is recommended to also disable plain TLS via
    /// [`with_tls(false)`](Self::with_tls).
    #[doc(alias("with_nla", "with_enable_credssp"))]
    #[must_use]
    pub fn with_credssp(mut self, enabled: bool) -> Self {
        self.enable_credssp = Some(enabled);
        self.properties.set_enable_credssp_support(enabled);
        self
    }

    /// Set the bitmap codecs (e.g. `["remotefx:on"]`). Not reflected in the PropertySet.
    #[must_use]
    pub fn with_codecs(mut self, codecs: Vec<String>) -> Self {
        self.codecs = codecs;
        self
    }

    #[must_use]
    pub fn with_autologon(mut self, enabled: bool) -> Self {
        self.autologon = Some(enabled);
        self.properties.set_autologon(enabled);
        self
    }

    /// Set or clear the opaque load-balance routing token carried by the X.224 Connection Request PDU.
    ///
    /// A nonempty token must contain 1–238 printable ASCII bytes, excluding an optional trailing CRLF.
    /// An empty value clears the token, and [`Self::build`] rejects other values.
    #[must_use]
    pub fn with_load_balance_info(mut self, value: impl Into<String>) -> Self {
        let value = value.into();
        let normalized = value.strip_suffix("\r\n").unwrap_or(&value);
        if normalized.is_empty() {
            self.load_balance_info = None;
            self.properties.remove("loadbalanceinfo");
        } else {
            self.properties.insert("loadbalanceinfo", value.clone());
            self.load_balance_info = Some(normalized.to_owned());
        }
        self
    }

    /// Connect to the target of a server redirection by name rather than by address.
    ///
    /// Upserts the `use redirection server name` property, which mirrors the mstsc ActiveX
    /// `UseRedirectionServerName` property. By default a direct connection follows a redirection to the target's IP
    /// address first; connections through a gateway or an RDCleanPath proxy always prefer the name.
    #[must_use]
    pub fn with_use_redirection_server_name(mut self, enabled: bool) -> Self {
        self.use_redirection_server_name = enabled;
        self.properties.insert("use redirection server name", enabled);
        self
    }

    /// Request the server's administrative session through GCC Client Cluster Data.
    #[must_use]
    pub fn with_administrative_session(mut self, enabled: bool) -> Self {
        self.administrative_session = enabled;
        self.properties.insert("administrative session", enabled);
        self
    }

    /// Set the alternate shell to start after logon. Upserts the `alternate shell` property.
    ///
    /// An empty value clears the alternate shell and requests the server's normal user shell.
    #[must_use]
    pub fn with_alternate_shell(mut self, shell: impl Into<String>) -> Self {
        let shell = shell.into();
        if shell.is_empty() {
            self.alternate_shell = None;
            self.properties.clear_alternate_shell();
        } else {
            self.properties.set_alternate_shell(shell.clone());
            self.alternate_shell = Some(shell);
        }
        self
    }

    /// Set the working directory for the alternate shell. Upserts the `shell working directory` property.
    ///
    /// An empty value clears the working directory.
    #[must_use]
    pub fn with_work_dir(mut self, work_dir: impl Into<String>) -> Self {
        let work_dir = work_dir.into();
        if work_dir.is_empty() {
            self.work_dir = None;
            self.properties.clear_shell_working_directory();
        } else {
            self.properties.set_shell_working_directory(work_dir.clone());
            self.work_dir = Some(work_dir);
        }
        self
    }

    /// Enable or disable RemoteApp/RAIL connection mode.
    ///
    /// RemoteApp launch information is sent on the RAIL static virtual channel
    /// rather than through the Client Info Alternate Shell field.
    #[must_use]
    pub fn with_remote_application_mode(mut self, enabled: bool) -> Self {
        self.properties.set_remote_application_mode(enabled);
        self.remote_application_mode = Some(enabled);
        self
    }

    /// Declares the RAIL capabilities implemented by this client.
    ///
    /// RemoteApp mode requires
    /// [`RailSupportLevel::SUPPORTED`](ironrdp_pdu::rdp::capability_sets::RailSupportLevel::SUPPORTED).
    #[must_use]
    pub fn with_rail_support_level(
        mut self,
        support_level: ironrdp_pdu::rdp::capability_sets::RailSupportLevel,
    ) -> Self {
        self.rail_support_level = Some(support_level);
        self
    }

    /// Overrides the RAIL Client Status flags advertised during channel initialization.
    #[must_use]
    pub fn with_rail_client_status_flags(mut self, flags: u32) -> Self {
        self.rail_client_status_flags = Some(flags);
        self
    }

    /// Overrides the RAIL client system parameters sent during channel initialization,
    /// typically with the local desktop's values.
    #[must_use]
    pub fn with_rail_system_parameters(mut self, parameters: Vec<ironrdp_rail::pdu::ClientSystemParameter>) -> Self {
        self.rail_system_parameters = Some(parameters);
        self
    }

    /// Enable or disable TLS + Graphical login (legacy security protocol; also called SSL). Upserts
    /// the `ironrdp_tls` property.
    ///
    /// When this security protocol is negotiated, the RDP server shows a graphical login screen and
    /// the full connection sequence is performed with all static channels joined and active. This
    /// exposes a wide attack surface (MITM, server-side and client-side takeover, file stealing) and
    /// is being phased out. Set this to `false` to effectively enforce usage of NLA/CredSSP on the
    /// client side (see [`with_credssp`](Self::with_credssp)).
    #[doc(alias("with_enable_tls"))]
    #[must_use]
    pub fn with_tls(mut self, enabled: bool) -> Self {
        self.enable_tls = Some(enabled);
        self.properties.set_enable_tls(enabled);
        self
    }

    /// Require Microsoft Entra RDS AAD session authentication.
    ///
    /// Create the session with [`crate::entra::EntraAuthorization`] and pass it here.
    /// This requires strictly validated TLS and never falls back to CredSSP or password authentication.
    #[cfg(feature = "entra-auth")]
    #[must_use]
    pub fn with_entra_auth(mut self, auth: Arc<crate::entra::EntraAuthSession>) -> Self {
        self.entra_auth = Some(auth);
        self
    }

    /// Supply a custom cross-platform token provider for Azure Virtual Desktop (`gatewaybrokeringtype:i:1`)
    /// orchestration and session-host PoP authentication.
    ///
    /// On Windows, when omitted, IronRDP defaults to Windows Web Account Manager (`WebAuthenticationCoreManager`).
    #[cfg(all(feature = "gateway", feature = "entra-auth"))]
    #[must_use]
    pub fn with_avd_token_provider(mut self, provider: Arc<dyn crate::avd::AvdTokenProvider>) -> Self {
        self.avd_token_provider = Some(provider);
        self
    }

    /// Set the TLS peer-certificate validation policy.
    ///
    /// The default preserves historic behavior by accepting invalid certificates. Use
    /// [`ironrdp_tls::CertificateValidation::Strict`] to validate the certificate
    /// chain and server name against the platform trust store.
    #[must_use]
    pub fn with_certificate_validation(mut self, validation: ironrdp_tls::CertificateValidation) -> Self {
        self.certificate_validation = Some(validation);
        self
    }

    /// Set an explicit handler for otherwise-invalid server certificates.
    ///
    /// This requires the Rustls backend. The callback is invoked only after normal
    /// platform-root and server-name validation fails.
    #[must_use]
    pub fn with_certificate_validation_callback(
        mut self,
        callback: ironrdp_tls::CertificateValidationCallback,
    ) -> Self {
        self.certificate_validation_callback = Some(callback);
        self
    }

    #[must_use]
    pub fn with_server_pointer(mut self, enabled: bool) -> Self {
        self.enable_server_pointer = Some(enabled);
        self.properties.set_server_pointer(enabled);
        self
    }

    /// Enable or disable software pointer rendering. When enabled, the session composites the
    /// remote cursor directly into the decoded framebuffer (instead of emitting it as separate
    /// pointer events for a hardware/overlay cursor). Useful for headless clients that have no
    /// overlay of their own and want the cursor captured in the frame.
    #[must_use]
    pub fn with_pointer_software_rendering(mut self, enabled: bool) -> Self {
        self.pointer_software_rendering = Some(enabled);
        self
    }

    /// Set the Client Info PDU performance flags.
    #[must_use]
    pub fn with_performance_flags(mut self, flags: ironrdp_pdu::rdp::client_info::PerformanceFlags) -> Self {
        self.performance_flags = Some(flags);
        self
    }

    /// Enable or disable bulk compression support. Upserts the `compression` property.
    #[must_use]
    pub fn with_compression(mut self, enabled: bool) -> Self {
        self.compression_enabled = Some(enabled);
        self.properties.set_compression(enabled);
        self
    }

    /// Set the bulk compression type directly. Upserts the `ironrdp_compressionlevel` property,
    /// or clears it when `ty` is `None`.
    ///
    /// When set, the `INFO_COMPRESSION` flag is included in the Client Info PDU and the specified
    /// compression type is advertised. The server may then send compressed PDUs using any
    /// compression algorithm up to and including this level:
    ///
    /// - `None` — no compression (default)
    /// - `Some(K8)` — MPPC with 8 KB history (RDP 4.0)
    /// - `Some(K64)` — MPPC with 64 KB history (RDP 5.0)
    /// - `Some(Rdp6)` — NCRUSH (RDP 6.0)
    /// - `Some(Rdp61)` — XCRUSH (RDP 6.1)
    #[must_use]
    pub fn with_compression_type(mut self, ty: Option<ironrdp_pdu::rdp::client_info::CompressionType>) -> Self {
        self.compression_type = ty;
        if let Some(ty) = ty {
            self.properties.set_compression_level(level_from_compression_type(ty));
        } else {
            self.properties.clear_compression_level();
        }
        self
    }

    /// Set the bulk compression type from a level (0–3). Out-of-range levels are ignored.
    ///
    /// The level maps to a [`CompressionType`](ironrdp_pdu::rdp::client_info::CompressionType):
    /// `0` → `K8`, `1` → `K64`, `2` → `Rdp6`, `3` → `Rdp61`. See
    /// [`with_compression_type`](Self::with_compression_type) for the semantics of each level.
    #[must_use]
    pub fn with_compression_level(self, level: u32) -> Self {
        match compression_type_from_level(level) {
            Ok(ty) => self.with_compression_type(Some(ty)),
            Err(_) => self,
        }
    }

    /// Select the transport. Upserts the corresponding addressing properties
    /// (`ironrdp_rdcleanpathurl`, `gatewayhostname`/`gatewayusagemethod`), clearing the other
    /// transport's properties so the PropertySet stays consistent.
    ///
    /// Secrets are *not* set here: supply gateway credentials via
    /// [`with_gateway_username`](Self::with_gateway_username) /
    /// [`with_gateway_password`](Self::with_gateway_password) and the RDCleanPath token via
    /// [`with_rdcleanpath_token`](Self::with_rdcleanpath_token).
    /// A `gatewaycredentialssource` property of
    /// [`UseServerCredentials`](ironrdp_cfg::GatewayCredentialsSource::UseServerCredentials)
    /// resolves unspecified gateway credentials from the RDP server account. Standard gateway
    /// properties select HTTP/WebSocket; `GatewayRpch` sets the IronRDP-specific
    /// `ironrdp_gatewayrpch:i:1` opt-in.
    #[must_use]
    pub fn with_transport(mut self, transport: TransportKind) -> Self {
        self.properties.remove("ironrdp_gatewayrpch");
        match &transport {
            TransportKind::Direct => {
                self.properties.clear_rdcleanpath();
                self.properties.clear_gateway();
                #[cfg(windows)]
                self.properties.clear_named_pipe();
            }
            TransportKind::RDCleanPath { url } => {
                self.properties.clear_gateway();
                #[cfg(windows)]
                self.properties.clear_named_pipe();
                self.properties.set_rdcleanpath_url(url.to_string());
            }
            #[cfg(feature = "gateway")]
            TransportKind::Gateway {
                endpoint,
                prefer_direct,
            }
            | TransportKind::GatewayRpch {
                endpoint,
                prefer_direct,
            } => {
                self.properties.clear_rdcleanpath();
                #[cfg(windows)]
                self.properties.clear_named_pipe();
                self.properties.set_gateway_hostname(endpoint.clone());
                self.properties.set_gateway_usage_method(if *prefer_direct {
                    ironrdp_cfg::GatewayUsageMethod::Detect
                } else {
                    ironrdp_cfg::GatewayUsageMethod::UseAlways
                });
                if matches!(transport, TransportKind::GatewayRpch { .. }) {
                    self.properties.insert("ironrdp_gatewayrpch", 1i64);
                }
            }
            #[cfg(windows)]
            TransportKind::NamedPipe { path } => {
                self.properties.clear_rdcleanpath();
                self.properties.clear_gateway();
                self.properties.set_named_pipe(path.clone());
            }
        }
        self.transport = transport;
        self
    }

    /// Enables reliable RDP-UDP2 for eligible direct connections.
    ///
    /// Unsupported carriers remain TCP-only, and a failed UDP bootstrap falls
    /// back to the main TCP connection.
    #[cfg(feature = "udp")]
    #[must_use]
    pub fn with_udp_transport(mut self, enabled: bool) -> Self {
        self.udp_transport_enabled = enabled;
        self
    }

    /// Selects the highest RDP-UDP protocol version the UDP handshake offers.
    ///
    /// The default, [`UdpVersion::V3`], offers MS-RDPEUDP2 and lets the server settle on a
    /// lower version. Offering [`UdpVersion::V2`] or [`UdpVersion::V1`] asks for MS-RDPEUDP
    /// outright, which is what Windows servers that do not implement version 3 answer.
    #[cfg(feature = "udp")]
    #[must_use]
    pub fn with_udp_offer_version(mut self, version: UdpVersion) -> Self {
        self.udp_offer_version = Some(version);
        self
    }

    /// Advertises the graphics pipeline ([MS-RDPEGFX]) to the server.
    ///
    /// On by default when an H.264 decoder is configured ([`Self::with_h264_decoder`]), so the
    /// server can send AVC420 and AVC444; `false` opts out. Without it, the server keeps graphics
    /// on the legacy bitmap path, which travels on the main connection and cannot move to a
    /// reliable UDP tunnel.
    ///
    /// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/
    #[must_use]
    pub fn with_graphics_pipeline(mut self, enabled: bool) -> Self {
        self.graphics_pipeline = Some(enabled);
        self
    }

    /// Keep the graphics pipeline's bitmap cache in `path` across sessions, or only in memory with
    /// `None` (the default). [`crate::egfx_cache::default_cache_path`] gives a per-user location.
    ///
    /// See [`ChannelConfig::egfx_persistent_cache`].
    #[must_use]
    pub fn with_egfx_persistent_cache(mut self, path: Option<PathBuf>) -> Self {
        self.channels.egfx_persistent_cache = path;
        self
    }

    /// Connect to a Hyper-V VM console by VM GUID. Destination must use port
    /// [`ironrdp_vmconnect::PORT`] (2179) unless the caller explicitly selects another port.
    /// A destination with no explicit port defaults to 2179 instead of the ordinary RDP port.
    ///
    /// Security (TLS + CredSSP) is required by [`ironrdp_vmconnect::connect_front`] for every
    /// embedder (error if disabled). Works over Direct, RDCleanPath, and RDS Gateway (the
    /// destination port, typically 2179, is forwarded in the MS-TSGU channel-create packet,
    /// then the VMConnect PCB / `connect_front` handshake runs on the tunneled stream).
    #[cfg(feature = "vmconnect")]
    #[must_use]
    pub fn with_vmconnect(mut self, vm_id: impl Into<String>) -> Self {
        let vm_id = vm_id.into();
        self.properties.set_vmconnect_id(vm_id.clone());
        self.properties.set_vmconnect_basic(false);
        self.vm_id = Some(vm_id);
        self.vmconnect_mode = VmConnectMode::Enhanced;
        self
    }

    /// Connect to a Hyper-V VM console using the selected mode.
    ///
    /// See [`with_vmconnect`](Self::with_vmconnect) for transport notes.
    #[cfg(feature = "vmconnect")]
    #[must_use]
    pub fn with_vmconnect_mode(mut self, vm_id: impl Into<String>, mode: VmConnectMode) -> Self {
        let vm_id = vm_id.into();
        self.properties.set_vmconnect_id(vm_id.clone());
        self.properties.set_vmconnect_basic(mode == VmConnectMode::Basic);
        self.vm_id = Some(vm_id);
        self.vmconnect_mode = mode;
        self
    }

    /// Use the caller's current Windows logon token for VMConnect host authentication.
    ///
    /// This is the native VMConnect behavior for local Hyper-V connections and avoids handling a reusable host password.
    #[cfg(all(windows, feature = "vmconnect"))]
    #[must_use]
    pub fn with_vmconnect_current_user(mut self, enabled: bool) -> Self {
        self.properties.set_vmconnect_current_user(enabled);
        self.vmconnect_current_user = enabled;
        self
    }

    /// Set the RDCleanPath authentication token (only meaningful with an RDCleanPath transport).
    ///
    /// The token is a secret: like the gateway password, it is *not* mirrored into the PropertySet,
    /// and [`build`](Self::build) strips any token loaded from a `.rdp` file before exposing
    /// [`Config::properties`].
    #[must_use]
    pub fn with_rdcleanpath_token(mut self, token: impl Into<String>) -> Self {
        self.rdcleanpath_token = Some(token.into());
        self
    }

    /// Set the kerberos config. Upserts the `kdcproxyurl` property (or clears it when the config
    /// has no KDC proxy URL) and always clears `kdcproxyname`, so the properties cannot select a
    /// different KDC than the explicit config; `hostname` is derived from the client name and not
    /// stored separately.
    #[must_use]
    pub fn with_kerberos_config(mut self, cfg: ironrdp_connector::credssp::KerberosConfig) -> Self {
        self.properties.clear_kdc_proxy_name();
        if let KdcResolution::KdcUrl(Some(url)) = &cfg.kdc_resolution {
            self.properties.set_kdc_proxy_url(url.to_string());
        } else {
            self.properties.clear_kdc_proxy_url();
        }
        self.kerberos_config = Some(cfg);
        self
    }

    /// Require experimental RCG instead of ordinary password-delegating CredSSP.
    ///
    /// Requires the `remote-guard` feature, strict TLS validation, and Kerberos-capable credentials.
    /// The mode mirrors CredSSP package selection: `Negotiate` redirects both Kerberos and
    /// NTLMv2, while `Kerberos` and `Ntlm` restrict redirection to one package.
    #[must_use]
    pub fn with_remote_guard_mode(mut self, mode: RemoteGuardMode) -> Self {
        self.properties.set_remote_guard_mode(mode);
        self.remote_guard_mode = mode;
        self
    }

    #[cfg(feature = "__test")]
    #[doc(hidden)]
    pub fn __test_remote_guard_mode_and_properties(&self) -> (RemoteGuardMode, &PropertySet) {
        (self.remote_guard_mode, &self.properties)
    }

    /// Set the minimum interval between non-forced input batches.
    ///
    /// Mouse movement is batched until this interval elapses.
    /// Keyboard, button, wheel, synchronization, and QoE events remain immediate.
    #[must_use]
    pub fn with_input_send_interval(mut self, interval: Duration) -> Self {
        self.input_send_interval = Some(interval);
        self
    }

    /// Set the interval between synthetic input keepalive events.
    ///
    /// Unlike [`with_fake_events_interval`](Self::with_fake_events_interval), this does not write the minute-based `ironrdp_fakeeventsinterval` compatibility property.
    /// When both intervals are configured, the shorter interval controls synthetic input.
    #[must_use]
    pub fn with_input_keepalive_interval(mut self, interval: Duration) -> Self {
        self.input_keepalive_interval = Some(interval);
        self
    }

    #[must_use]
    pub fn with_fake_events_interval(mut self, interval: Duration) -> Self {
        self.fake_events_interval = Some(interval);
        self.properties
            .set_fake_events_interval(u32::try_from(interval.as_secs() / 60).unwrap_or(u32::MAX));
        self
    }

    /// Bound each TCP connection attempt of a direct connection.
    ///
    /// The destination host name is resolved first, then each address is tried in order, and an
    /// address that does not accept the connection within `timeout` is abandoned for the next one.
    /// Without this setting the operating system's connect timeout applies to each address.
    /// [`build`](Self::build) rejects a zero timeout.
    #[must_use]
    pub fn with_tcp_connect_attempt_timeout(mut self, timeout: Duration) -> Self {
        self.tcp_connect_attempt_timeout = Some(timeout);
        self
    }

    /// Enable or disable RDPSND (audio) playback.
    #[cfg(feature = "sound")]
    #[must_use]
    pub fn with_sound(self, enabled: bool) -> Self {
        self.with_audio_mode(if enabled {
            ironrdp_cfg::AudioMode::RedirectToClient
        } else {
            ironrdp_cfg::AudioMode::Disabled
        })
    }

    /// Set the public RDP audio redirection mode.
    ///
    /// `PlayOnServer` and `Disabled` currently share the same client-info outcome:
    /// both clear local RDPSND playback and set `INFO_NOAUDIOPLAYBACK`. Distinct
    /// `INFO_REMOTECONSOLEAUDIO` (true “play on server console”) is not wired yet.
    #[cfg(feature = "sound")]
    #[must_use]
    pub fn with_audio_mode(mut self, mode: ironrdp_cfg::AudioMode) -> Self {
        self.channels.sound = matches!(mode, ironrdp_cfg::AudioMode::RedirectToClient);
        self.enable_audio_playback = Some(matches!(mode, ironrdp_cfg::AudioMode::RedirectToClient));
        self.properties.set_audio_mode(mode);
        self
    }

    /// Enable or disable client microphone capture (MS-RDPEAI / AUDIO_INPUT).
    #[cfg(feature = "sound")]
    #[must_use]
    pub fn with_audio_capture(mut self, enabled: bool) -> Self {
        self.channels.audio_capture = enabled;
        self.enable_audio_capture = Some(enabled);
        self.properties.set_audio_capture_mode(if enabled {
            ironrdp_cfg::AudioCaptureMode::CaptureFromClient
        } else {
            ironrdp_cfg::AudioCaptureMode::Disabled
        });
        self
    }

    /// Set the audio output device: a device ID or (part of) a device name; `None` follows the
    /// system default device.
    #[cfg(feature = "sound")]
    #[must_use]
    pub fn with_audio_output_device(mut self, device: Option<String>) -> Self {
        match &device {
            Some(device) => self
                .properties
                .insert(crate::audio::OUTPUT_DEVICE_PROPERTY, device.clone()),
            None => self.properties.remove(crate::audio::OUTPUT_DEVICE_PROPERTY),
        };
        self.channels.audio.output_device = device;
        self
    }

    /// Set the audio input (microphone) device: a device ID or (part of) a device name; `None`
    /// follows the system default device.
    #[cfg(feature = "sound")]
    #[must_use]
    pub fn with_audio_input_device(mut self, device: Option<String>) -> Self {
        match &device {
            Some(device) => self
                .properties
                .insert(crate::audio::INPUT_DEVICE_PROPERTY, device.clone()),
            None => self.properties.remove(crate::audio::INPUT_DEVICE_PROPERTY),
        };
        self.channels.audio.input_device = device;
        self
    }

    /// Set the audio kept buffered ahead of the output device (100 ms by default).
    ///
    /// Higher values ride out more network jitter at the cost of audio delay.
    #[cfg(feature = "sound")]
    #[must_use]
    pub fn with_audio_latency(mut self, latency: Duration) -> Self {
        self.properties.insert(
            crate::audio::LATENCY_PROPERTY,
            i64::try_from(latency.as_millis()).unwrap_or(i64::MAX),
        );
        self.channels.audio.latency = Some(latency);
        self
    }

    /// Audio device and buffering settings.
    #[cfg(feature = "sound")]
    pub fn audio(&self) -> &crate::audio::AudioConfig {
        &self.channels.audio
    }

    /// Set the RDPSND quality policy sent to servers supporting audio protocol version 6 or newer.
    #[cfg(any(feature = "sound", feature = "rdpdr"))]
    #[must_use]
    pub fn with_audio_quality_mode(mut self, mode: AudioQualityMode) -> Self {
        self.audio_quality_mode = Some(mode);
        self.properties.insert("audioqualitymode", mode.as_u32());
        self
    }

    /// Set the CLIPRDR (clipboard) redirection mode.
    #[cfg(feature = "clipboard")]
    #[must_use]
    pub fn with_clipboard(mut self, mode: ClipboardType) -> Self {
        self.channels.clipboard = mode;
        self.properties
            .set_redirect_clipboard(matches!(mode, ClipboardType::Enable));
        self
    }

    /// Enable or disable RDPDR (device redirection).
    #[cfg(feature = "rdpdr")]
    #[must_use]
    pub fn with_rdpdr(mut self, enabled: bool) -> Self {
        self.channels.rdpdr.enabled = enabled;
        self.properties.set_enable_rdpdr(enabled);
        self
    }

    /// Selects the local drives to redirect (`drivestoredirect`).
    #[cfg(feature = "rdpdr")]
    #[must_use]
    pub fn with_drive_redirection(mut self, drives: DriveRedirection) -> Self {
        self.channels.rdpdr.drives = drives;
        self.properties.set_drive_redirection(drives);
        self
    }

    /// Enable or disable serial and parallel port redirection within RDPDR (`redirectcomports`).
    #[cfg(feature = "rdpdr")]
    #[must_use]
    pub fn with_ports(mut self, enabled: bool) -> Self {
        self.channels.rdpdr.ports = enabled;
        self.properties.set_redirect_com_ports(enabled);
        self
    }

    /// Allow or drop printers announced by the RDPDR backend product (`redirectprinters`).
    #[cfg(feature = "rdpdr")]
    #[must_use]
    pub fn with_printers(mut self, enabled: bool) -> Self {
        self.channels.rdpdr.printers = enabled;
        self.properties.set_redirect_printers(enabled);
        self
    }

    /// Enable or disable smart-card redirection within RDPDR.
    #[cfg(feature = "smartcard")]
    #[must_use]
    pub fn with_smartcard(mut self, enabled: bool) -> Self {
        self.channels.rdpdr.smartcard = enabled;
        self.properties.set_enable_smartcard(enabled);
        self
    }

    /// Enable or disable QOI bitmap codec at runtime.
    #[cfg(feature = "qoi")]
    #[must_use]
    pub fn with_qoi(mut self, enabled: bool) -> Self {
        self.channels.qoi = enabled;
        self.properties.set_enable_qoi(enabled);
        self
    }

    /// Enable or disable QOIZ bitmap codec at runtime.
    #[cfg(feature = "qoiz")]
    #[must_use]
    pub fn with_qoiz(mut self, enabled: bool) -> Self {
        self.channels.qoiz = enabled;
        self.properties.set_enable_qoiz(enabled);
        self
    }

    /// Enable or disable native MS-RDPEWA WebAuthn redirection.
    #[cfg(feature = "webauthn")]
    #[must_use]
    pub fn with_webauthn(mut self, enabled: bool) -> Self {
        self.channels.webauthn = enabled;
        self.properties.set_redirect_webauthn(enabled);
        self
    }

    /// Enable or disable explicit caller-supplied location redirection.
    #[cfg(feature = "location")]
    #[must_use]
    pub fn with_location_redirection(mut self, enabled: bool) -> Self {
        self.channels.location = enabled;
        self
    }

    /// Advertise that the client sends relative mouse input.
    ///
    /// Sets `RNS_UD_CS_RELATIVE_MOUSE_INPUT` in the GCC Client Core Data. Enable it before sending
    /// relative pointer events.
    #[must_use]
    pub fn with_relative_mouse_input(mut self, enabled: bool) -> Self {
        self.relative_mouse_input = enabled;
        self
    }

    /// Set the maximum number of simultaneous touch contacts advertised over RDPEI.
    ///
    /// [`build`](Self::build) rejects values outside 1 through 256, the RDPEI limit.
    #[must_use]
    pub fn with_max_touch_contacts(mut self, contacts: u16) -> Self {
        self.channels.max_touch_contacts = contacts;
        self
    }

    /// Select the cameras to redirect over MS-RDPECAM.
    ///
    /// Capture is only implemented on Windows.
    #[cfg(feature = "camera")]
    #[must_use]
    pub fn with_cameras(mut self, cameras: crate::camera::CameraConfig) -> Self {
        self.channels.cameras = cameras;
        self
    }

    /// Set the H.264 decoder used by the graphics pipeline and MS-RDPEVOR video optimized remoting.
    ///
    /// `None` keeps the graphics pipeline to the codecs that need no H.264 decoder, turns it off
    /// unless [`Self::with_graphics_pipeline`] turns it on, and declines the video channels.
    #[must_use]
    pub fn with_h264_decoder(mut self, decoder: Option<crate::h264::H264DecoderFactory>) -> Self {
        self.channels.h264_decoder = decoder;
        self
    }

    /// Cameras currently selected for redirection, e.g. from a `.rdp` file.
    #[cfg(feature = "camera")]
    pub fn cameras(&self) -> &crate::camera::CameraConfig {
        &self.channels.cameras
    }

    /// Select the local USB devices to redirect over MS-RDPEUSB, or disable USB redirection with `None`.
    ///
    /// This is not mirrored into the property set: FreeRDP-style `addr:` and `auto` rules have no
    /// `usbdevicestoredirect` equivalent.
    #[cfg(feature = "usb")]
    #[must_use]
    pub fn with_usb_redirection(mut self, usb: Option<ironrdp_rdpeusb_native::UsbRedirectionConfig>) -> Self {
        self.channels.usb = usb;
        self
    }

    /// Parent HWND used for WebAuthn UI prompts (Windows only).
    #[cfg(all(windows, feature = "webauthn"))]
    #[must_use]
    pub fn with_webauthn_parent_hwnd(mut self, hwnd: isize) -> Self {
        self.webauthn_parent_hwnd = Some(hwnd);
        self
    }

    // TODO: It can be useful to have a method for enabling or disabling all the extra channels at once.
    // Example: in tests, disable all + enable only the required channel.

    /// Add a DVC pipe proxy channel.
    #[cfg(feature = "dvc-pipe-proxy")]
    #[must_use]
    pub fn with_dvc_pipe_proxy(mut self, info: DvcProxyInfo) -> Self {
        self.dvc_pipe_proxies.push(info);
        self.properties
            .set_dvc_pipe_proxies(self.dvc_pipe_proxies.iter().map(|p| ironrdp_cfg::DvcPipeProxy {
                channel_name: p.channel_name.clone(),
                pipe_name: p.pipe_name.clone(),
            }));
        self
    }

    /// Add a DVC COM plugin DLL path (Windows only).
    #[cfg(all(windows, feature = "dvc-com-plugin"))]
    #[must_use]
    pub fn with_dvc_plugin(mut self, path: impl Into<PathBuf>) -> Self {
        self.dvc_plugins.push(path.into());
        self.properties
            .set_dvc_plugins(self.dvc_plugins.iter().map(PathBuf::as_path));
        self
    }

    /// Register a factory for a user-defined static virtual channel.
    ///
    /// `factory` is called once per connection attempt with the shared (read-only) [`PropertySet`],
    /// so the channel can parametrize itself from the standard frontend config. Return `None` to
    /// disable the channel. Duplicate processor types follow `attach_static_channel` overwrite semantics.
    #[must_use]
    pub fn with_static_channel<P, F>(mut self, factory: F) -> Self
    where
        F: Fn(&PropertySet) -> Option<P> + Send + Sync + 'static,
        P: ironrdp_svc::SvcClientProcessor + 'static,
    {
        let cb: StaticChannelFn = Arc::new(move |connector: &mut ironrdp_connector::ClientConnector, ps| {
            if let Some(processor) = factory(ps) {
                connector.attach_static_channel(processor);
            }
        });
        self.extensions.static_channels.push(cb);
        self
    }

    /// Register a factory for multiple runtime-defined instances of one static-channel processor.
    #[must_use]
    pub fn with_static_channel_instances<P, F>(mut self, factory: F) -> Self
    where
        F: Fn(&PropertySet) -> Vec<P> + Send + Sync + 'static,
        P: ironrdp_svc::SvcClientProcessor + 'static,
    {
        let cb: StaticChannelFn = Arc::new(move |connector: &mut ironrdp_connector::ClientConnector, ps| {
            for processor in factory(ps) {
                match connector.try_attach_dynamic_static_channel(processor) {
                    Ok(()) => {}
                    Err(ironrdp_connector::DynamicStaticChannelAttachError::ChannelLimitReached) => {
                        tracing::error!("Unable to register runtime-defined static channel: key space exhausted");
                    }
                    Err(ironrdp_connector::DynamicStaticChannelAttachError::DuplicateChannelName) => {
                        tracing::warn!("Unable to register runtime-defined static channel: duplicate channel name");
                    }
                }
            }
        });
        self.extensions.static_channels.push(cb);
        self
    }

    /// Register a factory for a user-defined dynamic virtual channel.
    ///
    /// `factory` is called once per connection attempt with the shared (read-only) [`PropertySet`],
    /// so the channel can parametrize itself from the standard frontend config. Return `None` to
    /// disable the channel. Duplicate channel names follow `DrdynvcClient` overwrite semantics.
    #[must_use]
    pub fn with_dvc<P, F>(mut self, factory: F) -> Self
    where
        F: Fn(&PropertySet) -> Option<P> + Send + Sync + 'static,
        P: ironrdp_dvc::DvcClientProcessor + 'static,
    {
        let cb: DvcChannelFn = Arc::new(move |drdynvc, ps| {
            if let Some(processor) = factory(ps) {
                drdynvc.attach_dynamic_channel(processor);
            }
        });
        self.extensions.dvc_channels.push(cb);
        self
    }

    fn is_avd_gateway(&self) -> bool {
        #[cfg(all(feature = "gateway", feature = "entra-auth"))]
        {
            matches!(self.transport, TransportKind::Gateway { .. })
                && self.properties.get::<i64>("gatewaybrokeringtype") == Some(1)
        }
        #[cfg(not(all(feature = "gateway", feature = "entra-auth")))]
        {
            false
        }
    }

    /// List the required fields that still need a value before [`build`](Self::build) can succeed.
    ///
    /// Gateway credentials are only required when a gateway transport is selected.
    /// When `gatewaycredentialssource` selects `UseServerCredentials`, gateway fields are omitted
    /// because [`build`](Self::build) resolves them from the RDP server account.
    pub fn missing(&self) -> Vec<MissingField> {
        let mut missing = Vec::new();
        let is_avd_gateway = self.is_avd_gateway();
        #[cfg(feature = "entra-auth")]
        let uses_entra_auth = self.entra_auth.is_some()
            || (is_avd_gateway && self.properties.get::<bool>("enablerdsaadauth").unwrap_or(false));
        #[cfg(not(feature = "entra-auth"))]
        let uses_entra_auth = false;
        #[cfg(not(feature = "gateway"))]
        let _ = is_avd_gateway;
        #[cfg(all(windows, feature = "vmconnect"))]
        let uses_current_vmconnect_credentials = self.vm_id.is_some() && self.vmconnect_current_user;
        #[cfg(not(all(windows, feature = "vmconnect")))]
        let uses_current_vmconnect_credentials = false;
        #[cfg(feature = "gateway")]
        let gateway_uses_server_credentials = matches!(
            self.transport,
            TransportKind::Gateway { .. } | TransportKind::GatewayRpch { .. }
        ) && matches!(
            self.properties.gateway_credentials_source(),
            Ok(Some(ironrdp_cfg::GatewayCredentialsSource::UseServerCredentials))
        );
        #[cfg(not(feature = "gateway"))]
        let gateway_uses_server_credentials = false;
        if self.destination.is_none() {
            missing.push(MissingField::ServerAddress);
        }
        if !uses_entra_auth
            && self.username.is_none()
            && (!uses_current_vmconnect_credentials
                || (gateway_uses_server_credentials && self.gateway_username.is_none()))
        {
            missing.push(MissingField::Username);
        }
        if !uses_entra_auth
            && self.password.is_none()
            && (!uses_current_vmconnect_credentials
                || (gateway_uses_server_credentials && self.gateway_password.is_none()))
        {
            missing.push(MissingField::Password);
        }
        #[cfg(feature = "gateway")]
        if matches!(
            self.transport,
            TransportKind::Gateway { .. } | TransportKind::GatewayRpch { .. }
        ) && !gateway_uses_server_credentials
            && !is_avd_gateway
        {
            if self.gateway_username.is_none() {
                missing.push(MissingField::GatewayUsername);
            }
            if self.gateway_password.is_none() {
                missing.push(MissingField::GatewayPassword);
            }
        }
        if matches!(self.transport, TransportKind::RDCleanPath { .. }) && self.rdcleanpath_token.is_none() {
            missing.push(MissingField::RDCleanPathToken);
        }
        if self.client_build.is_none() {
            missing.push(MissingField::ClientBuild);
        }
        if self.client_dir.is_none() {
            missing.push(MissingField::ClientDir);
        }
        if self.platform.is_none() {
            missing.push(MissingField::Platform);
        }
        if self.client_name.is_none() {
            missing.push(MissingField::ClientName);
        }
        missing
    }

    /// Build the [`Config`], filling optional settings with sensible defaults.
    ///
    /// Fails if any required field is unset; inspect [`missing`](Self::missing) beforehand to resolve them.
    #[expect(
        clippy::missing_panics_doc,
        reason = "a panic here would be a bug (secrets are guaranteed present by missing()), not documented behavior"
    )]
    // `mut` is only required when the vmconnect default-port path mutates destination/properties.
    #[cfg_attr(not(any(feature = "vmconnect", feature = "gateway")), expect(unused_mut))]
    pub fn build(mut self) -> anyhow::Result<Config> {
        use ironrdp_pdu::rdp::capability_sets::client_codecs_capabilities;
        use ironrdp_pdu::rdp::client_info::TimezoneInfo;

        if let Some(scale) = self.device_scale_factor {
            anyhow::ensure!(
                matches!(scale, 100 | 140 | 180),
                "invalid device scale factor: valid values are 100, 140 and 180"
            );
        }
        anyhow::ensure!(
            (1..=256).contains(&self.channels.max_touch_contacts),
            "invalid maximum touch contacts: valid values are 1 through 256"
        );
        if let Some(timeout) = self.tcp_connect_attempt_timeout {
            anyhow::ensure!(!timeout.is_zero(), "the TCP connect attempt timeout must not be zero");
        }

        if let Some(load_balance_info) = &self.load_balance_info {
            anyhow::ensure!(
                load_balance_info.len() <= ironrdp_pdu::nego::MAX_ROUTING_TOKEN_LENGTH
                    && !load_balance_info.is_empty()
                    && load_balance_info
                        .as_bytes()
                        .iter()
                        .all(|byte| (0x20..=0x7E).contains(byte)),
                "invalid load-balance routing token"
            );
        }

        let is_avd_gateway = self.is_avd_gateway();
        if self.properties.get::<i64>("gatewaybrokeringtype") == Some(1) && !is_avd_gateway {
            anyhow::bail!(
                "AVD gateway orchestration (gatewaybrokeringtype:i:1) requires the HTTP/WebSocket gateway transport and the entra-auth feature"
            );
        }

        #[cfg(feature = "gateway")]
        if matches!(
            self.transport,
            TransportKind::Gateway { .. } | TransportKind::GatewayRpch { .. }
        ) && !is_avd_gateway
        {
            use ironrdp_cfg::GatewayCredentialsSource;

            match self
                .properties
                .gateway_credentials_source()
                .context("invalid gateway credential source")?
            {
                Some(GatewayCredentialsSource::UseServerCredentials) => {
                    #[cfg(feature = "entra-auth")]
                    if self.entra_auth.is_some() || self.properties.get::<bool>("enablerdsaadauth").unwrap_or(false) {
                        anyhow::bail!("gateway UseServerCredentials requires RDP username and password credentials");
                    }
                    if self.gateway_username.is_none() {
                        self.gateway_username = self.username.as_deref().map(|username| match self.domain.as_deref() {
                            Some(domain)
                                if !domain.is_empty() && !username.contains('\\') && !username.contains('@') =>
                            {
                                format!("{domain}\\{username}")
                            }
                            _ => username.to_owned(),
                        });
                    }
                    if self.gateway_password.is_none() {
                        self.gateway_password = self.password.clone();
                    }
                }
                None | Some(GatewayCredentialsSource::UseUserCredentials) => {}
                Some(GatewayCredentialsSource::UseProfile) => {
                    anyhow::bail!(
                        "gateway credential source UseProfile requires an unavailable profile credential provider"
                    );
                }
                Some(GatewayCredentialsSource::Prompt) => {
                    anyhow::bail!(
                        "gateway credential source Prompt requires an unavailable interactive credential prompt provider"
                    );
                }
                Some(GatewayCredentialsSource::SmartCard) => {
                    anyhow::bail!(
                        "gateway credential source SmartCard requires an unavailable smart-card credential provider"
                    );
                }
                Some(GatewayCredentialsSource::UseLogonCredentials) => {
                    anyhow::bail!(
                        "gateway credential source UseLogonCredentials requires an unavailable OS logon credential provider"
                    );
                }
            }
        }

        let missing = self.missing();
        if !missing.is_empty() {
            anyhow::bail!(
                "missing required configuration: {}",
                missing
                    .iter()
                    .map(MissingField::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        #[cfg(feature = "vmconnect")]
        if let Some(vm_id) = &self.vm_id {
            anyhow::ensure!(!vm_id.trim().is_empty(), "vmconnect VM ID is empty");
        }

        #[cfg(feature = "vmconnect")]
        if self.vm_id.is_some()
            && let Some(destination) = self.destination.as_mut()
            && destination.name == "."
        {
            destination.name = "localhost".to_owned();
            self.properties.set_full_address(&ironrdp_cfg::TargetAddr {
                host: ironrdp_cfg::TargetHost::Domain(destination.name.clone()),
                port: destination.port,
            });
            self.properties.clear_alternate_full_address();
        }

        #[cfg(all(windows, feature = "vmconnect"))]
        if self.vm_id.is_some()
            && self.vmconnect_current_user
            && matches!(self.transport, TransportKind::RDCleanPath { .. })
        {
            anyhow::bail!("vmconnect current-user authentication is not supported with RDCleanPath");
        }

        #[cfg(all(windows, feature = "vmconnect"))]
        let vmconnect_framebuffer_redirection = self.vm_id.is_some()
            && matches!(&self.transport, TransportKind::Direct)
            && self
                .destination
                .as_ref()
                .is_some_and(|destination| is_loopback_host(destination.name()));

        #[cfg(all(windows, feature = "vmconnect"))]
        if vmconnect_framebuffer_redirection && self.dig_product_id.as_deref().is_none_or(str::is_empty) {
            self.dig_product_id = Some(
                ironrdp_vmconnect::local_instance_id()
                    .context("read local RDP InstanceID for VMConnect frame-buffer redirection")?,
            );
        }

        #[cfg(feature = "vmconnect")]
        if self.vm_id.is_some()
            && let Some(destination) = self.destination.as_mut()
            && destination.port.is_none()
        {
            destination.port = Some(ironrdp_vmconnect::PORT);
            self.properties.set_server_port(ironrdp_vmconnect::PORT);
        }

        #[cfg(feature = "entra-auth")]
        let property_entra_auth = self.properties.get::<bool>("enablerdsaadauth").unwrap_or(false);
        #[cfg(feature = "entra-auth")]
        if property_entra_auth && self.entra_auth.is_none() && !is_avd_gateway {
            anyhow::bail!(
                "enablerdsaadauth requires an EntraAuthSession (ConfigBuilder::with_entra_auth) or an AVD gateway (gatewaybrokeringtype:i:1)"
            );
        }
        #[cfg(feature = "entra-auth")]
        let entra_auth_enabled = self.entra_auth.is_some() || property_entra_auth;
        #[cfg(not(feature = "entra-auth"))]
        let entra_auth_enabled = false;

        if self.certificate_validation == Some(ironrdp_tls::CertificateValidation::DangerouslyAcceptInvalidCertificate)
            && self.certificate_validation_callback.is_some()
        {
            anyhow::bail!("cannot combine a dangerous certificate policy with a validation callback");
        }

        let codecs: Vec<&str> = self.codecs.iter().map(String::as_str).collect();
        let codecs = client_codecs_capabilities(&codecs).map_err(|help| anyhow::anyhow!("{help}"))?;
        let color_depth = self.color_depth.unwrap_or(32);
        if color_depth != 16 && color_depth != 32 {
            anyhow::bail!("invalid color depth: only 16 and 32 bit color depths are supported");
        }
        let bitmap = ironrdp_connector::BitmapConfig {
            color_depth,
            lossy_compression: self.lossy_compression.unwrap_or(true),
            codecs,
        };

        // Named-pipe RDP (Windows Sandbox) is the only built-in path that opts into PROTOCOL_RDP
        // with ENCRYPTION_LEVEL_NONE. Keep TCP/gateway paths on enhanced security by default.
        // Check before consuming `self.transport` below.
        #[cfg(windows)]
        let enable_standard_rdp_security = matches!(&self.transport, TransportKind::NamedPipe { .. });
        #[cfg(not(windows))]
        let enable_standard_rdp_security = false;

        #[cfg(feature = "entra-auth")]
        if entra_auth_enabled {
            anyhow::ensure!(
                self.enable_tls.unwrap_or(true),
                "Entra RDS AAD authentication requires TLS"
            );
            anyhow::ensure!(
                self.certificate_validation_callback.is_none(),
                "Entra RDS AAD authentication does not support certificate-validation overrides"
            );
            anyhow::ensure!(
                !matches!(self.transport, TransportKind::RDCleanPath { .. }),
                "Entra RDS AAD authentication requires a validated TLS connection to the RDP server and is not supported with RDCleanPath"
            );
            anyhow::ensure!(
                !enable_standard_rdp_security,
                "Entra RDS AAD authentication is not supported over standard-security transports"
            );
            #[cfg(feature = "vmconnect")]
            anyhow::ensure!(
                self.vm_id.is_none(),
                "Entra RDS AAD authentication is not supported with VMConnect"
            );
        }

        // Resolve the granular transport selection into the bundled form, folding in the separately
        // tracked secrets (gateway credentials, RDCleanPath token).
        #[expect(
            clippy::unwrap_used,
            reason = "the transport secrets are guaranteed present by the missing() check above"
        )]
        let transport = match self.transport {
            TransportKind::Direct => Transport::Direct,
            #[cfg(feature = "gateway")]
            TransportKind::Gateway {
                endpoint,
                prefer_direct,
            } => Transport::Gateway(GatewayConfig {
                endpoint,
                username: self.gateway_username.unwrap_or_default(),
                password: self.gateway_password.unwrap_or_default(),
                prefer_direct,
            }),
            #[cfg(feature = "gateway")]
            TransportKind::GatewayRpch {
                endpoint,
                prefer_direct,
            } => Transport::GatewayRpch(GatewayConfig {
                endpoint,
                username: self.gateway_username.unwrap(),
                password: self.gateway_password.unwrap(),
                prefer_direct,
            }),
            TransportKind::RDCleanPath { url } => Transport::RDCleanPath(RDCleanPathConfig {
                url,
                auth_token: self.rdcleanpath_token.unwrap(),
            }),
            #[cfg(windows)]
            TransportKind::NamedPipe { path } => Transport::NamedPipe { path },
        };

        #[cfg(feature = "vmconnect")]
        if self.vm_id.is_some() {
            if !self.enable_tls.unwrap_or(true) {
                anyhow::bail!("vmconnect requires TLS");
            }
            if !self.enable_credssp.unwrap_or(true) {
                anyhow::bail!("vmconnect requires CredSSP");
            }
        }

        let client_name = self.client_name.unwrap_or_default();
        let mut kerberos_config = self
            .kerberos_config
            .or_else(|| kerberos_config_from_properties(&self.properties, &client_name));
        let certificate_validation = if entra_auth_enabled || is_avd_gateway {
            anyhow::ensure!(
                self.certificate_validation
                    != Some(ironrdp_tls::CertificateValidation::DangerouslyAcceptInvalidCertificate),
                "Entra RDS AAD and AVD gateway authentication require TLS certificate validation"
            );
            ironrdp_tls::CertificateValidation::Strict
        } else {
            self.certificate_validation.unwrap_or_else(|| {
                if self.certificate_validation_callback.is_some() {
                    ironrdp_tls::CertificateValidation::Strict
                } else {
                    ironrdp_tls::CertificateValidation::default()
                }
            })
        };

        // Bulk compression is enabled by default. We default to MPPC 64K (RDP5) rather than the
        // richer XCRUSH (RDP6.1) because it is the most universally supported and lowest-state
        // codec, and FastPath decompression is the only fully wired path.
        // FIXME: bump the default to RDP6.1 (XCRUSH) once slow-path bulk decompression is wired
        // (see ironrdp-session x224 path); until then a stateful codec risks silent corruption.
        let compression_type = if self.compression_enabled.unwrap_or(true) {
            Some(
                self.compression_type
                    .unwrap_or(ironrdp_pdu::rdp::client_info::CompressionType::K64),
            )
        } else {
            None
        };

        let enable_tls = if entra_auth_enabled {
            true
        } else if enable_standard_rdp_security {
            false
        } else {
            self.enable_tls.unwrap_or(true)
        };
        let enable_credssp =
            !entra_auth_enabled && !enable_standard_rdp_security && self.enable_credssp.unwrap_or(true);

        if self.remote_guard_mode.is_enabled() {
            if !cfg!(feature = "remote-guard") {
                anyhow::bail!("remote credential guard requires the experimental remote-guard build feature");
            }
            if !enable_tls || !enable_credssp || enable_standard_rdp_security {
                anyhow::bail!("remote credential guard requires TLS and CredSSP");
            }
            if matches!(transport, Transport::RDCleanPath(_)) {
                anyhow::bail!(
                    "remote credential guard requires a validated TLS connection to the RDP server and is not supported with RDCleanPath"
                );
            }
            #[cfg(feature = "vmconnect")]
            if self.vm_id.is_some() {
                anyhow::bail!("remote credential guard is not supported for VMConnect");
            }
            if certificate_validation != ironrdp_tls::CertificateValidation::Strict {
                anyhow::bail!("remote credential guard requires strict TLS certificate validation");
            }
            if client_name.is_empty() {
                anyhow::bail!("remote credential guard requires a client computer name");
            }
            if let Some(cfg) = &kerberos_config {
                if matches!(cfg.kdc_resolution, KdcResolution::IAKerb) {
                    anyhow::bail!("remote credential guard does not support IAKerb");
                }
            } else {
                if self.properties.kdc_proxy_url().is_some() || self.properties.kdc_proxy_name().is_some() {
                    anyhow::bail!("invalid KDC proxy configuration for remote credential guard");
                }
                kerberos_config = Some(ironrdp_connector::credssp::KerberosConfig {
                    kdc_resolution: KdcResolution::KdcUrl(None),
                    hostname: client_name.clone(),
                });
            }
        }

        let remote_application_mode = self.remote_application_mode.unwrap_or(false);
        let rail_support_level = self
            .rail_support_level
            .unwrap_or(ironrdp_pdu::rdp::capability_sets::RailSupportLevel::SUPPORTED);
        if remote_application_mode
            && !rail_support_level.contains(ironrdp_pdu::rdp::capability_sets::RailSupportLevel::SUPPORTED)
        {
            anyhow::bail!("RAIL support level must include remote programs support when RemoteApp mode is enabled");
        }

        let rail_initial_execute = if remote_application_mode {
            self.remote_application_program
                .as_deref()
                .filter(|program| !program.is_empty())
                .or_else(|| self.alternate_shell.as_deref().filter(|shell| !shell.is_empty()))
                .map(|executable| ExecutePdu {
                    flags: 0,
                    executable: executable.to_owned(),
                    working_directory: self.work_dir.clone().unwrap_or_default(),
                    arguments: String::new(),
                })
        } else {
            None
        };

        let connector = ironrdp_connector::Config {
            credentials: ironrdp_connector::Credentials::UsernamePassword {
                username: self.username.unwrap_or_default(),
                password: self.password.unwrap_or_default(),
            },
            domain: self.domain,
            enable_tls,
            enable_credssp,
            enable_standard_rdp_security,
            keyboard_type: self
                .keyboard_type
                .unwrap_or(ironrdp_pdu::gcc::KeyboardType::IBM_ENHANCED),
            keyboard_subtype: self.keyboard_subtype.unwrap_or(0),
            keyboard_layout: self.keyboard_layout.unwrap_or(0),
            keyboard_functional_keys_count: self.keyboard_functional_keys_count.unwrap_or(12),
            connection_type: self.connection_type.unwrap_or(ironrdp_pdu::gcc::ConnectionType::Lan),
            ime_file_name: self.ime_file_name.unwrap_or_default(),
            dig_product_id: self.dig_product_id.unwrap_or_default(),
            desktop_size: ironrdp_connector::DesktopSize {
                width: self.desktop_width.unwrap_or(DEFAULT_WIDTH),
                height: self.desktop_height.unwrap_or(DEFAULT_HEIGHT),
            },
            monitor_layout: self.monitor_layout,
            desktop_scale_factor: self.desktop_scale_factor.unwrap_or(0),
            desktop_physical_size: self.desktop_physical_size,
            desktop_orientation: self.desktop_orientation,
            device_scale_factor: self.device_scale_factor,
            monitor_extended_layout: self.monitor_extended_layout,
            relative_mouse_input: self.relative_mouse_input,
            bitmap: Some(bitmap),
            client_build: self.client_build.unwrap_or_default(),
            client_name,
            client_dir: self.client_dir.unwrap_or_default(),
            platform: self
                .platform
                .unwrap_or(ironrdp_pdu::rdp::capability_sets::MajorPlatformType::UNSPECIFIED),
            hardware_id: None,
            license_cache: None,
            enable_server_pointer: self.enable_server_pointer.unwrap_or(true),
            autologon: self.autologon.unwrap_or(false),
            enable_audio_playback: self.enable_audio_playback.unwrap_or(true),
            enable_audio_capture: self.enable_audio_capture.unwrap_or(false),
            request_data: None,
            pointer_software_rendering: self.pointer_software_rendering.unwrap_or(false),
            multitransport_flags: None,
            support_dyn_vc_gfx_protocol: self
                .graphics_pipeline
                .unwrap_or_else(|| self.channels.h264_decoder.is_some()),
            rdp_version: self.rdp_version.unwrap_or(ironrdp_pdu::gcc::RdpVersion::V5_PLUS),
            compression_type,
            performance_flags: self.performance_flags.unwrap_or_default(),
            timezone_info: TimezoneInfo::default(),
            alternate_shell: if remote_application_mode {
                String::new()
            } else {
                self.alternate_shell.unwrap_or_default()
            },
            work_dir: if remote_application_mode {
                String::new()
            } else {
                self.work_dir.unwrap_or_default()
            },
            remote_application_mode,
            rail_support_level,
        };

        // To avoid easily leaking secrets, strip any known secret property before returning the resulting Config.
        let mut properties = self.properties;
        let detected_secrets = properties
            .iter()
            .filter(|(key, _)| ironrdp_cfg::is_secret_key(key))
            .map(|(key, _)| key.clone().into_owned())
            .collect::<Vec<_>>();
        detected_secrets.into_iter().for_each(|key| {
            properties.remove(&key);
        });

        Ok(Config {
            connector,
            destination: self.destination.context("server address is required")?,
            transport,
            #[cfg(feature = "udp")]
            udp_transport_enabled: self.udp_transport_enabled,
            #[cfg(feature = "udp")]
            udp_offer_version: self.udp_offer_version.unwrap_or(UdpVersion::V3),
            certificate_validation,
            certificate_validation_callback: self.certificate_validation_callback,
            #[cfg(feature = "entra-auth")]
            entra_auth: self.entra_auth,
            #[cfg(all(feature = "gateway", feature = "entra-auth"))]
            avd_token_provider: self.avd_token_provider,
            #[cfg(all(feature = "gateway", feature = "entra-auth"))]
            avd_gateway_token: self.avd_gateway_token,
            #[cfg(all(feature = "gateway", feature = "entra-auth"))]
            avd_sso_logon_token: self.avd_sso_logon_token,
            #[cfg(feature = "vmconnect")]
            vm_id: self.vm_id,
            #[cfg(feature = "vmconnect")]
            vmconnect_mode: self.vmconnect_mode,
            #[cfg(all(windows, feature = "vmconnect"))]
            vmconnect_current_user: self.vmconnect_current_user,
            kerberos_config,
            remote_guard_mode: self.remote_guard_mode,
            input_send_interval: self.input_send_interval,
            input_keepalive_interval: self.input_keepalive_interval,
            fake_events_interval: self.fake_events_interval,
            tcp_connect_attempt_timeout: self.tcp_connect_attempt_timeout,
            channels: self.channels,
            administrative_session: self.administrative_session,
            load_balance_info: self.load_balance_info,
            use_redirection_server_name: self.use_redirection_server_name,
            redirection: None,
            #[cfg(any(feature = "sound", feature = "rdpdr"))]
            audio_quality_mode: self.audio_quality_mode.unwrap_or_default(),
            rail_client_status_flags: self.rail_client_status_flags,
            rail_system_parameters: self.rail_system_parameters,
            rail_initial_execute,
            #[cfg(feature = "dvc-pipe-proxy")]
            dvc_pipe_proxies: self.dvc_pipe_proxies,
            #[cfg(all(windows, feature = "dvc-com-plugin"))]
            dvc_plugins: self.dvc_plugins,
            #[cfg(all(windows, feature = "webauthn"))]
            webauthn_parent_hwnd: self.webauthn_parent_hwnd,
            properties,
            extensions: self.extensions,
        })
    }

    /// Build a [`Config`] from a `.rdp` [`PropertySet`], leaving anything not expressible as a
    /// property unset (query [`missing`](Self::missing) to resolve the rest).
    pub fn from_property_set(ps: &PropertySet) -> anyhow::Result<Self> {
        ConfigBuilder::new().with_property_set(ps)
    }

    /// Applies companion metadata from a sibling `<rdp_file>.json` file (such as Windows App / AVD
    /// launch files) onto `properties` when not already set in the `.rdp` file.
    pub fn apply_rdp_launch_companion(rdp_file: &std::path::Path, properties: &mut PropertySet) {
        #[cfg(any(feature = "cloud", feature = "entra-auth"))]
        if properties.username().is_none() {
            let mut added_os = rdp_file.as_os_str().to_os_string();
            added_os.push(".json");
            let added_json = PathBuf::from(added_os);
            if let Ok(json_text) = std::fs::read_to_string(&added_json)
                .or_else(|_| std::fs::read_to_string(rdp_file.with_extension("json")))
                && let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_text)
                && let Some(user) = val.get("UserName").and_then(serde_json::Value::as_str)
                && !user.is_empty()
            {
                properties.set_username(user.to_owned());
            }
        }
        #[cfg(not(any(feature = "cloud", feature = "entra-auth")))]
        let _ = (rdp_file, properties);
    }

    /// Overlay a `.rdp` [`PropertySet`] on top of the current builder.
    ///
    /// Only properties present in `ps` set values, so this can be layered:
    /// `explicit setters → PropertySet → more setters`, last writer wins. Resolution rules:
    /// `full address` beats `alternate full address`, an embedded port beats `server port`, and
    /// transport precedence is RDCleanPath > Gateway > Direct.
    pub fn with_property_set(mut self, ps: &PropertySet) -> anyhow::Result<Self> {
        #[cfg(feature = "gateway")]
        use ironrdp_cfg::GatewayUsageMethod;
        use ironrdp_cfg::{AudioCaptureMode, AudioMode, TargetHost};

        self.properties.merge(ps);

        #[cfg(all(feature = "gateway", feature = "entra-auth"))]
        {
            if let Some(token) = ps.get::<&str>("ironrdp_avd_gateway_token") {
                self.avd_gateway_token = (!token.is_empty()).then(|| zeroize::Zeroizing::new(token.to_owned()));
            }
            if let Some(token) = ps.get::<&str>("ironrdp_avd_sso_logon_token") {
                self.avd_sso_logon_token = (!token.is_empty()).then(|| zeroize::Zeroizing::new(token.to_owned()));
            }
        }

        let target = ps.full_address().context("invalid 'full address'")?.or(ps
            .alternate_full_address()
            .context("invalid 'alternate full address'")?);
        if let Some(target) = target {
            let port = target.port.or(ps.server_port().context("invalid 'server port'")?);
            let name = match target.host {
                TargetHost::Ip(ip) => ip.to_string(),
                TargetHost::Domain(host) => host,
            };
            self.destination = Some(Destination { name, port });
        }

        if let Some(username) = ps.username() {
            self.username = Some(username.to_owned());
        }
        if let Some(password) = ps.clear_text_password() {
            self.password = Some(password.to_owned());
        }
        if let Some(domain) = ps.domain() {
            self.domain = Some(domain.to_owned());
        }
        if let Some(enable_credssp) = ps.enable_credssp_support() {
            self.enable_credssp = Some(enable_credssp);
        }
        if let Some(mode) = ps.remote_guard_mode()? {
            self.remote_guard_mode = mode;
        }
        if let Some(enable_tls) = ps.enable_tls() {
            self.enable_tls = Some(enable_tls);
        }
        if let Some(server_pointer) = ps.server_pointer() {
            self.enable_server_pointer = Some(server_pointer);
        }
        if let Some(autologon) = ps.autologon() {
            self.autologon = Some(autologon);
        }
        if let Some(administrative_session) = ps.get::<bool>("administrative session") {
            self.administrative_session = administrative_session;
        }
        if let Some(use_redirection_server_name) = ps.get::<bool>("use redirection server name") {
            self.use_redirection_server_name = use_redirection_server_name;
        }
        if let Some(load_balance_info) = ps.get::<&str>("loadbalanceinfo") {
            let normalized = load_balance_info.strip_suffix("\r\n").unwrap_or(load_balance_info);
            self.load_balance_info = if normalized.is_empty() {
                None
            } else {
                Some(normalized.to_owned())
            };
        }
        if let Some(scale) = ps.desktop_scale_factor().ok().flatten() {
            self.desktop_scale_factor = Some(scale);
        }
        if let Some(scale) = ps.get::<i64>("devicescalefactor") {
            self.device_scale_factor = Some(match scale {
                100 => 100,
                140 => 140,
                180 => 180,
                _ => anyhow::bail!("invalid 'devicescalefactor': valid values are 100, 140 and 180"),
            });
        }
        if let Some(connection_type) = ps.get::<i64>("connection type") {
            use ironrdp_pdu::gcc::ConnectionType;

            self.connection_type = Some(
                u8::try_from(connection_type)
                    .ok()
                    .and_then(ConnectionType::from_specified)
                    .ok_or_else(|| anyhow::anyhow!("invalid 'connection type': valid values are 1 through 7"))?,
            );
        }
        if let Some(width) = ps.desktop_width().ok().flatten() {
            self.desktop_width = Some(width);
        }
        if let Some(height) = ps.desktop_height().ok().flatten() {
            self.desktop_height = Some(height);
        }
        if let Some(shell) = ps.alternate_shell() {
            self.alternate_shell = Some(shell.to_owned());
        }
        if let Some(dir) = ps.shell_working_directory() {
            self.work_dir = Some(dir.to_owned());
        }
        #[cfg(feature = "vmconnect")]
        if let Some(vm_id) = ps.vmconnect_id() {
            self.vm_id = Some(vm_id.to_owned());
        }
        #[cfg(feature = "vmconnect")]
        if let Some(basic) = ps.vmconnect_basic() {
            self.vmconnect_mode = if basic {
                VmConnectMode::Basic
            } else {
                VmConnectMode::Enhanced
            };
        }
        #[cfg(all(windows, feature = "vmconnect"))]
        if let Some(current_user) = ps.vmconnect_current_user() {
            self.vmconnect_current_user = current_user;
        }
        if let Some(remote_application_mode) = ps.remote_application_mode() {
            self.remote_application_mode = Some(remote_application_mode);
        }
        if let Some(program) = ps.remote_application_program() {
            self.remote_application_program = Some(program.to_owned());
        }
        if let Some(minutes) = ps.fake_events_interval() {
            self.fake_events_interval = Some(Duration::from_secs(u64::from(minutes) * 60));
        }
        if let Some(milliseconds) = ps.get::<u32>("mininputsendinterval") {
            if milliseconds > 2_000 {
                anyhow::bail!("invalid minimum input send interval: valid values are 0 through 2000 milliseconds");
            }
            self.input_send_interval = Some(Duration::from_millis(u64::from(milliseconds)));
        }
        if let Some(seconds) = ps.get::<u32>("keepaliveinterval") {
            self.input_keepalive_interval = (seconds != 0).then(|| Duration::from_secs(u64::from(seconds)));
        }
        if let Some(level) = ps.compression_level() {
            self.compression_type = Some(compression_type_from_level(level)?);
        }
        if let Some(enabled) = ps.compression() {
            self.compression_enabled = Some(enabled);
        }
        // IronRDP renders 16 or 32 bits per pixel: `session bpp` selects the closest of the two,
        // and the IronRDP-specific color depth, when present, takes precedence.
        if let Some(bpp) = ps.get::<i64>("session bpp") {
            self.color_depth = Some(match bpp {
                8 | 15 | 16 => 16,
                24 | 32 => 32,
                _ => anyhow::bail!("invalid 'session bpp': valid values are 8, 15, 16, 24 and 32"),
            });
        }
        if let Some(depth) = ps.color_depth() {
            self.color_depth = Some(depth);
        }
        #[cfg(feature = "sound")]
        match ps.audio_mode() {
            Ok(Some(AudioMode::RedirectToClient)) => {
                self.enable_audio_playback = Some(true);
                self.channels.sound = true;
            }
            Ok(Some(AudioMode::PlayOnServer | AudioMode::Disabled)) => {
                self.enable_audio_playback = Some(false);
                self.channels.sound = false;
            }
            _ => {}
        }
        #[cfg(not(feature = "sound"))]
        match ps.audio_mode() {
            Ok(Some(AudioMode::PlayOnServer | AudioMode::Disabled)) => self.enable_audio_playback = Some(false),
            Ok(Some(AudioMode::RedirectToClient)) => self.enable_audio_playback = Some(true),
            _ => {}
        }
        #[cfg(feature = "sound")]
        match ps.audio_capture_mode() {
            Ok(Some(AudioCaptureMode::CaptureFromClient)) => {
                self.enable_audio_capture = Some(true);
                self.channels.audio_capture = true;
            }
            Ok(Some(AudioCaptureMode::Disabled)) => {
                self.enable_audio_capture = Some(false);
                self.channels.audio_capture = false;
            }
            _ => {}
        }
        #[cfg(not(feature = "sound"))]
        match ps.audio_capture_mode() {
            Ok(Some(AudioCaptureMode::CaptureFromClient)) => self.enable_audio_capture = Some(true),
            Ok(Some(AudioCaptureMode::Disabled)) => self.enable_audio_capture = Some(false),
            _ => {}
        }
        #[cfg(feature = "sound")]
        {
            if let Some(device) = ps.get::<&str>(crate::audio::OUTPUT_DEVICE_PROPERTY) {
                self.channels.audio.output_device = Some(device.to_owned());
            }
            if let Some(device) = ps.get::<&str>(crate::audio::INPUT_DEVICE_PROPERTY) {
                self.channels.audio.input_device = Some(device.to_owned());
            }
            if let Some(latency) = ps.get::<i64>(crate::audio::LATENCY_PROPERTY) {
                let latency = u64::try_from(latency)
                    .map_err(|_| anyhow::anyhow!("invalid '{}': {latency}", crate::audio::LATENCY_PROPERTY))?;
                self.channels.audio.latency = Some(Duration::from_millis(latency));
            }
        }
        if let Some(enabled) = ps.get::<i64>("ironrdp_egfx") {
            self.graphics_pipeline = Some(enabled != 0);
        }
        // Off unless asked for, unlike mstsc. `1` asks for the per-user cache mstsc also keeps, at
        // the default location unless a path was already chosen; `0` turns it off.
        match ps.get::<i64>("bitmapcachepersistenable") {
            Some(0) => self.channels.egfx_persistent_cache = None,
            Some(_) if self.channels.egfx_persistent_cache.is_none() => {
                self.channels.egfx_persistent_cache = crate::egfx_cache::default_cache_path();
            }
            _ => {}
        }
        #[cfg(feature = "video")]
        if ps.get::<i64>("videoplaybackmode") == Some(0) {
            self.channels.video.enabled = false;
        }
        #[cfg(feature = "camera")]
        if let Some(cameras) = ps.get::<&str>("camerastoredirect") {
            self.channels.cameras.apply_rdp_value(cameras);
        }
        #[cfg(any(feature = "sound", feature = "rdpdr"))]
        if let Some(audio_quality_mode) = ps.get::<i64>("audioqualitymode") {
            self.audio_quality_mode = Some(match audio_quality_mode {
                0 => AudioQualityMode::Dynamic,
                1 => AudioQualityMode::Medium,
                2 => AudioQualityMode::High,
                _ => anyhow::bail!("invalid 'audioqualitymode': {audio_quality_mode}"),
            });
        }

        // Transport: NamedPipe > RDCleanPath > Gateway > Direct.
        #[cfg(windows)]
        if let Some(path) = ps.named_pipe() {
            self.transport = TransportKind::NamedPipe { path: path.to_owned() };
        } else if let Some((url, token)) = ps.rdcleanpath_url().zip(ps.rdcleanpath_token()) {
            let url = Url::parse(url).context("invalid 'ironrdp_rdcleanpathurl'")?;
            self.transport = TransportKind::RDCleanPath { url };
            self.rdcleanpath_token = Some(token.to_owned());
        } else {
            #[cfg(feature = "gateway")]
            {
                let gateway_usage = ps
                    .gateway_usage_method()
                    .context("invalid Gateway usage method")?
                    .unwrap_or_default();

                let gateway_hostname = ps.gateway_hostname();

                let select_gateway_transport = match gateway_usage {
                    // Explicit gateway use.
                    GatewayUsageMethod::UseAlways => Some(false),

                    // Try direct first; fall back to gateway when a hostname is configured.
                    GatewayUsageMethod::Detect if gateway_hostname.is_some() => Some(true),
                    GatewayUsageMethod::Detect => None,

                    // IronRDP does not currently resolve MSTSC/client/GPO default gateway policy.
                    GatewayUsageMethod::UseDefaultSettings => None,

                    // Explicit no-gateway modes.
                    GatewayUsageMethod::Direct | GatewayUsageMethod::DirectBypassLocal => None,
                };

                if let Some(prefer_direct) = select_gateway_transport {
                    let endpoint = gateway_hostname.context("missing Gateway hostname")?;

                    self.transport = gateway_transport_kind(ps, endpoint, prefer_direct)?;

                    if let Some(user) = ps.gateway_username() {
                        self.gateway_username = Some(user.to_owned());
                    }

                    if let Some(pass) = ps.gateway_password() {
                        self.gateway_password = Some(pass.to_owned());
                    }
                }
            }
        }
        #[cfg(not(windows))]
        if let Some((url, token)) = ps.rdcleanpath_url().zip(ps.rdcleanpath_token()) {
            let url = Url::parse(url).context("invalid 'ironrdp_rdcleanpathurl'")?;
            self.transport = TransportKind::RDCleanPath { url };
            self.rdcleanpath_token = Some(token.to_owned());
        } else {
            #[cfg(feature = "gateway")]
            {
                let gateway_usage = ps
                    .gateway_usage_method()
                    .context("invalid Gateway usage method")?
                    .unwrap_or_default();

                let gateway_hostname = ps.gateway_hostname();

                let select_gateway_transport = match gateway_usage {
                    GatewayUsageMethod::UseAlways => Some(false),
                    GatewayUsageMethod::Detect if gateway_hostname.is_some() => Some(true),
                    GatewayUsageMethod::Detect => None,
                    GatewayUsageMethod::UseDefaultSettings => None,
                    GatewayUsageMethod::Direct | GatewayUsageMethod::DirectBypassLocal => None,
                };

                if let Some(prefer_direct) = select_gateway_transport {
                    let endpoint = gateway_hostname.context("missing Gateway hostname")?;

                    self.transport = gateway_transport_kind(ps, endpoint, prefer_direct)?;

                    if let Some(user) = ps.gateway_username() {
                        self.gateway_username = Some(user.to_owned());
                    }

                    if let Some(pass) = ps.gateway_password() {
                        self.gateway_password = Some(pass.to_owned());
                    }
                }
            }
        }

        if let Some(redirect) = ps.redirect_clipboard() {
            #[cfg(feature = "clipboard")]
            {
                self.channels.clipboard = if redirect {
                    ClipboardType::Enable
                } else {
                    ClipboardType::Disable
                };
            }
            let _ = redirect;
        }
        if let Some(redirect) = ps.redirect_webauthn() {
            #[cfg(feature = "webauthn")]
            {
                self.channels.webauthn = redirect;
            }
            let _ = redirect;
        }
        #[cfg(feature = "location")]
        if let Some(redirect) = ps.get::<bool>("redirectlocation") {
            self.channels.location = redirect;
        }

        #[cfg(feature = "rdpdr")]
        if let Some(enabled) = ps.enable_rdpdr() {
            self.channels.rdpdr.enabled = enabled;
        }
        #[cfg(feature = "rdpdr")]
        if let Some(drives) = ps.drive_redirection() {
            self.channels.rdpdr.drives = drives;
        }
        #[cfg(feature = "smartcard")]
        if let Some(enabled) = ps.get::<bool>("redirectsmartcards") {
            self.channels.rdpdr.smartcard = enabled;
        }
        #[cfg(feature = "smartcard")]
        if let Some(enabled) = ps.enable_smartcard() {
            self.channels.rdpdr.smartcard = enabled;
        }
        #[cfg(feature = "rdpdr")]
        if let Some(enabled) = ps.redirect_com_ports() {
            self.channels.rdpdr.ports = enabled;
        }
        #[cfg(feature = "rdpdr")]
        if let Some(enabled) = ps.redirect_printers() {
            self.channels.rdpdr.printers = enabled;
        }
        #[cfg(feature = "qoi")]
        if let Some(enabled) = ps.enable_qoi() {
            self.channels.qoi = enabled;
        }
        #[cfg(feature = "qoiz")]
        if let Some(enabled) = ps.enable_qoiz() {
            self.channels.qoiz = enabled;
        }

        #[cfg(feature = "usb")]
        if let Some(devices) = ps.usb_devices_to_redirect() {
            let usb = ironrdp_rdpeusb_native::UsbRedirectionConfig::parse_rdp(devices)
                .context("invalid usbdevicestoredirect property")?;
            self.channels.usb = usb.is_enabled().then_some(usb);
        }

        #[cfg(feature = "dvc-pipe-proxy")]
        for (idx, proxy) in ps.dvc_pipe_proxies().enumerate() {
            let proxy = proxy.with_context(|| format!("invalid DVC pipe proxy spec at idx {idx}"))?;
            self.dvc_pipe_proxies.push(DvcProxyInfo {
                channel_name: proxy.channel_name,
                pipe_name: proxy.pipe_name,
            });
        }

        #[cfg(all(windows, feature = "dvc-com-plugin"))]
        self.dvc_plugins.extend(ps.dvc_plugins());

        Ok(self)
    }
}

/// Map a bulk-compression level (0–3) to the corresponding [`CompressionType`].
///
/// 0 = MPPC 8K (RDP4), 1 = MPPC 64K (RDP5), 2 = NCRUSH (RDP6), 3 = XCRUSH (RDP6.1).
///
/// [`CompressionType`]: ironrdp_pdu::rdp::client_info::CompressionType
fn compression_type_from_level(level: u32) -> anyhow::Result<ironrdp_pdu::rdp::client_info::CompressionType> {
    use ironrdp_pdu::rdp::client_info::CompressionType;

    match level {
        0 => Ok(CompressionType::K8),
        1 => Ok(CompressionType::K64),
        2 => Ok(CompressionType::Rdp6),
        3 => Ok(CompressionType::Rdp61),
        _ => anyhow::bail!("invalid compression level: valid values are 0, 1, 2, 3"),
    }
}

#[cfg(all(windows, feature = "vmconnect"))]
fn is_loopback_host(host: &str) -> bool {
    host.trim_end_matches('.').eq_ignore_ascii_case("localhost")
        || host
            .parse::<core::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn level_from_compression_type(ty: ironrdp_pdu::rdp::client_info::CompressionType) -> u32 {
    use ironrdp_pdu::rdp::client_info::CompressionType;

    match ty {
        CompressionType::K8 => 0,
        CompressionType::K64 => 1,
        CompressionType::Rdp6 => 2,
        CompressionType::Rdp61 => 3,
    }
}

/// Selects the gateway transport for a configured gateway endpoint.
///
/// Standard gateway properties keep selecting HTTP/WebSocket.
/// Authenticated RPCH requires the IronRDP-specific `ironrdp_gatewayrpch:i:1` opt-in.
#[cfg(feature = "gateway")]
fn gateway_transport_kind(ps: &PropertySet, endpoint: &str, prefer_direct: bool) -> anyhow::Result<TransportKind> {
    if ps.get::<&str>("ironrdp_gatewayrpch").is_some() {
        anyhow::bail!("invalid 'ironrdp_gatewayrpch': expected integer 0 or 1");
    }
    let endpoint = endpoint.to_owned();
    match ps.get::<i64>("ironrdp_gatewayrpch") {
        None | Some(0) => Ok(TransportKind::Gateway {
            endpoint,
            prefer_direct,
        }),
        Some(1) => Ok(TransportKind::GatewayRpch {
            endpoint,
            prefer_direct,
        }),
        Some(value) => anyhow::bail!("invalid 'ironrdp_gatewayrpch': {value}"),
    }
}

/// Derive a Kerberos/KDC-proxy config from `kdcproxyurl`/`kdcproxyname`, using `client_name` as the
/// SPN hostname. Returns `None` if no KDC proxy is configured or the URL is invalid.
fn kerberos_config_from_properties(
    ps: &PropertySet,
    client_name: &str,
) -> Option<ironrdp_connector::credssp::KerberosConfig> {
    use ironrdp_cfg::PropertySetExt as _;

    let kdc_proxy_url = ps.kdc_proxy_url().map(str::to_owned).or_else(|| {
        ps.kdc_proxy_name().map(|name| {
            if name.starts_with("http://") || name.starts_with("https://") {
                name.to_owned()
            } else {
                format!("https://{name}/KdcProxy")
            }
        })
    })?;

    Url::parse(&kdc_proxy_url)
        .ok()
        .map(|url| ironrdp_connector::credssp::KerberosConfig {
            kdc_resolution: KdcResolution::KdcUrl(Some(url)),
            hostname: client_name.to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use ironrdp_cfg::PropertySetExt as _;
    use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;

    use super::{ConfigBuilder, Destination};
    #[cfg(feature = "gateway")]
    use super::{MissingField, Transport, TransportKind};

    fn complete_builder() -> ConfigBuilder {
        ConfigBuilder::new()
            .with_destination(Destination::new("server.example:3389").unwrap())
            .with_username("user")
            .with_password("password")
            .with_client_build(1)
            .with_client_dir("C:\\")
            .with_platform(MajorPlatformType::WINDOWS)
            .with_client_name("client")
    }

    #[test]
    fn load_balance_info_enforces_x224_length_limit() {
        let maximum = "x".repeat(ironrdp_pdu::nego::MAX_ROUTING_TOKEN_LENGTH);
        assert!(complete_builder().with_load_balance_info(maximum).build().is_ok());

        let oversized = "x".repeat(ironrdp_pdu::nego::MAX_ROUTING_TOKEN_LENGTH + 1);
        assert!(complete_builder().with_load_balance_info(oversized).build().is_err());

        let cleared = complete_builder()
            .with_load_balance_info("\r\n")
            .build()
            .expect("terminator-only load-balance info clears the token");
        assert!(cleared.load_balance_info().is_none());
    }

    #[test]
    fn input_timing_configuration_preserves_subminute_precision() {
        let config = complete_builder()
            .with_input_send_interval(Duration::from_millis(100))
            .with_input_keepalive_interval(Duration::from_secs(5))
            .build()
            .expect("valid input timing configuration");

        assert_eq!(config.input_send_interval(), Some(Duration::from_millis(100)));
        assert_eq!(config.input_keepalive_interval(), Some(Duration::from_secs(5)));
        assert_eq!(config.fake_events_interval(), None);
        assert_eq!(config.properties.get::<u32>("ironrdp_fakeeventsinterval"), None);

        let mut properties = ironrdp_propertyset::PropertySet::new();
        properties.insert("mininputsendinterval", 2_000u32);
        properties.insert("keepaliveinterval", 30u32);
        let config = complete_builder()
            .with_property_set(&properties)
            .expect("valid input timing properties")
            .build()
            .expect("valid input timing configuration");
        assert_eq!(config.input_send_interval(), Some(Duration::from_secs(2)));
        assert_eq!(config.input_keepalive_interval(), Some(Duration::from_secs(30)));

        properties.insert("mininputsendinterval", 2_001u32);
        assert!(complete_builder().with_property_set(&properties).is_err());
    }

    #[test]
    fn build_rejects_display_and_input_values_outside_their_protocol_ranges() {
        assert!(complete_builder().with_device_scale_factor(140).build().is_ok());
        assert!(complete_builder().with_device_scale_factor(125).build().is_err());
        assert!(complete_builder().with_max_touch_contacts(256).build().is_ok());
        assert!(complete_builder().with_max_touch_contacts(0).build().is_err());
        assert!(complete_builder().with_max_touch_contacts(257).build().is_err());
        assert!(
            complete_builder()
                .with_tcp_connect_attempt_timeout(Duration::ZERO)
                .build()
                .is_err()
        );
    }

    #[test]
    fn tcp_connect_attempt_timeout_is_unset_by_default() {
        let config = complete_builder().build().expect("valid configuration");
        assert_eq!(config.tcp_connect_attempt_timeout(), None);

        let config = complete_builder()
            .with_tcp_connect_attempt_timeout(Duration::from_secs(8))
            .build()
            .expect("valid configuration");
        assert_eq!(config.tcp_connect_attempt_timeout(), Some(Duration::from_secs(8)));
    }

    #[cfg(any(feature = "sound", feature = "rdpdr"))]
    #[test]
    fn property_set_rejects_out_of_range_audio_quality() {
        let mut properties = ironrdp_propertyset::PropertySet::new();
        properties.insert("audioqualitymode", -1i64);
        assert!(ConfigBuilder::from_property_set(&properties).is_err());
    }

    #[cfg(feature = "gateway")]
    fn gateway_builder() -> ConfigBuilder {
        complete_builder().with_transport(TransportKind::Gateway {
            endpoint: "gateway.example:443".to_owned(),
            prefer_direct: false,
        })
    }

    #[cfg(feature = "gateway")]
    #[test]
    fn rpch_gateway_requires_explicit_selection_and_preserves_legacy_default() {
        let legacy = gateway_builder()
            .with_gateway_username("gateway-user")
            .with_gateway_password("gateway-password")
            .build()
            .expect("valid legacy gateway configuration");
        assert!(matches!(legacy.transport(), Transport::Gateway(_)));

        let rpch = complete_builder()
            .with_transport(TransportKind::GatewayRpch {
                endpoint: "gateway.example:443".to_owned(),
                prefer_direct: false,
            })
            .with_gateway_username("gateway-user")
            .with_gateway_password("gateway-password")
            .build()
            .expect("valid explicit RPCH gateway configuration");
        // oracle: regression #2202 (explicit RPCH selection must retain its exported opt-in).
        assert_eq!(rpch.properties().get::<i64>("ironrdp_gatewayrpch"), Some(1));
        let Transport::GatewayRpch(gateway) = rpch.transport() else {
            panic!("explicit RPCH gateway transport expected");
        };
        assert_eq!(gateway.endpoint, "gateway.example:443");
        assert_eq!(gateway.username, "gateway-user");
        assert_eq!(gateway.password, "gateway-password");
        assert!(!gateway.prefer_direct);
    }

    #[cfg(feature = "gateway")]
    fn gateway_test_properties() -> ironrdp_propertyset::PropertySet {
        let mut properties = ironrdp_propertyset::PropertySet::new();
        properties.insert("gatewayhostname", "gateway.example:443");
        properties.insert("gatewayusagemethod", 1i64);
        properties
    }

    #[cfg(feature = "gateway")]
    #[test]
    fn rpch_gateway_absent_property_selects_http() {
        // oracle: regression #2202: standard gateway properties preserve the HTTP/WebSocket default.
        let properties = gateway_test_properties();
        let legacy = complete_builder()
            .with_property_set(&properties)
            .expect("valid gateway properties");
        assert!(matches!(legacy.transport, TransportKind::Gateway { .. }));
    }

    #[cfg(feature = "gateway")]
    #[test]
    fn rpch_gateway_zero_property_selects_http() {
        // oracle: regression #2202: integer zero disables the RPCH opt-in.
        let mut properties = gateway_test_properties();
        properties.insert("ironrdp_gatewayrpch", 0i64);
        let builder = complete_builder()
            .with_property_set(&properties)
            .expect("valid disabled RPCH property");
        assert!(matches!(builder.transport, TransportKind::Gateway { .. }));
    }

    #[cfg(feature = "gateway")]
    #[test]
    fn rpch_gateway_one_property_selects_rpch() {
        // oracle: regression #2202: integer one explicitly selects RPCH.
        let mut properties = gateway_test_properties();
        properties.insert("ironrdp_gatewayrpch", 1i64);
        let rpch = complete_builder()
            .with_property_set(&properties)
            .expect("valid RPCH gateway properties");
        assert!(matches!(
            rpch.transport,
            TransportKind::GatewayRpch { ref endpoint, prefer_direct: false } if endpoint == "gateway.example:443"
        ));
    }

    #[cfg(feature = "gateway")]
    #[test]
    fn rpch_gateway_rejects_other_integer_properties() {
        // oracle: regression #2202: only integer zero and one are valid opt-in values.
        for value in [-1, 2, i64::MIN, i64::MAX] {
            let mut properties = gateway_test_properties();
            properties.insert("ironrdp_gatewayrpch", value);
            assert!(
                complete_builder().with_property_set(&properties).is_err(),
                "invalid value: {value}"
            );
        }
    }

    #[cfg(feature = "gateway")]
    #[test]
    fn rpch_gateway_rejects_string_properties() {
        // oracle: regression #2202: the opt-in property uses the RDP integer type, not a string.
        for value in ["", "0", "1"] {
            let mut properties = gateway_test_properties();
            properties.insert("ironrdp_gatewayrpch", value);
            assert!(
                complete_builder().with_property_set(&properties).is_err(),
                "invalid string: {value:?}"
            );
        }
    }

    // oracle: regression #2202 (exported direct settings must not retain the RPCH opt-in).
    #[cfg(feature = "gateway")]
    #[test]
    fn direct_transport_clears_rpch_opt_in() {
        let config = complete_builder()
            .with_transport(TransportKind::GatewayRpch {
                endpoint: "gateway.example:443".to_owned(),
                prefer_direct: false,
            })
            .with_transport(TransportKind::Direct)
            .build()
            .expect("valid direct configuration");

        assert!(matches!(config.transport(), Transport::Direct));
        assert_eq!(config.properties().get::<i64>("ironrdp_gatewayrpch"), None);

        let mut properties = config.properties().clone();
        properties.set_gateway_hostname("new-gateway.example:443");
        properties.set_gateway_usage_method(ironrdp_cfg::GatewayUsageMethod::UseAlways);
        let reloaded = complete_builder()
            .with_property_set(&properties)
            .expect("valid merged gateway properties")
            .with_gateway_username("gateway-user")
            .with_gateway_password("gateway-password")
            .build()
            .expect("valid merged gateway configuration");
        assert!(matches!(reloaded.transport(), Transport::Gateway(_)));
    }

    // oracle: regression #2202 (exported RDCleanPath settings must not retain the RPCH opt-in).
    #[cfg(feature = "gateway")]
    #[test]
    fn rdcleanpath_transport_clears_rpch_opt_in() {
        let config = complete_builder()
            .with_transport(TransportKind::GatewayRpch {
                endpoint: "gateway.example:443".to_owned(),
                prefer_direct: false,
            })
            .with_transport(TransportKind::RDCleanPath {
                url: "wss://proxy.example".parse().expect("valid RDCleanPath URL"),
            })
            .with_rdcleanpath_token("token")
            .build()
            .expect("valid RDCleanPath configuration");

        assert!(matches!(config.transport(), Transport::RDCleanPath(_)));
        assert_eq!(config.properties().get::<i64>("ironrdp_gatewayrpch"), None);
    }

    // oracle: regression #2202 (exported named-pipe settings must not retain the RPCH opt-in).
    #[cfg(all(windows, feature = "gateway"))]
    #[test]
    fn named_pipe_transport_clears_rpch_opt_in() {
        let config = complete_builder()
            .with_transport(TransportKind::GatewayRpch {
                endpoint: "gateway.example:443".to_owned(),
                prefer_direct: false,
            })
            .with_transport(TransportKind::NamedPipe {
                path: r"\\.\pipe\rdp".to_owned(),
            })
            .build()
            .expect("valid named-pipe configuration");

        assert!(matches!(config.transport(), Transport::NamedPipe { .. }));
        assert_eq!(config.properties().get::<i64>("ironrdp_gatewayrpch"), None);
    }

    // oracle: regression #2202 (exported HTTP gateway settings must not retain the RPCH opt-in).
    #[cfg(feature = "gateway")]
    #[test]
    fn http_gateway_transport_clears_rpch_opt_in() {
        let config = complete_builder()
            .with_transport(TransportKind::GatewayRpch {
                endpoint: "gateway.example:443".to_owned(),
                prefer_direct: false,
            })
            .with_transport(TransportKind::Gateway {
                endpoint: "gateway.example:443".to_owned(),
                prefer_direct: false,
            })
            .with_gateway_username("gateway-user")
            .with_gateway_password("gateway-password")
            .build()
            .expect("valid HTTP gateway configuration");

        assert!(matches!(config.transport(), Transport::Gateway(_)));
        assert_eq!(config.properties().get::<i64>("ironrdp_gatewayrpch"), None);
    }

    #[cfg(feature = "gateway")]
    #[test]
    fn rpch_gateway_resolves_server_credentials() {
        let mut properties = ironrdp_propertyset::PropertySet::new();
        properties.set_gateway_credentials_source(ironrdp_cfg::GatewayCredentialsSource::UseServerCredentials);
        let config = complete_builder()
            .with_transport(TransportKind::GatewayRpch {
                endpoint: "gateway.example:443".to_owned(),
                prefer_direct: false,
            })
            .with_property_set(&properties)
            .expect("valid gateway credentials source")
            .with_domain("CONTOSO")
            .build()
            .expect("valid RPCH server credential configuration");

        let Transport::GatewayRpch(gateway) = config.transport() else {
            panic!("explicit RPCH gateway transport expected");
        };
        assert_eq!(gateway.username, "CONTOSO\\user");
        assert_eq!(gateway.password, "password");
    }

    #[cfg(feature = "gateway")]
    fn with_gateway_credentials_source(
        builder: ConfigBuilder,
        source: ironrdp_cfg::GatewayCredentialsSource,
    ) -> ConfigBuilder {
        let mut properties = ironrdp_propertyset::PropertySet::new();
        properties.set_gateway_credentials_source(source);
        builder
            .with_property_set(&properties)
            .expect("valid gateway credentials source")
    }

    #[cfg(feature = "gateway")]
    #[test]
    fn gateway_server_credentials_fall_back_to_rdp_credentials() {
        let config = with_gateway_credentials_source(
            gateway_builder(),
            ironrdp_cfg::GatewayCredentialsSource::UseServerCredentials,
        )
        .build()
        .expect("valid server credential configuration");

        let Transport::Gateway(gateway) = config.transport() else {
            panic!("gateway transport expected");
        };
        assert_eq!(gateway.username, "user");
        assert_eq!(gateway.password, "password");
    }

    #[cfg(feature = "gateway")]
    #[test]
    fn gateway_server_credentials_qualify_bare_username_with_domain() {
        let config = with_gateway_credentials_source(
            gateway_builder().with_domain("CONTOSO"),
            ironrdp_cfg::GatewayCredentialsSource::UseServerCredentials,
        )
        .build()
        .expect("valid server credential configuration");

        let Transport::Gateway(gateway) = config.transport() else {
            panic!("gateway transport expected");
        };
        assert_eq!(gateway.username, "CONTOSO\\user");
    }

    #[cfg(feature = "gateway")]
    #[test]
    fn explicit_gateway_credentials_take_precedence_over_server_credentials() {
        let config = with_gateway_credentials_source(
            gateway_builder()
                .with_gateway_username("gateway-user")
                .with_gateway_password("gateway-password"),
            ironrdp_cfg::GatewayCredentialsSource::UseServerCredentials,
        )
        .build()
        .expect("valid server credential configuration");

        let Transport::Gateway(gateway) = config.transport() else {
            panic!("gateway transport expected");
        };
        assert_eq!(gateway.username, "gateway-user");
        assert_eq!(gateway.password, "gateway-password");
    }

    #[cfg(feature = "gateway")]
    #[test]
    fn gateway_user_credentials_require_explicit_credentials() {
        let builder = with_gateway_credentials_source(
            gateway_builder(),
            ironrdp_cfg::GatewayCredentialsSource::UseUserCredentials,
        );

        assert_eq!(
            builder.missing(),
            [MissingField::GatewayUsername, MissingField::GatewayPassword]
        );
        assert_eq!(
            builder
                .build()
                .expect_err("gateway credentials must be required")
                .to_string(),
            "missing required configuration: gateway username, gateway password"
        );
    }

    #[cfg(feature = "gateway")]
    #[test]
    fn gateway_user_credentials_select_explicit_credentials() {
        let config = with_gateway_credentials_source(
            gateway_builder()
                .with_gateway_username("gateway-user")
                .with_gateway_password("gateway-password"),
            ironrdp_cfg::GatewayCredentialsSource::UseUserCredentials,
        )
        .build()
        .expect("valid user credential configuration");

        let Transport::Gateway(gateway) = config.transport() else {
            panic!("gateway transport expected");
        };
        assert_eq!(gateway.username, "gateway-user");
        assert_eq!(gateway.password, "gateway-password");
    }

    #[cfg(feature = "gateway")]
    #[test]
    fn unavailable_gateway_credential_sources_are_rejected() {
        for (source, expected) in [
            (
                ironrdp_cfg::GatewayCredentialsSource::UseProfile,
                "gateway credential source UseProfile requires an unavailable profile credential provider",
            ),
            (
                ironrdp_cfg::GatewayCredentialsSource::Prompt,
                "gateway credential source Prompt requires an unavailable interactive credential prompt provider",
            ),
            (
                ironrdp_cfg::GatewayCredentialsSource::SmartCard,
                "gateway credential source SmartCard requires an unavailable smart-card credential provider",
            ),
            (
                ironrdp_cfg::GatewayCredentialsSource::UseLogonCredentials,
                "gateway credential source UseLogonCredentials requires an unavailable OS logon credential provider",
            ),
        ] {
            let error = with_gateway_credentials_source(
                gateway_builder()
                    .with_gateway_username("gateway-user")
                    .with_gateway_password("gateway-password"),
                source,
            )
            .build()
            .expect_err("unavailable credential source must fail");

            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn remote_application_program_queues_rail_execute_and_clears_client_info_shell() {
        let mut properties = ironrdp_propertyset::PropertySet::new();
        properties.set_remote_application_mode(true);
        properties.set_remote_application_program("notepad.exe");
        properties.set_alternate_shell("fallback.exe");
        properties.set_shell_working_directory("C:\\Temp");

        let config = complete_builder()
            .with_property_set(&properties)
            .expect("valid properties")
            .build()
            .expect("valid RemoteApp configuration");

        assert!(config.connector().alternate_shell.is_empty());
        assert!(config.connector().work_dir.is_empty());
        let execute = config.rail_initial_execute.expect("initial Execute PDU");
        assert_eq!(execute.executable, "notepad.exe");
        assert_eq!(execute.working_directory, "C:\\Temp");
    }

    #[cfg(feature = "webauthn")]
    #[test]
    fn with_webauthn_updates_the_channel_and_property() {
        let builder = ConfigBuilder::new().with_webauthn(false);

        assert!(!builder.channels.webauthn);
        assert!(!builder.properties.redirect_webauthn().unwrap());
    }

    #[cfg(feature = "webauthn")]
    #[test]
    fn property_set_enables_webauthn_redirection() {
        let mut properties = ironrdp_propertyset::PropertySet::new();
        properties.set_redirect_webauthn(true);

        let builder = ConfigBuilder::from_property_set(&properties).expect("valid WebAuthn property");
        assert!(builder.channels.webauthn);
        assert!(builder.properties.redirect_webauthn().unwrap());
    }

    #[cfg(feature = "webauthn")]
    #[test]
    fn property_set_disables_webauthn_redirection() {
        let mut properties = ironrdp_propertyset::PropertySet::new();
        properties.set_redirect_webauthn(false);

        let builder = ConfigBuilder::from_property_set(&properties).expect("valid WebAuthn property");
        assert!(!builder.channels.webauthn);
        assert!(!builder.properties.redirect_webauthn().unwrap());
    }
}
