use std::num::ParseIntError;

use anyhow::Context as _;
use clap::clap_derive::ValueEnum;
use clap::{crate_name, Parser};
use ironrdp::session::{GraphicsConfig, InputConfig};
use sspi::AuthIdentity;

const DEFAULT_WIDTH: u16 = 1920;
const DEFAULT_HEIGHT: u16 = 1080;
const GLOBAL_CHANNEL_NAME: &str = "GLOBAL";
const USER_CHANNEL_NAME: &str = "USER";

#[derive(Clone)]
pub struct Config {
    pub log_file: String,
    pub addr: String,
    pub input: InputConfig,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum SecurityProtocol {
    Ssl,
    Hybrid,
    HybridEx,
}

impl SecurityProtocol {
    fn parse(security_protocol: SecurityProtocol) -> ironrdp::core::SecurityProtocol {
        match security_protocol {
            SecurityProtocol::Ssl => ironrdp::core::SecurityProtocol::SSL,
            SecurityProtocol::Hybrid => ironrdp::core::SecurityProtocol::HYBRID,
            SecurityProtocol::HybridEx => ironrdp::core::SecurityProtocol::HYBRID_EX,
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
    fn parse(keyboard_type: KeyboardType) -> ironrdp::gcc::KeyboardType {
        match keyboard_type {
            KeyboardType::IbmEnhanced => ironrdp::gcc::KeyboardType::IbmEnhanced,
            KeyboardType::IbmPcAt => ironrdp::gcc::KeyboardType::IbmPcAt,
            KeyboardType::IbmPcXt => ironrdp::gcc::KeyboardType::IbmPcXt,
            KeyboardType::OlivettiIco => ironrdp::gcc::KeyboardType::OlivettiIco,
            KeyboardType::Nokia1050 => ironrdp::gcc::KeyboardType::Nokia1050,
            KeyboardType::Nokia9140 => ironrdp::gcc::KeyboardType::Nokia9140,
            KeyboardType::Japanese => ironrdp::gcc::KeyboardType::Japanese,
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

    /// Enabled capability versions. Each bit represents enabling a capability version
    /// starting from V8 to V10_7
    #[clap(long, value_parser = parse_hex, default_value_t = 0)]
    capabilities: u32,
}

impl Config {
    pub fn parse_args() -> anyhow::Result<Self> {
        let args = Args::parse();

        let addr = if let Some(addr) = args.addr {
            addr
        } else {
            inquire::Text::new("Server address:")
                .prompt()
                .context("Address prompt")?
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

        let graphics_config = if args.avc444 || args.h264 {
            Some(GraphicsConfig {
                avc444: args.avc444,
                h264: args.h264,
                thin_client: args.thin_client,
                small_cache: args.small_cache,
                capabilities: args.capabilities,
            })
        } else {
            None
        };

        let input = InputConfig {
            credentials: AuthIdentity {
                username,
                password: password.into(),
                domain: args.domain,
            },
            security_protocol: SecurityProtocol::parse(args.security_protocol),
            keyboard_type: KeyboardType::parse(args.keyboard_type),
            keyboard_subtype: args.keyboard_subtype,
            keyboard_functional_keys_count: args.keyboard_functional_keys_count,
            ime_file_name: args.ime_file_name,
            dig_product_id: args.dig_product_id,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            global_channel_name: GLOBAL_CHANNEL_NAME.to_string(),
            user_channel_name: USER_CHANNEL_NAME.to_string(),
            graphics_config,
        };

        Ok(Self {
            log_file: args.log_file,
            addr,
            input,
        })
    }
}
