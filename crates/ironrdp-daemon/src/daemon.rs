//! The long-lived daemon: owns the [`RdpClient`] engine and one RDP session, and serves IPC
//! requests until shut down.
//!
//! One daemon serves one RDP session (multi-session is out of scope for V1). It is started
//! explicitly with `daemon-start` and runs in the foreground; the caller is expected to background
//! it. On a clean shutdown the Unix socket file is removed (see [`crate::transport`]).

use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use ironrdp_cfg::PropertySetExt as _;
use ironrdp_client::config::{ConfigBuilder, MissingField};
use ironrdp_client::rail::RailControlEvent;
use ironrdp_client::rdp::{
    RailExecuteFailureReason as ClientRailExecuteFailureReason, RdpClient, RdpInputEvent, RdpInputSender,
    RdpOutputEvent,
};
use ironrdp_input::{Database, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_propertyset::{PropertySet, Value};
use ironrdp_rail::pdu::{ExecutePdu, RailPdu};
use ironrdp_tls::CertificateValidation;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};

#[cfg(windows)]
use std::collections::BTreeSet;

#[cfg(windows)]
use ironrdp_rdpdr_native::{RedirectedDrive, WindowsRdpdrBackendFactory};

use crate::ipc::{
    ConnState, KeyFilter, MAX_RAIL_RETAINED_EVENTS, MAX_UNICODE_TEXT_CHARS, NowDiagnostics, Payload, PenFrameRequest,
    PropValue, PropertyDump, PropertyEntry, RailEvent, RailEventDump, RailEventKind, RailExecuteFailureReason,
    RailExecuteRequest, RailLaunchInfo, RailStatusInfo, Request, Response, StatusInfo, TouchFrameRequest,
    pen_event_from_request, touch_event_from_request,
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
    skip_certificate_check: bool,
    rdpdr_drives: Vec<RdpdrDriveConfig>,
    /// Enables Windows WinSCard smartcard redirection when the connect property is unset.
    smartcard: bool,
}

impl DaemonOptions {
    /// Skips TLS certificate and hostname validation for this daemon.
    #[must_use]
    pub fn with_certificate_check_skipped(mut self, skip: bool) -> Self {
        self.skip_certificate_check = skip;
        self
    }

    fn certificate_validation(&self) -> CertificateValidation {
        if self.skip_certificate_check {
            CertificateValidation::DangerouslyAcceptInvalidCertificate
        } else {
            CertificateValidation::Strict
        }
    }

    /// Configures fixed local volumes for filesystem redirection.
    #[must_use]
    pub fn with_rdpdr_drives(mut self, rdpdr_drives: Vec<RdpdrDriveConfig>) -> Self {
        self.rdpdr_drives = rdpdr_drives;
        self
    }

    /// Enables WinSCard smartcard redirection for this daemon (Windows only).
    ///
    /// When `true`, smartcard is used unless a connect/`--prop`/`overlay` value of
    /// `ironrdp_smartcard:i:0` explicitly disables it. Connect-time `ironrdp_smartcard:i:1`
    /// can also enable smartcard without this startup flag.
    #[must_use]
    pub fn with_smartcard(mut self, enabled: bool) -> Self {
        self.smartcard = enabled;
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
    /// Monotonically assigns RAIL ledger generations so observations from a former session cannot be
    /// mistaken for the active connection.
    next_rail_generation: Arc<AtomicU64>,
    /// Assigns RAIL launch IDs across every session so they remain unambiguous with their generation.
    next_rail_launch_id: AtomicU64,
    certificate_validation: CertificateValidation,
    /// Default smartcard enablement when `ironrdp_smartcard` is absent from connect properties.
    smartcard_default: bool,
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
    rail_enabled: bool,
    live: Arc<Mutex<Live>>,
    rail_notify: Arc<tokio::sync::Notify>,
    now_endpoint: Arc<NowEndpoint>,
    operations: OperationManager,
}

fn enqueue_unicode_text(input_tx: &RdpInputSender, input_db: &mut Database, text: &str) -> Response {
    // Reserve every queue slot before changing keyboard state. A full queue therefore sends no
    // prefix of the requested text.
    let mut permits = Vec::with_capacity(text.chars().count());
    for _ in text.chars() {
        let permit = match input_tx.try_reserve() {
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
        let events = input_db.apply([Operation::UnicodeKeyPressed(ch), Operation::UnicodeKeyReleased(ch)]);
        if !events.is_empty() {
            permit.send(RdpInputEvent::FastPath(events));
        }
    }

    Response::ok()
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
    rail_initial_execute: Option<(u16, String)>,
    rail: RailLedger,
}

const MAX_PENDING_RAIL_LAUNCHES: usize = 64;

#[derive(Debug)]
enum RailLaunchQueueError {
    LimitReached,
    DuplicateResponse,
}

/// Session-local RAIL evidence. The daemon records only client-validated outputs; it never reparses
/// static-channel payloads or drawing orders.
struct RailLedger {
    generation: u64,
    next_sequence: u64,
    handshake_complete: bool,
    desktop_synchronized: bool,
    events: VecDeque<RailEvent>,
    pending_launches: VecDeque<PendingRailLaunch>,
}

/// A client Execute request awaiting its server result.
enum PendingRailLaunch {
    /// The Execute request configured before the RDP session becomes active.
    Initial { flags: u16, executable: String },
    /// An Execute request explicitly submitted through the agent IPC API.
    Agent(RailLaunchInfo),
}

impl RailLedger {
    fn new(generation: u64, next_sequence: u64, initial_execute: Option<(u16, String)>) -> Self {
        Self {
            generation,
            next_sequence,
            handshake_complete: false,
            desktop_synchronized: false,
            events: VecDeque::new(),
            pending_launches: initial_execute
                .into_iter()
                .map(|(flags, executable)| PendingRailLaunch::Initial { flags, executable })
                .collect(),
        }
    }

    fn record(&mut self, kind: RailEventKind) {
        let event = RailEvent {
            sequence: self.next_sequence,
            kind,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);
        if self.events.len() == MAX_RAIL_RETAINED_EVENTS {
            let _ = self.events.pop_front();
        }
        self.events.push_back(event);
    }

    fn status(&self) -> RailStatusInfo {
        RailStatusInfo {
            generation: self.generation,
            next_sequence: self.next_sequence,
            handshake_complete: self.handshake_complete,
            desktop_synchronized: self.desktop_synchronized,
            pending_launches: self
                .pending_launches
                .iter()
                .filter_map(|launch| match launch {
                    PendingRailLaunch::Initial { .. } => None,
                    PendingRailLaunch::Agent(launch) => Some(launch.clone()),
                })
                .collect(),
        }
    }

    fn events_after(&self, after_sequence: Option<u64>) -> RailEventDump {
        let after_sequence = after_sequence.unwrap_or(0);
        let mut events = Vec::new();
        if let Some(first) = self.events.front() {
            let lost_through = first.sequence.saturating_sub(1);
            if after_sequence < lost_through {
                events.push(RailEvent {
                    sequence: lost_through,
                    kind: RailEventKind::Gap { lost_through },
                });
            }
        }
        events.extend(
            self.events
                .iter()
                .filter(|event| event.sequence > after_sequence)
                .cloned(),
        );
        RailEventDump {
            generation: self.generation,
            events,
        }
    }

    fn queue_launch(&mut self, launch: RailLaunchInfo) -> Result<(), RailLaunchQueueError> {
        if self.pending_launches.len() >= MAX_PENDING_RAIL_LAUNCHES {
            return Err(RailLaunchQueueError::LimitReached);
        }
        if self.pending_launches.iter().any(|pending| match pending {
            PendingRailLaunch::Initial { flags, executable } => {
                *flags == launch.flags && executable == &launch.executable
            }
            PendingRailLaunch::Agent(pending) => {
                pending.flags == launch.flags && pending.executable == launch.executable
            }
        }) {
            return Err(RailLaunchQueueError::DuplicateResponse);
        }
        self.record(RailEventKind::ExecuteQueued(launch.clone()));
        self.pending_launches.push_back(PendingRailLaunch::Agent(launch));
        Ok(())
    }

    fn take_launch(&mut self, flags: u16, executable: &str) -> Option<u64> {
        let index = self.pending_launches.iter().position(|launch| match launch {
            PendingRailLaunch::Initial {
                flags: initial_flags,
                executable: initial_executable,
            } => *initial_flags == flags && initial_executable == executable,
            PendingRailLaunch::Agent(launch) => launch.flags == flags && launch.executable == executable,
        })?;
        match self.pending_launches.remove(index) {
            Some(PendingRailLaunch::Initial { .. }) | None => None,
            Some(PendingRailLaunch::Agent(launch)) => Some(launch.launch_id),
        }
    }

    fn fail_pending_launches(&mut self) -> bool {
        let pending_launches = core::mem::take(&mut self.pending_launches);
        let mut recorded_failure = false;
        for launch in pending_launches {
            if let PendingRailLaunch::Agent(launch) = launch {
                self.record(RailEventKind::ExecuteFailed {
                    launch_id: Some(launch.launch_id),
                    executable: launch.executable,
                    flags: launch.flags,
                    reason: RailExecuteFailureReason::RailUnavailable,
                });
                recorded_failure = true;
            }
        }
        recorded_failure
    }
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
        let certificate_validation = options.certificate_validation();
        if certificate_validation == CertificateValidation::DangerouslyAcceptInvalidCertificate {
            warn!("TLS certificate and hostname validation are disabled by explicit daemon configuration");
        }
        let DaemonOptions {
            rdpdr_drives,
            smartcard,
            ..
        } = options;
        // Overlay may pre-enable smartcard via `ironrdp_smartcard` without a dedicated CLI flag.
        let smartcard_default = smartcard || overlay.enable_smartcard().unwrap_or(false);
        #[cfg(windows)]
        let rdpdr_backend_factory = rdpdr_backend_factory(rdpdr_drives, smartcard_default)?;
        #[cfg(not(windows))]
        if !rdpdr_drives.is_empty() {
            anyhow::bail!("rdpdr filesystem redirection is only available on Windows");
        }
        #[cfg(not(windows))]
        if smartcard_default {
            anyhow::bail!("smartcard redirection is only available on Windows");
        }

        Ok(Self {
            state: Mutex::new(None),
            connect_lock: Mutex::new(()),
            logs,
            overlay,
            credentials_loaded,
            next_rail_generation: Arc::new(AtomicU64::new(1)),
            next_rail_launch_id: AtomicU64::new(1),
            certificate_validation,
            smartcard_default,
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
            Request::Touch { encode_time, frames } => DaemonResponse::Single(self.touch(encode_time, frames)),
            Request::Pen { encode_time, frames } => DaemonResponse::Single(self.pen(encode_time, frames)),
            Request::DismissHoveringTouchContact { contact_id } => {
                DaemonResponse::Single(self.dismiss_hovering_touch_contact(contact_id))
            }
            Request::RailStatus => DaemonResponse::Single(self.rail_status()),
            Request::RailEvents { after_sequence } => DaemonResponse::Single(self.rail_events(after_sequence)),
            Request::RailWait {
                after_sequence,
                timeout_ms,
            } => DaemonResponse::Single(self.rail_wait(after_sequence, timeout_ms).await),
            Request::RailExecute(request) => DaemonResponse::Single(self.rail_execute(request)),
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
            None | Some("strict") => self.certificate_validation,
            Some(value) => {
                return Response::typed_error(
                    crate::ipc::AgentErrorCategory::InvalidRequest,
                    format!("invalid certificate validation policy '{value}'; configure it when starting the daemon"),
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
            // The headless agent observes validated RAIL state but does not implement local
            // move/size, taskbar, cloak, z-order, or display-power behavior.
            .with_rail_client_status_flags(0)
            // Headless: composite the remote cursor into the framebuffer so it appears in
            // screenshots (there is no separate overlay to draw it).
            .with_pointer_software_rendering(true);
        // Prefer an explicit connect/overlay property; otherwise use the daemon startup default.
        // Always set smartcard explicitly so the client feature default (`true`) cannot announce a
        // smartcard device without a matching WinSCard backend.
        let smartcard = properties.enable_smartcard().unwrap_or(self.smartcard_default);
        #[cfg(windows)]
        let rdpdr_factory = match resolve_rdpdr_factory(self.rdpdr_backend_factory.as_ref(), smartcard) {
            Ok(factory) => factory,
            Err(error) => {
                return Response::typed_error(
                    crate::ipc::AgentErrorCategory::Internal,
                    format!("invalid rdpdr configuration: {error:#}"),
                );
            }
        };
        #[cfg(windows)]
        let builder = if rdpdr_factory.is_some() {
            builder.with_rdpdr(true).with_smartcard(smartcard)
        } else {
            builder.with_smartcard(false)
        };
        #[cfg(not(windows))]
        let builder = if smartcard {
            return Response::typed_error(
                crate::ipc::AgentErrorCategory::InvalidRequest,
                "smartcard redirection is only available on Windows",
            );
        } else {
            // Client feature defaults smartcard on; never announce without a WinSCard backend.
            builder.with_smartcard(false)
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
        let rail_enabled = live_seed.remote_application_mode().unwrap_or(false);
        let initial_rail_execute = if rail_enabled {
            live_seed
                .remote_application_program()
                .filter(|program| !program.is_empty())
                .or_else(|| live_seed.alternate_shell().filter(|shell| !shell.is_empty()))
                .map(|executable| (0, executable.to_owned()))
        } else {
            None
        };

        let (output_tx, output_rx) = mpsc::channel(16);
        let client = RdpClient::new(config, output_tx);
        #[cfg(windows)]
        let client = match rdpdr_factory {
            Some(factory) => client.with_rdpdr_backend_factory(Box::new(factory)),
            None => client,
        };
        let input_tx = client.input_sender();

        let rail_notify = Arc::new(tokio::sync::Notify::new());
        let live = Arc::new(Mutex::new(Live {
            properties: live_seed,
            state: ConnState::Connecting,
            error: None,
            frame: None,
            rail_initial_execute: initial_rail_execute.clone(),
            rail: RailLedger::new(
                self.next_rail_generation.fetch_add(1, Ordering::Relaxed),
                1,
                initial_rail_execute,
            ),
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

        tokio::spawn(consume_output(
            output_rx,
            Arc::clone(&live),
            self.notification.clone(),
            Arc::clone(&rail_notify),
            Arc::clone(&self.next_rail_generation),
        ));

        info!(%destination, "Started RDP session");

        *self.state.lock().expect("daemon state poisoned") = Some(Session {
            input_tx,
            input_db: Database::new(),
            destination,
            rail_enabled,
            live,
            rail_notify,
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

    fn rail_status(&self) -> Response {
        let guard = self.state.lock().expect("daemon state poisoned");
        let Some(session) = guard.as_ref() else {
            return Response::typed_error(crate::ipc::AgentErrorCategory::Unavailable, "no active session");
        };
        let live = session.live.lock().expect("session live state poisoned");
        Response::Ok(Payload::RailStatus(live.rail.status()))
    }

    fn rail_events(&self, after_sequence: Option<u64>) -> Response {
        let guard = self.state.lock().expect("daemon state poisoned");
        let Some(session) = guard.as_ref() else {
            return Response::typed_error(crate::ipc::AgentErrorCategory::Unavailable, "no active session");
        };
        let live = session.live.lock().expect("session live state poisoned");
        Response::Ok(Payload::RailEvents(live.rail.events_after(after_sequence)))
    }

    async fn rail_wait(&self, after_sequence: Option<u64>, timeout_ms: u32) -> Response {
        const MAX_RAIL_WAIT_MS: u32 = 60_000;
        if timeout_ms > MAX_RAIL_WAIT_MS {
            return Response::typed_error(
                crate::ipc::AgentErrorCategory::InvalidRequest,
                format!("RAIL wait timeout exceeds {MAX_RAIL_WAIT_MS} ms"),
            );
        }
        let (live, rail_notify) = {
            let guard = self.state.lock().expect("daemon state poisoned");
            let Some(session) = guard.as_ref() else {
                return Response::typed_error(crate::ipc::AgentErrorCategory::Unavailable, "no active session");
            };
            (Arc::clone(&session.live), Arc::clone(&session.rail_notify))
        };
        let deadline = tokio::time::Instant::now() + Duration::from_millis(u64::from(timeout_ms));
        loop {
            let notified = rail_notify.notified();
            tokio::pin!(notified);
            let _ = notified.as_mut().enable();
            let events = {
                let live = live.lock().expect("session live state poisoned");
                live.rail.events_after(after_sequence)
            };
            if !events.events.is_empty() || timeout_ms == 0 {
                return Response::Ok(Payload::RailEvents(events));
            }

            if tokio::time::timeout_at(deadline, &mut notified).await.is_err() {
                let live = live.lock().expect("session live state poisoned");
                return Response::Ok(Payload::RailEvents(live.rail.events_after(after_sequence)));
            }
        }
    }

    fn rail_execute(&self, request: RailExecuteRequest) -> Response {
        let execute = ExecutePdu {
            executable: request.executable,
            working_directory: request.working_directory,
            arguments: request.arguments,
            flags: request.flags,
        };
        if let Err(error) = RailPdu::Execute(execute.clone()).validate() {
            return Response::typed_error(
                crate::ipc::AgentErrorCategory::InvalidRequest,
                format!("invalid RAIL Execute request: {error}"),
            );
        }

        let guard = self.state.lock().expect("daemon state poisoned");
        let Some(session) = guard.as_ref() else {
            return Response::typed_error(crate::ipc::AgentErrorCategory::Unavailable, "no active session");
        };
        if !session.rail_enabled {
            return Response::typed_error(
                crate::ipc::AgentErrorCategory::Unavailable,
                "RAIL is not enabled for this session",
            );
        }
        let mut live = session.live.lock().expect("session live state poisoned");
        if !matches!(live.state, ConnState::Connecting | ConnState::Connected) {
            return Response::typed_error(
                crate::ipc::AgentErrorCategory::Unavailable,
                "RAIL session is not accepting launch requests",
            );
        }
        let permit = match session.input_tx.try_reserve() {
            Ok(permit) => permit,
            Err(_) => {
                return Response::typed_error(
                    crate::ipc::AgentErrorCategory::Unavailable,
                    "session input channel is unavailable",
                );
            }
        };
        let launch = RailLaunchInfo {
            launch_id: self.next_rail_launch_id.fetch_add(1, Ordering::Relaxed),
            executable: execute.executable.clone(),
            flags: execute.flags,
        };
        match live.rail.queue_launch(launch.clone()) {
            Ok(()) => {}
            Err(RailLaunchQueueError::LimitReached) => {
                return Response::typed_error(
                    crate::ipc::AgentErrorCategory::Unavailable,
                    "too many pending RAIL launch requests",
                );
            }
            Err(RailLaunchQueueError::DuplicateResponse) => {
                return Response::typed_error(
                    crate::ipc::AgentErrorCategory::Conflict,
                    "an indistinguishable RAIL launch request is already pending",
                );
            }
        }
        drop(live);
        permit.send(RdpInputEvent::RailExecute(execute));
        session.rail_notify.notify_waiters();
        Response::Ok(Payload::RailLaunch(launch))
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

    /// Sends one MS-RDPEI touch event to the active RDP session.
    ///
    /// # Panics
    ///
    /// Panics if the daemon state mutex is poisoned.
    fn touch(&self, encode_time: u32, frames: Vec<TouchFrameRequest>) -> Response {
        let event = match touch_event_from_request(encode_time, frames) {
            Ok(event) => event,
            Err(response) => return response,
        };

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
        permit.send(RdpInputEvent::Touch(event));
        Response::ok()
    }

    /// Sends one MS-RDPEI pen event to the active RDP session.
    ///
    /// # Panics
    ///
    /// Panics if the daemon state mutex is poisoned.
    fn pen(&self, encode_time: u32, frames: Vec<PenFrameRequest>) -> Response {
        let event = match pen_event_from_request(encode_time, frames) {
            Ok(event) => event,
            Err(response) => return response,
        };

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
        permit.send(RdpInputEvent::Pen(event));
        Response::ok()
    }

    /// Dismisses a hovering MS-RDPEI touch contact on the active RDP session.
    ///
    /// # Panics
    ///
    /// Panics if the daemon state mutex is poisoned.
    fn dismiss_hovering_touch_contact(&self, contact_id: u8) -> Response {
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
        permit.send(RdpInputEvent::DismissHoveringTouchContact { contact_id });
        Response::ok()
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

        enqueue_unicode_text(&session.input_tx, &mut session.input_db, text)
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
    rail_notify: Arc<tokio::sync::Notify>,
    next_rail_generation: Arc<AtomicU64>,
) {
    while let Some(event) = output_rx.recv().await {
        let mut guard = live.lock().expect("session live state poisoned");
        let previous = guard.state;
        let rail_changed = match event {
            RdpOutputEvent::Connected => {
                guard.state = ConnState::Connected;
                guard.error = None;
                if previous != ConnState::Connected {
                    info!("Session connected");
                }
                false
            }
            RdpOutputEvent::LoginComplete => false,
            RdpOutputEvent::WindowingOrders(orders) => {
                guard.rail.record(RailEventKind::WindowingOrders {
                    byte_count: u32::try_from(orders.len()).unwrap_or(u32::MAX),
                });
                true
            }
            RdpOutputEvent::RailHandshake {
                handshake_ex_flags,
                initialization_message_count,
                queued_execute_count,
            } => {
                guard.rail.handshake_complete = true;
                guard.rail.record(RailEventKind::Handshake {
                    handshake_ex_flags,
                    initialization_message_count: u16::try_from(initialization_message_count).unwrap_or(u16::MAX),
                    queued_execute_count: u16::try_from(queued_execute_count).unwrap_or(u16::MAX),
                });
                true
            }
            RdpOutputEvent::RailDesktopSynchronized { released_execute_count } => {
                guard.rail.desktop_synchronized = true;
                guard.rail.record(RailEventKind::DesktopSynchronized {
                    released_execute_count: u16::try_from(released_execute_count).unwrap_or(u16::MAX),
                });
                true
            }
            RdpOutputEvent::RailPostHandshakeQueueReleased { released_execute_count } => {
                guard.rail.record(RailEventKind::PostHandshakeQueueReleased {
                    released_execute_count: u16::try_from(released_execute_count).unwrap_or(u16::MAX),
                });
                true
            }
            RdpOutputEvent::RailExecuteResult(result) => {
                let launch_id = guard.rail.take_launch(result.flags, &result.executable);
                guard.rail.record(RailEventKind::ExecuteResult {
                    launch_id,
                    executable: result.executable,
                    flags: result.flags,
                    result: u16::from(result.result),
                    raw_result: result.raw_result,
                });
                true
            }
            RdpOutputEvent::RailExecuteFailed {
                executable,
                flags,
                reason,
            } => {
                let launch_id = guard.rail.take_launch(flags, &executable);
                let reason = match reason {
                    ClientRailExecuteFailureReason::RailUnavailable => RailExecuteFailureReason::RailUnavailable,
                    ClientRailExecuteFailureReason::QueueRejected => RailExecuteFailureReason::QueueRejected,
                    ClientRailExecuteFailureReason::MessageProcessingFailed => {
                        RailExecuteFailureReason::MessageProcessingFailed
                    }
                };
                guard.rail.record(RailEventKind::ExecuteFailed {
                    launch_id,
                    executable,
                    flags,
                    reason,
                });
                true
            }
            RdpOutputEvent::RailApplicationId {
                window_id,
                application_id,
                process_id,
                process_image_name,
            } => {
                guard.rail.record(RailEventKind::ApplicationId {
                    window_id,
                    application_id,
                    process_id,
                    process_image_name,
                });
                true
            }
            RdpOutputEvent::RailControl(control) => {
                let kind = match control {
                    RailControlEvent::SystemParameters(_) => "system-parameters",
                    RailControlEvent::LanguageBar(_) => "language-bar",
                    RailControlEvent::Compartment(_) => "compartment",
                    RailControlEvent::ZOrderSync(_) => "z-order-sync",
                    RailControlEvent::Cloak(_) => "cloak",
                    RailControlEvent::PowerDisplayRequest(_) => "power-display-request",
                };
                guard.rail.record(RailEventKind::Control { kind: kind.to_owned() });
                true
            }
            RdpOutputEvent::DisplayResizeFallback(_) => {
                let next_sequence = guard.rail.next_sequence;
                guard.rail = RailLedger::new(
                    next_rail_generation.fetch_add(1, Ordering::Relaxed),
                    next_sequence,
                    guard.rail_initial_execute.clone(),
                );
                true
            }
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
                false
            }
            RdpOutputEvent::ConnectionFailure(error) => {
                guard.state = ConnState::Failed;
                guard.error = Some(format!("{error}"));
                let rail_changed = guard.rail.fail_pending_launches();
                error!(%error, "Session connection failed");
                rail_changed
            }
            RdpOutputEvent::Terminated(Ok(reason)) => {
                guard.state = ConnState::Disconnected;
                guard.error = Some(format!("{reason:?}"));
                let rail_changed = guard.rail.fail_pending_launches();
                info!(?reason, "Session terminated");
                rail_changed
            }
            RdpOutputEvent::Terminated(Err(error)) => {
                guard.state = ConnState::Failed;
                guard.error = Some(format!("{error}"));
                let rail_changed = guard.rail.fail_pending_launches();
                warn!(%error, "Session terminated with an error");
                rail_changed
            }
            // With software pointer rendering the cursor is composited into the `Image` frames
            // above; the remaining pointer events (default/hidden) carry no live state we track.
            _ => false,
        };
        drop(guard);
        if rail_changed {
            rail_notify.notify_waiters();
        }
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
    let rail_changed = guard.rail.fail_pending_launches();
    drop(guard);
    if rail_changed {
        rail_notify.notify_waiters();
    }
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
fn rdpdr_backend_factory(
    drives: Vec<RdpdrDriveConfig>,
    smartcard: bool,
) -> anyhow::Result<Option<WindowsRdpdrBackendFactory>> {
    if drives.is_empty() && !smartcard {
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
        .map(|factory| factory.with_smartcard(smartcard))
        .map(Some)
        .map_err(|error| anyhow::anyhow!("invalid rdpdr drive configuration: {error}"))
}

/// Resolves the RDPDR factory for one connect: clone the startup factory (if any) and apply the
/// effective smartcard flag. Smartcard-only sessions may create an empty-drive factory on demand.
/// An empty-drive factory is dropped when smartcard is disabled so RDPDR is not attached without a
/// device to announce.
#[cfg(windows)]
fn resolve_rdpdr_factory(
    base: Option<&WindowsRdpdrBackendFactory>,
    smartcard: bool,
) -> anyhow::Result<Option<WindowsRdpdrBackendFactory>> {
    match base {
        Some(factory) => {
            let factory = factory.clone().with_smartcard(smartcard);
            if factory.initial_drives().is_empty() && !smartcard {
                Ok(None)
            } else {
                Ok(Some(factory))
            }
        }
        None if smartcard => WindowsRdpdrBackendFactory::from_drives(Vec::new())
            .map(|factory| factory.with_smartcard(true))
            .map(Some)
            .map_err(|error| anyhow::anyhow!("invalid smartcard-only rdpdr configuration: {error}")),
        None => Ok(None),
    }
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
    use core::sync::atomic::AtomicU64;
    use core::time::Duration;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use tokio::sync::mpsc;

    use ironrdp_client::rdp::{RdpInputEvent, RdpInputSender};
    use ironrdp_input::{Database, Operation};
    use ironrdp_pdu::input::fast_path::{FastPathInputEvent, KeyboardFlags};
    use ironrdp_propertyset::PropertySet;

    use super::{
        ConnState, Daemon, DaemonOptions, Live, MAX_PENDING_RAIL_LAUNCHES, MAX_RAIL_RETAINED_EVENTS,
        MAX_UNICODE_TEXT_CHARS, NowEndpoint, OperationManager, RailLedger, RdpdrDriveConfig, ResizeError, Session,
        consume_output, enqueue_unicode_text, notify,
    };
    use crate::ipc::{Payload, Response};
    use ironrdp_rpc::ipc::{RailEventKind, RailExecuteRequest, RailLaunchInfo};
    use ironrdp_tls::CertificateValidation;

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
    fn rail_ledger_reports_history_gaps_and_correlates_launches() {
        let mut ledger = RailLedger::new(7, 1, None);
        let launch = RailLaunchInfo {
            launch_id: 1,
            executable: "notepad.exe".to_owned(),
            flags: 0,
        };
        ledger.queue_launch(launch).expect("room for launch");
        assert_eq!(ledger.take_launch(0, "notepad.exe"), Some(1));

        for byte_count in 0..=MAX_RAIL_RETAINED_EVENTS {
            ledger.record(RailEventKind::WindowingOrders {
                byte_count: u32::try_from(byte_count).expect("test count fits"),
            });
        }

        let dump = ledger.events_after(Some(0));
        assert_eq!(dump.generation, 7);
        assert!(matches!(
            dump.events.first(),
            Some(event) if matches!(event.kind, RailEventKind::Gap { .. })
        ));
        assert_eq!(dump.events.len(), MAX_RAIL_RETAINED_EVENTS + 1);
    }

    #[test]
    fn rail_ledger_reserves_capacity_for_the_initial_launch() {
        let mut ledger = RailLedger::new(7, 1, Some((0, "notepad.exe".to_owned())));
        let max_launches = u64::try_from(MAX_PENDING_RAIL_LAUNCHES).expect("launch limit fits");

        for launch_id in 1..max_launches {
            ledger
                .queue_launch(RailLaunchInfo {
                    launch_id,
                    executable: format!("application-{launch_id}.exe"),
                    flags: 0,
                })
                .expect("room for dynamic launch");
        }
        assert_eq!(ledger.status().pending_launches.len(), MAX_PENDING_RAIL_LAUNCHES - 1);
        assert!(
            ledger
                .queue_launch(RailLaunchInfo {
                    launch_id: max_launches,
                    executable: "application-final.exe".to_owned(),
                    flags: 0,
                })
                .is_err()
        );

        assert_eq!(ledger.take_launch(0, "notepad.exe"), None);
        assert!(
            ledger
                .queue_launch(RailLaunchInfo {
                    launch_id: max_launches,
                    executable: "application-final.exe".to_owned(),
                    flags: 0,
                })
                .is_ok()
        );
    }

    #[test]
    fn rail_ledger_rejects_indistinguishable_pending_launches() {
        let mut ledger = RailLedger::new(7, 1, Some((0, "notepad.exe".to_owned())));

        assert!(
            ledger
                .queue_launch(RailLaunchInfo {
                    launch_id: 1,
                    executable: "notepad.exe".to_owned(),
                    flags: 0,
                })
                .is_err()
        );
        assert!(
            ledger
                .queue_launch(RailLaunchInfo {
                    launch_id: 2,
                    executable: "notepad.exe".to_owned(),
                    flags: 1,
                })
                .is_ok()
        );
    }

    #[tokio::test]
    async fn resize_fallback_starts_a_new_rail_generation() {
        let live = Arc::new(Mutex::new(Live {
            properties: PropertySet::new(),
            state: ConnState::Connected,
            error: None,
            frame: None,
            rail_initial_execute: Some((0, "notepad.exe".to_owned())),
            rail: RailLedger::new(1, 1, Some((0, "notepad.exe".to_owned()))),
        }));
        {
            let mut guard = live.lock().expect("session live state poisoned");
            guard.rail.handshake_complete = true;
            guard.rail.desktop_synchronized = true;
            guard
                .rail
                .queue_launch(RailLaunchInfo {
                    launch_id: 1,
                    executable: "wordpad.exe".to_owned(),
                    flags: 0,
                })
                .expect("queue a dynamic launch");
        }

        let (output_tx, output_rx) = mpsc::channel(1);
        let rail_notify = Arc::new(tokio::sync::Notify::new());
        let consumer = tokio::spawn(consume_output(
            output_rx,
            Arc::clone(&live),
            None,
            Arc::clone(&rail_notify),
            Arc::new(AtomicU64::new(2)),
        ));
        output_tx
            .send(ironrdp_client::rdp::RdpOutputEvent::DisplayResizeFallback(
                ironrdp_client::rdp::DisplayResizeFallbackReason::CapabilitiesTimedOut,
            ))
            .await
            .expect("send resize fallback");
        drop(output_tx);
        consumer.await.expect("consume output");

        let mut guard = live.lock().expect("session live state poisoned");
        assert_eq!(guard.rail.generation, 2);
        assert!(!guard.rail.handshake_complete);
        assert!(!guard.rail.desktop_synchronized);
        assert!(guard.rail.status().pending_launches.is_empty());
        assert_eq!(guard.rail.status().next_sequence, 2);
        assert_eq!(guard.rail.take_launch(0, "notepad.exe"), None);
    }

    fn active_rail_session(
        rail_enabled: bool,
    ) -> (
        Daemon,
        mpsc::Receiver<RdpInputEvent>,
        Arc<Mutex<Live>>,
        Arc<tokio::sync::Notify>,
    ) {
        let daemon = Daemon::with_overlay(PropertySet::new());
        let (input_tx, input_rx) = RdpInputSender::channel(1);
        let now_endpoint = Arc::new(NowEndpoint::new().expect("create NOW endpoint"));
        let live = Arc::new(Mutex::new(Live {
            properties: PropertySet::new(),
            state: ConnState::Connected,
            error: None,
            frame: None,
            rail_initial_execute: None,
            rail: RailLedger::new(1, 1, None),
        }));
        let rail_notify = Arc::new(tokio::sync::Notify::new());
        *daemon.state.lock().expect("daemon state poisoned") = Some(Session {
            input_tx,
            input_db: Database::new(),
            destination: "server.example".to_owned(),
            rail_enabled,
            live: Arc::clone(&live),
            rail_notify: Arc::clone(&rail_notify),
            operations: OperationManager::new(Arc::clone(&now_endpoint)),
            now_endpoint,
        });
        (daemon, input_rx, live, rail_notify)
    }

    #[tokio::test]
    async fn rail_wait_ignores_non_rail_output_until_evidence_arrives() {
        let (daemon, _, live, rail_notify) = active_rail_session(true);

        let (output_tx, output_rx) = mpsc::channel(1);
        let consumer = tokio::spawn(consume_output(
            output_rx,
            live,
            None,
            rail_notify,
            Arc::new(AtomicU64::new(2)),
        ));
        output_tx
            .send(ironrdp_client::rdp::RdpOutputEvent::LoginComplete)
            .await
            .expect("send ignored output");

        let mut wait = Box::pin(daemon.rail_wait(None, 1_000));
        assert!(
            tokio::time::timeout(Duration::from_millis(25), wait.as_mut())
                .await
                .is_err()
        );

        output_tx
            .send(ironrdp_client::rdp::RdpOutputEvent::RailHandshake {
                handshake_ex_flags: None,
                initialization_message_count: 0,
                queued_execute_count: 0,
            })
            .await
            .expect("send RAIL evidence");
        let response = tokio::time::timeout(Duration::from_secs(1), wait.as_mut())
            .await
            .expect("RAIL event wakes wait");
        assert!(matches!(
            response,
            Response::Ok(Payload::RailEvents(events))
                if matches!(events.events.as_slice(), [event] if matches!(event.kind, RailEventKind::Handshake { .. }))
        ));

        drop(output_tx);
        consumer.await.expect("consume output");
    }

    #[tokio::test]
    async fn rail_execute_wakes_waiters_after_queueing_evidence() {
        let (daemon, mut input_rx, _, _) = active_rail_session(true);
        let mut wait = Box::pin(daemon.rail_wait(Some(0), 1_000));
        assert!(
            tokio::time::timeout(Duration::from_millis(25), wait.as_mut())
                .await
                .is_err()
        );

        let response = daemon.rail_execute(RailExecuteRequest {
            executable: "notepad.exe".to_owned(),
            working_directory: String::new(),
            arguments: String::new(),
            flags: 0,
        });
        assert!(matches!(
            response,
            Response::Ok(Payload::RailLaunch(RailLaunchInfo { launch_id: 1, .. }))
        ));
        assert!(matches!(
            input_rx.recv().await,
            Some(RdpInputEvent::RailExecute(execute)) if execute.executable == "notepad.exe"
        ));

        let response = tokio::time::timeout(Duration::from_secs(1), wait.as_mut())
            .await
            .expect("queued launch wakes wait");
        assert!(matches!(
            response,
            Response::Ok(Payload::RailEvents(events))
                if matches!(
                    events.events.as_slice(),
                    [event] if matches!(event.kind, RailEventKind::ExecuteQueued(_))
                )
        ));
    }

    #[tokio::test]
    async fn rail_cursors_resume_after_resize_generation_resets() {
        let (daemon, _, live, _) = active_rail_session(true);
        let after_sequence = {
            let mut live = live.lock().expect("session live state poisoned");
            live.rail.record(RailEventKind::WindowingOrders { byte_count: 1 });
            let after_sequence = live.rail.next_sequence.saturating_sub(1);
            let next_sequence = live.rail.next_sequence;
            live.rail = RailLedger::new(2, next_sequence, None);
            live.rail.record(RailEventKind::WindowingOrders { byte_count: 2 });
            after_sequence
        };

        let events = daemon.rail_events(Some(after_sequence));
        assert!(matches!(
            events,
            Response::Ok(Payload::RailEvents(events))
                if events.generation == 2
                    && matches!(
                        events.events.as_slice(),
                        [event] if event.sequence > after_sequence
                            && matches!(event.kind, RailEventKind::WindowingOrders { byte_count: 2 })
                    )
        ));
        let events = daemon.rail_wait(Some(after_sequence), 0).await;
        assert!(matches!(
            events,
            Response::Ok(Payload::RailEvents(events))
                if events.generation == 2
                    && matches!(
                        events.events.as_slice(),
                        [event] if event.sequence > after_sequence
                            && matches!(event.kind, RailEventKind::WindowingOrders { byte_count: 2 })
                    )
        ));
    }

    #[tokio::test]
    async fn local_rail_execute_failure_resolves_pending_launch_without_terminating() {
        let (daemon, mut input_rx, live, rail_notify) = active_rail_session(true);
        let (output_tx, output_rx) = mpsc::channel(1);
        let consumer = tokio::spawn(consume_output(
            output_rx,
            Arc::clone(&live),
            None,
            rail_notify,
            Arc::new(AtomicU64::new(2)),
        ));

        assert!(matches!(
            daemon.rail_execute(RailExecuteRequest {
                executable: "notepad.exe".to_owned(),
                working_directory: String::new(),
                arguments: "--token secret-token".to_owned(),
                flags: 0,
            }),
            Response::Ok(Payload::RailLaunch(RailLaunchInfo { launch_id: 1, .. }))
        ));
        assert!(matches!(input_rx.recv().await, Some(RdpInputEvent::RailExecute(_))));

        let mut wait = Box::pin(daemon.rail_wait(Some(1), 1_000));
        assert!(
            tokio::time::timeout(Duration::from_millis(25), wait.as_mut())
                .await
                .is_err()
        );
        output_tx
            .send(ironrdp_client::rdp::RdpOutputEvent::RailExecuteFailed {
                executable: "notepad.exe".to_owned(),
                flags: 0,
                reason: ironrdp_client::rdp::RailExecuteFailureReason::RailUnavailable,
            })
            .await
            .expect("send local failure");

        let response = tokio::time::timeout(Duration::from_secs(1), wait.as_mut())
            .await
            .expect("local failure wakes wait");
        assert!(matches!(
            response,
            Response::Ok(Payload::RailEvents(events))
                if matches!(
                    events.events.as_slice(),
                    [event] if matches!(
                        event.kind,
                        RailEventKind::ExecuteFailed {
                            launch_id: Some(1),
                            reason: ironrdp_rpc::ipc::RailExecuteFailureReason::RailUnavailable,
                            ..
                        }
                    )
                )
        ));
        assert!(matches!(
            daemon.rail_status(),
            Response::Ok(Payload::RailStatus(status)) if status.pending_launches.is_empty()
        ));
        assert_eq!(
            live.lock().expect("session live state poisoned").state,
            ConnState::Connected
        );

        drop(output_tx);
        consumer.await.expect("consume output");
    }

    #[tokio::test]
    async fn connection_failure_discards_pending_rail_launches() {
        let (daemon, mut input_rx, live, rail_notify) = active_rail_session(true);
        let (output_tx, output_rx) = mpsc::channel(1);
        let consumer = tokio::spawn(consume_output(
            output_rx,
            Arc::clone(&live),
            None,
            rail_notify,
            Arc::new(AtomicU64::new(2)),
        ));

        assert!(matches!(
            daemon.rail_execute(RailExecuteRequest {
                executable: "notepad.exe".to_owned(),
                working_directory: String::new(),
                arguments: String::new(),
                flags: 0,
            }),
            Response::Ok(Payload::RailLaunch(RailLaunchInfo { launch_id: 1, .. }))
        ));
        assert!(matches!(input_rx.recv().await, Some(RdpInputEvent::RailExecute(_))));
        let mut wait = Box::pin(daemon.rail_wait(Some(1), 1_000));
        output_tx
            .send(ironrdp_client::rdp::RdpOutputEvent::ConnectionFailure(
                ironrdp_connector::map_sequence_error(ironrdp_connector::general_err!("test connection failure")),
            ))
            .await
            .expect("send connection failure");

        let response = tokio::time::timeout(Duration::from_secs(1), wait.as_mut())
            .await
            .expect("connection failure wakes wait");
        assert!(matches!(
            response,
            Response::Ok(Payload::RailEvents(events))
                if matches!(
                    events.events.as_slice(),
                    [event] if matches!(
                        event.kind,
                        RailEventKind::ExecuteFailed {
                            launch_id: Some(1),
                            reason: ironrdp_rpc::ipc::RailExecuteFailureReason::RailUnavailable,
                            ..
                        }
                    )
                )
        ));
        assert_eq!(
            live.lock().expect("session live state poisoned").state,
            ConnState::Failed
        );
        assert!(matches!(
            daemon.rail_status(),
            Response::Ok(Payload::RailStatus(status)) if status.pending_launches.is_empty()
        ));

        drop(output_tx);
        consumer.await.expect("consume output");
    }

    #[test]
    fn rail_launch_ids_do_not_repeat_after_a_session_reconnects() {
        let (daemon, mut input_rx, live, _) = active_rail_session(true);

        assert!(matches!(
            daemon.rail_execute(RailExecuteRequest {
                executable: "notepad.exe".to_owned(),
                working_directory: String::new(),
                arguments: String::new(),
                flags: 0,
            }),
            Response::Ok(Payload::RailLaunch(RailLaunchInfo { launch_id: 1, .. }))
        ));
        assert!(matches!(input_rx.try_recv(), Ok(RdpInputEvent::RailExecute(_))));
        live.lock().expect("session live state poisoned").rail = RailLedger::new(2, 1, None);

        assert!(matches!(
            daemon.rail_execute(RailExecuteRequest {
                executable: "wordpad.exe".to_owned(),
                working_directory: String::new(),
                arguments: String::new(),
                flags: 0,
            }),
            Response::Ok(Payload::RailLaunch(RailLaunchInfo { launch_id: 2, .. }))
        ));
    }

    #[test]
    fn rail_execute_rejects_terminal_sessions_without_queueing_input() {
        let (daemon, mut input_rx, live, _) = active_rail_session(true);
        live.lock().expect("session live state poisoned").state = ConnState::Failed;

        let response = daemon.rail_execute(RailExecuteRequest {
            executable: "notepad.exe".to_owned(),
            working_directory: String::new(),
            arguments: String::new(),
            flags: 0,
        });

        assert!(matches!(
            response,
            Response::Err(error) if error.message == "RAIL session is not accepting launch requests"
        ));
        assert!(matches!(input_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
    }

    #[test]
    fn rail_execute_rejects_desktop_sessions_without_queueing_input() {
        let (daemon, mut input_rx, _, _) = active_rail_session(false);

        let response = daemon.rail_execute(RailExecuteRequest {
            executable: "notepad.exe".to_owned(),
            working_directory: String::new(),
            arguments: String::new(),
            flags: 0,
        });

        assert!(matches!(response, Response::Err(error) if error.message == "RAIL is not enabled for this session"));
        assert!(matches!(input_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
    }

    #[test]
    fn resize_without_session_reports_no_active_session() {
        let daemon = Daemon::with_overlay(PropertySet::new());

        assert_eq!(daemon.try_resize(1024, 768), Err(ResizeError::NoSession));
        assert!(matches!(daemon.resize(1024, 768), Response::Err(error) if error.message == "no active session"));
    }

    #[test]
    fn daemon_options_default_to_strict_certificate_validation() {
        let strict = DaemonOptions::default();
        let insecure = DaemonOptions::default().with_certificate_check_skipped(true);

        assert_eq!(strict.certificate_validation(), CertificateValidation::Strict);
        assert_eq!(
            insecure.certificate_validation(),
            CertificateValidation::DangerouslyAcceptInvalidCertificate
        );
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
    fn unicode_text_enqueues_ordered_utf16_events() {
        let (sender, mut receiver) = RdpInputSender::channel(2);
        let mut database = Database::new();

        assert_eq!(
            enqueue_unicode_text(&sender, &mut database, "A\u{1f600}"),
            Response::ok()
        );
        let first = match receiver.try_recv().expect("first character is enqueued") {
            RdpInputEvent::FastPath(events) => events,
            event => panic!("expected FastPath input, got {event:?}"),
        };
        assert_eq!(
            first.as_slice(),
            [
                FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::empty(), 0x0041),
                FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::RELEASE, 0x0041),
            ]
        );
        let second = match receiver.try_recv().expect("second character is enqueued") {
            RdpInputEvent::FastPath(events) => events,
            event => panic!("expected FastPath input, got {event:?}"),
        };
        assert_eq!(
            second.as_slice(),
            [
                FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::empty(), 0xD83D),
                FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::empty(), 0xDE00),
                FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::RELEASE, 0xD83D),
                FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::RELEASE, 0xDE00),
            ]
        );
        assert!(matches!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
    }

    #[test]
    fn unicode_text_backpressure_enqueues_no_prefix_or_input_state() {
        let (sender, mut receiver) = RdpInputSender::channel(2);
        let mut database = Database::new();
        sender
            .try_send(RdpInputEvent::Resize {
                width: 1024,
                height: 768,
                scale_factor: 100,
                physical_size: None,
            })
            .expect("the first queue slot is available");

        assert!(matches!(
            enqueue_unicode_text(&sender, &mut database, "A\u{1f600}"),
            Response::Err(error) if error.message == "session input channel is unavailable"
        ));
        assert!(matches!(receiver.try_recv(), Ok(RdpInputEvent::Resize { .. })));
        assert!(matches!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        assert_eq!(
            database.apply([Operation::UnicodeKeyPressed('A')]).as_slice(),
            [FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::empty(), 0x0041)]
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
        assert!(!daemon.rdpdr_backend_factory.as_ref().expect("factory").smartcard());
    }

    #[cfg(windows)]
    #[test]
    fn smartcard_daemon_constructs_a_factory_without_drives() {
        let daemon = Daemon::with_options(PropertySet::new(), DaemonOptions::default().with_smartcard(true))
            .expect("valid daemon options");

        assert!(daemon.smartcard_default);
        let factory = daemon.rdpdr_backend_factory.as_ref().expect("smartcard factory");
        assert!(factory.smartcard());
        assert!(factory.initial_drives().is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn smartcard_overlay_enables_factory_at_startup() {
        use ironrdp_cfg::PropertySetExt as _;

        let mut overlay = PropertySet::new();
        overlay.set_enable_smartcard(true);
        let daemon = Daemon::with_options(overlay, DaemonOptions::default()).expect("valid daemon options");

        assert!(daemon.smartcard_default);
        assert!(daemon.rdpdr_backend_factory.as_ref().expect("factory").smartcard());
    }

    #[cfg(windows)]
    #[test]
    fn resolve_rdpdr_factory_creates_smartcard_only_on_demand() {
        let factory = super::resolve_rdpdr_factory(None, true)
            .expect("smartcard-only factory")
            .expect("factory present");
        assert!(factory.smartcard());
        assert!(factory.initial_drives().is_empty());
        assert!(super::resolve_rdpdr_factory(None, false).expect("no factory").is_none());
        assert!(
            super::resolve_rdpdr_factory(Some(&factory), false)
                .expect("disable smartcard-only")
                .is_none()
        );
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
