//! Strictly-typed IPC schema (V1) and its binary codec.
//!
//! # Framing
//!
//! Every message is sent length-delimited: a little-endian `u32` byte-count prefix followed by the
//! `Encode`d body. The framing is identical over Unix domain sockets and Windows named pipes (see
//! [`crate::transport`]). Both ends are the same binary at the same version, so there is no version
//! byte and no forward/backward-compatibility handling.
//!
//! # Schema
//!
//! Connection configuration travels as a binary-encoded [`PropertySet`] inside [`Request::Connect`];
//! everything else is a strictly-typed message. See [`Request`]/[`Response`].

use core::fmt;

use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_size};
use ironrdp_input::MouseButton;
use ironrdp_pdu::impl_pdu_pod;
use ironrdp_propertyset::PropertySet;

use crate::wire::{
    bytes_size, opt_string_size, opt_u16_size, propertyset, read_bool, read_bytes, read_char, read_mouse_button,
    read_opt_string, read_opt_u16, read_string, string_size, write_bool, write_bytes, write_char, write_mouse_button,
    write_opt_string, write_opt_u16, write_string,
};

/// A request sent by the CLI to the daemon.
///
/// `Connect` carries a binary-encoded [`PropertySet`] — never `argv` or CLI strings. Runtime
/// operations are strictly-typed.
#[derive(Clone, PartialEq, Eq)]
pub enum Request {
    /// Start an RDP session from a fully-merged property bag.
    ///
    /// `log_directive`, when set, is a [`tracing`]-style filter directive applied to *this*
    /// session's log capture (e.g. `ironrdp_connector=trace`), layered on top of the default
    /// `DEBUG` level. It lets a caller raise verbosity up-front to troubleshoot a connection.
    Connect {
        properties: PropertySet,
        log_directive: Option<String>,
    },
    /// Tear down the current RDP session (the daemon keeps running).
    Disconnect,
    /// Query the current session status.
    Status,
    /// Query the live session property bag, optionally filtered.
    QueryProps { filter: Option<KeyFilter> },
    /// Return retained log lines, optionally filtered by substring and/or limited to the last `n`.
    QueryLogs {
        substring: Option<String>,
        last: Option<u32>,
    },
    /// Capture the most recent frame (cursor composited in) as a PNG.
    Screenshot,
    /// Move the mouse pointer to an absolute position.
    MouseMove { x: u16, y: u16 },
    /// Press or release a mouse button.
    MouseButton { button: MouseButton, pressed: bool },
    /// Rotate the mouse wheel.
    Wheel { delta: i16, horizontal: bool },
    // TODO: questioning whether we need a way to send multiple keys at once, e.g. a small mini
    // format to express in a single command that keys A and B are pressed while key C is released.
    // This could save LLM tokens by collapsing several round-trips into one request.
    /// Press or release a key identified by its RDP scancode.
    KeyScancode { scancode: u16, pressed: bool },
    /// Press or release a key identified by a Unicode character.
    KeyUnicode { ch: char, pressed: bool },
    /// Resize the remote desktop.
    Resize { width: u16, height: u16 },
    /// Execute a PowerShell command through the negotiated NOW DVC channel.
    PowerShell {
        kind: PowerShellKind,
        command: String,
        no_profile: bool,
        non_interactive: bool,
    },
    /// Report the NOW protocol capabilities negotiated with the remote agent.
    NowCapabilities,
    /// Start one capability-gated NOW execution and stream its events on this IPC connection.
    NowExecute(NowExecutionRequest),
    /// Request normal cancellation of a running NOW execution.
    NowCancel { operation_id: u64 },
    /// List durable NOW operations for the active RDP session.
    NowOperations,
    /// Return a durable NOW operation snapshot.
    NowOperationStatus { operation_id: u64 },
    /// Replay retained output after `after_sequence` and continue until a terminal result.
    NowOperationAttach { operation_id: u64, after_sequence: u64 },
    /// Report local NOW endpoint and operation-management readiness details.
    NowDiagnostics,
    /// Deliver one bounded standard-input fragment to a running NOW operation.
    NowWriteStdin {
        operation_id: u64,
        data: Vec<u8>,
        last: bool,
    },
    // TODO: add clipboard support (CLIPRDR), e.g. requests to read the remote clipboard text and to
    // set it, so an LLM can copy/paste to and from the session.
}

// Manual `Debug` so the `Connect` payload's property *values* (which may include a password before
// it reaches `ConfigBuilder::build`) are never printed verbatim; only the keys are shown.
impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect {
                properties,
                log_directive,
            } => f
                .debug_struct("Connect")
                .field("properties", &PropertyKeys(properties))
                .field("log_directive", log_directive)
                .finish(),
            Self::Disconnect => f.write_str("Disconnect"),
            Self::Status => f.write_str("Status"),
            Self::QueryProps { filter } => f.debug_struct("QueryProps").field("filter", filter).finish(),
            Self::QueryLogs { substring, last } => f
                .debug_struct("QueryLogs")
                .field("substring", substring)
                .field("last", last)
                .finish(),
            Self::Screenshot => f.write_str("Screenshot"),
            Self::MouseMove { x, y } => f.debug_struct("MouseMove").field("x", x).field("y", y).finish(),
            Self::MouseButton { button, pressed } => f
                .debug_struct("MouseButton")
                .field("button", button)
                .field("pressed", pressed)
                .finish(),
            Self::Wheel { delta, horizontal } => f
                .debug_struct("Wheel")
                .field("delta", delta)
                .field("horizontal", horizontal)
                .finish(),
            Self::KeyScancode { scancode, pressed } => f
                .debug_struct("KeyScancode")
                .field("scancode", scancode)
                .field("pressed", pressed)
                .finish(),
            Self::KeyUnicode { ch, pressed } => f
                .debug_struct("KeyUnicode")
                .field("ch", ch)
                .field("pressed", pressed)
                .finish(),
            Self::Resize { width, height } => f
                .debug_struct("Resize")
                .field("width", width)
                .field("height", height)
                .finish(),
            Self::PowerShell {
                kind,
                command,
                no_profile,
                non_interactive,
            } => f
                .debug_struct("PowerShell")
                .field("kind", kind)
                .field("command_len", &command.len())
                .field("no_profile", no_profile)
                .field("non_interactive", non_interactive)
                .finish(),
            Self::NowCapabilities => f.write_str("NowCapabilities"),
            Self::NowExecute(request) => f.debug_tuple("NowExecute").field(request).finish(),
            Self::NowCancel { operation_id } => {
                f.debug_struct("NowCancel").field("operation_id", operation_id).finish()
            }
            Self::NowOperations => f.write_str("NowOperations"),
            Self::NowOperationStatus { operation_id } => f
                .debug_struct("NowOperationStatus")
                .field("operation_id", operation_id)
                .finish(),
            Self::NowOperationAttach {
                operation_id,
                after_sequence,
            } => f
                .debug_struct("NowOperationAttach")
                .field("operation_id", operation_id)
                .field("after_sequence", after_sequence)
                .finish(),
            Self::NowDiagnostics => f.write_str("NowDiagnostics"),
            Self::NowWriteStdin {
                operation_id,
                data,
                last,
            } => f
                .debug_struct("NowWriteStdin")
                .field("operation_id", operation_id)
                .field("data_len", &data.len())
                .field("last", last)
                .finish(),
        }
    }
}

/// The remote PowerShell implementation selected for a NOW execution request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerShellKind {
    /// Windows PowerShell 5 (`powershell.exe`).
    WindowsPowerShell,
    /// PowerShell 7 (`pwsh.exe`).
    PowerShell,
}

/// Execution style selected for a NOW request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NowExecutionKind {
    /// Windows PowerShell 5 (`powershell.exe`).
    WindowsPowerShell,
    /// PowerShell 7 (`pwsh.exe`).
    PowerShell,
    /// A Windows CreateProcess invocation.
    Process,
    /// A command interpreted by the remote host's configured shell.
    Shell,
    /// A Windows batch command.
    Batch,
}

/// Typed arguments shared by streamed NOW execution styles.
#[derive(Clone, PartialEq, Eq)]
pub struct NowExecutionRequest {
    /// The remote execution style.
    pub kind: NowExecutionKind,
    /// Command text or executable filename, depending on `kind`.
    pub command: String,
    /// Process parameters or an optional shell executable.
    pub parameters: Option<String>,
    /// Remote working directory.
    pub directory: Option<String>,
    /// Use `-NoProfile` for PowerShell styles.
    pub no_profile: bool,
    /// Use `-NonInteractive` for PowerShell styles.
    pub non_interactive: bool,
    /// Start without a terminal result or redirected output.
    pub detached: bool,
    /// Cancel the execution after this many seconds.
    pub timeout_secs: Option<u32>,
    /// Bytes to forward to the redirected remote standard input after the operation starts.
    pub stdin: Option<Vec<u8>>,
}

impl fmt::Debug for NowExecutionRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NowExecutionRequest")
            .field("kind", &self.kind)
            .field("command_len", &self.command.len())
            .field("parameters_len", &self.parameters.as_ref().map(String::len))
            .field("directory", &self.directory)
            .field("no_profile", &self.no_profile)
            .field("non_interactive", &self.non_interactive)
            .field("detached", &self.detached)
            .field("timeout_secs", &self.timeout_secs)
            .field("stdin_len", &self.stdin.as_ref().map(Vec::len))
            .finish()
    }
}

/// A redirected NOW execution stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NowStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// Snapshot of the remote NOW protocol capabilities after negotiation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowCapabilities {
    /// Negotiated NOW protocol major version.
    pub major: u16,
    /// Negotiated NOW protocol minor version.
    pub minor: u16,
    /// Server-advertised system capability bitset.
    pub system_capset: u16,
    /// Server-advertised session capability bitset.
    pub session_capset: u16,
    /// Execution capability bitset advertised by the remote NOW server.
    pub server_exec_capset: u16,
    /// Execution capability bitset advertised by this agent during negotiation.
    pub client_exec_capset: u16,
    /// Execution capabilities available to this agent after the server/client intersection.
    pub exec_capset: u16,
    /// Negotiated channel heartbeat interval in seconds, if set.
    pub heartbeat_secs: Option<u32>,
}

/// Lifecycle state of a daemon-owned NOW operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NowOperationState {
    /// The NOW protocol worker is running.
    Running,
    /// A cancellation request has been sent to the NOW protocol worker.
    Cancelling,
    /// The worker completed with a remote exit code.
    Succeeded,
    /// The worker completed with a local or remote protocol error.
    Failed,
    /// The operation was cancelled and the remote worker returned a terminal response.
    Cancelled,
    /// The remote command was intentionally launched without I/O redirection or completion tracking.
    Detached,
}

/// A machine-readable, durable snapshot of one NOW operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowOperationInfo {
    /// Locally assigned operation identity, unique for the daemon lifetime.
    pub operation_id: u64,
    /// Remote execution style.
    pub kind: NowExecutionKind,
    /// Current lifecycle state.
    pub state: NowOperationState,
    /// UNIX epoch millisecond timestamp when the daemon accepted the operation.
    pub started_unix_ms: u64,
    /// UNIX epoch millisecond terminal timestamp, if complete.
    pub finished_unix_ms: Option<u64>,
    /// Remote terminal exit code, if one was received.
    pub exit_code: Option<u32>,
    /// Stable human-readable terminal failure detail, if any.
    pub error: Option<String>,
    /// Total stdout bytes received from NOW.
    pub stdout_bytes: u64,
    /// Total stderr bytes received from NOW.
    pub stderr_bytes: u64,
    /// Output bytes still retained for later replay.
    pub retained_bytes: u64,
    /// Output bytes not retained because the bounded limit was exceeded.
    pub dropped_bytes: u64,
    /// Sequence number assigned to the next output event.
    pub next_sequence: u64,
}

/// Local NOW transport and operation-manager diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowDiagnostics {
    /// The local named-pipe or Unix-socket endpoint injected into the RDP DVC configuration.
    pub endpoint: String,
    /// Active NOW protocol operation, if one currently owns the byte stream.
    pub active_operation_id: Option<u64>,
    /// Bounded retained output size per operation.
    pub output_retention_bytes: u32,
    /// Bounded protocol-to-manager event queue depth.
    pub event_queue_capacity: u16,
    /// First connection readiness deadline after RDP connection.
    pub initial_connect_timeout_secs: u32,
    /// Endpoint reconnect deadline after the first successful connection.
    pub reconnect_timeout_secs: u32,
}

/// Stable category assigned to an agent IPC failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentErrorCode {
    /// Input was invalid or contradicted another requested option.
    InvalidRequest,
    /// A requested session, operation, or resource does not exist.
    NotFound,
    /// The RDP session has not reached a state that permits the requested operation.
    SessionNotReady,
    /// The negotiated NOW capability set does not permit the request.
    CapabilityUnavailable,
    /// A bounded connection or operation deadline elapsed.
    Timeout,
    /// A cancellation request completed or prevented execution.
    Cancelled,
    /// The requested operation conflicts with an existing in-progress operation.
    Conflict,
    /// A local IPC/DVC/NOW transport operation failed.
    Transport,
    /// The failure does not fit a more specific public category.
    Internal,
}

impl AgentErrorCode {
    /// Stable machine-readable spelling used by JSON/NDJSON output.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::NotFound => "not_found",
            Self::SessionNotReady => "session_not_ready",
            Self::CapabilityUnavailable => "capability_unavailable",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::Conflict => "conflict",
            Self::Transport => "transport",
            Self::Internal => "internal",
        }
    }
}

/// Structured daemon error envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentError {
    /// Stable error category.
    pub code: AgentErrorCode,
    /// Human-readable detail.
    pub message: String,
}

impl AgentError {
    /// Classifies the daemon's established error messages without changing its text contract.
    pub fn from_message(message: impl Into<String>) -> Self {
        let message = message.into();
        let code = if message.contains("not found") {
            AgentErrorCode::NotFound
        } else if message.contains("connected RDP") || message.contains("no active session") {
            AgentErrorCode::SessionNotReady
        } else if message.contains("does not support") || message.contains("capability") {
            AgentErrorCode::CapabilityUnavailable
        } else if message.contains("timeout") {
            AgentErrorCode::Timeout
        } else if message.contains("cancel") {
            AgentErrorCode::Cancelled
        } else if message.contains("already") || message.contains("in progress") || message.contains("backpressured") {
            AgentErrorCode::Conflict
        } else if message.contains("endpoint") || message.contains("pipe") || message.contains("transport") {
            AgentErrorCode::Transport
        } else if message.contains("invalid") || message.contains("must") || message.contains("provide") {
            AgentErrorCode::InvalidRequest
        } else {
            AgentErrorCode::Internal
        };
        Self { code, message }
    }
}

/// A [`PropertySet`] whose `Debug` output lists only the keys, never the (possibly secret) values.
struct PropertyKeys<'a>(&'a PropertySet);

impl fmt::Debug for PropertyKeys<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.0.iter().map(|(key, _)| key)).finish()
    }
}

/// The daemon's reply to a [`Request`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// Success, carrying an operation-specific [`Payload`].
    Ok(Payload),
    /// Failure. The message is lowercase with no trailing punctuation.
    Err(AgentError),
}

impl Response {
    /// A successful response with no payload.
    pub fn ok() -> Self {
        Self::Ok(Payload::Empty)
    }

    /// A failure response.
    pub fn error(message: impl Into<String>) -> Self {
        Self::Err(AgentError::from_message(message))
    }

    /// Whether this is a success response.
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok(_))
    }
}

/// The success payload carried by [`Response::Ok`].
#[derive(Clone, PartialEq, Eq)]
pub enum Payload {
    /// No data.
    Empty,
    /// Current session status.
    Status(StatusInfo),
    /// A dump of the live property bag.
    Properties(PropertyDump),
    /// Retained log lines.
    Logs(Vec<String>),
    /// The most recent frame encoded as a PNG (cursor included), with its dimensions.
    Screenshot { width: u16, height: u16, png: Vec<u8> },
    /// The byte streams and exit status returned by a NOW PowerShell invocation.
    PowerShell {
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        exit_code: u32,
    },
    /// NOW capability snapshot.
    NowCapabilities(NowCapabilities),
    /// A streamed execution has been accepted with this local operation ID.
    NowExecutionStarted { operation_id: u64 },
    /// A raw stdout or stderr chunk emitted by a streamed execution.
    NowExecutionData {
        operation_id: u64,
        /// Monotonically increasing operation-local event sequence for replay/reattachment.
        sequence: u64,
        stream: NowStream,
        data: Vec<u8>,
    },
    /// Terminal result for a streamed execution.
    NowExecutionResult { operation_id: u64, exit_code: u32 },
    /// Cancellation was requested for a streamed execution.
    NowCancelAccepted { operation_id: u64 },
    /// Snapshot of one durable NOW operation.
    NowOperationInfo(NowOperationInfo),
    /// Snapshots of all durable NOW operations.
    NowOperations(Vec<NowOperationInfo>),
    /// Local NOW transport and operation-manager diagnostics.
    NowDiagnostics(NowDiagnostics),
    /// Standard-input fragment accepted by a running NOW operation.
    NowStdinAccepted { operation_id: u64, last: bool },
}

impl fmt::Debug for Payload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("Empty"),
            Self::Status(status) => f.debug_tuple("Status").field(status).finish(),
            Self::Properties(dump) => f.debug_tuple("Properties").field(dump).finish(),
            Self::Logs(lines) => f.debug_tuple("Logs").field(lines).finish(),
            // Print the PNG byte length rather than the (large, binary) blob.
            Self::Screenshot { width, height, png } => f
                .debug_struct("Screenshot")
                .field("width", width)
                .field("height", height)
                .field("png_len", &png.len())
                .finish(),
            Self::PowerShell {
                stdout,
                stderr,
                exit_code,
            } => f
                .debug_struct("PowerShell")
                .field("stdout_len", &stdout.len())
                .field("stderr_len", &stderr.len())
                .field("exit_code", exit_code)
                .finish(),
            Self::NowCapabilities(capabilities) => f.debug_tuple("NowCapabilities").field(capabilities).finish(),
            Self::NowExecutionStarted { operation_id } => f
                .debug_struct("NowExecutionStarted")
                .field("operation_id", operation_id)
                .finish(),
            Self::NowExecutionData {
                operation_id,
                sequence,
                stream,
                data,
            } => f
                .debug_struct("NowExecutionData")
                .field("operation_id", operation_id)
                .field("sequence", sequence)
                .field("stream", stream)
                .field("data_len", &data.len())
                .finish(),
            Self::NowExecutionResult {
                operation_id,
                exit_code,
            } => f
                .debug_struct("NowExecutionResult")
                .field("operation_id", operation_id)
                .field("exit_code", exit_code)
                .finish(),
            Self::NowCancelAccepted { operation_id } => f
                .debug_struct("NowCancelAccepted")
                .field("operation_id", operation_id)
                .finish(),
            Self::NowOperationInfo(info) => f.debug_tuple("NowOperationInfo").field(info).finish(),
            Self::NowOperations(operations) => f.debug_tuple("NowOperations").field(operations).finish(),
            Self::NowDiagnostics(diagnostics) => f.debug_tuple("NowDiagnostics").field(diagnostics).finish(),
            Self::NowStdinAccepted { operation_id, last } => f
                .debug_struct("NowStdinAccepted")
                .field("operation_id", operation_id)
                .field("last", last)
                .finish(),
        }
    }
}

/// Coarse connection state reported by [`Request::Status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    /// No session has been started.
    NoSession,
    /// A session was started and is connecting.
    Connecting,
    /// A session is active (at least one frame received).
    Connected,
    /// A graceful disconnect was requested; the engine thread is still shutting down.
    Disconnecting,
    /// A session terminated gracefully.
    Disconnected,
    /// A session failed.
    Failed,
}

impl ConnState {
    fn tag(self) -> u8 {
        match self {
            Self::NoSession => 0,
            Self::Connecting => 1,
            Self::Connected => 2,
            Self::Disconnected => 3,
            Self::Failed => 4,
            Self::Disconnecting => 5,
        }
    }

    fn from_tag(tag: u8) -> DecodeResult<Self> {
        match tag {
            0 => Ok(Self::NoSession),
            1 => Ok(Self::Connecting),
            2 => Ok(Self::Connected),
            3 => Ok(Self::Disconnected),
            4 => Ok(Self::Failed),
            5 => Ok(Self::Disconnecting),
            _ => Err(ironrdp_core::invalid_field_err!("connection state", "unknown tag")),
        }
    }
}

/// Status snapshot returned by [`Request::Status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusInfo {
    /// Coarse connection state.
    pub state: ConnState,
    /// RDP target (`host:port`), if a session exists.
    pub destination: Option<String>,
    /// Most recent frame width, if any.
    pub width: Option<u16>,
    /// Most recent frame height, if any.
    pub height: Option<u16>,
    /// Human-readable detail, e.g. the failure reason.
    pub message: Option<String>,
    /// `true` when the daemon was started with preloaded credentials (an operator-provided overlay).
    ///
    /// When set, a caller driving `connect` does not need to supply a password (or other secrets):
    /// the daemon layers the overlay on top of the request before building the configuration.
    pub credentials_loaded: bool,
}

/// A bulk dump of live properties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyDump {
    /// One entry per property, in key order.
    pub entries: Vec<PropertyEntry>,
}

/// A single dumped property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyEntry {
    /// Property key.
    pub key: String,
    /// Property value.
    pub value: PropValue,
}

/// A dumped property value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropValue {
    /// Integer value.
    Int(i64),
    /// String value.
    Str(String),
}

/// A small key filter for [`Request::QueryProps`]. Matching is case-insensitive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyFilter {
    /// Match keys containing this substring.
    Substring(String),
    /// Match keys starting with this prefix.
    Prefix(String),
}

impl KeyFilter {
    /// Returns `true` when `key` matches this filter (case-insensitive).
    pub fn matches(&self, key: &str) -> bool {
        let key = key.to_ascii_lowercase();
        match self {
            Self::Substring(needle) => key.contains(&needle.to_ascii_lowercase()),
            Self::Prefix(prefix) => key.starts_with(&prefix.to_ascii_lowercase()),
        }
    }
}

// ── KeyFilter codec ─────────────────────────────────────────────────────────

impl Encode for KeyFilter {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::Substring(value) => {
                dst.write_u8(0);
                write_string(dst, value)
            }
            Self::Prefix(value) => {
                dst.write_u8(1);
                write_string(dst, value)
            }
        }
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::KeyFilter"
    }

    fn size(&self) -> usize {
        let value = match self {
            Self::Substring(value) | Self::Prefix(value) => value,
        };
        1 /* tag */ + string_size(value)
    }
}

impl Decode<'_> for KeyFilter {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => Ok(Self::Substring(read_string(src)?)),
            1 => Ok(Self::Prefix(read_string(src)?)),
            _ => Err(ironrdp_core::invalid_field_err!("key filter", "unknown tag")),
        }
    }
}

impl_pdu_pod!(KeyFilter);

// ── PowerShellKind codec ─────────────────────────────────────────────────────

impl Encode for PowerShellKind {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u8(match self {
            Self::WindowsPowerShell => 0,
            Self::PowerShell => 1,
        });
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::PowerShellKind"
    }

    fn size(&self) -> usize {
        1
    }
}

impl Decode<'_> for PowerShellKind {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => Ok(Self::WindowsPowerShell),
            1 => Ok(Self::PowerShell),
            _ => Err(ironrdp_core::invalid_field_err!("PowerShell kind", "unknown tag")),
        }
    }
}

impl_pdu_pod!(PowerShellKind);

// ── NOW streamed execution codec ────────────────────────────────────────────

impl Encode for NowExecutionKind {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u8(match self {
            Self::WindowsPowerShell => 0,
            Self::PowerShell => 1,
            Self::Process => 2,
            Self::Shell => 3,
            Self::Batch => 4,
        });
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::NowExecutionKind"
    }

    fn size(&self) -> usize {
        1
    }
}

impl Decode<'_> for NowExecutionKind {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => Ok(Self::WindowsPowerShell),
            1 => Ok(Self::PowerShell),
            2 => Ok(Self::Process),
            3 => Ok(Self::Shell),
            4 => Ok(Self::Batch),
            _ => Err(ironrdp_core::invalid_field_err!("NOW execution kind", "unknown tag")),
        }
    }
}

impl_pdu_pod!(NowExecutionKind);

impl Encode for NowStream {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u8(match self {
            Self::Stdout => 0,
            Self::Stderr => 1,
        });
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::NowStream"
    }

    fn size(&self) -> usize {
        1
    }
}

impl Decode<'_> for NowStream {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => Ok(Self::Stdout),
            1 => Ok(Self::Stderr),
            _ => Err(ironrdp_core::invalid_field_err!("NOW stream", "unknown tag")),
        }
    }
}

impl_pdu_pod!(NowStream);

impl Encode for NowExecutionRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.kind.encode(dst)?;
        write_string(dst, &self.command)?;
        write_opt_string(dst, self.parameters.as_deref())?;
        write_opt_string(dst, self.directory.as_deref())?;
        write_bool(dst, self.no_profile)?;
        write_bool(dst, self.non_interactive)?;
        write_bool(dst, self.detached)?;
        match self.timeout_secs {
            Some(timeout_secs) => {
                dst.write_u8(1);
                dst.write_u32(timeout_secs);
            }
            None => dst.write_u8(0),
        }
        match &self.stdin {
            Some(stdin) => {
                dst.write_u8(1);
                write_bytes(dst, stdin)?;
            }
            None => dst.write_u8(0),
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::NowExecutionRequest"
    }

    fn size(&self) -> usize {
        self.kind.size()
            + string_size(&self.command)
            + opt_string_size(self.parameters.as_deref())
            + opt_string_size(self.directory.as_deref())
            + 1 /* no_profile */
            + 1 /* non_interactive */
            + 1 /* detached */
            + 1 /* timeout presence */
            + self.timeout_secs.map_or(0, |_| 4)
            + 1 /* stdin presence */
            + self.stdin.as_ref().map_or(0, |stdin| bytes_size(stdin))
    }
}

impl Decode<'_> for NowExecutionRequest {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let kind = NowExecutionKind::decode(src)?;
        let command = read_string(src)?;
        let parameters = read_opt_string(src)?;
        let directory = read_opt_string(src)?;
        let no_profile = read_bool(src)?;
        let non_interactive = read_bool(src)?;
        let detached = read_bool(src)?;
        ensure_size!(in: src, size: 1);
        let timeout_secs = match src.read_u8() {
            0 => None,
            1 => {
                ensure_size!(in: src, size: 4);
                Some(src.read_u32())
            }
            _ => return Err(ironrdp_core::invalid_field_err!("NOW timeout", "invalid presence flag")),
        };
        ensure_size!(in: src, size: 1);
        let stdin = match src.read_u8() {
            0 => None,
            1 => Some(read_bytes(src)?),
            _ => return Err(ironrdp_core::invalid_field_err!("NOW stdin", "invalid presence flag")),
        };
        Ok(Self {
            kind,
            command,
            parameters,
            directory,
            no_profile,
            non_interactive,
            detached,
            timeout_secs,
            stdin,
        })
    }
}

impl_pdu_pod!(NowExecutionRequest);

impl Encode for NowCapabilities {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u16(self.major);
        dst.write_u16(self.minor);
        dst.write_u16(self.system_capset);
        dst.write_u16(self.session_capset);
        dst.write_u16(self.server_exec_capset);
        dst.write_u16(self.client_exec_capset);
        dst.write_u16(self.exec_capset);
        match self.heartbeat_secs {
            Some(heartbeat_secs) => {
                dst.write_u8(1);
                dst.write_u32(heartbeat_secs);
            }
            None => dst.write_u8(0),
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::NowCapabilities"
    }

    fn size(&self) -> usize {
        2 /* major */
            + 2 /* minor */
            + 2 /* system_capset */
            + 2 /* session_capset */
            + 2 /* server_exec_capset */
            + 2 /* client_exec_capset */
            + 2 /* exec_capset */
            + 1 /* heartbeat presence */
            + self.heartbeat_secs.map_or(0, |_| 4)
    }
}

impl Decode<'_> for NowCapabilities {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 14);
        let major = src.read_u16();
        let minor = src.read_u16();
        let system_capset = src.read_u16();
        let session_capset = src.read_u16();
        let server_exec_capset = src.read_u16();
        let client_exec_capset = src.read_u16();
        let exec_capset = src.read_u16();
        ensure_size!(in: src, size: 1);
        let heartbeat_secs = match src.read_u8() {
            0 => None,
            1 => {
                ensure_size!(in: src, size: 4);
                Some(src.read_u32())
            }
            _ => {
                return Err(ironrdp_core::invalid_field_err!(
                    "NOW heartbeat",
                    "invalid presence flag"
                ));
            }
        };
        Ok(Self {
            major,
            minor,
            system_capset,
            session_capset,
            server_exec_capset,
            client_exec_capset,
            exec_capset,
            heartbeat_secs,
        })
    }
}

impl_pdu_pod!(NowCapabilities);

impl Encode for NowOperationState {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u8(match self {
            Self::Running => 0,
            Self::Cancelling => 1,
            Self::Succeeded => 2,
            Self::Failed => 3,
            Self::Cancelled => 4,
            Self::Detached => 5,
        });
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::NowOperationState"
    }

    fn size(&self) -> usize {
        1
    }
}

impl Decode<'_> for NowOperationState {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => Ok(Self::Running),
            1 => Ok(Self::Cancelling),
            2 => Ok(Self::Succeeded),
            3 => Ok(Self::Failed),
            4 => Ok(Self::Cancelled),
            5 => Ok(Self::Detached),
            _ => Err(ironrdp_core::invalid_field_err!("NOW operation state", "unknown tag")),
        }
    }
}

impl_pdu_pod!(NowOperationState);

impl Encode for NowOperationInfo {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u64(self.operation_id);
        self.kind.encode(dst)?;
        self.state.encode(dst)?;
        dst.write_u64(self.started_unix_ms);
        match self.finished_unix_ms {
            Some(value) => {
                dst.write_u8(1);
                dst.write_u64(value);
            }
            None => dst.write_u8(0),
        }
        match self.exit_code {
            Some(value) => {
                dst.write_u8(1);
                dst.write_u32(value);
            }
            None => dst.write_u8(0),
        }
        write_opt_string(dst, self.error.as_deref())?;
        dst.write_u64(self.stdout_bytes);
        dst.write_u64(self.stderr_bytes);
        dst.write_u64(self.retained_bytes);
        dst.write_u64(self.dropped_bytes);
        dst.write_u64(self.next_sequence);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::NowOperationInfo"
    }

    fn size(&self) -> usize {
        8 /* operation_id */
            + self.kind.size()
            + self.state.size()
            + 8 /* started_unix_ms */
            + 1 /* finished presence */
            + self.finished_unix_ms.map_or(0, |_| 8)
            + 1 /* exit presence */
            + self.exit_code.map_or(0, |_| 4)
            + opt_string_size(self.error.as_deref())
            + 8 /* stdout_bytes */
            + 8 /* stderr_bytes */
            + 8 /* retained_bytes */
            + 8 /* dropped_bytes */
            + 8 /* next_sequence */
    }
}

impl Decode<'_> for NowOperationInfo {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 18);
        let operation_id = src.read_u64();
        let kind = NowExecutionKind::decode(src)?;
        let state = NowOperationState::decode(src)?;
        let started_unix_ms = src.read_u64();
        ensure_size!(in: src, size: 1);
        let finished_unix_ms = match src.read_u8() {
            0 => None,
            1 => {
                ensure_size!(in: src, size: 8);
                Some(src.read_u64())
            }
            _ => {
                return Err(ironrdp_core::invalid_field_err!(
                    "NOW operation finish",
                    "invalid presence flag"
                ));
            }
        };
        ensure_size!(in: src, size: 1);
        let exit_code = match src.read_u8() {
            0 => None,
            1 => {
                ensure_size!(in: src, size: 4);
                Some(src.read_u32())
            }
            _ => {
                return Err(ironrdp_core::invalid_field_err!(
                    "NOW operation exit",
                    "invalid presence flag"
                ));
            }
        };
        let error = read_opt_string(src)?;
        ensure_size!(in: src, size: 40);
        Ok(Self {
            operation_id,
            kind,
            state,
            started_unix_ms,
            finished_unix_ms,
            exit_code,
            error,
            stdout_bytes: src.read_u64(),
            stderr_bytes: src.read_u64(),
            retained_bytes: src.read_u64(),
            dropped_bytes: src.read_u64(),
            next_sequence: src.read_u64(),
        })
    }
}

impl_pdu_pod!(NowOperationInfo);

impl Encode for NowDiagnostics {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        write_string(dst, &self.endpoint)?;
        match self.active_operation_id {
            Some(value) => {
                dst.write_u8(1);
                dst.write_u64(value);
            }
            None => dst.write_u8(0),
        }
        dst.write_u32(self.output_retention_bytes);
        dst.write_u16(self.event_queue_capacity);
        dst.write_u32(self.initial_connect_timeout_secs);
        dst.write_u32(self.reconnect_timeout_secs);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::NowDiagnostics"
    }

    fn size(&self) -> usize {
        string_size(&self.endpoint)
            + 1 /* active operation presence */
            + self.active_operation_id.map_or(0, |_| 8)
            + 4 /* output_retention_bytes */
            + 2 /* event_queue_capacity */
            + 4 /* initial_connect_timeout_secs */
            + 4 /* reconnect_timeout_secs */
    }
}

impl Decode<'_> for NowDiagnostics {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let endpoint = read_string(src)?;
        ensure_size!(in: src, size: 1);
        let active_operation_id = match src.read_u8() {
            0 => None,
            1 => {
                ensure_size!(in: src, size: 8);
                Some(src.read_u64())
            }
            _ => {
                return Err(ironrdp_core::invalid_field_err!(
                    "NOW active operation",
                    "invalid presence flag"
                ));
            }
        };
        ensure_size!(in: src, size: 14);
        Ok(Self {
            endpoint,
            active_operation_id,
            output_retention_bytes: src.read_u32(),
            event_queue_capacity: src.read_u16(),
            initial_connect_timeout_secs: src.read_u32(),
            reconnect_timeout_secs: src.read_u32(),
        })
    }
}

impl_pdu_pod!(NowDiagnostics);

impl Encode for AgentErrorCode {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u8(match self {
            Self::InvalidRequest => 0,
            Self::NotFound => 1,
            Self::SessionNotReady => 2,
            Self::CapabilityUnavailable => 3,
            Self::Timeout => 4,
            Self::Cancelled => 5,
            Self::Conflict => 6,
            Self::Transport => 7,
            Self::Internal => 8,
        });
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::AgentErrorCode"
    }

    fn size(&self) -> usize {
        1
    }
}

impl Decode<'_> for AgentErrorCode {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => Ok(Self::InvalidRequest),
            1 => Ok(Self::NotFound),
            2 => Ok(Self::SessionNotReady),
            3 => Ok(Self::CapabilityUnavailable),
            4 => Ok(Self::Timeout),
            5 => Ok(Self::Cancelled),
            6 => Ok(Self::Conflict),
            7 => Ok(Self::Transport),
            8 => Ok(Self::Internal),
            _ => Err(ironrdp_core::invalid_field_err!("agent error code", "unknown tag")),
        }
    }
}

impl_pdu_pod!(AgentErrorCode);

impl Encode for AgentError {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.code.encode(dst)?;
        write_string(dst, &self.message)
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::AgentError"
    }

    fn size(&self) -> usize {
        self.code.size() + string_size(&self.message)
    }
}

impl Decode<'_> for AgentError {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(Self {
            code: AgentErrorCode::decode(src)?,
            message: read_string(src)?,
        })
    }
}

impl_pdu_pod!(AgentError);

// ── PropValue / PropertyEntry / PropertyDump codec ──────────────────────────

impl Encode for PropValue {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::Int(value) => {
                dst.write_u8(0);
                dst.write_i64(*value);
            }
            Self::Str(value) => {
                dst.write_u8(1);
                write_string(dst, value)?;
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::PropValue"
    }

    fn size(&self) -> usize {
        1 /* tag */
            + match self {
                Self::Int(_) => 8,
                Self::Str(value) => string_size(value),
            }
    }
}

impl Decode<'_> for PropValue {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => {
                ensure_size!(in: src, size: 8);
                Ok(Self::Int(src.read_i64()))
            }
            1 => Ok(Self::Str(read_string(src)?)),
            _ => Err(ironrdp_core::invalid_field_err!("property value", "unknown tag")),
        }
    }
}

impl_pdu_pod!(PropValue);

impl Encode for PropertyEntry {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        write_string(dst, &self.key)?;
        self.value.encode(dst)
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::PropertyEntry"
    }

    fn size(&self) -> usize {
        string_size(&self.key) + self.value.size()
    }
}

impl Decode<'_> for PropertyEntry {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let key = read_string(src)?;
        let value = PropValue::decode(src)?;
        Ok(Self { key, value })
    }
}

impl_pdu_pod!(PropertyEntry);

impl Encode for PropertyDump {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        let count: u32 = cast_length!("property count", self.entries.len())?;
        dst.write_u32(count);
        for entry in &self.entries {
            entry.encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::PropertyDump"
    }

    fn size(&self) -> usize {
        4 /* count */ + self.entries.iter().map(Encode::size).sum::<usize>()
    }
}

impl Decode<'_> for PropertyDump {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4);
        let count = src.read_u32();
        let mut entries = Vec::new();
        for _ in 0..count {
            entries.push(PropertyEntry::decode(src)?);
        }
        Ok(Self { entries })
    }
}

impl_pdu_pod!(PropertyDump);

// ── StatusInfo codec ────────────────────────────────────────────────────────

impl Encode for StatusInfo {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u8(self.state.tag());
        write_opt_string(dst, self.destination.as_deref())?;
        write_opt_u16(dst, self.width)?;
        write_opt_u16(dst, self.height)?;
        write_opt_string(dst, self.message.as_deref())?;
        write_bool(dst, self.credentials_loaded)
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::StatusInfo"
    }

    fn size(&self) -> usize {
        1 /* state */
            + opt_string_size(self.destination.as_deref())
            + opt_u16_size(self.width)
            + opt_u16_size(self.height)
            + opt_string_size(self.message.as_deref())
            + 1 /* credentials_loaded */
    }
}

impl Decode<'_> for StatusInfo {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        let state = ConnState::from_tag(src.read_u8())?;
        let destination = read_opt_string(src)?;
        let width = read_opt_u16(src)?;
        let height = read_opt_u16(src)?;
        let message = read_opt_string(src)?;
        let credentials_loaded = read_bool(src)?;
        Ok(Self {
            state,
            destination,
            width,
            height,
            message,
            credentials_loaded,
        })
    }
}

impl_pdu_pod!(StatusInfo);

// ── Payload codec ───────────────────────────────────────────────────────────

impl Encode for Payload {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::Empty => dst.write_u8(0),
            Self::Status(status) => {
                dst.write_u8(1);
                status.encode(dst)?;
            }
            Self::Properties(dump) => {
                dst.write_u8(2);
                dump.encode(dst)?;
            }
            Self::Logs(lines) => {
                dst.write_u8(3);
                let count: u32 = cast_length!("log line count", lines.len())?;
                dst.write_u32(count);
                for line in lines {
                    write_string(dst, line)?;
                }
            }
            Self::Screenshot { width, height, png } => {
                dst.write_u8(4);
                dst.write_u16(*width);
                dst.write_u16(*height);
                write_bytes(dst, png)?;
            }
            Self::PowerShell {
                stdout,
                stderr,
                exit_code,
            } => {
                dst.write_u8(5);
                write_bytes(dst, stdout)?;
                write_bytes(dst, stderr)?;
                dst.write_u32(*exit_code);
            }
            Self::NowCapabilities(capabilities) => {
                dst.write_u8(6);
                capabilities.encode(dst)?;
            }
            Self::NowExecutionStarted { operation_id } => {
                dst.write_u8(7);
                dst.write_u64(*operation_id);
            }
            Self::NowExecutionData {
                operation_id,
                sequence,
                stream,
                data,
            } => {
                dst.write_u8(8);
                dst.write_u64(*operation_id);
                dst.write_u64(*sequence);
                stream.encode(dst)?;
                write_bytes(dst, data)?;
            }
            Self::NowExecutionResult {
                operation_id,
                exit_code,
            } => {
                dst.write_u8(9);
                dst.write_u64(*operation_id);
                dst.write_u32(*exit_code);
            }
            Self::NowCancelAccepted { operation_id } => {
                dst.write_u8(10);
                dst.write_u64(*operation_id);
            }
            Self::NowOperationInfo(info) => {
                dst.write_u8(11);
                info.encode(dst)?;
            }
            Self::NowOperations(operations) => {
                dst.write_u8(12);
                let count: u32 = cast_length!("NOW operation count", operations.len())?;
                dst.write_u32(count);
                for operation in operations {
                    operation.encode(dst)?;
                }
            }
            Self::NowDiagnostics(diagnostics) => {
                dst.write_u8(13);
                diagnostics.encode(dst)?;
            }
            Self::NowStdinAccepted { operation_id, last } => {
                dst.write_u8(14);
                dst.write_u64(*operation_id);
                write_bool(dst, *last)?;
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::Payload"
    }

    fn size(&self) -> usize {
        1 /* tag */
            + match self {
                Self::Empty => 0,
                Self::Status(status) => status.size(),
                Self::Properties(dump) => dump.size(),
                Self::Logs(lines) => 4 + lines.iter().map(|line| string_size(line)).sum::<usize>(),
                Self::Screenshot { png, .. } => 2 /* width */ + 2 /* height */ + bytes_size(png),
                Self::PowerShell { stdout, stderr, .. } => {
                    bytes_size(stdout) + bytes_size(stderr) + 4 /* exit_code */
                }
                Self::NowCapabilities(capabilities) => capabilities.size(),
                Self::NowExecutionStarted { .. } | Self::NowCancelAccepted { .. } => 8 /* operation_id */,
                Self::NowExecutionData { stream, data, .. } => {
                    8 /* operation_id */ + 8 /* sequence */ + stream.size() + bytes_size(data)
                }
                Self::NowExecutionResult { .. } => 8 /* operation_id */ + 4 /* exit_code */,
                Self::NowOperationInfo(info) => info.size(),
                Self::NowOperations(operations) => 4 + operations.iter().map(Encode::size).sum::<usize>(),
                Self::NowDiagnostics(diagnostics) => diagnostics.size(),
                Self::NowStdinAccepted { .. } => 8 /* operation_id */ + 1 /* last */,
            }
    }
}

impl Decode<'_> for Payload {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => Ok(Self::Empty),
            1 => Ok(Self::Status(StatusInfo::decode(src)?)),
            2 => Ok(Self::Properties(PropertyDump::decode(src)?)),
            3 => {
                ensure_size!(in: src, size: 4);
                let count = src.read_u32();
                let mut lines = Vec::new();
                for _ in 0..count {
                    lines.push(read_string(src)?);
                }
                Ok(Self::Logs(lines))
            }
            4 => {
                ensure_size!(in: src, size: 4);
                let width = src.read_u16();
                let height = src.read_u16();
                let png = read_bytes(src)?;
                Ok(Self::Screenshot { width, height, png })
            }
            5 => {
                let stdout = read_bytes(src)?;
                let stderr = read_bytes(src)?;
                ensure_size!(in: src, size: 4);
                let exit_code = src.read_u32();
                Ok(Self::PowerShell {
                    stdout,
                    stderr,
                    exit_code,
                })
            }
            6 => Ok(Self::NowCapabilities(NowCapabilities::decode(src)?)),
            7 => {
                ensure_size!(in: src, size: 8);
                Ok(Self::NowExecutionStarted {
                    operation_id: src.read_u64(),
                })
            }
            8 => {
                ensure_size!(in: src, size: 16);
                let operation_id = src.read_u64();
                let sequence = src.read_u64();
                let stream = NowStream::decode(src)?;
                let data = read_bytes(src)?;
                Ok(Self::NowExecutionData {
                    operation_id,
                    sequence,
                    stream,
                    data,
                })
            }
            9 => {
                ensure_size!(in: src, size: 12);
                Ok(Self::NowExecutionResult {
                    operation_id: src.read_u64(),
                    exit_code: src.read_u32(),
                })
            }
            10 => {
                ensure_size!(in: src, size: 8);
                Ok(Self::NowCancelAccepted {
                    operation_id: src.read_u64(),
                })
            }
            11 => Ok(Self::NowOperationInfo(NowOperationInfo::decode(src)?)),
            12 => {
                ensure_size!(in: src, size: 4);
                let count = src.read_u32();
                let mut operations = Vec::new();
                for _ in 0..count {
                    operations.push(NowOperationInfo::decode(src)?);
                }
                Ok(Self::NowOperations(operations))
            }
            13 => Ok(Self::NowDiagnostics(NowDiagnostics::decode(src)?)),
            14 => {
                ensure_size!(in: src, size: 8);
                Ok(Self::NowStdinAccepted {
                    operation_id: src.read_u64(),
                    last: read_bool(src)?,
                })
            }
            _ => Err(ironrdp_core::invalid_field_err!("payload", "unknown tag")),
        }
    }
}

impl_pdu_pod!(Payload);

// ── Response codec ──────────────────────────────────────────────────────────

impl Encode for Response {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::Ok(payload) => {
                dst.write_u8(0);
                payload.encode(dst)
            }
            Self::Err(error) => {
                dst.write_u8(1);
                error.encode(dst)
            }
        }
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::Response"
    }

    fn size(&self) -> usize {
        1 /* tag */
            + match self {
                Self::Ok(payload) => payload.size(),
                Self::Err(error) => error.size(),
            }
    }
}

impl Decode<'_> for Response {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => Ok(Self::Ok(Payload::decode(src)?)),
            1 => Ok(Self::Err(AgentError::decode(src)?)),
            _ => Err(ironrdp_core::invalid_field_err!("response", "unknown tag")),
        }
    }
}

impl_pdu_pod!(Response);

// ── Request codec ───────────────────────────────────────────────────────────

impl Encode for Request {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::Connect {
                properties,
                log_directive,
            } => {
                dst.write_u8(0);
                propertyset::write(properties, dst)?;
                write_opt_string(dst, log_directive.as_deref())?;
            }
            Self::Disconnect => dst.write_u8(1),
            Self::Status => dst.write_u8(2),
            Self::QueryProps { filter } => {
                dst.write_u8(3);
                match filter {
                    Some(filter) => {
                        dst.write_u8(1);
                        filter.encode(dst)?;
                    }
                    None => dst.write_u8(0),
                }
            }
            Self::QueryLogs { substring, last } => {
                dst.write_u8(4);
                write_opt_string(dst, substring.as_deref())?;
                match last {
                    Some(last) => {
                        dst.write_u8(1);
                        dst.write_u32(*last);
                    }
                    None => dst.write_u8(0),
                }
            }
            Self::Screenshot => dst.write_u8(5),
            Self::MouseMove { x, y } => {
                dst.write_u8(6);
                dst.write_u16(*x);
                dst.write_u16(*y);
            }
            Self::MouseButton { button, pressed } => {
                dst.write_u8(7);
                write_mouse_button(dst, *button)?;
                write_bool(dst, *pressed)?;
            }
            Self::Wheel { delta, horizontal } => {
                dst.write_u8(8);
                dst.write_i16(*delta);
                write_bool(dst, *horizontal)?;
            }
            Self::KeyScancode { scancode, pressed } => {
                dst.write_u8(9);
                dst.write_u16(*scancode);
                write_bool(dst, *pressed)?;
            }
            Self::KeyUnicode { ch, pressed } => {
                dst.write_u8(10);
                write_char(dst, *ch)?;
                write_bool(dst, *pressed)?;
            }
            Self::Resize { width, height } => {
                dst.write_u8(11);
                dst.write_u16(*width);
                dst.write_u16(*height);
            }
            Self::PowerShell {
                kind,
                command,
                no_profile,
                non_interactive,
            } => {
                dst.write_u8(12);
                kind.encode(dst)?;
                write_string(dst, command)?;
                write_bool(dst, *no_profile)?;
                write_bool(dst, *non_interactive)?;
            }
            Self::NowCapabilities => dst.write_u8(13),
            Self::NowExecute(request) => {
                dst.write_u8(14);
                request.encode(dst)?;
            }
            Self::NowCancel { operation_id } => {
                dst.write_u8(15);
                dst.write_u64(*operation_id);
            }
            Self::NowOperations => dst.write_u8(16),
            Self::NowOperationStatus { operation_id } => {
                dst.write_u8(17);
                dst.write_u64(*operation_id);
            }
            Self::NowOperationAttach {
                operation_id,
                after_sequence,
            } => {
                dst.write_u8(18);
                dst.write_u64(*operation_id);
                dst.write_u64(*after_sequence);
            }
            Self::NowDiagnostics => dst.write_u8(19),
            Self::NowWriteStdin {
                operation_id,
                data,
                last,
            } => {
                dst.write_u8(20);
                dst.write_u64(*operation_id);
                write_bytes(dst, data)?;
                write_bool(dst, *last)?;
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::Request"
    }

    fn size(&self) -> usize {
        1 /* tag */
            + match self {
                Self::Connect { properties, log_directive } => {
                    propertyset::size(properties) + opt_string_size(log_directive.as_deref())
                }
                Self::Disconnect | Self::Status | Self::Screenshot => 0,
                Self::QueryProps { filter } => 1 /* presence */ + filter.as_ref().map_or(0, Encode::size),
                Self::QueryLogs { substring, last } => {
                    opt_string_size(substring.as_deref()) + 1 /* presence */ + last.map_or(0, |_| 4)
                }
                Self::MouseMove { .. } => 2 /* x */ + 2 /* y */,
                Self::MouseButton { .. } => 1 /* button */ + 1 /* pressed */,
                Self::Wheel { .. } => 2 /* delta */ + 1 /* horizontal */,
                Self::KeyScancode { .. } => 2 /* scancode */ + 1 /* pressed */,
                Self::KeyUnicode { .. } => 4 /* ch */ + 1 /* pressed */,
                Self::Resize { .. } => 2 /* width */ + 2 /* height */,
                Self::PowerShell {
                    kind,
                    command,
                    no_profile: _,
                    non_interactive: _,
                } => kind.size() + string_size(command) + 1 /* no_profile */ + 1 /* non_interactive */,
                Self::NowCapabilities => 0,
                Self::NowExecute(request) => request.size(),
                Self::NowCancel { .. } => 8 /* operation_id */,
                Self::NowOperations | Self::NowDiagnostics => 0,
                Self::NowOperationStatus { .. } => 8 /* operation_id */,
                Self::NowOperationAttach { .. } => 8 /* operation_id */ + 8 /* after_sequence */,
                Self::NowWriteStdin { data, .. } => 8 /* operation_id */ + bytes_size(data) + 1 /* last */,
            }
    }
}

impl Decode<'_> for Request {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => {
                let mut properties = PropertySet::new();
                propertyset::read(&mut properties, src)?;
                let log_directive = read_opt_string(src)?;
                Ok(Self::Connect {
                    properties,
                    log_directive,
                })
            }
            1 => Ok(Self::Disconnect),
            2 => Ok(Self::Status),
            3 => {
                ensure_size!(in: src, size: 1);
                let filter = match src.read_u8() {
                    0 => None,
                    1 => Some(KeyFilter::decode(src)?),
                    _ => return Err(ironrdp_core::invalid_field_err!("dump filter", "invalid presence flag")),
                };
                Ok(Self::QueryProps { filter })
            }
            4 => {
                let substring = read_opt_string(src)?;
                ensure_size!(in: src, size: 1);
                let last = match src.read_u8() {
                    0 => None,
                    1 => {
                        ensure_size!(in: src, size: 4);
                        Some(src.read_u32())
                    }
                    _ => return Err(ironrdp_core::invalid_field_err!("query last", "invalid presence flag")),
                };
                Ok(Self::QueryLogs { substring, last })
            }
            5 => Ok(Self::Screenshot),
            6 => {
                ensure_size!(in: src, size: 4);
                let x = src.read_u16();
                let y = src.read_u16();
                Ok(Self::MouseMove { x, y })
            }
            7 => {
                let button = read_mouse_button(src)?;
                let pressed = read_bool(src)?;
                Ok(Self::MouseButton { button, pressed })
            }
            8 => {
                ensure_size!(in: src, size: 2);
                let delta = src.read_i16();
                let horizontal = read_bool(src)?;
                Ok(Self::Wheel { delta, horizontal })
            }
            9 => {
                ensure_size!(in: src, size: 2);
                let scancode = src.read_u16();
                let pressed = read_bool(src)?;
                Ok(Self::KeyScancode { scancode, pressed })
            }
            10 => {
                let ch = read_char(src)?;
                let pressed = read_bool(src)?;
                Ok(Self::KeyUnicode { ch, pressed })
            }
            11 => {
                ensure_size!(in: src, size: 4);
                let width = src.read_u16();
                let height = src.read_u16();
                Ok(Self::Resize { width, height })
            }
            12 => {
                let kind = PowerShellKind::decode(src)?;
                let command = read_string(src)?;
                let no_profile = read_bool(src)?;
                let non_interactive = read_bool(src)?;
                Ok(Self::PowerShell {
                    kind,
                    command,
                    no_profile,
                    non_interactive,
                })
            }
            13 => Ok(Self::NowCapabilities),
            14 => Ok(Self::NowExecute(NowExecutionRequest::decode(src)?)),
            15 => {
                ensure_size!(in: src, size: 8);
                Ok(Self::NowCancel {
                    operation_id: src.read_u64(),
                })
            }
            16 => Ok(Self::NowOperations),
            17 => {
                ensure_size!(in: src, size: 8);
                Ok(Self::NowOperationStatus {
                    operation_id: src.read_u64(),
                })
            }
            18 => {
                ensure_size!(in: src, size: 16);
                Ok(Self::NowOperationAttach {
                    operation_id: src.read_u64(),
                    after_sequence: src.read_u64(),
                })
            }
            19 => Ok(Self::NowDiagnostics),
            20 => {
                ensure_size!(in: src, size: 8);
                let operation_id = src.read_u64();
                let data = read_bytes(src)?;
                let last = read_bool(src)?;
                Ok(Self::NowWriteStdin {
                    operation_id,
                    data,
                    last,
                })
            }
            _ => Err(ironrdp_core::invalid_field_err!("request", "unknown tag")),
        }
    }
}

impl_pdu_pod!(Request);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powershell_request_roundtrips_without_exposing_command_in_debug() {
        let request = Request::PowerShell {
            kind: PowerShellKind::PowerShell,
            command: "Write-Output secret".to_owned(),
            no_profile: true,
            non_interactive: true,
        };
        let bytes = ironrdp_core::encode_vec(&request).expect("encode request");
        let decoded = ironrdp_core::decode_owned::<Request>(&bytes).expect("decode request");
        assert_eq!(decoded, request);
        assert!(!format!("{request:?}").contains("secret"));
    }

    #[test]
    fn powershell_result_roundtrips_raw_stream_bytes() {
        let response = Response::Ok(Payload::PowerShell {
            stdout: vec![0, 0x80, b'a'],
            stderr: vec![b'e', 0xff],
            exit_code: 23,
        });
        let bytes = ironrdp_core::encode_vec(&response).expect("encode response");
        let decoded = ironrdp_core::decode_owned::<Response>(&bytes).expect("decode response");
        assert_eq!(decoded, response);
    }

    #[test]
    fn streamed_now_execution_roundtrips_raw_stdin_and_output() {
        let request = Request::NowExecute(NowExecutionRequest {
            kind: NowExecutionKind::Process,
            command: r"C:\Program Files\Tool\tool.exe".to_owned(),
            parameters: Some("--input -".to_owned()),
            directory: Some(r"C:\work".to_owned()),
            no_profile: true,
            non_interactive: true,
            detached: false,
            timeout_secs: Some(45),
            stdin: Some(vec![0, 0x80, b'\n']),
        });
        let bytes = ironrdp_core::encode_vec(&request).expect("encode request");
        let decoded = ironrdp_core::decode_owned::<Request>(&bytes).expect("decode request");
        assert_eq!(decoded, request);
        assert!(!format!("{request:?}").contains("tool.exe"));

        let response = Response::Ok(Payload::NowExecutionData {
            operation_id: 17,
            sequence: 3,
            stream: NowStream::Stderr,
            data: vec![0, 0xff],
        });
        let bytes = ironrdp_core::encode_vec(&response).expect("encode response");
        let decoded = ironrdp_core::decode_owned::<Response>(&bytes).expect("decode response");
        assert_eq!(decoded, response);
    }

    #[test]
    fn now_capabilities_roundtrip() {
        let response = Response::Ok(Payload::NowCapabilities(NowCapabilities {
            major: 1,
            minor: 6,
            system_capset: 1,
            session_capset: 0x1f,
            server_exec_capset: 0x107f,
            client_exec_capset: 0x107f,
            exec_capset: 0x107f,
            heartbeat_secs: Some(60),
        }));
        let bytes = ironrdp_core::encode_vec(&response).expect("encode response");
        let decoded = ironrdp_core::decode_owned::<Response>(&bytes).expect("decode response");
        assert_eq!(decoded, response);
    }

    #[test]
    fn durable_now_operation_protocol_roundtrips_with_typed_errors() {
        let request = Request::NowOperationAttach {
            operation_id: 17,
            after_sequence: 8,
        };
        let bytes = ironrdp_core::encode_vec(&request).expect("encode request");
        let decoded = ironrdp_core::decode_owned::<Request>(&bytes).expect("decode request");
        assert_eq!(decoded, request);

        let response = Response::Ok(Payload::NowOperationInfo(NowOperationInfo {
            operation_id: 17,
            kind: NowExecutionKind::PowerShell,
            state: NowOperationState::Succeeded,
            started_unix_ms: 10,
            finished_unix_ms: Some(20),
            exit_code: Some(7),
            error: None,
            stdout_bytes: 3,
            stderr_bytes: 4,
            retained_bytes: 7,
            dropped_bytes: 0,
            next_sequence: 3,
        }));
        let bytes = ironrdp_core::encode_vec(&response).expect("encode response");
        let decoded = ironrdp_core::decode_owned::<Response>(&bytes).expect("decode response");
        assert_eq!(decoded, response);

        let error = Response::error("NOW operation 17 was not found");
        let bytes = ironrdp_core::encode_vec(&error).expect("encode error");
        let decoded = ironrdp_core::decode_owned::<Response>(&bytes).expect("decode error");
        assert_eq!(decoded, error);
        assert!(matches!(
            decoded,
            Response::Err(AgentError {
                code: AgentErrorCode::NotFound,
                ..
            })
        ));
    }
}
