#![allow(clippy::print_stdout, clippy::print_stderr)]
#![allow(unused_crate_dependencies)]

use core::convert::Infallible;
use core::str::FromStr;
use core::time::Duration;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::Context as _;
use bytes::Bytes;
use clap::{Args, Parser, Subcommand, ValueEnum};
use http_body_util::{BodyExt as _, Full};
use hyper::body::Incoming;
use hyper::header::{CONTENT_TYPE, HeaderValue};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use ironrdp::input::{self, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp_client::config::{ClipboardType, KeyboardType, PartialConfig};
use ironrdp_client::rdp::{DvcPipeProxyFactory, RdpClient, RdpInputEvent, RdpOutputEvent};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Mutex, Notify, RwLock, mpsc};
use tracing::metadata::LevelFilter;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

#[cfg(windows)]
const DEFAULT_ENDPOINT: &str = "pipe:ironrdp-agent";
#[cfg(unix)]
const DEFAULT_ENDPOINT: &str = "unix:/tmp/ironrdp-agent.sock";

type ResponseBody = Full<Bytes>;

trait AsyncReadWrite: AsyncRead + AsyncWrite {}

impl<T> AsyncReadWrite for T where T: AsyncRead + AsyncWrite {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Endpoint {
    Pipe(String),
    Unix(PathBuf),
}

impl FromStr for Endpoint {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> anyhow::Result<Self> {
        if let Some(name) = value.strip_prefix("pipe:") {
            if name.is_empty() {
                anyhow::bail!("pipe endpoint name is empty");
            }

            Ok(Self::Pipe(name.to_owned()))
        } else if let Some(path) = value.strip_prefix("unix:") {
            if path.is_empty() {
                anyhow::bail!("unix endpoint path is empty");
            }

            Ok(Self::Unix(PathBuf::from(path)))
        } else if cfg!(windows) {
            Ok(Self::Pipe(value.to_owned()))
        } else {
            Ok(Self::Unix(PathBuf::from(value)))
        }
    }
}

impl core::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Pipe(name) => write!(f, "pipe:{name}"),
            Self::Unix(path) => write!(f, "unix:{}", path.display()),
        }
    }
}

#[derive(Parser, Debug)]
#[clap(author = "Devolutions", about = "Agentic IronRDP client daemon and CLI")]
#[clap(version, long_about = None)]
struct Cli {
    /// IPC endpoint used by both daemon and client modes.
    #[clap(long, default_value = DEFAULT_ENDPOINT)]
    endpoint: Endpoint,

    /// Default logging level when IRONRDP_LOG and --log-filter are not set.
    #[clap(long, value_enum, default_value_t = LogLevel::Warn)]
    log_level: LogLevel,

    /// Tracing filter directives. Overrides --log-level and IRONRDP_LOG when set.
    #[clap(long)]
    log_filter: Option<String>,

    /// Write logs to this file instead of stderr.
    #[clap(long)]
    log_file: Option<PathBuf>,

    /// Do not spawn the daemon automatically when a client command cannot connect.
    #[clap(long)]
    no_spawn_daemon: bool,

    #[clap(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    fn as_level_filter(self) -> LevelFilter {
        match self {
            Self::Error => LevelFilter::ERROR,
            Self::Warn => LevelFilter::WARN,
            Self::Info => LevelFilter::INFO,
            Self::Debug => LevelFilter::DEBUG,
            Self::Trace => LevelFilter::TRACE,
        }
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the IPC daemon in the foreground.
    Daemon,
    /// Connect an RDP session through the daemon.
    Connect(Box<ConnectCommand>),
    /// List daemon sessions.
    Sessions,
    /// Get daemon or session status.
    Status(SessionArg),
    /// Disconnect a session.
    Disconnect(SessionArg),
    /// Send mouse input.
    Mouse(MouseCommand),
    /// Send keyboard input.
    Keyboard(KeyboardCommand),
    /// Resize a session.
    Resize(ResizeCommand),
    /// Wait for a framebuffer update.
    WaitFrame(WaitFrameCommand),
    /// Save the latest session framebuffer as PNG.
    Screenshot(ScreenshotCommand),
}

#[derive(Args, Debug)]
struct SessionArg {
    #[clap(long)]
    session: Option<String>,
}

#[derive(Args, Debug)]
struct ConnectCommand {
    /// RDP server host or host:port.
    destination: Option<String>,

    #[clap(short, long)]
    username: Option<String>,

    #[clap(short, long)]
    domain: Option<String>,

    #[clap(short, long, conflicts_with = "password_env")]
    password: Option<String>,

    /// Environment variable containing the RDP password.
    #[clap(long)]
    password_env: Option<String>,

    #[clap(long)]
    gw_endpoint: Option<String>,

    #[clap(long)]
    gw_user: Option<String>,

    #[clap(long)]
    gw_pass: Option<String>,

    #[clap(long)]
    rdp_file: Option<PathBuf>,

    #[clap(long, value_enum, default_value_t = KeyboardType::IbmEnhanced)]
    keyboard_type: KeyboardType,

    #[clap(long, default_value_t = 0)]
    keyboard_subtype: u32,

    #[clap(long, default_value_t = 12)]
    keyboard_functional_keys_count: u32,

    #[clap(long, default_value_t = String::new())]
    ime_file_name: String,

    #[clap(long, default_value_t = String::new())]
    dig_product_id: String,

    #[clap(long)]
    thin_client: bool,

    #[clap(long)]
    small_cache: bool,

    /// Desired desktop size for the RDP session, formatted as WxH (for example, 1920x1080).
    #[clap(long, value_parser = parse_desktop_size, conflicts_with_all = ["desktop_width", "desktop_height"])]
    desktop_size: Option<DesktopSize>,

    #[clap(long, value_parser = clap::value_parser!(u16).range(1..=8192))]
    desktop_width: Option<u16>,

    #[clap(long, value_parser = clap::value_parser!(u16).range(1..=8192))]
    desktop_height: Option<u16>,

    #[clap(long, value_parser = clap::value_parser!(u32).range(100..=500))]
    scale_desktop: Option<u32>,

    #[clap(long)]
    color_depth: Option<u32>,

    #[clap(long)]
    no_server_pointer: bool,

    #[clap(long, value_parser = parse_u32)]
    capabilities: Option<u32>,

    #[clap(long)]
    autologon: bool,

    #[clap(long)]
    no_tls: bool,

    #[clap(long, alias = "no-nla")]
    no_credssp: bool,

    #[clap(long, action = clap::ArgAction::Set)]
    compression_enabled: Option<bool>,

    #[clap(long, value_parser = clap::value_parser!(u32).range(0..=3))]
    compression_level: Option<u32>,

    #[clap(long, value_enum, default_value_t = ClipboardType::Default)]
    clipboard_type: ClipboardType,

    #[clap(long, num_args = 1.., value_delimiter = ',')]
    codecs: Vec<String>,

    #[clap(long)]
    prevent_session_lock: Option<u32>,

    #[clap(long)]
    dvc_proxy: Vec<String>,

    #[cfg(windows)]
    #[clap(long)]
    dvc_plugin: Vec<PathBuf>,

    #[clap(long)]
    rdcleanpath_url: Option<String>,

    #[clap(long)]
    rdcleanpath_token: Option<String>,
}

impl ConnectCommand {
    fn to_ironrdp_args(&self) -> anyhow::Result<Vec<String>> {
        if self.rdp_file.is_none() {
            if self.destination.is_none() {
                anyhow::bail!("missing destination or .rdp file");
            }

            if self.username.is_none() {
                anyhow::bail!("missing username or .rdp file");
            }

            if self.password.is_none() && self.password_env.is_none() {
                anyhow::bail!("missing password, password environment variable, or .rdp file");
            }
        }

        let mut args = vec!["ironrdp-agent".to_owned()];

        if let Some(destination) = &self.destination {
            args.push(destination.clone());
        }

        push_option(&mut args, "--username", self.username.as_deref());
        push_option(&mut args, "--domain", self.domain.as_deref());
        match (&self.password, &self.password_env) {
            (Some(password), None) => push_option(&mut args, "--password", Some(password.as_str())),
            (None, Some(env_name)) => {
                let password =
                    std::env::var(env_name).with_context(|| format!("failed to read password from {env_name}"))?;
                args.push("--password".to_owned());
                args.push(password);
            }
            (None, None) => {}
            (Some(_), Some(_)) => anyhow::bail!("password and password-env are mutually exclusive"),
        }
        push_option(&mut args, "--gw-endpoint", self.gw_endpoint.as_deref());
        push_option(&mut args, "--gw-user", self.gw_user.as_deref());
        push_option(&mut args, "--gw-pass", self.gw_pass.as_deref());
        push_path_option(&mut args, "--rdp-file", self.rdp_file.as_ref());
        push_value_enum(&mut args, "--keyboard-type", self.keyboard_type);
        push_display_option(&mut args, "--keyboard-subtype", Some(self.keyboard_subtype));
        push_display_option(
            &mut args,
            "--keyboard-functional-keys-count",
            Some(self.keyboard_functional_keys_count),
        );
        push_option(
            &mut args,
            "--ime-file-name",
            (!self.ime_file_name.is_empty()).then_some(self.ime_file_name.as_str()),
        );
        push_option(
            &mut args,
            "--dig-product-id",
            (!self.dig_product_id.is_empty()).then_some(self.dig_product_id.as_str()),
        );
        if let Some(size) = self.desktop_size {
            push_display_option(&mut args, "--desktop-width", Some(size.width));
            push_display_option(&mut args, "--desktop-height", Some(size.height));
        } else {
            push_display_option(&mut args, "--desktop-width", self.desktop_width);
            push_display_option(&mut args, "--desktop-height", self.desktop_height);
        }
        push_display_option(&mut args, "--scale-desktop", self.scale_desktop);
        push_display_option(&mut args, "--color-depth", self.color_depth);
        push_display_option(&mut args, "--capabilities", self.capabilities);
        push_display_option(&mut args, "--compression-level", self.compression_level);
        push_value_enum(&mut args, "--clipboard-type", self.clipboard_type);
        push_display_option(&mut args, "--prevent-session-lock", self.prevent_session_lock);
        push_option(&mut args, "--rdcleanpath-url", self.rdcleanpath_url.as_deref());
        push_option(&mut args, "--rdcleanpath-token", self.rdcleanpath_token.as_deref());

        if self.thin_client {
            args.push("--thin-client".to_owned());
        }

        if self.small_cache {
            args.push("--small-cache".to_owned());
        }

        if self.no_server_pointer {
            args.push("--no-server-pointer".to_owned());
        }

        if self.autologon {
            args.push("--autologon".to_owned());
        }

        if self.no_tls {
            args.push("--no-tls".to_owned());
        }

        if self.no_credssp {
            args.push("--no-credssp".to_owned());
        }

        if let Some(enabled) = self.compression_enabled {
            args.push("--compression-enabled".to_owned());
            args.push(enabled.to_string());
        }

        if !self.codecs.is_empty() {
            args.push("--codecs".to_owned());
            args.push(self.codecs.join(","));
        }

        for dvc_proxy in &self.dvc_proxy {
            args.push("--dvc-proxy".to_owned());
            args.push(dvc_proxy.clone());
        }

        #[cfg(windows)]
        for dvc_plugin in &self.dvc_plugin {
            args.push("--dvc-plugin".to_owned());
            args.push(dvc_plugin.display().to_string());
        }

        Ok(args)
    }
}

fn push_option(args: &mut Vec<String>, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        args.push(name.to_owned());
        args.push(value.to_owned());
    }
}

fn push_path_option(args: &mut Vec<String>, name: &str, value: Option<&PathBuf>) {
    if let Some(value) = value {
        args.push(name.to_owned());
        args.push(value.display().to_string());
    }
}

fn push_value_enum<T>(args: &mut Vec<String>, name: &str, value: T)
where
    T: ValueEnum,
{
    if let Some(value) = value.to_possible_value() {
        args.push(name.to_owned());
        args.push(value.get_name().to_owned());
    }
}

fn push_display_option<T>(args: &mut Vec<String>, name: &str, value: Option<T>)
where
    T: ToString,
{
    if let Some(value) = value {
        args.push(name.to_owned());
        args.push(value.to_string());
    }
}

#[derive(Args, Debug)]
struct MouseCommand {
    #[clap(long)]
    session: String,
    #[clap(subcommand)]
    action: MouseAction,
}

#[derive(Subcommand, Debug)]
enum MouseAction {
    Move {
        #[clap(long)]
        x: u16,
        #[clap(long)]
        y: u16,
    },
    Click {
        #[clap(long, value_enum)]
        button: MouseButton,
        #[clap(long)]
        x: Option<u16>,
        #[clap(long)]
        y: Option<u16>,
    },
    Down {
        #[clap(long, value_enum)]
        button: MouseButton,
    },
    Up {
        #[clap(long, value_enum)]
        button: MouseButton,
    },
    Wheel {
        #[clap(long)]
        units: i16,
        #[clap(long)]
        horizontal: bool,
    },
    Position,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum MouseButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

impl From<MouseButton> for input::MouseButton {
    fn from(value: MouseButton) -> Self {
        match value {
            MouseButton::Left => Self::Left,
            MouseButton::Middle => Self::Middle,
            MouseButton::Right => Self::Right,
            MouseButton::X1 => Self::X1,
            MouseButton::X2 => Self::X2,
        }
    }
}

#[derive(Args, Debug)]
struct KeyboardCommand {
    #[clap(long)]
    session: String,
    #[clap(subcommand)]
    action: KeyboardAction,
}

#[derive(Subcommand, Debug)]
enum KeyboardAction {
    Key {
        #[clap(long, value_parser = parse_scancode)]
        scancode: u16,
        #[clap(long)]
        release: bool,
    },
    Text {
        #[clap(long)]
        text: String,
    },
    Shortcut {
        #[clap(long, value_delimiter = ',', value_parser = parse_scancode)]
        scancodes: Vec<u16>,
    },
    ReleaseAll,
}

fn parse_scancode(input: &str) -> Result<u16, String> {
    if let Some(hex) = input.strip_prefix("0x") {
        u16::from_str_radix(hex, 16).map_err(|e| e.to_string())
    } else {
        input.parse::<u16>().map_err(|e| e.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DesktopSize {
    width: u16,
    height: u16,
}

fn parse_desktop_size(input: &str) -> Result<DesktopSize, String> {
    let (width, height) = input
        .split_once('x')
        .or_else(|| input.split_once('X'))
        .ok_or_else(|| "expected desktop size in WxH format, for example 1920x1080".to_owned())?;

    let width = parse_dimension(width, "width")?;
    let height = parse_dimension(height, "height")?;

    Ok(DesktopSize { width, height })
}

fn parse_dimension(input: &str, name: &str) -> Result<u16, String> {
    let value = input
        .parse::<u16>()
        .map_err(|e| format!("invalid desktop {name}: {e}"))?;

    if (1..=8192).contains(&value) {
        Ok(value)
    } else {
        Err(format!("desktop {name} must be between 1 and 8192"))
    }
}

fn parse_u32(input: &str) -> Result<u32, String> {
    if let Some(hex) = input.strip_prefix("0x") {
        u32::from_str_radix(hex, 16).map_err(|e| e.to_string())
    } else {
        input.parse::<u32>().map_err(|e| e.to_string())
    }
}

#[derive(Args, Debug)]
struct ResizeCommand {
    #[clap(long)]
    session: String,
    #[clap(long)]
    width: u16,
    #[clap(long)]
    height: u16,
    #[clap(long, default_value_t = 100)]
    scale: u32,
}

#[derive(Args, Debug)]
struct WaitFrameCommand {
    #[clap(long)]
    session: String,
    #[clap(long, default_value_t = 30_000)]
    timeout_ms: u64,
    /// Wait until the session has a framebuffer sequence greater than this value.
    #[clap(long)]
    after_frame: Option<u64>,
}

#[derive(Args, Debug)]
struct ScreenshotCommand {
    #[clap(long)]
    session: String,
    #[clap(long)]
    output: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
struct ConnectRequest {
    argv: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ConnectResponse {
    session_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct SessionsResponse {
    sessions: Vec<SessionSummary>,
}

#[derive(Debug, Deserialize, Serialize)]
struct SessionSummary {
    session_id: String,
    status: SessionStatus,
    width: Option<u16>,
    height: Option<u16>,
    frame_sequence: u64,
    mouse_x: u16,
    mouse_y: u16,
    last_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum SessionStatus {
    Connecting,
    Connected,
    Failed,
    Disconnected,
}

#[derive(Debug, Deserialize, Serialize)]
struct MouseRequest {
    action: MouseRequestAction,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
enum MouseRequestAction {
    Move {
        x: u16,
        y: u16,
    },
    Click {
        button: MouseButton,
        x: Option<u16>,
        y: Option<u16>,
    },
    Down {
        button: MouseButton,
    },
    Up {
        button: MouseButton,
    },
    Wheel {
        units: i16,
        horizontal: bool,
    },
}

#[derive(Debug, Deserialize, Serialize)]
struct KeyboardRequest {
    action: KeyboardRequestAction,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
enum KeyboardRequestAction {
    Key { scancode: u16, release: bool },
    Text { text: String },
    Shortcut { scancodes: Vec<u16> },
    ReleaseAll,
}

#[derive(Debug, Deserialize, Serialize)]
struct ResizeRequest {
    width: u16,
    height: u16,
    scale: u32,
}

#[derive(Debug, Deserialize, Serialize)]
struct WaitFrameRequest {
    timeout_ms: u64,
    after_frame: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
struct MousePositionResponse {
    x: u16,
    y: u16,
}

#[derive(Debug, Deserialize, Serialize)]
struct ErrorResponse {
    error: String,
}

struct Daemon {
    sessions: RwLock<HashMap<String, Arc<SessionEntry>>>,
}

impl Daemon {
    fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }

    async fn create_session(&self, request: ConnectRequest) -> anyhow::Result<ConnectResponse> {
        let partial = PartialConfig::parse_from(request.argv).context("configuration arguments")?;
        let config = partial.into_config().context("configuration")?;
        let session_id = Uuid::new_v4().to_string();
        let snapshot = Arc::new(RwLock::new(SessionSnapshot::new(session_id.clone())));
        let notify = Arc::new(Notify::new());
        let input_database = Arc::new(Mutex::new(input::Database::new()));
        let (input_sender, input_receiver) = RdpInputEvent::create_channel();
        let (output_sender, output_receiver) = mpsc::unbounded_channel();
        let dvc_pipe_proxy_factory = DvcPipeProxyFactory::new(input_sender.clone());

        let client = RdpClient {
            config,
            output_event_sender: Box::new(output_sender),
            input_event_receiver: input_receiver,
            cliprdr_factory: None,
            dvc_pipe_proxy_factory,
        };

        std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime) => runtime,
                Err(error) => {
                    error!(%error, "Failed to create RDP session runtime");
                    return;
                }
            };

            runtime.block_on(client.run());
        });
        tokio::spawn(process_output_events(
            Arc::clone(&snapshot),
            Arc::clone(&notify),
            output_receiver,
        ));

        let entry = Arc::new(SessionEntry {
            session_id: session_id.clone(),
            input_sender,
            input_database,
            snapshot,
            notify,
        });

        self.sessions.write().await.insert(session_id.clone(), entry);

        Ok(ConnectResponse { session_id })
    }

    async fn session(&self, session_id: &str) -> Result<Arc<SessionEntry>, ApiError> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "session not found"))
    }

    async fn summaries(&self) -> SessionsResponse {
        let sessions = self.sessions.read().await;
        let mut summaries = Vec::with_capacity(sessions.len());

        for entry in sessions.values() {
            summaries.push(entry.summary().await);
        }

        SessionsResponse { sessions: summaries }
    }
}

struct SessionEntry {
    session_id: String,
    input_sender: mpsc::UnboundedSender<RdpInputEvent>,
    input_database: Arc<Mutex<input::Database>>,
    snapshot: Arc<RwLock<SessionSnapshot>>,
    notify: Arc<Notify>,
}

impl SessionEntry {
    async fn summary(&self) -> SessionSummary {
        let snapshot = self.snapshot.read().await;
        let mouse_position = self.input_database.lock().await.mouse_position();

        SessionSummary {
            session_id: self.session_id.clone(),
            status: snapshot.status.clone(),
            width: snapshot.frame.as_ref().map(|frame| frame.width),
            height: snapshot.frame.as_ref().map(|frame| frame.height),
            frame_sequence: snapshot.frame_sequence,
            mouse_x: mouse_position.x,
            mouse_y: mouse_position.y,
            last_error: snapshot.last_error.clone(),
        }
    }

    async fn apply_operations(&self, operations: impl IntoIterator<Item = Operation>) -> Result<(), ApiError> {
        let mut database = self.input_database.lock().await;
        let events = database.apply(operations);

        if !events.is_empty() {
            self.input_sender
                .send(RdpInputEvent::FastPath(events))
                .map_err(|_| ApiError::new(StatusCode::CONFLICT, "session input channel is closed"))?;
        }

        Ok(())
    }

    async fn mouse_position(&self) -> MousePositionResponse {
        let position = self.input_database.lock().await.mouse_position();

        MousePositionResponse {
            x: position.x,
            y: position.y,
        }
    }

    async fn wait_frame(&self, timeout: Duration, after_frame: Option<u64>) -> Result<(), ApiError> {
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
        .map_err(|_| ApiError::new(StatusCode::REQUEST_TIMEOUT, "timed out waiting for frame"))?;

        if self.has_requested_frame(after_frame).await {
            Ok(())
        } else {
            Err(ApiError::new(StatusCode::CONFLICT, "session has no frame"))
        }
    }

    async fn has_requested_frame(&self, after_frame: Option<u64>) -> bool {
        let snapshot = self.snapshot.read().await;
        snapshot.frame.is_some() && after_frame.is_none_or(|after_frame| snapshot.frame_sequence > after_frame)
    }

    async fn screenshot_png(&self) -> Result<Vec<u8>, ApiError> {
        let snapshot = self.snapshot.read().await;
        let frame = snapshot
            .frame
            .as_ref()
            .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "session has no frame"))?;

        frame.to_png()
    }
}

#[derive(Clone)]
struct Frame {
    buffer: Vec<u32>,
    width: u16,
    height: u16,
}

impl Frame {
    fn to_png(&self) -> Result<Vec<u8>, ApiError> {
        let mut rgba = Vec::with_capacity(self.buffer.len() * 4);

        for pixel in &self.buffer {
            let [_, r, g, b] = pixel.to_be_bytes();
            rgba.extend_from_slice(&[r, g, b, 255]);
        }

        let image =
            image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(u32::from(self.width), u32::from(self.height), rgba)
                .ok_or_else(|| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "invalid framebuffer"))?;
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

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
    fn new(_session_id: String) -> Self {
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

async fn process_output_events(
    snapshot: Arc<RwLock<SessionSnapshot>>,
    notify: Arc<Notify>,
    mut output_receiver: mpsc::UnboundedReceiver<RdpOutputEvent>,
) {
    while let Some(event) = output_receiver.recv().await {
        match event {
            RdpOutputEvent::Image { buffer, width, height } => {
                let mut snapshot = snapshot.write().await;
                snapshot.status = SessionStatus::Connected;
                snapshot.frame_sequence = snapshot.frame_sequence.saturating_add(1);
                snapshot.frame = Some(Frame {
                    buffer,
                    width: width.get(),
                    height: height.get(),
                });
                notify.notify_waiters();
            }
            RdpOutputEvent::ConnectionFailure(error) => {
                let mut snapshot = snapshot.write().await;
                snapshot.status = SessionStatus::Failed;
                snapshot.last_error = Some(error.report().to_string());
                notify.notify_waiters();
            }
            RdpOutputEvent::Terminated(result) => {
                let mut snapshot = snapshot.write().await;
                snapshot.status = match &result {
                    Ok(_) => SessionStatus::Disconnected,
                    Err(_) => SessionStatus::Failed,
                };
                snapshot.last_error = match result {
                    Ok(reason) => Some(reason.to_string()),
                    Err(error) => Some(error.report().to_string()),
                };
                notify.notify_waiters();
            }
            RdpOutputEvent::PointerPosition { x, y } => {
                let mut snapshot = snapshot.write().await;
                snapshot.pointer_x = x;
                snapshot.pointer_y = y;
            }
            RdpOutputEvent::PointerDefault | RdpOutputEvent::PointerHidden | RdpOutputEvent::PointerBitmap(_) => {}
        }
    }
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    setup_logging(cli.log_level, cli.log_filter.as_deref(), cli.log_file.as_deref())
        .context("unable to initialize logging")?;

    match cli.command {
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
    hyper::server::conn::http1::Builder::new()
        .serve_connection(
            TokioIo::new(stream),
            service_fn(move |request| handle_request(request, Arc::clone(&daemon))),
        )
        .await
        .context("serve IPC connection")
}

async fn handle_request(request: Request<Incoming>, daemon: Arc<Daemon>) -> Result<Response<ResponseBody>, Infallible> {
    let response = match route_request(request, daemon).await {
        Ok(response) => response,
        Err(error) => json_response(error.status, &ErrorResponse { error: error.message }),
    };

    Ok(response)
}

async fn route_request(request: Request<Incoming>, daemon: Arc<Daemon>) -> Result<Response<ResponseBody>, ApiError> {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let segments: Vec<_> = path
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();

    match (method, segments.as_slice()) {
        (Method::GET, ["health"]) => Ok(json_response(StatusCode::OK, &serde_json::json!({ "ok": true }))),
        (Method::POST, ["sessions"]) => {
            let request = read_json::<ConnectRequest>(request).await?;
            let response = daemon
                .create_session(request)
                .await
                .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.to_string()))?;
            Ok(json_response(StatusCode::OK, &response))
        }
        (Method::GET, ["sessions"]) => Ok(json_response(StatusCode::OK, &daemon.summaries().await)),
        (Method::GET, ["sessions", session_id]) => {
            let session = daemon.session(session_id).await?;
            Ok(json_response(StatusCode::OK, &session.summary().await))
        }
        (Method::DELETE, ["sessions", session_id]) => {
            let session = daemon.session(session_id).await?;
            session
                .input_sender
                .send(RdpInputEvent::Close)
                .map_err(|_| ApiError::new(StatusCode::CONFLICT, "session input channel is closed"))?;
            Ok(empty_response(StatusCode::NO_CONTENT))
        }
        (Method::POST, ["sessions", session_id, "mouse"]) => {
            let request = read_json::<MouseRequest>(request).await?;
            let session = daemon.session(session_id).await?;
            apply_mouse_request(&session, request).await?;
            Ok(empty_response(StatusCode::NO_CONTENT))
        }
        (Method::GET, ["sessions", session_id, "mouse", "position"]) => {
            let session = daemon.session(session_id).await?;
            Ok(json_response(StatusCode::OK, &session.mouse_position().await))
        }
        (Method::POST, ["sessions", session_id, "keyboard"]) => {
            let request = read_json::<KeyboardRequest>(request).await?;
            let session = daemon.session(session_id).await?;
            apply_keyboard_request(&session, request).await?;
            Ok(empty_response(StatusCode::NO_CONTENT))
        }
        (Method::POST, ["sessions", session_id, "resize"]) => {
            let request = read_json::<ResizeRequest>(request).await?;
            let session = daemon.session(session_id).await?;
            session
                .input_sender
                .send(RdpInputEvent::Resize {
                    width: request.width,
                    height: request.height,
                    scale_factor: request.scale,
                    physical_size: None,
                })
                .map_err(|_| ApiError::new(StatusCode::CONFLICT, "session input channel is closed"))?;
            Ok(empty_response(StatusCode::NO_CONTENT))
        }
        (Method::POST, ["sessions", session_id, "wait-frame"]) => {
            let request = read_json::<WaitFrameRequest>(request).await?;
            let session = daemon.session(session_id).await?;
            session
                .wait_frame(Duration::from_millis(request.timeout_ms), request.after_frame)
                .await?;
            Ok(empty_response(StatusCode::NO_CONTENT))
        }
        (Method::GET, ["sessions", session_id, "screenshot"]) => {
            let session = daemon.session(session_id).await?;
            let png = session.screenshot_png().await?;
            Ok(binary_response(StatusCode::OK, "image/png", png))
        }
        _ => Err(ApiError::new(StatusCode::NOT_FOUND, "endpoint not found")),
    }
}

async fn read_json<T>(request: Request<Incoming>) -> Result<T, ApiError>
where
    T: DeserializeOwned,
{
    let bytes = request
        .into_body()
        .collect()
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.to_string()))?
        .to_bytes();

    serde_json::from_slice(&bytes).map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.to_string()))
}

async fn apply_mouse_request(session: &SessionEntry, request: MouseRequest) -> Result<(), ApiError> {
    match request.action {
        MouseRequestAction::Move { x, y } => {
            session
                .apply_operations([Operation::MouseMove(MousePosition { x, y })])
                .await
        }
        MouseRequestAction::Click { button, x, y } => {
            let mut operations = Vec::new();
            if let (Some(x), Some(y)) = (x, y) {
                operations.push(Operation::MouseMove(MousePosition { x, y }));
            }
            let button = input::MouseButton::from(button);
            operations.push(Operation::MouseButtonPressed(button));
            operations.push(Operation::MouseButtonReleased(button));
            session.apply_operations(operations).await
        }
        MouseRequestAction::Down { button } => {
            session
                .apply_operations([Operation::MouseButtonPressed(input::MouseButton::from(button))])
                .await
        }
        MouseRequestAction::Up { button } => {
            session
                .apply_operations([Operation::MouseButtonReleased(input::MouseButton::from(button))])
                .await
        }
        MouseRequestAction::Wheel { units, horizontal } => {
            session
                .apply_operations([Operation::WheelRotations(WheelRotations {
                    is_vertical: !horizontal,
                    rotation_units: units,
                })])
                .await
        }
    }
}

async fn apply_keyboard_request(session: &SessionEntry, request: KeyboardRequest) -> Result<(), ApiError> {
    match request.action {
        KeyboardRequestAction::Key { scancode, release } => {
            let operation = if release {
                Operation::KeyReleased(Scancode::from_u16(scancode))
            } else {
                Operation::KeyPressed(Scancode::from_u16(scancode))
            };
            session.apply_operations([operation]).await
        }
        KeyboardRequestAction::Text { text } => {
            let operations = text
                .chars()
                .flat_map(|character| {
                    [
                        Operation::UnicodeKeyPressed(character),
                        Operation::UnicodeKeyReleased(character),
                    ]
                })
                .collect::<Vec<_>>();
            session.apply_operations(operations).await
        }
        KeyboardRequestAction::Shortcut { scancodes } => {
            let mut operations = Vec::with_capacity(scancodes.len() * 2);

            for scancode in &scancodes {
                operations.push(Operation::KeyPressed(Scancode::from_u16(*scancode)));
            }

            for scancode in scancodes.iter().rev() {
                operations.push(Operation::KeyReleased(Scancode::from_u16(*scancode)));
            }

            session.apply_operations(operations).await
        }
        KeyboardRequestAction::ReleaseAll => {
            let mut database = session.input_database.lock().await;
            let events = database.release_all();

            if !events.is_empty() {
                session
                    .input_sender
                    .send(RdpInputEvent::FastPath(events))
                    .map_err(|_| ApiError::new(StatusCode::CONFLICT, "session input channel is closed"))?;
            }

            Ok(())
        }
    }
}

fn json_response<T>(status: StatusCode, value: &T) -> Response<ResponseBody>
where
    T: Serialize,
{
    let body = serde_json::to_vec(value).expect("JSON response serialization");
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

fn binary_response(status: StatusCode, content_type: &'static str, body: Vec<u8>) -> Response<ResponseBody> {
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

fn empty_response(status: StatusCode) -> Response<ResponseBody> {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    response
}

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
    let request = client_request_from_command(command).await?;

    let response = match send_client_request(&endpoint, &request).await {
        Ok(response) => response,
        Err(error) if spawn_daemon => {
            debug!(%error, "Daemon request failed, spawning daemon");
            spawn_daemon_process(&endpoint, &daemon_logging).context("spawn daemon")?;
            wait_for_daemon(&endpoint).await.context("wait for daemon")?;
            send_client_request(&endpoint, &request).await?
        }
        Err(error) => return Err(error),
    };

    handle_client_response(response, request.output).await
}

struct ClientRequest {
    method: Method,
    path: String,
    body: Option<Vec<u8>>,
    output: ClientOutput,
}

enum ClientOutput {
    Json,
    None,
    Screenshot(PathBuf),
}

async fn client_request_from_command(command: Command) -> anyhow::Result<ClientRequest> {
    match command {
        Command::Connect(connect) => {
            let request = ConnectRequest {
                argv: connect.to_ironrdp_args()?,
            };
            json_client_request(Method::POST, "/sessions", &request, ClientOutput::Json)
        }
        Command::Sessions => Ok(ClientRequest {
            method: Method::GET,
            path: "/sessions".to_owned(),
            body: None,
            output: ClientOutput::Json,
        }),
        Command::Status(arg) => {
            let path = match arg.session {
                Some(session_id) => format!("/sessions/{session_id}"),
                None => "/health".to_owned(),
            };
            Ok(ClientRequest {
                method: Method::GET,
                path,
                body: None,
                output: ClientOutput::Json,
            })
        }
        Command::Disconnect(arg) => {
            let session_id = arg.session.context("missing --session")?;
            Ok(ClientRequest {
                method: Method::DELETE,
                path: format!("/sessions/{session_id}"),
                body: None,
                output: ClientOutput::None,
            })
        }
        Command::Mouse(command) => match command.action {
            MouseAction::Position => Ok(ClientRequest {
                method: Method::GET,
                path: format!("/sessions/{}/mouse/position", command.session),
                body: None,
                output: ClientOutput::Json,
            }),
            MouseAction::Move { x, y } => json_client_request(
                Method::POST,
                format!("/sessions/{}/mouse", command.session),
                &MouseRequest {
                    action: MouseRequestAction::Move { x, y },
                },
                ClientOutput::None,
            ),
            MouseAction::Click { button, x, y } => json_client_request(
                Method::POST,
                format!("/sessions/{}/mouse", command.session),
                &MouseRequest {
                    action: MouseRequestAction::Click { button, x, y },
                },
                ClientOutput::None,
            ),
            MouseAction::Down { button } => json_client_request(
                Method::POST,
                format!("/sessions/{}/mouse", command.session),
                &MouseRequest {
                    action: MouseRequestAction::Down { button },
                },
                ClientOutput::None,
            ),
            MouseAction::Up { button } => json_client_request(
                Method::POST,
                format!("/sessions/{}/mouse", command.session),
                &MouseRequest {
                    action: MouseRequestAction::Up { button },
                },
                ClientOutput::None,
            ),
            MouseAction::Wheel { units, horizontal } => json_client_request(
                Method::POST,
                format!("/sessions/{}/mouse", command.session),
                &MouseRequest {
                    action: MouseRequestAction::Wheel { units, horizontal },
                },
                ClientOutput::None,
            ),
        },
        Command::Keyboard(command) => {
            let action = match command.action {
                KeyboardAction::Key { scancode, release } => KeyboardRequestAction::Key { scancode, release },
                KeyboardAction::Text { text } => KeyboardRequestAction::Text { text },
                KeyboardAction::Shortcut { scancodes } => KeyboardRequestAction::Shortcut { scancodes },
                KeyboardAction::ReleaseAll => KeyboardRequestAction::ReleaseAll,
            };
            json_client_request(
                Method::POST,
                format!("/sessions/{}/keyboard", command.session),
                &KeyboardRequest { action },
                ClientOutput::None,
            )
        }
        Command::Resize(command) => json_client_request(
            Method::POST,
            format!("/sessions/{}/resize", command.session),
            &ResizeRequest {
                width: command.width,
                height: command.height,
                scale: command.scale,
            },
            ClientOutput::None,
        ),
        Command::WaitFrame(command) => json_client_request(
            Method::POST,
            format!("/sessions/{}/wait-frame", command.session),
            &WaitFrameRequest {
                timeout_ms: command.timeout_ms,
                after_frame: command.after_frame,
            },
            ClientOutput::None,
        ),
        Command::Screenshot(command) => Ok(ClientRequest {
            method: Method::GET,
            path: format!("/sessions/{}/screenshot", command.session),
            body: None,
            output: ClientOutput::Screenshot(command.output),
        }),
        Command::Daemon => anyhow::bail!("daemon cannot be used as a client command"),
    }
}

fn json_client_request<T>(
    method: Method,
    path: impl Into<String>,
    value: &T,
    output: ClientOutput,
) -> anyhow::Result<ClientRequest>
where
    T: Serialize,
{
    Ok(ClientRequest {
        method,
        path: path.into(),
        body: Some(serde_json::to_vec(value).context("serialize request")?),
        output,
    })
}

struct ClientResponse {
    status: StatusCode,
    bytes: Bytes,
}

async fn send_client_request(endpoint: &Endpoint, request: &ClientRequest) -> anyhow::Result<ClientResponse> {
    let stream = connect_endpoint(endpoint).await?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .context("HTTP handshake")?;

    tokio::spawn(async move {
        if let Err(error) = connection.await {
            warn!(%error, "IPC client connection failed");
        }
    });

    let body = request.body.clone().unwrap_or_default();
    let mut builder = Request::builder()
        .method(request.method.clone())
        .uri(request.path.as_str());

    if request.body.is_some() {
        builder = builder.header(CONTENT_TYPE, "application/json");
    }

    let response = sender
        .send_request(builder.body(Full::new(Bytes::from(body))).context("build request")?)
        .await
        .context("send request")?;

    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .context("read response")?
        .to_bytes();

    if !status.is_success() {
        if let Ok(error) = serde_json::from_slice::<ErrorResponse>(&bytes) {
            anyhow::bail!("{}", error.error);
        }

        anyhow::bail!("daemon returned HTTP {status}");
    }

    Ok(ClientResponse { status, bytes })
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
    let request = ClientRequest {
        method: Method::GET,
        path: "/health".to_owned(),
        body: None,
        output: ClientOutput::None,
    };

    for _ in 0..50 {
        if send_client_request(endpoint, &request).await.is_ok() {
            return Ok(());
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    anyhow::bail!("daemon did not become ready")
}

async fn handle_client_response(response: ClientResponse, output: ClientOutput) -> anyhow::Result<()> {
    match output {
        ClientOutput::Json => {
            let value: serde_json::Value = serde_json::from_slice(&response.bytes).context("parse response JSON")?;
            println!(
                "{}",
                serde_json::to_string_pretty(&value).context("format response JSON")?
            );
        }
        ClientOutput::None => {
            if response.status != StatusCode::NO_CONTENT && !response.bytes.is_empty() {
                println!("{}", String::from_utf8_lossy(&response.bytes));
            }
        }
        ClientOutput::Screenshot(path) => {
            std::fs::write(&path, &response.bytes).with_context(|| format!("failed to write {}", path.display()))?;
            println!("{}", path.display());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pipe_endpoint() {
        assert_eq!(
            Endpoint::from_str("pipe:ironrdp-agent").expect("endpoint"),
            Endpoint::Pipe("ironrdp-agent".to_owned())
        );
    }

    #[test]
    fn parses_scancodes() {
        assert_eq!(parse_scancode("0x1d").expect("hex"), 0x1d);
        assert_eq!(parse_scancode("42").expect("decimal"), 42);
    }

    #[test]
    fn parses_desktop_size() {
        assert_eq!(
            parse_desktop_size("1920x1080").expect("desktop size"),
            DesktopSize {
                width: 1920,
                height: 1080,
            }
        );
        assert_eq!(
            parse_desktop_size("800X600").expect("desktop size"),
            DesktopSize {
                width: 800,
                height: 600,
            }
        );
        assert!(parse_desktop_size("1920").is_err());
        assert!(parse_desktop_size("0x1080").is_err());
    }

    #[test]
    fn converts_connect_command_to_client_args() {
        let command = ConnectCommand {
            destination: Some("server.example.test".to_owned()),
            username: Some("user".to_owned()),
            domain: Some("domain".to_owned()),
            password: Some("secret".to_owned()),
            password_env: None,
            gw_endpoint: None,
            gw_user: None,
            gw_pass: None,
            rdp_file: None,
            keyboard_type: KeyboardType::IbmEnhanced,
            keyboard_subtype: 0,
            keyboard_functional_keys_count: 12,
            ime_file_name: String::new(),
            dig_product_id: String::new(),
            thin_client: false,
            small_cache: false,
            desktop_size: Some(DesktopSize {
                width: 1920,
                height: 1080,
            }),
            desktop_width: None,
            desktop_height: None,
            scale_desktop: Some(100),
            color_depth: Some(32),
            no_server_pointer: false,
            capabilities: None,
            autologon: false,
            no_tls: false,
            no_credssp: false,
            compression_enabled: Some(true),
            compression_level: Some(3),
            clipboard_type: ClipboardType::Default,
            codecs: vec!["rfx".to_owned()],
            prevent_session_lock: None,
            dvc_proxy: Vec::new(),
            #[cfg(windows)]
            dvc_plugin: Vec::new(),
            rdcleanpath_url: None,
            rdcleanpath_token: None,
        };

        let args = command.to_ironrdp_args().expect("args");

        assert!(args.contains(&"--desktop-width".to_owned()));
        assert!(args.contains(&"1920".to_owned()));
        assert!(args.contains(&"--desktop-height".to_owned()));
        assert!(args.contains(&"1080".to_owned()));
    }
}
