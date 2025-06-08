#![allow(clippy::print_stdout)]
use core::num::ParseIntError;
use core::str::FromStr;

use anyhow::Context as _;
use clap::clap_derive::ValueEnum;
use clap::Parser;
use ironrdp::connector::{self, Credentials};
use ironrdp::pdu::rdp::capability_sets::{client_codecs_capabilities, MajorPlatformType};
use ironrdp::pdu::rdp::client_info::PerformanceFlags;
use tap::prelude::*;
use url::Url;

const DEFAULT_WIDTH: u16 = 1920;
const DEFAULT_HEIGHT: u16 = 1080;

#[derive(Clone, Debug)]
pub struct Config {
    pub log_file: Option<String>,
    pub destination: Destination,
    pub connector: connector::Config,
    pub clipboard_type: ClipboardType,
    pub rdcleanpath: Option<RDCleanPathConfig>,
    pub pcb: Option<PreconnectionBlobPayload>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum ClipboardType {
    Default,
    Stub,
    #[cfg(windows)]
    Windows,
    None,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum KeyboardType {
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

        if let Some(idx) = addr.rfind(':') {
            if let Ok(sock_addr) = addr.parse::<std::net::SocketAddr>() {
                Ok(Self {
                    name: sock_addr.ip().to_string(),
                    port: sock_addr.port(),
                })
            } else if addr.parse::<std::net::Ipv6Addr>().is_ok() {
                Ok(Self {
                    name: addr,
                    port: RDP_DEFAULT_PORT,
                })
            } else {
                Ok(Self {
                    name: addr[..idx].to_owned(),
                    port: addr[idx + 1..].parse().context("invalid port")?,
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

/// Devolutions IronRDP client
#[derive(Parser, Debug)]
#[clap(author = "Devolutions", about = "Devolutions-IronRDP client")]
#[clap(version, long_about = None)]
struct Args {
    /// A file with IronRDP client logs
    #[clap(short, long, value_parser)]
    log_file: Option<String>,

    /// An address on which the client will connect.
    destination: Option<Destination>,

    /// A target RDP server user name
    #[clap(short, long, value_parser)]
    username: Option<String>,

    /// An optional target RDP server domain name
    #[clap(short, long, value_parser)]
    domain: Option<String>,

    /// A target RDP server user password
    #[clap(short, long, value_parser)]
    password: Option<String>,

    /// Proxy URL to connect to for the RDCleanPath
    #[clap(long, requires("rdcleanpath_token"))]
    rdcleanpath_url: Option<Url>,

    /// Authentication token to insert in the RDCleanPath packet
    #[clap(long, requires("rdcleanpath_url"))]
    rdcleanpath_token: Option<String>,

    /// The keyboard type
    #[clap(long, value_enum, value_parser, default_value_t = KeyboardType::IbmEnhanced)]
    keyboard_type: KeyboardType,

    /// The keyboard subtype (an original equipment manufacturer-dependent value)
    #[clap(long, value_parser, default_value_t = 0)]
    keyboard_subtype: u32,

    /// The number of function keys on the keyboard
    #[clap(long, value_parser, default_value_t = 12)]
    keyboard_functional_keys_count: u32,

    /// The input method editor (IME) file name associated with the active input locale
    #[clap(long, value_parser, default_value_t = String::from(""))]
    ime_file_name: String,

    /// Contains a value that uniquely identifies the client
    #[clap(long, value_parser, default_value_t = String::from(""))]
    dig_product_id: String,

    /// Enable thin client
    #[clap(long)]
    thin_client: bool,

    /// Enable small cache
    #[clap(long)]
    small_cache: bool,

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
    #[clap(long, value_enum, value_parser, default_value_t = ClipboardType::Default)]
    clipboard_type: ClipboardType,

    /// The bitmap codecs to use (remotefx:on, ...)
    #[clap(long, value_parser, num_args = 1.., value_delimiter = ',')]
    codecs: Vec<String>,

    /// The ID for the HyperV VM server to connect to
    #[clap(long, value_parser)]
    vmconnect: Option<String>,

    /// Preconnection Blob payload to use, cannot be used with `--vmconnect`
    #[clap(long, value_parser)]
    pcb: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreconnectionBlobPayload {
    General(String),
    VmConnect(String),
}

impl PreconnectionBlobPayload {
    pub fn general(&self) -> Option<&str> {
        match self {
            PreconnectionBlobPayload::General(pcb) => Some(pcb),
            PreconnectionBlobPayload::VmConnect(_) => None,
        }
    }

    pub fn vmconnect(&self) -> Option<&str> {
        match self {
            PreconnectionBlobPayload::VmConnect(vm_id) => Some(vm_id),
            PreconnectionBlobPayload::General(_) => None,
        }
    }
}

impl Config {
    pub fn parse_args() -> anyhow::Result<Self> {
        let args = Args::parse();

        let destination = if let Some(destination) = args.destination {
            destination
        } else {
            inquire::Text::new("Server address:")
                .prompt()
                .context("Address prompt")?
                .pipe(Destination::new)?
        };

        let username = if let Some(username) = args.username {
            username
        } else {
            inquire::Text::new("Username:").prompt().context("Username prompt")?
        };

        let password = if let Some(password) = args.password {
            password
        } else {
            inquire::Password::new("Password:")
                .without_confirmation()
                .prompt()
                .context("Password prompt")?
        };

        let codecs: Vec<_> = args.codecs.iter().map(|s| s.as_str()).collect();
        let codecs = match client_codecs_capabilities(&codecs) {
            Ok(codecs) => codecs,
            Err(help) => {
                print!("{}", help);
                std::process::exit(0);
            }
        };
        let mut bitmap = connector::BitmapConfig {
            color_depth: 32,
            lossy_compression: true,
            codecs,
        };

        if let Some(color_depth) = args.color_depth {
            if color_depth != 16 && color_depth != 32 {
                anyhow::bail!("Invalid color depth. Only 16 and 32 bit color depths are supported.");
            }
            bitmap.color_depth = color_depth;
        };

        let clipboard_type = if args.clipboard_type == ClipboardType::Default {
            #[cfg(windows)]
            {
                ClipboardType::Windows
            }
            #[cfg(not(windows))]
            {
                ClipboardType::None
            }
        } else {
            args.clipboard_type
        };

        let connector = connector::Config {
            credentials: Credentials::UsernamePassword { username, password },
            domain: args.domain,
            enable_tls: !args.no_tls,
            enable_credssp: !args.no_credssp,
            keyboard_type: KeyboardType::parse(args.keyboard_type),
            keyboard_subtype: args.keyboard_subtype,
            keyboard_layout: 0, // the server SHOULD use the default active input locale identifier
            keyboard_functional_keys_count: args.keyboard_functional_keys_count,
            ime_file_name: args.ime_file_name,
            dig_product_id: args.dig_product_id,
            desktop_size: connector::DesktopSize {
                width: DEFAULT_WIDTH,
                height: DEFAULT_HEIGHT,
            },
            desktop_scale_factor: 0, // Default to 0 per FreeRDP
            bitmap: Some(bitmap),
            client_build: semver::Version::parse(env!("CARGO_PKG_VERSION"))
                .map(|version| version.major * 100 + version.minor * 10 + version.patch)
                .unwrap_or(0)
                .pipe(u32::try_from)
                .unwrap(),
            client_name: whoami::fallible::hostname().unwrap_or_else(|_| "ironrdp".to_owned()),
            // NOTE: hardcode this value like in freerdp
            // https://github.com/FreeRDP/FreeRDP/blob/4e24b966c86fdf494a782f0dfcfc43a057a2ea60/libfreerdp/core/settings.c#LL49C34-L49C70
            client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),
            platform: match whoami::platform() {
                whoami::Platform::Windows => MajorPlatformType::WINDOWS,
                whoami::Platform::Linux => MajorPlatformType::UNIX,
                whoami::Platform::MacOS => MajorPlatformType::MACINTOSH,
                whoami::Platform::Ios => MajorPlatformType::IOS,
                whoami::Platform::Android => MajorPlatformType::ANDROID,
                _ => MajorPlatformType::UNSPECIFIED,
            },
            hardware_id: None,
            license_cache: None,
            no_server_pointer: args.no_server_pointer,
            autologon: args.autologon,
            no_audio_playback: false,
            request_data: None,
            pointer_software_rendering: true,
            performance_flags: PerformanceFlags::default(),
        };

        let rdcleanpath = args
            .rdcleanpath_url
            .zip(args.rdcleanpath_token)
            .map(|(url, auth_token)| RDCleanPathConfig { url, auth_token });

        let pcb = match (args.vmconnect, args.pcb) {
            (Some(_), Some(_)) => {
                anyhow::bail!("Cannot use both --vmconnect and --pcb options. Choose one of them.");
            }
            (Some(vm_id), None) => Some(PreconnectionBlobPayload::VmConnect(vm_id)),
            (None, Some(pcb)) => Some(PreconnectionBlobPayload::General(pcb)),
            (None, None) => None,
        };

        Ok(Self {
            log_file: args.log_file,
            destination,
            connector,
            clipboard_type,
            rdcleanpath,
            pcb,
        })
    }
}
