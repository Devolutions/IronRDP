//! CLI argument types for ironrdp-agent (binary).

use core::str::FromStr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[cfg(windows)]
pub const DEFAULT_ENDPOINT: &str = "pipe:ironrdp-agent";
#[cfg(unix)]
pub const DEFAULT_ENDPOINT: &str = "unix:/tmp/ironrdp-agent.sock";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Pipe(String),
    Unix(PathBuf),
}

impl FromStr for Endpoint {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> anyhow::Result<Self> {
        if let Some(name) = value.strip_prefix("pipe:") {
            if name.is_empty() {
                anyhow::bail!("pipe endpoint name is empty");
            }
            Ok(Self::Pipe(name.to_owned()))
        } else if let Some(path) = value.strip_prefix("unix:") {
            if path.is_empty() {
                anyhow::bail!("unix endpoint path is empty");
            }
            Ok(Self::Unix(PathBuf::from(path)))
        } else if cfg!(windows) {
            Ok(Self::Pipe(value.to_owned()))
        } else {
            Ok(Self::Unix(PathBuf::from(value)))
        }
    }
}

impl core::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Pipe(name) => write!(f, "pipe:{name}"),
            Self::Unix(path) => write!(f, "unix:{}", path.display()),
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn as_level_filter(self) -> tracing::metadata::LevelFilter {
        use tracing::metadata::LevelFilter;
        match self {
            Self::Error => LevelFilter::ERROR,
            Self::Warn => LevelFilter::WARN,
            Self::Info => LevelFilter::INFO,
            Self::Debug => LevelFilter::DEBUG,
            Self::Trace => LevelFilter::TRACE,
        }
    }
}

#[derive(Parser, Debug)]
#[clap(author = "Devolutions", about = "Agentic IronRDP client daemon and CLI")]
#[clap(version, long_about = None)]
pub struct Cli {
    /// IPC endpoint used by both daemon and client modes.
    #[clap(long, default_value = DEFAULT_ENDPOINT)]
    pub endpoint: Endpoint,

    /// Default logging level when IRONRDP_LOG and --log-filter are not set.
    #[clap(long, value_enum, default_value_t = LogLevel::Warn)]
    pub log_level: LogLevel,

    /// Tracing filter directives. Overrides --log-level and IRONRDP_LOG when set.
    #[clap(long)]
    pub log_filter: Option<String>,

    /// Write logs to this file instead of stderr.
    #[clap(long)]
    pub log_file: Option<PathBuf>,

    /// Do not spawn the daemon automatically when a client command cannot connect.
    #[clap(long)]
    pub no_spawn_daemon: bool,

    /// Print the agent's extended help text and exit.
    #[clap(long)]
    pub help_agent: bool,

    #[clap(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the IPC daemon in the foreground.
    Daemon,
    /// Connect an RDP session through the daemon.
    Connect(Box<ConnectCommand>),
    /// List daemon sessions.
    Sessions,
    /// Get daemon or session status.
    Status(SessionArg),
    /// Disconnect a session.
    Disconnect(RequiredSessionArg),
    /// Send mouse input.
    Mouse(MouseCommand),
    /// Send keyboard input.
    Keyboard(KeyboardCommand),
    /// Resize a session.
    Resize(ResizeCommand),
    /// Wait for a framebuffer update.
    WaitFrame(WaitFrameCommand),
    /// Save the latest session framebuffer as PNG.
    Screenshot(ScreenshotCommand),
    /// Dump the session's live PropertySet with descriptions.
    DumpProperties(RequiredSessionArg),
    /// Update a single property on a live session.
    SetProperty(SetPropertyCommand),
}

#[derive(Args, Debug)]
pub struct SessionArg {
    #[clap(long)]
    pub session: Option<String>,
}

#[derive(Args, Debug)]
pub struct RequiredSessionArg {
    #[clap(long)]
    pub session: String,
}

#[derive(Args, Debug)]
pub struct ConnectCommand {
    /// RDP server host or host:port.
    pub destination: Option<String>,

    #[clap(short, long)]
    pub username: Option<String>,
    #[clap(short, long)]
    pub domain: Option<String>,
    #[clap(short, long, conflicts_with = "password_env")]
    pub password: Option<String>,
    /// Environment variable containing the RDP password.
    #[clap(long)]
    pub password_env: Option<String>,

    #[clap(long)]
    pub gw_endpoint: Option<String>,
    #[clap(long)]
    pub gw_user: Option<String>,
    #[clap(long)]
    pub gw_pass: Option<String>,

    /// Path to a `.rdp` file used as the base property set.
    #[clap(long)]
    pub rdp_file: Option<PathBuf>,

    /// Optional human-readable label associated with the session.
    #[clap(long)]
    pub label: Option<String>,

    #[clap(long, value_parser = clap::value_parser!(u16).range(1..=8192))]
    pub desktop_width: Option<u16>,
    #[clap(long, value_parser = clap::value_parser!(u16).range(1..=8192))]
    pub desktop_height: Option<u16>,
    #[clap(long, value_parser = clap::value_parser!(u32).range(100..=500))]
    pub scale_desktop: Option<u32>,
    #[clap(long)]
    pub color_depth: Option<u32>,
    #[clap(long)]
    pub no_tls: bool,
    #[clap(long, alias = "no-nla")]
    pub no_credssp: bool,
    #[clap(long, action = clap::ArgAction::Set)]
    pub compression_enabled: Option<bool>,
}

#[derive(Args, Debug)]
pub struct MouseCommand {
    #[clap(long)]
    pub session: String,
    #[clap(subcommand)]
    pub action: MouseAction,
}

#[derive(Subcommand, Debug)]
pub enum MouseAction {
    Move {
        #[clap(long)]
        x: u16,
        #[clap(long)]
        y: u16,
    },
    Click {
        #[clap(long, value_enum)]
        button: MouseButton,
        #[clap(long)]
        x: Option<u16>,
        #[clap(long)]
        y: Option<u16>,
    },
    Down {
        #[clap(long, value_enum)]
        button: MouseButton,
    },
    Up {
        #[clap(long, value_enum)]
        button: MouseButton,
    },
    Wheel {
        #[clap(long)]
        units: i16,
        #[clap(long)]
        horizontal: bool,
    },
    Position,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

impl From<MouseButton> for crate::ipc::MouseButton {
    fn from(value: MouseButton) -> Self {
        match value {
            MouseButton::Left => Self::Left,
            MouseButton::Middle => Self::Middle,
            MouseButton::Right => Self::Right,
            MouseButton::X1 => Self::X1,
            MouseButton::X2 => Self::X2,
        }
    }
}

#[derive(Args, Debug)]
pub struct KeyboardCommand {
    #[clap(long)]
    pub session: String,
    #[clap(subcommand)]
    pub action: KeyboardAction,
}

#[derive(Subcommand, Debug)]
pub enum KeyboardAction {
    Key {
        #[clap(long, value_parser = parse_scancode)]
        scancode: u16,
        #[clap(long)]
        release: bool,
    },
    Text {
        #[clap(long)]
        text: String,
    },
    Shortcut {
        #[clap(long, value_delimiter = ',', value_parser = parse_scancode)]
        scancodes: Vec<u16>,
    },
    ReleaseAll,
}

pub fn parse_scancode(input: &str) -> Result<u16, String> {
    if let Some(hex) = input.strip_prefix("0x") {
        u16::from_str_radix(hex, 16).map_err(|e| e.to_string())
    } else {
        input.parse::<u16>().map_err(|e| e.to_string())
    }
}

#[derive(Args, Debug)]
pub struct ResizeCommand {
    #[clap(long)]
    pub session: String,
    #[clap(long)]
    pub width: u16,
    #[clap(long)]
    pub height: u16,
    #[clap(long, default_value_t = 100)]
    pub scale: u32,
}

#[derive(Args, Debug)]
pub struct WaitFrameCommand {
    #[clap(long)]
    pub session: String,
    #[clap(long, default_value_t = 30_000)]
    pub timeout_ms: u64,
    #[clap(long)]
    pub after_frame: Option<u64>,
}

#[derive(Args, Debug)]
pub struct ScreenshotCommand {
    #[clap(long)]
    pub session: String,
    #[clap(long)]
    pub output: PathBuf,
}

#[derive(Args, Debug)]
pub struct SetPropertyCommand {
    #[clap(long)]
    pub session: String,
    #[clap(long)]
    pub key: String,
    #[clap(long)]
    pub value: String,
}

impl ConnectCommand {
    /// Render the connect command into a synthetic `.rdp` text payload,
    /// combining `--rdp-file` (if any) with CLI overrides.
    pub fn to_rdp_content(&self) -> anyhow::Result<String> {
        use anyhow::Context as _;

        let mut props = ironrdp_propertyset::PropertySet::new();
        if let Some(rdp_file) = &self.rdp_file {
            let input =
                std::fs::read_to_string(rdp_file).with_context(|| format!("failed to read {}", rdp_file.display()))?;
            if let Err(errors) = ironrdp_rdpfile::load(&mut props, &input) {
                for error in &errors {
                    tracing::warn!(%error, file = %rdp_file.display(), "Ignored .rdp entry");
                }
            }
        }

        if let Some(dest) = &self.destination {
            props.insert("full address", dest.as_str());
        }
        if let Some(username) = &self.username {
            props.insert("username", username.as_str());
        }
        match (&self.password, &self.password_env) {
            (Some(p), None) => {
                props.insert("ClearTextPassword", p.as_str());
            }
            (None, Some(env)) => {
                let p = std::env::var(env).with_context(|| format!("failed to read password from {env}"))?;
                props.insert("ClearTextPassword", p);
            }
            (None, None) => {}
            (Some(_), Some(_)) => anyhow::bail!("--password and --password-env are mutually exclusive"),
        }
        if let Some(domain) = &self.domain {
            props.insert("domain", domain.as_str());
        }
        if let Some(gw) = &self.gw_endpoint {
            props.insert("gatewayhostname", gw.as_str());
            props.insert(
                "gatewayusagemethod",
                ironrdp_cfg::GatewayUsageMethod::UseAlways.as_i64(),
            );
        }
        if let Some(u) = &self.gw_user {
            props.insert("gatewayusername", u.as_str());
        }
        if let Some(p) = &self.gw_pass {
            props.insert("GatewayPassword", p.as_str());
        }
        if let Some(w) = self.desktop_width {
            props.insert("desktopwidth", i64::from(w));
        }
        if let Some(h) = self.desktop_height {
            props.insert("desktopheight", i64::from(h));
        }
        if let Some(s) = self.scale_desktop {
            props.insert("desktopscalefactor", i64::from(s));
        }
        if let Some(cd) = self.color_depth {
            props.insert("session bpp", i64::from(cd));
        }
        if self.no_credssp {
            props.insert("enablecredsspsupport", 0i64);
        }
        if let Some(enabled) = self.compression_enabled {
            props.insert("compression", enabled);
        }
        if self.no_tls {
            props.insert("agent:no_tls", true);
        }

        Ok(ironrdp_rdpfile::write(&props))
    }
}
