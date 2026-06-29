#![allow(clippy::print_stdout, clippy::print_stderr)]

use core::num::ParseIntError;
use std::path::PathBuf;

use anyhow::Context as _;
use clap::Parser;
use clap::clap_derive::ValueEnum;
use ironrdp::client::config::{
    ClipboardType as ResolvedClipboardType, Config, ConfigBuilder, Destination, DvcProxyInfo, MissingField,
};
use ironrdp::pdu::rdp::capability_sets::{MajorPlatformType, client_codecs_capabilities};
use ironrdp_cfg::PropertySetExt as _;
use tap::prelude::*;
use url::Url;

/// CLI selection for the clipboard backend.
///
/// Maps directly into the library's [`ResolvedClipboardType`] when the typed [`Config`] is built.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum ClipboardType {
    /// Enable clipboard redirection (use the best available backend).
    Enable,
    /// Disable clipboard redirection entirely.
    Disable,
    /// Use a stub clipboard backend (for testing or headless usage).
    Stub,
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
    fn into_pdu(self) -> ironrdp::pdu::gcc::KeyboardType {
        match self {
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

    if let Some(width) = args.desktop_width {
        properties.insert("desktopwidth", i64::from(width));
    }

    if let Some(height) = args.desktop_height {
        properties.insert("desktopheight", i64::from(height));
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
        properties.set_enable_credssp_support(false);
    }

    if args.no_tls {
        properties.set_enable_tls(false);
    }

    if args.no_server_pointer {
        properties.set_server_pointer(false);
    }

    if args.autologon {
        properties.set_autologon(true);
    }

    if let Some(enabled) = args.compression_enabled {
        properties.set_compression(enabled);
    }

    if let Some(level) = args.compression_level {
        properties.set_compression_level(level);
    }

    if let Some(color_depth) = args.color_depth {
        properties.set_color_depth(color_depth);
    }

    #[cfg(windows)]
    if !args.dvc_plugin.is_empty() {
        let value = args
            .dvc_plugin
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(",");
        properties.set_dvc_plugins(value);
    }

    if let Some(url) = &args.rdcleanpath_url {
        properties.set_rdcleanpath_url(url.as_str());
    }

    if let Some(token) = &args.rdcleanpath_token {
        properties.set_rdcleanpath_token(token.as_str());
    }

    if let Some(minutes) = args.prevent_session_lock {
        properties.set_fake_events_interval(minutes);
    }

    if !args.dvc_proxy.is_empty() {
        let value = args
            .dvc_proxy
            .iter()
            .map(|p| format!("{}={}", p.channel_name, p.pipe_name))
            .collect::<Vec<_>>()
            .join(",");
        properties.set_dvc_pipe_proxies(value);
    }
}

fn parse_hex(input: &str) -> Result<u32, ParseIntError> {
    if input.starts_with("0x") {
        u32::from_str_radix(input.get(2..).unwrap_or(""), 16)
    } else {
        input.parse::<u32>()
    }
}

/// Devolutions IronRDP viewer
#[derive(Parser, Debug)]
#[clap(author = "Devolutions", about = "Devolutions-IronRDP viewer")]
#[clap(version, long_about = None)]
struct Args {
    /// A file with IronRDP viewer logs
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

    /// Desired desktop width for the RDP session
    #[clap(long, value_parser = clap::value_parser!(u16).range(1..=8192))]
    desktop_width: Option<u16>,

    /// Desired desktop height for the RDP session
    #[clap(long, value_parser = clap::value_parser!(u16).range(1..=8192))]
    desktop_height: Option<u16>,

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
    /// You can use `--no-credssp` to ensure it's not.
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
    /// It's not recommended to disable this.
    #[clap(long, alias = "no-nla")]
    no_credssp: bool,

    /// The clipboard type
    #[clap(long, value_enum, default_value_t = ClipboardType::Enable)]
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
    #[clap(long, value_parser = clap::value_parser!(u32).range(0..=3))]
    compression_level: Option<u32>,

    /// Prevents session locking by injecting fake mouse movement events when
    /// the connection is idle (interval in minutes)
    #[clap(long)]
    prevent_session_lock: Option<u32>,

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
    pub keyboard_type: KeyboardType,
    pub keyboard_subtype: u32,
    pub keyboard_functional_keys_count: u32,
    pub ime_file_name: String,
    pub dig_product_id: String,
    pub thin_client: bool,
    pub small_cache: bool,
    pub capabilities: u32,
    pub clipboard_type: ClipboardType,
    pub codecs: Vec<String>,
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

        Ok(Self {
            properties,
            log_file: args.log_file,
            dump_rdp: args.dump_rdp,
            keyboard_type: args.keyboard_type,
            keyboard_subtype: args.keyboard_subtype,
            keyboard_functional_keys_count: args.keyboard_functional_keys_count,
            ime_file_name: args.ime_file_name,
            dig_product_id: args.dig_product_id,
            thin_client: args.thin_client,
            small_cache: args.small_cache,
            capabilities: args.capabilities,
            clipboard_type: args.clipboard_type,
            codecs: args.codecs,
        })
    }

    pub fn into_config(self) -> anyhow::Result<Config> {
        use ironrdp_cfg::PropertySetExt as _;

        // The library overlays everything expressible as a `.rdp` property: destination, credentials,
        // transport, channels, desktop size, audio, DVC proxies, etc.
        let mut builder = ConfigBuilder::from_property_set(&self.properties)?;

        // CLI-only knobs that are not representable as `.rdp` properties.
        builder = builder
            .with_keyboard_type(self.keyboard_type.into_pdu())
            .with_keyboard_subtype(self.keyboard_subtype)
            .with_keyboard_functional_keys_count(self.keyboard_functional_keys_count)
            .with_ime_file_name(self.ime_file_name)
            .with_dig_product_id(self.dig_product_id)
            .with_codecs(self.codecs.clone());

        // Validate the codecs early to surface help text before connecting.
        let codecs: Vec<_> = self.codecs.iter().map(String::as_str).collect();
        if let Err(help) = client_codecs_capabilities(&codecs) {
            print!("{help}");
            std::process::exit(0);
        }

        let redirect_clipboard = self.properties.redirect_clipboard().unwrap_or(true);
        builder = builder.with_clipboard(resolve_clipboard_type(self.clipboard_type, redirect_clipboard));

        prompt_missing(builder)
    }
}

/// Resolve the remaining [`MissingField`]s by prompting for credentials/addresses and deriving the
/// frontend-specific client identity, then build the [`Config`].
fn prompt_missing(mut builder: ConfigBuilder) -> anyhow::Result<Config> {
    for field in builder.missing() {
        builder = match field {
            MissingField::ServerAddress => {
                let dest = inquire::Text::new("Server address:")
                    .prompt()
                    .context("Address prompt")?
                    .pipe(Destination::new)?;
                builder.with_destination(dest)
            }
            MissingField::Username => {
                let username = inquire::Text::new("Username:").prompt().context("Username prompt")?;
                builder.with_username(username)
            }
            MissingField::Password => {
                let password = inquire::Password::new("Password:")
                    .without_confirmation()
                    .prompt()
                    .context("Password prompt")?;
                builder.with_password(password)
            }
            MissingField::GatewayUsername => {
                let username = inquire::Text::new("Gateway username:")
                    .prompt()
                    .context("Gateway username prompt")?;
                builder.with_gateway_username(username)
            }
            MissingField::GatewayPassword => {
                let password = inquire::Password::new("Gateway password:")
                    .without_confirmation()
                    .prompt()
                    .context("Gateway password prompt")?;
                builder.with_gateway_password(password)
            }
            // Frontend-derived identity: never prompted.
            MissingField::ClientBuild => builder.with_client_build(client_build()),
            MissingField::ClientDir => {
                // NOTE: hardcode this value like in freerdp
                // https://github.com/FreeRDP/FreeRDP/blob/4e24b966c86fdf494a782f0dfcfc43a057a2ea60/libfreerdp/core/settings.c#LL49C34-L49C70
                builder.with_client_dir("C:\\Windows\\System32\\mstscax.dll")
            }
            MissingField::Platform => builder.with_platform(current_platform()),
            MissingField::ClientName => builder.with_client_name(client_name()),
        };
    }

    builder.build()
}

fn client_build() -> u32 {
    semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .map_or(0, |v| v.major * 100 + v.minor * 10 + v.patch)
        .try_into()
        .unwrap_or(0)
}

fn client_name() -> String {
    whoami::hostname().unwrap_or_else(|_| "ironrdp".to_owned())
}

fn current_platform() -> MajorPlatformType {
    match whoami::platform() {
        whoami::Platform::Windows => MajorPlatformType::WINDOWS,
        whoami::Platform::Linux => MajorPlatformType::UNIX,
        whoami::Platform::Mac => MajorPlatformType::MACINTOSH,
        whoami::Platform::Ios => MajorPlatformType::IOS,
        whoami::Platform::Android => MajorPlatformType::ANDROID,
        _ => MajorPlatformType::UNSPECIFIED,
    }
}

pub fn parse_config() -> anyhow::Result<Config> {
    PartialConfig::parse_args()?.into_config()
}

pub fn parse_config_from<I, T>(args: I) -> anyhow::Result<Config>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    PartialConfig::parse_from(args)?.into_config()
}

fn resolve_clipboard_type(cli: ClipboardType, redirect_clipboard: bool) -> ResolvedClipboardType {
    if !redirect_clipboard {
        return ResolvedClipboardType::Disable;
    }

    match cli {
        ClipboardType::Enable => ResolvedClipboardType::Enable,
        ClipboardType::Disable => ResolvedClipboardType::Disable,
        ClipboardType::Stub => ResolvedClipboardType::Stub,
    }
}
