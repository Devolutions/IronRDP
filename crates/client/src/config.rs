use std::num::ParseIntError;

use anyhow::Context as _;
use clap::clap_derive::ValueEnum;
use clap::{crate_name, Parser};
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::{connector, pdu};
use tap::prelude::*;

const DEFAULT_WIDTH: u16 = 1920;
const DEFAULT_HEIGHT: u16 = 1080;

#[derive(Clone)]
pub struct Config {
    pub log_file: String,
    pub addr: String,
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

/// Devolutions IronRDP client
#[derive(Parser, Debug)]
#[clap(author = "Devolutions", about = "Devolutions-IronRDP client")]
#[clap(version, long_about = None)]
struct Args {
    /// A file with IronRDP client logs
    #[clap(short, long, value_parser, default_value_t = format!("{}.log", crate_name!()))]
    log_file: String,

    /// An address on which the client will connect.
    addr: Option<String>,

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
    #[clap(long, value_enum, value_parser, default_value_t = SecurityProtocol::HybridEx)]
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

    /// Enable RDP6 lossy bitmap compression algorithm. Please note that lossy compression
    /// only works with 32 bit color depth
    #[clap(long)]
    lossy_bitmap_compression: bool,

    /// Set required color depth. Currently only 32 and 16 bit color depths are supported
    #[clap(long)]
    color_depth: Option<u32>,

    /// Enabled capability versions. Each bit represents enabling a capability version
    /// starting from V8 to V10_7
    #[clap(long, value_parser = parse_hex, default_value_t = 0)]
    capabilities: u32,
}

impl Config {
    pub fn parse_args() -> anyhow::Result<Self> {
        let args = Args::parse();

        let mut addr = if let Some(addr) = args.addr {
            addr
        } else {
            inquire::Text::new("Server address:")
                .prompt()
                .context("Address prompt")?
        };

        if addr.find(':').is_none() {
            addr.push_str(":3389");
        }

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

            if color_depth != 32 && args.lossy_bitmap_compression {
                anyhow::bail!("Lossy bitmap compression only works with 32 bit color depth.");
            }

            Some(connector::BitmapConfig {
                color_depth,
                lossy_compression: args.lossy_bitmap_compression,
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
            username,
            password,
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
            client_dir: std::env::current_dir()
                .expect("current directory")
                .to_string_lossy()
                .into_owned(),
            platform: match whoami::platform() {
                whoami::Platform::Windows => MajorPlatformType::Windows,
                whoami::Platform::Linux => MajorPlatformType::Unix,
                whoami::Platform::MacOS => MajorPlatformType::Macintosh,
                whoami::Platform::Ios => MajorPlatformType::IOs,
                whoami::Platform::Android => MajorPlatformType::Android,
                _ => MajorPlatformType::Unspecified,
            },
        };

        Ok(Self {
            log_file: args.log_file,
            addr,
            connector,
        })
    }
}
