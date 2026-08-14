//! The short-lived CLI: parse arguments, build a request (merging a `.rdp` file with overrides for
//! `connect`), send it to the daemon, and print the response.
//!
//! The CLI operates purely at the [`PropertySet`] level for connection config — it never calls
//! typed `ConfigBuilder` setters.
//!
//! For `connect`, property precedence from low to high is: `.rdp` file → `--prop` overrides →
//! named flags (`--server`/`--username`/…). The daemon's own overlay (`daemon-start --overlay`,
//! itself built from a `.rdp` file with `--prop` overrides layered on top) wins over all of that —
//! see `Daemon::connect` in `daemon.rs`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use core::fmt;
use core::str::FromStr;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::{Args, CommandFactory as _, Parser, Subcommand, ValueEnum};
use ironrdp_cfg::{PropertySetExt as _, TargetAddr};
use ironrdp_input::MouseButton;
use ironrdp_propertyset::{PropertySet, Value};

use ironrdp_rpc::ipc::{
    AgentError, KeyFilter, MAX_UNICODE_TEXT_CHARS, NowExecutionKind, NowExecutionRequest, NowStream, OperationEvent,
    OperationEventKind, OperationInfo, OperationState, Payload, PenContactRequest, PenFrameRequest, PropValue,
    RailEvent, RailEventKind, RailExecuteRequest, Request, Response, TouchContactRequest, TouchFrameRequest,
};
use ironrdp_rpc::transport::{self, Endpoint};

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

    /// Select the local RPC backend.
    #[arg(long, global = true, value_enum, default_value_t = Backend::Daemon)]
    backend: Backend,

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
    /// Print the RDP session's captured log lines (from the daemon's in-memory ring buffer).
    QueryLogs(QueryLogsArgs),
    /// Capture the current frame (cursor included) as a PNG written to disk.
    Screenshot(ScreenshotArgs),
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
    /// Type bounded Unicode text without exposing a bulk-input operation to ActiveX.
    TypeUnicode {
        #[arg(long, value_parser = parse_unicode_text)]
        text: String,
    },
    /// Send one MS-RDPEI touch contact sample (legal flag sets only).
    Touch {
        #[arg(long, default_value_t = 0)]
        contact_id: u8,
        #[arg(long, allow_hyphen_values = true)]
        x: i32,
        #[arg(long, allow_hyphen_values = true)]
        y: i32,
        #[arg(long, value_enum)]
        action: CliTouchAction,
        #[arg(long, default_value_t = 0)]
        encode_time: u32,
        #[arg(long, default_value_t = 0)]
        frame_offset: u64,
    },
    /// Tap once via MS-RDPEI (one touch PDU: DOWN then out-of-range UP).
    TouchTap {
        #[arg(long, default_value_t = 0)]
        contact_id: u8,
        #[arg(long, allow_hyphen_values = true)]
        x: i32,
        #[arg(long, allow_hyphen_values = true)]
        y: i32,
    },
    /// Send one multi-contact MS-RDPEI touch frame (`id:x:y:action` entries).
    TouchFrame {
        /// Contacts as `id:x:y:action` (action uses the same names as `touch --action`).
        #[arg(long = "contact", required = true, num_args = 1..)]
        contacts: Vec<String>,
        #[arg(long, default_value_t = 0)]
        encode_time: u32,
        #[arg(long, default_value_t = 0)]
        frame_offset: u64,
    },
    /// Send one MS-RDPEI pen contact sample (legal flag sets only).
    Pen {
        #[arg(long, default_value_t = 0)]
        device_id: u8,
        #[arg(long, allow_hyphen_values = true)]
        x: i32,
        #[arg(long, allow_hyphen_values = true)]
        y: i32,
        #[arg(long, value_enum)]
        action: CliPenAction,
        #[arg(long, default_value_t = 0)]
        encode_time: u32,
        #[arg(long, default_value_t = 0)]
        frame_offset: u64,
        #[arg(long)]
        pressure: Option<u32>,
        #[arg(long)]
        rotation: Option<u16>,
        #[arg(long, allow_hyphen_values = true)]
        tilt_x: Option<i16>,
        #[arg(long, allow_hyphen_values = true)]
        tilt_y: Option<i16>,
        #[arg(long, default_value_t = false)]
        eraser: bool,
        #[arg(long, default_value_t = false)]
        inverted: bool,
    },
    /// Tap once via MS-RDPEI pen (one pen PDU: DOWN then out-of-range UP).
    PenTap {
        #[arg(long, default_value_t = 0)]
        device_id: u8,
        #[arg(long, allow_hyphen_values = true)]
        x: i32,
        #[arg(long, allow_hyphen_values = true)]
        y: i32,
        #[arg(long)]
        pressure: Option<u32>,
    },
    /// Dismiss a hovering MS-RDPEI touch contact.
    DismissHovering {
        #[arg(long, default_value_t = 0)]
        contact_id: u8,
    },
    /// Resize the remote desktop.
    Resize {
        #[arg(long)]
        width: u16,
        #[arg(long)]
        height: u16,
    },
    /// Execute commands over the session's NOW DVC endpoint.
    Now(NowArgs),
    /// Inspect and exercise the validated, headless RemoteApp/RAIL audit plane.
    Rail(RailArgs),
    /// Windows Sandbox lifecycle helpers (list/config/stop via WindowsSandboxServer gRPC).
    #[cfg(windows)]
    Sandbox(SandboxArgs),
}

#[cfg(windows)]
#[derive(Args, Debug)]
struct SandboxArgs {
    #[command(subcommand)]
    command: SandboxCommand,
}

#[cfg(windows)]
#[derive(Subcommand, Debug)]
enum SandboxCommand {
    /// List running sandbox ids (`EnumerateSandboxVMs`).
    List,
    /// Show RDP config for a sandbox (password redacted).
    Config {
        /// Sandbox id from `wsb start` / `sandbox list`.
        id: String,
    },
    /// Shut down a running sandbox (`ShutdownSandbox`).
    Stop { id: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Backend {
    Daemon,
    ActiveX,
}

#[derive(Args, Debug)]
struct NowArgs {
    /// Output format for NOW metadata and events. Human output preserves stdout/stderr bytes.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,
    #[command(subcommand)]
    command: NowCommand,
}

#[derive(Subcommand, Debug)]
enum NowCommand {
    /// Negotiate and display supported NOW capabilities.
    Capabilities,
    /// Submit an untracked generic Run request.
    Run(NowRunArgs),
    /// Execute a Windows PowerShell command.
    Powershell(PowerShellArgs),
    /// Execute a PowerShell 7 command.
    Pwsh(PowerShellArgs),
    /// Execute a Process or Batch request.
    Exec(NowExecArgs),
    /// Cancel an active tracked operation.
    Cancel { operation_id: u64 },
    /// List retained daemon-owned operations.
    List,
    /// Show one retained operation.
    Status { operation_id: u64 },
    /// Replay retained output and follow a running operation.
    Attach {
        operation_id: u64,
        /// Only replay events with a sequence greater than this value.
        #[arg(long)]
        after_sequence: Option<u64>,
    },
    /// Forward a raw stdin chunk to an active operation.
    Stdin(NowStdinArgs),
    /// Display the local NOW endpoint state without making a new connection.
    Diagnostics,
}

#[derive(Args, Debug)]
struct RailArgs {
    /// Output format for RAIL evidence.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,
    #[command(subcommand)]
    command: RailCommand,
}

#[derive(Subcommand, Debug)]
enum RailCommand {
    /// Display the current RAIL handshake and pending-launch state.
    Status,
    /// Display validated RAIL events retained by the daemon.
    Events {
        /// Only show events with a sequence greater than this value.
        #[arg(long)]
        after_sequence: Option<u64>,
    },
    /// Wait for the next validated RAIL event without polling.
    Wait {
        /// Only return events with a sequence greater than this value.
        #[arg(long)]
        after_sequence: Option<u64>,
        /// Maximum wait time in milliseconds, capped at 60,000.
        #[arg(long, default_value_t = 30_000)]
        timeout_ms: u32,
    },
    /// Queue a RemoteApp launch without exposing raw protocol injection.
    Execute {
        /// Remote executable path or alias.
        executable: String,
        /// Initial remote working directory.
        #[arg(long)]
        working_directory: Option<String>,
        /// Command-line arguments for the executable.
        #[arg(long)]
        arguments: Option<String>,
        /// RAIL Execute flags.
        #[arg(long, default_value_t = 0)]
        flags: u16,
    },
}

#[derive(Args, Debug)]
struct NowRunArgs {
    /// Command line for the remote agent.
    command: String,
    /// Optional remote working directory.
    #[arg(long)]
    directory: Option<String>,
}

#[derive(Args, Debug)]
struct NowExecArgs {
    #[command(subcommand)]
    command: NowExecCommand,
}

#[derive(Subcommand, Debug)]
enum NowExecCommand {
    /// Execute a program via Windows CreateProcess.
    Process(ProcessArgs),
    /// Execute a Windows batch command.
    Batch(CommandArgs),
}

#[derive(Args, Debug)]
struct ProcessArgs {
    /// Program filename.
    filename: String,
    /// Command-line parameters passed to the program.
    #[arg(long)]
    parameters: Option<String>,
    #[command(flatten)]
    common: CommonExecutionArgs,
}

#[derive(Args, Debug)]
struct PowerShellArgs {
    /// PowerShell script or command.
    command: String,
    /// Opt out of the default `-NoProfile` behavior.
    #[arg(long)]
    profile: bool,
    /// Opt out of the default `-NonInteractive` behavior.
    #[arg(long)]
    interactive: bool,
    #[command(flatten)]
    common: CommonExecutionArgs,
}

#[derive(Args, Debug)]
struct CommandArgs {
    /// Batch command.
    command: String,
    #[command(flatten)]
    common: CommonExecutionArgs,
}

#[derive(Args, Debug)]
struct CommonExecutionArgs {
    /// Optional remote working directory.
    #[arg(long)]
    directory: Option<String>,
    /// Read initial stdin bytes from this file. Use `-` for this CLI's standard input.
    #[arg(long, value_name = "FILE")]
    stdin: Option<PathBuf>,
    /// Remote execution timeout in seconds.
    #[arg(long)]
    timeout: Option<u64>,
    /// Submit the command detached. Detached commands cannot receive stdin, output, or results.
    #[arg(long)]
    detached: bool,
    /// Write the daemon-owned operation ID to this file after local submission.
    #[arg(long)]
    operation_id_file: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct NowStdinArgs {
    /// Daemon-owned operation identity.
    operation_id: u64,
    /// Read raw bytes from this file. Use `-` for this CLI's standard input.
    #[arg(long, value_name = "FILE")]
    input: PathBuf,
    /// Mark this chunk as the final stdin chunk.
    #[arg(long)]
    last: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputFormat {
    /// Human-readable metadata; raw stdout/stderr are written unchanged.
    Human,
    /// One JSON document for the reply or completed stream.
    Json,
    /// One JSON object per reply/event.
    Ndjson,
}

const MAX_JSON_STREAM_EVENTS: usize = 8 * 1024;
const MAX_JSON_STREAM_OUTPUT: usize = 2 * 1024 * 1024;

/// A daemon-provided error that must be rendered according to the selected NOW output format.
#[derive(Debug)]
struct NowRequestError(AgentError);

impl fmt::Display for NowRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.message)
    }
}

impl core::error::Error for NowRequestError {}

/// A daemon-provided RAIL error that must be rendered according to the selected output format.
#[derive(Debug)]
struct RailRequestError(AgentError);

impl fmt::Display for RailRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.message)
    }
}

impl core::error::Error for RailRequestError {}

#[derive(Args, Debug)]
struct DaemonArgs {
    /// Path to a .rdp file whose properties are preloaded as an overlay applied to every `connect`
    /// (overlay wins). Use this to provision any setting out of band â€” credentials in particular
    /// (e.g. `ClearTextPassword`), so a caller never needs to supply them; `status` then reports
    /// `credentials loaded: true`.
    #[arg(long)]
    overlay: Option<PathBuf>,
    /// Arbitrary overlay property override (repeatable): `KEY:TYPE:VALUE`, the same grammar as one
    /// `.rdp` file line (`TYPE` is `i` for integer or `s` for string), e.g.
    /// `--prop ironrdp_autologon:i:1`. Applied on top of `--overlay`, so it lets an operator set any
    /// property without a dedicated flag existing for it.
    #[arg(long = "prop", value_name = "KEY:TYPE:VALUE")]
    prop: Vec<PropOverride>,
    /// Skip TLS certificate and hostname validation for this daemon.
    ///
    /// Use only for an explicitly authorized test endpoint. This startup-only flag accepts any
    /// certificate and is vulnerable to on-path attacks.
    #[arg(long)]
    skip_certificate_check: bool,
    /// Named local Windows volume exposed as an RDPDR filesystem drive.
    ///
    /// Repeat this flag to redirect multiple volumes. `NAME` is protocol-visible
    /// and must be unique.
    #[cfg(windows)]
    #[arg(long = "rdpdr-drive", value_name = "NAME=VOLUME_ROOT", value_parser = parse_rdpdr_drive)]
    rdpdr_drives: Vec<ironrdp_daemon::daemon::RdpdrDriveConfig>,
}

#[derive(Args, Debug)]
struct ConnectArgs {
    /// Path to a .rdp file to read the base configuration from.
    #[arg(long)]
    rdp_file: Option<PathBuf>,
    /// Arbitrary property override (repeatable): `KEY:TYPE:VALUE`, the same grammar as one `.rdp`
    /// file line (`TYPE` is `i` for integer or `s` for string), e.g. `--prop
    /// ironrdp_autologon:i:1 --prop username:s:admin`. Applied on top of `--rdp-file` but under the
    /// named flags below (e.g. `--username`), which always win for the same key. Use this to set
    /// any property without a dedicated flag existing for it.
    #[arg(long = "prop", value_name = "KEY:TYPE:VALUE")]
    prop: Vec<PropOverride>,
    /// RDP server address (host[:port]). Overrides the .rdp file.
    #[arg(long, env = "RDP_HOSTNAME")]
    server: Option<String>,
    /// RDP account user name. Overrides the .rdp file.
    #[arg(short, long, env = "RDP_USERNAME")]
    username: Option<String>,
    /// RDP account password. Overrides the .rdp file.
    #[arg(short, long, env = "RDP_PASSWORD", hide_env_values = true)]
    password: Option<String>,
    /// RDP account domain. Overrides the .rdp file.
    #[arg(short, long)]
    domain: Option<String>,
    /// Tracing filter directive applied to this session's log capture (e.g.
    /// `ironrdp_connector=trace`), layered on top of the default `debug` level. Use it to raise
    /// verbosity up-front when troubleshooting a connection.
    #[arg(long)]
    log_directive: Option<String>,
    /// Connect to a running Windows Sandbox by id (fetches pipe path + creds via gRPC).
    /// Prefer creating the VM with `wsb start` first.
    #[cfg(windows)]
    #[arg(long, value_name = "GUID", conflicts_with_all = ["server", "sandbox_pipe"])]
    sandbox_id: Option<String>,
    /// Low-level escape hatch: connect over a Windows Sandbox named pipe path
    /// (`\\.\pipe\{VMId}` or bare VMId). Requires `--username` / `--password`.
    #[cfg(windows)]
    #[arg(long, value_name = "PIPE", conflicts_with = "server")]
    sandbox_pipe: Option<String>,
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

#[derive(Args, Debug)]
struct ScreenshotArgs {
    /// Destination PNG path (defaults to `screenshot.png` in the current directory).
    path: Option<PathBuf>,
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

/// A single `--prop KEY:TYPE:VALUE` override, parsed with the same grammar as one `.rdp` file line
/// (see `ironrdp_rdpfile::load`): `TYPE` is `i` for integer or `s` for string.
#[derive(Clone, Debug)]
struct PropOverride {
    key: String,
    value: Value,
}

impl FromStr for PropOverride {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let mut parts = input.splitn(3, ':');
        let (Some(key), Some(ty), Some(value)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(format!("malformed --prop '{input}', expected KEY:TYPE:VALUE"));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("empty key in --prop '{input}', expected KEY:TYPE:VALUE"));
        }
        let value = match ty {
            "i" => value
                .parse::<i64>()
                .map(Value::from)
                .map_err(|_| format!("invalid integer value in --prop '{input}'"))?,
            "s" => Value::from(value),
            other => {
                return Err(format!(
                    "unknown type '{other}' in --prop '{input}', expected 'i' or 's'"
                ));
            }
        };
        Ok(Self {
            key: key.to_owned(),
            value,
        })
    }
}

/// Applies `--prop` overrides onto `properties`, in argument order (last one for a given key wins).
fn apply_prop_overrides(properties: &mut PropertySet, overrides: Vec<PropOverride>) {
    for over in overrides {
        properties.insert(over.key, over.value);
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

fn parse_unicode_text(input: &str) -> Result<String, String> {
    let char_count = input.chars().count();
    if char_count == 0 {
        return Err("text must not be empty".to_owned());
    }
    if char_count > MAX_UNICODE_TEXT_CHARS {
        return Err(format!(
            "text must contain at most {MAX_UNICODE_TEXT_CHARS} Unicode characters"
        ));
    }
    Ok(input.to_owned())
}

#[cfg(windows)]
fn parse_rdpdr_drive(input: &str) -> Result<ironrdp_daemon::daemon::RdpdrDriveConfig, String> {
    let (display_name, root_path) = input
        .split_once('=')
        .ok_or_else(|| "rdpdr drive must use NAME=VOLUME_ROOT syntax".to_owned())?;
    ironrdp_daemon::daemon::RdpdrDriveConfig::new(PathBuf::from(root_path), display_name.to_owned())
        .map_err(|error| error.to_string())
}

/// Legal MS-RDPEI pen contact transitions for agent testing.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliPenAction {
    /// DOWN | INRANGE | INCONTACT
    Down,
    /// UPDATE | INRANGE | INCONTACT
    Move,
    /// UP | INRANGE
    Up,
    /// UP
    OutOfRange,
    /// UPDATE | CANCELED
    Cancel,
    /// UPDATE | INRANGE
    Hover,
}

impl CliPenAction {
    fn flags(self) -> u16 {
        const DOWN: u16 = 0x0001;
        const UPDATE: u16 = 0x0002;
        const UP: u16 = 0x0004;
        const INRANGE: u16 = 0x0008;
        const INCONTACT: u16 = 0x0010;
        const CANCELED: u16 = 0x0020;

        match self {
            Self::Down => DOWN | INRANGE | INCONTACT,
            Self::Move => UPDATE | INRANGE | INCONTACT,
            Self::Up => UP | INRANGE,
            Self::OutOfRange => UP,
            Self::Cancel => UPDATE | CANCELED,
            Self::Hover => UPDATE | INRANGE,
        }
    }

    fn pen_flags(eraser: bool, inverted: bool) -> Option<u32> {
        // MS-RDPEI penFlags: ERASER_PRESSED=0x2, INVERTED=0x4
        let mut flags = 0u32;
        if eraser {
            flags |= 0x0002;
        }
        if inverted {
            flags |= 0x0004;
        }
        if flags == 0 { None } else { Some(flags) }
    }
}

fn pen_request(encode_time: u32, frame_offset: u64, contact: PenContactRequest) -> Request {
    Request::Pen {
        encode_time,
        frames: vec![PenFrameRequest {
            frame_offset,
            contacts: vec![contact],
        }],
    }
}

fn parse_touch_contact_spec(spec: &str) -> anyhow::Result<TouchContactRequest> {
    let mut parts = spec.splitn(4, ':');
    let (Some(id), Some(x), Some(y), Some(action)) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        anyhow::bail!("malformed contact '{spec}', expected id:x:y:action");
    };
    Ok(TouchContactRequest {
        contact_id: id.parse().with_context(|| format!("contact id in '{spec}'"))?,
        x: x.parse().with_context(|| format!("contact x in '{spec}'"))?,
        y: y.parse().with_context(|| format!("contact y in '{spec}'"))?,
        flags: action.parse::<CliTouchAction>().map_err(anyhow::Error::msg)?.flags(),
    })
}

/// Legal MS-RDPEI touch contact transitions for agent testing.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliTouchAction {
    /// DOWN | INRANGE | INCONTACT (engage)
    Down,
    /// UPDATE | INRANGE | INCONTACT (engaged move)
    Move,
    /// UP | INRANGE (leave contact while still in range / hover)
    Up,
    /// UP (leave range / out of range)
    OutOfRange,
    /// UPDATE | CANCELED
    Cancel,
    /// UPDATE | INRANGE (hover move)
    Hover,
}

impl CliTouchAction {
    fn flags(self) -> u16 {
        // Matches `ironrdp_rdpei::pdu::TouchContactFlags` / MS-RDPEI 2.2.3.3.1.1.
        const DOWN: u16 = 0x0001;
        const UPDATE: u16 = 0x0002;
        const UP: u16 = 0x0004;
        const INRANGE: u16 = 0x0008;
        const INCONTACT: u16 = 0x0010;
        const CANCELED: u16 = 0x0020;

        match self {
            Self::Down => DOWN | INRANGE | INCONTACT,
            Self::Move => UPDATE | INRANGE | INCONTACT,
            Self::Up => UP | INRANGE,
            Self::OutOfRange => UP,
            Self::Cancel => UPDATE | CANCELED,
            Self::Hover => UPDATE | INRANGE,
        }
    }
}

impl FromStr for CliTouchAction {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ValueEnum::from_str(s, true)
    }
}

fn touch_request(encode_time: u32, frame_offset: u64, contact_id: u8, x: i32, y: i32, flags: u16) -> Request {
    Request::Touch {
        encode_time,
        frames: vec![TouchFrameRequest {
            frame_offset,
            contacts: vec![TouchContactRequest {
                contact_id,
                x,
                y,
                flags,
            }],
        }],
    }
}

/// Entry point shared by the binary: dispatches the parsed [`Cli`].
pub async fn run(cli: Cli) -> anyhow::Result<()> {
    if cli.help_agent {
        print!("{}", crate::help::AGENT_GUIDE);
        return Ok(());
    }

    let endpoint = endpoint_from_arg(cli.endpoint, cli.backend);

    let Some(command) = cli.command else {
        let _ = Cli::command().print_help();
        println!();
        return Ok(());
    };

    if cli.backend == Backend::ActiveX && !matches!(&command, Command::DaemonStart(_)) {
        ensure_activex_backend(&endpoint).await?;
    }

    let request = match command {
        Command::DaemonStart(args) => {
            if cli.backend != Backend::Daemon {
                anyhow::bail!("daemon-start requires --backend daemon");
            }
            let overlay = load_overlay(args.overlay.as_deref(), args.prop)?;
            #[cfg(windows)]
            let rdpdr_drives = args.rdpdr_drives;
            #[cfg(not(windows))]
            let rdpdr_drives = Vec::new();
            let options = ironrdp_daemon::daemon::DaemonOptions::default()
                .with_certificate_check_skipped(args.skip_certificate_check)
                .with_rdpdr_drives(rdpdr_drives);
            return ironrdp_daemon::daemon::run(endpoint, overlay, options).await;
        }
        Command::Now(args) => {
            let format = args.format;
            let exit_code = match run_now(&endpoint, args).await {
                Ok(exit_code) => exit_code,
                Err(error) => {
                    if let Some(error) = error.downcast_ref::<NowRequestError>() {
                        print_now_error(&error.0, format)?;
                        std::process::exit(1);
                    }
                    return Err(error);
                }
            };
            if let Some(exit_code) = exit_code {
                if exit_code != 0 {
                    std::process::exit(remote_exit_status(exit_code));
                }
            }
            return Ok(());
        }
        Command::Rail(args) => return run_rail(&endpoint, args).await,
        Command::Connect(args) => build_connect_request(args)?,
        #[cfg(windows)]
        Command::Sandbox(args) => {
            return run_sandbox_command(args);
        }
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
        Command::Screenshot(args) => {
            let response = transport::send_request(&endpoint, &Request::Screenshot).await?;
            let payload = match response {
                Response::Ok(payload) => payload,
                Response::Err(message) => anyhow::bail!("{message}"),
            };
            let Payload::Screenshot { width, height, png } = payload else {
                anyhow::bail!("unexpected response to screenshot request");
            };
            let path = args.path.unwrap_or_else(|| PathBuf::from("screenshot.png"));
            return write_screenshot(width, height, &png, &path);
        }
        Command::MouseMove { x, y } => Request::MouseMove { x, y },
        Command::MouseButton { button, pressed } => Request::MouseButton {
            button: button.into_button(),
            pressed,
        },
        Command::Wheel { delta, horizontal } => Request::Wheel { delta, horizontal },
        Command::KeyScancode { scancode, pressed } => Request::KeyScancode { scancode, pressed },
        Command::KeyUnicode { character, pressed } => Request::KeyUnicode { ch: character, pressed },
        Command::TypeUnicode { text } => Request::UnicodeText { text },
        Command::Touch {
            contact_id,
            x,
            y,
            action,
            encode_time,
            frame_offset,
        } => touch_request(encode_time, frame_offset, contact_id, x, y, action.flags()),
        Command::TouchTap { contact_id, x, y } => Request::Touch {
            encode_time: 0,
            frames: vec![
                TouchFrameRequest {
                    frame_offset: 0,
                    contacts: vec![TouchContactRequest {
                        contact_id,
                        x,
                        y,
                        flags: CliTouchAction::Down.flags(),
                    }],
                },
                TouchFrameRequest {
                    // 16 ms between engage and release, matching a short physical tap.
                    frame_offset: 16_000,
                    contacts: vec![TouchContactRequest {
                        contact_id,
                        x,
                        y,
                        // Bare UP ends the contact lifecycle (out of range), not hover-up.
                        flags: CliTouchAction::OutOfRange.flags(),
                    }],
                },
            ],
        },
        Command::TouchFrame {
            contacts,
            encode_time,
            frame_offset,
        } => {
            let mut parsed = Vec::with_capacity(contacts.len());
            for spec in contacts {
                parsed.push(parse_touch_contact_spec(&spec)?);
            }
            Request::Touch {
                encode_time,
                frames: vec![TouchFrameRequest {
                    frame_offset,
                    contacts: parsed,
                }],
            }
        }
        Command::Pen {
            device_id,
            x,
            y,
            action,
            encode_time,
            frame_offset,
            pressure,
            rotation,
            tilt_x,
            tilt_y,
            eraser,
            inverted,
        } => pen_request(
            encode_time,
            frame_offset,
            PenContactRequest {
                device_id,
                x,
                y,
                flags: action.flags(),
                pressure,
                rotation,
                tilt_x,
                tilt_y,
                pen_flags: CliPenAction::pen_flags(eraser, inverted),
            },
        ),
        Command::PenTap {
            device_id,
            x,
            y,
            pressure,
        } => Request::Pen {
            encode_time: 0,
            frames: vec![
                PenFrameRequest {
                    frame_offset: 0,
                    contacts: vec![PenContactRequest {
                        device_id,
                        x,
                        y,
                        flags: CliPenAction::Down.flags(),
                        pressure,
                        rotation: None,
                        tilt_x: None,
                        tilt_y: None,
                        pen_flags: None,
                    }],
                },
                PenFrameRequest {
                    // 16 ms between engage and release, matching a short physical tap.
                    frame_offset: 16_000,
                    contacts: vec![PenContactRequest {
                        device_id,
                        x,
                        y,
                        flags: CliPenAction::OutOfRange.flags(),
                        pressure,
                        rotation: None,
                        tilt_x: None,
                        tilt_y: None,
                        pen_flags: None,
                    }],
                },
            ],
        },
        Command::DismissHovering { contact_id } => Request::DismissHoveringTouchContact { contact_id },
        Command::Resize { width, height } => Request::Resize { width, height },
    };

    let response = transport::send_request(&endpoint, &request).await?;
    print_response(response)
}

/// Loads an operator-provided overlay [`PropertySet`] from an optional `.rdp` file, then layers
/// `--prop` overrides on top. Returns an empty set when neither is given.
fn load_overlay(path: Option<&Path>, prop_overrides: Vec<PropOverride>) -> anyhow::Result<PropertySet> {
    let mut properties = PropertySet::new();
    if let Some(path) = path {
        let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        if let Err(errors) = ironrdp_rdpfile::load(&mut properties, &text) {
            for error in &errors {
                eprintln!("warning: skipped entry in {}: {error}", path.display());
            }
        }
    }
    apply_prop_overrides(&mut properties, prop_overrides);
    Ok(properties)
}

/// Builds a `Connect` request by merging an optional `.rdp` file with CLI overrides into one
/// [`PropertySet`]. Configuration validation happens daemon-side (via
/// `ConfigBuilder::from_property_set`); this only parses and merges the inputs.
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

    // `--prop` overrides win over the .rdp file but lose to the named flags below.
    apply_prop_overrides(&mut properties, args.prop);

    // Named CLI flags win over everything above.
    if let Some(server) = args.server {
        let address: TargetAddr = server
            .parse()
            .with_context(|| format!("invalid server address: {server}"))?;
        properties.set_full_address(&address);
    }
    if let Some(username) = args.username {
        properties.set_username(username);
    }
    if let Some(password) = args.password {
        properties.set_clear_text_password(password);
    }
    if let Some(domain) = args.domain {
        properties.set_domain(domain);
    }

    #[cfg(windows)]
    {
        // Sandbox defaults are the base; explicit .rdp / --prop / named flags win on conflict.
        // Transport/security invariants from the sandbox path are re-applied last so a file
        // cannot force TLS/CredSSP onto a NamedPipe session.
        if let Some(sandbox_id) = args.sandbox_id {
            let sandbox_props =
                crate::sandbox::properties_for_sandbox_id(&sandbox_id).context("resolve Windows Sandbox RDP config")?;
            let mut merged = sandbox_props;
            merged.merge(&properties);
            crate::sandbox::reassert_named_pipe_security(&mut merged);
            properties = merged;
        } else if let Some(pipe) = args.sandbox_pipe {
            let user = properties
                .username()
                .map(str::to_owned)
                .context("--sandbox-pipe requires --username (or username in the .rdp file)")?;
            let pass = properties
                .clear_text_password()
                .map(str::to_owned)
                .context("--sandbox-pipe requires --password (or ClearTextPassword in the .rdp file)")?;
            let mut merged = crate::sandbox::properties_for_pipe(&pipe, &user, &pass);
            merged.merge(&properties);
            // Keep the explicit pipe path and plain-security defaults after user overrides.
            merged.set_named_pipe(if pipe.starts_with(r"\\.\pipe\") || pipe.starts_with(r"\\?\pipe\") {
                pipe
            } else {
                format!(r"\\.\pipe\{pipe}")
            });
            crate::sandbox::reassert_named_pipe_security(&mut merged);
            properties = merged;
        }
    }

    Ok(Request::Connect {
        properties,
        log_directive: args.log_directive,
    })
}

#[cfg(windows)]
fn run_sandbox_command(args: SandboxArgs) -> anyhow::Result<()> {
    match args.command {
        SandboxCommand::List => {
            let ids = crate::sandbox::list_sandbox_ids().context("list Windows sandboxes")?;
            if ids.is_empty() {
                println!("(no running sandboxes)");
            } else {
                for id in ids {
                    println!("{id}");
                }
            }
            Ok(())
        }
        SandboxCommand::Config { id } => {
            let cfg = crate::sandbox::get_rdp_config(&id).context("get sandbox RDP config")?;
            crate::sandbox::print_config_summary(&cfg);
            Ok(())
        }
        SandboxCommand::Stop { id } => {
            crate::sandbox::stop_sandbox(&id).context("stop sandbox")?;
            println!("stopped {id}");
            Ok(())
        }
    }
}

async fn run_rail(endpoint: &Endpoint, args: RailArgs) -> anyhow::Result<()> {
    let RailArgs { format, command } = args;
    let request = match command {
        RailCommand::Status => Request::RailStatus,
        RailCommand::Events { after_sequence } => Request::RailEvents { after_sequence },
        RailCommand::Wait {
            after_sequence,
            timeout_ms,
        } => Request::RailWait {
            after_sequence,
            timeout_ms,
        },
        RailCommand::Execute {
            executable,
            working_directory,
            arguments,
            flags,
        } => Request::RailExecute(RailExecuteRequest {
            executable,
            working_directory: working_directory.unwrap_or_default(),
            arguments: arguments.unwrap_or_default(),
            flags,
        }),
    };
    let response = transport::send_request(endpoint, &request).await?;
    let payload = match response {
        Response::Ok(payload) => payload,
        Response::Err(error) => return Err(RailRequestError(error).into()),
    };
    print_rail_payload(payload, format)
}

fn print_rail_payload(payload: Payload, format: OutputFormat) -> anyhow::Result<()> {
    use serde_json::json;

    match format {
        OutputFormat::Human => match payload {
            Payload::RailStatus(status) => {
                println!("generation: {}", status.generation);
                println!("next sequence: {}", status.next_sequence);
                println!("handshake complete: {}", status.handshake_complete);
                println!("desktop synchronized: {}", status.desktop_synchronized);
                for launch in status.pending_launches {
                    println!(
                        "pending launch {}: {} (flags 0x{:04x})",
                        launch.launch_id, launch.executable, launch.flags
                    );
                }
            }
            Payload::RailEvents(events) => {
                println!("generation: {}", events.generation);
                for event in events.events {
                    print_rail_event(&event);
                }
            }
            Payload::RailLaunch(launch) => {
                println!(
                    "queued launch {}: {} (flags 0x{:04x})",
                    launch.launch_id, launch.executable, launch.flags
                );
            }
            _ => anyhow::bail!("unexpected response to RAIL request"),
        },
        OutputFormat::Json => {
            let value = match payload {
                Payload::RailStatus(status) => json!({
                    "type": "rail_status",
                    "generation": status.generation,
                    "next_sequence": status.next_sequence,
                    "handshake_complete": status.handshake_complete,
                    "desktop_synchronized": status.desktop_synchronized,
                    "pending_launches": status.pending_launches.iter().map(rail_launch_json).collect::<Vec<_>>(),
                }),
                Payload::RailEvents(events) => json!({
                    "type": "rail_events",
                    "generation": events.generation,
                    "events": events.events.iter().map(rail_event_json).collect::<Vec<_>>(),
                }),
                Payload::RailLaunch(launch) => json!({
                    "type": "rail_launch",
                    "launch": rail_launch_json(&launch),
                }),
                _ => anyhow::bail!("unexpected response to RAIL request"),
            };
            println!("{}", serde_json::to_string(&value)?);
        }
        OutputFormat::Ndjson => match payload {
            Payload::RailEvents(events) => {
                for event in events.events {
                    println!(
                        "{}",
                        serde_json::to_string(&json!({
                            "type": "rail_event",
                            "generation": events.generation,
                            "event": rail_event_json(&event),
                        }))?
                    );
                }
            }
            Payload::RailStatus(status) => println!(
                "{}",
                serde_json::to_string(&json!({
                    "type": "rail_status",
                    "generation": status.generation,
                    "next_sequence": status.next_sequence,
                    "handshake_complete": status.handshake_complete,
                    "desktop_synchronized": status.desktop_synchronized,
                    "pending_launches": status.pending_launches.iter().map(rail_launch_json).collect::<Vec<_>>(),
                }))?
            ),
            Payload::RailLaunch(launch) => println!(
                "{}",
                serde_json::to_string(&json!({
                    "type": "rail_launch",
                    "launch": rail_launch_json(&launch),
                }))?
            ),
            _ => anyhow::bail!("unexpected response to RAIL request"),
        },
    }
    Ok(())
}

fn print_rail_event(event: &RailEvent) {
    match &event.kind {
        RailEventKind::Handshake {
            handshake_ex_flags,
            initialization_message_count,
            queued_execute_count,
        } => println!(
            "{}: handshake flags={handshake_ex_flags:?} initialization_messages={initialization_message_count} queued_executes={queued_execute_count}",
            event.sequence
        ),
        RailEventKind::DesktopSynchronized { released_execute_count } => println!(
            "{}: desktop synchronized released_executes={released_execute_count}",
            event.sequence
        ),
        RailEventKind::PostHandshakeQueueReleased { released_execute_count } => println!(
            "{}: post-handshake queue released executes={released_execute_count}",
            event.sequence
        ),
        RailEventKind::ExecuteQueued(launch) => println!(
            "{}: queued launch {} {} flags=0x{:04x}",
            event.sequence, launch.launch_id, launch.executable, launch.flags
        ),
        RailEventKind::ExecuteResult {
            launch_id,
            executable,
            flags,
            result,
            raw_result,
        } => println!(
            "{}: execute result launch={launch_id:?} executable={executable} flags=0x{flags:04x} result=0x{result:04x} raw=0x{raw_result:08x}",
            event.sequence
        ),
        RailEventKind::ExecuteFailed {
            launch_id,
            executable,
            flags,
            reason,
        } => println!(
            "{}: execute failed launch={launch_id:?} executable={executable} flags=0x{flags:04x} reason={}",
            event.sequence,
            reason.as_str()
        ),
        RailEventKind::ApplicationId {
            window_id,
            application_id,
            process_id,
            process_image_name,
        } => println!(
            "{}: application ID window=0x{window_id:08x} application={application_id} process={process_id:?} image={process_image_name:?}",
            event.sequence
        ),
        RailEventKind::Control { kind } => println!("{}: control {kind}", event.sequence),
        RailEventKind::WindowingOrders { byte_count } => {
            println!("{}: validated windowing orders ({byte_count} bytes)", event.sequence)
        }
        RailEventKind::Gap { lost_through } => {
            println!("{}: history gap through sequence {lost_through}", event.sequence)
        }
    }
}

fn rail_launch_json(launch: &ironrdp_rpc::ipc::RailLaunchInfo) -> serde_json::Value {
    serde_json::json!({
        "launch_id": launch.launch_id,
        "executable": launch.executable,
        "flags": launch.flags,
    })
}

fn rail_event_json(event: &RailEvent) -> serde_json::Value {
    use serde_json::json;

    let kind = match &event.kind {
        RailEventKind::Handshake {
            handshake_ex_flags,
            initialization_message_count,
            queued_execute_count,
        } => json!({
            "kind": "handshake",
            "handshake_ex_flags": handshake_ex_flags,
            "initialization_message_count": initialization_message_count,
            "queued_execute_count": queued_execute_count,
        }),
        RailEventKind::DesktopSynchronized { released_execute_count } => {
            json!({"kind": "desktop_synchronized", "released_execute_count": released_execute_count})
        }
        RailEventKind::PostHandshakeQueueReleased { released_execute_count } => {
            json!({"kind": "post_handshake_queue_released", "released_execute_count": released_execute_count})
        }
        RailEventKind::ExecuteQueued(launch) => json!({"kind": "execute_queued", "launch": rail_launch_json(launch)}),
        RailEventKind::ExecuteResult {
            launch_id,
            executable,
            flags,
            result,
            raw_result,
        } => json!({
            "kind": "execute_result",
            "launch_id": launch_id,
            "executable": executable,
            "flags": flags,
            "result": result,
            "raw_result": raw_result,
        }),
        RailEventKind::ExecuteFailed {
            launch_id,
            executable,
            flags,
            reason,
        } => json!({
            "kind": "execute_failed",
            "launch_id": launch_id,
            "executable": executable,
            "flags": flags,
            "reason": reason.as_str(),
        }),
        RailEventKind::ApplicationId {
            window_id,
            application_id,
            process_id,
            process_image_name,
        } => json!({
            "kind": "application_id",
            "window_id": window_id,
            "application_id": application_id,
            "process_id": process_id,
            "process_image_name": process_image_name,
        }),
        RailEventKind::Control { kind } => json!({"kind": "control", "control": kind}),
        RailEventKind::WindowingOrders { byte_count } => json!({"kind": "windowing_orders", "byte_count": byte_count}),
        RailEventKind::Gap { lost_through } => json!({"kind": "gap", "lost_through": lost_through}),
    };
    json!({
        "sequence": event.sequence,
        "event": kind,
    })
}

async fn run_now(endpoint: &Endpoint, args: NowArgs) -> anyhow::Result<Option<u32>> {
    let NowArgs { format, command } = args;
    match command {
        NowCommand::Capabilities => now_single(endpoint, Request::NowCapabilities, format).await,
        NowCommand::Run(args) => {
            now_single(
                endpoint,
                Request::NowRun {
                    command: args.command,
                    directory: args.directory,
                },
                format,
            )
            .await
        }
        NowCommand::Powershell(args) => {
            let operation_id_file = args.common.operation_id_file.clone();
            let request = build_now_execution(
                NowExecutionKind::PowerShell,
                args.command,
                None,
                args.common,
                !args.profile,
                !args.interactive,
            )?;
            now_execution(endpoint, request, format, operation_id_file).await
        }
        NowCommand::Pwsh(args) => {
            let operation_id_file = args.common.operation_id_file.clone();
            let request = build_now_execution(
                NowExecutionKind::Pwsh,
                args.command,
                None,
                args.common,
                !args.profile,
                !args.interactive,
            )?;
            now_execution(endpoint, request, format, operation_id_file).await
        }
        NowCommand::Exec(NowExecArgs {
            command: NowExecCommand::Process(args),
        }) => {
            let operation_id_file = args.common.operation_id_file.clone();
            let request = build_now_execution(
                NowExecutionKind::Process,
                args.filename,
                args.parameters,
                args.common,
                false,
                false,
            )?;
            now_execution(endpoint, request, format, operation_id_file).await
        }
        NowCommand::Exec(NowExecArgs {
            command: NowExecCommand::Batch(args),
        }) => {
            let operation_id_file = args.common.operation_id_file.clone();
            let request = build_now_execution(NowExecutionKind::Batch, args.command, None, args.common, false, false)?;
            now_execution(endpoint, request, format, operation_id_file).await
        }
        NowCommand::Cancel { operation_id } => now_single(endpoint, Request::NowCancel { operation_id }, format).await,
        NowCommand::List => now_single(endpoint, Request::NowList, format).await,
        NowCommand::Status { operation_id } => now_single(endpoint, Request::NowStatus { operation_id }, format).await,
        NowCommand::Attach {
            operation_id,
            after_sequence,
        } => {
            now_stream(
                endpoint,
                Request::NowAttach {
                    operation_id,
                    after_sequence,
                },
                format,
                None,
                true,
            )
            .await
        }
        NowCommand::Stdin(args) => {
            now_single(
                endpoint,
                Request::NowStdin {
                    operation_id: args.operation_id,
                    data: read_input(&args.input)?,
                    last: args.last,
                },
                format,
            )
            .await
        }
        NowCommand::Diagnostics => now_single(endpoint, Request::NowDiagnostics, format).await,
    }
}

fn build_now_execution(
    kind: NowExecutionKind,
    command: String,
    parameters: Option<String>,
    args: CommonExecutionArgs,
    no_profile: bool,
    non_interactive: bool,
) -> anyhow::Result<NowExecutionRequest> {
    let timeout_ms = args
        .timeout
        .map(|seconds| {
            seconds
                .checked_mul(1_000)
                .ok_or_else(|| anyhow::anyhow!("timeout is too large"))
        })
        .transpose()?;
    Ok(NowExecutionRequest {
        kind,
        command,
        parameters,
        directory: args.directory,
        stdin: args.stdin.as_deref().map(read_input).transpose()?,
        timeout_ms,
        detached: args.detached,
        no_profile,
        non_interactive,
    })
}

fn read_input(path: &Path) -> anyhow::Result<Vec<u8>> {
    if path == Path::new("-") {
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut data).context("read standard input")?;
        Ok(data)
    } else {
        std::fs::read(path).with_context(|| format!("read {}", path.display()))
    }
}

async fn now_execution(
    endpoint: &Endpoint,
    request: NowExecutionRequest,
    format: OutputFormat,
    operation_id_file: Option<PathBuf>,
) -> anyhow::Result<Option<u32>> {
    if request.detached {
        if let Some(path) = operation_id_file {
            let response = transport::send_request(endpoint, &Request::NowExecute(request)).await?;
            let payload = match response {
                Response::Ok(payload) => payload,
                Response::Err(error) => return Err(NowRequestError(error).into()),
            };
            let Payload::NowOperation(operation) = &payload else {
                anyhow::bail!("unexpected response while writing operation ID");
            };
            std::fs::write(&path, format!("{}\n", operation.id))
                .with_context(|| format!("write {}", path.display()))?;
            print_now_payload(&payload, format)?;
            return Ok(payload_remote_exit(&payload));
        }
        return now_single(endpoint, Request::NowExecute(request), format).await;
    }
    now_stream(endpoint, Request::NowExecute(request), format, operation_id_file, false).await
}

async fn now_single(endpoint: &Endpoint, request: Request, format: OutputFormat) -> anyhow::Result<Option<u32>> {
    let response = transport::send_request(endpoint, &request).await?;
    let payload = match response {
        Response::Ok(payload) => payload,
        Response::Err(error) => return Err(NowRequestError(error).into()),
    };
    print_now_payload(&payload, format)?;
    Ok(payload_remote_exit(&payload))
}

async fn now_stream(
    endpoint: &Endpoint,
    request: Request,
    format: OutputFormat,
    operation_id_file: Option<PathBuf>,
    print_initial_human: bool,
) -> anyhow::Result<Option<u32>> {
    let mut stream = transport::open_stream(endpoint, &request).await?;
    let first: Response = transport::read_message(&mut stream).await?;
    let first = match first {
        Response::Ok(payload) => payload,
        Response::Err(error) => return Err(NowRequestError(error).into()),
    };

    let mut exit_code = payload_remote_exit(&first);
    let mut terminal_observed = payload_is_terminal_operation(&first);
    if let Some(path) = operation_id_file {
        let Payload::NowOperation(operation) = &first else {
            anyhow::bail!("unexpected response while writing operation ID");
        };
        std::fs::write(&path, format!("{}\n", operation.id)).with_context(|| format!("write {}", path.display()))?;
    }
    let mut json_values = Vec::new();
    let mut json_output_bytes = 0;
    match format {
        OutputFormat::Human if print_initial_human => print_now_human(&first)?,
        OutputFormat::Human => {}
        OutputFormat::Ndjson => print_now_ndjson(&first)?,
        OutputFormat::Json => push_json_payload(&mut json_values, &mut json_output_bytes, &first)?,
    }

    loop {
        let response: Response = match transport::read_message(&mut stream).await {
            Ok(response) => response,
            Err(error)
                if error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                    matches!(
                        error.kind(),
                        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
                    )
                }) =>
            {
                if !terminal_observed {
                    anyhow::bail!("NOW operation stream closed before a terminal event");
                }
                break;
            }
            Err(error) => return Err(error),
        };
        let payload = match response {
            Response::Ok(payload) => payload,
            Response::Err(error) => return Err(NowRequestError(error).into()),
        };
        if let Payload::NowEvent(event) = &payload {
            match &event.kind {
                OperationEventKind::Completed { exit_code: code } => {
                    terminal_observed = true;
                    exit_code = Some(*code);
                }
                OperationEventKind::Cancelled | OperationEventKind::Failed(_) => {
                    terminal_observed = true;
                    exit_code = Some(1);
                }
                OperationEventKind::Started
                | OperationEventKind::Output { .. }
                | OperationEventKind::CancelAccepted => {}
            }
        }
        match format {
            OutputFormat::Human => print_now_human(&payload)?,
            OutputFormat::Ndjson => print_now_ndjson(&payload)?,
            OutputFormat::Json => push_json_payload(&mut json_values, &mut json_output_bytes, &payload)?,
        }
    }

    if matches!(format, OutputFormat::Json) {
        println!(
            "{}",
            serde_json::to_string(&json_values).context("serialize JSON output")?
        );
    }
    Ok(exit_code)
}

fn print_now_error(error: &AgentError, format: OutputFormat) -> anyhow::Result<()> {
    match format {
        OutputFormat::Human => {
            eprintln!("{}", error.message);
            Ok(())
        }
        OutputFormat::Json | OutputFormat::Ndjson => {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "type": "error",
                    "category": error.category.as_str(),
                    "message": error.message,
                }))
                .context("serialize NOW error")?
            );
            Ok(())
        }
    }
}

fn push_json_payload(
    values: &mut Vec<serde_json::Value>,
    output_bytes: &mut usize,
    payload: &Payload,
) -> anyhow::Result<()> {
    if values.len() == MAX_JSON_STREAM_EVENTS {
        anyhow::bail!("JSON stream exceeds the {MAX_JSON_STREAM_EVENTS}-event limit; use --format ndjson");
    }
    let bytes = payload_output_size(payload);
    *output_bytes = output_bytes
        .checked_add(bytes)
        .ok_or_else(|| anyhow::anyhow!("JSON stream output length overflow"))?;
    if *output_bytes > MAX_JSON_STREAM_OUTPUT {
        anyhow::bail!("JSON stream exceeds the {MAX_JSON_STREAM_OUTPUT}-byte output limit; use --format ndjson");
    }
    values.push(now_payload_json(payload));
    Ok(())
}

fn payload_output_size(payload: &Payload) -> usize {
    match payload {
        Payload::NowEvent(OperationEvent {
            kind: OperationEventKind::Output { data, .. },
            ..
        }) => data.len(),
        _ => 0,
    }
}

fn payload_is_terminal_operation(payload: &Payload) -> bool {
    matches!(
        payload,
        Payload::NowOperation(OperationInfo {
            state: OperationState::Completed
                | OperationState::Cancelled
                | OperationState::Failed
                | OperationState::Detached,
            ..
        })
    )
}

fn print_now_payload(payload: &Payload, format: OutputFormat) -> anyhow::Result<()> {
    match format {
        OutputFormat::Human => print_now_human(payload),
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string(&now_payload_json(payload)).context("serialize JSON output")?
            );
            Ok(())
        }
        OutputFormat::Ndjson => print_now_ndjson(payload),
    }
}

fn print_now_human(payload: &Payload) -> anyhow::Result<()> {
    match payload {
        Payload::Empty => println!("ok"),
        Payload::NowCapabilities(capabilities) => {
            println!("version: {}.{}", capabilities.version_major, capabilities.version_minor);
            println!("run: {}", capabilities.run);
            println!("process: {}", capabilities.process);
            println!("batch: {}", capabilities.batch);
            println!("powershell: {}", capabilities.powershell);
            println!("pwsh: {}", capabilities.pwsh);
            println!("io redirection: {}", capabilities.io_redirection);
        }
        Payload::NowOperation(operation) => print_operation_info(operation),
        Payload::NowOperations(operations) => {
            for operation in operations {
                print_operation_info(operation);
            }
        }
        Payload::NowEvent(event) => print_operation_event(event)?,
        Payload::NowDiagnostics(diagnostics) => {
            println!("endpoint allocated: {}", diagnostics.endpoint_allocated);
            println!("connected: {}", diagnostics.connected);
            if let Some(capabilities) = &diagnostics.capabilities {
                println!("version: {}.{}", capabilities.version_major, capabilities.version_minor);
            }
        }
        _ => print_payload(payload.clone()),
    }
    Ok(())
}

fn print_operation_info(operation: &OperationInfo) {
    println!(
        "operation {}: {:?} ({:?})",
        operation.id, operation.kind, operation.state
    );
    if let Some(exit_code) = operation.exit_code {
        println!("exit code: {exit_code}");
    }
    if let Some(error) = &operation.error {
        eprintln!("{}", error.message);
    }
}

fn print_operation_event(event: &OperationEvent) -> anyhow::Result<()> {
    match &event.kind {
        OperationEventKind::Output {
            stream: NowStream::Stdout,
            data,
            ..
        } => {
            let mut stdout = std::io::stdout();
            stdout.write_all(data).context("write remote stdout")?;
            stdout.flush().context("flush remote stdout")?;
        }
        OperationEventKind::Output {
            stream: NowStream::Stderr,
            data,
            ..
        } => {
            let mut stderr = std::io::stderr();
            stderr.write_all(data).context("write remote stderr")?;
            stderr.flush().context("flush remote stderr")?;
        }
        OperationEventKind::Failed(error) => eprintln!("{}", error.message),
        _ => {}
    }
    Ok(())
}

fn print_now_ndjson(payload: &Payload) -> anyhow::Result<()> {
    println!(
        "{}",
        serde_json::to_string(&now_payload_json(payload)).context("serialize NDJSON output")?
    );
    Ok(())
}

fn now_payload_json(payload: &Payload) -> serde_json::Value {
    use serde_json::json;

    match payload {
        Payload::Empty => json!({"type": "ok"}),
        Payload::NowCapabilities(capabilities) => json!({
            "type": "capabilities",
            "version": {"major": capabilities.version_major, "minor": capabilities.version_minor},
            "heartbeat_ms": capabilities.heartbeat_ms,
            "run": capabilities.run,
            "process": capabilities.process,
            "batch": capabilities.batch,
            "powershell": capabilities.powershell,
            "pwsh": capabilities.pwsh,
            "io_redirection": capabilities.io_redirection,
            "unicode_console": capabilities.unicode_console,
        }),
        Payload::NowOperation(operation) => operation_json(operation),
        Payload::NowOperations(operations) => {
            json!({"type": "operations", "operations": operations.iter().map(operation_json).collect::<Vec<_>>()})
        }
        Payload::NowEvent(event) => match &event.kind {
            OperationEventKind::Output { stream, data, last } => json!({
                "type": "output",
                "operation_id": event.operation_id,
                "sequence": event.sequence,
                "stream": format!("{stream:?}").to_ascii_lowercase(),
                "data": data,
                "last": last,
            }),
            OperationEventKind::Completed { exit_code } => json!({
                "type": "completed", "operation_id": event.operation_id, "sequence": event.sequence, "exit_code": exit_code
            }),
            OperationEventKind::Started => {
                json!({"type": "started", "operation_id": event.operation_id, "sequence": event.sequence})
            }
            OperationEventKind::CancelAccepted => {
                json!({"type": "cancel_accepted", "operation_id": event.operation_id, "sequence": event.sequence})
            }
            OperationEventKind::Cancelled => {
                json!({"type": "cancelled", "operation_id": event.operation_id, "sequence": event.sequence})
            }
            OperationEventKind::Failed(error) => json!({
                "type": "failed",
                "operation_id": event.operation_id,
                "sequence": event.sequence,
                "error": {"category": format!("{:?}", error.category).to_ascii_lowercase(), "message": error.message},
            }),
        },
        Payload::NowDiagnostics(diagnostics) => json!({
            "type": "diagnostics",
            "endpoint_allocated": diagnostics.endpoint_allocated,
            "connected": diagnostics.connected,
            "capabilities": diagnostics.capabilities.as_ref().map(|capabilities| json!({
                "version": {"major": capabilities.version_major, "minor": capabilities.version_minor}
            })),
        }),
        _ => json!({"type": "unsupported_payload"}),
    }
}

fn operation_json(operation: &OperationInfo) -> serde_json::Value {
    serde_json::json!({
        "type": "operation",
        "id": operation.id,
        "kind": format!("{:?}", operation.kind).to_ascii_lowercase(),
        "state": format!("{:?}", operation.state).to_ascii_lowercase(),
        "detached": operation.detached,
        "exit_code": operation.exit_code,
        "retained_output_bytes": operation.retained_output_bytes,
        "next_sequence": operation.next_sequence,
        "error": operation.error.as_ref().map(|error| serde_json::json!({
            "category": format!("{:?}", error.category).to_ascii_lowercase(),
            "message": error.message,
        })),
    })
}

fn payload_remote_exit(payload: &Payload) -> Option<u32> {
    match payload {
        Payload::NowOperation(operation) => operation.exit_code,
        _ => None,
    }
}

/// Maps a remote `u32` process code to the CLI's platform process code contract.
pub fn remote_exit_status(exit_code: u32) -> i32 {
    match exit_code {
        0 => 0,
        1..=255 => i32::try_from(exit_code).unwrap_or(255),
        _ => 255,
    }
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
        // Screenshots are handled out-of-band by `write_screenshot`, never printed.
        Payload::Screenshot { width, height, .. } => println!("frame {width}x{height}"),
        Payload::NowCapabilities(capabilities) => {
            println!("NOW {}.{}", capabilities.version_major, capabilities.version_minor);
        }
        Payload::NowOperation(operation) => print_operation_info(&operation),
        Payload::NowOperations(operations) => {
            for operation in &operations {
                print_operation_info(operation);
            }
        }
        Payload::NowEvent(event) => {
            let _ = print_operation_event(&event);
        }
        Payload::NowDiagnostics(diagnostics) => {
            println!("NOW endpoint allocated: {}", diagnostics.endpoint_allocated);
            println!("NOW connected: {}", diagnostics.connected);
        }
        Payload::RailStatus(status) => {
            println!("RAIL generation: {}", status.generation);
            println!("RAIL handshake complete: {}", status.handshake_complete);
        }
        Payload::RailEvents(events) => {
            println!("RAIL generation: {}", events.generation);
            for event in &events.events {
                print_rail_event(event);
            }
        }
        Payload::RailLaunch(launch) => {
            println!("queued RAIL launch {}: {}", launch.launch_id, launch.executable);
        }
    }
}

/// Writes screenshot PNG bytes to disk, defaulting to `screenshot.png`.
fn write_screenshot(width: u16, height: u16, png: &[u8], path: &Path) -> anyhow::Result<()> {
    std::fs::write(path, png).with_context(|| format!("write {}", path.display()))?;
    println!("wrote {} ({width}x{height}, {} bytes)", path.display(), png.len());
    Ok(())
}

fn endpoint_from_arg(arg: Option<String>, backend: Backend) -> Endpoint {
    match arg {
        Some(value) => transport::endpoint_from_string(value),
        None => match backend {
            Backend::Daemon => transport::default_endpoint_named("ironrdp-agent"),
            Backend::ActiveX => transport::default_endpoint_named("ironrdp-activex"),
        },
    }
}

async fn ensure_activex_backend(endpoint: &Endpoint) -> anyhow::Result<()> {
    match transport::send_request(endpoint, &Request::Status).await {
        Ok(Response::Ok(_)) => Ok(()),
        Ok(Response::Err(error)) => anyhow::bail!("ActiveX RPC endpoint at {endpoint} rejected status: {error}"),
        Err(_) => anyhow::bail!(
            "ActiveX RPC endpoint is unavailable at {endpoint}; start an ActiveX host with IRONRDP_ACTIVEX_RPC=1"
        ),
    }
}

/// Short, LLM-facing descriptions for the configuration keys recognized by [`ironrdp_cfg`], derived
/// locally from the key name when printing a dump (kept out of the wire protocol on purpose).
///
/// Keys are the canonical lowercase `.rdp` names. Secret keys are listed for completeness even
/// though `ConfigBuilder::build` strips them before a session starts, so they never appear in a
/// dump.
fn property_description(key: &str) -> Option<&'static str> {
    // PropertySet keys are case-sensitive and ironrdp-cfg mixes casings (e.g. `ClearTextPassword`),
    // so normalize to lowercase to match the canonical lowercase arms below.
    let description = match key.to_ascii_lowercase().as_str() {
        // â”€â”€ Standard .rdp keys â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
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
        // â”€â”€ RD gateway â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        "gatewayhostname" => "RD gateway host name",
        "gatewayusername" => "RD gateway user name",
        "gatewaypassword" => "RD gateway password (secret)",
        "gatewayusagemethod" => {
            "when to use the RD gateway (0 = direct, 1 = always, 2 = detect, 3 = default, 4 = direct, bypass for local)"
        }
        "gatewaycredentialssource" => {
            "RD gateway credential source (0 = server, 1 = user, 2 = profile, 3 = prompt, 4 = smart card, 5 = logon)"
        }
        // â”€â”€ Kerberos â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        "kdcproxyname" => "Kerberos KDC proxy name",
        "kdcproxyurl" => "Kerberos KDC proxy URL",
        // â”€â”€ IronRDP extensions (ironrdp_ prefix) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
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
        "ironrdp_certificate_validation" => "agent daemon TLS certificate validation policy set at daemon startup",
        "ironrdp_fakeeventsinterval" => "interval in minutes between synthetic keep-alive input events",
        "ironrdp_rdcleanpathtoken" => "RDCleanPath authentication token (secret)",
        "ironrdp_rdcleanpathurl" => "RDCleanPath proxy URL",
        "ironrdp_serverpointer" => "render the server-side pointer instead of a client-drawn pointer (0/1)",
        _ => return None,
    };
    Some(description)
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::path::PathBuf;

    use clap::{CommandFactory as _, Parser as _};

    use super::Command;
    use super::{
        Backend, Cli, CommonExecutionArgs, MAX_UNICODE_TEXT_CHARS, NowExecutionKind, build_now_execution,
        endpoint_from_arg,
    };

    #[test]
    fn backend_endpoint_selection_is_distinct_and_overridable() {
        let daemon = endpoint_from_arg(None, Backend::Daemon).to_string();
        let activex = endpoint_from_arg(None, Backend::ActiveX).to_string();

        assert_ne!(daemon, activex);
        assert!(daemon.contains("ironrdp-agent"));
        assert!(activex.contains("ironrdp-activex"));
        #[cfg(windows)]
        assert_eq!(
            endpoint_from_arg(Some("custom-rpc-endpoint".to_owned()), Backend::ActiveX).to_string(),
            r"\\.\pipe\custom-rpc-endpoint"
        );
        #[cfg(unix)]
        assert_eq!(
            endpoint_from_arg(Some("custom-rpc-endpoint".to_owned()), Backend::ActiveX).to_string(),
            "custom-rpc-endpoint"
        );
    }

    #[cfg(windows)]
    #[test]
    fn daemon_rdpdr_drive_flags_parse_multiple_static_volumes() {
        let cli = Cli::try_parse_from([
            "ironrdp-agent",
            "daemon-start",
            "--rdpdr-drive",
            r"System=C:\",
            "--rdpdr-drive",
            r"Data=D:\",
        ])
        .expect("valid multiple-drive configuration");

        let Some(Command::DaemonStart(args)) = cli.command else {
            panic!("expected daemon-start command");
        };
        assert_eq!(args.rdpdr_drives.len(), 2);
        assert_eq!(args.rdpdr_drives[0].display_name(), "System");
        assert_eq!(args.rdpdr_drives[0].root_path(), PathBuf::from(r"C:\"));
        assert_eq!(args.rdpdr_drives[1].display_name(), "Data");
        assert_eq!(args.rdpdr_drives[1].root_path(), PathBuf::from(r"D:\"));
    }

    #[cfg(windows)]
    #[test]
    fn daemon_rdpdr_drive_flags_reject_invalid_definitions() {
        for drive in ["C:\\", "=C:\\", "Data=", "too-long=C:\\", "Data/C:\\"] {
            assert!(Cli::try_parse_from(["ironrdp-agent", "daemon-start", "--rdpdr-drive", drive]).is_err());
        }
    }

    #[test]
    fn daemon_start_can_skip_certificate_check() {
        let cli = Cli::try_parse_from(["ironrdp-agent", "daemon-start", "--skip-certificate-check"])
            .expect("valid explicit certificate-validation override");

        let Some(Command::DaemonStart(args)) = cli.command else {
            panic!("expected daemon-start command");
        };
        assert!(args.skip_certificate_check);
    }

    #[test]
    fn daemon_start_rejects_superseded_certificate_flag() {
        assert!(Cli::try_parse_from(["ironrdp-agent", "daemon-start", "--ignore-certificates"]).is_err());
    }

    #[test]
    fn connection_flags_use_process_local_environment_defaults() {
        let command = Cli::command();
        let connect = command
            .get_subcommands()
            .find(|command| command.get_name() == "connect")
            .expect("connect subcommand must be registered");

        for (argument, variable) in [
            ("server", "RDP_HOSTNAME"),
            ("username", "RDP_USERNAME"),
            ("password", "RDP_PASSWORD"),
        ] {
            let environment = connect
                .get_arguments()
                .find(|candidate| candidate.get_id() == argument)
                .and_then(clap::Arg::get_env);
            assert_eq!(environment, Some(variable.as_ref()));
        }
    }

    #[test]
    fn shell_is_not_an_agent_command() {
        assert!(Cli::try_parse_from(["ironrdp-agent", "now", "shell"]).is_err());
    }

    #[test]
    fn unicode_text_rejects_empty_and_oversized_requests() {
        assert!(Cli::try_parse_from(["ironrdp-agent", "type-unicode", "--text", ""]).is_err());
        assert!(
            Cli::try_parse_from([
                "ironrdp-agent",
                "type-unicode",
                "--text",
                &"x".repeat(MAX_UNICODE_TEXT_CHARS + 1),
            ])
            .is_err()
        );
        assert!(Cli::try_parse_from(["ironrdp-agent", "type-unicode", "--text", "test"]).is_ok());
    }

    #[test]
    fn powershell_request_defaults_are_safe() {
        let request = build_now_execution(
            NowExecutionKind::PowerShell,
            "Get-Date".to_owned(),
            None,
            CommonExecutionArgs {
                directory: None,
                stdin: None,
                timeout: None,
                detached: false,
                operation_id_file: None,
            },
            true,
            true,
        )
        .expect("valid request");

        assert!(request.no_profile);
        assert!(request.non_interactive);
    }
}
