//! Agent-owned boundary between the RDP DVC proxy and `now-client`.
//!
//! NOW framing, negotiation, heartbeats, request encoding, and execution lifecycle all belong to
//! `now-client`. This module deliberately only creates a per-session local endpoint, waits for the
//! DVC proxy to expose it, and caches a connected client handle.

use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;
use std::fmt;

use ironrdp_client::config::DvcProxyInfo;
use now_client::{NowCapabilities, NowClient, NowClientConfig, NowClientError, NowClientHandle};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex;
use tokio::time::{Instant, sleep};

/// The fixed DVC channel exposed by the NOW agent plugin.
pub const DVC_CHANNEL_NAME: &str = "Devolutions::Now::Agent";
/// First local DVC endpoint readiness deadline.
pub const INITIAL_ENDPOINT_TIMEOUT: Duration = Duration::from_secs(30);
/// Replacement local DVC endpoint readiness deadline after a previous connection.
pub const RECONNECT_ENDPOINT_TIMEOUT: Duration = Duration::from_secs(10);
const RETRY_DELAY: Duration = Duration::from_millis(100);
static NEXT_ENDPOINT_ID: AtomicU64 = AtomicU64::new(1);

/// A summarized capability snapshot suitable for agent IPC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// NOW protocol major version.
    pub version_major: u16,
    /// NOW protocol minor version.
    pub version_minor: u16,
    /// The negotiated heartbeat period in milliseconds, if one is active.
    pub heartbeat_ms: Option<u64>,
    /// Whether generic `Run` requests are supported.
    pub run: bool,
    /// Whether Windows CreateProcess requests are supported.
    pub process: bool,
    /// Whether batch requests are supported.
    pub batch: bool,
    /// Whether Windows PowerShell requests are supported.
    pub powershell: bool,
    /// Whether PowerShell 7 requests are supported.
    pub pwsh: bool,
    /// Whether tracked stdin/stdout/stderr redirection is supported.
    pub io_redirection: bool,
    /// Whether UTF-8 console output is supported.
    pub unicode_console: bool,
}

impl From<NowCapabilities> for Capabilities {
    fn from(value: NowCapabilities) -> Self {
        let version = value.version();
        Self {
            version_major: version.major,
            version_minor: version.minor,
            heartbeat_ms: value
                .heartbeat_interval()
                .map(|interval| u64::try_from(interval.as_millis()).unwrap_or(u64::MAX)),
            run: value.supports_run(),
            process: value.supports_process(),
            batch: value.supports_batch(),
            powershell: value.supports_win_ps(),
            pwsh: value.supports_pwsh(),
            io_redirection: value.supports_io_redirection(),
            unicode_console: value.supports_unicode_console(),
        }
    }
}

/// Failures at the local agent/DVC boundary.
#[derive(Debug)]
pub enum NowEndpointError {
    /// The proxy did not expose its local endpoint by the prescribed deadline.
    Unavailable {
        /// The deadline used for this connection attempt.
        timeout: Duration,
        /// The last local connection error, when one was available.
        last_error: Option<String>,
    },
    /// `now-client` rejected the local stream or its peer.
    Client(NowClientError),
}

impl fmt::Display for NowEndpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable {
                timeout,
                last_error: Some(error),
            } => write!(
                f,
                "NOW DVC endpoint did not become available within {} seconds: {error}",
                timeout.as_secs()
            ),
            Self::Unavailable {
                timeout,
                last_error: None,
            } => write!(
                f,
                "NOW DVC endpoint did not become available within {} seconds",
                timeout.as_secs()
            ),
            Self::Client(error) => write!(f, "{error}"),
        }
    }
}

impl core::error::Error for NowEndpointError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Client(error) => Some(error),
            Self::Unavailable { .. } => None,
        }
    }
}

struct EndpointState {
    handle: Option<NowClientHandle>,
    connected_once: bool,
}

/// Per-RDP-session endpoint and cached NOW client.
///
/// Construct this before building an RDP [`ironrdp_client::config::Config`], inject
/// [`Self::dvc_proxy_info`] into its builder, and retain the value for the lifetime of that
/// session. It intentionally does no I/O until [`Self::handle`] is first called.
pub struct NowEndpoint {
    pipe_name: String,
    state: Mutex<EndpointState>,
}

impl fmt::Debug for NowEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NowEndpoint")
            .field("pipe_name", &self.pipe_name)
            .finish_non_exhaustive()
    }
}

impl NowEndpoint {
    /// Allocates a unique endpoint for one RDP session.
    pub fn new() -> Self {
        let id = NEXT_ENDPOINT_ID.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        #[cfg(unix)]
        let pipe_name = {
            let root = std::env::var_os("XDG_RUNTIME_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            root.join(format!("ironrdp-now-agent-{pid}-{id}.sock"))
                .to_string_lossy()
                .into_owned()
        };
        #[cfg(windows)]
        let pipe_name = format!("ironrdp-now-agent-{pid}-{id}");

        Self {
            pipe_name,
            state: Mutex::new(EndpointState {
                handle: None,
                connected_once: false,
            }),
        }
    }

    /// Returns the agent's DVC proxy mapping for this endpoint.
    pub fn dvc_proxy_info(&self) -> DvcProxyInfo {
        DvcProxyInfo {
            channel_name: DVC_CHANNEL_NAME.to_owned(),
            pipe_name: self.pipe_name.clone(),
        }
    }

    /// Returns the local endpoint name. Windows returns the intentionally unqualified pipe name.
    pub fn pipe_name(&self) -> &str {
        &self.pipe_name
    }

    /// Gets the cached handle or waits for the DVC proxy, connects it, and negotiates NOW.
    ///
    /// The first successful connection has a 30-second readiness deadline. Every later replacement
    /// has a 10-second deadline.
    pub async fn handle(&self) -> Result<NowClientHandle, NowEndpointError> {
        let mut state = self.state.lock().await;
        if let Some(handle) = &state.handle {
            return Ok(handle.clone());
        }

        let timeout = if state.connected_once {
            RECONNECT_ENDPOINT_TIMEOUT
        } else {
            INITIAL_ENDPOINT_TIMEOUT
        };
        let handle = self.connect(timeout).await?;
        state.connected_once = true;
        state.handle = Some(handle.clone());
        Ok(handle)
    }

    /// Discards a potentially unusable worker. The next operation reconnects with the replacement
    /// deadline rather than attempting to reuse a closed NOW command queue.
    pub async fn invalidate(&self) {
        self.state.lock().await.handle = None;
    }

    /// Returns the currently cached capability snapshot without initiating a local connection.
    pub async fn cached_capabilities(&self) -> Option<Capabilities> {
        self.state
            .lock()
            .await
            .handle
            .as_ref()
            .map(|handle| handle.capabilities().into())
    }

    /// Whether a negotiated NOW client is currently cached.
    pub async fn is_connected(&self) -> bool {
        self.state.lock().await.handle.is_some()
    }

    async fn connect(&self, timeout: Duration) -> Result<NowClientHandle, NowEndpointError> {
        let deadline = Instant::now() + timeout;
        let mut last_error: Option<String>;

        loop {
            match self.connect_once().await {
                Ok(handle) => return Ok(handle),
                Err(ConnectAttemptError::Client(error)) => return Err(NowEndpointError::Client(error)),
                Err(ConnectAttemptError::Io(error)) => last_error = Some(error.to_string()),
            }

            if Instant::now() >= deadline {
                return Err(NowEndpointError::Unavailable { timeout, last_error });
            }

            sleep(RETRY_DELAY).await;
        }
    }

    #[cfg(unix)]
    async fn connect_once(&self) -> Result<NowClientHandle, ConnectAttemptError> {
        let stream = tokio::net::UnixStream::connect(&self.pipe_name)
            .await
            .map_err(ConnectAttemptError::Io)?;
        connect_stream(stream).await.map_err(ConnectAttemptError::Client)
    }

    #[cfg(windows)]
    async fn connect_once(&self) -> Result<NowClientHandle, ConnectAttemptError> {
        let path = format!(r"\\.\pipe\{}", self.pipe_name);
        let stream = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(path)
            .map_err(ConnectAttemptError::Io)?;
        connect_stream(stream).await.map_err(ConnectAttemptError::Client)
    }
}

async fn connect_stream<S>(stream: S) -> Result<NowClientHandle, NowClientError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    NowClient::connect(stream, NowClientConfig::default()).await
}

impl Default for NowEndpoint {
    fn default() -> Self {
        Self::new()
    }
}

enum ConnectAttemptError {
    Io(std::io::Error),
    Client(NowClientError),
}

/// Whether an error means the cached client must be replaced before another request.
pub fn invalidates_handle(error: &NowClientError) -> bool {
    matches!(
        error,
        NowClientError::Io(_)
            | NowClientError::PduEncode(_)
            | NowClientError::PduDecode(_)
            | NowClientError::Protocol(_)
            | NowClientError::FrameTooLarge { .. }
            | NowClientError::FrameBufferTooLarge { .. }
            | NowClientError::WorkerClosed(_)
            | NowClientError::EventQueueFull { .. }
    )
}

#[cfg(test)]
mod tests {
    use now_client::{ExecutionEvent, ExecutionStatus, ProcessRequest, RunRequest};
    use now_proto_pdu::ironrdp_core::{Decode as _, IntoOwned as _, ReadCursor, encode_vec};
    use now_proto_pdu::{
        NowChannelCapsetMsg, NowExecCapsetFlags, NowExecDataMsg, NowExecDataStreamKind, NowExecMessage,
        NowExecResultMsg, NowExecStartedMsg, NowMessage,
    };
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;

    fn encode(message: impl Into<NowMessage<'static>>) -> Vec<u8> {
        encode_vec(&message.into()).expect("test NOW PDU must encode")
    }

    async fn read_message<S>(stream: &mut S) -> NowMessage<'static>
    where
        S: AsyncRead + Unpin,
    {
        let mut header = [0; 8];
        stream
            .read_exact(&mut header)
            .await
            .expect("test peer must receive a header");
        let body_size = usize::try_from(u32::from_le_bytes(
            header[..4].try_into().expect("header is four bytes"),
        ))
        .expect("u32 fits usize");
        let mut bytes = header.to_vec();
        bytes.resize(8 + body_size, 0);
        stream
            .read_exact(&mut bytes[8..])
            .await
            .expect("test peer must receive a body");
        let mut cursor = ReadCursor::new(&bytes);
        NowMessage::decode(&mut cursor)
            .expect("test peer must decode a NOW PDU")
            .into_owned()
    }

    async fn write_message<S>(stream: &mut S, message: impl Into<NowMessage<'static>>)
    where
        S: AsyncWrite + Unpin,
    {
        stream
            .write_all(&encode(message))
            .await
            .expect("test peer must write a NOW PDU");
    }

    #[test]
    fn endpoint_names_are_unique_and_use_the_agent_channel() {
        let first = NowEndpoint::new();
        let second = NowEndpoint::new();

        assert_ne!(first.pipe_name(), second.pipe_name());
        assert_eq!(first.dvc_proxy_info().channel_name, DVC_CHANNEL_NAME);
        #[cfg(windows)]
        assert!(!first.pipe_name().starts_with(r"\\.\pipe\"));
    }

    #[test]
    fn connection_deadlines_match_the_reconnect_policy() {
        assert_eq!(INITIAL_ENDPOINT_TIMEOUT, Duration::from_secs(30));
        assert_eq!(RECONNECT_ENDPOINT_TIMEOUT, Duration::from_secs(10));
    }

    #[tokio::test]
    async fn adapter_discards_immediate_run_frames_before_process() {
        let (client_stream, mut peer_stream) = tokio::io::duplex(4096);
        let peer = tokio::spawn(async move {
            let _ = read_message(&mut peer_stream).await;
            let capset = NowChannelCapsetMsg::default().with_exec_capset(
                NowExecCapsetFlags::STYLE_RUN | NowExecCapsetFlags::STYLE_PROCESS | NowExecCapsetFlags::IO_REDIRECTION,
            );
            write_message(&mut peer_stream, capset).await;

            let run_session = match read_message(&mut peer_stream).await {
                NowMessage::Exec(NowExecMessage::Run(message)) => message.session_id(),
                message => panic!("expected Run request, got {message:?}"),
            };
            write_message(&mut peer_stream, NowExecStartedMsg::new(run_session)).await;
            write_message(
                &mut peer_stream,
                NowExecDataMsg::new(run_session, NowExecDataStreamKind::Stdout, true, vec![0xaa, 0xbb])
                    .expect("test Run data PDU must encode"),
            )
            .await;
            write_message(&mut peer_stream, NowExecResultMsg::new_success(run_session, 0)).await;

            let process_session = match read_message(&mut peer_stream).await {
                NowMessage::Exec(NowExecMessage::Process(message)) => message.session_id(),
                message => panic!("expected Process request, got {message:?}"),
            };
            write_message(&mut peer_stream, NowExecStartedMsg::new(process_session)).await;
            write_message(
                &mut peer_stream,
                NowExecDataMsg::new(
                    process_session,
                    NowExecDataStreamKind::Stdout,
                    true,
                    vec![0xff, 0x00, 0x80],
                )
                .expect("test Process data PDU must encode"),
            )
            .await;
            write_message(&mut peer_stream, NowExecResultMsg::new_success(process_session, 17)).await;
        });

        let handle = connect_stream(client_stream)
            .await
            .expect("adapter connection must succeed");
        handle
            .run(RunRequest::new("run.exe"))
            .await
            .expect("Run request must be accepted");
        let mut process = handle
            .process(ProcessRequest::new("process.exe"))
            .await
            .expect("Process request must start");

        assert_eq!(process.next_event().await, Some(ExecutionEvent::Started));
        assert_eq!(
            process.next_event().await,
            Some(ExecutionEvent::Stdout {
                data: vec![0xff, 0x00, 0x80],
                last: true,
            })
        );
        match process.wait().await {
            Ok(ExecutionStatus::Completed { exit_code: 17 }) => {}
            Ok(status) => panic!("Process returned unexpected status: {status:?}"),
            Err(error) => panic!("Process must complete: {error}"),
        }
        peer.await.expect("test peer task must complete");
    }
}
