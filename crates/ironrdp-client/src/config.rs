use std::io;
use std::num::ParseIntError;
use std::str::FromStr;

use anyhow::Context as _;
use clap::clap_derive::ValueEnum;
use clap::{crate_name, Parser};
use ironrdp::connector::Credentials;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::{connector, pdu};
use tap::prelude::*;

const DEFAULT_WIDTH: u16 = 1920;
const DEFAULT_HEIGHT: u16 = 1080;

#[derive(Clone, Debug)]
pub struct Config {
    pub log_file: String,
    pub destination: Destination,
    pub connector: connector::Config,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum SecurityProtocol {
    Ssl,
    Hybrid,
    HybridEx,
}

impl SecurityProtocol {
    fn parse(security_protocol: SecurityProtocol) -> pdu::nego::SecurityProtocol {
        match security_protocol {
            SecurityProtocol::Ssl => pdu::nego::SecurityProtocol::SSL,
            SecurityProtocol::Hybrid => pdu::nego::SecurityProtocol::HYBRID,
            SecurityProtocol::HybridEx => pdu::nego::SecurityProtocol::HYBRID_EX,
        }
    }
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

    pub fn lookup_addr(&self) -> io::Result<std::net::SocketAddr> {
        use std::net::ToSocketAddrs as _;

        let sockaddr = (self.name.as_str(), self.port).to_socket_addrs()?.next().unwrap();

        Ok(sockaddr)
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

/// Devolutions IronRDP client
#[derive(Parser, Debug)]
#[clap(author = "Devolutions", about = "Devolutions-IronRDP client")]
#[clap(version, long_about = None)]
struct Args {
    /// A file with IronRDP client logs
    #[clap(short, long, value_parser, default_value_t = format!("{}.log", crate_name!()))]
    log_file: String,

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

    /// Specify the security protocols to use
    #[clap(long, value_enum, value_parser, default_value_t = SecurityProtocol::Hybrid)]
    security_protocol: SecurityProtocol,

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

    /// Enable AVC444
    #[clap(long, group = "avc")]
    avc444: bool,

    /// Enable H264
    #[clap(long, group = "avc")]
    h264: bool,

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

        let bitmap = if let Some(color_depth) = args.color_depth {
            if color_depth != 16 && color_depth != 32 {
                anyhow::bail!("Invalid color depth. Only 16 and 32 bit color depths are supported.");
            }

            Some(connector::BitmapConfig {
                color_depth,
                lossy_compression: true,
            })
        } else {
            None
        };

        let graphics = if args.avc444 || args.h264 {
            Some(connector::GraphicsConfig {
                avc444: args.avc444,
                h264: args.h264,
                thin_client: args.thin_client,
                small_cache: args.small_cache,
                capabilities: args.capabilities,
            })
        } else {
            None
        };

        let connector = connector::Config {
            credentials: Credentials::UsernamePassword { username, password },
            domain: args.domain,
            security_protocol: SecurityProtocol::parse(args.security_protocol),
            keyboard_type: KeyboardType::parse(args.keyboard_type),
            keyboard_subtype: args.keyboard_subtype,
            keyboard_functional_keys_count: args.keyboard_functional_keys_count,
            ime_file_name: args.ime_file_name,
            dig_product_id: args.dig_product_id,
            desktop_size: connector::DesktopSize {
                width: DEFAULT_WIDTH,
                height: DEFAULT_HEIGHT,
            },
            graphics,
            bitmap,
            client_build: semver::Version::parse(env!("CARGO_PKG_VERSION"))
                .map(|version| version.major * 100 + version.minor * 10 + version.patch)
                .unwrap_or(0)
                .pipe(u32::try_from)
                .unwrap(),
            client_name: whoami::hostname(),
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
            no_server_pointer: args.no_server_pointer,
            auto_login: false,
        };

        Ok(Self {
            log_file: args.log_file,
            destination,
            connector,
        })
    }
}
