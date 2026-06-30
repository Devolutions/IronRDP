//! The short-lived CLI: parse arguments, build a request (merging a `.rdp` file with overrides for
//! `connect`), send it to the daemon, and print the response.
//!
//! The CLI operates purely at the [`PropertySet`] level for connection config — it never calls
//! typed `ConfigBuilder` setters.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::PathBuf;

use anyhow::Context as _;
use clap::{Args, CommandFactory as _, Parser, Subcommand, ValueEnum};
use ironrdp_client::config::{ConfigBuilder, MissingField};
use ironrdp_input::MouseButton;
use ironrdp_propertyset::PropertySet;

use crate::ipc::{KeyFilter, Payload, PropValue, Request, Response};
use crate::transport::{self, Endpoint};

/// IronRDP agent: a CLI-driven, daemon-backed RDP client.
#[derive(Parser, Debug)]
#[command(name = "ironrdp-agent", version, about, long_about = None)]
pub struct Cli {
    /// Print a structured, LLM-friendly guide to every operation and exit.
    #[arg(long, global = true)]
    help_agent: bool,

    /// Override the IPC endpoint (defaults to the per-user socket/pipe).
    #[arg(long, global = true)]
    endpoint: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the long-lived daemon in the foreground (owns the RDP session).
    DaemonStart(DaemonArgs),
    /// Open an RDP session from a .rdp file and/or CLI overrides.
    Connect(ConnectArgs),
    /// Tear down the current RDP session (the daemon keeps running).
    Disconnect,
    /// Report the current session status.
    Status,
    /// Query the live session properties.
    QueryProps(QueryPropsArgs),
    /// Print retained daemon log lines.
    QueryLogs(QueryLogsArgs),
    /// Print the most recent frame dimensions.
    Screenshot,
    /// Move the mouse pointer to an absolute position.
    MouseMove {
        #[arg(long)]
        x: u16,
        #[arg(long)]
        y: u16,
    },
    /// Press or release a mouse button.
    MouseButton {
        #[arg(long, value_enum)]
        button: CliMouseButton,
        #[arg(long, action = clap::ArgAction::Set)]
        pressed: bool,
    },
    /// Rotate the mouse wheel (negative delta scrolls down/left).
    Wheel {
        #[arg(long, allow_hyphen_values = true)]
        delta: i16,
        #[arg(long)]
        horizontal: bool,
    },
    /// Press or release a key identified by its RDP scancode.
    KeyScancode {
        #[arg(long, value_parser = parse_scancode)]
        scancode: u16,
        #[arg(long, action = clap::ArgAction::Set)]
        pressed: bool,
    },
    /// Press or release a key identified by a Unicode character.
    KeyUnicode {
        #[arg(long = "char")]
        character: char,
        #[arg(long, action = clap::ArgAction::Set)]
        pressed: bool,
    },
}

#[derive(Args, Debug)]
struct DaemonArgs {
    /// Path to a .rdp file whose properties are preloaded as an overlay applied to every `connect`
    /// (overlay wins). Use this to provision any setting out of band — credentials in particular
    /// (e.g. `ClearTextPassword`), so a caller never needs to supply them; `status` then reports
    /// `credentials loaded: true`.
    #[arg(long)]
    overlay: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct ConnectArgs {
    /// Path to a .rdp file to read the base configuration from.
    #[arg(long)]
    rdp_file: Option<PathBuf>,
    /// RDP server address (host[:port]). Overrides the .rdp file.
    #[arg(long)]
    server: Option<String>,
    /// RDP account user name. Overrides the .rdp file.
    #[arg(short, long)]
    username: Option<String>,
    /// RDP account password. Overrides the .rdp file.
    #[arg(short, long)]
    password: Option<String>,
    /// RDP account domain. Overrides the .rdp file.
    #[arg(short, long)]
    domain: Option<String>,
    /// Tracing filter directive applied to this session's log capture (e.g.
    /// `ironrdp_connector=trace`), layered on top of the default `debug` level. Use it to raise
    /// verbosity up-front when troubleshooting a connection.
    #[arg(long)]
    log_directive: Option<String>,
}

#[derive(Args, Debug)]
struct QueryPropsArgs {
    /// Only show keys containing this substring (case-insensitive).
    #[arg(long, conflicts_with = "prefix")]
    filter: Option<String>,
    /// Only show keys starting with this prefix (case-insensitive).
    #[arg(long)]
    prefix: Option<String>,
}

#[derive(Args, Debug)]
struct QueryLogsArgs {
    /// Only show lines containing this substring.
    #[arg(long)]
    substring: Option<String>,
    /// Only show the last N retained lines.
    #[arg(long)]
    last: Option<u32>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliMouseButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

impl CliMouseButton {
    fn into_button(self) -> MouseButton {
        match self {
            Self::Left => MouseButton::Left,
            Self::Middle => MouseButton::Middle,
            Self::Right => MouseButton::Right,
            Self::X1 => MouseButton::X1,
            Self::X2 => MouseButton::X2,
        }
    }
}

/// Parses an RDP scancode in decimal or `0x`-prefixed hexadecimal.
fn parse_scancode(input: &str) -> Result<u16, core::num::ParseIntError> {
    if let Some(hex) = input.strip_prefix("0x").or_else(|| input.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16)
    } else {
        input.parse()
    }
}

/// Entry point shared by the binary: dispatches the parsed [`Cli`].
pub async fn run(cli: Cli) -> anyhow::Result<()> {
    if cli.help_agent {
        print!("{}", crate::help::AGENT_GUIDE);
        return Ok(());
    }

    let endpoint = endpoint_from_arg(cli.endpoint);

    let Some(command) = cli.command else {
        let _ = Cli::command().print_help();
        println!();
        return Ok(());
    };

    let request = match command {
        Command::DaemonStart(args) => {
            let overlay = load_overlay(args.overlay.as_deref())?;
            return crate::daemon::run(endpoint, overlay).await;
        }
        Command::Connect(args) => build_connect_request(args)?,
        Command::Disconnect => Request::Disconnect,
        Command::Status => Request::Status,
        Command::QueryProps(args) => Request::QueryProps {
            filter: args
                .filter
                .map(KeyFilter::Substring)
                .or_else(|| args.prefix.map(KeyFilter::Prefix)),
        },
        Command::QueryLogs(args) => Request::QueryLogs {
            substring: args.substring,
            last: args.last,
        },
        Command::Screenshot => Request::Screenshot,
        Command::MouseMove { x, y } => Request::MouseMove { x, y },
        Command::MouseButton { button, pressed } => Request::MouseButton {
            button: button.into_button(),
            pressed,
        },
        Command::Wheel { delta, horizontal } => Request::Wheel { delta, horizontal },
        Command::KeyScancode { scancode, pressed } => Request::KeyScancode { scancode, pressed },
        Command::KeyUnicode { character, pressed } => Request::KeyUnicode { ch: character, pressed },
    };

    let response = transport::send_request(&endpoint, &request).await?;
    print_response(response)
}

/// Loads an operator-provided overlay [`PropertySet`] from an optional `.rdp` file. Returns an
/// empty set when no path is given.
fn load_overlay(path: Option<&std::path::Path>) -> anyhow::Result<PropertySet> {
    let mut properties = PropertySet::new();
    if let Some(path) = path {
        let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        if let Err(errors) = ironrdp_rdpfile::load(&mut properties, &text) {
            for error in &errors {
                eprintln!("warning: skipped entry in {}: {error}", path.display());
            }
        }
    }
    Ok(properties)
}

/// Builds a `Connect` request by merging an optional `.rdp` file with CLI overrides into one
/// [`PropertySet`], then pre-validating it locally.
fn build_connect_request(args: ConnectArgs) -> anyhow::Result<Request> {
    let mut properties = PropertySet::new();

    if let Some(path) = &args.rdp_file {
        let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        if let Err(errors) = ironrdp_rdpfile::load(&mut properties, &text) {
            for error in &errors {
                eprintln!("warning: skipped entry in {}: {error}", path.display());
            }
        }
    }

    // CLI overrides win: plain inserts with canonical .rdp keys.
    if let Some(server) = args.server {
        properties.set_full_address(server);
    }
    if let Some(username) = args.username {
        properties.set_username("username", username);
    }
    if let Some(password) = args.password {
        properties.set_clear_text_password(password);
    }
    if let Some(domain) = args.domain {
        properties.set_domain(domain);
    }

    Ok(Request::Connect {
        properties,
        log_directive: args.log_directive,
    })
}

fn print_response(response: Response) -> anyhow::Result<()> {
    match response {
        Response::Ok(payload) => {
            print_payload(payload);
            Ok(())
        }
        Response::Err(message) => anyhow::bail!("{message}"),
    }
}

fn print_payload(payload: Payload) {
    match payload {
        Payload::Empty => println!("ok"),
        Payload::Status(status) => {
            println!("state: {:?}", status.state);
            if let Some(destination) = status.destination {
                println!("destination: {destination}");
            }
            if let (Some(width), Some(height)) = (status.width, status.height) {
                println!("resolution: {width}x{height}");
            }
            if let Some(message) = status.message {
                println!("detail: {message}");
            }
            println!("credentials loaded: {}", status.credentials_loaded);
        }
        Payload::Properties(dump) => {
            for entry in dump.entries {
                let value = match entry.value {
                    PropValue::Int(value) => value.to_string(),
                    PropValue::Str(value) => value,
                };
                // Descriptions are derived locally from the key: they are a static function of the
                // property name, so there is no reason to carry them over the wire.
                match property_description(&entry.key) {
                    Some(description) => println!("{} = {value}  # {description}", entry.key),
                    None => println!("{} = {value}", entry.key),
                }
            }
        }
        Payload::Logs(lines) => {
            for line in lines {
                println!("{line}");
            }
        }
        Payload::Screenshot { width, height } => println!("frame {width}x{height}"),
    }
}

#[cfg(unix)]
fn endpoint_from_arg(arg: Option<String>) -> Endpoint {
    match arg {
        Some(value) => Endpoint(PathBuf::from(value)),
        None => transport::default_endpoint(),
    }
}

#[cfg(windows)]
fn endpoint_from_arg(arg: Option<String>) -> Endpoint {
    match arg {
        Some(value) => Endpoint(value),
        None => transport::default_endpoint(),
    }
}

/// Short, LLM-facing descriptions for the configuration keys recognized by [`ironrdp_cfg`], derived
/// locally from the key name when printing a dump (kept out of the wire protocol on purpose).
///
/// Keys are the canonical lowercase `.rdp` names. Secret keys are listed for completeness even
/// though `ConfigBuilder::build` strips them before a session starts, so they never appear in a
/// dump.
fn property_description(key: &str) -> Option<&'static str> {
    let description = match key {
        // ── Standard .rdp keys ──────────────────────────────────────────────
        "full address" => "RDP server address as host[:port]",
        "alternate full address" => "fallback RDP server address (host[:port]) tried if 'full address' fails",
        "server port" => "RDP server TCP port (default 3389)",
        "username" => "RDP account user name",
        "domain" => "RDP account domain",
        "cleartextpassword" => "plaintext RDP account password (secret)",
        "desktopwidth" => "requested remote desktop width in pixels",
        "desktopheight" => "requested remote desktop height in pixels",
        "desktopscalefactor" => "remote desktop DPI scale factor, in percent (e.g. 100, 150)",
        "compression" => "enable bulk data compression (0/1)",
        "audiomode" => "remote audio mode (0 = play on client, 1 = play on server, 2 = disabled)",
        "redirectclipboard" => "enable clipboard redirection (0/1)",
        "enablecredsspsupport" => "enable CredSSP/NLA authentication (0/1)",
        "alternate shell" => "program to launch on connect instead of the desktop shell",
        "shell working directory" => "working directory for the alternate shell or RemoteApp program",
        "remoteapplicationname" => "RemoteApp display name",
        "remoteapplicationprogram" => "RemoteApp program path to launch",
        // ── RD gateway ──────────────────────────────────────────────────────
        "gatewayhostname" => "RD gateway host name",
        "gatewayusername" => "RD gateway user name",
        "gatewaypassword" => "RD gateway password (secret)",
        "gatewayusagemethod" => {
            "when to use the RD gateway (0 = direct, 1 = always, 2 = detect, 3 = default, 4 = direct, bypass for local)"
        }
        "gatewaycredentialssource" => {
            "RD gateway credential source (0 = server, 1 = user, 2 = profile, 3 = prompt, 4 = smart card, 5 = logon)"
        }
        // ── Kerberos ────────────────────────────────────────────────────────
        "kdcproxyname" => "Kerberos KDC proxy name",
        "kdcproxyurl" => "Kerberos KDC proxy URL",
        // ── IronRDP extensions (ironrdp_ prefix) ────────────────────────────
        "ironrdp_autologon" => "attempt automatic logon with the supplied credentials (0/1)",
        "ironrdp_colordepth" => "color depth in bits per pixel (e.g. 16 or 32)",
        "ironrdp_compressionlevel" => "bulk compression level",
        "ironrdp_dvcpipeproxy" => "DVC pipe proxy specs, comma-separated 'channel=pipe' pairs",
        "ironrdp_dvcplugin" => "DVC plugin library paths, comma-separated",
        "ironrdp_qoi" => "enable the QOI graphics codec (0/1)",
        "ironrdp_qoiz" => "enable the QOIZ (compressed QOI) graphics codec (0/1)",
        "ironrdp_rdpdr" => "enable the RDPDR device-redirection channel (0/1)",
        "ironrdp_smartcard" => "enable smart-card device redirection (0/1)",
        "ironrdp_tls" => "use plain TLS security instead of CredSSP/Hybrid (0/1)",
        "ironrdp_fakeeventsinterval" => "interval in minutes between synthetic keep-alive input events",
        "ironrdp_rdcleanpathtoken" => "RDCleanPath authentication token (secret)",
        "ironrdp_rdcleanpathurl" => "RDCleanPath proxy URL",
        "ironrdp_serverpointer" => "render the server-side pointer instead of a client-drawn pointer (0/1)",
        _ => return None,
    };
    Some(description)
}
