use core::fmt;
use core::str::FromStr;
use core::time::Duration;
#[cfg(windows)]
use std::path::PathBuf;

use anyhow::Context as _;
use ironrdp::connector::{self, Credentials};
use ironrdp::pdu::rdp::capability_sets::{MajorPlatformType, client_codecs_capabilities};
use ironrdp::pdu::rdp::client_info::{CompressionType, PerformanceFlags, TimezoneInfo};
#[cfg(feature = "gateway")]
use ironrdp_mstsgu::GwConnectTarget;
pub use ironrdp_propertyset::PropertySet;
use url::Url;

const RDP_DEFAULT_PORT: u16 = 3389;
const DEFAULT_WIDTH: u16 = 1920;
const DEFAULT_HEIGHT: u16 = 1080;

/// Fully resolved client configuration.
///
/// Build one via [`ConfigBuilder`]. The viewer/agent translate CLI/IPC inputs into a
/// [`PropertySet`], then call [`ConfigBuilder::from_property_set`] to seed a builder.
pub struct Config {
    pub log_file: Option<String>,
    #[cfg(feature = "gateway")]
    pub gw: Option<GwConnectTarget>,
    pub kerberos_config: Option<connector::credssp::KerberosConfig>,
    pub destination: Destination,
    pub connector: connector::Config,
    pub clipboard_type: ClipboardType,
    pub rdcleanpath: Option<RDCleanPathConfig>,
    pub fake_events_interval: Option<Duration>,

    /// Runtime feature gates (composed with Cargo feature gates).
    pub features: Features,

    /// DVC channel ↔ named-pipe proxy configuration.
    pub dvc_pipe_proxies: Vec<DvcProxyInfo>,

    /// Paths to DVC client plugin DLLs to load (Windows only).
    #[cfg(windows)]
    pub dvc_plugins: Vec<PathBuf>,

    /// Optional override for the sound (rdpsnd) backend.
    #[cfg(feature = "sound")]
    pub sound_backend: Option<Box<dyn ironrdp::rdpsnd::client::RdpsndClientHandler>>,

    /// Optional override for the clipboard (cliprdr) backend factory.
    #[cfg(feature = "clipboard")]
    pub clipboard_backend: Option<Box<dyn ironrdp::cliprdr::backend::CliprdrBackendFactory + Send>>,

    /// Optional override for the rdpdr backend.
    #[cfg(feature = "rdpdr")]
    pub rdpdr_backend: Option<Box<dyn ironrdp::rdpdr::backend::RdpdrBackend>>,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("destination", &self.destination)
            .field("clipboard_type", &self.clipboard_type)
            .field("features", &self.features)
            .field("rdcleanpath", &self.rdcleanpath)
            .field("fake_events_interval", &self.fake_events_interval)
            .field("log_file", &self.log_file)
            .finish_non_exhaustive()
    }
}

/// Runtime feature gates.
///
/// Each flag is composed with the matching Cargo feature: a feature is only effectively
/// enabled when both the Cargo feature is compiled in and the runtime flag is `true`.
///
/// The defaults match "everything that is compiled in is enabled": every flag defaults to
/// `true` even when its Cargo feature is absent (a no-op in that case).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Features {
    pub sound: bool,
    pub clipboard: bool,
    pub rdpdr: bool,
    pub smartcard: bool,
    pub gateway: bool,
    pub qoi: bool,
    pub qoiz: bool,
    pub dvc_pipe_proxy: bool,
    pub dvc_com_plugin: bool,
}

impl Default for Features {
    fn default() -> Self {
        Self {
            sound: true,
            clipboard: true,
            rdpdr: true,
            smartcard: true,
            gateway: true,
            qoi: true,
            qoiz: true,
            dvc_pipe_proxy: true,
            dvc_com_plugin: true,
        }
    }
}

/// Resolved clipboard backend selection.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClipboardType {
    #[default]
    Enable,
    Disable,
    Stub,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    name: String,
    port: u16,
}

impl Destination {
    pub fn new(addr: impl Into<String>) -> anyhow::Result<Self> {
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

    pub fn from_parts(name: impl Into<String>, port: u16) -> Self {
        Self {
            name: name.into(),
            port,
        }
    }
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

impl From<Destination> for connector::ServerName {
    fn from(value: Destination) -> Self {
        Self::new(value.name)
    }
}

impl From<&Destination> for connector::ServerName {
    fn from(value: &Destination) -> Self {
        Self::new(&value.name)
    }
}

#[derive(Clone, Debug)]
pub struct RDCleanPathConfig {
    pub url: Url,
    pub auth_token: String,
}

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
            .ok_or_else(|| anyhow::anyhow!("missing dvc channel name"))?
            .to_owned();
        let pipe_name = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing dvc proxy pipe name"))?
            .to_owned();

        Ok(Self {
            channel_name,
            pipe_name,
        })
    }
}

/// Builder for [`Config`]. The single public entrypoint for materialising a typed configuration.
///
/// Typical flow:
///
/// ```ignore
/// let props = ...; // parsed from .rdp + CLI overrides
/// let config = ConfigBuilder::from_property_set(&props)
///     .with_log_file(args.log_file)
///     .with_clipboard(true)
///     .build()?;
/// ```
#[derive(Default)]
pub struct ConfigBuilder {
    // Server / destination
    destination: Option<Destination>,
    server_username: Option<String>,
    server_password: Option<String>,
    server_domain: Option<String>,
    no_tls: bool,
    enable_credssp: Option<bool>,
    autologon: bool,

    // Desktop
    desktop_width: Option<u16>,
    desktop_height: Option<u16>,
    desktop_scale_factor: Option<u32>,
    color_depth: Option<u32>,

    // Compression
    compression_enabled: Option<bool>,
    compression_level: Option<u32>,

    // Codecs
    codecs: Vec<String>,

    // Gateway
    #[cfg(feature = "gateway")]
    gw: Option<GwConnectTarget>,

    // Kerberos
    kerberos_config: Option<connector::credssp::KerberosConfig>,

    // Capabilities/keyboard
    keyboard_type: Option<ironrdp::pdu::gcc::KeyboardType>,
    keyboard_subtype: u32,
    keyboard_functional_keys_count: u32,
    keyboard_layout: u32,
    ime_file_name: String,
    dig_product_id: String,

    // Audio
    audio_playback: Option<bool>,

    // Clipboard / etc.
    clipboard_type: ClipboardType,

    // Session prevention
    prevent_session_lock_minutes: Option<u32>,

    // RDCleanPath
    rdcleanpath: Option<RDCleanPathConfig>,

    // Cli-only
    log_file: Option<String>,
    no_server_pointer: bool,
    thin_client: bool,
    small_cache: bool,
    capabilities: u32,

    // DVC
    dvc_pipe_proxies: Vec<DvcProxyInfo>,
    #[cfg(windows)]
    dvc_plugins: Vec<PathBuf>,

    // Application
    alternate_shell: String,
    work_dir: String,

    // Runtime feature gates
    features: Features,

    // Backend slots
    #[cfg(feature = "sound")]
    sound_backend: Option<Box<dyn ironrdp::rdpsnd::client::RdpsndClientHandler>>,
    #[cfg(feature = "clipboard")]
    clipboard_backend: Option<Box<dyn ironrdp::cliprdr::backend::CliprdrBackendFactory + Send>>,
    #[cfg(feature = "rdpdr")]
    rdpdr_backend: Option<Box<dyn ironrdp::rdpdr::backend::RdpdrBackend>>,
}

impl ConfigBuilder {
    pub fn new() -> Self {
        Self {
            keyboard_type: None,
            keyboard_subtype: 0,
            keyboard_functional_keys_count: 12,
            keyboard_layout: 0,
            ime_file_name: String::new(),
            dig_product_id: String::new(),
            clipboard_type: ClipboardType::Enable,
            features: Features::default(),
            ..Default::default()
        }
    }

    /// Seed the builder from an `.rdp`-style [`PropertySet`].
    ///
    /// Sets all properties present in the bag. Properties absent from the bag are not touched
    /// (the builder keeps its prior/defaulted values).
    pub fn from_property_set(props: &PropertySet) -> Self {
        use ironrdp_cfg::{AudioMode, PropertySetExt as _};

        let mut b = Self::new();

        // Destination
        let target = match props.full_address() {
            Ok(Some(addr)) => Some(addr),
            Ok(None) => props.alternate_full_address().ok().flatten(),
            Err(_) => None,
        };
        if let Some(target) = target {
            let port = target
                .port
                .or_else(|| props.server_port().ok().flatten())
                .unwrap_or(RDP_DEFAULT_PORT);
            let name = match target.host {
                ironrdp_cfg::TargetHost::Ip(ip) => ip.to_string(),
                ironrdp_cfg::TargetHost::Domain(host) => host,
            };
            b.destination = Some(Destination::from_parts(name, port));
        }

        if let Some(u) = props.username() {
            b.server_username = Some(u.to_owned());
        }
        if let Some(p) = props.clear_text_password() {
            b.server_password = Some(p.to_owned());
        }
        if let Some(d) = props.domain() {
            b.server_domain = Some(d.to_owned());
        }

        if let Some(v) = props.enable_credssp_support() {
            b.enable_credssp = Some(v);
        }

        if let Ok(Some(w)) = props.desktop_width() {
            b.desktop_width = Some(w);
        }
        if let Ok(Some(h)) = props.desktop_height() {
            b.desktop_height = Some(h);
        }
        if let Ok(Some(s)) = props.desktop_scale_factor() {
            b.desktop_scale_factor = Some(s);
        }

        if let Some(c) = props.compression() {
            b.compression_enabled = Some(c);
        }

        if let Ok(audio_mode) = props.audio_mode() {
            b.audio_playback = Some(matches!(audio_mode, None | Some(AudioMode::RedirectToClient)));
        }

        if let Some(redirect_clipboard) = props.redirect_clipboard() {
            if !redirect_clipboard {
                b.clipboard_type = ClipboardType::Disable;
            }
        }

        // Application launch
        if let Some(s) = props.alternate_shell() {
            b.alternate_shell = s.to_owned();
        }
        if let Some(s) = props.shell_working_directory() {
            b.work_dir = s.to_owned();
        }

        // Kerberos KDC proxy
        let kdc_proxy_url = props
            .kdc_proxy_url()
            .map(str::to_owned)
            .or_else(|| props.kdc_proxy_name().map(normalize_kdc_proxy_url_from_name));
        if let Some(url_str) = kdc_proxy_url {
            if let Ok(url) = Url::parse(&url_str) {
                b.kerberos_config = Some(connector::credssp::KerberosConfig {
                    kdc_proxy_url: Some(url),
                    hostname: whoami::hostname().unwrap_or_else(|_| "ironrdp".to_owned()),
                });
            }
        }

        // Gateway
        #[cfg(feature = "gateway")]
        {
            let has_host = props.gateway_hostname().is_some();
            let use_gateway = props
                .gateway_usage_method()
                .ok()
                .flatten()
                .map_or(has_host, ironrdp_cfg::GatewayUsageMethod::is_gateway_required);

            if use_gateway {
                if let Some(host) = props.gateway_hostname() {
                    b.gw = Some(GwConnectTarget {
                        gw_endpoint: host.to_owned(),
                        gw_user: props.gateway_username().unwrap_or_default().to_owned(),
                        gw_pass: props.gateway_password().unwrap_or_default().to_owned(),
                        server: String::new(),
                    });
                }
            }
        }

        b
    }

    #[must_use]
    pub fn with_destination(mut self, destination: Destination) -> Self {
        self.destination = Some(destination);
        self
    }

    #[must_use]
    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.server_username = Some(username.into());
        self
    }

    #[must_use]
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.server_password = Some(password.into());
        self
    }

    #[must_use]
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.server_domain = Some(domain.into());
        self
    }

    #[must_use]
    pub fn with_log_file(mut self, log_file: Option<String>) -> Self {
        self.log_file = log_file;
        self
    }

    #[must_use]
    pub fn with_codecs(mut self, codecs: Vec<String>) -> Self {
        self.codecs = codecs;
        self
    }

    #[must_use]
    pub fn with_color_depth(mut self, depth: Option<u32>) -> Self {
        self.color_depth = depth;
        self
    }

    #[must_use]
    pub fn with_capabilities(mut self, capabilities: u32) -> Self {
        self.capabilities = capabilities;
        self
    }

    #[must_use]
    pub fn with_no_tls(mut self, value: bool) -> Self {
        self.no_tls = value;
        self
    }

    #[must_use]
    pub fn with_autologon(mut self, value: bool) -> Self {
        self.autologon = value;
        self
    }

    #[must_use]
    pub fn with_no_server_pointer(mut self, value: bool) -> Self {
        self.no_server_pointer = value;
        self
    }

    #[must_use]
    pub fn with_thin_client(mut self, value: bool) -> Self {
        self.thin_client = value;
        self
    }

    #[must_use]
    pub fn with_small_cache(mut self, value: bool) -> Self {
        self.small_cache = value;
        self
    }

    #[must_use]
    pub fn with_compression_level(mut self, level: Option<u32>) -> Self {
        self.compression_level = level;
        self
    }

    #[must_use]
    pub fn with_prevent_session_lock_minutes(mut self, minutes: Option<u32>) -> Self {
        self.prevent_session_lock_minutes = minutes;
        self
    }

    #[must_use]
    pub fn with_clipboard_type(mut self, t: ClipboardType) -> Self {
        self.clipboard_type = t;
        self
    }

    #[must_use]
    pub fn with_rdcleanpath(mut self, cfg: Option<RDCleanPathConfig>) -> Self {
        self.rdcleanpath = cfg;
        self
    }

    #[must_use]
    pub fn with_keyboard_type(mut self, kt: ironrdp::pdu::gcc::KeyboardType) -> Self {
        self.keyboard_type = Some(kt);
        self
    }

    #[must_use]
    pub fn with_keyboard_subtype(mut self, subtype: u32) -> Self {
        self.keyboard_subtype = subtype;
        self
    }

    #[must_use]
    pub fn with_keyboard_functional_keys_count(mut self, count: u32) -> Self {
        self.keyboard_functional_keys_count = count;
        self
    }

    #[must_use]
    pub fn with_keyboard_layout(mut self, layout: u32) -> Self {
        self.keyboard_layout = layout;
        self
    }

    #[must_use]
    pub fn with_ime_file_name(mut self, name: impl Into<String>) -> Self {
        self.ime_file_name = name.into();
        self
    }

    #[must_use]
    pub fn with_dig_product_id(mut self, id: impl Into<String>) -> Self {
        self.dig_product_id = id.into();
        self
    }

    #[must_use]
    pub fn with_dvc_pipe_proxies(mut self, proxies: Vec<DvcProxyInfo>) -> Self {
        self.dvc_pipe_proxies = proxies;
        self
    }

    #[cfg(windows)]
    #[must_use]
    pub fn with_dvc_plugins(mut self, plugins: Vec<PathBuf>) -> Self {
        self.dvc_plugins = plugins;
        self
    }

    #[cfg(feature = "gateway")]
    #[must_use]
    pub fn with_gateway_target(mut self, gw: Option<GwConnectTarget>) -> Self {
        self.gw = gw;
        self
    }

    // ---- Runtime feature gates (always compile, no-op when Cargo feature is absent) ----

    /// Enable or disable the rdpsnd channel for this session.
    #[must_use]
    pub fn with_sound(mut self, enable: bool) -> Self {
        self.features.sound = enable;
        self
    }

    #[must_use]
    pub fn with_clipboard(mut self, enable: bool) -> Self {
        self.features.clipboard = enable;
        self
    }

    #[must_use]
    pub fn with_rdpdr(mut self, enable: bool) -> Self {
        self.features.rdpdr = enable;
        self
    }

    #[must_use]
    pub fn with_smartcard(mut self, enable: bool) -> Self {
        self.features.smartcard = enable;
        self
    }

    #[must_use]
    pub fn with_gateway(mut self, enable: bool) -> Self {
        self.features.gateway = enable;
        self
    }

    #[must_use]
    pub fn with_qoi(mut self, enable: bool) -> Self {
        self.features.qoi = enable;
        self
    }

    #[must_use]
    pub fn with_qoiz(mut self, enable: bool) -> Self {
        self.features.qoiz = enable;
        self
    }

    #[must_use]
    pub fn with_dvc_pipe_proxy(mut self, enable: bool) -> Self {
        self.features.dvc_pipe_proxy = enable;
        self
    }

    #[must_use]
    pub fn with_dvc_com_plugin(mut self, enable: bool) -> Self {
        self.features.dvc_com_plugin = enable;
        self
    }

    // ---- Backend escape hatches ----

    #[cfg(feature = "sound")]
    #[must_use]
    pub fn with_sound_backend(mut self, backend: Box<dyn ironrdp::rdpsnd::client::RdpsndClientHandler>) -> Self {
        self.sound_backend = Some(backend);
        self
    }

    #[cfg(feature = "clipboard")]
    #[must_use]
    pub fn with_clipboard_backend(
        mut self,
        backend: Box<dyn ironrdp::cliprdr::backend::CliprdrBackendFactory + Send>,
    ) -> Self {
        self.clipboard_backend = Some(backend);
        self
    }

    #[cfg(feature = "rdpdr")]
    #[must_use]
    pub fn with_rdpdr_backend(mut self, backend: Box<dyn ironrdp::rdpdr::backend::RdpdrBackend>) -> Self {
        self.rdpdr_backend = Some(backend);
        self
    }

    /// Finalise the builder into a [`Config`].
    ///
    /// Returns an error if a required field (destination, username, password) is missing.
    /// Front-ends should fill those in via their own UI (e.g. inquire) before calling `build`.
    pub fn build(self) -> anyhow::Result<Config> {
        let destination = self.destination.context("server address is required")?;
        let username = self.server_username.context("username is required")?;
        let password = self.server_password.context("password is required")?;

        let codecs: Vec<&str> = self.codecs.iter().map(String::as_str).collect();
        let codecs = match client_codecs_capabilities(&codecs) {
            Ok(c) => c,
            Err(help) => anyhow::bail!("invalid codecs spec: {help}"),
        };

        let mut bitmap = connector::BitmapConfig {
            color_depth: 32,
            lossy_compression: true,
            codecs,
        };
        if let Some(color_depth) = self.color_depth {
            if color_depth != 16 && color_depth != 32 {
                anyhow::bail!("invalid color depth (only 16 and 32 are supported)");
            }
            bitmap.color_depth = color_depth;
        }

        let fake_events_interval = self
            .prevent_session_lock_minutes
            .map(|v| Duration::from_secs(u64::from(v) * 60));

        let compression_enabled = self.compression_enabled.unwrap_or(true);
        let compression_type = if compression_enabled {
            Some(compression_type_from_level(self.compression_level.unwrap_or(3))?)
        } else {
            None
        };

        let desktop_width = self.desktop_width.unwrap_or(DEFAULT_WIDTH);
        let desktop_height = self.desktop_height.unwrap_or(DEFAULT_HEIGHT);
        let desktop_scale_factor = self.desktop_scale_factor.unwrap_or(0);

        let clipboard_type = self.clipboard_type;

        #[cfg(feature = "gateway")]
        let gw = {
            let mut gw = self.gw;
            if let Some(ref mut gw) = gw {
                gw.server = destination.name().to_owned();
            }
            gw
        };

        let keyboard_type = self
            .keyboard_type
            .unwrap_or(ironrdp::pdu::gcc::KeyboardType::IbmEnhanced);

        let connector = connector::Config {
            credentials: Credentials::UsernamePassword { username, password },
            domain: self.server_domain,
            enable_tls: !self.no_tls,
            enable_credssp: self.enable_credssp.unwrap_or(true),
            keyboard_type,
            keyboard_subtype: self.keyboard_subtype,
            keyboard_layout: self.keyboard_layout,
            keyboard_functional_keys_count: self.keyboard_functional_keys_count,
            ime_file_name: self.ime_file_name,
            dig_product_id: self.dig_product_id,
            desktop_size: connector::DesktopSize {
                width: desktop_width,
                height: desktop_height,
            },
            desktop_scale_factor,
            bitmap: Some(bitmap),
            client_build: package_version_as_build()?,
            client_name: whoami::hostname().unwrap_or_else(|_| "ironrdp".to_owned()),
            client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),
            platform: match whoami::platform() {
                whoami::Platform::Windows => MajorPlatformType::WINDOWS,
                whoami::Platform::Linux => MajorPlatformType::UNIX,
                whoami::Platform::Mac => MajorPlatformType::MACINTOSH,
                whoami::Platform::Ios => MajorPlatformType::IOS,
                whoami::Platform::Android => MajorPlatformType::ANDROID,
                _ => MajorPlatformType::UNSPECIFIED,
            },
            hardware_id: None,
            license_cache: None,
            enable_server_pointer: !self.no_server_pointer,
            autologon: self.autologon,
            enable_audio_playback: self.audio_playback.unwrap_or(true),
            request_data: None,
            pointer_software_rendering: false,
            multitransport_flags: None,
            compression_type,
            performance_flags: PerformanceFlags::default(),
            timezone_info: TimezoneInfo::default(),
            alternate_shell: self.alternate_shell,
            work_dir: self.work_dir,
        };

        Ok(Config {
            log_file: self.log_file,
            #[cfg(feature = "gateway")]
            gw,
            kerberos_config: self.kerberos_config,
            destination,
            connector,
            clipboard_type,
            rdcleanpath: self.rdcleanpath,
            fake_events_interval,
            features: self.features,
            dvc_pipe_proxies: self.dvc_pipe_proxies,
            #[cfg(windows)]
            dvc_plugins: self.dvc_plugins,
            #[cfg(feature = "sound")]
            sound_backend: self.sound_backend,
            #[cfg(feature = "clipboard")]
            clipboard_backend: self.clipboard_backend,
            #[cfg(feature = "rdpdr")]
            rdpdr_backend: self.rdpdr_backend,
        })
    }
}

fn compression_type_from_level(level: u32) -> anyhow::Result<CompressionType> {
    match level {
        0 => Ok(CompressionType::K8),
        1 => Ok(CompressionType::K64),
        2 => Ok(CompressionType::Rdp6),
        3 => Ok(CompressionType::Rdp61),
        _ => anyhow::bail!("invalid compression level (valid values: 0..=3)"),
    }
}

fn package_version_as_build() -> anyhow::Result<u32> {
    let version = env!("CARGO_PKG_VERSION");
    let parts: Vec<u32> = version
        .split('.')
        .take(3)
        .filter_map(|p| {
            p.split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|s| s.parse().ok())
        })
        .collect();
    let major = parts.first().copied().unwrap_or(0);
    let minor = parts.get(1).copied().unwrap_or(0);
    let patch = parts.get(2).copied().unwrap_or(0);
    Ok(major
        .saturating_mul(100)
        .saturating_add(minor.saturating_mul(10))
        .saturating_add(patch))
}

fn normalize_kdc_proxy_url_from_name(name: &str) -> String {
    if name.starts_with("http://") || name.starts_with("https://") {
        name.to_owned()
    } else {
        format!("https://{name}/KdcProxy")
    }
}
