use std::net::SocketAddr;

use clap::{crate_name, crate_version, App, Arg};
use ironrdp::nego::SecurityProtocol;
use sspi::AuthIdentity;

const DEFAULT_WIDTH: u16 = 1920;
const DEFAULT_HEIGHT: u16 = 1080;

pub struct Config {
    pub log_file: String,
    pub routing_addr: SocketAddr,
    pub width: u16,
    pub height: u16,
    pub input: Input,
}

impl Config {
    pub fn parse_args() -> Self {
        let log_file_name = format!("{}.log", crate_name!());
        let cli_app = App::new(crate_name!())
            .author("Devolutions")
            .version(crate_version!())
            .version_short("v")
            .about("Devolutions-IronRDP client")
            .arg(
                Arg::with_name("log-file")
                    .long("log-file")
                    .value_name("LOG_FILE")
                    .help("A file with IronRDP client logs")
                    .takes_value(true)
                    .empty_values(false)
                    .default_value(&log_file_name),
            )
            .arg(
                Arg::with_name("addr")
                    .value_name("ADDR")
                    .help("An address on which the client will connect. Format: <ip>:<port>")
                    .takes_value(true)
                    .empty_values(false)
                    .required(true)
                    .index(1)
                    .validator(|u| match u.parse::<SocketAddr>() {
                        Ok(_) => Ok(()),
                        Err(_) => Err(String::from(
                            "The address does not match the format: <ip>:<port>",
                        )),
                    }),
            )
            .args(&Input::args());
        let matches = cli_app.get_matches();

        let log_file = matches
            .value_of("log-file")
            .map(String::from)
            .expect("log file must be at least the default");

        let routing_addr = matches
            .value_of("addr")
            .map(|u| u.parse().unwrap())
            .expect("addr must be at least the default");

        let input = Input::from_matches(&matches);

        Self {
            log_file,
            routing_addr,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            input,
        }
    }
}

pub struct Input {
    pub credentials: AuthIdentity,
    pub security_protocol: SecurityProtocol,
    pub keyboard_type: ironrdp::gcc::KeyboardType,
    pub keyboard_subtype: u32,
    pub keyboard_functional_keys_count: u32,
    pub ime_file_name: String,
    pub dig_product_id: String,
}

impl Input {
    fn args<'a, 'b>() -> [Arg<'a, 'b>; 9] {
        [
            Arg::with_name("username")
                .short("u")
                .long("username")
                .value_name("USERNAME")
                .help("A target RDP server user name")
                .takes_value(true)
                .empty_values(false)
                .required(true),
            Arg::with_name("domain")
                .short("d")
                .long("domain")
                .value_name("DOMAIN")
                .help("An optional target RDP server domain name")
                .takes_value(true)
                .required(false),
            Arg::with_name("password")
                .short("p")
                .long("password")
                .value_name("PASSWORD")
                .help("A target RDP server user password")
                .takes_value(true)
                .required(true),
            Arg::with_name("security-protocol")
                .long("security-protocol")
                .value_name("SECURITY_PROTOCOL")
                .help("Specify the security protocols to use")
                .takes_value(true)
                .multiple(true)
                .possible_values(&["ssl", "hybrid", "hybrid_ex"])
                .default_value(&"hybrid_ex")
                .required(true),
            Arg::with_name("keyboard-type")
                .long("keyboard-type")
                .value_name("KEYBOARD_TYPE")
                .help("The keyboard type")
                .takes_value(true)
                .possible_values(&[
                    "ibm_pc_xt",
                    "olivetti_ico",
                    "ibm_pc_at",
                    "ibm_enhanced",
                    "nokia1050",
                    "nokia9140",
                    "japanese",
                ])
                .default_value(&"ibm_enhanced"),
            Arg::with_name("keyboard-subtype")
                .long("keyboard-subtype")
                .value_name("KEYBOARD_SUBTYPE")
                .help(
                    "The keyboard subtype (an original equipment manufacturer-dependent value)",
                )
                .takes_value(true)
                .default_value(&"0")
                .validator(is_uint),
            Arg::with_name("keyboard-functional-keys-count")
                .long("keyboard-functional-keys-count")
                .value_name("KEYBOARD_FUNCTIONAL_KEYS_COUNT")
                .help("The number of function keys on the keyboard")
                .takes_value(true)
                .default_value(&"12")
                .validator(is_uint),
            Arg::with_name("ime-file-name")
                .long("ime-file-name")
                .value_name("IME_FILENAME")
                .help("The input method editor (IME) file name associated with the active input locale")
                .takes_value(true)
                .default_value(&""),
            Arg::with_name("dig-product-id")
                .long("dig-product-id")
                .value_name("DIG_PRODUCT_ID")
                .help("Contains a value that uniquely identifies the client")
                .takes_value(true)
                .default_value(&""),
]
    }
    fn from_matches(matches: &clap::ArgMatches<'_>) -> Self {
        let username = matches
            .value_of("username")
            .map(String::from)
            .expect("username must be specified");
        let domain = matches.value_of("domain").map(String::from);
        let password = matches
            .value_of("password")
            .map(String::from)
            .expect("password must be specified");
        let credentials = AuthIdentity {
            username,
            password,
            domain,
        };

        let security_protocol = matches
            .values_of("security-protocol")
            .expect("security-protocol must be specified")
            .map(|value| match value {
                "ssl" => SecurityProtocol::SSL,
                "hybrid" => SecurityProtocol::HYBRID,
                "hybrid_ex" => SecurityProtocol::HYBRID_EX,
                _ => unreachable!("clap must not allow other security protocols"),
            })
            .collect();

        let keyboard_type = matches
            .value_of("keyboard-type")
            .map(|value| match value {
                "ibm_pc_xt" => ironrdp::gcc::KeyboardType::IbmPcXt,
                "olivetti_ico" => ironrdp::gcc::KeyboardType::OlivettiIco,
                "ibm_pc_at" => ironrdp::gcc::KeyboardType::IbmPcAt,
                "ibm_enhanced" => ironrdp::gcc::KeyboardType::IbmEnhanced,
                "nokia1050" => ironrdp::gcc::KeyboardType::Nokia1050,
                "nokia9140" => ironrdp::gcc::KeyboardType::Nokia9140,
                "japanese" => ironrdp::gcc::KeyboardType::Japanese,
                _ => unreachable!("clap must not allow other keyboard types"),
            })
            .expect("keyboard type must be at least the default");

        let keyboard_subtype = matches
            .value_of("keyboard-subtype")
            .map(|value| value.parse::<u32>().unwrap())
            .expect("keyboard subtype must be at least the default");

        let keyboard_functional_keys_count = matches
            .value_of("keyboard-functional-keys-count")
            .map(|value| value.parse::<u32>().unwrap())
            .expect("keyboard functional keys count must be at least the default");

        let ime_file_name = matches
            .value_of("ime-file-name")
            .map(String::from)
            .expect("IME file name must be at least the default");

        let dig_product_id = matches
            .value_of("dig-product-id")
            .map(String::from)
            .expect("DIG product ID must be at least the default");

        Self {
            credentials,
            security_protocol,
            keyboard_type,
            keyboard_subtype,
            keyboard_functional_keys_count,
            ime_file_name,
            dig_product_id,
        }
    }
}

fn is_uint(s: String) -> Result<(), String> {
    match s.parse::<usize>() {
        Ok(_) => Ok(()),
        Err(_) => Err(String::from("The value is not numeric")),
    }
}
