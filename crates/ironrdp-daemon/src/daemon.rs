//! The long-lived daemon: owns the [`RdpClient`] engine and one RDP session, and serves IPC
//! requests until shut down.
//!
//! One daemon serves one RDP session (multi-session is out of scope for V1). It is started
//! explicitly with `daemon-start` and runs in the foreground; the caller is expected to background
//! it. On a clean shutdown the Unix socket file is removed (see [`crate::transport`]).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use ironrdp_client::config::{ConfigBuilder, MissingField};
use ironrdp_client::rdp::{RdpClient, RdpInputEvent, RdpInputSender, RdpOutputEvent};
use ironrdp_input::{Database, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_propertyset::{PropertySet, Value};
use ironrdp_tls::CertificateValidation;
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};

#[cfg(windows)]
use std::collections::BTreeSet;

#[cfg(windows)]
use ironrdp_rdpdr_native::{RedirectedDrive, WindowsRdpdrBackendFactory};

use crate::ipc::{
    ConnState, KeyFilter, MAX_UNICODE_TEXT_CHARS, NowDiagnostics, Payload, PropValue, PropertyDump, PropertyEntry,
    Request, Response, StatusInfo,
};
use crate::logbuf::{self, LogBuffer};
use crate::now::NowEndpoint;
use crate::operations::{OperationAttachment, OperationManager};
use crate::transport::{Endpoint, Listener, read_message, write_message};

/// Binds the IPC endpoint and serves requests until a shutdown signal is received.
///
/// `overlay` is an operator-provided [`PropertySet`] layered on top of every `Connect` request
/// (overlay wins), so any setting — credentials in particular — can be preconfigured without the
/// caller ever supplying it.
pub async fn run(endpoint: Endpoint, overlay: PropertySet, options: DaemonOptions) -> anyhow::Result<()> {
    init_daemon_logging();
    let daemon = Arc::new(Daemon::with_options(overlay, options)?);
    serve(endpoint, daemon).await
}

/// Serves `daemon` on `endpoint` until its owner requests shutdown.
///
/// The caller owns the daemon so it can share the same session state with another frontend, such
/// as the viewer window.
pub async fn serve(endpoint: Endpoint, daemon: Arc<Daemon>) -> anyhow::Result<()> {
    crate::transport::prepare_endpoint(&endpoint).await?;
    let mut listener = Listener::bind(&endpoint).with_context(|| format!("bind IPC endpoint {endpoint}"))?;
    info!(%endpoint, "Daemon listening");
    let mut shutdown = daemon.shutdown_receiver();
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
            result = shutdown.changed() => {
                result.context("wait for shutdown signal")?;
                info!("Received shutdown request, stopping");
                break;
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

/// The reason a resize could not be queued for the active RDP session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeError {
    /// The requested dimensions are invalid.
    InvalidDimensions,
    /// No RDP session is active.
    NoSession,
    /// The bounded RDP input channel is temporarily full.
    Full,
    /// The active RDP session has stopped accepting input.
    Closed,
}

/// SHA-256 fingerprint accepted only when normal certificate validation fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CertificateSha256([u8; 32]);

impl CertificateSha256 {
    fn matches(self, certificate: &[u8]) -> bool {
        let actual: [u8; 32] = Sha256::digest(certificate).into();
        actual == self.0
    }
}

impl core::str::FromStr for CertificateSha256 {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let normalized: String = input
            .chars()
            .filter(|character| !matches!(character, ':' | '-'))
            .collect();
        if normalized.len() != 64 || !normalized.is_ascii() {
            return Err("certificate SHA-256 fingerprint must contain 64 hexadecimal characters".to_owned());
        }

        let mut bytes = [0; 32];
        for (hex, byte) in normalized.as_bytes().chunks_exact(2).zip(&mut bytes) {
            let hex = core::str::from_utf8(hex)
                .map_err(|_| "certificate SHA-256 fingerprint must be hexadecimal".to_owned())?;
            *byte = u8::from_str_radix(hex, 16)
                .map_err(|_| "certificate SHA-256 fingerprint must be hexadecimal".to_owned())?;
        }
        Ok(Self(bytes))
    }
}

/// Windows volume definition exposed as one static RDPDR filesystem drive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RdpdrDriveConfig {
    root_path: PathBuf,
    display_name: String,
}

impl RdpdrDriveConfig {
    /// Creates a filesystem-drive definition.
    pub fn new(root_path: PathBuf, display_name: String) -> anyhow::Result<Self> {
        if root_path.as_os_str().is_empty() {
            anyhow::bail!("rdpdr volume root must not be empty");
        }
        if display_name.is_empty() {
            anyhow::bail!("rdpdr drive name must not be empty");
        }
        if display_name.len() > 7 || !display_name.is_ascii() {
            anyhow::bail!("rdpdr drive name must contain at most seven ASCII characters");
        }
        if !display_name.chars().enumerate().all(|(index, character)| {
            character.is_ascii_alphanumeric()
                || matches!(character, ' ' | '_' | '-' | '.')
                || (character == ':' && index + 1 == display_name.len())
        }) {
            anyhow::bail!("rdpdr drive name contains an invalid DOS device-name character");
        }

        Ok(Self {
            root_path,
            display_name,
        })
    }

    /// Returns the volume root selected for redirection.
    #[must_use]
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    /// Returns the protocol-visible drive name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }
}

/// Startup-only settings that are deliberately unavailable through session IPC.
#[derive(Clone, Debug, Default)]
pub struct DaemonOptions {
    certificate_pin: Option<CertificateSha256>,
    rdpdr_drives: Vec<RdpdrDriveConfig>,
}

impl DaemonOptions {
    /// Accepts a known leaf certificate only when normal strict validation fails.
    #[must_use]
    pub fn with_certificate_pin(mut self, certificate_pin: Option<CertificateSha256>) -> Self {
        self.certificate_pin = certificate_pin;
        self
    }

    /// Configures fixed local volumes for filesystem redirection.
    #[must_use]
    pub fn with_rdpdr_drives(mut self, rdpdr_drives: Vec<RdpdrDriveConfig>) -> Self {
        self.rdpdr_drives = rdpdr_drives;
        self
    }
}

/// The daemon's mutable state: the (single) current session, plus the shared log buffer.
pub struct Daemon {
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
    certificate_pin: Option<CertificateSha256>,
    #[cfg(windows)]
    rdpdr_backend_factory: Option<WindowsRdpdrBackendFactory>,
    /// Notifies an optional GUI frontend whenever retained live state changes.
    notification: Option<mpsc::Sender<()>>,
    shutdown: tokio::sync::watch::Sender<()>,
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
#[derive(Clone)]
pub struct Frame {
    pub width: u16,
    pub height: u16,
    pub pixels: Vec<u32>,
}

impl Daemon {
    fn new(logs: Arc<LogBuffer>, overlay: PropertySet, options: DaemonOptions) -> anyhow::Result<Self> {
        // Credentials are considered "loaded" when the overlay provides at least one secret value,
        // which is what frees the caller from supplying a password.
        let credentials_loaded = overlay.iter().any(|(key, _)| ironrdp_cfg::is_secret_key(key));
        let (shutdown, _) = tokio::sync::watch::channel(());
        let DaemonOptions {
            certificate_pin,
            rdpdr_drives,
        } = options;
        #[cfg(windows)]
        let rdpdr_backend_factory = rdpdr_backend_factory(rdpdr_drives)?;
        #[cfg(not(windows))]
        if !rdpdr_drives.is_empty() {
            anyhow::bail!("rdpdr filesystem redirection is only available on Windows");
        }

        Ok(Self {
            state: Mutex::new(None),
            connect_lock: Mutex::new(()),
            logs,
            overlay,
            credentials_loaded,
            certificate_pin,
            #[cfg(windows)]
            rdpdr_backend_factory,
            notification: None,
            shutdown,
        })
    }

    /// Creates a daemon with no preconfigured connection properties.
    ///
    /// # Panics
    ///
    /// Panics if the default daemon configuration becomes invalid.
    pub fn with_overlay(overlay: PropertySet) -> Self {
        Self::with_options(overlay, DaemonOptions::default()).expect("default daemon configuration is valid")
    }

    /// Creates a daemon with startup-only settings that are not available to IPC callers.
    pub fn with_options(overlay: PropertySet, options: DaemonOptions) -> anyhow::Result<Self> {
        Self::new(LogBuffer::new(), overlay, options)
    }

    /// Creates a daemon with fixed Windows filesystem drives for every connection.
    pub fn with_rdpdr_drives(overlay: PropertySet, rdpdr_drives: Vec<RdpdrDriveConfig>) -> anyhow::Result<Self> {
        Self::with_options(overlay, DaemonOptions::default().with_rdpdr_drives(rdpdr_drives))
    }

    /// Adds a capacity-one notification channel for frontends that render retained live state.
    ///
    /// The caller must provide a channel with capacity one. Notifications are coalesced because a
    /// frontend always reads the latest retained framebuffer.
    #[must_use]
    pub fn with_notification(mut self, notification: mpsc::Sender<()>) -> Self {
        self.notification = Some(notification);
        self
    }

    /// Returns a receiver that is notified when the server should stop.
    pub fn shutdown_receiver(&self) -> tokio::sync::watch::Receiver<()> {
        self.shutdown.subscribe()
    }

    /// Returns the latest cursor-composited framebuffer, if one is available.
    ///
    /// # Panics
    ///
    /// Panics if the daemon or session state mutex is poisoned.
    pub fn current_frame(&self) -> Option<Frame> {
        let guard = self.state.lock().expect("daemon state poisoned");
        guard
            .as_ref()
            .and_then(|session| session.live.lock().expect("session live state poisoned").frame.clone())
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
            Request::UnicodeText { text } => DaemonResponse::Single(self.unicode_text(&text)),
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
        let certificate_validation = match properties.get::<&str>("ironrdp_certificate_validation") {
            None | Some("strict") => CertificateValidation::Strict,
            Some(value) => {
                return Response::typed_error(
                    crate::ipc::AgentErrorCategory::InvalidRequest,
                    format!("invalid certificate validation policy '{value}'; agent sessions require 'strict'"),
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
            .with_certificate_validation(certificate_validation)
            // Headless: composite the remote cursor into the framebuffer so it appears in
            // screenshots (there is no separate overlay to draw it).
            .with_pointer_software_rendering(true);
        let builder = match self.certificate_pin {
            Some(certificate_pin) => {
                let callback: ironrdp_tls::CertificateValidationCallback =
                    Arc::new(move |certificate, _reason| certificate_pin.matches(certificate));
                builder.with_certificate_validation_callback(callback)
            }
            None => builder,
        };
        #[cfg(windows)]
        let builder = if self.rdpdr_backend_factory.is_some() {
            builder.with_rdpdr(true)
        } else {
            builder
        };

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
        #[cfg(windows)]
        let client = match &self.rdpdr_backend_factory {
            Some(factory) => client.with_rdpdr_backend_factory(Box::new(factory.clone())),
            None => client,
        };
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

        tokio::spawn(consume_output(output_rx, Arc::clone(&live), self.notification.clone()));

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

    /// Tears down the active RDP session.
    ///
    /// # Panics
    ///
    /// Panics if the daemon state mutex is poisoned.
    pub fn disconnect(&self) -> Response {
        let mut guard = self.state.lock().expect("daemon state poisoned");
        match guard.as_mut() {
            None => {
                debug!("Disconnect requested but no session is active");
                Response::typed_error(crate::ipc::AgentErrorCategory::Unavailable, "no active session")
            }
            Some(session) => {
                let mut live = session.live.lock().expect("session live state poisoned");
                match live.state {
                    ConnState::Connecting => {
                        info!(destination = %session.destination, "Cancelling RDP connection");
                        session.input_tx.request_close();
                        live.state = ConnState::Disconnecting;
                        Response::ok()
                    }
                    ConnState::Connected => {
                        info!(destination = %session.destination, "Disconnecting RDP session");
                        session.input_tx.request_graceful_close();
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

    /// Requests that the active RDP session resize.
    pub fn resize(&self, width: u16, height: u16) -> Response {
        match self.try_resize(width, height) {
            Ok(()) => Response::ok(),
            Err(ResizeError::InvalidDimensions) => Response::typed_error(
                crate::ipc::AgentErrorCategory::InvalidRequest,
                "width and height must be non-zero",
            ),
            Err(ResizeError::NoSession) => {
                Response::typed_error(crate::ipc::AgentErrorCategory::Unavailable, "no active session")
            }
            Err(ResizeError::Closed) => Response::typed_error(
                crate::ipc::AgentErrorCategory::Unavailable,
                "session input channel is unavailable",
            ),
            Err(ResizeError::Full) => Response::typed_error(
                crate::ipc::AgentErrorCategory::Unavailable,
                "session input channel is full",
            ),
        }
    }

    /// Attempts to enqueue a resize without flattening temporary input-channel backpressure.
    ///
    /// # Panics
    ///
    /// Panics if the daemon state mutex is poisoned.
    pub fn try_resize(&self, width: u16, height: u16) -> Result<(), ResizeError> {
        if width == 0 || height == 0 {
            return Err(ResizeError::InvalidDimensions);
        }
        let guard = self.state.lock().expect("daemon state poisoned");
        let Some(session) = guard.as_ref() else {
            return Err(ResizeError::NoSession);
        };
        match session.input_tx.try_send(RdpInputEvent::Resize {
            width,
            height,
            // No window/DPI concept in a headless agent: request the plain pixel size unscaled.
            scale_factor: 100,
            physical_size: None,
        }) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err(ResizeError::Full),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(ResizeError::Closed),
        }
    }

    /// Sends an input operation to the active RDP session.
    ///
    /// # Panics
    ///
    /// Panics if the daemon state mutex is poisoned.
    pub fn input(&self, operation: Operation) -> Response {
        self.input_operations([operation])
    }

    fn unicode_text(&self, text: &str) -> Response {
        let char_count = text.chars().count();
        if char_count > MAX_UNICODE_TEXT_CHARS {
            return Response::typed_error(
                crate::ipc::AgentErrorCategory::InvalidRequest,
                format!("text exceeds the {MAX_UNICODE_TEXT_CHARS}-character limit"),
            );
        }

        let mut guard = self.state.lock().expect("daemon state poisoned");
        let Some(session) = guard.as_mut() else {
            return Response::typed_error(crate::ipc::AgentErrorCategory::Unavailable, "no active session");
        };

        // Reserve every queue slot before changing keyboard state. A full queue therefore sends no
        // prefix of the requested text.
        let mut permits = Vec::with_capacity(char_count);
        for _ in 0..char_count {
            let permit = match session.input_tx.try_reserve() {
                Ok(permit) => permit,
                Err(_) => {
                    return Response::typed_error(
                        crate::ipc::AgentErrorCategory::Unavailable,
                        "session input channel is unavailable",
                    );
                }
            };
            permits.push(permit);
        }

        for (permit, ch) in permits.into_iter().zip(text.chars()) {
            let events = session
                .input_db
                .apply([Operation::UnicodeKeyPressed(ch), Operation::UnicodeKeyReleased(ch)]);
            if !events.is_empty() {
                permit.send(RdpInputEvent::FastPath(events));
            }
        }

        Response::ok()
    }

    /// Sends input operations to the active RDP session in one FastPath message.
    ///
    /// # Panics
    ///
    /// Panics if the daemon state mutex is poisoned.
    pub fn input_operations(&self, operations: impl IntoIterator<Item = Operation>) -> Response {
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
        let events = session.input_db.apply(operations);
        if events.is_empty() {
            return Response::ok();
        }
        permit.send(RdpInputEvent::FastPath(events));
        Response::ok()
    }

    /// Stops an in-process RPC server that shares this daemon.
    pub fn shutdown(&self) {
        let _ = self.shutdown.send(());
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
async fn consume_output(
    mut output_rx: mpsc::Receiver<RdpOutputEvent>,
    live: Arc<Mutex<Live>>,
    notification: Option<mpsc::Sender<()>>,
) {
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
        drop(guard);
        notify(&notification);
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
    drop(guard);
    notify(&notification);
}

fn notify(notification: &Option<mpsc::Sender<()>>) {
    // A single queued signal is sufficient: the frontend always reads the latest frame.
    if let Some(notification) = notification {
        let _ = notification.try_send(());
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

#[cfg(windows)]
fn rdpdr_backend_factory(drives: Vec<RdpdrDriveConfig>) -> anyhow::Result<Option<WindowsRdpdrBackendFactory>> {
    if drives.is_empty() {
        return Ok(None);
    }

    let mut names = BTreeSet::new();
    let mut roots = BTreeSet::new();
    let drives = drives
        .iter()
        .enumerate()
        .map(|(index, drive)| {
            if !names.insert(drive.display_name.to_ascii_uppercase()) {
                anyhow::bail!("rdpdr drive names must be unique");
            }
            if !roots.insert(validate_rdpdr_volume_root(&drive.root_path)?) {
                anyhow::bail!("rdpdr volume roots must be unique");
            }

            let device_id =
                u32::try_from(index + 1).map_err(|_| anyhow::anyhow!("too many rdpdr drive configurations"))?;
            RedirectedDrive::new(device_id, &drive.display_name, &drive.root_path, false)
                .map_err(|error| anyhow::anyhow!("invalid rdpdr drive configuration: {error}"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    WindowsRdpdrBackendFactory::from_drives(drives)
        .map(Some)
        .map_err(|error| anyhow::anyhow!("invalid rdpdr drive configuration: {error}"))
}

#[cfg(windows)]
fn validate_rdpdr_volume_root(root_path: &Path) -> anyhow::Result<String> {
    use std::os::windows::ffi::OsStrExt as _;

    let mut path = root_path.as_os_str().encode_wide();
    let is_disk_root = matches!(
        (path.next(), path.next(), path.next(), path.next()),
        (
            Some(drive_letter),
            Some(colon),
            Some(separator),
            None
        ) if matches!(drive_letter, 65..=90 | 97..=122)
            && colon == u16::from(b':')
            && separator == u16::from(b'\\')
    );
    if !is_disk_root {
        anyhow::bail!("rdpdr volume root must use the X:\\ form");
    }

    let metadata =
        std::fs::metadata(root_path).with_context(|| format!("inspect rdpdr volume root {}", root_path.display()))?;
    if !metadata.is_dir() {
        anyhow::bail!("rdpdr volume root must be a directory");
    }

    Ok(std::fs::canonicalize(root_path)
        .with_context(|| format!("canonicalize rdpdr volume root {}", root_path.display()))?
        .to_string_lossy()
        .to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tokio::sync::mpsc;

    use ironrdp_propertyset::PropertySet;

    use super::{CertificateSha256, Daemon, MAX_UNICODE_TEXT_CHARS, RdpdrDriveConfig, ResizeError, notify};
    use crate::ipc::Response;

    #[test]
    fn framebuffer_notifications_are_coalesced() {
        let (sender, mut receiver) = mpsc::channel(1);
        let notification = Some(sender);

        notify(&notification);
        notify(&notification);

        assert_eq!(receiver.try_recv(), Ok(()));
        assert!(matches!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
    }

    #[test]
    fn resize_without_session_reports_no_active_session() {
        let daemon = Daemon::with_overlay(PropertySet::new());

        assert_eq!(daemon.try_resize(1024, 768), Err(ResizeError::NoSession));
        assert!(matches!(daemon.resize(1024, 768), Response::Err(error) if error.message == "no active session"));
    }

    #[test]
    fn certificate_sha256_parses_fingerprints_and_matches_leaf_certificates() {
        let fingerprint =
            "BA:78-16:BF-8F:01-CF:EA-41:41-40:DE-5D:AE-22:23-B0:03-61:A3-96:17-7A:9C-B4:10-FF:61-F2:00-15:AD";
        let fingerprint: CertificateSha256 = fingerprint.parse().expect("valid fingerprint");

        assert!(fingerprint.matches(b"abc"));
        assert!(!fingerprint.matches(b"abcd"));
        assert!("not-a-fingerprint".parse::<CertificateSha256>().is_err());
    }

    #[test]
    fn unicode_text_rejects_requests_that_exceed_the_bounded_queue_capacity() {
        let daemon = Daemon::with_overlay(PropertySet::new());
        let text = "x".repeat(MAX_UNICODE_TEXT_CHARS + 1);

        assert!(
            matches!(daemon.unicode_text(&text), Response::Err(error) if error.message == "text exceeds the 96-character limit")
        );
    }

    #[test]
    fn shutdown_notification_stops_the_server() {
        let daemon = Daemon::with_overlay(PropertySet::new());
        let shutdown = daemon.shutdown_receiver();

        daemon.shutdown();

        assert!(shutdown.has_changed().expect("shutdown sender is live"));
    }

    #[test]
    fn rdpdr_drive_configuration_rejects_invalid_names_and_roots() {
        assert!(RdpdrDriveConfig::new(PathBuf::new(), "Data".to_owned()).is_err());
        assert!(RdpdrDriveConfig::new(PathBuf::from(r"C:\"), String::new()).is_err());
        assert!(RdpdrDriveConfig::new(PathBuf::from(r"C:\"), "too-long".to_owned()).is_err());
        assert!(RdpdrDriveConfig::new(PathBuf::from(r"C:\"), "data/".to_owned()).is_err());
        assert!(RdpdrDriveConfig::new(PathBuf::from(r"C:\"), "Data".to_owned()).is_ok());
    }

    #[cfg(windows)]
    fn system_volume_root() -> PathBuf {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        PathBuf::from(format!(r"{system_drive}\"))
    }

    #[cfg(windows)]
    #[test]
    fn rdpdr_daemon_constructs_a_factory_for_a_volume_root() {
        let drive = RdpdrDriveConfig::new(system_volume_root(), "System".to_owned()).expect("valid drive");
        let daemon = Daemon::with_rdpdr_drives(PropertySet::new(), vec![drive]).expect("valid daemon options");

        assert!(daemon.rdpdr_backend_factory.is_some());
    }

    #[cfg(windows)]
    #[test]
    fn rdpdr_daemon_rejects_duplicate_names_and_roots() {
        let root = system_volume_root();
        let duplicate_name = Daemon::with_rdpdr_drives(
            PropertySet::new(),
            vec![
                RdpdrDriveConfig::new(root.clone(), "System".to_owned()).expect("valid drive"),
                RdpdrDriveConfig::new(root.clone(), "system".to_owned()).expect("valid drive"),
            ],
        );
        assert!(matches!(duplicate_name, Err(error) if error.to_string() == "rdpdr drive names must be unique"));

        let duplicate_root = Daemon::with_rdpdr_drives(
            PropertySet::new(),
            vec![
                RdpdrDriveConfig::new(root.clone(), "System".to_owned()).expect("valid drive"),
                RdpdrDriveConfig::new(root, "Data".to_owned()).expect("valid drive"),
            ],
        );
        assert!(matches!(duplicate_root, Err(error) if error.to_string() == "rdpdr volume roots must be unique"));
    }

    #[cfg(windows)]
    #[test]
    fn rdpdr_daemon_rejects_non_root_directories() {
        let drive = RdpdrDriveConfig::new(std::env::current_dir().expect("current directory"), "Data".to_owned())
            .expect("non-empty drive config");

        let result = Daemon::with_rdpdr_drives(PropertySet::new(), vec![drive]);

        assert!(matches!(result, Err(error) if error.to_string() == "rdpdr volume root must use the X:\\ form"));
    }

    #[cfg(windows)]
    #[test]
    fn rdpdr_daemon_rejects_noncanonical_volume_root_spellings() {
        for root in ["C:/", r"C:\.", r"\\?\C:\"] {
            let drive = RdpdrDriveConfig::new(PathBuf::from(root), "Data".to_owned()).expect("non-empty drive config");

            let result = Daemon::with_rdpdr_drives(PropertySet::new(), vec![drive]);

            assert!(matches!(
                result,
                Err(error) if error.to_string() == "rdpdr volume root must use the X:\\ form"
            ));
        }
    }
}
