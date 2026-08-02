//! The long-lived daemon: owns the [`RdpClient`] engine and one RDP session, and serves IPC
//! requests until shut down.
//!
//! One daemon serves one RDP session (multi-session is out of scope for V1). It is started
//! explicitly with `daemon-start` and runs in the foreground; the caller is expected to background
//! it. On a clean shutdown the Unix socket file is removed (see [`crate::transport`]).

use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use ironrdp_client::config::{ConfigBuilder, MissingField};
use ironrdp_client::rdp::{RdpClient, RdpInputEvent, RdpInputSender, RdpOutputEvent};
use ironrdp_input::{Database, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_propertyset::{PropertySet, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};

use crate::ipc::{
    ConnState, KeyFilter, NowDiagnostics, Payload, PropValue, PropertyDump, PropertyEntry, Request, Response,
    StatusInfo,
};
use crate::logbuf::{self, LogBuffer};
use crate::now::NowEndpoint;
use crate::operations::{OperationAttachment, OperationManager};
use crate::transport::{Endpoint, Listener, read_message, write_message};

/// Binds the IPC endpoint and serves requests until a shutdown signal is received.
///
/// `overlay` is an operator-provided [`PropertySet`] layered on top of every `Connect` request
/// (overlay wins), so any setting — credentials in particular — can be preconfigured without the
/// caller ever supplying it. Pass an empty set when no overlay is desired.
pub async fn run(endpoint: Endpoint, overlay: PropertySet) -> anyhow::Result<()> {
    // On Unix a leftover socket file would make `bind` fail; clear it if no daemon is alive.
    #[cfg(unix)]
    if endpoint.0.exists() {
        if crate::transport::connect(&endpoint).await.is_ok() {
            anyhow::bail!("a daemon already appears to be running at {endpoint}");
        }
        // No daemon answered, so the path is a stale socket we can reclaim. Guard against deleting
        // an unrelated regular file (or following a symlink) when `--endpoint` points elsewhere:
        // inspect the path itself and only remove genuine sockets.
        use std::os::unix::fs::FileTypeExt as _;
        let metadata =
            std::fs::symlink_metadata(&endpoint.0).with_context(|| format!("stat IPC endpoint {endpoint}"))?;
        if !metadata.file_type().is_socket() {
            anyhow::bail!("refusing to remove {endpoint}: path exists and is not a socket");
        }
        std::fs::remove_file(&endpoint.0).with_context(|| format!("remove stale socket {endpoint}"))?;
    }

    init_daemon_logging();
    let logs = LogBuffer::new();

    let mut listener = Listener::bind(&endpoint).with_context(|| format!("bind IPC endpoint {endpoint}"))?;
    let daemon = Arc::new(Daemon::new(logs, overlay));

    info!(%endpoint, "Daemon listening");

    loop {
        tokio::select! {
            result = listener.accept() => {
                let stream = result.context("accept IPC connection")?;
                let daemon = Arc::clone(&daemon);
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(stream, daemon.as_ref()).await {
                        debug!(error = format!("{error:#}"), "IPC connection error");
                    }
                });
            }
            _ = tokio::signal::ctrl_c() => {
                info!("Received shutdown signal, stopping");
                break;
            }
        }
    }

    Ok(())
}

async fn handle_connection<S>(mut stream: S, daemon: &Daemon) -> anyhow::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request: Request = read_message(&mut stream).await?;
    trace!(?request, "Handling IPC request");
    let response = daemon.handle(request).await;
    trace!(ok = response.response().is_ok(), "Replying to IPC request");
    match response {
        DaemonResponse::Single(response) => write_message(&mut stream, &response).await?,
        DaemonResponse::Stream(response, mut attachment) => {
            write_message(&mut stream, &response).await?;
            for event in attachment.replay {
                write_message(&mut stream, &Response::Ok(Payload::NowEvent(event))).await?;
            }
            while let Some(event) = attachment.live.recv().await {
                write_message(&mut stream, &Response::Ok(Payload::NowEvent(event))).await?;
            }
        }
    }
    Ok(())
}

enum DaemonResponse {
    Single(Response),
    Stream(Response, OperationAttachment),
}

impl DaemonResponse {
    fn response(&self) -> &Response {
        match self {
            Self::Single(response) | Self::Stream(response, _) => response,
        }
    }
}

/// The daemon's mutable state: the (single) current session, plus the shared log buffer.
struct Daemon {
    state: Mutex<Option<Session>>,
    /// Serializes synchronous RDP session construction so concurrent IPC `connect` requests cannot
    /// both observe an empty session slot before either one installs its session.
    connect_lock: Mutex<()>,
    logs: Arc<LogBuffer>,
    /// Operator-provided overlay layered on top of every `Connect` (overlay wins). Holds any
    /// preconfigured settings, credentials in particular.
    overlay: PropertySet,
    /// Whether [`Self::overlay`] contributes any secret (password/token) value, i.e. whether the
    /// caller can omit credentials of its own.
    credentials_loaded: bool,
}

/// Per-session state owned by the request handler.
struct Session {
    input_tx: RdpInputSender,
    input_db: Database,
    destination: String,
    live: Arc<Mutex<Live>>,
    now_endpoint: Arc<NowEndpoint>,
    operations: OperationManager,
}

/// Per-session state shared with the output-consumer task.
struct Live {
    /// Live property bag, seeded from `Config::properties` and updated on (re)negotiation.
    properties: PropertySet,
    state: ConnState,
    error: Option<String>,
    /// Most recent frame (with the cursor already composited in by the session). Replaced on every
    /// graphics update; `None` until the first frame arrives.
    frame: Option<Frame>,
}

/// A decoded frame retained for screenshots. `pixels` are `0x00RRGGBB` (`to_be_bytes()` yields
/// `[0, R, G, B]`), row-major, `width * height` entries, with the remote cursor blended in.
struct Frame {
    width: u16,
    height: u16,
    pixels: Vec<u32>,
}

impl Daemon {
    fn new(logs: Arc<LogBuffer>, overlay: PropertySet) -> Self {
        // Credentials are considered "loaded" when the overlay provides at least one secret value,
        // which is what frees the caller from supplying a password.
        let credentials_loaded = overlay.iter().any(|(key, _)| ironrdp_cfg::is_secret_key(key));
        Self {
            state: Mutex::new(None),
            connect_lock: Mutex::new(()),
            logs,
            overlay,
            credentials_loaded,
        }
    }

    async fn handle(&self, request: Request) -> DaemonResponse {
        match request {
            Request::Connect {
                properties,
                log_directive,
            } => DaemonResponse::Single(self.connect(properties, log_directive)),
            Request::Disconnect => DaemonResponse::Single(self.disconnect()),
            Request::Status => DaemonResponse::Single(self.status()),
            Request::QueryProps { filter } => DaemonResponse::Single(self.query_props(filter.as_ref())),
            Request::QueryLogs { substring, last } => {
                DaemonResponse::Single(self.query_logs(substring.as_deref(), last))
            }
            Request::Screenshot => DaemonResponse::Single(self.screenshot()),
            Request::MouseMove { x, y } => {
                DaemonResponse::Single(self.input(Operation::MouseMove(MousePosition { x, y })))
            }
            Request::MouseButton { button, pressed } => DaemonResponse::Single(self.input(if pressed {
                Operation::MouseButtonPressed(button)
            } else {
                Operation::MouseButtonReleased(button)
            })),
            Request::Wheel { delta, horizontal } => {
                DaemonResponse::Single(self.input(Operation::WheelRotations(WheelRotations {
                    is_vertical: !horizontal,
                    rotation_units: delta,
                })))
            }
            Request::KeyScancode { scancode, pressed } => {
                let scancode = Scancode::from_u16(scancode);
                DaemonResponse::Single(self.input(if pressed {
                    Operation::KeyPressed(scancode)
                } else {
                    Operation::KeyReleased(scancode)
                }))
            }
            Request::KeyUnicode { ch, pressed } => DaemonResponse::Single(self.input(if pressed {
                Operation::UnicodeKeyPressed(ch)
            } else {
                Operation::UnicodeKeyReleased(ch)
            })),
            Request::Resize { width, height } => DaemonResponse::Single(self.resize(width, height)),
            Request::NowCapabilities => DaemonResponse::Single(self.now_capabilities().await),
            Request::NowRun { command, directory } => DaemonResponse::Single(self.now_run(command, directory).await),
            Request::NowExecute(request) => self.now_execute(request).await,
            Request::NowCancel { operation_id } => DaemonResponse::Single(self.now_cancel(operation_id).await),
            Request::NowList => DaemonResponse::Single(self.now_list()),
            Request::NowStatus { operation_id } => DaemonResponse::Single(self.now_status(operation_id)),
            Request::NowAttach {
                operation_id,
                after_sequence,
            } => self.now_attach(operation_id, after_sequence),
            Request::NowStdin {
                operation_id,
                data,
                last,
            } => DaemonResponse::Single(self.now_stdin(operation_id, data, last).await),
            Request::NowDiagnostics => DaemonResponse::Single(self.now_diagnostics().await),
        }
    }

    fn connect(&self, mut properties: PropertySet, log_directive: Option<String>) -> Response {
        let _connect_guard = self.connect_lock.lock().expect("connect state poisoned");
        debug!(?log_directive, "Received connect request");
        // Refuse to clobber a live session: the previous RDP engine runs on its own thread and is
        // not torn down by simply replacing the session slot. Require an explicit `disconnect` first.
        {
            let guard = self.state.lock().expect("daemon state poisoned");
            if let Some(session) = guard.as_ref() {
                let state = session.live.lock().expect("session live state poisoned").state;
                if matches!(
                    state,
                    ConnState::Connecting | ConnState::Connected | ConnState::Disconnecting
                ) {
                    debug!("Refusing connect: a session is already active");
                    return Response::typed_error(
                        crate::ipc::AgentErrorCategory::Conflict,
                        "a session is already active; disconnect first",
                    );
                }
            }
        }

        // Layer the operator-provided overlay on top (overlay wins), so any setting — credentials
        // in particular — can be preconfigured without the (possibly untrusted) caller supplying it.
        properties.merge(&self.overlay);

        // Each RDP session receives a distinct DVC endpoint. It is only contacted later by a NOW
        // request, after the RDP engine has connected and the proxy has opened its local listener.
        let now_endpoint = match NowEndpoint::new() {
            Ok(endpoint) => Arc::new(endpoint),
            Err(error) => {
                return Response::typed_error(
                    crate::ipc::AgentErrorCategory::Internal,
                    format!("failed to allocate NOW endpoint: {error}"),
                );
            }
        };
        let builder = match ConfigBuilder::from_property_set(&properties) {
            Ok(builder) => builder,
            Err(error) => {
                return Response::typed_error(
                    crate::ipc::AgentErrorCategory::InvalidRequest,
                    format!("invalid configuration: {error:#}"),
                );
            }
        };

        // Derive the headless client identity. These fields are never representable as `.rdp`
        // properties and are never prompted; the daemon supplies them itself.
        let builder = builder
            .with_client_build(client_build())
            .with_client_dir("C:\\Windows\\System32\\mstscax.dll")
            .with_platform(current_platform())
            .with_client_name(client_name())
            .with_dvc_pipe_proxy(now_endpoint.dvc_proxy_info())
            // Headless: composite the remote cursor into the framebuffer so it appears in
            // screenshots (there is no separate overlay to draw it).
            .with_pointer_software_rendering(true);

        let missing = builder.missing();
        if !missing.is_empty() {
            return Response::typed_error(
                crate::ipc::AgentErrorCategory::InvalidRequest,
                format!(
                    "missing required fields: {}",
                    missing
                        .iter()
                        .map(MissingField::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }

        let config = match builder.build() {
            Ok(config) => config,
            Err(error) => {
                return Response::typed_error(crate::ipc::AgentErrorCategory::InvalidRequest, format!("{error:#}"));
            }
        };

        // `ConfigBuilder::build` strips every secret property, so the live bag carries no secrets.
        let live_seed = config.properties().clone();
        let destination = config.destination().to_string();

        let (output_tx, output_rx) = mpsc::channel(16);
        let client = RdpClient::new(config, output_tx);
        let input_tx = client.input_sender();

        let live = Arc::new(Mutex::new(Live {
            properties: live_seed,
            state: ConnState::Connecting,
            error: None,
            frame: None,
        }));

        // Capture this session's logs into the ring buffer (queryable via `Request::QueryLogs`)
        // instead of the daemon's terminal, refined by the caller-supplied directive. The dispatch
        // is installed as the session thread's thread-local default below.
        let dispatch = logbuf::session_dispatch(Arc::clone(&self.logs), log_directive.as_deref());

        // The RDP client engine runs on its own thread with a current-thread runtime, mirroring
        // `ironrdp-viewer`. This sidesteps any `Send` requirement on the connection future.
        let spawn_result = std::thread::Builder::new()
            .name("ironrdp-agent-session".to_owned())
            .spawn(move || {
                tracing::dispatcher::with_default(&dispatch, || {
                    match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                        Ok(runtime) => runtime.block_on(client.run()),
                        Err(error) => error!(%error, "Failed to build the session runtime"),
                    }
                });
            });
        if let Err(error) = spawn_result {
            return Response::typed_error(
                crate::ipc::AgentErrorCategory::Internal,
                format!("failed to spawn session thread: {error}"),
            );
        }

        tokio::spawn(consume_output(output_rx, Arc::clone(&live)));

        info!(%destination, "Started RDP session");

        *self.state.lock().expect("daemon state poisoned") = Some(Session {
            input_tx,
            input_db: Database::new(),
            destination,
            live,
            operations: OperationManager::new(Arc::clone(&now_endpoint)),
            now_endpoint,
        });

        Response::ok()
    }

    fn disconnect(&self) -> Response {
        let mut guard = self.state.lock().expect("daemon state poisoned");
        match guard.as_mut() {
            None => {
                debug!("Disconnect requested but no session is active");
                Response::typed_error(crate::ipc::AgentErrorCategory::Unavailable, "no active session")
            }
            Some(session) => {
                let mut live = session.live.lock().expect("session live state poisoned");
                match live.state {
                    ConnState::Connecting | ConnState::Connected => {
                        info!(destination = %session.destination, "Disconnecting RDP session");
                        // Request a graceful shutdown and move to `Disconnecting`. The engine thread
                        // keeps running until it drains the close; `consume_output` flips the state
                        // to a terminal one once it does, which is what re-enables `connect`. Leaving
                        // it `Connected` here would let a new `connect` race the still-live thread.
                        session.input_tx.request_close();
                        live.state = ConnState::Disconnecting;
                        Response::ok()
                    }
                    // Already shutting down or terminated: nothing to do (idempotent).
                    _ => Response::ok(),
                }
            }
        }
    }

    fn status(&self) -> Response {
        let guard = self.state.lock().expect("daemon state poisoned");
        let info = match guard.as_ref() {
            None => StatusInfo {
                state: ConnState::NoSession,
                destination: None,
                width: None,
                height: None,
                message: None,
                credentials_loaded: self.credentials_loaded,
            },
            Some(session) => {
                let live = session.live.lock().expect("session live state poisoned");
                let (width, height) = match &live.frame {
                    Some(frame) => (Some(frame.width), Some(frame.height)),
                    None => (None, None),
                };
                StatusInfo {
                    state: live.state,
                    destination: Some(session.destination.clone()),
                    width,
                    height,
                    message: live.error.clone(),
                    credentials_loaded: self.credentials_loaded,
                }
            }
        };
        Response::Ok(Payload::Status(info))
    }

    fn query_props(&self, filter: Option<&KeyFilter>) -> Response {
        let guard = self.state.lock().expect("daemon state poisoned");
        let Some(session) = guard.as_ref() else {
            return Response::typed_error(crate::ipc::AgentErrorCategory::Unavailable, "no active session");
        };
        let live = session.live.lock().expect("session live state poisoned");

        let mut entries = Vec::new();
        for (key, value) in live.properties.iter() {
            let key = key.as_ref();
            if filter.is_some_and(|filter| !filter.matches(key)) {
                continue;
            }
            let value = match value {
                Value::Int(value) => PropValue::Int(*value),
                Value::Str(value) => PropValue::Str(value.clone()),
            };
            entries.push(PropertyEntry {
                key: key.to_owned(),
                value,
            });
        }

        Response::Ok(Payload::Properties(PropertyDump { entries }))
    }

    fn query_logs(&self, substring: Option<&str>, last: Option<u32>) -> Response {
        let mut lines = self.logs.query(substring);
        if let Some(last) = last {
            let last = usize::try_from(last).unwrap_or(usize::MAX);
            if last < lines.len() {
                lines.drain(0..lines.len() - last);
            }
        }
        Response::Ok(Payload::Logs(lines))
    }

    fn screenshot(&self) -> Response {
        let guard = self.state.lock().expect("daemon state poisoned");
        let Some(session) = guard.as_ref() else {
            return Response::typed_error(crate::ipc::AgentErrorCategory::Unavailable, "no active session");
        };
        let live = session.live.lock().expect("session live state poisoned");
        let Some(frame) = live.frame.as_ref() else {
            return Response::typed_error(crate::ipc::AgentErrorCategory::Unavailable, "no frame available yet");
        };
        match encode_png(frame.width, frame.height, &frame.pixels) {
            Ok(png) => {
                debug!(
                    width = frame.width,
                    height = frame.height,
                    bytes = png.len(),
                    "Encoded screenshot"
                );
                Response::Ok(Payload::Screenshot {
                    width: frame.width,
                    height: frame.height,
                    png,
                })
            }
            Err(error) => Response::typed_error(
                crate::ipc::AgentErrorCategory::Internal,
                format!("failed to encode screenshot: {error:#}"),
            ),
        }
    }

    fn resize(&self, width: u16, height: u16) -> Response {
        if width == 0 || height == 0 {
            return Response::typed_error(
                crate::ipc::AgentErrorCategory::InvalidRequest,
                "width and height must be non-zero",
            );
        }
        let guard = self.state.lock().expect("daemon state poisoned");
        let Some(session) = guard.as_ref() else {
            return Response::typed_error(crate::ipc::AgentErrorCategory::Unavailable, "no active session");
        };
        match session.input_tx.try_send(RdpInputEvent::Resize {
            width,
            height,
            // No window/DPI concept in a headless agent: request the plain pixel size unscaled.
            scale_factor: 100,
            physical_size: None,
        }) {
            Ok(()) => Response::ok(),
            Err(_) => Response::typed_error(
                crate::ipc::AgentErrorCategory::Unavailable,
                "session input channel is closed",
            ),
        }
    }

    fn input(&self, operation: Operation) -> Response {
        let mut guard = self.state.lock().expect("daemon state poisoned");
        let Some(session) = guard.as_mut() else {
            return Response::typed_error(crate::ipc::AgentErrorCategory::Unavailable, "no active session");
        };
        let permit = match session.input_tx.try_reserve() {
            Ok(permit) => permit,
            Err(_) => {
                return Response::typed_error(
                    crate::ipc::AgentErrorCategory::Unavailable,
                    "session input channel is unavailable",
                );
            }
        };
        let events = session.input_db.apply([operation]);
        if events.is_empty() {
            return Response::ok();
        }
        permit.send(RdpInputEvent::FastPath(events));
        Response::ok()
    }

    fn operations(&self) -> Result<OperationManager, Response> {
        self.state
            .lock()
            .expect("daemon state poisoned")
            .as_ref()
            .map(|session| session.operations.clone())
            .ok_or_else(|| Response::typed_error(crate::ipc::AgentErrorCategory::Unavailable, "no active RDP session"))
    }

    async fn now_capabilities(&self) -> Response {
        let operations = match self.operations() {
            Ok(operations) => operations,
            Err(response) => return response,
        };
        match operations.capabilities().await {
            Ok(capabilities) => Response::Ok(Payload::NowCapabilities(capabilities)),
            Err(error) => Response::Err(error),
        }
    }

    async fn now_run(&self, command: String, directory: Option<String>) -> Response {
        let operations = match self.operations() {
            Ok(operations) => operations,
            Err(response) => return response,
        };
        match operations.run(command, directory).await {
            Ok(()) => Response::ok(),
            Err(error) => Response::Err(error),
        }
    }

    async fn now_execute(&self, request: crate::ipc::NowExecutionRequest) -> DaemonResponse {
        let operations = match self.operations() {
            Ok(operations) => operations,
            Err(response) => return DaemonResponse::Single(response),
        };
        match operations.execute(request).await {
            Ok(info) if info.detached => DaemonResponse::Single(Response::Ok(Payload::NowOperation(info))),
            Ok(info) => match operations.attach(info.id, None) {
                Ok(attachment) => DaemonResponse::Stream(Response::Ok(Payload::NowOperation(info)), attachment),
                Err(error) => DaemonResponse::Single(Response::Err(error)),
            },
            Err(error) => DaemonResponse::Single(Response::Err(error)),
        }
    }

    async fn now_cancel(&self, operation_id: u64) -> Response {
        let operations = match self.operations() {
            Ok(operations) => operations,
            Err(response) => return response,
        };
        match operations.cancel(operation_id).await {
            Ok(()) => Response::ok(),
            Err(error) => Response::Err(error),
        }
    }

    fn now_list(&self) -> Response {
        match self.operations() {
            Ok(operations) => Response::Ok(Payload::NowOperations(operations.list())),
            Err(response) => response,
        }
    }

    fn now_status(&self, operation_id: u64) -> Response {
        match self.operations().and_then(|operations| {
            operations
                .status(operation_id)
                .map(|operation| Response::Ok(Payload::NowOperation(operation)))
                .map_err(Response::Err)
        }) {
            Ok(response) | Err(response) => response,
        }
    }

    fn now_attach(&self, operation_id: u64, after_sequence: Option<u64>) -> DaemonResponse {
        match self.operations().and_then(|operations| {
            operations
                .attach(operation_id, after_sequence)
                .map(|attachment| (attachment.info.clone(), attachment))
                .map_err(Response::Err)
        }) {
            Ok((info, attachment)) => DaemonResponse::Stream(Response::Ok(Payload::NowOperation(info)), attachment),
            Err(response) => DaemonResponse::Single(response),
        }
    }

    async fn now_stdin(&self, operation_id: u64, data: Vec<u8>, last: bool) -> Response {
        let operations = match self.operations() {
            Ok(operations) => operations,
            Err(response) => return response,
        };
        match operations.send_stdin(operation_id, data, last).await {
            Ok(()) => Response::ok(),
            Err(error) => Response::Err(error),
        }
    }

    async fn now_diagnostics(&self) -> Response {
        let endpoint = match self
            .state
            .lock()
            .expect("daemon state poisoned")
            .as_ref()
            .map(|session| Arc::clone(&session.now_endpoint))
        {
            Some(endpoint) => endpoint,
            None => {
                return Response::typed_error(crate::ipc::AgentErrorCategory::Unavailable, "no active RDP session");
            }
        };
        let (connected, capabilities) = endpoint.diagnostic_snapshot().await;
        let capabilities = capabilities.map(|capabilities| NowDiagnostics {
            endpoint_allocated: true,
            connected,
            capabilities: Some(crate::ipc::NowCapabilities {
                version_major: capabilities.version_major,
                version_minor: capabilities.version_minor,
                heartbeat_ms: capabilities.heartbeat_ms,
                run: capabilities.run,
                process: capabilities.process,
                batch: capabilities.batch,
                powershell: capabilities.powershell,
                pwsh: capabilities.pwsh,
                io_redirection: capabilities.io_redirection,
                unicode_console: capabilities.unicode_console,
            }),
        });
        Response::Ok(Payload::NowDiagnostics(capabilities.unwrap_or(NowDiagnostics {
            endpoint_allocated: true,
            connected,
            capabilities: None,
        })))
    }
}

/// Consumes the bounded output-event stream, keeping the live state current.
async fn consume_output(mut output_rx: mpsc::Receiver<RdpOutputEvent>, live: Arc<Mutex<Live>>) {
    while let Some(event) = output_rx.recv().await {
        let mut guard = live.lock().expect("session live state poisoned");
        let previous = guard.state;
        match event {
            RdpOutputEvent::Connected => {
                guard.state = ConnState::Connected;
                guard.error = None;
                if previous != ConnState::Connected {
                    info!("Session connected");
                }
            }
            RdpOutputEvent::LoginComplete => {}
            RdpOutputEvent::Image { buffer, width, height } => {
                let width = width.get();
                let height = height.get();
                guard.properties.insert("desktopwidth", width);
                guard.properties.insert("desktopheight", height);
                guard.frame = Some(Frame {
                    width,
                    height,
                    pixels: buffer,
                });
                guard.state = ConnState::Connected;
                guard.error = None;
                if previous != ConnState::Connected {
                    info!(width, height, "Session connected");
                }
            }
            RdpOutputEvent::ConnectionFailure(error) => {
                guard.state = ConnState::Failed;
                guard.error = Some(format!("{error}"));
                error!(%error, "Session connection failed");
            }
            RdpOutputEvent::Terminated(Ok(reason)) => {
                guard.state = ConnState::Disconnected;
                guard.error = Some(format!("{reason:?}"));
                info!(?reason, "Session terminated");
            }
            RdpOutputEvent::Terminated(Err(error)) => {
                guard.state = ConnState::Failed;
                guard.error = Some(format!("{error}"));
                warn!(%error, "Session terminated with an error");
            }
            // With software pointer rendering the cursor is composited into the `Image` frames
            // above; the remaining pointer events (default/hidden) carry no live state we track.
            _ => {}
        }
    }

    // The engine thread has ended (channel closed). Resolve any transient state so a subsequent
    // `connect` is not blocked indefinitely, even if no explicit `Terminated` event was emitted.
    let mut guard = live.lock().expect("session live state poisoned");
    if matches!(
        guard.state,
        ConnState::Connecting | ConnState::Connected | ConnState::Disconnecting
    ) {
        guard.state = ConnState::Disconnected;
    }
}

/// Encodes a retained framebuffer to PNG bytes.
///
/// `pixels` are `0x00RRGGBB` (`to_be_bytes()` yields `[0, R, G, B]`); the leading byte is the unused
/// alpha placeholder, so we emit opaque 8-bit RGB.
fn encode_png(width: u16, height: u16, pixels: &[u32]) -> anyhow::Result<Vec<u8>> {
    let mut rgb = Vec::with_capacity(pixels.len() * 3 /* RGB */);
    for pixel in pixels {
        let [_, r, g, b] = pixel.to_be_bytes();
        rgb.extend_from_slice(&[r, g, b]);
    }

    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, u32::from(width), u32::from(height));
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().context("write PNG header")?;
    writer.write_image_data(&rgb).context("write PNG image data")?;
    writer.finish().context("finish PNG stream")?;
    Ok(png)
}

/// Installs the daemon's global tracing subscriber: a compact formatter to stderr, defaulting to
/// `INFO` and tunable via `IRONRDP_LOG`.
///
/// This is the daemon's *own* operational logging (IPC handling, lifecycle), mirroring
/// `ironrdp-viewer` but quieter by default. The RDP session's logs are captured separately into a
/// ring buffer (see [`logbuf::session_dispatch`]). Best-effort: a no-op if a global subscriber is
/// already set.
fn init_daemon_logging() {
    use tracing::level_filters::LevelFilter;
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .with_env_var("IRONRDP_LOG")
        .from_env_lossy();

    let fmt_layer = tracing_subscriber::fmt::layer().compact().with_writer(std::io::stderr);

    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .try_init();
}

/// Derives a build number from the crate version (`major*100 + minor*10 + patch`).
fn client_build() -> u32 {
    let mut parts = env!("CARGO_PKG_VERSION")
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);
    major
        .saturating_mul(100)
        .saturating_add(minor.saturating_mul(10))
        .saturating_add(patch)
}

fn client_name() -> String {
    whoami::hostname().unwrap_or_else(|_| "ironrdp-agent".to_owned())
}

fn current_platform() -> MajorPlatformType {
    match whoami::platform() {
        whoami::Platform::Windows => MajorPlatformType::WINDOWS,
        whoami::Platform::Linux => MajorPlatformType::UNIX,
        whoami::Platform::Mac => MajorPlatformType::MACINTOSH,
        whoami::Platform::Ios => MajorPlatformType::IOS,
        whoami::Platform::Android => MajorPlatformType::ANDROID,
        _ => MajorPlatformType::UNSPECIFIED,
    }
}
