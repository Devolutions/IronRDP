#![allow(clippy::print_stdout, clippy::print_stderr)]

use core::time::Duration;
use std::path::PathBuf;

use anyhow::Context as _;
use clap::Parser;
use clap::clap_derive::ValueEnum;
use ironrdp::client::config::{
    ClipboardType as ResolvedClipboardType, Config, ConfigBuilder, Destination, DvcProxyInfo, MissingField,
    TransportKind,
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
    ///
    /// The accompanying token may be supplied via `--rdcleanpath-token` or entered interactively.
    #[clap(long)]
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

    /// Disable bulk compression support.
    ///
    /// By default the client advertises support for bulk compression and the
    /// server may send compressed PDUs. Pass `--no-compression` to disable it.
    /// When not specified, the value from the `.rdp` file is used (if present),
    /// otherwise compression is enabled by default.
    #[clap(long)]
    no_compression: bool,

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

/// Result of parsing CLI args + loading the `.rdp` file: a configured [`ConfigBuilder`] plus the
/// CLI-only settings that cannot live on the builder.
///
/// Call [`ViewerConfig::into_config`] to resolve the remaining required fields (interactive prompts
/// + frontend-derived client identity) and build the strongly-typed [`Config`].
pub struct ViewerConfig {
    builder: ConfigBuilder,

    // CLI-only settings that are not representable as `.rdp` file properties.
    log_file: Option<String>,
    dump_rdp: Option<PathBuf>,
}

impl ViewerConfig {
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

        let log_file = args.log_file.clone();
        let dump_rdp = args.dump_rdp.clone();

        // The library overlays everything expressible as a `.rdp` property: destination, credentials,
        // transport, channels, desktop size, audio, DVC proxies, etc.
        let builder = ConfigBuilder::from_property_set(&properties)?;

        // Whether the `.rdp` file requested clipboard redirection; the CLI `--clipboard-type` is
        // resolved against this when applied below.
        let redirect_clipboard = properties.redirect_clipboard().unwrap_or(true);

        // CLI arguments take precedence: apply them on top of the `.rdp`-derived builder.
        let builder = apply_cli_to_builder(builder, args, redirect_clipboard);

        Ok(Self {
            builder,
            log_file,
            dump_rdp,
        })
    }

    pub fn into_config(self) -> anyhow::Result<Config> {
        // When dumping, the built config is only used to observe the effective, secret-stripped
        // PropertySet; we never start a session. Secrets are stripped on `build()` anyway, so there
        // is no point prompting for them: fill a placeholder instead.
        let dump = self.dump_rdp.is_some();
        prompt_missing(self.builder, dump)
    }

    /// Path to the log file requested on the CLI, if any.
    pub fn log_file(&self) -> Option<&str> {
        self.log_file.as_deref()
    }

    /// Path to dump the effective `.rdp` PropertySet to, if `--dump-rdp` was given.
    pub fn dump_rdp(&self) -> Option<&std::path::Path> {
        self.dump_rdp.as_deref()
    }
}

/// Apply CLI overrides on top of a builder that already reflects the `.rdp` file. Every flag that is
/// present overwrites the corresponding builder (and mirrored property) value.
fn apply_cli_to_builder(mut builder: ConfigBuilder, args: Args, redirect_clipboard: bool) -> ConfigBuilder {
    // Validate the codecs early to surface help text before connecting.
    {
        let codecs: Vec<_> = args.codecs.iter().map(String::as_str).collect();
        if let Err(help) = client_codecs_capabilities(&codecs) {
            print!("{help}");
            std::process::exit(0);
        }
    }

    if let Some(destination) = args.destination {
        builder = builder.with_destination(destination);
    }
    if let Some(username) = args.username {
        builder = builder.with_username(username);
    }
    if let Some(password) = args.password {
        builder = builder.with_password(password);
    }
    if let Some(domain) = args.domain {
        builder = builder.with_domain(domain);
    }
    if let Some(scale) = args.scale_desktop {
        builder = builder.with_desktop_scale_factor(scale);
    }
    if let Some(width) = args.desktop_width {
        builder = builder.with_desktop_width(width);
    }
    if let Some(height) = args.desktop_height {
        builder = builder.with_desktop_height(height);
    }
    if let Some(color_depth) = args.color_depth {
        builder = builder.with_color_depth(color_depth);
    }
    if args.no_credssp {
        builder = builder.with_credssp(false);
    }
    if args.no_tls {
        builder = builder.with_tls(false);
    }
    if args.no_server_pointer {
        builder = builder.with_server_pointer(false);
    }
    if args.autologon {
        builder = builder.with_autologon(true);
    }
    if args.no_compression {
        builder = builder.with_compression(false);
    }
    if let Some(level) = args.compression_level {
        builder = builder.with_compression_level(level);
    }
    if let Some(minutes) = args.prevent_session_lock {
        builder = builder.with_fake_events_interval(Duration::from_secs(u64::from(minutes) * 60));
    }

    // Transport overrides: RDCleanPath takes precedence over Gateway.
    if let Some(url) = args.rdcleanpath_url {
        builder = builder.with_transport(TransportKind::RDCleanPath { url });

        if let Some(token) = args.rdcleanpath_token {
            builder = builder.with_rdcleanpath_token(token);
        }
    } else if let Some(endpoint) = args.gw_endpoint {
        builder = builder.with_transport(TransportKind::Gateway { endpoint });

        if let Some(username) = args.gw_user {
            builder = builder.with_gateway_username(username);
        }
        if let Some(password) = args.gw_pass {
            builder = builder.with_gateway_password(password);
        }
    }

    builder = builder.with_clipboard(resolve_clipboard_type(args.clipboard_type, redirect_clipboard));

    // CLI-only knobs that are not representable as `.rdp` properties.
    // TODO/FIXME: Some of these, we may want to add support for storing in .rdp files (e.g.: IME file name can be reasonably seen as a connection option)
    builder = builder
        .with_keyboard_type(args.keyboard_type.into_pdu())
        .with_keyboard_subtype(args.keyboard_subtype)
        .with_keyboard_functional_keys_count(args.keyboard_functional_keys_count)
        .with_ime_file_name(args.ime_file_name)
        .with_dig_product_id(args.dig_product_id)
        .with_codecs(args.codecs);

    for proxy in args.dvc_proxy {
        builder = builder.with_dvc_pipe_proxy(proxy);
    }

    #[cfg(windows)]
    for plugin in args.dvc_plugin {
        builder = builder.with_dvc_plugin(plugin);
    }

    builder
}

/// Resolve the remaining [`MissingField`]s by prompting for credentials/addresses and deriving the
/// frontend-specific client identity, then build the [`Config`].
///
/// When `dump` is set, the resulting config is only used to observe the effective, secret-stripped
/// PropertySet (no session is started). Secret fields are stripped on `build()` regardless, so they
/// are filled with a placeholder instead of being prompted for.
fn prompt_missing(mut builder: ConfigBuilder, dump: bool) -> anyhow::Result<Config> {
    // Stripped on `build()`, so any value works when only dumping the PropertySet.
    const DUMP_SECRET_PLACEHOLDER: &str = "<stripped>";

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
            MissingField::Password if dump => builder.with_password(DUMP_SECRET_PLACEHOLDER),
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
            MissingField::GatewayPassword if dump => builder.with_gateway_password(DUMP_SECRET_PLACEHOLDER),
            MissingField::GatewayPassword => {
                let password = inquire::Password::new("Gateway password:")
                    .without_confirmation()
                    .prompt()
                    .context("Gateway password prompt")?;
                builder.with_gateway_password(password)
            }
            MissingField::RDCleanPathToken if dump => builder.with_rdcleanpath_token(DUMP_SECRET_PLACEHOLDER),
            MissingField::RDCleanPathToken => {
                let token = inquire::Text::new("RDCleanPath token:")
                    .prompt()
                    .context("RDCleanPath token prompt")?;
                builder.with_rdcleanpath_token(token)
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
    ViewerConfig::parse_args()?.into_config()
}

pub fn parse_config_from<I, T>(args: I) -> anyhow::Result<Config>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    ViewerConfig::parse_from(args)?.into_config()
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
