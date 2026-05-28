#![expect(clippy::print_stdout, reason = "CLI prints structured output to stdout")]
#![expect(unused_crate_dependencies, reason = "split lib/bin causes false positives")]

use core::time::Duration;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::Context as _;
use ironrdp::input::{Database, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp_agent::cli::{
    Cli, Command, Endpoint, KeyboardAction as CliKeyboardAction, LogLevel, MouseAction as CliMouseAction,
    ScreenshotCommand, SessionArg, SetPropertyCommand, WaitFrameCommand,
};
use ironrdp_agent::descriptions::property_description;
use ironrdp_agent::help::HELP_AGENT;
use ironrdp_agent::ipc::{
    KeyboardAction, MouseAction, PropertyEntry, Request, Response, SessionStatus, SessionSummary, read_frame,
    write_frame,
};
use ironrdp_client::config::ConfigBuilder;
use ironrdp_client::rdp::{DvcPipeProxyFactory, RdpClient, RdpInputEvent, RdpOutputEvent};
use ironrdp_propertyset::PropertySet;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Mutex, Notify, RwLock, mpsc};
use tracing::{debug, error, info, warn};

use clap::Parser as _;

trait AsyncReadWrite: AsyncRead + AsyncWrite {}
impl<T> AsyncReadWrite for T where T: AsyncRead + AsyncWrite {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if cli.help_agent {
        print!("{HELP_AGENT}");
        return Ok(());
    }

    setup_logging(cli.log_level, cli.log_filter.as_deref(), cli.log_file.as_deref())
        .context("unable to initialize logging")?;

    let command = cli
        .command
        .ok_or_else(|| anyhow::anyhow!("missing subcommand; try --help-agent"))?;

    match command {
        Command::Daemon => serve_daemon(cli.endpoint).await,
        command => {
            let daemon_logging = DaemonLogging {
                log_level: cli.log_level,
                log_filter: cli.log_filter,
                log_file: cli.log_file,
            };
            run_client_command(cli.endpoint, !cli.no_spawn_daemon, daemon_logging, command).await
        }
    }
}

// --- logging -----------------------------------------------------------------

fn setup_logging(log_level: LogLevel, log_filter: Option<&str>, log_file: Option<&Path>) -> anyhow::Result<()> {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

    let env_filter = if let Some(log_filter) = log_filter {
        EnvFilter::builder().parse_lossy(log_filter)
    } else {
        EnvFilter::builder()
            .with_default_directive(log_level.as_level_filter().into())
            .with_env_var("IRONRDP_LOG")
            .from_env_lossy()
    };

    if let Some(log_file) = log_file {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_file)
            .with_context(|| format!("couldn't open {}", log_file.display()))?;
        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(file)
            .compact();
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .try_init()
            .context("failed to set tracing global subscriber")?;
    } else {
        let fmt_layer = tracing_subscriber::fmt::layer().compact();
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .try_init()
            .context("failed to set tracing global subscriber")?;
    }

    Ok(())
}

// --- daemon ------------------------------------------------------------------

struct Frame {
    buffer: Vec<u32>,
    width: u16,
    height: u16,
}

impl Frame {
    fn to_png(&self) -> anyhow::Result<Vec<u8>> {
        use std::io::Cursor;

        let mut rgba = Vec::with_capacity(self.buffer.len() * 4);
        for pixel in &self.buffer {
            let [_, r, g, b] = pixel.to_be_bytes();
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
        let image =
            image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(u32::from(self.width), u32::from(self.height), rgba)
                .context("invalid framebuffer dimensions")?;
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut png, image::ImageFormat::Png)
            .context("encode PNG")?;
        Ok(png.into_inner())
    }
}

struct SessionSnapshot {
    status: SessionStatus,
    frame: Option<Frame>,
    frame_sequence: u64,
    pointer_x: u16,
    pointer_y: u16,
    last_error: Option<String>,
}

impl SessionSnapshot {
    fn new() -> Self {
        Self {
            status: SessionStatus::Connecting,
            frame: None,
            frame_sequence: 0,
            pointer_x: 0,
            pointer_y: 0,
            last_error: None,
        }
    }
}

struct SessionEntry {
    session_id: String,
    label: Option<String>,
    input_sender: mpsc::UnboundedSender<RdpInputEvent>,
    input_database: Arc<Mutex<Database>>,
    snapshot: Arc<RwLock<SessionSnapshot>>,
    notify: Arc<Notify>,
    properties: Arc<RwLock<PropertySet>>,
}

impl SessionEntry {
    async fn summary(&self) -> SessionSummary {
        let snap = self.snapshot.read().await;
        let mouse = self.input_database.lock().await.mouse_position();
        SessionSummary {
            session_id: self.session_id.clone(),
            label: self.label.clone(),
            status: snap.status,
            width: snap.frame.as_ref().map(|f| f.width),
            height: snap.frame.as_ref().map(|f| f.height),
            frame_sequence: snap.frame_sequence,
            mouse_x: mouse.x,
            mouse_y: mouse.y,
            last_error: snap.last_error.clone(),
        }
    }

    async fn apply_operations(&self, operations: impl IntoIterator<Item = Operation>) -> anyhow::Result<()> {
        let mut db = self.input_database.lock().await;
        let events = db.apply(operations);
        if !events.is_empty() {
            self.input_sender
                .send(RdpInputEvent::FastPath(events))
                .map_err(|_| anyhow::anyhow!("session input channel is closed"))?;
        }
        Ok(())
    }

    async fn wait_frame(&self, timeout: Duration, after_frame: Option<u64>) -> anyhow::Result<()> {
        if self.has_requested_frame(after_frame).await {
            return Ok(());
        }
        tokio::time::timeout(timeout, async {
            loop {
                self.notify.notified().await;
                if self.has_requested_frame(after_frame).await {
                    break;
                }
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for frame"))?;
        if self.has_requested_frame(after_frame).await {
            Ok(())
        } else {
            anyhow::bail!("session has no frame")
        }
    }

    async fn has_requested_frame(&self, after_frame: Option<u64>) -> bool {
        let snap = self.snapshot.read().await;
        snap.frame.is_some() && after_frame.is_none_or(|af| snap.frame_sequence > af)
    }

    async fn screenshot_png(&self) -> anyhow::Result<Vec<u8>> {
        let snap = self.snapshot.read().await;
        let frame = snap.frame.as_ref().context("session has no frame")?;
        frame.to_png()
    }

    async fn dump_properties(&self) -> Vec<PropertyEntry> {
        let snap = self.snapshot.read().await;
        let mouse = self.input_database.lock().await.mouse_position();
        let mut props = self.properties.read().await.clone();
        // Surface agent:* synthetics so the consumer sees them in one shot.
        props.insert("agent:state", snap.status.as_str());
        if let Some(err) = &snap.last_error {
            props.insert("agent:last_error", err.as_str());
        }
        if let Some(frame) = &snap.frame {
            props.insert("agent:current_width", i64::from(frame.width));
            props.insert("agent:current_height", i64::from(frame.height));
        }
        props.insert(
            "agent:frame_sequence",
            i64::try_from(snap.frame_sequence).unwrap_or(i64::MAX),
        );
        props.insert("agent:mouse_x", i64::from(mouse.x));
        props.insert("agent:mouse_y", i64::from(mouse.y));
        if let Some(label) = &self.label {
            props.insert("agent:label", label.as_str());
        }

        let mut entries: Vec<_> = props
            .iter()
            .map(|(k, v)| {
                let value_string = v.to_string();
                let key_str = k.as_ref();
                let value = ironrdp_agent::redact::redact_value(key_str, &value_string).to_owned();
                PropertyEntry {
                    key: k.to_string(),
                    value,
                    description: property_description(key_str).to_owned(),
                }
            })
            .collect();
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        entries
    }

    async fn set_property(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let mut props = self.properties.write().await;
        let key_owned = key.to_owned();
        if let Ok(n) = value.parse::<i64>() {
            props.insert(key_owned, n);
        } else {
            props.insert(key_owned, value.to_owned());
        }
        Ok(())
    }
}

struct Daemon {
    sessions: RwLock<HashMap<String, Arc<SessionEntry>>>,
    counter: core::sync::atomic::AtomicU64,
}

impl Daemon {
    fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            counter: core::sync::atomic::AtomicU64::new(1),
        }
    }

    fn new_session_id(&self) -> String {
        let n = self.counter.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        format!("s{:08x}{:04x}", std::process::id(), n & 0xffff,)
    }

    async fn create_session(&self, rdp_content: String, label: Option<String>) -> anyhow::Result<String> {
        let mut properties = PropertySet::new();
        if let Err(errors) = ironrdp_rdpfile::load(&mut properties, &rdp_content) {
            for error in &errors {
                warn!(%error, "Ignored .rdp entry from IPC connect payload");
            }
        }

        let config = ConfigBuilder::from_property_set(&properties)
            .build()
            .context("build session config from property set")?;

        let session_id = self.new_session_id();
        let snapshot = Arc::new(RwLock::new(SessionSnapshot::new()));
        let notify = Arc::new(Notify::new());
        let input_database = Arc::new(Mutex::new(Database::new()));
        let (input_sender, input_receiver) = RdpInputEvent::create_channel();
        let (output_sender, output_receiver) = mpsc::channel::<RdpOutputEvent>(64);
        let dvc_pipe_proxy_factory = DvcPipeProxyFactory::new(input_sender.clone());

        let client = RdpClient {
            config,
            output_event_sender: output_sender,
            input_event_receiver: input_receiver,
            dvc_pipe_proxy_factory,
        };

        std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(error) => {
                    error!(%error, "Failed to create RDP session runtime");
                    return;
                }
            };
            runtime.block_on(client.run());
        });

        let properties = Arc::new(RwLock::new(properties));
        tokio::spawn(process_output_events(
            Arc::clone(&snapshot),
            Arc::clone(&notify),
            Arc::clone(&properties),
            output_receiver,
        ));

        let entry = Arc::new(SessionEntry {
            session_id: session_id.clone(),
            label,
            input_sender,
            input_database,
            snapshot,
            notify,
            properties,
        });

        self.sessions.write().await.insert(session_id.clone(), entry);
        Ok(session_id)
    }

    async fn session(&self, session_id: &str) -> anyhow::Result<Arc<SessionEntry>> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .with_context(|| format!("session {session_id} not found"))
    }

    async fn summaries(&self) -> Vec<SessionSummary> {
        let sessions = self.sessions.read().await;
        let mut out = Vec::with_capacity(sessions.len());
        for entry in sessions.values() {
            out.push(entry.summary().await);
        }
        out
    }
}

async fn process_output_events(
    snapshot: Arc<RwLock<SessionSnapshot>>,
    notify: Arc<Notify>,
    properties: Arc<RwLock<PropertySet>>,
    mut receiver: mpsc::Receiver<RdpOutputEvent>,
) {
    while let Some(event) = receiver.recv().await {
        match event {
            RdpOutputEvent::Image { buffer, width, height } => {
                let mut snap = snapshot.write().await;
                snap.status = SessionStatus::Connected;
                snap.frame_sequence = snap.frame_sequence.saturating_add(1);
                snap.frame = Some(Frame {
                    buffer,
                    width: width.get(),
                    height: height.get(),
                });
                let mut props = properties.write().await;
                props.insert("agent:state", "connected");
                props.insert("agent:current_width", core::num::NonZeroI64::from(width).get());
                props.insert("agent:current_height", core::num::NonZeroI64::from(height).get());
                notify.notify_waiters();
            }
            RdpOutputEvent::ConnectionFailure(error) => {
                let mut snap = snapshot.write().await;
                snap.status = SessionStatus::Failed;
                snap.last_error = Some(error.to_string());
                properties.write().await.insert("agent:state", "failed");
                notify.notify_waiters();
            }
            RdpOutputEvent::Terminated(result) => {
                let mut snap = snapshot.write().await;
                snap.status = match &result {
                    Ok(_) => SessionStatus::Disconnected,
                    Err(_) => SessionStatus::Failed,
                };
                snap.last_error = match result {
                    Ok(reason) => Some(reason.to_string()),
                    Err(error) => Some(error.to_string()),
                };
                properties.write().await.insert("agent:state", snap.status.as_str());
                notify.notify_waiters();
            }
            RdpOutputEvent::PointerPosition { x, y } => {
                let mut snap = snapshot.write().await;
                snap.pointer_x = x;
                snap.pointer_y = y;
            }
            RdpOutputEvent::PointerDefault | RdpOutputEvent::PointerHidden | RdpOutputEvent::PointerBitmap(_) => {}
        }
    }
}

async fn serve_daemon(endpoint: Endpoint) -> anyhow::Result<()> {
    let daemon = Arc::new(Daemon::new());
    info!(%endpoint, "Start ironrdp-agent daemon");

    match endpoint {
        Endpoint::Pipe(name) => serve_pipe(name, daemon).await,
        Endpoint::Unix(path) => serve_unix(path, daemon).await,
    }
}

#[cfg(windows)]
async fn serve_pipe(name: String, daemon: Arc<Daemon>) -> anyhow::Result<()> {
    use tokio::net::windows::named_pipe::{PipeMode, ServerOptions};

    let path = format!(r"\\.\pipe\{name}");
    loop {
        let server = ServerOptions::new()
            .access_inbound(true)
            .access_outbound(true)
            .pipe_mode(PipeMode::Byte)
            .create(&path)
            .with_context(|| format!("failed to create named pipe {path}"))?;
        server
            .connect()
            .await
            .with_context(|| format!("failed to accept named pipe connection on {path}"))?;
        let daemon = Arc::clone(&daemon);
        tokio::spawn(async move {
            if let Err(error) = serve_stream(server, daemon).await {
                error!(%error, "IPC connection failed");
            }
        });
    }
}

#[cfg(not(windows))]
async fn serve_pipe(_name: String, _daemon: Arc<Daemon>) -> anyhow::Result<()> {
    anyhow::bail!("named pipe endpoints are only supported on Windows")
}

#[cfg(unix)]
async fn serve_unix(path: PathBuf, daemon: Arc<Daemon>) -> anyhow::Result<()> {
    use std::os::unix::fs::FileTypeExt as _;

    if let Ok(metadata) = std::fs::metadata(&path) {
        if metadata.file_type().is_socket() {
            std::fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))?;
        } else {
            anyhow::bail!("{} already exists and is not a socket", path.display());
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let listener =
        tokio::net::UnixListener::bind(&path).with_context(|| format!("failed to bind {}", path.display()))?;

    loop {
        let (stream, _addr) = listener
            .accept()
            .await
            .with_context(|| format!("failed to accept {}", path.display()))?;
        let daemon = Arc::clone(&daemon);
        tokio::spawn(async move {
            if let Err(error) = serve_stream(stream, daemon).await {
                error!(%error, "IPC connection failed");
            }
        });
    }
}

#[cfg(not(unix))]
async fn serve_unix(_path: PathBuf, _daemon: Arc<Daemon>) -> anyhow::Result<()> {
    anyhow::bail!("unix endpoints are only supported on Unix-like systems")
}

async fn serve_stream<S>(stream: S, daemon: Arc<Daemon>) -> anyhow::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read_half, write_half) = tokio::io::split(stream);
    let mut reader = read_half;
    let writer = Arc::new(Mutex::new(write_half));

    loop {
        let request = match read_frame::<_, Request>(&mut reader).await {
            Ok(r) => r,
            Err(error) => {
                debug!(%error, "IPC stream closed");
                break;
            }
        };
        let response = match handle_request(request, &daemon).await {
            Ok(r) => r,
            Err(error) => Response::Error {
                message: error.to_string(),
            },
        };
        let mut w = writer.lock().await;
        if let Err(error) = write_frame(&mut *w, &response).await {
            warn!(%error, "Failed to write IPC response");
            break;
        }
    }

    Ok(())
}

async fn handle_request(request: Request, daemon: &Arc<Daemon>) -> anyhow::Result<Response> {
    match request {
        Request::Health => Ok(Response::Health),
        Request::Connect { rdp_content, label } => {
            let session_id = daemon.create_session(rdp_content, label).await?;
            Ok(Response::Connect { session_id })
        }
        Request::Sessions => Ok(Response::Sessions {
            sessions: daemon.summaries().await,
        }),
        Request::Status { session_id } => {
            if let Some(id) = session_id {
                let session = daemon.session(&id).await?;
                Ok(Response::Status {
                    summary: session.summary().await,
                })
            } else {
                Ok(Response::Health)
            }
        }
        Request::Disconnect { session_id } => {
            let session = daemon.session(&session_id).await?;
            session
                .input_sender
                .send(RdpInputEvent::Close)
                .map_err(|_| anyhow::anyhow!("session input channel is closed"))?;
            Ok(Response::Ok)
        }
        Request::Mouse { session_id, action } => {
            let session = daemon.session(&session_id).await?;
            apply_mouse(&session, action).await?;
            Ok(Response::Ok)
        }
        Request::Keyboard { session_id, action } => {
            let session = daemon.session(&session_id).await?;
            apply_keyboard(&session, action).await?;
            Ok(Response::Ok)
        }
        Request::Resize {
            session_id,
            width,
            height,
            scale,
        } => {
            let session = daemon.session(&session_id).await?;
            session
                .input_sender
                .send(RdpInputEvent::Resize {
                    width,
                    height,
                    scale_factor: scale,
                    physical_size: None,
                })
                .map_err(|_| anyhow::anyhow!("session input channel is closed"))?;
            Ok(Response::Ok)
        }
        Request::WaitFrame {
            session_id,
            timeout_ms,
            after_frame,
        } => {
            let session = daemon.session(&session_id).await?;
            session
                .wait_frame(Duration::from_millis(timeout_ms), after_frame)
                .await?;
            Ok(Response::Ok)
        }
        Request::Screenshot { session_id } => {
            let session = daemon.session(&session_id).await?;
            let png = session.screenshot_png().await?;
            Ok(Response::Screenshot { png })
        }
        Request::MousePosition { session_id } => {
            let session = daemon.session(&session_id).await?;
            let pos = session.input_database.lock().await.mouse_position();
            Ok(Response::MousePosition { x: pos.x, y: pos.y })
        }
        Request::DumpProperties { session_id } => {
            let session = daemon.session(&session_id).await?;
            Ok(Response::Properties {
                entries: session.dump_properties().await,
            })
        }
        Request::SetProperty { session_id, key, value } => {
            let session = daemon.session(&session_id).await?;
            session.set_property(&key, &value).await?;
            Ok(Response::Ok)
        }
    }
}

async fn apply_mouse(session: &SessionEntry, action: MouseAction) -> anyhow::Result<()> {
    match action {
        MouseAction::Move { x, y } => {
            session
                .apply_operations([Operation::MouseMove(MousePosition { x, y })])
                .await
        }
        MouseAction::Click { button, x, y } => {
            let mut ops = Vec::new();
            if let (Some(x), Some(y)) = (x, y) {
                ops.push(Operation::MouseMove(MousePosition { x, y }));
            }
            let b = ironrdp::input::MouseButton::from(button);
            ops.push(Operation::MouseButtonPressed(b));
            ops.push(Operation::MouseButtonReleased(b));
            session.apply_operations(ops).await
        }
        MouseAction::Down { button } => {
            session
                .apply_operations([Operation::MouseButtonPressed(ironrdp::input::MouseButton::from(button))])
                .await
        }
        MouseAction::Up { button } => {
            session
                .apply_operations([Operation::MouseButtonReleased(ironrdp::input::MouseButton::from(
                    button,
                ))])
                .await
        }
        MouseAction::Wheel { units, horizontal } => {
            session
                .apply_operations([Operation::WheelRotations(WheelRotations {
                    is_vertical: !horizontal,
                    rotation_units: units,
                })])
                .await
        }
        MouseAction::Position => Ok(()),
    }
}

async fn apply_keyboard(session: &SessionEntry, action: KeyboardAction) -> anyhow::Result<()> {
    match action {
        KeyboardAction::Key { scancode, release } => {
            let op = if release {
                Operation::KeyReleased(Scancode::from_u16(scancode))
            } else {
                Operation::KeyPressed(Scancode::from_u16(scancode))
            };
            session.apply_operations([op]).await
        }
        KeyboardAction::Text { text } => {
            let ops: Vec<_> = text
                .chars()
                .flat_map(|c| [Operation::UnicodeKeyPressed(c), Operation::UnicodeKeyReleased(c)])
                .collect();
            session.apply_operations(ops).await
        }
        KeyboardAction::Shortcut { scancodes } => {
            let mut ops = Vec::with_capacity(scancodes.len() * 2);
            for s in &scancodes {
                ops.push(Operation::KeyPressed(Scancode::from_u16(*s)));
            }
            for s in scancodes.iter().rev() {
                ops.push(Operation::KeyReleased(Scancode::from_u16(*s)));
            }
            session.apply_operations(ops).await
        }
        KeyboardAction::ReleaseAll => {
            let mut db = session.input_database.lock().await;
            let events = db.release_all();
            if !events.is_empty() {
                session
                    .input_sender
                    .send(RdpInputEvent::FastPath(events))
                    .map_err(|_| anyhow::anyhow!("session input channel is closed"))?;
            }
            Ok(())
        }
    }
}

// --- client mode -------------------------------------------------------------

struct DaemonLogging {
    log_level: LogLevel,
    log_filter: Option<String>,
    log_file: Option<PathBuf>,
}

async fn run_client_command(
    endpoint: Endpoint,
    spawn_daemon: bool,
    daemon_logging: DaemonLogging,
    command: Command,
) -> anyhow::Result<()> {
    let (request, screenshot_output) = command_to_request(command).await?;

    let response = match send_request(&endpoint, &request).await {
        Ok(r) => r,
        Err(error) if spawn_daemon => {
            debug!(%error, "Daemon request failed, spawning daemon");
            spawn_daemon_process(&endpoint, &daemon_logging).context("spawn daemon")?;
            wait_for_daemon(&endpoint).await.context("wait for daemon")?;
            send_request(&endpoint, &request).await?
        }
        Err(error) => return Err(error),
    };

    print_response(response, screenshot_output).await
}

async fn command_to_request(command: Command) -> anyhow::Result<(Request, Option<PathBuf>)> {
    Ok(match command {
        Command::Daemon => anyhow::bail!("daemon cannot be used as a client command"),
        Command::Connect(connect) => {
            let label = connect.label.clone();
            let rdp_content = connect.to_rdp_content()?;
            (Request::Connect { rdp_content, label }, None)
        }
        Command::Sessions => (Request::Sessions, None),
        Command::Status(SessionArg { session }) => (Request::Status { session_id: session }, None),
        Command::Disconnect(arg) => (
            Request::Disconnect {
                session_id: arg.session,
            },
            None,
        ),
        Command::Mouse(cmd) => (
            Request::Mouse {
                session_id: cmd.session,
                action: cli_mouse_to_ipc(cmd.action),
            },
            None,
        ),
        Command::Keyboard(cmd) => (
            Request::Keyboard {
                session_id: cmd.session,
                action: cli_keyboard_to_ipc(cmd.action),
            },
            None,
        ),
        Command::Resize(cmd) => (
            Request::Resize {
                session_id: cmd.session,
                width: cmd.width,
                height: cmd.height,
                scale: cmd.scale,
            },
            None,
        ),
        Command::WaitFrame(WaitFrameCommand {
            session,
            timeout_ms,
            after_frame,
        }) => (
            Request::WaitFrame {
                session_id: session,
                timeout_ms,
                after_frame,
            },
            None,
        ),
        Command::Screenshot(ScreenshotCommand { session, output }) => {
            (Request::Screenshot { session_id: session }, Some(output))
        }
        Command::DumpProperties(arg) => (
            Request::DumpProperties {
                session_id: arg.session,
            },
            None,
        ),
        Command::SetProperty(SetPropertyCommand { session, key, value }) => (
            Request::SetProperty {
                session_id: session,
                key,
                value,
            },
            None,
        ),
    })
}

fn cli_mouse_to_ipc(a: CliMouseAction) -> MouseAction {
    match a {
        CliMouseAction::Move { x, y } => MouseAction::Move { x, y },
        CliMouseAction::Click { button, x, y } => MouseAction::Click {
            button: button.into(),
            x,
            y,
        },
        CliMouseAction::Down { button } => MouseAction::Down { button: button.into() },
        CliMouseAction::Up { button } => MouseAction::Up { button: button.into() },
        CliMouseAction::Wheel { units, horizontal } => MouseAction::Wheel { units, horizontal },
        CliMouseAction::Position => MouseAction::Position,
    }
}

fn cli_keyboard_to_ipc(a: CliKeyboardAction) -> KeyboardAction {
    match a {
        CliKeyboardAction::Key { scancode, release } => KeyboardAction::Key { scancode, release },
        CliKeyboardAction::Text { text } => KeyboardAction::Text { text },
        CliKeyboardAction::Shortcut { scancodes } => KeyboardAction::Shortcut { scancodes },
        CliKeyboardAction::ReleaseAll => KeyboardAction::ReleaseAll,
    }
}

async fn send_request(endpoint: &Endpoint, request: &Request) -> anyhow::Result<Response> {
    let mut stream = connect_endpoint(endpoint).await?;
    write_frame(&mut stream, request).await?;
    let response: Response = read_frame(&mut stream).await?;
    if let Response::Error { message } = &response {
        anyhow::bail!("{}", message);
    }
    Ok(response)
}

async fn connect_endpoint(endpoint: &Endpoint) -> anyhow::Result<Box<dyn AsyncReadWrite + Unpin + Send>> {
    match endpoint {
        Endpoint::Pipe(name) => connect_pipe(name).await,
        Endpoint::Unix(path) => connect_unix(path.as_path()).await,
    }
}

#[cfg(windows)]
async fn connect_pipe(name: &str) -> anyhow::Result<Box<dyn AsyncReadWrite + Unpin + Send>> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let path = format!(r"\\.\pipe\{name}");
    let stream = ClientOptions::new()
        .open(&path)
        .with_context(|| format!("failed to open named pipe {path}"))?;
    Ok(Box::new(stream))
}

#[cfg(not(windows))]
async fn connect_pipe(_name: &str) -> anyhow::Result<Box<dyn AsyncReadWrite + Unpin + Send>> {
    anyhow::bail!("named pipe endpoints are only supported on Windows")
}

#[cfg(unix)]
async fn connect_unix(path: &Path) -> anyhow::Result<Box<dyn AsyncReadWrite + Unpin + Send>> {
    let stream = tokio::net::UnixStream::connect(path)
        .await
        .with_context(|| format!("failed to connect {}", path.display()))?;
    Ok(Box::new(stream))
}

#[cfg(not(unix))]
async fn connect_unix(_path: &Path) -> anyhow::Result<Box<dyn AsyncReadWrite + Unpin + Send>> {
    anyhow::bail!("unix endpoints are only supported on Unix-like systems")
}

fn spawn_daemon_process(endpoint: &Endpoint, daemon_logging: &DaemonLogging) -> anyhow::Result<()> {
    use clap::ValueEnum as _;

    let exe = std::env::current_exe().context("current executable")?;
    let mut command = std::process::Command::new(exe);
    command
        .arg("--endpoint")
        .arg(endpoint.to_string())
        .arg("--log-level")
        .arg(
            daemon_logging
                .log_level
                .to_possible_value()
                .expect("log level")
                .get_name(),
        );

    if let Some(log_filter) = &daemon_logging.log_filter {
        command.arg("--log-filter").arg(log_filter);
    }
    if let Some(log_file) = &daemon_logging.log_file {
        command.arg("--log-file").arg(log_file);
    }

    command
        .arg("--no-spawn-daemon")
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn daemon")?;
    Ok(())
}

async fn wait_for_daemon(endpoint: &Endpoint) -> anyhow::Result<()> {
    for _ in 0..50 {
        if send_request(endpoint, &Request::Health).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("daemon did not become ready")
}

async fn print_response(response: Response, screenshot_output: Option<PathBuf>) -> anyhow::Result<()> {
    match response {
        Response::Ok | Response::Health => Ok(()),
        Response::Error { message } => anyhow::bail!("{message}"),
        Response::Connect { session_id } => {
            println!("{session_id}");
            Ok(())
        }
        Response::Sessions { sessions } => {
            for s in sessions {
                println!(
                    "{}  {:11}  {}x{}  seq={}  label={}  err={}",
                    s.session_id,
                    s.status.as_str(),
                    s.width.unwrap_or(0),
                    s.height.unwrap_or(0),
                    s.frame_sequence,
                    s.label.as_deref().unwrap_or("-"),
                    s.last_error.as_deref().unwrap_or("-"),
                );
            }
            Ok(())
        }
        Response::Status { summary: s } => {
            println!(
                "{}  {:11}  {}x{}  seq={}  label={}  err={}",
                s.session_id,
                s.status.as_str(),
                s.width.unwrap_or(0),
                s.height.unwrap_or(0),
                s.frame_sequence,
                s.label.as_deref().unwrap_or("-"),
                s.last_error.as_deref().unwrap_or("-"),
            );
            Ok(())
        }
        Response::MousePosition { x, y } => {
            println!("{x} {y}");
            Ok(())
        }
        Response::Screenshot { png } => {
            let path = screenshot_output.context("missing screenshot output path")?;
            std::fs::write(&path, &png).with_context(|| format!("write {}", path.display()))?;
            println!("{}", path.display());
            Ok(())
        }
        Response::Properties { entries } => {
            for entry in entries {
                println!("{}={}  # {}", entry.key, entry.value, entry.description);
            }
            Ok(())
        }
    }
}
