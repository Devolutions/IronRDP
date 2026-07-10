//! Client-side NOW protocol support over the agent's per-session DVC pipe proxy.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use core::time::Duration;
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{Context as _, bail};
use now_proto_pdu::ironrdp_core::{Decode as _, DecodeErrorKind, IntoOwned as _, ReadCursor};
use now_proto_pdu::{
    NowChannelCapsetMsg, NowChannelHeartbeatMsg, NowChannelMessage, NowExecBatchMsg, NowExecCancelReqMsg,
    NowExecCapsetFlags, NowExecDataMsg, NowExecDataStreamKind, NowExecMessage, NowExecProcessMsg, NowExecPwshMsg,
    NowExecShellMsg, NowExecWinPsMsg, NowMessage, NowProtoVersion,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::{Mutex, Notify, mpsc};
use tracing::{debug, info};

use crate::ipc::{NowCapabilities, NowExecutionKind, NowExecutionRequest, NowStream};

pub(crate) const DVC_CHANNEL_NAME: &str = "Devolutions::Now::Agent";
const MAX_MESSAGE_BODY_LEN: usize = 16 * 1024 * 1024;
/// Leaves 1 MiB for IPC framing and conservative future response overhead.
pub(crate) const MAX_POWERSHELL_OUTPUT_LEN: usize = 15 * 1024 * 1024;
const IO_BUFFER_LEN: usize = 64 * 1024;
const NON_INTERACTIVE_FLAG: u16 = 0x0020;
const INITIAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

static NEXT_ENDPOINT_ID: AtomicU64 = AtomicU64::new(0);
static NEXT_OPERATION_ID: AtomicU64 = AtomicU64::new(1);

/// The remote PowerShell implementation to invoke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PowerShellKind {
    WindowsPowerShell,
    PowerShell,
}

/// A PowerShell invocation sent to the NOW agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PowerShellRequest {
    pub(crate) kind: PowerShellKind,
    pub(crate) command: String,
    pub(crate) no_profile: bool,
    pub(crate) non_interactive: bool,
}

/// The byte streams and exit status emitted by a remote PowerShell invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PowerShellResult {
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) exit_code: u32,
}

/// Owns the unique local endpoint used by IronRDP's DVC pipe proxy for one RDP session.
pub(crate) struct NowClient {
    endpoint: String,
    state: Arc<Mutex<NowState>>,
    active: Arc<StdMutex<Option<ActiveOperation>>>,
}

#[derive(Clone)]
struct ActiveOperation {
    id: u64,
    cancellation_requested: Arc<AtomicBool>,
    cancellation: Arc<Notify>,
}

/// A raw NOW execution event sent from the protocol worker to the daemon IPC connection.
#[derive(Debug)]
pub(crate) enum NowOperationEvent {
    Data { stream: NowStream, data: Vec<u8> },
    Finished { exit_code: u32 },
    Failed { message: String },
}

/// A running NOW operation with a local ID and a stream of protocol events.
pub(crate) struct NowOperation {
    pub(crate) id: u64,
    pub(crate) events: mpsc::UnboundedReceiver<NowOperationEvent>,
}

struct NowState {
    endpoint: LocalEndpoint,
    stream: Option<LocalStream>,
    buffer: MessageBuffer,
    read_buffer: Box<[u8; IO_BUFFER_LEN]>,
    caps: Option<NowChannelCapsetMsg>,
    heartbeat_interval: Option<Duration>,
    next_session_id: u32,
    connected_once: bool,
}

impl NowClient {
    pub(crate) fn new() -> anyhow::Result<Self> {
        let endpoint = LocalEndpoint::new()?;
        let endpoint_name = endpoint.name().to_owned();
        Ok(Self {
            endpoint: endpoint_name,
            state: Arc::new(Mutex::new(NowState {
                endpoint,
                stream: None,
                buffer: MessageBuffer::default(),
                read_buffer: Box::new([0; IO_BUFFER_LEN]),
                caps: None,
                heartbeat_interval: None,
                next_session_id: 1,
                connected_once: false,
            })),
            active: Arc::new(StdMutex::new(None)),
        })
    }

    /// Returns the unique DVC proxy endpoint to add to the RDP client configuration.
    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(crate) async fn execute(&self, request: PowerShellRequest) -> anyhow::Result<PowerShellResult> {
        let mut state = self.state.lock().await;
        state.ensure_connected().await?;
        state.negotiate().await?;

        let required_style = match request.kind {
            PowerShellKind::WindowsPowerShell => NowExecCapsetFlags::STYLE_WINPS,
            PowerShellKind::PowerShell => NowExecCapsetFlags::STYLE_PWSH,
        };
        // Gateway currently replies with its full capability set rather than the negotiated
        // intersection, so retain the client's advertised limit when gating optional behavior.
        let caps =
            state.caps.as_ref().expect("NOW capabilities negotiated").exec_capset() & client_capset()?.exec_capset();
        if !caps.contains(required_style) {
            let shell = match request.kind {
                PowerShellKind::WindowsPowerShell => "Windows PowerShell",
                PowerShellKind::PowerShell => "PowerShell 7",
            };
            bail!("remote NOW agent does not support {shell}");
        }
        if !caps.contains(NowExecCapsetFlags::IO_REDIRECTION) {
            bail!("remote NOW agent does not support execution stream redirection");
        }

        let session_id = state.allocate_session_id();
        let encoded_request = encode_powershell_request(session_id, &request, caps)?;
        state.write_raw(&encoded_request).await?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        loop {
            match state.read_message().await? {
                NowMessage::Exec(NowExecMessage::Started(started)) if started.session_id() == session_id => {
                    // A started notification is advisory; the matching result below is authoritative.
                }
                NowMessage::Exec(NowExecMessage::Data(data)) if data.session_id() == session_id => {
                    match data.stream_kind().context("invalid NOW execution stream kind")? {
                        NowExecDataStreamKind::Stdout => append_output(&mut stdout, stderr.len(), data.data())?,
                        NowExecDataStreamKind::Stderr => append_output(&mut stderr, stdout.len(), data.data())?,
                        NowExecDataStreamKind::Stdin => bail!("remote NOW agent returned unexpected stdin data"),
                    }
                }
                NowMessage::Exec(NowExecMessage::Result(result)) if result.session_id() == session_id => {
                    let exit_code = result
                        .to_result()
                        .map_err(|error| anyhow::anyhow!("remote NOW execution failed: {error}"))?;
                    debug!(session_id, exit_code, "Received NOW PowerShell execution result");
                    return Ok(PowerShellResult {
                        stdout,
                        stderr,
                        exit_code,
                    });
                }
                NowMessage::Channel(NowChannelMessage::Capset(caps)) => {
                    state.caps = Some(caps);
                }
                NowMessage::Channel(NowChannelMessage::Heartbeat(_)) => {}
                message => bail!("unexpected NOW message while waiting for execution result: {message:?}"),
            }
        }
    }

    /// Establishes the local NOW transport and returns its negotiated capabilities.
    pub(crate) async fn capabilities(&self) -> anyhow::Result<NowCapabilities> {
        let mut state = self.state.lock().await;
        state.ensure_connected().await?;
        state.negotiate().await?;
        state.capabilities()
    }

    /// Starts a streamed execution. Only one execution can own a NOW transport at a time, but the
    /// returned operation can be cancelled from another daemon IPC connection.
    pub(crate) fn start_execution(self: &Arc<Self>, request: NowExecutionRequest) -> anyhow::Result<NowOperation> {
        validate_execution_request(&request)?;

        let id = NEXT_OPERATION_ID.fetch_add(1, Ordering::Relaxed);
        let id = if id == 0 {
            NEXT_OPERATION_ID.fetch_add(1, Ordering::Relaxed)
        } else {
            id
        };
        let active = ActiveOperation {
            id,
            cancellation_requested: Arc::new(AtomicBool::new(false)),
            cancellation: Arc::new(Notify::new()),
        };
        {
            let mut guard = self.active.lock().expect("NOW operation state poisoned");
            if guard.is_some() {
                bail!("a NOW execution is already running");
            }
            *guard = Some(active.clone());
        }

        let (events_tx, events) = mpsc::unbounded_channel();
        let client = Arc::clone(self);
        tokio::spawn(async move {
            let result = {
                let mut state = client.state.lock().await;
                state.execute_streamed(request, &active, &events_tx).await
            };
            match result {
                Ok(exit_code) => {
                    let _ = events_tx.send(NowOperationEvent::Finished { exit_code });
                }
                Err(error) => {
                    let _ = events_tx.send(NowOperationEvent::Failed {
                        message: format!("{error:#}"),
                    });
                }
            }
            let mut guard = client.active.lock().expect("NOW operation state poisoned");
            if guard.as_ref().is_some_and(|current| current.id == id) {
                *guard = None;
            }
        });

        Ok(NowOperation { id, events })
    }

    /// Requests normal NOW cancellation. The active worker sends `CANCEL_REQ` while continuing to
    /// read until its matching result, so the byte stream remains protocol-synchronized.
    pub(crate) fn cancel(&self, operation_id: u64) -> anyhow::Result<()> {
        let guard = self.active.lock().expect("NOW operation state poisoned");
        let Some(active) = guard.as_ref() else {
            bail!("no NOW execution is running");
        };
        if active.id != operation_id {
            bail!("NOW operation {operation_id} is not running");
        }
        active.cancellation_requested.store(true, Ordering::Release);
        active.cancellation.notify_waiters();
        Ok(())
    }
}

impl NowState {
    async fn ensure_connected(&mut self) -> anyhow::Result<()> {
        if self.stream.is_none() {
            let timeout = if self.connected_once {
                RECONNECT_TIMEOUT
            } else {
                INITIAL_CONNECT_TIMEOUT
            };
            info!(
                endpoint = %self.endpoint.display_name(),
                proxy_name = self.endpoint.name(),
                timeout_secs = timeout.as_secs(),
                reconnect = self.connected_once,
                "Waiting for NOW DVC proxy endpoint"
            );
            let started = tokio::time::Instant::now();
            self.stream = Some(self.endpoint.connect_with_timeout(timeout).await?);
            self.connected_once = true;
            info!(
                endpoint = %self.endpoint.display_name(),
                proxy_name = self.endpoint.name(),
                wait_ms = started.elapsed().as_millis(),
                "Connected to NOW DVC proxy endpoint"
            );
        }
        Ok(())
    }

    async fn negotiate(&mut self) -> anyhow::Result<()> {
        if self.caps.is_some() {
            return Ok(());
        }

        let requested = client_capset()?;
        info!("Sending NOW capability request");
        self.write_message(NowMessage::from(requested)).await?;

        loop {
            match self.read_message().await? {
                NowMessage::Channel(NowChannelMessage::Capset(caps)) => {
                    if caps.version().major != NowProtoVersion::CURRENT.major {
                        bail!(
                            "incompatible NOW protocol version {}.{}",
                            caps.version().major,
                            caps.version().minor
                        );
                    }
                    self.heartbeat_interval = caps.heartbeat_interval();
                    info!(
                        major = caps.version().major,
                        minor = caps.version().minor,
                        heartbeat_secs = ?self.heartbeat_interval.map(|interval| interval.as_secs()),
                        "Negotiated NOW capabilities"
                    );
                    self.caps = Some(caps);
                    return Ok(());
                }
                NowMessage::Channel(NowChannelMessage::Heartbeat(_)) => {}
                message => bail!("expected NOW capability response, received {message:?}"),
            }
        }
    }

    fn allocate_session_id(&mut self) -> u32 {
        let session_id = self.next_session_id;
        self.next_session_id = self.next_session_id.wrapping_add(1);
        if self.next_session_id == 0 {
            self.next_session_id = 1;
        }
        session_id
    }

    async fn write_message(&mut self, message: NowMessage<'_>) -> anyhow::Result<()> {
        let bytes = now_proto_pdu::ironrdp_core::encode_vec(&message).context("encode NOW message")?;
        self.write_raw(&bytes).await
    }

    async fn write_raw(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        let result = {
            let stream = self.stream.as_mut().expect("NOW stream connected");
            async {
                stream.write_all(bytes).await.context("write NOW message")?;
                stream.flush().await.context("flush NOW message")
            }
            .await
        };
        if result.is_err() {
            self.reset_connection();
        }
        result
    }

    async fn read_message(&mut self) -> anyhow::Result<NowMessage<'static>> {
        loop {
            if let Some(message) = self.buffer.next()? {
                return Ok(message);
            }

            let result = {
                let (stream, read_buffer) = (&mut self.stream, &mut self.read_buffer);
                let stream = stream.as_mut().expect("NOW stream connected");
                match self.heartbeat_interval {
                    Some(interval) => match tokio::time::timeout(interval, stream.read(&mut **read_buffer)).await {
                        Ok(result) => Some(result.context("read NOW message")),
                        Err(_) => None,
                    },
                    None => Some(stream.read(&mut **read_buffer).await.context("read NOW message")),
                }
            };
            let Some(result) = result else {
                self.write_message(NowMessage::from(NowChannelHeartbeatMsg::default()))
                    .await
                    .context("send NOW heartbeat")?;
                continue;
            };
            let read = match result {
                Ok(read) if read != 0 => read,
                Ok(_) => {
                    self.reset_connection();
                    bail!("NOW DVC pipe closed");
                }
                Err(error) => {
                    self.reset_connection();
                    return Err(error);
                }
            };
            self.buffer.push(&self.read_buffer[..read]);
        }
    }

    fn reset_connection(&mut self) {
        self.stream = None;
        self.buffer = MessageBuffer::default();
        self.caps = None;
        self.heartbeat_interval = None;
    }

    fn capabilities(&self) -> anyhow::Result<NowCapabilities> {
        let server = self.caps.as_ref().context("NOW capabilities not negotiated")?;
        let requested = client_capset()?;
        let exec_capset = server.exec_capset() & requested.exec_capset();
        Ok(NowCapabilities {
            major: server.version().major,
            minor: server.version().minor,
            system_capset: server.system_capset().bits(),
            session_capset: server.session_capset().bits(),
            exec_capset: exec_capset.bits(),
            heartbeat_secs: server
                .heartbeat_interval()
                .map(|interval| u32::try_from(interval.as_secs()).unwrap_or(u32::MAX)),
        })
    }

    async fn execute_streamed(
        &mut self,
        request: NowExecutionRequest,
        active: &ActiveOperation,
        events: &mpsc::UnboundedSender<NowOperationEvent>,
    ) -> anyhow::Result<u32> {
        self.ensure_connected().await?;
        self.negotiate().await?;

        let caps = self.capabilities()?.exec_capset;
        let caps = NowExecCapsetFlags::from_bits_retain(caps);
        let required_style = execution_style_capability(request.kind);
        if !caps.contains(required_style) {
            bail!(
                "remote NOW agent does not support {}",
                execution_kind_name(request.kind)
            );
        }
        if !request.detached && !caps.contains(NowExecCapsetFlags::IO_REDIRECTION) {
            bail!("remote NOW agent does not support execution stream redirection");
        }
        if active.cancellation_requested.load(Ordering::Acquire) {
            bail!("NOW execution was cancelled before it started");
        }

        let session_id = self.allocate_session_id();
        self.write_raw(&encode_execution_request(session_id, &request, caps)?)
            .await?;
        if request.detached {
            return Ok(0);
        }

        let deadline = request
            .timeout_secs
            .map(|timeout_secs| tokio::time::Instant::now() + Duration::from_secs(u64::from(timeout_secs)));
        let mut cancellation_sent = false;
        let mut cancellation_requested = false;
        let mut stdin_sent = false;

        loop {
            if active.cancellation_requested.load(Ordering::Acquire) && !cancellation_sent {
                self.write_message(NowMessage::from(NowExecCancelReqMsg::new(session_id)))
                    .await
                    .context("send NOW execution cancellation")?;
                cancellation_sent = true;
                cancellation_requested = true;
            }

            let message = match deadline {
                Some(deadline) if !cancellation_sent => {
                    tokio::select! {
                        message = self.read_message() => Some(message?),
                        _ = active.cancellation.notified() => None,
                        _ = tokio::time::sleep_until(deadline) => {
                            active.cancellation_requested.store(true, Ordering::Release);
                            None
                        }
                    }
                }
                _ if !cancellation_sent => {
                    tokio::select! {
                        message = self.read_message() => Some(message?),
                        _ = active.cancellation.notified() => None,
                    }
                }
                _ => Some(self.read_message().await?),
            };
            let Some(message) = message else {
                continue;
            };

            match message {
                NowMessage::Exec(NowExecMessage::Started(started)) if started.session_id() == session_id => {
                    if !stdin_sent {
                        if let Some(stdin) = request.stdin.as_deref() {
                            self.write_stdin(session_id, stdin).await?;
                        }
                        stdin_sent = true;
                    }
                }
                NowMessage::Exec(NowExecMessage::Data(data)) if data.session_id() == session_id => {
                    let stream = match data.stream_kind().context("invalid NOW execution stream kind")? {
                        NowExecDataStreamKind::Stdout => NowStream::Stdout,
                        NowExecDataStreamKind::Stderr => NowStream::Stderr,
                        NowExecDataStreamKind::Stdin => bail!("remote NOW agent returned unexpected stdin data"),
                    };
                    events
                        .send(NowOperationEvent::Data {
                            stream,
                            data: data.data().to_vec(),
                        })
                        .map_err(|_| anyhow::anyhow!("NOW execution output consumer disconnected"))?;
                }
                NowMessage::Exec(NowExecMessage::CancelRsp(response)) if response.session_id() == session_id => {
                    response
                        .to_result()
                        .map_err(|error| anyhow::anyhow!("remote NOW cancellation failed: {error}"))?;
                }
                NowMessage::Exec(NowExecMessage::Result(result)) if result.session_id() == session_id => {
                    let exit_code = result
                        .to_result()
                        .map_err(|error| anyhow::anyhow!("remote NOW execution failed: {error}"))?;
                    debug!(session_id, exit_code, "Received NOW execution result");
                    if cancellation_requested {
                        bail!("NOW execution was cancelled");
                    }
                    return Ok(exit_code);
                }
                NowMessage::Channel(NowChannelMessage::Capset(caps)) => {
                    self.caps = Some(caps);
                }
                NowMessage::Channel(NowChannelMessage::Heartbeat(_)) => {}
                message => bail!("unexpected NOW message while waiting for execution result: {message:?}"),
            }
        }
    }

    async fn write_stdin(&mut self, session_id: u32, stdin: &[u8]) -> anyhow::Result<()> {
        for (index, chunk) in stdin.chunks(IO_BUFFER_LEN).enumerate() {
            let is_last = (index + 1) * IO_BUFFER_LEN >= stdin.len();
            self.write_message(NowMessage::from(
                NowExecDataMsg::new(session_id, NowExecDataStreamKind::Stdin, is_last, chunk)
                    .context("encode NOW standard input")?,
            ))
            .await?;
        }
        if stdin.is_empty() {
            self.write_message(NowMessage::from(
                NowExecDataMsg::new(session_id, NowExecDataStreamKind::Stdin, true, &[])
                    .context("encode NOW empty standard input")?,
            ))
            .await?;
        }
        Ok(())
    }
}

fn append_output(output: &mut Vec<u8>, other_output_len: usize, data: &[u8]) -> anyhow::Result<()> {
    let output_len = output
        .len()
        .checked_add(other_output_len)
        .and_then(|len| len.checked_add(data.len()))
        .context("NOW PowerShell output length overflow")?;
    if output_len > MAX_POWERSHELL_OUTPUT_LEN {
        bail!(
            "NOW PowerShell output exceeds the {} MiB IPC limit",
            MAX_POWERSHELL_OUTPUT_LEN / (1024 * 1024)
        );
    }
    output.extend_from_slice(data);
    Ok(())
}

fn encode_powershell_request(
    session_id: u32,
    request: &PowerShellRequest,
    caps: NowExecCapsetFlags,
) -> anyhow::Result<Vec<u8>> {
    let message = match request.kind {
        PowerShellKind::WindowsPowerShell => {
            let mut message = NowExecWinPsMsg::new(session_id, request.command.as_str())
                .context("encode Windows PowerShell command")?;
            if request.no_profile {
                message = message.set_no_profile();
            }

            message = message.with_io_redirection();
            if caps.contains(NowExecCapsetFlags::UNICODE_CONSOLE) {
                message = message.with_raw_encoding().with_unicode_console();
            }
            NowMessage::from(message)
        }
        PowerShellKind::PowerShell => {
            let mut message =
                NowExecPwshMsg::new(session_id, request.command.as_str()).context("encode PowerShell command")?;
            if request.no_profile {
                message = message.set_no_profile();
            }
            message = message.with_io_redirection();
            if caps.contains(NowExecCapsetFlags::UNICODE_CONSOLE) {
                message = message.with_raw_encoding().with_unicode_console();
            }
            NowMessage::from(message)
        }
    };

    let mut encoded = now_proto_pdu::ironrdp_core::encode_vec(&message).context("encode NOW PowerShell request")?;
    if request.non_interactive {
        // now-proto-pdu exposes the decoder for this standard flag but not a builder setter yet.
        // The common NOW header stores flags as a little-endian u16 at byte offset 6.
        let flag_bytes = encoded
            .get_mut(6..8)
            .context("encoded NOW PowerShell request is missing header flags")?;
        let flags = u16::from_le_bytes([flag_bytes[0], flag_bytes[1]]) | NON_INTERACTIVE_FLAG;
        flag_bytes.copy_from_slice(&flags.to_le_bytes());
    }
    Ok(encoded)
}

fn validate_execution_request(request: &NowExecutionRequest) -> anyhow::Result<()> {
    if request.command.is_empty() {
        bail!("NOW execution command must not be empty");
    }
    if request.timeout_secs == Some(0) {
        bail!("NOW execution timeout must be greater than zero");
    }
    if request.detached && request.stdin.is_some() {
        bail!("detached NOW execution does not support standard input");
    }
    if request.detached && request.timeout_secs.is_some() {
        bail!("detached NOW execution does not support a timeout");
    }
    match request.kind {
        NowExecutionKind::Process if request.parameters.is_none() => {}
        NowExecutionKind::Shell if request.parameters.is_none() => {}
        NowExecutionKind::Batch if request.parameters.is_some() => {
            bail!("NOW batch execution does not support --parameters");
        }
        NowExecutionKind::WindowsPowerShell | NowExecutionKind::PowerShell if request.parameters.is_some() => {
            bail!("NOW PowerShell execution does not support --parameters");
        }
        _ => {}
    }
    Ok(())
}

fn execution_style_capability(kind: NowExecutionKind) -> NowExecCapsetFlags {
    match kind {
        NowExecutionKind::WindowsPowerShell => NowExecCapsetFlags::STYLE_WINPS,
        NowExecutionKind::PowerShell => NowExecCapsetFlags::STYLE_PWSH,
        NowExecutionKind::Process => NowExecCapsetFlags::STYLE_PROCESS,
        NowExecutionKind::Shell => NowExecCapsetFlags::STYLE_SHELL,
        NowExecutionKind::Batch => NowExecCapsetFlags::STYLE_BATCH,
    }
}

fn execution_kind_name(kind: NowExecutionKind) -> &'static str {
    match kind {
        NowExecutionKind::WindowsPowerShell => "Windows PowerShell",
        NowExecutionKind::PowerShell => "PowerShell 7",
        NowExecutionKind::Process => "CreateProcess execution",
        NowExecutionKind::Shell => "shell execution",
        NowExecutionKind::Batch => "batch execution",
    }
}

fn encode_execution_request(
    session_id: u32,
    request: &NowExecutionRequest,
    caps: NowExecCapsetFlags,
) -> anyhow::Result<Vec<u8>> {
    let message = match request.kind {
        NowExecutionKind::WindowsPowerShell => {
            let mut message = NowExecWinPsMsg::new(session_id, request.command.as_str())
                .context("encode Windows PowerShell command")?;
            if request.no_profile {
                message = message.set_no_profile();
            }
            if let Some(directory) = request.directory.as_deref() {
                message = message
                    .with_directory(directory)
                    .context("encode Windows PowerShell directory")?;
            }
            if !request.detached {
                message = message.with_io_redirection();
            } else {
                message = message.with_detached();
            }
            if caps.contains(NowExecCapsetFlags::UNICODE_CONSOLE) {
                message = message.with_raw_encoding().with_unicode_console();
            }
            let mut encoded =
                now_proto_pdu::ironrdp_core::encode_vec(&NowMessage::from(message)).context("encode NOW request")?;
            if request.non_interactive {
                set_non_interactive(&mut encoded)?;
            }
            return Ok(encoded);
        }
        NowExecutionKind::PowerShell => {
            let mut message =
                NowExecPwshMsg::new(session_id, request.command.as_str()).context("encode PowerShell command")?;
            if request.no_profile {
                message = message.set_no_profile();
            }
            if let Some(directory) = request.directory.as_deref() {
                message = message
                    .with_directory(directory)
                    .context("encode PowerShell directory")?;
            }
            if !request.detached {
                message = message.with_io_redirection();
            } else {
                message = message.with_detached();
            }
            if caps.contains(NowExecCapsetFlags::UNICODE_CONSOLE) {
                message = message.with_raw_encoding().with_unicode_console();
            }
            let mut encoded =
                now_proto_pdu::ironrdp_core::encode_vec(&NowMessage::from(message)).context("encode NOW request")?;
            if request.non_interactive {
                set_non_interactive(&mut encoded)?;
            }
            return Ok(encoded);
        }
        NowExecutionKind::Process => {
            let mut message =
                NowExecProcessMsg::new(session_id, request.command.as_str()).context("encode NOW process filename")?;
            if let Some(parameters) = request.parameters.as_deref() {
                message = message
                    .with_parameters(parameters)
                    .context("encode NOW process parameters")?;
            }
            if let Some(directory) = request.directory.as_deref() {
                message = message
                    .with_directory(directory)
                    .context("encode NOW process directory")?;
            }
            if request.detached {
                message = message.with_detached();
            } else {
                message = message.with_io_redirection();
            }
            if caps.contains(NowExecCapsetFlags::UNICODE_CONSOLE) {
                message = message.with_encoding_utf8();
            }
            NowMessage::from(message)
        }
        NowExecutionKind::Shell => {
            let mut message =
                NowExecShellMsg::new(session_id, request.command.as_str()).context("encode NOW shell command")?;
            if let Some(shell) = request.parameters.as_deref() {
                message = message.with_shell(shell).context("encode NOW shell path")?;
            }
            if let Some(directory) = request.directory.as_deref() {
                message = message
                    .with_directory(directory)
                    .context("encode NOW shell directory")?;
            }
            if request.detached {
                message = message.with_detached();
            } else {
                message = message.with_io_redirection();
            }
            NowMessage::from(message)
        }
        NowExecutionKind::Batch => {
            let mut message =
                NowExecBatchMsg::new(session_id, request.command.as_str()).context("encode NOW batch command")?;
            if let Some(directory) = request.directory.as_deref() {
                message = message
                    .with_directory(directory)
                    .context("encode NOW batch directory")?;
            }
            if request.detached {
                message = message.with_detached();
            } else {
                message = message.with_io_redirection();
            }
            if caps.contains(NowExecCapsetFlags::UNICODE_CONSOLE) {
                message = message.with_raw_encoding().with_unicode_console();
            }
            NowMessage::from(message)
        }
    };
    now_proto_pdu::ironrdp_core::encode_vec(&message).context("encode NOW execution request")
}

fn set_non_interactive(encoded: &mut [u8]) -> anyhow::Result<()> {
    // now-proto-pdu exposes the decoder for this standard flag but not a builder setter yet.
    // The common NOW header stores flags as a little-endian u16 at byte offset 6.
    let flag_bytes = encoded
        .get_mut(6..8)
        .context("encoded NOW PowerShell request is missing header flags")?;
    let flags = u16::from_le_bytes([flag_bytes[0], flag_bytes[1]]) | NON_INTERACTIVE_FLAG;
    flag_bytes.copy_from_slice(&flags.to_le_bytes());
    Ok(())
}

fn client_capset() -> anyhow::Result<NowChannelCapsetMsg> {
    NowChannelCapsetMsg::default()
        .with_exec_capset(
            NowExecCapsetFlags::STYLE_PROCESS
                | NowExecCapsetFlags::STYLE_SHELL
                | NowExecCapsetFlags::STYLE_BATCH
                | NowExecCapsetFlags::STYLE_WINPS
                | NowExecCapsetFlags::STYLE_PWSH
                | NowExecCapsetFlags::IO_REDIRECTION
                | NowExecCapsetFlags::UNICODE_CONSOLE,
        )
        .with_heartbeat_interval(HEARTBEAT_INTERVAL)
        .context("set NOW heartbeat interval")
}

/// Buffered NOW message decoder that handles DVC fragmentation and coalescing.
#[derive(Default)]
struct MessageBuffer {
    bytes: Vec<u8>,
    start: usize,
}

impl MessageBuffer {
    fn push(&mut self, data: &[u8]) {
        self.bytes.extend_from_slice(data);
    }

    fn next(&mut self) -> anyhow::Result<Option<NowMessage<'static>>> {
        let available = &self.bytes[self.start..];
        if available.len() >= 4 {
            let body_len = u32::from_le_bytes(available[..4].try_into().expect("slice length checked"));
            let body_len = usize::try_from(body_len).expect("u32 fits in usize on supported platforms");
            if body_len > MAX_MESSAGE_BODY_LEN {
                bail!("NOW message body length {body_len} exceeds the {MAX_MESSAGE_BODY_LEN}-byte limit");
            }
        }

        let mut cursor = ReadCursor::new(available);
        match NowMessage::decode(&mut cursor) {
            Ok(message) => {
                let consumed = cursor.pos();
                let message = message.into_owned();
                let _ = cursor;
                self.start += consumed;
                if self.start == self.bytes.len() {
                    self.bytes.clear();
                    self.start = 0;
                } else if self.start >= self.bytes.len() / 2 {
                    let remaining = self.bytes.len() - self.start;
                    self.bytes.copy_within(self.start.., 0);
                    self.bytes.truncate(remaining);
                    self.start = 0;
                }
                Ok(Some(message))
            }
            Err(error) if matches!(error.kind, DecodeErrorKind::NotEnoughBytes { .. }) => Ok(None),
            Err(error) => Err(error).context("decode NOW message"),
        }
    }
}

#[cfg(unix)]
type LocalStream = tokio::net::UnixStream;
#[cfg(windows)]
type LocalStream = tokio::net::windows::named_pipe::NamedPipeClient;

#[cfg(unix)]
struct LocalEndpoint {
    path: std::path::PathBuf,
    name: String,
}

#[cfg(unix)]
impl LocalEndpoint {
    fn new() -> anyhow::Result<Self> {
        let directory = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let id = NEXT_ENDPOINT_ID.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!("ironrdp-agent-now-{}-{id}.sock", std::process::id()));
        let name = path.to_string_lossy().into_owned();
        Ok(Self { path, name })
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn display_name(&self) -> String {
        self.path.display().to_string()
    }

    async fn connect_with_timeout(&self, timeout: Duration) -> anyhow::Result<LocalStream> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match tokio::net::UnixStream::connect(&self.path).await {
                Ok(stream) => return Ok(stream),
                Err(_) if tokio::time::Instant::now() < deadline => {
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    tokio::time::sleep(remaining.min(Duration::from_millis(100))).await;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "NOW DVC pipe {} did not connect within {} seconds",
                            self.path.display(),
                            timeout.as_secs()
                        )
                    });
                }
            }
        }
    }
}

#[cfg(windows)]
struct LocalEndpoint {
    name: String,
}

#[cfg(windows)]
impl LocalEndpoint {
    fn new() -> anyhow::Result<Self> {
        let id = NEXT_ENDPOINT_ID.fetch_add(1, Ordering::Relaxed);
        // DvcNamedPipeProxy adds the Windows pipe namespace itself.
        let name = format!("ironrdp-agent-now-{}-{id}", std::process::id());
        Ok(Self { name })
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn display_name(&self) -> String {
        format!(r"\\.\pipe\{}", self.name)
    }

    async fn connect_with_timeout(&self, timeout: Duration) -> anyhow::Result<LocalStream> {
        use tokio::net::windows::named_pipe::ClientOptions;

        let pipe_path = format!(r"\\.\pipe\{}", self.name);
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match ClientOptions::new().open(&pipe_path) {
                Ok(pipe) => return Ok(pipe),
                Err(_) if tokio::time::Instant::now() < deadline => {
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    tokio::time::sleep(remaining.min(Duration::from_millis(100))).await;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "NOW DVC pipe {} did not connect within {} seconds",
                            pipe_path,
                            timeout.as_secs()
                        )
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_buffer_reassembles_fragmented_and_coalesced_messages() {
        let capset = NowMessage::from(
            NowChannelCapsetMsg::default()
                .with_exec_capset(NowExecCapsetFlags::STYLE_WINPS | NowExecCapsetFlags::IO_REDIRECTION),
        );
        let started = NowMessage::from(now_proto_pdu::NowExecStartedMsg::new(7));
        let stdout = NowMessage::from(
            NowExecDataMsg::new(7, NowExecDataStreamKind::Stdout, false, b"stdout".as_slice()).expect("valid output"),
        );
        let result = NowMessage::from(now_proto_pdu::NowExecResultMsg::new_success(7, 0));

        let mut bytes = now_proto_pdu::ironrdp_core::encode_vec(&capset).expect("encode capset");
        bytes.extend(now_proto_pdu::ironrdp_core::encode_vec(&started).expect("encode started"));
        bytes.extend(now_proto_pdu::ironrdp_core::encode_vec(&stdout).expect("encode stdout"));
        bytes.extend(now_proto_pdu::ironrdp_core::encode_vec(&result).expect("encode result"));

        let split = 5;
        let mut buffer = MessageBuffer::default();
        buffer.push(&bytes[..split]);
        assert!(buffer.next().expect("decode fragment").is_none());
        buffer.push(&bytes[split..]);

        let mut messages = Vec::new();
        while let Some(message) = buffer.next().expect("decode message") {
            messages.push(message);
        }
        assert_eq!(messages.len(), 4);
        assert!(matches!(messages[0], NowMessage::Channel(NowChannelMessage::Capset(_))));
        assert!(matches!(messages[1], NowMessage::Exec(NowExecMessage::Started(_))));
        assert!(matches!(messages[2], NowMessage::Exec(NowExecMessage::Data(_))));
        assert!(matches!(messages[3], NowMessage::Exec(NowExecMessage::Result(_))));
    }

    #[test]
    fn powershell_request_defaults_to_no_profile_and_non_interactive() {
        let request = PowerShellRequest {
            kind: PowerShellKind::WindowsPowerShell,
            command: "$PSVersionTable.PSVersion".to_owned(),
            no_profile: true,
            non_interactive: true,
        };
        let encoded = encode_powershell_request(
            42,
            &request,
            NowExecCapsetFlags::STYLE_WINPS | NowExecCapsetFlags::IO_REDIRECTION | NowExecCapsetFlags::UNICODE_CONSOLE,
        )
        .expect("encode PowerShell request");
        let mut cursor = ReadCursor::new(&encoded);
        let message = NowMessage::decode(&mut cursor).expect("decode PowerShell request");
        let NowMessage::Exec(NowExecMessage::WinPs(message)) = message else {
            panic!("expected Windows PowerShell request");
        };
        assert!(message.is_no_profile());
        assert!(message.is_non_interactive());
        assert!(message.is_with_io_redirection());
        assert!(message.is_raw_encoding());
        assert!(message.is_unicode_console());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn now_exec_result_preserves_exit_code() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        use tokio::net::windows::named_pipe::ServerOptions;

        let client = NowClient::new().expect("create NOW client");
        let pipe_path = format!(r"\\.\pipe\{}", client.endpoint());
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(pipe_path)
            .expect("create DVC proxy endpoint");

        let server_task = tokio::spawn(async move {
            server.connect().await.expect("accept NOW client");
            let mut buffer = MessageBuffer::default();
            let mut read_buffer = [0; 1024];

            let capset = loop {
                let read = server.read(&mut read_buffer).await.expect("read capability request");
                buffer.push(&read_buffer[..read]);
                if let Some(message) = buffer.next().expect("decode capability request") {
                    break message;
                }
            };
            assert!(matches!(capset, NowMessage::Channel(NowChannelMessage::Capset(_))));

            let caps = NowChannelCapsetMsg::default()
                .with_exec_capset(NowExecCapsetFlags::STYLE_PWSH | NowExecCapsetFlags::IO_REDIRECTION);
            let caps = now_proto_pdu::ironrdp_core::encode_vec(&NowMessage::from(caps)).expect("encode capabilities");
            server.write_all(&caps).await.expect("write capabilities");

            let request = loop {
                let read = server.read(&mut read_buffer).await.expect("read PowerShell request");
                buffer.push(&read_buffer[..read]);
                if let Some(message) = buffer.next().expect("decode PowerShell request") {
                    break message;
                }
            };
            assert!(matches!(request, NowMessage::Exec(NowExecMessage::Pwsh(_))));

            let result = NowMessage::from(now_proto_pdu::NowExecResultMsg::new_success(1, 7));
            let result = now_proto_pdu::ironrdp_core::encode_vec(&result).expect("encode result");
            server.write_all(&result).await.expect("write result");
            server.flush().await.expect("flush result");
        });

        let result = client
            .execute(PowerShellRequest {
                kind: PowerShellKind::PowerShell,
                command: "exit 7".to_owned(),
                no_profile: true,
                non_interactive: true,
            })
            .await
            .expect("execute NOW PowerShell request");
        server_task.await.expect("join DVC proxy endpoint");
        assert_eq!(result.exit_code, 7);

        let payload = crate::ipc::Payload::PowerShell {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code,
        };
        let crate::ipc::Payload::PowerShell { exit_code, .. } = payload else {
            panic!("expected PowerShell payload");
        };
        assert_eq!(exit_code, 7);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn streamed_execution_forwards_stdin_and_raw_output() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        use tokio::net::windows::named_pipe::ServerOptions;

        let client = Arc::new(NowClient::new().expect("create NOW client"));
        let pipe_path = format!(r"\\.\pipe\{}", client.endpoint());
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(pipe_path)
            .expect("create DVC proxy endpoint");

        let server_task = tokio::spawn(async move {
            server.connect().await.expect("accept NOW client");
            let mut buffer = MessageBuffer::default();
            let mut read_buffer = [0; 1024];

            async fn read_next(
                server: &mut tokio::net::windows::named_pipe::NamedPipeServer,
                buffer: &mut MessageBuffer,
                read_buffer: &mut [u8],
            ) -> NowMessage<'static> {
                loop {
                    if let Some(message) = buffer.next().expect("decode NOW message") {
                        return message;
                    }
                    let read = server.read(read_buffer).await.expect("read NOW message");
                    buffer.push(&read_buffer[..read]);
                }
            }

            assert!(matches!(
                read_next(&mut server, &mut buffer, &mut read_buffer).await,
                NowMessage::Channel(NowChannelMessage::Capset(_))
            ));
            let caps = NowChannelCapsetMsg::default().with_exec_capset(
                NowExecCapsetFlags::STYLE_PWSH
                    | NowExecCapsetFlags::IO_REDIRECTION
                    | NowExecCapsetFlags::UNICODE_CONSOLE,
            );
            let caps = now_proto_pdu::ironrdp_core::encode_vec(&NowMessage::from(caps)).expect("encode capabilities");
            server.write_all(&caps).await.expect("write capabilities");

            assert!(matches!(
                read_next(&mut server, &mut buffer, &mut read_buffer).await,
                NowMessage::Exec(NowExecMessage::Pwsh(_))
            ));
            let started =
                now_proto_pdu::ironrdp_core::encode_vec(&NowMessage::from(now_proto_pdu::NowExecStartedMsg::new(1)))
                    .expect("encode started");
            server.write_all(&started).await.expect("write started");

            let stdin = read_next(&mut server, &mut buffer, &mut read_buffer).await;
            let NowMessage::Exec(NowExecMessage::Data(stdin)) = stdin else {
                panic!("expected standard input data");
            };
            assert_eq!(stdin.stream_kind().expect("stdin stream"), NowExecDataStreamKind::Stdin);
            assert!(stdin.is_last());
            assert_eq!(stdin.data(), b"input\x00");

            let stdout = NowMessage::from(
                NowExecDataMsg::new(1, NowExecDataStreamKind::Stdout, false, b"stdout\x80".as_slice())
                    .expect("encode stdout"),
            );
            let stderr = NowMessage::from(
                NowExecDataMsg::new(1, NowExecDataStreamKind::Stderr, true, b"stderr\xff".as_slice())
                    .expect("encode stderr"),
            );
            let result = NowMessage::from(now_proto_pdu::NowExecResultMsg::new_success(1, 7));
            for message in [stdout, stderr, result] {
                let bytes = now_proto_pdu::ironrdp_core::encode_vec(&message).expect("encode response");
                server.write_all(&bytes).await.expect("write response");
            }
            server.flush().await.expect("flush responses");
        });

        let request = NowExecutionRequest {
            kind: NowExecutionKind::PowerShell,
            command: "ignored".to_owned(),
            parameters: None,
            directory: None,
            no_profile: true,
            non_interactive: true,
            detached: false,
            timeout_secs: Some(30),
            stdin: Some(b"input\x00".to_vec()),
        };
        let mut operation = client.start_execution(request).expect("start streamed execution");
        let mut events = Vec::new();
        while let Some(event) = operation.events.recv().await {
            events.push(event);
        }
        server_task.await.expect("join DVC proxy endpoint");

        assert!(matches!(
            events.as_slice(),
            [
                NowOperationEvent::Data {
                    stream: NowStream::Stdout,
                    data,
                },
                NowOperationEvent::Data {
                    stream: NowStream::Stderr,
                    data: _,
                },
                NowOperationEvent::Finished { exit_code: 7 },
            ] if data == b"stdout\x80"
        ));
        let NowOperationEvent::Data { data, .. } = &events[1] else {
            panic!("expected stderr event");
        };
        assert_eq!(data, b"stderr\xff");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn streamed_execution_sends_cancel_request_and_waits_for_terminal_result() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        use tokio::net::windows::named_pipe::ServerOptions;
        use tokio::sync::oneshot;

        let client = Arc::new(NowClient::new().expect("create NOW client"));
        let pipe_path = format!(r"\\.\pipe\{}", client.endpoint());
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(pipe_path)
            .expect("create DVC proxy endpoint");
        let (request_seen_tx, request_seen_rx) = oneshot::channel();

        let server_task = tokio::spawn(async move {
            server.connect().await.expect("accept NOW client");
            let mut buffer = MessageBuffer::default();
            let mut read_buffer = [0; 1024];

            async fn read_next(
                server: &mut tokio::net::windows::named_pipe::NamedPipeServer,
                buffer: &mut MessageBuffer,
                read_buffer: &mut [u8],
            ) -> NowMessage<'static> {
                loop {
                    if let Some(message) = buffer.next().expect("decode NOW message") {
                        return message;
                    }
                    let read = server.read(read_buffer).await.expect("read NOW message");
                    buffer.push(&read_buffer[..read]);
                }
            }

            assert!(matches!(
                read_next(&mut server, &mut buffer, &mut read_buffer).await,
                NowMessage::Channel(NowChannelMessage::Capset(_))
            ));
            let caps = NowChannelCapsetMsg::default()
                .with_exec_capset(NowExecCapsetFlags::STYLE_PWSH | NowExecCapsetFlags::IO_REDIRECTION);
            let caps = now_proto_pdu::ironrdp_core::encode_vec(&NowMessage::from(caps)).expect("encode capabilities");
            server.write_all(&caps).await.expect("write capabilities");

            assert!(matches!(
                read_next(&mut server, &mut buffer, &mut read_buffer).await,
                NowMessage::Exec(NowExecMessage::Pwsh(_))
            ));
            request_seen_tx.send(()).expect("notify request");

            let cancel = read_next(&mut server, &mut buffer, &mut read_buffer).await;
            let NowMessage::Exec(NowExecMessage::CancelReq(cancel)) = cancel else {
                panic!("expected NOW cancellation request");
            };
            assert_eq!(cancel.session_id(), 1);

            for message in [
                NowMessage::from(now_proto_pdu::NowExecCancelRspMsg::new_success(1)),
                NowMessage::from(now_proto_pdu::NowExecResultMsg::new_success(1, 0)),
            ] {
                let bytes = now_proto_pdu::ironrdp_core::encode_vec(&message).expect("encode response");
                server.write_all(&bytes).await.expect("write response");
            }
            server.flush().await.expect("flush responses");
        });

        let mut operation = client
            .start_execution(NowExecutionRequest {
                kind: NowExecutionKind::PowerShell,
                command: "Start-Sleep 60".to_owned(),
                parameters: None,
                directory: None,
                no_profile: true,
                non_interactive: true,
                detached: false,
                timeout_secs: None,
                stdin: None,
            })
            .expect("start streamed execution");
        request_seen_rx.await.expect("wait for request");
        client.cancel(operation.id).expect("request cancellation");

        let event = operation.events.recv().await.expect("terminal event");
        let NowOperationEvent::Failed { message } = event else {
            panic!("cancellation must produce a terminal error");
        };
        assert!(message.contains("was cancelled"));
        assert!(operation.events.recv().await.is_none());
        server_task.await.expect("join DVC proxy endpoint");
    }

    #[test]
    fn client_capset_advertises_power_shell_and_heartbeat_support() {
        let capset = client_capset().expect("build client capset");
        let capabilities = capset.exec_capset();
        assert!(capabilities.contains(NowExecCapsetFlags::STYLE_PROCESS));
        assert!(capabilities.contains(NowExecCapsetFlags::STYLE_SHELL));
        assert!(capabilities.contains(NowExecCapsetFlags::STYLE_BATCH));
        assert!(capabilities.contains(NowExecCapsetFlags::STYLE_WINPS));
        assert!(capabilities.contains(NowExecCapsetFlags::STYLE_PWSH));
        assert!(capabilities.contains(NowExecCapsetFlags::IO_REDIRECTION));
        assert_eq!(capset.heartbeat_interval(), Some(HEARTBEAT_INTERVAL));
    }

    #[test]
    fn generic_process_request_uses_redirection_and_utf8_when_available() {
        let request = NowExecutionRequest {
            kind: NowExecutionKind::Process,
            command: r"C:\tool.exe".to_owned(),
            parameters: Some("--json".to_owned()),
            directory: Some(r"C:\work".to_owned()),
            no_profile: true,
            non_interactive: true,
            detached: false,
            timeout_secs: None,
            stdin: None,
        };
        let encoded = encode_execution_request(
            12,
            &request,
            NowExecCapsetFlags::STYLE_PROCESS
                | NowExecCapsetFlags::IO_REDIRECTION
                | NowExecCapsetFlags::UNICODE_CONSOLE,
        )
        .expect("encode process request");
        let mut cursor = ReadCursor::new(&encoded);
        let message = NowMessage::decode(&mut cursor).expect("decode process request");
        let NowMessage::Exec(NowExecMessage::Process(message)) = message else {
            panic!("expected process request");
        };
        assert_eq!(message.filename(), r"C:\tool.exe");
        assert_eq!(message.parameters(), Some("--json"));
        assert_eq!(message.directory(), Some(r"C:\work"));
        assert!(message.is_with_io_redirection());
        assert!(message.is_encoding_utf8());
    }

    #[test]
    fn detached_execution_rejects_stdin_and_timeout() {
        let request = NowExecutionRequest {
            kind: NowExecutionKind::Batch,
            command: "start cmd".to_owned(),
            parameters: None,
            directory: None,
            no_profile: true,
            non_interactive: true,
            detached: true,
            timeout_secs: None,
            stdin: Some(vec![1]),
        };
        assert!(
            validate_execution_request(&request)
                .expect_err("detached stdin must be rejected")
                .to_string()
                .contains("does not support standard input")
        );

        let request = NowExecutionRequest {
            stdin: None,
            timeout_secs: Some(1),
            ..request
        };
        assert!(
            validate_execution_request(&request)
                .expect_err("detached timeout must be rejected")
                .to_string()
                .contains("does not support a timeout")
        );
    }

    #[test]
    fn cancellation_marks_the_matching_running_operation() {
        let client = NowClient::new().expect("create NOW client");
        let active = ActiveOperation {
            id: 22,
            cancellation_requested: Arc::new(AtomicBool::new(false)),
            cancellation: Arc::new(Notify::new()),
        };
        *client.active.lock().expect("operation state") = Some(active.clone());
        client.cancel(22).expect("cancel matching operation");
        assert!(active.cancellation_requested.load(Ordering::Acquire));
        assert!(
            client
                .cancel(23)
                .expect_err("wrong operation must fail")
                .to_string()
                .contains("not running")
        );
    }

    #[test]
    fn powershell_output_limit_is_shared_across_streams() {
        let mut stdout = Vec::new();
        append_output(&mut stdout, MAX_POWERSHELL_OUTPUT_LEN - 1, b"x").expect("output below limit");

        let error =
            append_output(&mut stdout, MAX_POWERSHELL_OUTPUT_LEN, b"x").expect_err("output above limit must fail");
        assert!(error.to_string().contains("15 MiB IPC limit"));
    }

    #[tokio::test]
    async fn endpoint_fails_when_dvc_proxy_does_not_open() {
        assert_eq!(INITIAL_CONNECT_TIMEOUT, Duration::from_secs(30));
        assert_eq!(RECONNECT_TIMEOUT, Duration::from_secs(10));
        let endpoint = LocalEndpoint::new().expect("create endpoint");
        let error = endpoint
            .connect_with_timeout(Duration::ZERO)
            .await
            .expect_err("missing DVC proxy endpoint must fail");
        assert!(error.to_string().contains("NOW DVC pipe"));
        assert!(error.to_string().contains("did not connect within 0 seconds"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn endpoint_connects_as_the_now_client() {
        let endpoint = LocalEndpoint::new().expect("create endpoint");
        let listener = tokio::net::UnixListener::bind(&endpoint.path).expect("bind DVC proxy endpoint");
        let (client, server) = tokio::join!(
            endpoint.connect_with_timeout(INITIAL_CONNECT_TIMEOUT),
            listener.accept()
        );
        client.expect("connect NOW client");
        server.expect("accept NOW client");
        std::fs::remove_file(&endpoint.path).expect("remove test endpoint");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn endpoint_connects_as_the_now_client() {
        use tokio::net::windows::named_pipe::ServerOptions;

        let endpoint = LocalEndpoint::new().expect("create endpoint");
        assert!(!endpoint.name().starts_with(r"\\.\pipe\"));
        let pipe_path = format!(r"\\.\pipe\{}", endpoint.name());
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(pipe_path)
            .expect("create DVC proxy endpoint");
        let (client, accepted) = tokio::join!(endpoint.connect_with_timeout(INITIAL_CONNECT_TIMEOUT), server.connect());
        client.expect("connect NOW client");
        accepted.expect("accept NOW client");
    }
}
