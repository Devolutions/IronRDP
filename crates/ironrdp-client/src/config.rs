use core::fmt;
use core::str::FromStr;
use core::time::Duration;
#[cfg(all(windows, feature = "dvc-com-plugin"))]
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use url::Url;

// ── Extension registry ────────────────────────────────────────────────────────

type StaticChannelFn = Arc<dyn Fn(&mut ironrdp_connector::ClientConnector) + Send + Sync>;
type DvcChannelFn = Arc<dyn Fn(&mut ironrdp_dvc::DrdynvcClient) + Send + Sync>;

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

/// Fully resolved client configuration.
///
/// This is the typed surface consumed by [`crate::rdp::RdpClient`]. Build it with
/// [`ConfigBuilder`]; producing a `Config` from CLI arguments, `.rdp` files, or interactive
/// prompts is the consumer's responsibility (see `ironrdp-viewer` for a reference front-end).
#[derive(Clone)]
#[expect(
    clippy::partial_pub_fields,
    reason = "extensions must stay crate-private because its type ExtensionRegistry is pub(crate)"
)]
pub struct Config {
    pub connector: ironrdp_connector::Config,
    pub destination: Destination,
    pub transport: Transport,
    pub kerberos_config: Option<ironrdp_connector::credssp::KerberosConfig>,
    pub log_file: Option<String>,
    pub fake_events_interval: Option<Duration>,
    pub channels: ChannelConfig,

    /// DVC channel ↔ named-pipe proxy configuration.
    ///
    /// Each entry causes IronRDP to forward that DVC channel's traffic to/from the
    /// named pipe, allowing out-of-process DVC logic.
    #[cfg(feature = "dvc-pipe-proxy")]
    pub dvc_pipe_proxies: Vec<DvcProxyInfo>,

    /// Paths to DVC client plugin DLLs to load (Windows only).
    ///
    /// Each DLL is loaded via `LoadLibraryW` and its `VirtualChannelGetInstance` export is
    /// called to obtain DVC plugin COM objects.  Example: `C:\Windows\System32\webauthn.dll`.
    #[cfg(all(windows, feature = "dvc-com-plugin"))]
    pub dvc_plugins: Vec<PathBuf>,

    pub(crate) extensions: ExtensionRegistry,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("Config");
        s.field("connector", &self.connector);
        s.field("destination", &self.destination);
        s.field("transport", &self.transport);
        s.field("kerberos_config", &self.kerberos_config);
        s.field("log_file", &self.log_file);
        s.field("fake_events_interval", &self.fake_events_interval);
        s.field("channels", &self.channels);
        #[cfg(feature = "dvc-pipe-proxy")]
        s.field("dvc_pipe_proxies", &self.dvc_pipe_proxies);
        #[cfg(all(windows, feature = "dvc-com-plugin"))]
        s.field("dvc_plugins", &self.dvc_plugins);
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
    Stub,
}

/// Channel and codec runtime toggles.
///
/// Each field is only present when the corresponding Cargo feature is enabled.
/// The defaults for all optional fields are `true` (enabled) when the feature is on.
#[derive(Clone, Debug)]
pub struct ChannelConfig {
    /// Enable the RDPSND (audio) virtual channel.
    #[cfg(feature = "sound")]
    pub sound: bool,

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
}

#[cfg_attr(
    not(any(feature = "sound", feature = "clipboard", feature = "qoi", feature = "qoiz")),
    expect(
        clippy::derivable_impls,
        reason = "fields setting non-default values are feature-gated; the impl is only trivially derivable in some feature combinations"
    )
)]
impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            #[cfg(feature = "sound")]
            sound: true,
            #[cfg(feature = "clipboard")]
            clipboard: ClipboardType::Enable,
            #[cfg(feature = "rdpdr")]
            rdpdr: RdpdrConfig::default(),
            #[cfg(feature = "qoi")]
            qoi: true,
            #[cfg(feature = "qoiz")]
            qoiz: true,
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
}

#[cfg(feature = "rdpdr")]
impl Default for RdpdrConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            #[cfg(feature = "smartcard")]
            smartcard: true,
        }
    }
}

/// Transport selection for the RDP connection.
#[derive(Clone, Debug)]
pub enum Transport {
    /// Plain TCP → TLS direct connection to the RDP server.
    Direct,

    /// Connect via an RDS gateway (MS-TSGU / MSTSGU).
    ///
    /// The target RDP server is derived from [`Config::destination`]; the gateway
    /// only needs its own endpoint and credentials.
    ///
    /// NOTE: the destination port is currently not forwarded to the gateway.
    /// If `ironrdp-mstsgu` hardcodes port 3389, open a follow-up issue.
    #[cfg(feature = "gateway")]
    Gateway(GatewayConfig),

    /// Connect via an RDCleanPath proxy (WebSocket-based).
    RDCleanPath(RDCleanPathConfig),
}

/// Credentials and endpoint for an RDS gateway connection.
#[cfg(feature = "gateway")]
#[derive(Clone, Debug)]
pub struct GatewayConfig {
    /// Gateway endpoint address (e.g., `"rdg.contoso.com:443"`).
    pub endpoint: String,
    /// Gateway username.
    pub username: String,
    /// Gateway password.
    pub password: String,
}

// ── Destination ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    name: String,
    port: u16,
}

impl Destination {
    pub fn new(addr: impl Into<String>) -> anyhow::Result<Self> {
        const RDP_DEFAULT_PORT: u16 = 3389;

        let addr = addr.into();

        if let Some(addr_split) = addr.rsplit_once(':') {
            if let Ok(sock_addr) = addr.parse::<core::net::SocketAddr>() {
                Ok(Self {
                    name: sock_addr.ip().to_string(),
                    port: sock_addr.port(),
                })
            } else if addr.parse::<core::net::Ipv6Addr>().is_ok() {
                Ok(Self {
                    name: addr,
                    port: RDP_DEFAULT_PORT,
                })
            } else {
                Ok(Self {
                    name: addr_split.0.to_owned(),
                    port: addr_split.1.parse().context("invalid port")?,
                })
            }
        } else {
            Ok(Self {
                name: addr,
                port: RDP_DEFAULT_PORT,
            })
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Construct a `Destination` from already-validated components.
    ///
    /// Intended for front-ends that have already resolved the host and port from their own
    /// configuration sources (CLI flags, `.rdp` files, IPC schemas).
    pub fn from_parts(name: impl Into<String>, port: u16) -> Self {
        Self {
            name: name.into(),
            port,
        }
    }
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // IPv6 addresses must be bracketed in host:port notation.
        if self.name.parse::<core::net::Ipv6Addr>().is_ok() {
            write!(f, "[{}]:{}", self.name, self.port)
        } else {
            write!(f, "{}:{}", self.name, self.port)
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

#[derive(Clone, Debug)]
pub struct RDCleanPathConfig {
    pub url: Url,
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
        let mut parts = s.split('=');
        let channel_name = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing DVC channel name"))?
            .to_owned();
        let pipe_name = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing DVC proxy pipe name"))?
            .to_owned();

        Ok(Self {
            channel_name,
            pipe_name,
        })
    }
}

// ── ConfigBuilder ─────────────────────────────────────────────────────────────

/// Builder for [`Config`].
///
/// # Duplicate-channel behaviour
///
/// * **Static channels** are keyed by the concrete processor `TypeId`; registering two factories
///   with the same concrete type silently shadows the earlier one via
///   [`ironrdp_connector::ClientConnector::attach_static_channel`].
/// * **DVC channels** are keyed by channel name; duplicate names follow
///   [`ironrdp_dvc::DrdynvcClient`]'s overwrite semantics.
pub struct ConfigBuilder {
    config: Config,
}

impl ConfigBuilder {
    pub fn new(connector: ironrdp_connector::Config, destination: Destination) -> Self {
        Self {
            config: Config {
                connector,
                destination,
                transport: Transport::Direct,
                kerberos_config: None,
                log_file: None,
                fake_events_interval: None,
                channels: ChannelConfig::default(),
                #[cfg(feature = "dvc-pipe-proxy")]
                dvc_pipe_proxies: Vec::new(),
                #[cfg(all(windows, feature = "dvc-com-plugin"))]
                dvc_plugins: Vec::new(),
                extensions: ExtensionRegistry::default(),
            },
        }
    }

    #[must_use]
    pub fn with_transport(mut self, transport: Transport) -> Self {
        self.config.transport = transport;
        self
    }

    #[must_use]
    pub fn with_kerberos_config(mut self, cfg: ironrdp_connector::credssp::KerberosConfig) -> Self {
        self.config.kerberos_config = Some(cfg);
        self
    }

    #[must_use]
    pub fn with_log_file(mut self, path: impl Into<String>) -> Self {
        self.config.log_file = Some(path.into());
        self
    }

    #[must_use]
    pub fn with_fake_events_interval(mut self, interval: Duration) -> Self {
        self.config.fake_events_interval = Some(interval);
        self
    }

    /// Enable or disable RDPSND (audio) playback.
    #[cfg(feature = "sound")]
    #[must_use]
    pub fn with_sound(mut self, enabled: bool) -> Self {
        self.config.channels.sound = enabled;
        self
    }

    /// Set the CLIPRDR (clipboard) redirection mode.
    #[cfg(feature = "clipboard")]
    #[must_use]
    pub fn with_clipboard(mut self, mode: ClipboardType) -> Self {
        self.config.channels.clipboard = mode;
        self
    }

    /// Enable or disable RDPDR (device redirection).
    #[cfg(feature = "rdpdr")]
    #[must_use]
    pub fn with_rdpdr(mut self, enabled: bool) -> Self {
        self.config.channels.rdpdr.enabled = enabled;
        self
    }

    /// Enable or disable smart-card redirection within RDPDR.
    #[cfg(feature = "smartcard")]
    #[must_use]
    pub fn with_smartcard(mut self, enabled: bool) -> Self {
        self.config.channels.rdpdr.smartcard = enabled;
        self
    }

    /// Enable or disable QOI bitmap codec at runtime.
    #[cfg(feature = "qoi")]
    #[must_use]
    pub fn with_qoi(mut self, enabled: bool) -> Self {
        self.config.channels.qoi = enabled;
        self
    }

    /// Enable or disable QOIZ bitmap codec at runtime.
    #[cfg(feature = "qoiz")]
    #[must_use]
    pub fn with_qoiz(mut self, enabled: bool) -> Self {
        self.config.channels.qoiz = enabled;
        self
    }

    /// Add a DVC pipe proxy channel.
    #[cfg(feature = "dvc-pipe-proxy")]
    #[must_use]
    pub fn with_dvc_pipe_proxy(mut self, info: DvcProxyInfo) -> Self {
        self.config.dvc_pipe_proxies.push(info);
        self
    }

    /// Add a DVC COM plugin DLL path (Windows only).
    #[cfg(all(windows, feature = "dvc-com-plugin"))]
    #[must_use]
    pub fn with_dvc_plugin(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.dvc_plugins.push(path.into());
        self
    }

    /// Register a factory for a user-defined static virtual channel.
    ///
    /// `factory` is called once per connection attempt to create a fresh channel instance.
    /// Duplicate processor types follow `attach_static_channel` overwrite semantics.
    #[must_use]
    pub fn with_static_channel<P, F>(mut self, factory: F) -> Self
    where
        F: Fn() -> P + Send + Sync + 'static,
        P: ironrdp_svc::SvcClientProcessor + 'static,
    {
        let cb: StaticChannelFn = Arc::new(move |connector: &mut ironrdp_connector::ClientConnector| {
            connector.attach_static_channel(factory())
        });
        self.config.extensions.static_channels.push(cb);
        self
    }

    /// Register a factory for a user-defined dynamic virtual channel.
    ///
    /// `factory` is called once per connection attempt to create a fresh channel instance.
    /// Duplicate channel names follow `DrdynvcClient` overwrite semantics.
    #[must_use]
    pub fn with_dvc<P, F>(mut self, factory: F) -> Self
    where
        F: Fn() -> P + Send + Sync + 'static,
        P: ironrdp_dvc::DvcProcessor + 'static,
    {
        let cb: DvcChannelFn = Arc::new(move |drdynvc| drdynvc.attach_dynamic_channel(factory()));
        self.config.extensions.dvc_channels.push(cb);
        self
    }

    pub fn build(self) -> Config {
        self.config
    }
}
