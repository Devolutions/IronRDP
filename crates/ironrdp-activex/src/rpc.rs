//! Opt-in local RPC service for an ActiveX-hosted session.
//!
//! The service owns no COM state. Requests which need the apartment-bound control are carried to
//! its message-only dispatcher and completed there; status, frame, log, and NOW state are shared
//! independently with the listener threads.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::Context as _;
use ironrdp_agent::ipc::{
    AgentErrorCategory, ConnState, KeyFilter, NowDiagnostics, Payload, PropValue, PropertyDump, PropertyEntry, Request,
    Response, StatusInfo,
};
use ironrdp_daemon::logbuf::{self, LogBuffer};
use ironrdp_daemon::now::NowEndpoint;
use ironrdp_daemon::operations::{OperationAttachment, OperationManager};
use ironrdp_input::Operation;
use ironrdp_propertyset::{PropertySet, Value};
use ironrdp_rpc as ironrdp_agent;
use ironrdp_rpc::transport::{self, Endpoint, Listener, read_message, write_message};
use tokio::sync::{oneshot, watch};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

pub(crate) const WM_DISPATCH_RPC: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 0x54;
const COMMAND_QUEUE_CAPACITY: usize = 64;
const MAX_SCREENSHOT_PIXELS: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct ActiveXRpc {
    shared: Arc<Shared>,
    listener: Arc<Mutex<Option<ListenerHandle>>>,
}

struct ListenerHandle {
    shutdown: watch::Sender<()>,
    join: JoinHandle<()>,
}

struct Shared {
    live: Mutex<Live>,
    logs: Arc<LogBuffer>,
    commands: Mutex<VecDeque<Command>>,
    command_posted: AtomicBool,
    closing: AtomicBool,
}

struct Live {
    state: ConnState,
    destination: Option<String>,
    properties: PropertySet,
    frame: Option<Frame>,
    error: Option<String>,
    now_endpoint: Option<Arc<NowEndpoint>>,
    operations: Option<OperationManager>,
}

#[derive(Clone)]
struct Frame {
    width: u16,
    height: u16,
    pixels: Vec<u32>,
}

pub(crate) enum Command {
    Connect {
        properties: PropertySet,
        log_directive: Option<String>,
        response: oneshot::Sender<Response>,
    },
    Disconnect {
        response: oneshot::Sender<Response>,
    },
    Input {
        operation: Operation,
        response: oneshot::Sender<Response>,
    },
    Resize {
        width: u16,
        height: u16,
        response: oneshot::Sender<Response>,
    },
}

enum ConnectionResponse {
    Single(Response),
    Stream(Response, OperationAttachment),
}

impl ActiveXRpc {
    pub(crate) fn from_environment() -> Option<Self> {
        if std::env::var_os("IRONRDP_ACTIVEX_RPC").as_deref() != Some(std::ffi::OsStr::new("1")) {
            return None;
        }

        Some(Self {
            shared: Arc::new(Shared {
                live: Mutex::new(Live {
                    state: ConnState::NoSession,
                    destination: None,
                    properties: PropertySet::new(),
                    frame: None,
                    error: None,
                    now_endpoint: None,
                    operations: None,
                }),
                logs: LogBuffer::new(),
                commands: Mutex::new(VecDeque::with_capacity(COMMAND_QUEUE_CAPACITY)),
                command_posted: AtomicBool::new(false),
                closing: AtomicBool::new(false),
            }),
            listener: Arc::new(Mutex::new(None)),
        })
    }

    pub(crate) fn start(&self, dispatcher: HWND) -> anyhow::Result<()> {
        let mut listener = lock(&self.listener);
        if listener.as_ref().is_some_and(|handle| !handle.join.is_finished()) {
            return Ok(());
        }
        if let Some(handle) = listener.take()
            && handle.join.join().is_err()
        {
            tracing::warn!("ActiveX RPC listener thread panicked");
        }

        let endpoint = endpoint_from_environment();
        let shared = Arc::clone(&self.shared);
        let dispatcher = dispatcher.0 as isize;
        let (shutdown, shutdown_rx) = watch::channel(());
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let join = std::thread::Builder::new()
            .name("ironrdp-activex-rpc".to_owned())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.into()));
                        return;
                    }
                };
                let listener = match runtime.block_on(async {
                    transport::prepare_endpoint(&endpoint).await?;
                    Listener::bind(&endpoint).with_context(|| format!("bind ActiveX RPC endpoint {endpoint}"))
                }) {
                    Ok(listener) => listener,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                if let Err(error) = runtime.block_on(serve(endpoint, listener, shared, dispatcher, shutdown_rx)) {
                    tracing::warn!(?error, "ActiveX RPC listener stopped with an error");
                }
            })
            .context("start ActiveX RPC listener")?;
        ready_rx.recv().context("wait for ActiveX RPC listener startup")??;
        *listener = Some(ListenerHandle { shutdown, join });
        Ok(())
    }

    pub(crate) fn stop(&self) {
        self.shared.closing.store(true, Ordering::Release);
        self.fail_pending("ActiveX control is closing");
        let handle = lock(&self.listener).take();
        if let Some(handle) = handle {
            let _ = handle.shutdown.send(());
            if handle.join.join().is_err() {
                tracing::warn!("ActiveX RPC listener thread panicked");
            }
        }
    }

    pub(crate) fn drain_commands<F>(&self, mut execute: F)
    where
        F: FnMut(Command),
    {
        self.shared.command_posted.store(false, Ordering::Release);
        let commands = {
            let mut commands = lock(&self.shared.commands);
            core::mem::take(&mut *commands)
        };
        for command in commands {
            execute(command);
        }
    }

    pub(crate) fn session_started(&self, destination: String, mut properties: PropertySet, endpoint: Arc<NowEndpoint>) {
        properties.remove("RDCleanPathToken");
        properties.remove("ironrdp_rdcleanpathtoken");
        self.shared.logs.clear();
        let mut live = lock(&self.shared.live);
        live.state = ConnState::Connecting;
        live.destination = Some(destination);
        live.properties = properties;
        live.frame = None;
        live.error = None;
        live.operations = Some(OperationManager::new(Arc::clone(&endpoint)));
        live.now_endpoint = Some(endpoint);
    }

    pub(crate) fn session_connected(&self) {
        let mut live = lock(&self.shared.live);
        live.state = ConnState::Connected;
        live.error = None;
    }

    pub(crate) fn session_disconnecting(&self) {
        let mut live = lock(&self.shared.live);
        if matches!(live.state, ConnState::Connecting | ConnState::Connected) {
            live.state = ConnState::Disconnecting;
        }
    }

    pub(crate) fn session_failed(&self, message: String) {
        let mut live = lock(&self.shared.live);
        live.state = ConnState::Failed;
        live.error = Some(message);
    }

    pub(crate) fn session_disconnected(&self, message: String) {
        let mut live = lock(&self.shared.live);
        live.state = ConnState::Disconnecting;
        live.error = Some(message);
    }

    pub(crate) fn session_stopped(&self) {
        let mut live = lock(&self.shared.live);
        if !matches!(live.state, ConnState::Failed) {
            live.state = ConnState::Disconnected;
        }
        live.frame = None;
    }

    pub(crate) fn retain_frame(&self, width: u16, height: u16, pixels: &[u32]) {
        let mut live = lock(&self.shared.live);
        live.properties.insert("desktopwidth", i64::from(width));
        live.properties.insert("desktopheight", i64::from(height));
        live.frame = Some(Frame {
            width,
            height,
            pixels: pixels.to_vec(),
        });
    }

    pub(crate) fn session_dispatch(&self, directive: Option<&str>) -> tracing::Dispatch {
        logbuf::session_dispatch(Arc::clone(&self.shared.logs), directive)
    }

    pub(crate) fn allocate_now_endpoint() -> Result<Arc<NowEndpoint>, Response> {
        NowEndpoint::new().map(Arc::new).map_err(|error| {
            Response::typed_error(
                AgentErrorCategory::Internal,
                format!("failed to allocate NOW endpoint: {error}"),
            )
        })
    }

    fn fail_pending(&self, message: &str) {
        let pending = {
            let mut commands = lock(&self.shared.commands);
            core::mem::take(&mut *commands)
        };
        for command in pending {
            respond(command, Response::typed_error(AgentErrorCategory::Unavailable, message));
        }
    }
}

async fn serve(
    endpoint: Endpoint,
    mut listener: Listener,
    shared: Arc<Shared>,
    dispatcher: isize,
    mut shutdown: watch::Receiver<()>,
) -> anyhow::Result<()> {
    tracing::info!(%endpoint, "ActiveX RPC listener started");

    loop {
        tokio::select! {
            result = listener.accept() => {
                let stream = result.context("accept ActiveX RPC connection")?;
                let shared = Arc::clone(&shared);
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(stream, shared, dispatcher).await {
                        tracing::debug!(error = format!("{error:#}"), "ActiveX RPC connection error");
                    }
                });
            }
            _ = shutdown.changed() => break,
        }
    }
    Ok(())
}

async fn handle_connection<S>(mut stream: S, shared: Arc<Shared>, dispatcher: isize) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let request: Request = read_message(&mut stream).await?;
    let response = handle_request(&shared, dispatcher, request).await;
    match response {
        ConnectionResponse::Single(response) => write_message(&mut stream, &response).await?,
        ConnectionResponse::Stream(response, mut attachment) => {
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

async fn handle_request(shared: &Arc<Shared>, dispatcher: isize, request: Request) -> ConnectionResponse {
    let response = match request {
        Request::Connect {
            properties,
            log_directive,
        } => {
            queue_command(shared, dispatcher, |response| Command::Connect {
                properties,
                log_directive,
                response,
            })
            .await
        }
        Request::Disconnect => queue_command(shared, dispatcher, |response| Command::Disconnect { response }).await,
        Request::Status => status(shared),
        Request::QueryProps { filter } => query_props(shared, filter.as_ref()),
        Request::QueryLogs { substring, last } => query_logs(shared, substring.as_deref(), last),
        Request::Screenshot => screenshot(shared),
        Request::MouseMove { x, y } => {
            queue_command(shared, dispatcher, |response| Command::Input {
                operation: Operation::MouseMove(ironrdp_input::MousePosition { x, y }),
                response,
            })
            .await
        }
        Request::MouseButton { button, pressed } => {
            queue_command(shared, dispatcher, |response| Command::Input {
                operation: if pressed {
                    Operation::MouseButtonPressed(button)
                } else {
                    Operation::MouseButtonReleased(button)
                },
                response,
            })
            .await
        }
        Request::Wheel { delta, horizontal } => {
            queue_command(shared, dispatcher, |response| Command::Input {
                operation: Operation::WheelRotations(ironrdp_input::WheelRotations {
                    is_vertical: !horizontal,
                    rotation_units: delta,
                }),
                response,
            })
            .await
        }
        Request::KeyScancode { scancode, pressed } => {
            queue_command(shared, dispatcher, |response| Command::Input {
                operation: if pressed {
                    Operation::KeyPressed(ironrdp_input::Scancode::from_u16(scancode))
                } else {
                    Operation::KeyReleased(ironrdp_input::Scancode::from_u16(scancode))
                },
                response,
            })
            .await
        }
        Request::KeyUnicode { ch, pressed } => {
            queue_command(shared, dispatcher, |response| Command::Input {
                operation: if pressed {
                    Operation::UnicodeKeyPressed(ch)
                } else {
                    Operation::UnicodeKeyReleased(ch)
                },
                response,
            })
            .await
        }
        Request::UnicodeText { .. } => Response::typed_error(
            AgentErrorCategory::InvalidRequest,
            "bulk Unicode text input is unsupported by ActiveX",
        ),
        Request::RailStatus | Request::RailEvents { .. } | Request::RailWait { .. } | Request::RailExecute(_) => {
            Response::typed_error(
                AgentErrorCategory::Unavailable,
                "RAIL audit endpoints are unavailable through ActiveX",
            )
        }
        Request::Resize { width, height } => {
            queue_command(shared, dispatcher, |response| Command::Resize {
                width,
                height,
                response,
            })
            .await
        }
        Request::NowCapabilities => return ConnectionResponse::Single(now_capabilities(shared).await),
        Request::NowRun { command, directory } => {
            return ConnectionResponse::Single(now_run(shared, command, directory).await);
        }
        Request::NowExecute(request) => return now_execute(shared, request).await,
        Request::NowCancel { operation_id } => {
            return ConnectionResponse::Single(now_cancel(shared, operation_id).await);
        }
        Request::NowList => return now_list(shared),
        Request::NowStatus { operation_id } => return now_status(shared, operation_id),
        Request::NowAttach {
            operation_id,
            after_sequence,
        } => return now_attach(shared, operation_id, after_sequence),
        Request::NowStdin {
            operation_id,
            data,
            last,
        } => return ConnectionResponse::Single(now_stdin(shared, operation_id, data, last).await),
        Request::NowDiagnostics => return ConnectionResponse::Single(now_diagnostics(shared).await),
    };
    ConnectionResponse::Single(response)
}

async fn queue_command<F>(shared: &Arc<Shared>, dispatcher: isize, command: F) -> Response
where
    F: FnOnce(oneshot::Sender<Response>) -> Command,
{
    if shared.closing.load(Ordering::Acquire) {
        return Response::typed_error(AgentErrorCategory::Unavailable, "ActiveX RPC listener is stopped");
    }
    let (response_tx, response_rx) = oneshot::channel();
    {
        let mut commands = lock(&shared.commands);
        if commands.len() == COMMAND_QUEUE_CAPACITY {
            return Response::typed_error(AgentErrorCategory::Unavailable, "ActiveX RPC command queue is full");
        }
        commands.push_back(command(response_tx));
        if !shared.command_posted.swap(true, Ordering::AcqRel)
            && unsafe {
                PostMessageW(
                    Some(HWND(dispatcher as *mut core::ffi::c_void)),
                    WM_DISPATCH_RPC,
                    WPARAM(0),
                    LPARAM(0),
                )
            }
            .is_err()
        {
            shared.command_posted.store(false, Ordering::Release);
            let _ = commands.pop_back();
            return Response::typed_error(AgentErrorCategory::Unavailable, "ActiveX dispatcher is unavailable");
        }
    }
    response_rx
        .await
        .unwrap_or_else(|_| Response::typed_error(AgentErrorCategory::Unavailable, "ActiveX control is unavailable"))
}

fn status(shared: &Shared) -> Response {
    let live = lock(&shared.live);
    let (width, height) = live
        .frame
        .as_ref()
        .map(|frame| (Some(frame.width), Some(frame.height)))
        .unwrap_or((None, None));
    Response::Ok(Payload::Status(StatusInfo {
        state: live.state,
        destination: live.destination.clone(),
        width,
        height,
        message: live.error.clone(),
        credentials_loaded: false,
    }))
}

fn query_props(shared: &Shared, filter: Option<&KeyFilter>) -> Response {
    let live = lock(&shared.live);
    if live.state == ConnState::NoSession {
        return unavailable_session();
    }
    let entries = live
        .properties
        .iter()
        .filter(|(key, _)| filter.is_none_or(|filter| filter.matches(key)))
        .map(|(key, value)| PropertyEntry {
            key: key.to_string(),
            value: match value {
                Value::Int(value) => PropValue::Int(*value),
                Value::Str(value) => PropValue::Str(value.clone()),
            },
        })
        .collect();
    Response::Ok(Payload::Properties(PropertyDump { entries }))
}

fn query_logs(shared: &Shared, substring: Option<&str>, last: Option<u32>) -> Response {
    let mut lines = shared.logs.query(substring);
    if let Some(last) = last
        && let Ok(last) = usize::try_from(last)
        && last < lines.len()
    {
        lines.drain(..lines.len() - last);
    }
    Response::Ok(Payload::Logs(lines))
}

fn screenshot(shared: &Shared) -> Response {
    let frame = match lock(&shared.live).frame.clone() {
        Some(frame) => frame,
        None => return Response::typed_error(AgentErrorCategory::Unavailable, "no frame available yet"),
    };
    let frame = downscale_frame(frame, MAX_SCREENSHOT_PIXELS);
    match encode_png(frame.width, frame.height, &frame.pixels) {
        Ok(png) => Response::Ok(Payload::Screenshot {
            width: frame.width,
            height: frame.height,
            png,
        }),
        Err(error) => Response::typed_error(
            AgentErrorCategory::Internal,
            format!("failed to encode screenshot: {error:#}"),
        ),
    }
}

fn operations(shared: &Shared) -> Result<OperationManager, Response> {
    lock(&shared.live).operations.clone().ok_or_else(unavailable_session)
}

async fn now_capabilities(shared: &Shared) -> Response {
    let operations = match operations(shared) {
        Ok(operations) => operations,
        Err(response) => return response,
    };
    match operations.capabilities().await {
        Ok(capabilities) => Response::Ok(Payload::NowCapabilities(capabilities)),
        Err(error) => Response::Err(error),
    }
}

async fn now_run(shared: &Shared, command: String, directory: Option<String>) -> Response {
    let operations = match operations(shared) {
        Ok(operations) => operations,
        Err(response) => return response,
    };
    match operations.run(command, directory).await {
        Ok(()) => Response::ok(),
        Err(error) => Response::Err(error),
    }
}

async fn now_execute(shared: &Shared, request: ironrdp_agent::ipc::NowExecutionRequest) -> ConnectionResponse {
    let operations = match operations(shared) {
        Ok(operations) => operations,
        Err(response) => return ConnectionResponse::Single(response),
    };
    match operations.execute(request).await {
        Ok(info) if info.detached => ConnectionResponse::Single(Response::Ok(Payload::NowOperation(info))),
        Ok(info) => match operations.attach(info.id, None) {
            Ok(attachment) => ConnectionResponse::Stream(Response::Ok(Payload::NowOperation(info)), attachment),
            Err(error) => ConnectionResponse::Single(Response::Err(error)),
        },
        Err(error) => ConnectionResponse::Single(Response::Err(error)),
    }
}

async fn now_cancel(shared: &Shared, operation_id: u64) -> Response {
    let operations = match operations(shared) {
        Ok(operations) => operations,
        Err(response) => return response,
    };
    operations
        .cancel(operation_id)
        .await
        .map_or_else(Response::Err, |_| Response::ok())
}

fn now_list(shared: &Shared) -> ConnectionResponse {
    match operations(shared) {
        Ok(operations) => ConnectionResponse::Single(Response::Ok(Payload::NowOperations(operations.list()))),
        Err(response) => ConnectionResponse::Single(response),
    }
}

fn now_status(shared: &Shared, operation_id: u64) -> ConnectionResponse {
    match operations(shared).and_then(|operations| {
        operations
            .status(operation_id)
            .map(|operation| Response::Ok(Payload::NowOperation(operation)))
            .map_err(Response::Err)
    }) {
        Ok(response) | Err(response) => ConnectionResponse::Single(response),
    }
}

fn now_attach(shared: &Shared, operation_id: u64, after_sequence: Option<u64>) -> ConnectionResponse {
    match operations(shared).and_then(|operations| {
        operations
            .attach(operation_id, after_sequence)
            .map(|attachment| (attachment.info.clone(), attachment))
            .map_err(Response::Err)
    }) {
        Ok((info, attachment)) => ConnectionResponse::Stream(Response::Ok(Payload::NowOperation(info)), attachment),
        Err(response) => ConnectionResponse::Single(response),
    }
}

async fn now_stdin(shared: &Shared, operation_id: u64, data: Vec<u8>, last: bool) -> Response {
    let operations = match operations(shared) {
        Ok(operations) => operations,
        Err(response) => return response,
    };
    operations
        .send_stdin(operation_id, data, last)
        .await
        .map_or_else(Response::Err, |_| Response::ok())
}

async fn now_diagnostics(shared: &Shared) -> Response {
    let endpoint = match lock(&shared.live).now_endpoint.clone() {
        Some(endpoint) => endpoint,
        None => return unavailable_session(),
    };
    let (connected, capabilities) = endpoint.diagnostic_snapshot().await;
    Response::Ok(Payload::NowDiagnostics(NowDiagnostics {
        endpoint_allocated: true,
        connected,
        capabilities: capabilities.map(|capabilities| ironrdp_agent::ipc::NowCapabilities {
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
    }))
}

fn downscale_frame(frame: Frame, max_pixels: usize) -> Frame {
    let source_width = usize::from(frame.width);
    let source_height = usize::from(frame.height);
    let source_pixels = source_width * source_height;
    if source_pixels <= max_pixels {
        return frame;
    }

    let mut lower = 1usize;
    let mut upper = source_width;
    while lower < upper {
        let candidate = (lower + upper).div_ceil(2);
        let candidate_height = (source_height * candidate / source_width).max(1);
        if candidate * candidate_height <= max_pixels {
            lower = candidate;
        } else {
            upper = candidate - 1;
        }
    }

    let width = lower;
    let height = (source_height * width / source_width).max(1);
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        let source_y = y * source_height / height;
        for x in 0..width {
            let source_x = x * source_width / width;
            pixels.push(frame.pixels[source_y * source_width + source_x]);
        }
    }

    Frame {
        width: u16::try_from(width).expect("scaled frame width fits source width"),
        height: u16::try_from(height).expect("scaled frame height fits source height"),
        pixels,
    }
}

fn unavailable_session() -> Response {
    Response::typed_error(AgentErrorCategory::Unavailable, "no active RDP session")
}

fn respond(command: Command, response: Response) {
    let sender = match command {
        Command::Connect { response, .. }
        | Command::Disconnect { response }
        | Command::Input { response, .. }
        | Command::Resize { response, .. } => response,
    };
    let _ = sender.send(response);
}

fn endpoint_from_environment() -> Endpoint {
    std::env::var("IRONRDP_ACTIVEX_RPC_ENDPOINT")
        .map(transport::endpoint_from_string)
        .unwrap_or_else(|_| transport::default_endpoint_named("ironrdp-activex"))
}

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

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rpc() -> ActiveXRpc {
        ActiveXRpc {
            shared: Arc::new(Shared {
                live: Mutex::new(Live {
                    state: ConnState::NoSession,
                    destination: None,
                    properties: PropertySet::new(),
                    frame: None,
                    error: None,
                    now_endpoint: None,
                    operations: None,
                }),
                logs: LogBuffer::new(),
                commands: Mutex::new(VecDeque::new()),
                command_posted: AtomicBool::new(false),
                closing: AtomicBool::new(false),
            }),
            listener: Arc::new(Mutex::new(None)),
        }
    }

    #[test]
    fn screenshot_without_a_frame_is_unavailable() {
        let rpc = rpc();
        assert!(!screenshot(&rpc.shared).is_ok());
    }

    #[tokio::test]
    async fn unicode_text_is_explicitly_unsupported() {
        let rpc = rpc();

        let ConnectionResponse::Single(Response::Err(error)) = handle_request(
            &rpc.shared,
            0,
            Request::UnicodeText {
                text: "test".to_owned(),
            },
        )
        .await
        else {
            panic!("bulk Unicode text input must be rejected");
        };
        assert_eq!(error.category, AgentErrorCategory::InvalidRequest);
        assert_eq!(error.message, "bulk Unicode text input is unsupported by ActiveX");
    }

    #[tokio::test]
    async fn rail_audit_is_explicitly_unavailable() {
        let rpc = rpc();

        let ConnectionResponse::Single(Response::Err(error)) =
            handle_request(&rpc.shared, 0, Request::RailStatus).await
        else {
            panic!("RAIL audit endpoints must be unavailable through ActiveX");
        };
        assert_eq!(error.category, AgentErrorCategory::Unavailable);
        assert_eq!(error.message, "RAIL audit endpoints are unavailable through ActiveX");
    }

    #[test]
    fn session_lifecycle_retains_and_clears_cursor_composited_frames() {
        let rpc = rpc();
        let mut properties = PropertySet::new();
        properties.insert("username", "alice");
        let endpoint = Arc::new(NowEndpoint::new().expect("allocate NOW endpoint"));
        rpc.session_started("server.example:3389".to_owned(), properties, endpoint);
        rpc.session_connected();
        rpc.retain_frame(1, 1, &[0x0011_2233]);

        let Response::Ok(Payload::Status(initial_status)) = status(&rpc.shared) else {
            panic!("status must be available");
        };
        assert_eq!(initial_status.state, ConnState::Connected);
        assert_eq!(initial_status.destination.as_deref(), Some("server.example:3389"));
        assert_eq!((initial_status.width, initial_status.height), (Some(1), Some(1)));
        assert!(matches!(
            screenshot(&rpc.shared),
            Response::Ok(Payload::Screenshot {
                width: 1,
                height: 1,
                ..
            })
        ));

        rpc.session_stopped();
        let Response::Ok(Payload::Status(stopped_status)) = status(&rpc.shared) else {
            panic!("status must be available");
        };
        assert_eq!(stopped_status.state, ConnState::Disconnected);
        assert!(!screenshot(&rpc.shared).is_ok());
    }

    #[test]
    fn rdcleanpath_token_is_not_exposed_in_session_properties() {
        let rpc = rpc();
        let mut properties = PropertySet::new();
        properties.insert("RDCleanPathUrl", "wss://rdcleanpath.example.test/rdp");
        properties.insert("RDCleanPathToken", "test-token");
        let endpoint = Arc::new(NowEndpoint::new().expect("allocate NOW endpoint"));

        rpc.session_started("server.example:3389".to_owned(), properties, endpoint);

        let Response::Ok(Payload::Properties(properties)) = query_props(&rpc.shared, None) else {
            panic!("session properties must be available");
        };
        assert!(properties.entries.iter().any(|entry| entry.key == "RDCleanPathUrl"));
        assert!(!properties.entries.iter().any(|entry| entry.key == "RDCleanPathToken"));
    }

    #[test]
    fn downscale_frame_preserves_aspect_ratio_within_pixel_limit() {
        let frame = Frame {
            width: 4,
            height: 2,
            pixels: (0..8).collect(),
        };

        let scaled = downscale_frame(frame, 3);

        assert_eq!((scaled.width, scaled.height), (3, 1));
        assert_eq!(scaled.pixels, vec![0, 1, 2]);
    }
}
