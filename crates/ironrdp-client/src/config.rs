#![allow(clippy::print_stdout, clippy::print_stderr)]

use core::num::ParseIntError;
use core::str::FromStr;
use std::path::PathBuf;

use anyhow::Context as _;
use clap::Parser;
use clap::clap_derive::ValueEnum;
use ironrdp::connector::{self, Credentials};
use ironrdp::pdu::rdp::capability_sets::{MajorPlatformType, client_codecs_capabilities};
use ironrdp::pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp_mstsgu::GwConnectTarget;
use tap::prelude::*;
use url::Url;

const DEFAULT_WIDTH: u16 = 1920;
const DEFAULT_HEIGHT: u16 = 1080;

#[derive(Clone, Debug)]
pub struct Config {
    pub log_file: Option<String>,
    pub gw: Option<GwConnectTarget>,
    pub kerberos_config: Option<connector::credssp::KerberosConfig>,
    pub destination: Destination,
    pub connector: connector::Config,
    pub clipboard_type: ClipboardType,
    pub rdcleanpath: Option<RDCleanPathConfig>,

    /// DVC channel <-> named pipe proxy configuration.
    ///
    /// Each configured proxy enables IronRDP to connect to DVC channel and create a named pipe
    /// server, which will be used for proxying DVC messages to/from user-defined DVC logic
    /// implemented as named pipe clients (either in the same process or in a different process).
    pub dvc_pipe_proxies: Vec<DvcProxyInfo>,

    /// Paths to DVC client plugin DLLs to load (Windows only).
    ///
    /// Each DLL is loaded via `LoadLibraryW` and its `VirtualChannelGetInstance` export is called
    /// to obtain DVC plugin COM objects. Example: `C:\Windows\System32\webauthn.dll`.
    #[cfg(windows)]
    pub dvc_plugins: Vec<PathBuf>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum ClipboardType {
    Default,
    Stub,
    #[cfg(windows)]
    Windows,
    None,
}

fn apply_cli_args_to_properties(properties: &mut ironrdp_propertyset::PropertySet, args: &Args) {
    if let Some(dest) = &args.destination {
        // Format the host in .rdp canonical form: IPv6 gets bracketed ("[::1]"), others are plain.
        let host = dest
            .name()
            .parse::<core::net::IpAddr>()
            .map(ironrdp_cfg::TargetHost::Ip)
            .unwrap_or_else(|_| ironrdp_cfg::TargetHost::Domain(dest.name().to_owned()));
        properties.insert("full address", format!("{host}:{}", dest.port()));
    }

    if let Some(username) = &args.username {
        properties.insert("username", username.as_str());
    }

    if let Some(password) = &args.password {
        properties.insert("ClearTextPassword", password.as_str());
    }

    if let Some(domain) = &args.domain {
        properties.insert("domain", domain.as_str());
    }

    if let Some(scale) = args.scale_desktop {
        properties.insert("desktopscalefactor", i64::from(scale));
    }

    if let Some(gw_host) = &args.gw_endpoint {
        properties.insert("gatewayhostname", gw_host.as_str());
        // Ensure the gateway is treated as enabled when a host is provided explicitly.
        properties.insert(
            "gatewayusagemethod",
            ironrdp_cfg::GatewayUsageMethod::UseAlways.as_i64(),
        );
    }

    if let Some(gw_user) = &args.gw_user {
        properties.insert("gatewayusername", gw_user.as_str());
    }

    if let Some(gw_pass) = &args.gw_pass {
        properties.insert("GatewayPassword", gw_pass.as_str());
    }

    if args.no_credssp {
        properties.insert("enablecredsspsupport", 0i64);
    }

    if let Some(enabled) = args.compression_enabled {
        properties.insert("compression", enabled);
    }
}

fn compression_type_from_level(level: u32) -> anyhow::Result<ironrdp::pdu::rdp::client_info::CompressionType> {
    use ironrdp::pdu::rdp::client_info::CompressionType;

    match level {
        0 => Ok(CompressionType::K8),
        1 => Ok(CompressionType::K64),
        2 => Ok(CompressionType::Rdp6),
        3 => Ok(CompressionType::Rdp61),
        _ => anyhow::bail!("Invalid compression level. Valid values are 0, 1, 2, 3."),
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum KeyboardType {
    IbmPcXt,
    OlivettiIco,
    IbmPcAt,
    IbmEnhanced,
    Nokia1050,
    Nokia9140,
    Japanese,
}

impl KeyboardType {
    fn parse(keyboard_type: KeyboardType) -> ironrdp::pdu::gcc::KeyboardType {
        match keyboard_type {
            KeyboardType::IbmEnhanced => ironrdp::pdu::gcc::KeyboardType::IbmEnhanced,
            KeyboardType::IbmPcAt => ironrdp::pdu::gcc::KeyboardType::IbmPcAt,
            KeyboardType::IbmPcXt => ironrdp::pdu::gcc::KeyboardType::IbmPcXt,
            KeyboardType::OlivettiIco => ironrdp::pdu::gcc::KeyboardType::OlivettiIco,
            KeyboardType::Nokia1050 => ironrdp::pdu::gcc::KeyboardType::Nokia1050,
            KeyboardType::Nokia9140 => ironrdp::pdu::gcc::KeyboardType::Nokia9140,
            KeyboardType::Japanese => ironrdp::pdu::gcc::KeyboardType::Japanese,
        }
    }
}

fn parse_hex(input: &str) -> Result<u32, ParseIntError> {
    if input.starts_with("0x") {
        u32::from_str_radix(input.get(2..).unwrap_or(""), 16)
    } else {
        input.parse::<u32>()
    }
}

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

/// Devolutions IronRDP client
#[derive(Parser, Debug)]
#[clap(author = "Devolutions", about = "Devolutions-IronRDP client")]
#[clap(version, long_about = None)]
struct Args {
    /// A file with IronRDP client logs
    #[clap(short, long, value_parser)]
    log_file: Option<String>,

    #[clap(long, value_parser)]
    gw_endpoint: Option<String>,
    #[clap(long, value_parser)]
    gw_user: Option<String>,
    #[clap(long, value_parser)]
    gw_pass: Option<String>,

    /// An address on which the client will connect.
    destination: Option<Destination>,

    /// Path to a .rdp file to read the configuration from.
    #[clap(long)]
    rdp_file: Option<PathBuf>,

    /// A target RDP server user name
    #[clap(short, long)]
    username: Option<String>,

    /// An optional target RDP server domain name
    #[clap(short, long)]
    domain: Option<String>,

    /// A target RDP server user password
    #[clap(short, long)]
    password: Option<String>,

    /// Proxy URL to connect to for the RDCleanPath
    #[clap(long, requires("rdcleanpath_token"))]
    rdcleanpath_url: Option<Url>,

    /// Authentication token to insert in the RDCleanPath packet
    #[clap(long, requires("rdcleanpath_url"))]
    rdcleanpath_token: Option<String>,

    /// The keyboard type
    #[clap(long, value_enum, default_value_t = KeyboardType::IbmEnhanced)]
    keyboard_type: KeyboardType,

    /// The keyboard subtype (an original equipment manufacturer-dependent value)
    #[clap(long, default_value_t = 0)]
    keyboard_subtype: u32,

    /// The number of function keys on the keyboard
    #[clap(long, default_value_t = 12)]
    keyboard_functional_keys_count: u32,

    /// The input method editor (IME) file name associated with the active input locale
    #[clap(long, default_value_t = String::from(""))]
    ime_file_name: String,

    /// Contains a value that uniquely identifies the client
    #[clap(long, default_value_t = String::from(""))]
    dig_product_id: String,

    /// Enable thin client
    #[clap(long)]
    thin_client: bool,

    /// Enable small cache
    #[clap(long)]
    small_cache: bool,

    /// Scaling factor for desktop applications, percentage (value between 100 and 500)
    #[clap(long, value_parser = clap::value_parser!(u32).range(100..=500))]
    scale_desktop: Option<u32>,

    /// Set required color depth. Currently only 32 and 16 bit color depths are supported
    #[clap(long)]
    color_depth: Option<u32>,

    /// Ignore mouse pointer messages sent by the server. Increases performance when enabled, as the
    /// client could skip costly software rendering of the pointer with alpha blending
    #[clap(long)]
    no_server_pointer: bool,

    /// Enabled capability versions. Each bit represents enabling a capability version
    /// starting from V8 to V10_7
    #[clap(long, value_parser = parse_hex, default_value_t = 0)]
    capabilities: u32,

    /// Automatically logon to the server by passing the INFO_AUTOLOGON flag
    ///
    /// This flag is ignored if CredSSP authentication is used.
    /// You can use `--no-credssp` to ensure it’s not.
    #[clap(long)]
    autologon: bool,

    /// Disable TLS + Graphical login (legacy authentication method)
    ///
    /// Disabling this in order to enforce usage of CredSSP (NLA) is recommended.
    #[clap(long)]
    no_tls: bool,

    /// Disable TLS + Network Level Authentication (NLA) using CredSSP
    ///
    /// NLA is used to authenticates RDP clients and servers before sending credentials over the network.
    /// It’s not recommended to disable this.
    #[clap(long, alias = "no-nla")]
    no_credssp: bool,

    /// The clipboard type
    #[clap(long, value_enum, default_value_t = ClipboardType::Default)]
    clipboard_type: ClipboardType,

    /// The bitmap codecs to use (remotefx:on, ...)
    #[clap(long, num_args = 1.., value_delimiter = ',')]
    codecs: Vec<String>,

    /// Enable bulk compression support (default: true).
    ///
    /// When enabled, the client advertises support for bulk compression and the
    /// server may send compressed PDUs. Use `--compression-enabled=false` to
    /// disable. When not specified, the value from the `.rdp` file is used (if
    /// present), otherwise compression is enabled by default.
    #[clap(long, action = clap::ArgAction::Set)]
    compression_enabled: Option<bool>,

    /// Bulk compression level to negotiate with the server.
    ///
    /// Valid values:
    ///   0 — MPPC with 8 KB history (RDP 4.0)
    ///   1 — MPPC with 64 KB history (RDP 5.0)
    ///   2 — NCRUSH (RDP 6.0)
    ///   3 — XCRUSH (RDP 6.1)
    #[clap(long, value_parser = clap::value_parser!(u32).range(0..=3), default_value_t = 3)]
    compression_level: u32,

    /// Add DVC channel named pipe proxy
    ///
    /// The format is `<name>=<pipe>`, e.g., `ChannelName=PipeName` where `ChannelName` is the name of the channel,
    /// and `PipeName` is the name of the named pipe to connect to (without OS-specific prefix).
    /// `<pipe>` will automatically be prefixed with `\\.\pipe\` on Windows.
    #[clap(long)]
    dvc_proxy: Vec<DvcProxyInfo>,
    /// Load a DVC client plugin DLL (Windows only).
    ///
    /// Path to a DVC plugin DLL that exports VirtualChannelGetInstance.
    /// Example: C:\Windows\System32\webauthn.dll
    #[cfg(windows)]
    #[clap(long)]
    dvc_plugin: Vec<PathBuf>,

    /// Write the effective PropertySet (merged .rdp file and CLI overrides) to the given path and exit.
    ///
    /// The output is a standard `.rdp` file that can be used as a starting point for customisation
    /// or passed back via `--rdp-file` on the next invocation.
    #[clap(long)]
    dump_rdp: Option<PathBuf>,
}

/// The result of phase 1 parsing: the merged PropertySet plus CLI-only settings.
///
/// After obtaining a `PartialConfig`, callers may inspect or serialise [`PartialConfig::properties`]
/// (e.g., with the `--dump-rdp` flag) before committing to a full session. Call
/// [`PartialConfig::into_config`] to complete phase 2 (interactive prompts + strong typing).
#[derive(Debug)]
pub struct PartialConfig {
    /// The merged PropertySet (`.rdp` file + CLI overrides).
    pub properties: ironrdp_propertyset::PropertySet,

    // CLI-only settings that are not representable as `.rdp` file properties.
    pub log_file: Option<String>,
    pub dump_rdp: Option<PathBuf>,
    pub rdcleanpath: Option<RDCleanPathConfig>,
    pub keyboard_type: KeyboardType,
    pub keyboard_subtype: u32,
    pub keyboard_functional_keys_count: u32,
    pub ime_file_name: String,
    pub dig_product_id: String,
    pub thin_client: bool,
    pub small_cache: bool,
    pub color_depth: Option<u32>,
    pub no_server_pointer: bool,
    pub capabilities: u32,
    pub autologon: bool,
    pub no_tls: bool,
    pub clipboard_type: ClipboardType,
    pub codecs: Vec<String>,
    pub compression_level: u32,
    pub dvc_pipe_proxies: Vec<DvcProxyInfo>,
    #[cfg(windows)]
    pub dvc_plugins: Vec<PathBuf>,
}

impl PartialConfig {
    pub fn parse_args() -> anyhow::Result<Self> {
        Self::parse_from(std::env::args_os())
    }

    pub fn parse_from<I, T>(args: I) -> anyhow::Result<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let args = Args::parse_from(args);

        let mut properties = ironrdp_propertyset::PropertySet::new();

        if let Some(rdp_file) = &args.rdp_file {
            let input =
                std::fs::read_to_string(rdp_file).with_context(|| format!("failed to read {}", rdp_file.display()))?;

            if let Err(errors) = ironrdp_rdpfile::load(&mut properties, &input) {
                for error in &errors {
                    eprintln!("Warning: skipped entry in {}: {error}", rdp_file.display());
                }
            }
        }

        // CLI arguments take precedence: upsert them after the .rdp file is loaded.
        apply_cli_args_to_properties(&mut properties, &args);

        let rdcleanpath = args
            .rdcleanpath_url
            .zip(args.rdcleanpath_token)
            .map(|(url, auth_token)| RDCleanPathConfig { url, auth_token });

        Ok(Self {
            properties,
            log_file: args.log_file,
            dump_rdp: args.dump_rdp,
            rdcleanpath,
            keyboard_type: args.keyboard_type,
            keyboard_subtype: args.keyboard_subtype,
            keyboard_functional_keys_count: args.keyboard_functional_keys_count,
            ime_file_name: args.ime_file_name,
            dig_product_id: args.dig_product_id,
            thin_client: args.thin_client,
            small_cache: args.small_cache,
            color_depth: args.color_depth,
            no_server_pointer: args.no_server_pointer,
            capabilities: args.capabilities,
            autologon: args.autologon,
            no_tls: args.no_tls,
            clipboard_type: args.clipboard_type,
            codecs: args.codecs,
            compression_level: args.compression_level,
            dvc_pipe_proxies: args.dvc_proxy,
            #[cfg(windows)]
            dvc_plugins: args.dvc_plugin,
        })
    }

    pub fn into_config(self) -> anyhow::Result<Config> {
        use ironrdp_cfg::{AudioMode, PropertySetExt as _};

        let properties = &self.properties;

        let use_gateway = gateway_enabled_by_usage_method(
            properties.gateway_usage_method().unwrap_or_else(|e| {
                eprintln!("Warning: {e}, assuming no gateway");
                None
            }),
            properties.gateway_hostname().is_some(),
        );

        let mut gw: Option<GwConnectTarget> =
            use_gateway
                .then(|| properties.gateway_hostname())
                .flatten()
                .map(|gw_addr| GwConnectTarget {
                    gw_endpoint: gw_addr.to_owned(),
                    gw_user: String::new(),
                    gw_pass: String::new(),
                    server: String::new(), // TODO: non-standard port? also dont use here?
                });

        if let Some(ref mut gw) = gw {
            if let Ok(Some(gateway_credentials_source)) = properties.gateway_credentials_source() {
                // All known credential sources fall through to username/password prompts.
                // The value is available for future differentiation if needed.
                let _ = gateway_credentials_source;
            }

            gw.gw_user = if let Some(gw_user) = properties.gateway_username() {
                gw_user.to_owned()
            } else {
                inquire::Text::new("Gateway username:")
                    .prompt()
                    .context("Username prompt")?
            };

            gw.gw_pass = if let Some(gw_pass) = properties.gateway_password() {
                gw_pass.to_owned()
            } else {
                inquire::Password::new("Gateway password:")
                    .without_confirmation()
                    .prompt()
                    .context("Password prompt")?
            };
        };

        let target = match properties.full_address().context("invalid 'full address' property")? {
            Some(addr) => Some(addr),
            None => properties
                .alternate_full_address()
                .context("invalid 'alternate full address' property")?,
        };

        let destination = if let Some(target) = target {
            const RDP_DEFAULT_PORT: u16 = 3389;
            let port = match target.port {
                Some(p) => p,
                None => properties
                    .server_port()
                    .context("invalid 'server port' property")?
                    .unwrap_or(RDP_DEFAULT_PORT),
            };
            let name = match target.host {
                ironrdp_cfg::TargetHost::Ip(ip) => ip.to_string(),
                ironrdp_cfg::TargetHost::Domain(host) => host,
            };
            Destination { name, port }
        } else {
            inquire::Text::new("Server address:")
                .prompt()
                .context("Address prompt")?
                .pipe(Destination::new)?
        };

        if let Some(ref mut gw) = gw {
            gw.server = destination.name.clone(); // TODO
        }

        let username = if let Some(username) = properties.username() {
            username.to_owned()
        } else {
            inquire::Text::new("Username:").prompt().context("Username prompt")?
        };

        let password = if let Some(password) = properties.clear_text_password() {
            password.to_owned()
        } else {
            inquire::Password::new("Password:")
                .without_confirmation()
                .prompt()
                .context("Password prompt")?
        };

        let codecs: Vec<_> = self.codecs.iter().map(|s| s.as_str()).collect();
        let codecs = match client_codecs_capabilities(&codecs) {
            Ok(codecs) => codecs,
            Err(help) => {
                print!("{help}");
                std::process::exit(0);
            }
        };
        let mut bitmap = connector::BitmapConfig {
            color_depth: 32,
            lossy_compression: true,
            codecs,
        };

        if let Some(color_depth) = self.color_depth {
            if color_depth != 16 && color_depth != 32 {
                anyhow::bail!("Invalid color depth. Only 16 and 32 bit color depths are supported.");
            }
            bitmap.color_depth = color_depth;
        };

        let enable_credssp = properties.enable_credssp_support().unwrap_or(true);

        let redirect_clipboard = properties.redirect_clipboard().unwrap_or(true);
        let clipboard_type = if self.clipboard_type == ClipboardType::Default {
            if !redirect_clipboard {
                ClipboardType::None
            } else {
                #[cfg(windows)]
                {
                    ClipboardType::Windows
                }
                #[cfg(not(windows))]
                {
                    ClipboardType::None
                }
            }
        } else {
            self.clipboard_type
        };

        let enable_audio_playback = match properties.audio_mode() {
            Ok(None) | Ok(Some(AudioMode::RedirectToClient)) => true,
            Ok(Some(AudioMode::PlayOnServer | AudioMode::Disabled)) => false,
            Err(e) => {
                eprintln!("Warning: {e}, defaulting to audio playback enabled");
                true
            }
        };

        let compression_enabled = properties.compression().unwrap_or(true);

        let compression_type = if compression_enabled {
            Some(compression_type_from_level(self.compression_level)?)
        } else {
            None
        };

        let desktop_width = properties
            .desktop_width()
            .unwrap_or_else(|_| {
                eprintln!("Warning: ignored out-of-range 'desktopwidth' property");
                None
            })
            .unwrap_or(DEFAULT_WIDTH);
        let desktop_height = properties
            .desktop_height()
            .unwrap_or_else(|_| {
                eprintln!("Warning: ignored out-of-range 'desktopheight' property");
                None
            })
            .unwrap_or(DEFAULT_HEIGHT);
        let desktop_scale_factor = properties
            .desktop_scale_factor()
            .unwrap_or_else(|_| {
                eprintln!("Warning: ignored out-of-range 'desktopscalefactor' property");
                None
            })
            .unwrap_or(0);

        let kdc_proxy_url = properties
            .kdc_proxy_url()
            .map(str::to_owned)
            .or_else(|| properties.kdc_proxy_name().map(normalize_kdc_proxy_url_from_name));

        let kerberos_config = kdc_proxy_url.and_then(|kdc_proxy_url| {
            Url::parse(&kdc_proxy_url)
                .ok()
                .map(|url| connector::credssp::KerberosConfig {
                    kdc_proxy_url: Some(url),
                    // The hostname field is the client computer name used for Kerberos SPN negotiation.
                    hostname: whoami::hostname().unwrap_or_else(|_| "ironrdp".to_owned()),
                })
                .or_else(|| {
                    eprintln!("Warning: ignored invalid KDC proxy URL from .RDP property: {kdc_proxy_url}");
                    None
                })
        });

        let connector = connector::Config {
            credentials: Credentials::UsernamePassword { username, password },
            domain: properties.domain().map(str::to_owned),
            enable_tls: !self.no_tls,
            enable_credssp,
            keyboard_type: KeyboardType::parse(self.keyboard_type),
            keyboard_subtype: self.keyboard_subtype,
            keyboard_layout: 0, // the server SHOULD use the default active input locale identifier
            keyboard_functional_keys_count: self.keyboard_functional_keys_count,
            ime_file_name: self.ime_file_name,
            dig_product_id: self.dig_product_id,
            desktop_size: connector::DesktopSize {
                width: desktop_width,
                height: desktop_height,
            },
            desktop_scale_factor,
            bitmap: Some(bitmap),
            client_build: semver::Version::parse(env!("CARGO_PKG_VERSION"))
                .map_or(0, |version| version.major * 100 + version.minor * 10 + version.patch)
                .pipe(u32::try_from)
                .context("cargo package version")?,
            client_name: whoami::hostname().unwrap_or_else(|_| "ironrdp".to_owned()),
            // NOTE: hardcode this value like in freerdp
            // https://github.com/FreeRDP/FreeRDP/blob/4e24b966c86fdf494a782f0dfcfc43a057a2ea60/libfreerdp/core/settings.c#LL49C34-L49C70
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
            enable_audio_playback,
            request_data: None,
            pointer_software_rendering: false,
            multitransport_flags: None,
            compression_type,
            performance_flags: PerformanceFlags::default(),
            timezone_info: TimezoneInfo::default(),
            alternate_shell: properties.alternate_shell().unwrap_or_default().to_owned(),
            work_dir: properties.shell_working_directory().unwrap_or_default().to_owned(),
        };

        Ok(Config {
            log_file: self.log_file,
            gw,
            kerberos_config,
            destination,
            connector,
            clipboard_type,
            rdcleanpath: self.rdcleanpath,
            dvc_pipe_proxies: self.dvc_pipe_proxies,
            #[cfg(windows)]
            dvc_plugins: self.dvc_plugins,
        })
    }
}

impl Config {
    pub fn parse_args() -> anyhow::Result<Self> {
        Self::parse_from(std::env::args_os())
    }

    pub fn parse_from<I, T>(args: I) -> anyhow::Result<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        PartialConfig::parse_from(args)?.into_config()
    }
}

fn normalize_kdc_proxy_url_from_name(name: &str) -> String {
    if name.starts_with("http://") || name.starts_with("https://") {
        name.to_owned()
    } else {
        format!("https://{name}/KdcProxy")
    }
}

fn gateway_enabled_by_usage_method(method: Option<ironrdp_cfg::GatewayUsageMethod>, has_gateway_host: bool) -> bool {
    method.map_or(has_gateway_host, ironrdp_cfg::GatewayUsageMethod::is_gateway_required)
}
