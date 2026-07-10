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

use core::str::FromStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use base64::Engine as _;
use clap::{Args, CommandFactory as _, Parser, Subcommand, ValueEnum};
use ironrdp_cfg::{PropertySetExt as _, TargetAddr};
use ironrdp_input::MouseButton;
use ironrdp_propertyset::{PropertySet, Value};

use crate::ipc::{
    KeyFilter, NowDiagnostics, NowExecutionKind, NowExecutionRequest, NowOperationInfo, NowOperationState, NowStream,
    Payload, PropValue, Request, Response,
};
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

    /// Render NOW responses as text (default), JSON snapshots, or streaming NDJSON.
    #[arg(long, global = true, value_enum, default_value_t = NowOutputFormat::Text)]
    format: NowOutputFormat,

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
    /// Resize the remote desktop.
    Resize {
        #[arg(long)]
        width: u16,
        #[arg(long)]
        height: u16,
    },
    /// Execute a PowerShell command through the remote NOW agent.
    Now(NowArgs),
}

#[derive(Args, Debug)]
struct DaemonArgs {
    /// Path to a .rdp file whose properties are preloaded as an overlay applied to every `connect`
    /// (overlay wins). Use this to provision any setting out of band — credentials in particular
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

#[derive(Args, Debug)]
struct ScreenshotArgs {
    /// Destination PNG path (defaults to `screenshot.png` in the current directory).
    path: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct NowArgs {
    #[command(subcommand)]
    command: NowCommand,
}

#[derive(Subcommand, Debug)]
enum NowCommand {
    /// Report the NOW DVC protocol version and remote capability bitsets.
    Capabilities,
    /// Request normal cancellation of a streamed NOW operation.
    Cancel {
        /// Operation ID emitted when execution starts.
        operation_id: u64,
    },
    /// List daemon-owned NOW operations, including retained-output and terminal metadata.
    List,
    /// Show the durable state and result metadata for one NOW operation.
    Status {
        /// Locally assigned NOW operation ID.
        operation_id: u64,
    },
    /// Replay retained output after a sequence and continue until the operation finishes.
    Attach {
        /// Locally assigned NOW operation ID.
        operation_id: u64,
        /// Do not replay events at or below this operation-local sequence number.
        #[arg(long, default_value_t = 0)]
        after_sequence: u64,
    },
    /// Show local DVC endpoint readiness and operation-manager limits.
    Diagnostics,
    /// Stream local input to a running NOW operation without buffering it in the command that started it.
    Stdin {
        /// Locally assigned NOW operation ID.
        operation_id: u64,
        /// Read input from PATH instead of this command's standard input.
        #[arg(long, value_name = "PATH")]
        file: Option<PathBuf>,
    },
    /// Execute with Windows PowerShell 5 (`powershell.exe`).
    Powershell(NowPowerShellArgs),
    /// Execute with PowerShell 7 (`pwsh.exe`).
    Pwsh(NowPowerShellArgs),
    /// Execute a capability-gated non-PowerShell NOW operation.
    Exec(NowExecArgs),
}

#[derive(Args, Debug)]
struct NowPowerShellArgs {
    /// Load the remote user's PowerShell profile instead of the safe default `-NoProfile`.
    #[arg(long)]
    profile: bool,
    /// Permit interactive PowerShell behavior instead of the safe default `-NonInteractive`.
    #[arg(long)]
    interactive: bool,
    /// The PowerShell command to run. Quote it when it contains spaces or shell metacharacters.
    #[arg(required_unless_present = "file", conflicts_with = "file")]
    command: Option<String>,
    /// Read the PowerShell command from a UTF-8 script file instead of the positional command.
    #[arg(long, value_name = "PATH", conflicts_with = "command")]
    file: Option<PathBuf>,
    #[command(flatten)]
    execution: NowExecutionOptions,
}

#[derive(Args, Debug)]
struct NowExecutionOptions {
    /// Remote working directory.
    #[arg(long)]
    directory: Option<String>,
    /// Cancel after this many seconds.
    #[arg(long, value_name = "SECONDS")]
    timeout: Option<u32>,
    /// Buffer raw bytes from PATH and forward them after the operation starts. Use `-` for local
    /// stdin. For live, bounded input to a separately started operation, use `now stdin`.
    #[arg(long, value_name = "PATH")]
    stdin: Option<PathBuf>,
    /// Start without waiting for a remote result or receiving redirected output.
    #[arg(long)]
    detached: bool,
    /// Write the locally assigned operation ID to PATH as soon as the daemon accepts it.
    #[arg(long, value_name = "PATH")]
    operation_id_file: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct NowExecArgs {
    #[command(subcommand)]
    command: NowExecCommand,
}

#[derive(Subcommand, Debug)]
enum NowExecCommand {
    /// Invoke a remote executable using CreateProcess.
    Process(NowProcessArgs),
    /// Invoke a command through the remote host's configured shell.
    Shell(NowShellArgs),
    /// Invoke a Windows batch command.
    Batch(NowBatchArgs),
}

#[derive(Args, Debug)]
struct NowProcessArgs {
    /// Executable path.
    filename: String,
    /// Raw command-line parameters passed to the executable. Mutually exclusive with `--arg`.
    #[arg(long, conflicts_with = "arg")]
    parameters: Option<String>,
    /// One Windows command-line argument. Repeat to build `--parameters` using deterministic
    /// CreateProcess-compatible quoting. The remote program still owns final argument parsing.
    #[arg(long, conflicts_with = "parameters")]
    arg: Vec<String>,
    #[command(flatten)]
    execution: NowExecutionOptions,
}

#[derive(Args, Debug)]
struct NowShellArgs {
    /// Shell command text.
    #[arg(required_unless_present = "file", conflicts_with = "file")]
    command: Option<String>,
    /// Read UTF-8 shell command text from PATH instead of the positional command.
    #[arg(long, value_name = "PATH", conflicts_with = "command")]
    file: Option<PathBuf>,
    /// Optional remote shell executable.
    #[arg(long)]
    shell: Option<String>,
    #[command(flatten)]
    execution: NowExecutionOptions,
}

#[derive(Args, Debug)]
struct NowBatchArgs {
    /// Batch command text.
    #[arg(required_unless_present = "file", conflicts_with = "file")]
    command: Option<String>,
    /// Read UTF-8 batch command text from PATH instead of the positional command.
    #[arg(long, value_name = "PATH", conflicts_with = "command")]
    file: Option<PathBuf>,
    #[command(flatten)]
    execution: NowExecutionOptions,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliMouseButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum NowOutputFormat {
    /// Human-oriented output; execution writes remote stdout/stderr bytes unchanged.
    #[default]
    Text,
    /// One JSON object for a non-streaming NOW query.
    Json,
    /// One JSON object per NOW lifecycle/output event.
    Ndjson,
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

/// Entry point shared by the binary: dispatches the parsed [`Cli`].
pub async fn run(cli: Cli) -> anyhow::Result<()> {
    if cli.help_agent {
        print!("{}", crate::help::AGENT_GUIDE);
        return Ok(());
    }

    let format = cli.format;
    let endpoint = endpoint_from_arg(cli.endpoint);

    let Some(command) = cli.command else {
        let _ = Cli::command().print_help();
        println!();
        return Ok(());
    };

    let request = match command {
        Command::DaemonStart(args) => {
            let overlay = load_overlay(args.overlay.as_deref(), args.prop)?;
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
        Command::Screenshot(args) => {
            let response = transport::send_request(&endpoint, &Request::Screenshot).await?;
            let payload = match response {
                Response::Ok(payload) => payload,
                Response::Err(error) => anyhow::bail!("{}", error.message),
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
        Command::Resize { width, height } => Request::Resize { width, height },
        Command::Now(args) => {
            return match args.command {
                NowCommand::Capabilities => print_now_response(
                    transport::send_request(&endpoint, &Request::NowCapabilities).await?,
                    format,
                ),
                NowCommand::Cancel { operation_id } => print_now_response(
                    transport::send_request(&endpoint, &Request::NowCancel { operation_id }).await?,
                    format,
                ),
                NowCommand::List => print_now_response(
                    transport::send_request(&endpoint, &Request::NowOperations).await?,
                    format,
                ),
                NowCommand::Status { operation_id } => print_now_response(
                    transport::send_request(&endpoint, &Request::NowOperationStatus { operation_id }).await?,
                    format,
                ),
                NowCommand::Attach {
                    operation_id,
                    after_sequence,
                } => execute_now_attachment(&endpoint, operation_id, after_sequence, format).await,
                NowCommand::Diagnostics => print_now_response(
                    transport::send_request(&endpoint, &Request::NowDiagnostics).await?,
                    format,
                ),
                NowCommand::Stdin { operation_id, file } => {
                    send_now_stdin(&endpoint, operation_id, file.as_deref(), format).await
                }
                NowCommand::Powershell(args) => {
                    let command = read_powershell_command(args.command, args.file)?;
                    execute_now(
                        &endpoint,
                        NowExecutionRequest {
                            kind: NowExecutionKind::WindowsPowerShell,
                            command,
                            parameters: None,
                            directory: args.execution.directory,
                            no_profile: !args.profile,
                            non_interactive: !args.interactive,
                            detached: args.execution.detached,
                            timeout_secs: args.execution.timeout,
                            stdin: read_standard_input(args.execution.stdin.as_deref())?,
                        },
                        args.execution.operation_id_file.as_deref(),
                        format,
                    )
                    .await
                }
                NowCommand::Pwsh(args) => {
                    let command = read_powershell_command(args.command, args.file)?;
                    execute_now(
                        &endpoint,
                        NowExecutionRequest {
                            kind: NowExecutionKind::PowerShell,
                            command,
                            parameters: None,
                            directory: args.execution.directory,
                            no_profile: !args.profile,
                            non_interactive: !args.interactive,
                            detached: args.execution.detached,
                            timeout_secs: args.execution.timeout,
                            stdin: read_standard_input(args.execution.stdin.as_deref())?,
                        },
                        args.execution.operation_id_file.as_deref(),
                        format,
                    )
                    .await
                }
                NowCommand::Exec(NowExecArgs { command }) => {
                    let (kind, command, parameters, execution) = match command {
                        NowExecCommand::Process(args) => {
                            let parameters = args
                                .parameters
                                .or_else(|| (!args.arg.is_empty()).then(|| windows_command_line(&args.arg)));
                            (NowExecutionKind::Process, args.filename, parameters, args.execution)
                        }
                        NowExecCommand::Shell(args) => (
                            NowExecutionKind::Shell,
                            read_powershell_command(args.command, args.file)?,
                            args.shell,
                            args.execution,
                        ),
                        NowExecCommand::Batch(args) => (
                            NowExecutionKind::Batch,
                            read_powershell_command(args.command, args.file)?,
                            None,
                            args.execution,
                        ),
                    };
                    execute_now(
                        &endpoint,
                        NowExecutionRequest {
                            kind,
                            command,
                            parameters,
                            directory: execution.directory,
                            no_profile: true,
                            non_interactive: true,
                            detached: execution.detached,
                            timeout_secs: execution.timeout,
                            stdin: read_standard_input(execution.stdin.as_deref())?,
                        },
                        execution.operation_id_file.as_deref(),
                        format,
                    )
                    .await
                }
            };
        }
    };

    let response = transport::send_request(&endpoint, &request).await?;
    print_response(response)
}

#[derive(Debug)]
struct RemoteExit {
    code: u32,
}

impl core::fmt::Display for RemoteExit {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "remote NOW command exited with status {}", self.code)
    }
}

impl core::error::Error for RemoteExit {}

/// Returns a process exit status for a nonzero remote PowerShell exit, if this is such an error.
pub fn remote_exit_code(error: &anyhow::Error) -> Option<i32> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<RemoteExit>())
        .map(|exit| {
            // Shells conventionally expose only 8-bit process statuses. Preserve the remote
            // code where that is portable and map wider nonzero values to a nonzero failure.
            i32::try_from(exit.code)
                .ok()
                .filter(|code| (1..=255).contains(code))
                .unwrap_or(255)
        })
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
        Response::Err(error) => anyhow::bail!("{}", error.message),
    }
}

#[cfg(test)]
fn write_powershell_response(response: Response) -> anyhow::Result<()> {
    let payload = match response {
        Response::Ok(payload) => payload,
        Response::Err(error) => anyhow::bail!("{}", error.message),
    };
    let Payload::PowerShell {
        stdout,
        stderr,
        exit_code,
    } = payload
    else {
        anyhow::bail!("unexpected response to NOW PowerShell request");
    };

    std::io::stdout().write_all(&stdout).context("write remote stdout")?;
    std::io::stdout().flush().context("flush remote stdout")?;
    std::io::stderr().write_all(&stderr).context("write remote stderr")?;
    std::io::stderr().flush().context("flush remote stderr")?;
    if exit_code != 0 {
        return Err(RemoteExit { code: exit_code }.into());
    }
    Ok(())
}

fn read_powershell_command(command: Option<String>, file: Option<PathBuf>) -> anyhow::Result<String> {
    match (command, file) {
        (Some(command), None) => Ok(command),
        (None, Some(path)) => {
            std::fs::read_to_string(&path).with_context(|| format!("read PowerShell script {}", path.display()))
        }
        (Some(_), Some(_)) => anyhow::bail!("provide either a PowerShell command or --file, not both"),
        (None, None) => anyhow::bail!("provide a PowerShell command or --file"),
    }
}

fn read_standard_input(path: Option<&Path>) -> anyhow::Result<Option<Vec<u8>>> {
    let Some(path) = path else {
        return Ok(None);
    };
    if path == Path::new("-") {
        let mut stdin = std::io::stdin().lock();
        let mut bytes = Vec::new();
        stdin.read_to_end(&mut bytes).context("read local standard input")?;
        return Ok(Some(bytes));
    }

    std::fs::read(path)
        .map(Some)
        .with_context(|| format!("read NOW standard input {}", path.display()))
}

/// Builds a Windows command line from explicitly delimited arguments. This follows the
/// `CommandLineToArgvW` backslash/quote rules used by common Windows C runtimes; callers that need
/// shell-specific syntax must use the explicit `--parameters` string instead.
fn windows_command_line(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| {
            if !argument.is_empty() && !argument.bytes().any(|byte| matches!(byte, b' ' | b'\t' | b'"')) {
                return argument.clone();
            }
            let mut quoted = String::with_capacity(argument.len() + 2);
            quoted.push('"');
            let mut backslashes = 0;
            for character in argument.chars() {
                if character == '\\' {
                    backslashes += 1;
                } else if character == '"' {
                    quoted.extend(core::iter::repeat_n('\\', backslashes * 2 + 1));
                    quoted.push('"');
                    backslashes = 0;
                } else {
                    quoted.extend(core::iter::repeat_n('\\', backslashes));
                    quoted.push(character);
                    backslashes = 0;
                }
            }
            quoted.extend(core::iter::repeat_n('\\', backslashes * 2));
            quoted.push('"');
            quoted
        })
        .collect::<Vec<_>>()
        .join(" ")
}

async fn send_now_stdin(
    endpoint: &Endpoint,
    operation_id: u64,
    file: Option<&Path>,
    format: NowOutputFormat,
) -> anyhow::Result<()> {
    let mut source: Box<dyn Read> = match file {
        Some(path) => {
            Box::new(std::fs::File::open(path).with_context(|| format!("open NOW standard input {}", path.display()))?)
        }
        None => Box::new(std::io::stdin()),
    };
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let count = source.read(&mut buffer).context("read local standard input")?;
        if count == 0 {
            send_now_stdin_chunk(endpoint, operation_id, Vec::new(), true).await?;
            if matches!(format, NowOutputFormat::Ndjson) {
                print_ndjson(serde_json::json!({
                    "schema": "ironrdp-agent.now.v1",
                    "type": "stdin_closed",
                    "operation_id": operation_id,
                }))?;
            }
            return Ok(());
        }
        send_now_stdin_chunk(endpoint, operation_id, buffer[..count].to_vec(), false).await?;
    }
}

async fn send_now_stdin_chunk(endpoint: &Endpoint, operation_id: u64, data: Vec<u8>, last: bool) -> anyhow::Result<()> {
    loop {
        let response = transport::send_request(
            endpoint,
            &Request::NowWriteStdin {
                operation_id,
                data: data.clone(),
                last,
            },
        )
        .await?;
        match response {
            Response::Ok(Payload::NowStdinAccepted { .. }) => return Ok(()),
            Response::Err(error) if error.message.contains("backpressured") => {
                tokio::time::sleep(core::time::Duration::from_millis(10)).await;
            }
            Response::Err(error) => anyhow::bail!("{}", error.message),
            Response::Ok(payload) => anyhow::bail!("unexpected response to NOW standard input: {payload:?}"),
        }
    }
}

async fn execute_now(
    endpoint: &Endpoint,
    request: NowExecutionRequest,
    operation_id_file: Option<&Path>,
    format: NowOutputFormat,
) -> anyhow::Result<()> {
    if matches!(format, NowOutputFormat::Json) {
        anyhow::bail!("NOW execution is streamed; use --format text or --format ndjson");
    }
    stream_now_request(endpoint, Request::NowExecute(request), operation_id_file, None, format).await
}

async fn execute_now_attachment(
    endpoint: &Endpoint,
    operation_id: u64,
    after_sequence: u64,
    format: NowOutputFormat,
) -> anyhow::Result<()> {
    if matches!(format, NowOutputFormat::Json) {
        anyhow::bail!("NOW operation attachment is streamed; use --format text or --format ndjson");
    }
    stream_now_request(
        endpoint,
        Request::NowOperationAttach {
            operation_id,
            after_sequence,
        },
        None,
        Some(operation_id),
        format,
    )
    .await
}

async fn stream_now_request(
    endpoint: &Endpoint,
    request: Request,
    operation_id_file: Option<&Path>,
    known_operation_id: Option<u64>,
    format: NowOutputFormat,
) -> anyhow::Result<()> {
    let mut stream = transport::connect(endpoint)
        .await
        .with_context(|| format!("connect to daemon at {endpoint}"))?;
    transport::write_message(&mut stream, &request).await?;
    let mut operation_id = known_operation_id;
    let mut cancellation_requested = false;
    let interrupt = tokio::signal::ctrl_c();
    tokio::pin!(interrupt);

    loop {
        let response = tokio::select! {
            response = transport::read_message(&mut stream) => response?,
            _ = &mut interrupt, if !cancellation_requested && operation_id.is_some() => {
                let operation_id = operation_id.expect("select guard checked operation ID");
                let response = transport::send_request(endpoint, &Request::NowCancel { operation_id }).await?;
                match response {
                    Response::Ok(Payload::NowCancelAccepted { .. }) => {
                        emit_now_control_event("cancellation_requested", operation_id, format)?;
                    }
                    Response::Err(error) => anyhow::bail!("{}", error.message),
                    Response::Ok(payload) => anyhow::bail!("unexpected response to NOW cancellation: {payload:?}"),
                }
                cancellation_requested = true;
                continue;
            }
        };
        match response {
            Response::Err(error) => {
                if matches!(format, NowOutputFormat::Ndjson) {
                    print_ndjson(serde_json::json!({
                        "schema": "ironrdp-agent.now.v1",
                        "type": "error",
                        "code": error.code.as_str(),
                        "message": error.message,
                        "operation_id": operation_id,
                    }))?;
                }
                anyhow::bail!("{}", error.message)
            }
            Response::Ok(Payload::NowExecutionStarted {
                operation_id: started_id,
            }) => {
                operation_id = Some(started_id);
                if let Some(path) = operation_id_file {
                    std::fs::write(path, started_id.to_string())
                        .with_context(|| format!("write NOW operation ID {}", path.display()))?;
                }
                emit_now_control_event("started", started_id, format)?;
            }
            Response::Ok(Payload::NowExecutionData {
                operation_id,
                sequence,
                stream: NowStream::Stdout,
                data,
            }) => {
                emit_now_data(operation_id, sequence, NowStream::Stdout, &data, format)?;
            }
            Response::Ok(Payload::NowExecutionData {
                operation_id,
                sequence,
                stream: NowStream::Stderr,
                data,
            }) => {
                emit_now_data(operation_id, sequence, NowStream::Stderr, &data, format)?;
            }
            Response::Ok(Payload::NowExecutionResult {
                operation_id,
                exit_code: result_exit_code,
            }) => {
                emit_now_result(operation_id, result_exit_code, format)?;
                if result_exit_code != 0 {
                    return Err(RemoteExit { code: result_exit_code }.into());
                }
                return Ok(());
            }
            Response::Ok(Payload::NowOperationInfo(info)) => {
                print_now_info(&info, format)?;
                if matches!(info.state, NowOperationState::Detached) {
                    return Ok(());
                }
                anyhow::bail!("unexpected non-terminal NOW operation attachment response")
            }
            Response::Ok(payload) => anyhow::bail!("unexpected response to streamed NOW execution: {payload:?}"),
        }
    }
}

fn emit_now_control_event(kind: &str, operation_id: u64, format: NowOutputFormat) -> anyhow::Result<()> {
    match format {
        // Text execution remains byte-for-byte compatible: only remote stdout/stderr are emitted.
        NowOutputFormat::Text => {}
        NowOutputFormat::Ndjson => print_ndjson(serde_json::json!({
            "schema": "ironrdp-agent.now.v1",
            "type": kind,
            "operation_id": operation_id,
        }))?,
        NowOutputFormat::Json => print_ndjson(serde_json::json!({
            "schema": "ironrdp-agent.now.v1",
            "type": kind,
            "operation_id": operation_id,
        }))?,
    }
    Ok(())
}

fn emit_now_data(
    operation_id: u64,
    sequence: u64,
    stream: NowStream,
    data: &[u8],
    format: NowOutputFormat,
) -> anyhow::Result<()> {
    match format {
        NowOutputFormat::Text => {
            let output: &mut dyn Write = match stream {
                NowStream::Stdout => &mut std::io::stdout(),
                NowStream::Stderr => &mut std::io::stderr(),
            };
            output.write_all(data).context("write remote NOW output")?;
            output.flush().context("flush remote NOW output")?;
        }
        NowOutputFormat::Ndjson => print_ndjson(now_data_json(operation_id, sequence, stream, data))?,
        NowOutputFormat::Json => unreachable!("streaming mode rejects JSON"),
    }
    Ok(())
}

fn now_data_json(operation_id: u64, sequence: u64, stream: NowStream, data: &[u8]) -> serde_json::Value {
    serde_json::json!({
        "schema": "ironrdp-agent.now.v1",
        "type": "data",
        "operation_id": operation_id,
        "sequence": sequence,
        "stream": match stream { NowStream::Stdout => "stdout", NowStream::Stderr => "stderr" },
        "data_base64": base64::engine::general_purpose::STANDARD.encode(data),
    })
}

fn emit_now_result(operation_id: u64, exit_code: u32, format: NowOutputFormat) -> anyhow::Result<()> {
    match format {
        // Text execution remains byte-for-byte compatible: only remote stdout/stderr are emitted.
        NowOutputFormat::Text => {}
        NowOutputFormat::Ndjson => print_ndjson(serde_json::json!({
            "schema": "ironrdp-agent.now.v1",
            "type": "result",
            "operation_id": operation_id,
            "exit_code": exit_code,
        }))?,
        NowOutputFormat::Json => unreachable!("streaming mode rejects JSON"),
    }
    Ok(())
}

fn print_ndjson(value: serde_json::Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string(&value).context("serialize NOW NDJSON")?);
    Ok(())
}

fn print_now_response(response: Response, format: NowOutputFormat) -> anyhow::Result<()> {
    match response {
        Response::Ok(payload) => print_now_payload(payload, format),
        Response::Err(error) => {
            if matches!(format, NowOutputFormat::Json | NowOutputFormat::Ndjson) {
                print_ndjson(serde_json::json!({
                    "schema": "ironrdp-agent.now.v1",
                    "type": "error",
                    "code": error.code.as_str(),
                    "message": error.message,
                }))?;
            }
            anyhow::bail!("{}", error.message)
        }
    }
}

fn print_now_payload(payload: Payload, format: NowOutputFormat) -> anyhow::Result<()> {
    match payload {
        Payload::NowCapabilities(capabilities) => match format {
            NowOutputFormat::Text => print_now_capabilities(&capabilities),
            NowOutputFormat::Json | NowOutputFormat::Ndjson => print_ndjson(serde_json::json!({
                "schema": "ironrdp-agent.now.v1",
                "type": "capabilities",
                "version": { "major": capabilities.major, "minor": capabilities.minor },
                "server": {
                    "system_capset": capabilities.system_capset,
                    "session_capset": capabilities.session_capset,
                    "exec_capset": capabilities.server_exec_capset,
                },
                "agent_advertised_exec_capset": capabilities.client_exec_capset,
                "agent_exposed_exec_capset": capabilities.exec_capset,
                "heartbeat_secs": capabilities.heartbeat_secs,
            }))?,
        },
        Payload::NowCancelAccepted { operation_id } => {
            emit_now_control_event("cancellation_requested", operation_id, format)?
        }
        Payload::NowOperationInfo(info) => print_now_info(&info, format)?,
        Payload::NowOperations(operations) => match format {
            NowOutputFormat::Text => {
                for info in &operations {
                    print_now_info_text(info);
                }
            }
            NowOutputFormat::Json | NowOutputFormat::Ndjson => {
                let operations = operations.iter().map(now_operation_json).collect::<Vec<_>>();
                print_ndjson(serde_json::json!({
                    "schema": "ironrdp-agent.now.v1",
                    "type": "operations",
                    "operations": operations,
                }))?;
            }
        },
        Payload::NowDiagnostics(diagnostics) => print_now_diagnostics(&diagnostics, format)?,
        Payload::NowStdinAccepted { operation_id, last } => {
            if matches!(format, NowOutputFormat::Ndjson) {
                print_ndjson(serde_json::json!({
                    "schema": "ironrdp-agent.now.v1",
                    "type": if last { "stdin_closed" } else { "stdin_accepted" },
                    "operation_id": operation_id,
                }))?;
            }
        }
        payload => anyhow::bail!("unexpected response to NOW request: {payload:?}"),
    }
    Ok(())
}

fn print_now_info(info: &NowOperationInfo, format: NowOutputFormat) -> anyhow::Result<()> {
    match format {
        NowOutputFormat::Text => print_now_info_text(info),
        NowOutputFormat::Json | NowOutputFormat::Ndjson => print_ndjson(serde_json::json!({
            "schema": "ironrdp-agent.now.v1",
            "type": "operation",
            "operation": now_operation_json(info),
        }))?,
    }
    Ok(())
}

fn print_now_info_text(info: &NowOperationInfo) {
    println!("operation id: {}", info.operation_id);
    println!("kind: {:?}", info.kind);
    println!("state: {:?}", info.state);
    println!("started unix ms: {}", info.started_unix_ms);
    if let Some(finished) = info.finished_unix_ms {
        println!("finished unix ms: {finished}");
    }
    if let Some(exit_code) = info.exit_code {
        println!("exit code: {exit_code}");
    }
    if let Some(error) = &info.error {
        println!("error: {error}");
    }
    println!("stdout bytes: {}", info.stdout_bytes);
    println!("stderr bytes: {}", info.stderr_bytes);
    println!("retained bytes: {}", info.retained_bytes);
    println!("dropped bytes: {}", info.dropped_bytes);
    println!("next sequence: {}", info.next_sequence);
}

fn now_operation_json(info: &NowOperationInfo) -> serde_json::Value {
    serde_json::json!({
        "operation_id": info.operation_id,
        "kind": match info.kind {
            NowExecutionKind::WindowsPowerShell => "powershell",
            NowExecutionKind::PowerShell => "pwsh",
            NowExecutionKind::Process => "process",
            NowExecutionKind::Shell => "shell",
            NowExecutionKind::Batch => "batch",
        },
        "state": match info.state {
            NowOperationState::Running => "running",
            NowOperationState::Cancelling => "cancelling",
            NowOperationState::Succeeded => "succeeded",
            NowOperationState::Failed => "failed",
            NowOperationState::Cancelled => "cancelled",
            NowOperationState::Detached => "detached",
        },
        "started_unix_ms": info.started_unix_ms,
        "finished_unix_ms": info.finished_unix_ms,
        "exit_code": info.exit_code,
        "error": info.error,
        "stdout_bytes": info.stdout_bytes,
        "stderr_bytes": info.stderr_bytes,
        "retained_bytes": info.retained_bytes,
        "dropped_bytes": info.dropped_bytes,
        "next_sequence": info.next_sequence,
    })
}

fn print_now_diagnostics(diagnostics: &NowDiagnostics, format: NowOutputFormat) -> anyhow::Result<()> {
    match format {
        NowOutputFormat::Text => {
            println!("endpoint: {}", diagnostics.endpoint);
            println!(
                "active operation id: {}",
                diagnostics
                    .active_operation_id
                    .map_or_else(|| "none".to_owned(), |id| id.to_string())
            );
            println!("output retention bytes: {}", diagnostics.output_retention_bytes);
            println!("event queue capacity: {}", diagnostics.event_queue_capacity);
            println!(
                "initial connect timeout seconds: {}",
                diagnostics.initial_connect_timeout_secs
            );
            println!("reconnect timeout seconds: {}", diagnostics.reconnect_timeout_secs);
        }
        NowOutputFormat::Json | NowOutputFormat::Ndjson => print_ndjson(serde_json::json!({
            "schema": "ironrdp-agent.now.v1",
            "type": "diagnostics",
            "endpoint": diagnostics.endpoint,
            "active_operation_id": diagnostics.active_operation_id,
            "output_retention_bytes": diagnostics.output_retention_bytes,
            "event_queue_capacity": diagnostics.event_queue_capacity,
            "initial_connect_timeout_secs": diagnostics.initial_connect_timeout_secs,
            "reconnect_timeout_secs": diagnostics.reconnect_timeout_secs,
        }))?,
    }
    Ok(())
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
        // NOW PowerShell byte streams are handled out-of-band by `write_powershell_response`.
        Payload::PowerShell { .. } => {}
        Payload::NowCapabilities(capabilities) => print_now_capabilities(&capabilities),
        Payload::NowExecutionStarted { operation_id } => println!("NOW operation {operation_id} started"),
        Payload::NowExecutionData { .. } => {}
        Payload::NowExecutionResult {
            operation_id,
            exit_code,
        } => println!("NOW operation {operation_id} exited with status {exit_code}"),
        Payload::NowCancelAccepted { operation_id } => println!("NOW operation {operation_id} cancellation requested"),
        Payload::NowOperationInfo(info) => print_now_info_text(&info),
        Payload::NowOperations(operations) => {
            for info in &operations {
                print_now_info_text(info);
            }
        }
        Payload::NowDiagnostics(diagnostics) => {
            let _ = print_now_diagnostics(&diagnostics, NowOutputFormat::Text);
        }
        Payload::NowStdinAccepted { operation_id, last } => {
            println!(
                "NOW operation {operation_id} standard input {}",
                if last { "closed" } else { "accepted" }
            );
        }
    }
}

fn print_now_capabilities(capabilities: &crate::ipc::NowCapabilities) {
    let mut system = Vec::new();
    if capabilities.system_capset & 0x0001 != 0 {
        system.push("shutdown");
    }
    let mut session = Vec::new();
    for (flag, name) in [
        (0x0001, "lock"),
        (0x0002, "logoff"),
        (0x0004, "message-box"),
        (0x0008, "set-keyboard-layout"),
        (0x0010, "window-recording"),
    ] {
        if capabilities.session_capset & flag != 0 {
            session.push(name);
        }
    }
    let mut execution = Vec::new();
    for (flag, name) in [
        (0x0001, "run"),
        (0x0002, "process"),
        (0x0004, "shell"),
        (0x0008, "batch"),
        (0x0010, "powershell"),
        (0x0020, "pwsh"),
        (0x0040, "unicode-console"),
        (0x1000, "io-redirection"),
    ] {
        if capabilities.exec_capset & flag != 0 {
            execution.push(name);
        }
    }

    println!("version: {}.{}", capabilities.major, capabilities.minor);
    println!("system: {}", system.join(", "));
    println!("session: {}", session.join(", "));
    println!("execution: {}", execution.join(", "));
    println!("system capset: 0x{:04x}", capabilities.system_capset);
    println!("session capset: 0x{:04x}", capabilities.session_capset);
    println!("server exec capset: 0x{:04x}", capabilities.server_exec_capset);
    println!(
        "agent advertised exec capset: 0x{:04x}",
        capabilities.client_exec_capset
    );
    println!("agent exposed exec capset: 0x{:04x}", capabilities.exec_capset);
    if let Some(heartbeat_secs) = capabilities.heartbeat_secs {
        println!("heartbeat seconds: {heartbeat_secs}");
    }
}

/// Writes screenshot PNG bytes to disk, defaulting to `screenshot.png`.
fn write_screenshot(width: u16, height: u16, png: &[u8], path: &Path) -> anyhow::Result<()> {
    std::fs::write(path, png).with_context(|| format!("write {}", path.display()))?;
    println!("wrote {} ({width}x{height}, {} bytes)", path.display(), png.len());
    Ok(())
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
    // PropertySet keys are case-sensitive and ironrdp-cfg mixes casings (e.g. `ClearTextPassword`),
    // so normalize to lowercase to match the canonical lowercase arms below.
    let description = match key.to_ascii_lowercase().as_str() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_exit_status_is_preserved() {
        let error = anyhow::Error::new(RemoteExit { code: 37 });
        assert_eq!(remote_exit_code(&error), Some(37));
    }

    #[test]
    fn remote_exit_status_is_preserved_through_anyhow_context() {
        let error = anyhow::Error::new(RemoteExit { code: 7 }).context("write NOW PowerShell response");
        assert_eq!(remote_exit_code(&error), Some(7));
    }

    #[test]
    fn wide_remote_exit_status_maps_to_portable_failure() {
        let error = anyhow::Error::new(RemoteExit { code: 256 });
        assert_eq!(remote_exit_code(&error), Some(255));
    }

    #[test]
    fn powershell_agent_errors_are_returned_to_the_cli() {
        let error = write_powershell_response(Response::error("NOW DVC pipe unavailable"))
            .expect_err("agent error must fail the CLI command");
        assert_eq!(error.to_string(), "NOW DVC pipe unavailable");
    }

    #[test]
    fn structured_process_arguments_use_windows_compatible_quoting() {
        assert_eq!(
            windows_command_line(&[
                "plain".to_owned(),
                "two words".to_owned(),
                r#"embedded"quote"#.to_owned(),
                r"C:\path with spaces\".to_owned(),
                String::new(),
            ]),
            r#"plain "two words" "embedded\"quote" "C:\path with spaces\\" """#
        );
    }

    #[test]
    fn ndjson_data_envelope_preserves_non_utf8_stream_bytes() {
        let value = now_data_json(17, 3, NowStream::Stderr, &[0, 0xff]);
        assert_eq!(value["schema"], "ironrdp-agent.now.v1");
        assert_eq!(value["sequence"], 3);
        assert_eq!(value["stream"], "stderr");
        assert_eq!(value["data_base64"], "AP8=");
        assert!(serde_json::from_str::<serde_json::Value>(&value.to_string()).is_ok());
    }
}
