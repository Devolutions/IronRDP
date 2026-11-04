//! Agent-owned boundary between the RDP DVC proxy and `now-client`.
//!
//! `now-client` implements NOW framing, capability negotiation, heartbeat enforcement, request
//! encoding, and the execution lifecycle.
//! This module owns the per-session endpoint, the no-heartbeat client configuration, and the
//! background connection and reconnection to the DVC proxy.

use core::pin::Pin;
use core::sync::atomic::{AtomicBool, Ordering};
use core::task::{Context, Poll};
use core::time::Duration;
use std::fmt;
use std::sync::{Mutex, PoisonError};

use ironrdp_client::config::DvcProxyInfo;
use now_client::{NowCapabilities, NowClient, NowClientConfig, NowClientError, NowClientHandle};
use now_proto_pdu::NowChannelCapsetMsg;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{Mutex as AsyncMutex, mpsc};
use tokio::time::{Instant, sleep, timeout_at};
use tracing::{debug, warn};
use uuid::Uuid;

/// The fixed DVC channel exposed by the NOW agent plugin.
pub const DVC_CHANNEL_NAME: &str = "Devolutions::Now::Agent";
/// How long a request waits for the first negotiated channel.
pub const INITIAL_ENDPOINT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a request waits for a replacement channel after a previous connection.
pub const RECONNECT_ENDPOINT_TIMEOUT: Duration = Duration::from_secs(10);
/// Pause between connection attempts while the proxy has not exposed its endpoint.
const RETRY_DELAY: Duration = Duration::from_millis(100);
/// Pause after a peer accepted the connection but did not complete negotiation.
const NEGOTIATION_RETRY_DELAY: Duration = Duration::from_secs(1);

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
    /// No negotiated channel was available by the prescribed deadline.
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
                "NOW channel was not negotiated within {} seconds: {error}",
                timeout.as_secs()
            ),
            Self::Unavailable {
                timeout,
                last_error: None,
            } => write!(f, "NOW channel was not negotiated within {} seconds", timeout.as_secs()),
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

/// A negotiated NOW client and a signal that fires once its transport is gone.
#[derive(Clone)]
struct Connection {
    handle: NowClientHandle,
    transport_closed: mpsc::Sender<()>,
}

/// Transport wrapper that closes its [`Connection`] signal when it is dropped.
///
/// In [`now-client`][now-client], `OwnedWorker::run` owns both split halves of the transport and
/// returns on every transport, framing (`FrameTooLarge`, `FrameBufferTooLarge`, `PduDecode`,
/// `PduEncode`), protocol, channel-close, or `EventQueueFull` error, dropping the transport when
/// `run` exits; per-request validation errors rejected before a write is queued leave the stream
/// intact.
///
/// [now-client]: https://docs.rs/now-client/0.1.0/src/now_client/client.rs.html
struct Tracked<S> {
    inner: S,
    _alive: mpsc::Receiver<()>,
}

impl<S: AsyncRead + Unpin> AsyncRead for Tracked<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Tracked<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Per-RDP-session endpoint and negotiated NOW client.
///
/// Construct this before building an RDP [`ironrdp_client::config::Config`], inject
/// [`Self::dvc_proxy_info`] into its builder, and retain the value for the lifetime of that
/// session.
/// Construction does no I/O.
/// Run [`Self::keep_negotiated`] for the session so negotiation starts as soon as the DVC appears.
pub struct NowEndpoint {
    pipe_name: String,
    connection: Mutex<Option<Connection>>,
    /// Serializes connection attempts so that a request and the negotiator never race for the pipe.
    connecting: AsyncMutex<()>,
    connected_once: AtomicBool,
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
    pub fn new() -> std::io::Result<Self> {
        let id = Uuid::new_v4();
        #[cfg(unix)]
        let pipe_name = {
            let root = private_runtime_directory()?;
            root.join(format!("now-agent-{id}.sock")).to_string_lossy().into_owned()
        };
        #[cfg(windows)]
        let pipe_name = format!("ironrdp-now-agent-{id}");

        Ok(Self {
            pipe_name,
            connection: Mutex::new(None),
            connecting: AsyncMutex::new(()),
            connected_once: AtomicBool::new(false),
        })
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

    /// Returns the negotiated client, connecting first when no live connection exists.
    ///
    /// The first connection waits up to 30 seconds for a negotiated channel.
    /// Every later replacement waits up to 10 seconds.
    /// The wait includes the capability handshake, which is abandoned when the deadline passes.
    pub async fn handle(&self) -> Result<NowClientHandle, NowEndpointError> {
        if let Some(connection) = self.live_connection() {
            return Ok(connection.handle);
        }

        let timeout = if self.connected_once.load(Ordering::Relaxed) {
            RECONNECT_ENDPOINT_TIMEOUT
        } else {
            INITIAL_ENDPOINT_TIMEOUT
        };
        let deadline = Instant::now() + timeout;
        let mut last_error = None;

        loop {
            // The negotiator can hold the connection lock through a stalled handshake,
            // so the deadline must cover the wait for the lock as well.
            match timeout_at(deadline, self.attempt()).await {
                Ok(Ok(connection)) => return Ok(connection.handle),
                Ok(Err(ConnectAttemptError::Client(error))) => return Err(NowEndpointError::Client(error)),
                Ok(Err(ConnectAttemptError::Io(error))) => last_error = Some(error.to_string()),
                Err(_elapsed) => return Err(NowEndpointError::Unavailable { timeout, last_error }),
            }

            if timeout_at(deadline, sleep(RETRY_DELAY)).await.is_err() {
                return Err(NowEndpointError::Unavailable { timeout, last_error });
            }
        }
    }

    /// Keeps a negotiated connection available until the returned future is dropped.
    ///
    /// The [NOW host][now-host] gives up when no client capability set arrives within seconds of
    /// the channel opening.
    /// Waiting for a request before connecting is therefore too late.
    /// The proxy exposes its endpoint only once the host opens the channel, so this polls for it.
    /// [`NowClient::connect`] bounds each capability handshake by
    /// [`NowClientConfig::connect_timeout`] (15 seconds by default), returning
    /// [`NowClientError::HandshakeTimeout`] and dropping the stream when a peer accepts the pipe
    /// without answering the capability set.
    /// After a closure or failed handshake it connects again, which makes the host negotiate
    /// afresh.
    ///
    /// [now-host]:
    ///     https://github.com/Devolutions/devolutions-gateway/blob/8d2910734ac3/devolutions-session/src/dvc/task.rs
    #[expect(clippy::infinite_loop, reason = "the negotiator runs until the session drops it")]
    pub async fn keep_negotiated(&self) {
        loop {
            match self.attempt().await {
                Ok(connection) => {
                    debug!("NOW channel negotiated");
                    connection.transport_closed.closed().await;
                    debug!("NOW channel closed");
                    sleep(RETRY_DELAY).await;
                }
                Err(ConnectAttemptError::Io(_)) => sleep(RETRY_DELAY).await,
                Err(ConnectAttemptError::Client(error)) => {
                    warn!(%error, "NOW negotiation failed");
                    sleep(NEGOTIATION_RETRY_DELAY).await;
                }
            }
        }
    }

    /// Returns a consistent local diagnostic snapshot without initiating a connection.
    pub fn diagnostic_snapshot(&self) -> (bool, Option<Capabilities>) {
        let connection = self.live_connection();
        let capabilities = connection
            .as_ref()
            .map(|connection| connection.handle.capabilities().into());
        (connection.is_some(), capabilities)
    }

    fn live_connection(&self) -> Option<Connection> {
        self.connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|connection| !connection.transport_closed.is_closed())
            .cloned()
    }

    /// Returns the live connection, or makes one attempt to establish it.
    async fn attempt(&self) -> Result<Connection, ConnectAttemptError> {
        let _serialized = self.connecting.lock().await;
        if let Some(connection) = self.live_connection() {
            return Ok(connection);
        }

        let connection = self.connect_once().await?;
        self.connected_once.store(true, Ordering::Relaxed);
        *self.connection.lock().unwrap_or_else(PoisonError::into_inner) = Some(connection.clone());
        Ok(connection)
    }

    #[cfg(unix)]
    async fn connect_once(&self) -> Result<Connection, ConnectAttemptError> {
        let stream = tokio::net::UnixStream::connect(&self.pipe_name)
            .await
            .map_err(ConnectAttemptError::Io)?;
        connect_stream(stream).await.map_err(ConnectAttemptError::Client)
    }

    #[cfg(windows)]
    async fn connect_once(&self) -> Result<Connection, ConnectAttemptError> {
        let path = format!(r"\\.\pipe\{}", self.pipe_name);
        let stream = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(path)
            .map_err(ConnectAttemptError::Io)?;
        connect_stream(stream).await.map_err(ConnectAttemptError::Client)
    }
}

async fn connect_stream<S>(stream: S) -> Result<Connection, NowClientError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (transport_closed, alive) = mpsc::channel(1);
    let stream = Tracked {
        inner: stream,
        _alive: alive,
    };
    // `now-client` closes a connection when no heartbeat arrives within twice the negotiated
    // interval (`NowCapabilities::heartbeat_interval`), and hosts that restart their heartbeat
    // timer on every processed event send none while busy, so an active channel would be closed and
    // its running operations lost.
    // `NowChannelCapsetMsg::default` leaves the heartbeat interval unset (`None`), and
    // `NowChannelCapsetMsg::downgrade` negotiates `None` when neither peer requests one.
    let config = {
        let default = NowClientConfig::default();
        let client_capset = NowChannelCapsetMsg::default().with_exec_capset(default.client_capset.exec_capset());
        NowClientConfig {
            client_capset,
            ..default
        }
    };
    let handle = NowClient::connect(stream, config).await?;
    Ok(Connection {
        handle,
        transport_closed,
    })
}

enum ConnectAttemptError {
    Io(std::io::Error),
    Client(NowClientError),
}

#[cfg(unix)]
fn private_runtime_directory() -> std::io::Result<std::path::PathBuf> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let root = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(root) => std::path::PathBuf::from(root),
        None => {
            let home = std::env::var_os("HOME").ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "HOME is required when XDG_RUNTIME_DIR is unset",
                )
            })?;
            std::path::PathBuf::from(home).join(".local").join("state")
        }
    };
    let directory = root.join("ironrdp-agent");
    std::fs::create_dir_all(&directory)?;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;

    let metadata = std::fs::symlink_metadata(&directory)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "NOW runtime path is not a directory",
        ));
    }
    // SAFETY: `getuid` has no preconditions and is always safe to call.
    let uid = unsafe { libc::getuid() };
    if metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "NOW runtime directory is not private to the current user",
        ));
    }

    Ok(directory)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use now_client::{ExecutionEvent, ExecutionStatus, ProcessRequest, RunRequest};
    use now_proto_pdu::ironrdp_core::{Decode as _, IntoOwned as _, ReadCursor, encode_vec};
    use now_proto_pdu::{
        NowChannelMessage, NowExecCapsetFlags, NowExecDataMsg, NowExecDataStreamKind, NowExecMessage, NowExecResultMsg,
        NowExecStartedMsg, NowMessage,
    };
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;

    #[cfg(unix)]
    type Peer = tokio::net::UnixStream;
    #[cfg(windows)]
    type Peer = tokio::net::windows::named_pipe::NamedPipeServer;

    /// The DVC proxy's side of the endpoint: it hands out one stream per client that connects.
    #[cfg(unix)]
    struct Host {
        listener: tokio::net::UnixListener,
        path: String,
    }

    #[cfg(unix)]
    impl Host {
        fn bind(endpoint: &NowEndpoint) -> Self {
            let path = endpoint.pipe_name().to_owned();
            let listener = tokio::net::UnixListener::bind(&path).expect("test host must bind its endpoint");
            Self { listener, path }
        }

        async fn accept(&mut self) -> Peer {
            self.listener.accept().await.expect("test host must accept a client").0
        }
    }

    #[cfg(unix)]
    impl Drop for Host {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[cfg(windows)]
    struct Host {
        path: String,
        next: Peer,
    }

    #[cfg(windows)]
    impl Host {
        fn bind(endpoint: &NowEndpoint) -> Self {
            let path = format!(r"\\.\pipe\{}", endpoint.pipe_name());
            let next = tokio::net::windows::named_pipe::ServerOptions::new()
                .first_pipe_instance(true)
                .create(&path)
                .expect("test host must create its pipe");
            Self { path, next }
        }

        async fn accept(&mut self) -> Peer {
            self.next.connect().await.expect("test host must accept a client");
            let replacement = tokio::net::windows::named_pipe::ServerOptions::new()
                .create(&self.path)
                .expect("test host must create the next pipe instance");
            core::mem::replace(&mut self.next, replacement)
        }
    }

    async fn bounded<T>(future: impl Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(10), future)
            .await
            .expect("test step must complete in time")
    }

    /// Plays the host's half of the handshake: the client's capability set arrives first.
    async fn negotiate<S>(peer: &mut S)
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let first = bounded(read_message(peer)).await;
        let NowMessage::Channel(NowChannelMessage::Capset(capset)) = first else {
            panic!("the client must open with its capability set, got {first:?}");
        };
        assert_eq!(capset.heartbeat_interval(), None);
        write_message(
            peer,
            NowChannelCapsetMsg::default().with_exec_capset(NowExecCapsetFlags::STYLE_RUN),
        )
        .await;
    }

    /// How long the NOW host keeps an opened channel without a client capability set before giving
    /// up on it.
    const HOST_HANDSHAKE_DEADLINE: Duration = Duration::from_secs(5);

    /// Plays the host from the moment it opens the channel: one client must connect and negotiate
    /// in time.
    async fn accept_and_negotiate(host: &mut Host) -> Peer {
        tokio::time::timeout(HOST_HANDSHAKE_DEADLINE, async {
            let mut peer = host.accept().await;
            negotiate(&mut peer).await;
            peer
        })
        .await
        .expect("the client must negotiate before the host gives up on the channel")
    }

    /// Obtains a handle through a request while the host accepts and negotiates one client.
    async fn request_handle(endpoint: &NowEndpoint, host: &mut Host) -> Peer {
        let (handle, peer) = bounded(async { tokio::join!(endpoint.handle(), accept_and_negotiate(host)) }).await;
        handle.expect("a request must obtain a negotiated handle");
        peer
    }

    fn spawn_negotiator(endpoint: &Arc<NowEndpoint>) -> tokio::task::JoinHandle<()> {
        let endpoint = Arc::clone(endpoint);
        tokio::spawn(async move { endpoint.keep_negotiated().await })
    }

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
        let first = NowEndpoint::new().expect("endpoint allocation must succeed");
        let second = NowEndpoint::new().expect("endpoint allocation must succeed");

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
            .expect("adapter connection must succeed")
            .handle;
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

    // oracle: reference Devolutions/devolutions-gateway@8d2910734ac3:devolutions-session/src/dvc/task.rs
    // (`HANDSHAKE_TIMEOUT`: the host gives up 5 seconds after opening the channel unless a client
    // capability set arrived).
    #[tokio::test]
    async fn negotiation_starts_before_any_request() {
        let endpoint = Arc::new(NowEndpoint::new().expect("endpoint allocation must succeed"));
        let negotiator = spawn_negotiator(&endpoint);

        // The proxy exposes its endpoint only when the host opens the channel, after the
        // negotiator started.
        sleep(Duration::from_millis(250)).await;
        let mut host = Host::bind(&endpoint);
        let _peer = accept_and_negotiate(&mut host).await;
        let _handle = bounded(endpoint.handle())
            .await
            .expect("the negotiated handle must be available");

        let (connected, capabilities) = endpoint.diagnostic_snapshot();
        assert!(connected);
        assert_eq!(capabilities.map(|capabilities| capabilities.run), Some(true));
        negotiator.abort();
    }

    // oracle: mutant a negotiator that serves only a session's first channel and ignores reopens
    #[tokio::test]
    async fn negotiation_resumes_when_the_channel_reopens() {
        let endpoint = Arc::new(NowEndpoint::new().expect("endpoint allocation must succeed"));
        let mut host = Host::bind(&endpoint);
        let negotiator = spawn_negotiator(&endpoint);

        let first = accept_and_negotiate(&mut host).await;
        drop(first);

        let _second = accept_and_negotiate(&mut host).await;
        bounded(endpoint.handle())
            .await
            .expect("the reopened channel must serve requests");
        assert!(endpoint.diagnostic_snapshot().0);
        negotiator.abort();
    }

    // oracle: mutant a negotiator that stops after a failed handshake, leaving the session dead
    #[tokio::test]
    async fn negotiation_retries_after_a_failed_handshake() {
        let endpoint = Arc::new(NowEndpoint::new().expect("endpoint allocation must succeed"));
        let mut host = Host::bind(&endpoint);
        let negotiator = spawn_negotiator(&endpoint);

        drop(bounded(host.accept()).await);

        let _second = accept_and_negotiate(&mut host).await;
        negotiator.abort();
    }

    // oracle: mutant a request that gives up while the proxy has not yet exposed its endpoint.
    #[tokio::test]
    async fn a_request_waits_for_the_endpoint_to_appear() {
        let endpoint = NowEndpoint::new().expect("endpoint allocation must succeed");

        let late_host = async {
            sleep(Duration::from_millis(250)).await;
            let mut host = Host::bind(&endpoint);
            let peer = accept_and_negotiate(&mut host).await;
            (host, peer)
        };
        let (handle, _host_and_peer) = bounded(async { tokio::join!(endpoint.handle(), late_host) }).await;

        handle.expect("a request must wait for the endpoint to appear");
    }

    // oracle: mutant a request that connects while the negotiator's handshake is in flight
    #[tokio::test]
    async fn a_request_joins_the_negotiation_in_progress() {
        let endpoint = Arc::new(NowEndpoint::new().expect("endpoint allocation must succeed"));
        let mut host = Host::bind(&endpoint);
        let negotiator = spawn_negotiator(&endpoint);

        // The negotiator is connected and has sent its capability set, but the host has not
        // answered it yet.
        let mut peer = bounded(host.accept()).await;
        let (handle, ()) = bounded(async { tokio::join!(endpoint.handle(), negotiate(&mut peer)) }).await;

        handle.expect("the request must receive the negotiator's connection");
        negotiator.abort();
    }

    // oracle: mutant a failed request that shortens the next request's wait to the reconnect bound
    #[tokio::test(start_paused = true)]
    async fn a_request_without_an_endpoint_waits_the_initial_deadline_every_time() {
        let endpoint = NowEndpoint::new().expect("endpoint allocation must succeed");

        for _ in 0..2 {
            let started = Instant::now();
            let outcome = tokio::time::timeout(2 * INITIAL_ENDPOINT_TIMEOUT, endpoint.handle())
                .await
                .expect("a request must stop waiting at its own deadline");
            let waited = started.elapsed();

            let error = outcome
                .err()
                .expect("a request without an endpoint must not yield a handle");
            assert!(
                matches!(
                    &error,
                    NowEndpointError::Unavailable { timeout, last_error: Some(_) } if *timeout == INITIAL_ENDPOINT_TIMEOUT
                ),
                "the request must report the endpoint unavailable with its last connection error: {error:?}"
            );
            assert!(
                (INITIAL_ENDPOINT_TIMEOUT..INITIAL_ENDPOINT_TIMEOUT + Duration::from_secs(1)).contains(&waited),
                "the request waited {waited:?} for a {INITIAL_ENDPOINT_TIMEOUT:?} deadline"
            );
        }
    }

    // oracle: mutant a request whose last retry pause runs past its deadline before answering
    #[tokio::test(start_paused = true)]
    async fn a_request_never_outlives_its_deadline() {
        let endpoint = Arc::new(NowEndpoint::new().expect("endpoint allocation must succeed"));

        // A negotiator that holds the connection lock for 50 ms moves the request's retries off
        // the pause grid.
        let busy = Arc::clone(&endpoint);
        let negotiator = tokio::spawn(async move {
            let _serialized = busy.connecting.lock().await;
            sleep(Duration::from_millis(50)).await;
        });
        tokio::task::yield_now().await;

        let started = Instant::now();
        let outcome = tokio::time::timeout(2 * INITIAL_ENDPOINT_TIMEOUT, endpoint.handle())
            .await
            .expect("a request must stop waiting at its own deadline");
        let waited = started.elapsed();
        negotiator.await.expect("the lock holder must finish");

        let error = outcome
            .err()
            .expect("a request without an endpoint must not yield a handle");
        assert!(
            matches!(error, NowEndpointError::Unavailable { timeout, .. } if timeout == INITIAL_ENDPOINT_TIMEOUT),
            "the request must report the endpoint unavailable at its deadline: {error:?}"
        );
        assert!(
            (INITIAL_ENDPOINT_TIMEOUT..INITIAL_ENDPOINT_TIMEOUT + Duration::from_millis(10)).contains(&waited),
            "the request waited {waited:?} for a {INITIAL_ENDPOINT_TIMEOUT:?} deadline"
        );
    }

    // oracle: mutant a request served a cached handle whose channel has already closed.
    #[tokio::test]
    async fn request_replaces_a_connection_whose_channel_closed() {
        let endpoint = NowEndpoint::new().expect("endpoint allocation must succeed");
        let mut host = Host::bind(&endpoint);

        let first = request_handle(&endpoint, &mut host).await;
        let stale = endpoint
            .live_connection()
            .expect("the negotiated connection must be cached");
        drop(first);
        bounded(stale.transport_closed.closed()).await;
        assert!(!endpoint.diagnostic_snapshot().0);

        let _second = request_handle(&endpoint, &mut host).await;
        assert!(endpoint.diagnostic_snapshot().0);
    }

    // oracle: mutant a request that waits for a stalled handshake beyond its own deadline
    #[tokio::test]
    async fn a_request_stops_waiting_at_its_deadline_while_a_handshake_is_stalled() {
        let endpoint = Arc::new(NowEndpoint::new().expect("endpoint allocation must succeed"));
        let mut host = Host::bind(&endpoint);

        // A first connection selects the shorter reconnect deadline.
        let first = request_handle(&endpoint, &mut host).await;
        let stale = endpoint
            .live_connection()
            .expect("the negotiated connection must be cached");
        drop(first);
        bounded(stale.transport_closed.closed()).await;

        // The negotiator reconnects and the host never answers its capability set.
        let negotiator = spawn_negotiator(&endpoint);
        let _silent = bounded(host.accept()).await;

        // Only timers are pending from here on, so the clock may jump to the next deadline.
        tokio::time::pause();
        let started = Instant::now();
        let outcome = tokio::time::timeout(2 * RECONNECT_ENDPOINT_TIMEOUT, endpoint.handle())
            .await
            .expect("a request must stop waiting at its own deadline");
        let waited = started.elapsed();
        negotiator.abort();

        let error = outcome.err().expect("a stalled negotiation must not yield a handle");
        assert!(
            matches!(error, NowEndpointError::Unavailable { timeout, .. } if timeout == RECONNECT_ENDPOINT_TIMEOUT),
            "the request must report the endpoint unavailable at its deadline: {error:?}"
        );
        assert!(
            (RECONNECT_ENDPOINT_TIMEOUT..RECONNECT_ENDPOINT_TIMEOUT + Duration::from_secs(1)).contains(&waited),
            "the request waited {waited:?} for a {RECONNECT_ENDPOINT_TIMEOUT:?} deadline"
        );
    }

    // oracle: reference Devolutions/devolutions-gateway@8d2910734ac3:devolutions-session/src/dvc/task.rs
    // (the heartbeat sleep is recreated on every pass of the select loop, so a busy host can stay
    // silent for minutes).
    #[tokio::test(start_paused = true)]
    async fn a_peer_that_sends_no_heartbeats_keeps_the_connection_open() {
        // More than twice the longest heartbeat interval `now-proto-pdu` lets a client request
        // (24 hours).
        const SILENCE: Duration = Duration::from_secs(3 * 24 * 60 * 60);

        let (client_stream, mut peer_stream) = tokio::io::duplex(4096);
        let peer = tokio::spawn(async move {
            negotiate(&mut peer_stream).await;
            core::future::pending::<()>().await;
        });

        let connection = connect_stream(client_stream)
            .await
            .expect("adapter connection must succeed");
        assert_eq!(connection.handle.capabilities().heartbeat_interval(), None);
        sleep(SILENCE).await;

        assert!(!connection.transport_closed.is_closed());
        peer.abort();
    }
}
