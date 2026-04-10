use core::cell::RefCell;
use core::net::{Ipv4Addr, SocketAddrV4};
use core::num::NonZeroU32;
use core::time::Duration;
use std::borrow::Cow;
use std::rc::Rc;

use anyhow::Context as _;
use base64::Engine as _;
use futures_channel::mpsc;
use futures_util::io::{ReadHalf, WriteHalf};
use futures_util::{AsyncWriteExt as _, FutureExt as _, StreamExt as _, select};
use gloo_net::websocket;
use gloo_net::websocket::futures::WebSocket;
use gloo_timers::future::IntervalStream;
use iron_remote_desktop::{CursorStyle, DesktopSize, Extension, IronErrorKind};
use ironrdp::cliprdr::CliprdrClient;
use ironrdp::cliprdr::backend::ClipboardMessage;
use ironrdp::cliprdr::pdu::{FileContentsFlags, FileContentsRequest, FileContentsResponse, FileDescriptor};
use ironrdp::connector::connection_activation::ConnectionActivationState;
use ironrdp::connector::credssp::KerberosConfig;
use ironrdp::connector::{self, ClientConnector, Credentials};
use ironrdp::displaycontrol::client::DisplayControlClient;
use ironrdp::dvc::DrdynvcClient;
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::pdu::input::fast_path::FastPathInputEvent;
use ironrdp::pdu::rdp::capability_sets::client_codecs_capabilities;
use ironrdp::pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput, GracefulDisconnectReason, fast_path};
use ironrdp_core::WriteBuf;
use ironrdp_futures::{FramedWrite, single_sequence_step_read};
use rgb::AsPixels as _;
use tap::prelude::*;
use tracing::{debug, error, info, trace, warn};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlCanvasElement;

use crate::canvas::Canvas;
use crate::clipboard;
use crate::clipboard::{ClipboardData, FileMetadata, WasmClipboard, WasmClipboardBackend, WasmClipboardBackendMessage};
use crate::error::IronError;
use crate::image::extract_partial_image;
use crate::input::InputTransaction;
use crate::network_client::WasmNetworkClient;

const DEFAULT_WIDTH: u16 = 1280;
const DEFAULT_HEIGHT: u16 = 720;

#[derive(Clone, Default)]
pub(crate) struct SessionBuilder(Rc<RefCell<SessionBuilderInner>>);

struct SessionBuilderInner {
    username: Option<String>,
    destination: Option<String>,
    server_domain: Option<String>,
    password: Option<String>,
    proxy_address: Option<String>,
    auth_token: Option<String>,
    pcb: Option<String>,
    kdc_proxy_url: Option<String>,
    client_name: String,
    desktop_size: DesktopSize,

    render_canvas: Option<HtmlCanvasElement>,
    set_cursor_style_callback: Option<js_sys::Function>,
    set_cursor_style_callback_context: Option<JsValue>,
    remote_clipboard_changed_callback: Option<js_sys::Function>,
    force_clipboard_update_callback: Option<js_sys::Function>,
    // File transfer callbacks
    files_available_callback: Option<js_sys::Function>,
    file_contents_request_callback: Option<js_sys::Function>,
    file_contents_response_callback: Option<js_sys::Function>,
    lock_callback: Option<js_sys::Function>,
    unlock_callback: Option<js_sys::Function>,
    locks_expired_callback: Option<js_sys::Function>,

    use_display_control: bool,
    enable_credssp: bool,
    outbound_message_size_limit: Option<usize>,
}

impl Default for SessionBuilderInner {
    fn default() -> Self {
        Self {
            username: None,
            destination: None,
            server_domain: None,
            password: None,
            proxy_address: None,
            auth_token: None,
            pcb: None,
            kdc_proxy_url: None,
            client_name: "ironrdp-web".to_owned(),
            desktop_size: DesktopSize {
                width: DEFAULT_WIDTH,
                height: DEFAULT_HEIGHT,
            },

            render_canvas: None,
            set_cursor_style_callback: None,
            set_cursor_style_callback_context: None,
            remote_clipboard_changed_callback: None,
            force_clipboard_update_callback: None,
            files_available_callback: None,
            file_contents_request_callback: None,
            file_contents_response_callback: None,
            lock_callback: None,
            unlock_callback: None,
            locks_expired_callback: None,

            use_display_control: false,
            enable_credssp: true,
            outbound_message_size_limit: None,
        }
    }
}

impl iron_remote_desktop::SessionBuilder for SessionBuilder {
    type Session = Session;
    type Error = IronError;

    fn create() -> Self {
        Self(Rc::new(RefCell::new(SessionBuilderInner::default())))
    }

    /// Required
    fn username(&self, username: String) -> Self {
        self.0.borrow_mut().username = Some(username);
        self.clone()
    }

    /// Required
    fn destination(&self, destination: String) -> Self {
        self.0.borrow_mut().destination = Some(destination);
        self.clone()
    }

    /// Optional
    fn server_domain(&self, server_domain: String) -> Self {
        self.0.borrow_mut().server_domain = if server_domain.is_empty() {
            None
        } else {
            Some(server_domain)
        };
        self.clone()
    }

    /// Required
    fn password(&self, password: String) -> Self {
        self.0.borrow_mut().password = Some(password);
        self.clone()
    }

    /// Required
    fn proxy_address(&self, address: String) -> Self {
        self.0.borrow_mut().proxy_address = Some(address);
        self.clone()
    }

    /// Required
    fn auth_token(&self, token: String) -> Self {
        self.0.borrow_mut().auth_token = Some(token);
        self.clone()
    }

    /// Optional
    fn desktop_size(&self, desktop_size: DesktopSize) -> Self {
        self.0.borrow_mut().desktop_size = desktop_size;
        self.clone()
    }

    /// Optional
    fn render_canvas(&self, canvas: HtmlCanvasElement) -> Self {
        self.0.borrow_mut().render_canvas = Some(canvas);
        self.clone()
    }

    /// Required.
    ///
    /// # Callback signature:
    /// ```typescript
    /// function callback(
    ///     cursor_kind: string,
    ///     cursor_data: string | undefined,
    ///     hotspot_x: number | undefined,
    ///     hotspot_y: number | undefined
    /// ): void
    /// ```
    ///
    /// # Cursor kinds:
    /// - `default` (default system cursor); other arguments are `UNDEFINED`
    /// - `none` (hide cursor); other arguments are `UNDEFINED`
    /// - `url` (custom cursor data URL); `cursor_data` contains the data URL with Base64-encoded
    ///   cursor bitmap; `hotspot_x` and `hotspot_y` are set to the cursor hotspot coordinates.
    fn set_cursor_style_callback(&self, callback: js_sys::Function) -> Self {
        self.0.borrow_mut().set_cursor_style_callback = Some(callback);
        self.clone()
    }

    /// Required.
    fn set_cursor_style_callback_context(&self, context: JsValue) -> Self {
        self.0.borrow_mut().set_cursor_style_callback_context = Some(context);
        self.clone()
    }

    /// Optional
    fn remote_clipboard_changed_callback(&self, callback: js_sys::Function) -> Self {
        self.0.borrow_mut().remote_clipboard_changed_callback = Some(callback);
        self.clone()
    }

    /// Optional
    fn force_clipboard_update_callback(&self, callback: js_sys::Function) -> Self {
        self.0.borrow_mut().force_clipboard_update_callback = Some(callback);
        self.clone()
    }

    /// Because the server does not resize the framebuffer in the RDP protocol, this feature is unused in IronRDP.
    fn canvas_resized_callback(&self, _callback: js_sys::Function) -> Self {
        self.clone()
    }

    fn extension(&self, ext: Extension) -> Self {
        iron_remote_desktop::extension_match! {
            match ext;
            |pcb: String| { self.0.borrow_mut().pcb = Some(pcb) };
            |kdc_proxy_url: String| { self.0.borrow_mut().kdc_proxy_url = Some(kdc_proxy_url) };
            |display_control: bool| { self.0.borrow_mut().use_display_control = display_control };
            |enable_credssp: bool| { self.0.borrow_mut().enable_credssp = enable_credssp };
            |outbound_message_size_limit: f64| {
                let limit = if outbound_message_size_limit >= 0.0 && outbound_message_size_limit <= f64::from(u32::MAX) {
                    #[expect(clippy::as_conversions, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    { outbound_message_size_limit as usize }
                } else {
                    warn!(outbound_message_size_limit, "Invalid outbound message size limit; fallback to unlimited");
                    0 // Fallback to no limit for invalid values.
                };
                self.0.borrow_mut().outbound_message_size_limit = if limit > 0 { Some(limit) } else { None };
            };
            // File transfer callbacks - protocol-specific, routed through extension()
            // rather than dedicated trait methods to keep iron-remote-desktop protocol-agnostic.
            |files_available_callback: JsValue| {
                self.0.borrow_mut().files_available_callback = files_available_callback.dyn_into::<js_sys::Function>().ok();
            };
            |file_contents_request_callback: JsValue| {
                self.0.borrow_mut().file_contents_request_callback = file_contents_request_callback.dyn_into::<js_sys::Function>().ok();
            };
            |file_contents_response_callback: JsValue| {
                self.0.borrow_mut().file_contents_response_callback = file_contents_response_callback.dyn_into::<js_sys::Function>().ok();
            };
            |lock_callback: JsValue| {
                self.0.borrow_mut().lock_callback = lock_callback.dyn_into::<js_sys::Function>().ok();
            };
            |unlock_callback: JsValue| {
                self.0.borrow_mut().unlock_callback = unlock_callback.dyn_into::<js_sys::Function>().ok();
            };
            |locks_expired_callback: JsValue| {
                self.0.borrow_mut().locks_expired_callback = locks_expired_callback.dyn_into::<js_sys::Function>().ok();
            };
        }

        self.clone()
    }

    async fn connect(&self) -> Result<Self::Session, Self::Error> {
        let (
            username,
            destination,
            server_domain,
            password,
            proxy_address,
            auth_token,
            pcb,
            kdc_proxy_url,
            client_name,
            desktop_size,
            render_canvas,
            set_cursor_style_callback,
            set_cursor_style_callback_context,
            remote_clipboard_changed_callback,
            force_clipboard_update_callback,
            files_available_callback,
            file_contents_request_callback,
            file_contents_response_callback,
            lock_callback,
            unlock_callback,
            locks_expired_callback,
            outbound_message_size_limit,
        );

        {
            let inner = self.0.borrow();

            username = inner.username.clone().context("username missing")?;
            destination = inner.destination.clone().context("destination missing")?;
            server_domain = inner.server_domain.clone();
            password = inner.password.clone().context("password missing")?;
            proxy_address = inner.proxy_address.clone().context("proxy_address missing")?;
            auth_token = inner.auth_token.clone().context("auth_token missing")?;
            pcb = inner.pcb.clone();
            kdc_proxy_url = inner.kdc_proxy_url.clone();
            client_name = inner.client_name.clone();
            desktop_size = inner.desktop_size;

            render_canvas = inner.render_canvas.clone().context("render_canvas missing")?;

            set_cursor_style_callback = inner
                .set_cursor_style_callback
                .clone()
                .context("set_cursor_style_callback missing")?;
            set_cursor_style_callback_context = inner
                .set_cursor_style_callback_context
                .clone()
                .context("set_cursor_style_callback_context missing")?;
            remote_clipboard_changed_callback = inner.remote_clipboard_changed_callback.clone();
            force_clipboard_update_callback = inner.force_clipboard_update_callback.clone();
            files_available_callback = inner.files_available_callback.clone();
            file_contents_request_callback = inner.file_contents_request_callback.clone();
            file_contents_response_callback = inner.file_contents_response_callback.clone();
            lock_callback = inner.lock_callback.clone();
            unlock_callback = inner.unlock_callback.clone();
            locks_expired_callback = inner.locks_expired_callback.clone();
            outbound_message_size_limit = inner.outbound_message_size_limit;
        }

        info!("Connect to RDP host");

        let mut config = build_config(username, password, server_domain, client_name, desktop_size);

        let enable_credssp = self.0.borrow().enable_credssp;
        config.enable_credssp = enable_credssp;

        let (input_events_tx, input_events_rx) = mpsc::unbounded();

        let clipboard = remote_clipboard_changed_callback.clone().map(|callback| {
            WasmClipboard::new(
                clipboard::WasmClipboardMessageProxy::new(input_events_tx.clone()),
                clipboard::JsClipboardCallbacks {
                    on_remote_clipboard_changed: callback,
                    on_force_clipboard_update: force_clipboard_update_callback,
                    on_files_available: files_available_callback,
                    on_file_contents_request: file_contents_request_callback,
                    on_file_contents_response: file_contents_response_callback,
                    on_lock: lock_callback,
                    on_unlock: unlock_callback,
                    on_locks_expired: locks_expired_callback,
                },
            )
        });

        let ws = WebSocket::open(&proxy_address).context("couldn't open WebSocket")?;

        // NOTE: ideally, when the WebSocket can't be opened, the above call should fail with details on why is that
        // (e.g., the proxy hostname could not be resolved, proxy service is not running), but errors are neved
        // bubbled up in practice, so instead we poll the WebSocket state until we know its connected (i.e., the
        // WebSocket handshake is a success and user data can be exchanged).
        loop {
            match ws.state() {
                websocket::State::Closing | websocket::State::Closed => {
                    return Err(IronError::from(anyhow::anyhow!(
                        "failed to connect to {proxy_address} (WebSocket is `{:?}`)",
                        ws.state()
                    ))
                    .with_kind(IronErrorKind::ProxyConnect));
                }
                websocket::State::Connecting => {
                    trace!("WebSocket is connecting to proxy at {proxy_address}...");
                    gloo_timers::future::sleep(Duration::from_millis(50)).await;
                }
                websocket::State::Open => {
                    debug!("WebSocket connected to {proxy_address} with success");
                    break;
                }
            }
        }

        let use_display_control = self.0.borrow().use_display_control;

        let (connection_result, ws) = connect(ConnectParams {
            ws,
            config,
            proxy_auth_token: auth_token,
            destination,
            pcb,
            kdc_proxy_url,
            clipboard_backend: clipboard.as_ref().map(|clip| clip.backend()),
            use_display_control,
        })
        .await?;

        info!("Connected!");

        let (rdp_reader, rdp_writer) = futures_util::AsyncReadExt::split(ws);

        let (writer_tx, writer_rx) = mpsc::unbounded();

        spawn_local(writer_task(writer_rx, rdp_writer, outbound_message_size_limit));

        Ok(Session {
            desktop_size: connection_result.desktop_size,
            input_database: RefCell::new(ironrdp::input::Database::new()),
            writer_tx,
            input_events_tx,

            render_canvas,
            set_cursor_style_callback,
            set_cursor_style_callback_context,

            input_events_rx: RefCell::new(Some(input_events_rx)),
            rdp_reader: RefCell::new(Some(rdp_reader)),
            connection_result: RefCell::new(Some(connection_result)),
            clipboard: RefCell::new(Some(clipboard)),
        })
    }
}

pub(crate) type FastPathInputEvents = smallvec::SmallVec<[FastPathInputEvent; 2]>;

#[derive(Debug)]
pub(crate) enum RdpInputEvent {
    Cliprdr(ClipboardMessage),
    ClipboardBackend(WasmClipboardBackendMessage),
    FastPath(FastPathInputEvents),
    Resize {
        width: u32,
        height: u32,
        scale_factor: Option<u32>,
        physical_size: Option<(u32, u32)>,
    },
    TerminateSession,
}

pub(crate) struct SessionTerminationInfo {
    reason: GracefulDisconnectReason,
}

impl iron_remote_desktop::SessionTerminationInfo for SessionTerminationInfo {
    fn reason(&self) -> String {
        self.reason.to_string()
    }
}

pub(crate) struct Session {
    desktop_size: connector::DesktopSize,
    input_database: RefCell<ironrdp::input::Database>,
    writer_tx: mpsc::UnboundedSender<Vec<u8>>,
    input_events_tx: mpsc::UnboundedSender<RdpInputEvent>,

    render_canvas: HtmlCanvasElement,
    set_cursor_style_callback: js_sys::Function,
    set_cursor_style_callback_context: JsValue,

    // Consumed when `run` is called
    input_events_rx: RefCell<Option<mpsc::UnboundedReceiver<RdpInputEvent>>>,
    connection_result: RefCell<Option<connector::ConnectionResult>>,
    rdp_reader: RefCell<Option<ReadHalf<WebSocket>>>,
    clipboard: RefCell<Option<Option<WasmClipboard>>>,
}

impl Session {
    fn h_send_inputs(&self, inputs: smallvec::SmallVec<[FastPathInputEvent; 2]>) -> Result<(), IronError> {
        if !inputs.is_empty() {
            trace!("Inputs: {inputs:?}");

            self.input_events_tx
                .unbounded_send(RdpInputEvent::FastPath(inputs))
                .context("Send input events to writer task")?;
        }

        Ok(())
    }

    fn set_cursor_style(&self, style: CursorStyle) -> Result<(), IronError> {
        let (kind, data, hotspot_x, hotspot_y) = match style {
            CursorStyle::Default => ("default", None, None, None),
            CursorStyle::Hidden => ("hidden", None, None, None),
            CursorStyle::Url {
                data,
                hotspot_x,
                hotspot_y,
            } => ("url", Some(data), Some(hotspot_x), Some(hotspot_y)),
        };

        let args = js_sys::Array::from_iter([
            JsValue::from_str(kind),
            JsValue::from(data),
            JsValue::from_f64(hotspot_x.unwrap_or_default().into()),
            JsValue::from_f64(hotspot_y.unwrap_or_default().into()),
        ]);

        let _ret = self
            .set_cursor_style_callback
            .apply(&self.set_cursor_style_callback_context, &args)
            .map_err(|e| anyhow::Error::msg(format!("set cursor style callback failed: {e:?}")))?;

        Ok(())
    }
}

impl iron_remote_desktop::Session for Session {
    type SessionTerminationInfo = SessionTerminationInfo;
    type InputTransaction = InputTransaction;
    type ClipboardData = ClipboardData;
    type Error = IronError;

    async fn run(&self) -> Result<Self::SessionTerminationInfo, Self::Error> {
        let rdp_reader = self
            .rdp_reader
            .borrow_mut()
            .take()
            .context("RDP session can be started only once")?;

        let mut input_events = self
            .input_events_rx
            .borrow_mut()
            .take()
            .context("RDP session can be started only once")?;

        let connection_result = self
            .connection_result
            .borrow_mut()
            .take()
            .expect("run called only once");

        let mut clipboard = self.clipboard.borrow_mut().take().expect("run called only once");

        let mut framed = ironrdp_futures::LocalFuturesFramed::new(rdp_reader);

        debug!("Initialize canvas");

        let desktop_width =
            NonZeroU32::new(u32::from(connection_result.desktop_size.width)).context("desktop width is zero")?;
        let desktop_height =
            NonZeroU32::new(u32::from(connection_result.desktop_size.height)).context("desktop height is zero")?;

        let mut gui =
            Canvas::new(self.render_canvas.clone(), desktop_width, desktop_height).context("canvas initialization")?;

        debug!("Canvas initialized");

        info!("Start RDP session");

        let mut image = DecodedImage::new(
            PixelFormat::RgbA32,
            connection_result.desktop_size.width,
            connection_result.desktop_size.height,
        );

        let mut requested_resize = None;

        let mut active_stage = ActiveStage::new(connection_result);

        // Timer interval for driving clipboard lock timeouts (5 second interval)
        let mut cleanup_interval = IntervalStream::new(5_000).fuse();

        let disconnect_reason = 'outer: loop {
            let outputs = select! {
                frame = framed.read_pdu().fuse() => {
                    let (action, payload) = frame.context("read frame")?;
                    trace!(?action, frame_length = payload.len(), "Frame received");

                    active_stage.process(&mut image, action, &payload)?
                }
                input_events = input_events.next() => {
                    let event = input_events.context("read next input events")?;

                    match event {
                        RdpInputEvent::Cliprdr(message) => {
                            if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
                                if let Some(svc_messages) = match message {
                                    ClipboardMessage::SendInitiateCopy(formats) => Some(
                                        cliprdr.initiate_copy(&formats)
                                            .context("cliprdr initiate copy")?
                                    ),
                                    ClipboardMessage::SendFormatData(response) => Some(
                                        cliprdr.submit_format_data(response)
                                            .context("cliprdr submit format data")?
                                    ),
                                    ClipboardMessage::SendInitiatePaste(format) => Some(
                                        cliprdr.initiate_paste(format)
                                            .context("cliprdr initiate paste")?
                                    ),
                                    ClipboardMessage::SendFileContentsRequest(request) => Some(
                                        cliprdr.request_file_contents(request)
                                            .context("cliprdr request file contents")?
                                    ),
                                    ClipboardMessage::SendFileContentsResponse(response) => Some(
                                        cliprdr.submit_file_contents(response)
                                            .context("cliprdr submit file contents")?
                                    ),
                                    ClipboardMessage::Error(e) => {
                                        error!(error = %e, "Clipboard backend error");
                                        None
                                    }
                                } {
                                    let frame = active_stage.process_svc_processor_messages(svc_messages)?;
                                    // Send the messages to the server
                                    vec![ActiveStageOutput::ResponseFrame(frame)]
                                } else {
                                    // No messages to send to the server
                                    Vec::new()
                                }
                            } else  {
                                warn!("Clipboard event received, but Cliprdr is not available");
                                Vec::new()
                            }
                        }
                        RdpInputEvent::ClipboardBackend(event) => {
                            use crate::clipboard::WasmClipboardBackendMessage;

                            // Handle messages that need direct cliprdr access
                            match event {
                                WasmClipboardBackendMessage::FileContentsRequestSend { stream_id, index, flags, position, size, clip_data_id } => {
                                    if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
                                        let request = FileContentsRequest {
                                            stream_id,
                                            index,
                                            flags,
                                            position,
                                            requested_size: size,
                                            data_id: clip_data_id,
                                        };
                                        match cliprdr.request_file_contents(request) {
                                            Ok(svc_messages) => {
                                                let frame = active_stage.process_svc_processor_messages(svc_messages)?;
                                                vec![ActiveStageOutput::ResponseFrame(frame)]
                                            }
                                            Err(e) => {
                                                error!(error = %e, "File contents request failed");
                                                Vec::new()
                                            }
                                        }
                                    } else {
                                        warn!("Request file contents received, but Cliprdr is not available");
                                        Vec::new()
                                    }
                                }
                                WasmClipboardBackendMessage::FileContentsResponseSend { stream_id, is_error, data } => {
                                    if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
                                        let response = if is_error {
                                            FileContentsResponse::new_error(stream_id)
                                        } else {
                                            FileContentsResponse::new_data_response(stream_id, data)
                                        };
                                        match cliprdr.submit_file_contents(response) {
                                            Ok(svc_messages) => {
                                                let frame = active_stage.process_svc_processor_messages(svc_messages)?;
                                                vec![ActiveStageOutput::ResponseFrame(frame)]
                                            }
                                            Err(e) => {
                                                error!(error = %e, "File contents submit failed");
                                                Vec::new()
                                            }
                                        }
                                    } else {
                                        warn!("Submit file contents received, but Cliprdr is not available");
                                        Vec::new()
                                    }
                                }
                                WasmClipboardBackendMessage::InitiateFileCopy { files } => {
                                    if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
                                        // Convert FileMetadata to FileDescriptor using the
                                        // validated conversion that checks name length/emptiness
                                        // and sets proper file attributes.
                                        let file_descriptors: Vec<FileDescriptor> = files
                                            .into_iter()
                                            .filter_map(|f| match f.to_file_descriptor() {
                                                Ok(desc) => Some(desc),
                                                Err(e) => {
                                                    warn!(error = format!("{e:#}"), "Skipping file with invalid metadata");
                                                    None
                                                }
                                            })
                                            .collect();

                                        match cliprdr.initiate_file_copy(file_descriptors) {
                                            Ok(svc_messages) => {
                                                let frame = active_stage.process_svc_processor_messages(svc_messages)?;
                                                vec![ActiveStageOutput::ResponseFrame(frame)]
                                            }
                                            Err(e) => {
                                                error!(error = %e, "Initiate file copy failed");
                                                Vec::new()
                                            }
                                        }
                                    } else {
                                        warn!("Initiate file copy received, but Cliprdr is not available");
                                        Vec::new()
                                    }
                                }
                                // All other messages are forwarded to clipboard backend
                                other => {
                                    if let Some(clipboard) = &mut clipboard {
                                        clipboard.process_event(other)?;
                                    }
                                    Vec::new()
                                }
                            }
                        }
                        RdpInputEvent::FastPath(events) => {
                            active_stage.process_fastpath_input(&mut image, &events)
                                .context("fast path input events processing")?
                        }
                        RdpInputEvent::Resize { width, height, scale_factor, physical_size } => {
                            debug!(width, height, scale_factor, "Resize event received");
                            if width == 0 || height == 0 {
                                warn!("Resize event ignored: width or height is zero");
                                Vec::new()
                            } else if let Some(response_frame) = active_stage.encode_resize(width, height, scale_factor, physical_size) {
                                let width = NonZeroU32::new(width).expect("width is guaranteed to be non-zero due to the prior check");
                                let height = NonZeroU32::new(height).expect("height is guaranteed to be non-zero due to the prior check");

                                requested_resize = Some((width, height));
                                vec![ActiveStageOutput::ResponseFrame(response_frame?)]
                            } else {
                                debug!("Resize event ignored");
                                Vec::new()
                            }
                        },
                        RdpInputEvent::TerminateSession => {
                            active_stage.graceful_shutdown()
                                .context("graceful shutdown")?
                        }
                    }
                }
                _ = cleanup_interval.next() => {
                    // Drive clipboard lock timeout cleanup
                    if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
                        match cliprdr.drive_timeouts() {
                            Ok(svc_messages) => {
                                let frame = active_stage.process_svc_processor_messages(svc_messages)?;
                                if !frame.is_empty() {
                                    vec![ActiveStageOutput::ResponseFrame(frame)]
                                } else {
                                    Vec::new()
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "Clipboard timeout cleanup failed");
                                Vec::new()
                            }
                        }
                    } else {
                        Vec::new()
                    }
                }
            };

            for out in outputs {
                match out {
                    ActiveStageOutput::ResponseFrame(frame) => {
                        self.writer_tx
                            .unbounded_send(frame)
                            .context("Send frame to writer task")?;
                    }
                    ActiveStageOutput::GraphicsUpdate(region) => {
                        // PERF: some copies and conversion could be optimized
                        let (region, buffer) = extract_partial_image(&image, region);
                        gui.draw(&buffer, region).context("draw updated region")?;
                    }
                    ActiveStageOutput::PointerDefault => {
                        self.set_cursor_style(CursorStyle::Default)?;
                    }
                    ActiveStageOutput::PointerHidden => {
                        self.set_cursor_style(CursorStyle::Hidden)?;
                    }
                    ActiveStageOutput::PointerPosition { .. } => {
                        // Not applicable for web.
                    }
                    ActiveStageOutput::PointerBitmap(pointer) => {
                        // Maximum allowed cursor size for browsers is 32x32, because bigger sizes
                        // will cause the following issues:
                        // - cursors bigger than 128x128 are not supported in browsers.
                        // - cursors bigger than 32x32 will default to the system cursor if their
                        //   sprite does not fit in the browser's viewport, introducing an abrupt
                        //   cursor style change when the cursor is moved to the edge of the
                        //   browser window.
                        //
                        // Therefore, we need to scale the cursor sprite down to 32x32 if it is
                        // bigger than that.
                        const MAX_CURSOR_SIZE: u16 = 32;
                        // INVARIANT: 0 < scale <= 1.0
                        // INVARIANT: pointer.width * scale <= MAX_CURSOR_SIZE
                        // INVARIANT: pointer.height * scale <= MAX_CURSOR_SIZE
                        let scale = if pointer.width >= pointer.height && pointer.width > MAX_CURSOR_SIZE {
                            Some(f64::from(MAX_CURSOR_SIZE) / f64::from(pointer.width))
                        } else if pointer.height > MAX_CURSOR_SIZE {
                            Some(f64::from(MAX_CURSOR_SIZE) / f64::from(pointer.height))
                        } else {
                            None
                        };

                        let (png_width, png_height, hotspot_x, hotspot_y, rgba_buffer) = if let Some(scale) = scale {
                            // Per invariants: Following conversions will never saturate.
                            let scaled_width = f64_to_u16_saturating_cast(f64::from(pointer.width) * scale);
                            let scaled_height = f64_to_u16_saturating_cast(f64::from(pointer.height) * scale);
                            let hotspot_x = f64_to_u16_saturating_cast(f64::from(pointer.hotspot_x) * scale);
                            let hotspot_y = f64_to_u16_saturating_cast(f64::from(pointer.hotspot_y) * scale);

                            // Per invariants: scaled_width * scaled_height * 4 <= 32 * 32 * 4 < usize::MAX
                            #[expect(clippy::arithmetic_side_effects)]
                            let resized_rgba_buffer_size = usize::from(scaled_width * scaled_height * 4);

                            let mut rgba_resized = vec![0u8; resized_rgba_buffer_size];
                            let mut resizer = resize::new(
                                usize::from(pointer.width),
                                usize::from(pointer.height),
                                usize::from(scaled_width),
                                usize::from(scaled_height),
                                resize::Pixel::RGBA8P,
                                resize::Type::Lanczos3,
                            )
                            .context("failed to initialize cursor resizer")?;

                            resizer
                                .resize(pointer.bitmap_data.as_pixels(), rgba_resized.as_pixels_mut())
                                .context("failed to resize cursor")?;

                            (
                                scaled_width,
                                scaled_height,
                                hotspot_x,
                                hotspot_y,
                                Cow::Owned(rgba_resized),
                            )
                        } else {
                            (
                                pointer.width,
                                pointer.height,
                                pointer.hotspot_x,
                                pointer.hotspot_y,
                                Cow::Borrowed(pointer.bitmap_data.as_slice()),
                            )
                        };

                        // Encode PNG.
                        let mut png_buffer = Vec::new();
                        {
                            let mut encoder =
                                png::Encoder::new(&mut png_buffer, u32::from(png_width), u32::from(png_height));

                            encoder.set_color(png::ColorType::Rgba);
                            encoder.set_depth(png::BitDepth::Eight);
                            encoder.set_compression(png::Compression::Fast);
                            let mut writer = encoder.write_header().context("PNG encoder header write failed")?;
                            writer
                                .write_image_data(&rgba_buffer)
                                .context("failed to encode pointer PNG")?;
                        }

                        // Encode PNG into Base64 data URL.
                        let mut style = "data:image/png;base64,".to_owned();
                        base64::engine::general_purpose::STANDARD.encode_string(png_buffer, &mut style);

                        self.set_cursor_style(CursorStyle::Url {
                            data: style,
                            hotspot_x,
                            hotspot_y,
                        })?;
                    }
                    ActiveStageOutput::DeactivateAll(mut box_connection_activation) => {
                        // Execute the Deactivation-Reactivation Sequence:
                        // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dfc234ce-481a-4674-9a5d-2a7bafb14432
                        debug!("Received Server Deactivate All PDU, executing Deactivation-Reactivation Sequence");

                        // We need to perform resize after receiving the Deactivate All PDU, because there may be frames
                        // with the previous dimensions arriving between the resize request and this message.
                        if let Some((width, height)) = requested_resize {
                            self.render_canvas.set_width(width.get());
                            self.render_canvas.set_height(height.get());
                            gui.resize(width, height);
                            requested_resize = None;
                        }

                        let mut buf = WriteBuf::new();
                        'activation_seq: loop {
                            let written =
                                single_sequence_step_read(&mut framed, &mut *box_connection_activation, &mut buf)
                                    .await?;

                            if written.size().is_some() {
                                self.writer_tx
                                    .unbounded_send(buf.filled().to_vec())
                                    .context("Send frame to writer task")?;
                            }

                            if let ConnectionActivationState::Finalized {
                                io_channel_id,
                                user_channel_id,
                                desktop_size,
                                share_id,
                                enable_server_pointer,
                                pointer_software_rendering,
                            } = box_connection_activation.connection_activation_state()
                            {
                                debug!("Deactivation-Reactivation Sequence completed");
                                image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
                                // Create a new [`FastPathProcessor`] with potentially updated
                                // io/user channel ids.
                                active_stage.set_fastpath_processor(
                                    fast_path::ProcessorBuilder {
                                        io_channel_id,
                                        user_channel_id,
                                        share_id,
                                        enable_server_pointer,
                                        pointer_software_rendering,
                                        bulk_decompressor: None,
                                    }
                                    .build(),
                                );
                                active_stage.set_share_id(share_id);
                                active_stage.set_enable_server_pointer(enable_server_pointer);
                                break 'activation_seq;
                            }
                        }
                    }
                    ActiveStageOutput::MultitransportRequest(pdu) => {
                        debug!(
                            request_id = pdu.request_id,
                            requested_protocol = ?pdu.requested_protocol,
                            "Multitransport request received (UDP transport not implemented)"
                        );
                    }
                    ActiveStageOutput::AutoDetect(request) => {
                        debug!(?request, "Auto-detect");
                    }
                    ActiveStageOutput::Terminate(reason) => break 'outer reason,
                }
            }
        };

        info!(%disconnect_reason, "RDP session terminated");

        Ok(SessionTerminationInfo {
            reason: disconnect_reason,
        })
    }

    fn desktop_size(&self) -> DesktopSize {
        DesktopSize {
            width: self.desktop_size.width,
            height: self.desktop_size.height,
        }
    }

    fn apply_inputs(&self, transaction: Self::InputTransaction) -> Result<(), Self::Error> {
        let inputs = self.input_database.borrow_mut().apply(transaction);
        self.h_send_inputs(inputs)
    }

    fn release_all_inputs(&self) -> Result<(), Self::Error> {
        let inputs = self.input_database.borrow_mut().release_all();
        self.h_send_inputs(inputs)
    }

    fn synchronize_lock_keys(
        &self,
        scroll_lock: bool,
        num_lock: bool,
        caps_lock: bool,
        kana_lock: bool,
    ) -> Result<(), Self::Error> {
        use ironrdp::pdu::input::fast_path::FastPathInput;

        let event = ironrdp::input::synchronize_event(scroll_lock, num_lock, caps_lock, kana_lock);
        let fastpath_input = FastPathInput::single(event);

        let frame = ironrdp::core::encode_vec(&fastpath_input).context("FastPathInput encoding")?;

        self.writer_tx
            .unbounded_send(frame)
            .context("Send frame to writer task")?;

        Ok(())
    }

    fn shutdown(&self) -> Result<(), Self::Error> {
        self.input_events_tx
            .unbounded_send(RdpInputEvent::TerminateSession)
            .context("failed to send terminate session event to writer task")?;

        Ok(())
    }

    async fn on_clipboard_paste(&self, content: &Self::ClipboardData) -> Result<(), Self::Error> {
        self.input_events_tx
            .unbounded_send(RdpInputEvent::ClipboardBackend(
                WasmClipboardBackendMessage::LocalClipboardChanged(content.clone()),
            ))
            .context("send clipboard backend event")?;

        Ok(())
    }

    fn resize(
        &self,
        width: u32,
        height: u32,
        scale_factor: Option<u32>,
        physical_width: Option<u32>,
        physical_height: Option<u32>,
    ) {
        if self
            .input_events_tx
            .unbounded_send(RdpInputEvent::Resize {
                width,
                height,
                scale_factor,
                physical_size: physical_width.and_then(|width| physical_height.map(|height| (width, height))),
            })
            .is_err()
        {
            warn!("Failed to send resize event, receiver is closed");
        }
    }

    fn supports_unicode_keyboard_shortcuts(&self) -> bool {
        // RDP does not support Unicode keyboard shortcuts.
        // When key combinations are executed, only plain scancode events are allowed to function correctly.
        false
    }

    fn invoke_extension(&self, ext: Extension) -> Result<JsValue, Self::Error> {
        // File transfer operations are protocol-specific (RDPECLIP) and routed
        // through invoke_extension rather than dedicated Session trait methods
        // to keep the iron-remote-desktop trait surface protocol-agnostic.
        iron_remote_desktop::extension_match! {
            match ext;
            |request_file_contents: JsValue| {
                let obj = into_object(request_file_contents)?;
                let stream_id = get_u32(&obj, "stream_id")?;
                let file_index = get_i32(&obj, "file_index")?;
                let flags = get_u32(&obj, "flags")?;
                let position = get_u64(&obj, "position")?;
                let size = get_u32(&obj, "size")?;
                let clip_data_id = get_u32_opt(&obj, "clip_data_id")?;

                self.input_events_tx
                    .unbounded_send(RdpInputEvent::ClipboardBackend(
                        WasmClipboardBackendMessage::FileContentsRequestSend {
                            stream_id,
                            index: file_index,
                            flags: FileContentsFlags::from_bits_truncate(flags),
                            position,
                            size,
                            clip_data_id,
                        },
                    ))
                    .context("send file contents request")
                    .map_err(IronError::from)?;

                return Ok(JsValue::NULL);
            };
            |submit_file_contents: JsValue| {
                let obj = into_object(submit_file_contents)?;
                let stream_id = get_u32(&obj, "stream_id")?;
                let is_error = get_bool(&obj, "is_error")?;
                let data_val = js_sys::Reflect::get(&obj, &JsValue::from_str("data"))
                    .map_err(|e| IronError::from(anyhow::anyhow!("get property `data`: {e:?}")))?;
                let data = js_sys::Uint8Array::new(&data_val).to_vec();

                self.input_events_tx
                    .unbounded_send(RdpInputEvent::ClipboardBackend(
                        WasmClipboardBackendMessage::FileContentsResponseSend {
                            stream_id,
                            is_error,
                            data,
                        },
                    ))
                    .context("send file contents response")
                    .map_err(IronError::from)?;

                return Ok(JsValue::NULL);
            };
            |initiate_file_copy: JsValue| {
                let file_list = parse_file_metadata_array(initiate_file_copy)?;

                self.input_events_tx
                    .unbounded_send(RdpInputEvent::ClipboardBackend(
                        WasmClipboardBackendMessage::InitiateFileCopy { files: file_list },
                    ))
                    .context("send initiate file copy")
                    .map_err(IronError::from)?;

                return Ok(JsValue::NULL);
            };
        }

        Err(
            IronError::from(anyhow::Error::msg(format!("unknown extension: {}", ext.ident())))
                .with_kind(IronErrorKind::General),
        )
    }
}

fn into_object(val: JsValue) -> Result<js_sys::Object, IronError> {
    val.dyn_into::<js_sys::Object>()
        .map_err(|_| anyhow::anyhow!("expected object").into())
}

fn get_u32(obj: &js_sys::Object, key: &str) -> Result<u32, IronError> {
    let val = js_sys::Reflect::get(obj, &JsValue::from_str(key))
        .map_err(|e| anyhow::anyhow!("get property `{key}`: {e:?}"))?;
    let f = val
        .as_f64()
        .with_context(|| format!("invalid type for property `{key}`"))?;
    Ok(f64_to_u32_saturating_cast(f))
}

fn get_i32(obj: &js_sys::Object, key: &str) -> Result<i32, IronError> {
    let val = js_sys::Reflect::get(obj, &JsValue::from_str(key))
        .map_err(|e| anyhow::anyhow!("get property `{key}`: {e:?}"))?;
    let f = val
        .as_f64()
        .with_context(|| format!("invalid type for property `{key}`"))?;
    Ok(f64_to_i32_saturating_cast(f))
}

fn get_u64(obj: &js_sys::Object, key: &str) -> Result<u64, IronError> {
    let val = js_sys::Reflect::get(obj, &JsValue::from_str(key))
        .map_err(|e| anyhow::anyhow!("get property `{key}`: {e:?}"))?;
    let f = val
        .as_f64()
        .with_context(|| format!("invalid type for property `{key}`"))?;
    // Validate integer precision before casting
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    if !f.is_finite() || f < 0.0 || f.fract() != 0.0 || f > MAX_SAFE_INTEGER {
        return Err(anyhow::anyhow!(
            "property `{key}` must be a finite non-negative integer <= Number.MAX_SAFE_INTEGER (got: {f})"
        )
        .into());
    }
    #[expect(clippy::as_conversions, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(f as u64)
}

fn get_bool(obj: &js_sys::Object, key: &str) -> Result<bool, IronError> {
    let val = js_sys::Reflect::get(obj, &JsValue::from_str(key))
        .map_err(|e| anyhow::anyhow!("get property `{key}`: {e:?}"))?;
    val.as_bool()
        .with_context(|| format!("invalid type for property `{key}`"))
        .map_err(Into::into)
}

fn get_u32_opt(obj: &js_sys::Object, key: &str) -> Result<Option<u32>, IronError> {
    let val = js_sys::Reflect::get(obj, &JsValue::from_str(key))
        .map_err(|e| anyhow::anyhow!("get property `{key}`: {e:?}"))?;
    if val.is_undefined() || val.is_null() {
        return Ok(None);
    }
    let f = val
        .as_f64()
        .with_context(|| format!("invalid type for property `{key}`"))?;
    Ok(Some(f64_to_u32_saturating_cast(f)))
}

#[expect(clippy::as_conversions, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn f64_to_u32_saturating_cast(f: f64) -> u32 {
    f.clamp(0.0, f64::from(u32::MAX)) as u32
}

#[expect(clippy::as_conversions, clippy::cast_possible_truncation)]
fn f64_to_i32_saturating_cast(f: f64) -> i32 {
    f.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

/// Parse a JsValue (expected to be a JS array of file metadata objects)
/// into a `Vec<FileMetadata>`.
fn parse_file_metadata_array(files: JsValue) -> Result<Vec<FileMetadata>, IronError> {
    let js_array = js_sys::Array::from(&files);
    #[expect(
        clippy::as_conversions,
        reason = "JavaScript array length is u32, safe to convert to usize"
    )]
    let mut file_list = Vec::with_capacity(js_array.length() as usize);

    for i in 0..js_array.length() {
        let file_obj = js_array.get(i);
        let name = js_sys::Reflect::get(&file_obj, &JsValue::from_str("name"))
            .ok()
            .and_then(|v| v.as_string())
            .context("file name is required")?;
        let size_f64 = js_sys::Reflect::get(&file_obj, &JsValue::from_str("size"))
            .ok()
            .and_then(|v| v.as_f64())
            .context("file size is required")?;
        // JS numbers are f64; reject fractional or out-of-safe-integer-range values
        // to avoid silent truncation when casting to u64
        const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
        if !size_f64.is_finite() || size_f64 < 0.0 || size_f64.fract() != 0.0 || size_f64 > MAX_SAFE_INTEGER {
            return Err(anyhow::anyhow!(
                "file size must be a finite non-negative integer <= Number.MAX_SAFE_INTEGER (got: {size_f64})"
            )
            .into());
        }
        #[expect(clippy::as_conversions, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let size = size_f64 as u64;

        let last_modified_f64 = js_sys::Reflect::get(&file_obj, &JsValue::from_str("lastModified"))
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        // Store as JS timestamp (ms since Unix epoch). FileMetadata::to_file_descriptor()
        // handles the conversion to Windows FILETIME for the wire format.
        const MAX_SAFE_TS: f64 = 9_007_199_254_740_991.0;
        #[expect(clippy::as_conversions, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let last_modified = if last_modified_f64.is_finite()
            && (0.0..=MAX_SAFE_TS).contains(&last_modified_f64)
            && last_modified_f64.fract() == 0.0
        {
            last_modified_f64 as u64
        } else {
            0
        };

        let path = js_sys::Reflect::get(&file_obj, &JsValue::from_str("path"))
            .ok()
            .and_then(|v| v.as_string())
            .filter(|s| !s.is_empty());

        let is_directory = js_sys::Reflect::get(&file_obj, &JsValue::from_str("isDirectory"))
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        file_list.push(FileMetadata {
            name,
            path,
            size,
            last_modified,
            is_directory,
        });
    }

    Ok(file_list)
}

fn build_config(
    username: String,
    password: String,
    domain: Option<String>,
    client_name: String,
    desktop_size: DesktopSize,
) -> connector::Config {
    connector::Config {
        credentials: Credentials::UsernamePassword { username, password },
        domain,
        // TODO(#327): expose these options from the WASM module.
        enable_tls: true,
        enable_credssp: true,
        allow_encryption_level_none: false,
        keyboard_type: ironrdp::pdu::gcc::KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_layout: 0, // the server SHOULD use the default active input locale identifier
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: connector::DesktopSize {
            width: desktop_size.width,
            height: desktop_size.height,
        },
        bitmap: Some(connector::BitmapConfig {
            color_depth: 16,
            lossy_compression: true,
            codecs: client_codecs_capabilities(&[]).expect("can't panic for &[]"),
        }),
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "fine unless we end up with an insanely big version"
        )]
        client_build: semver::Version::parse(env!("CARGO_PKG_VERSION"))
            .map_or(0, |version| version.major * 100 + version.minor * 10 + version.patch)
            .pipe(u32::try_from)
            .expect("fine until major ~42949672"),
        client_name,
        // NOTE: hardcode this value like in freerdp
        // https://github.com/FreeRDP/FreeRDP/blob/4e24b966c86fdf494a782f0dfcfc43a057a2ea60/libfreerdp/core/settings.c#LL49C34-L49C70
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),
        platform: ironrdp::pdu::rdp::capability_sets::MajorPlatformType::UNSPECIFIED,
        compression_type: None,
        enable_server_pointer: false,
        autologon: false,
        enable_audio_playback: false,
        request_data: None,
        pointer_software_rendering: false,
        multitransport_flags: None,
        performance_flags: PerformanceFlags::default(),
        desktop_scale_factor: 0,
        hardware_id: None,
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
    }
}

async fn writer_task(
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
    rdp_writer: WriteHalf<WebSocket>,
    outbound_limit: Option<usize>,
) {
    debug!("writer task started");

    async fn inner(
        mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
        mut rdp_writer: WriteHalf<WebSocket>,
        outbound_limit: Option<usize>,
    ) -> anyhow::Result<()> {
        while let Some(frame) = rx.next().await {
            match outbound_limit {
                Some(max_size) if frame.len() > max_size => {
                    // Send in chunks.
                    for chunk in frame.chunks(max_size) {
                        rdp_writer.write_all(chunk).await.context("couldn't write chunk")?;
                        rdp_writer.flush().await.context("couldn't flush chunk")?;
                    }
                }
                _ => {
                    // Send complete frame (default case).
                    rdp_writer.write_all(&frame).await.context("couldn't write frame")?;
                    rdp_writer.flush().await.context("couldn't flush frame")?;
                }
            }
        }

        Ok(())
    }

    match inner(rx, rdp_writer, outbound_limit).await {
        Ok(()) => debug!("writer task ended gracefully"),
        Err(e) => error!("writer task ended unexpectedly: {e:#}"),
    }
}

struct ConnectParams {
    ws: WebSocket,
    config: connector::Config,
    proxy_auth_token: String,
    destination: String,
    pcb: Option<String>,
    kdc_proxy_url: Option<String>,
    clipboard_backend: Option<WasmClipboardBackend>,
    use_display_control: bool,
}

async fn connect(
    ConnectParams {
        ws,
        config,
        proxy_auth_token,
        destination,
        pcb,
        kdc_proxy_url,
        clipboard_backend,
        use_display_control,
    }: ConnectParams,
) -> Result<(connector::ConnectionResult, WebSocket), IronError> {
    let mut framed = ironrdp_futures::LocalFuturesFramed::new(ws);

    // In web browser environments, we do not have an easy access to the local address of the socket.
    let dummy_client_addr = core::net::SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 33899));

    let mut connector = ClientConnector::new(config, dummy_client_addr);

    if let Some(clipboard_backend) = clipboard_backend {
        connector.attach_static_channel(CliprdrClient::new(Box::new(clipboard_backend)));
    }

    if use_display_control {
        connector.attach_static_channel(
            DrdynvcClient::new().with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new()))),
        );
    }

    let (upgraded, server_public_key) =
        connect_rdcleanpath(&mut framed, &mut connector, destination.clone(), proxy_auth_token, pcb).await?;

    let connection_result = ironrdp_futures::connect_finalize(
        upgraded,
        connector,
        &mut framed,
        &mut WasmNetworkClient,
        (&destination).into(),
        server_public_key,
        url::Url::parse(kdc_proxy_url.unwrap_or_default().as_str()) // if kdc_proxy_url does not exit, give url parser a empty string, it will fail anyway and map to a None
            .ok()
            .map(|url| KerberosConfig {
                kdc_proxy_url: Some(url),
                // HACK: It's supposed to be the computer name of the client, but since it's not easy to retrieve this information in the browser,
                // we set the destination hostname instead because it happens to work.
                hostname: destination,
            }),
    )
    .await?;

    let ws = framed.into_inner_no_leftover();

    Ok((connection_result, ws))
}

async fn connect_rdcleanpath<S>(
    framed: &mut ironrdp_futures::Framed<S>,
    connector: &mut ClientConnector,
    destination: String,
    proxy_auth_token: String,
    pcb: Option<String>,
) -> Result<(ironrdp_futures::Upgraded, Vec<u8>), IronError>
where
    S: ironrdp_futures::FramedRead + FramedWrite,
{
    use ironrdp::connector::Sequence as _;
    use x509_cert::der::Decode as _;

    #[derive(Clone, Copy, Debug)]
    struct RDCleanPathHint;

    const RDCLEANPATH_HINT: RDCleanPathHint = RDCleanPathHint;

    impl ironrdp::pdu::PduHint for RDCleanPathHint {
        fn find_size(&self, bytes: &[u8]) -> ironrdp::core::DecodeResult<Option<(bool, usize)>> {
            match ironrdp_rdcleanpath::RDCleanPathPdu::detect(bytes) {
                ironrdp_rdcleanpath::DetectionResult::Detected { total_length, .. } => Ok(Some((true, total_length))),
                ironrdp_rdcleanpath::DetectionResult::NotEnoughBytes => Ok(None),
                ironrdp_rdcleanpath::DetectionResult::Failed => Err(ironrdp::core::other_err!(
                    "RDCleanPathHint",
                    "detection failed (invalid PDU)"
                )),
            }
        }
    }

    let mut buf = WriteBuf::new();

    info!("Begin connection procedure");

    {
        // RDCleanPath request

        let connector::ClientConnectorState::ConnectionInitiationSendRequest = connector.state else {
            return Err(anyhow::Error::msg("invalid connector state (send request)").into());
        };

        debug_assert!(connector.next_pdu_hint().is_none());

        let written = connector.step_no_input(&mut buf)?;
        let x224_pdu_len = written.size().expect("written size");
        debug_assert_eq!(x224_pdu_len, buf.filled_len());
        let x224_pdu = buf.filled().to_vec();

        let rdcleanpath_req =
            ironrdp_rdcleanpath::RDCleanPathPdu::new_request(x224_pdu, destination, proxy_auth_token, pcb)
                .context("new RDCleanPath request")?;
        debug!(message = ?rdcleanpath_req, "Send RDCleanPath request");
        let rdcleanpath_req = rdcleanpath_req.to_der().context("RDCleanPath request encode")?;

        framed
            .write_all(&rdcleanpath_req)
            .await
            .context("couldn't write RDCleanPath request")?;
    }

    {
        // RDCleanPath response

        let rdcleanpath_res = framed
            .read_by_hint(&RDCLEANPATH_HINT)
            .await
            .context("read RDCleanPath request")?;

        let rdcleanpath_res =
            ironrdp_rdcleanpath::RDCleanPathPdu::from_der(&rdcleanpath_res).context("RDCleanPath response decode")?;

        debug!(message = ?rdcleanpath_res, "Received RDCleanPath PDU");

        let (x224_connection_response, server_cert_chain) =
            match rdcleanpath_res.into_enum().context("invalid RDCleanPath PDU")? {
                ironrdp_rdcleanpath::RDCleanPath::Request { .. } => {
                    return Err(anyhow::Error::msg("received an unexpected RDCleanPath type (request)").into());
                }
                ironrdp_rdcleanpath::RDCleanPath::Response {
                    x224_connection_response,
                    server_cert_chain,
                    server_addr: _,
                } => (x224_connection_response, server_cert_chain),
                ironrdp_rdcleanpath::RDCleanPath::GeneralErr(error) => {
                    let details = iron_remote_desktop::RDCleanPathDetails::new(
                        error.http_status_code,
                        error.wsa_last_error,
                        error.tls_alert_code,
                    );
                    return Err(
                        IronError::from(anyhow::Error::new(error).context("received an RDCleanPath error"))
                            .with_kind(IronErrorKind::RDCleanPath)
                            .with_rdcleanpath_details(details),
                    );
                }
                ironrdp_rdcleanpath::RDCleanPath::NegotiationErr {
                    x224_connection_response,
                } => {
                    // Try to decode as X.224 Connection Confirm to extract negotiation failure details.
                    if let Ok(x224_confirm) = ironrdp_core::decode::<
                        ironrdp::pdu::x224::X224<ironrdp::pdu::nego::ConnectionConfirm>,
                    >(&x224_connection_response)
                    {
                        if let ironrdp::pdu::nego::ConnectionConfirm::Failure { code } = x224_confirm.0 {
                            // Convert to negotiation failure instead of generic RDCleanPath error.
                            let negotiation_failure = connector::NegotiationFailure::from(code);
                            return Err(IronError::from(
                                anyhow::Error::new(negotiation_failure).context("RDP negotiation failed"),
                            )
                            .with_kind(IronErrorKind::NegotiationFailure));
                        }
                    }

                    // Fallback to generic error if we can't decode the negotiation failure.
                    return Err(
                        IronError::from(anyhow::Error::msg("received an RDCleanPath negotiation error"))
                            .with_kind(IronErrorKind::RDCleanPath),
                    );
                }
            };

        let connector::ClientConnectorState::ConnectionInitiationWaitConfirm { .. } = connector.state else {
            return Err(anyhow::Error::msg("invalid connector state (wait confirm)").into());
        };

        debug_assert!(connector.next_pdu_hint().is_some());

        buf.clear();
        let written = connector.step(x224_connection_response.as_bytes(), &mut buf)?;

        debug_assert!(written.is_nothing());

        let server_cert = server_cert_chain
            .into_iter()
            .next()
            .context("server cert chain missing from rdcleanpath response")?;

        let cert = x509_cert::Certificate::from_der(server_cert.as_bytes())
            .context("failed to decode x509 certificate sent by proxy")?;

        let server_public_key = cert
            .tbs_certificate
            .subject_public_key_info
            .subject_public_key
            .as_bytes()
            .context("subject public key BIT STRING is not aligned")?
            .to_owned();

        let should_upgrade = ironrdp_futures::skip_connect_begin(connector);

        // At this point, proxy established the TLS session.

        let upgraded = ironrdp_futures::mark_as_upgraded(should_upgrade, connector);

        Ok((upgraded, server_public_key))
    }
}

#[expect(clippy::as_conversions, clippy::cast_sign_loss, clippy::cast_possible_truncation)]
fn f64_to_u16_saturating_cast(value: f64) -> u16 {
    value as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test helpers
    fn create_test_input_channel() -> (
        mpsc::UnboundedSender<RdpInputEvent>,
        mpsc::UnboundedReceiver<RdpInputEvent>,
    ) {
        mpsc::unbounded()
    }

    #[test]
    fn test_request_file_contents_parameter_marshalling() {
        let (tx, mut rx) = create_test_input_channel();

        // Send request with various parameters
        tx.unbounded_send(RdpInputEvent::ClipboardBackend(
            WasmClipboardBackendMessage::FileContentsRequestSend {
                stream_id: 123,
                index: 5,
                flags: FileContentsFlags::RANGE,
                position: 1024,
                size: 4096,
                clip_data_id: Some(42),
            },
        ))
        .unwrap();

        // Verify message parameters
        match rx.try_recv() {
            Ok(RdpInputEvent::ClipboardBackend(WasmClipboardBackendMessage::FileContentsRequestSend {
                stream_id,
                index,
                flags,
                position,
                size,
                clip_data_id,
            })) => {
                assert_eq!(stream_id, 123);
                assert_eq!(index, 5);
                assert_eq!(flags, FileContentsFlags::RANGE);
                assert_eq!(position, 1024);
                assert_eq!(size, 4096);
                assert_eq!(clip_data_id, Some(42));
            }
            _ => panic!("Expected FileContentsRequestSend with correct parameters"),
        }
    }

    #[test]
    fn test_request_file_contents_size_flag() {
        let (tx, mut rx) = create_test_input_channel();

        // Send SIZE request
        tx.unbounded_send(RdpInputEvent::ClipboardBackend(
            WasmClipboardBackendMessage::FileContentsRequestSend {
                stream_id: 1,
                index: 0,
                flags: FileContentsFlags::SIZE,
                position: 0,
                size: 8,
                clip_data_id: Some(1),
            },
        ))
        .unwrap();

        match rx.try_recv() {
            Ok(RdpInputEvent::ClipboardBackend(WasmClipboardBackendMessage::FileContentsRequestSend {
                flags, ..
            })) => {
                assert_eq!(flags, FileContentsFlags::SIZE);
            }
            _ => panic!("Expected SIZE request"),
        }
    }

    #[test]
    fn test_request_file_contents_without_clip_data_id() {
        let (tx, mut rx) = create_test_input_channel();

        // Send request without clip_data_id
        tx.unbounded_send(RdpInputEvent::ClipboardBackend(
            WasmClipboardBackendMessage::FileContentsRequestSend {
                stream_id: 10,
                index: 0,
                flags: FileContentsFlags::RANGE,
                position: 0,
                size: 1024,
                clip_data_id: None,
            },
        ))
        .unwrap();

        match rx.try_recv() {
            Ok(RdpInputEvent::ClipboardBackend(WasmClipboardBackendMessage::FileContentsRequestSend {
                clip_data_id,
                ..
            })) => {
                assert_eq!(clip_data_id, None);
            }
            _ => panic!("Expected request without clip_data_id"),
        }
    }

    #[test]
    fn test_submit_file_contents_success_response() {
        let (tx, mut rx) = create_test_input_channel();

        let data = vec![1, 2, 3, 4, 5];
        tx.unbounded_send(RdpInputEvent::ClipboardBackend(
            WasmClipboardBackendMessage::FileContentsResponseSend {
                stream_id: 42,
                is_error: false,
                data: data.clone(),
            },
        ))
        .unwrap();

        match rx.try_recv() {
            Ok(RdpInputEvent::ClipboardBackend(WasmClipboardBackendMessage::FileContentsResponseSend {
                stream_id,
                is_error,
                data: received_data,
            })) => {
                assert_eq!(stream_id, 42);
                assert!(!is_error);
                assert_eq!(received_data, data);
            }
            _ => panic!("Expected FileContentsResponseSend success"),
        }
    }

    #[test]
    fn test_submit_file_contents_error_response() {
        let (tx, mut rx) = create_test_input_channel();

        tx.unbounded_send(RdpInputEvent::ClipboardBackend(
            WasmClipboardBackendMessage::FileContentsResponseSend {
                stream_id: 99,
                is_error: true,
                data: vec![],
            },
        ))
        .unwrap();

        match rx.try_recv() {
            Ok(RdpInputEvent::ClipboardBackend(WasmClipboardBackendMessage::FileContentsResponseSend {
                is_error,
                ..
            })) => {
                assert!(is_error);
            }
            _ => panic!("Expected error response"),
        }
    }

    #[test]
    fn test_submit_file_contents_size_response() {
        let (tx, mut rx) = create_test_input_channel();

        // 8-byte size response (little-endian)
        let size_data = vec![0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]; // 4096 bytes

        tx.unbounded_send(RdpInputEvent::ClipboardBackend(
            WasmClipboardBackendMessage::FileContentsResponseSend {
                stream_id: 1,
                is_error: false,
                data: size_data.clone(),
            },
        ))
        .unwrap();

        match rx.try_recv() {
            Ok(RdpInputEvent::ClipboardBackend(WasmClipboardBackendMessage::FileContentsResponseSend {
                data, ..
            })) => {
                assert_eq!(data.len(), 8);
                assert_eq!(data, size_data);
            }
            _ => panic!("Expected size response"),
        }
    }

    #[test]
    fn test_initiate_file_copy_message() {
        let (tx, mut rx) = create_test_input_channel();

        let files = vec![
            FileMetadata {
                name: "file1.txt".to_owned(),
                path: None,
                size: 1024,
                last_modified: 1_700_000_000_000,
                is_directory: false,
            },
            FileMetadata {
                name: "file2.pdf".to_owned(),
                path: Some("docs".to_owned()),
                size: 2048,
                last_modified: 1_700_000_001_000,
                is_directory: false,
            },
        ];

        tx.unbounded_send(RdpInputEvent::ClipboardBackend(
            WasmClipboardBackendMessage::InitiateFileCopy { files },
        ))
        .unwrap();

        match rx.try_recv() {
            Ok(RdpInputEvent::ClipboardBackend(WasmClipboardBackendMessage::InitiateFileCopy {
                files: received_files,
            })) => {
                assert_eq!(received_files.len(), 2);
                assert_eq!(received_files[0].name, "file1.txt");
                assert_eq!(received_files[0].path, None);
                assert_eq!(received_files[0].size, 1024);
                assert_eq!(received_files[1].name, "file2.pdf");
                assert_eq!(received_files[1].path, Some("docs".to_owned()));
                assert_eq!(received_files[1].size, 2048);
            }
            _ => panic!("Expected InitiateFileCopy message"),
        }
    }

    #[test]
    fn test_large_position_value_marshalling() {
        let (tx, mut rx) = create_test_input_channel();

        // Test with large position value (near u64 max)
        let large_position = u64::MAX - 1000;

        tx.unbounded_send(RdpInputEvent::ClipboardBackend(
            WasmClipboardBackendMessage::FileContentsRequestSend {
                stream_id: 1,
                index: 0,
                flags: FileContentsFlags::RANGE,
                position: large_position,
                size: 1024,
                clip_data_id: Some(1),
            },
        ))
        .unwrap();

        match rx.try_recv() {
            Ok(RdpInputEvent::ClipboardBackend(WasmClipboardBackendMessage::FileContentsRequestSend {
                position,
                ..
            })) => {
                assert_eq!(position, large_position);
            }
            _ => panic!("Expected correct position marshalling"),
        }
    }

    #[test]
    fn test_zero_size_file() {
        let (tx, mut rx) = create_test_input_channel();

        let files = vec![FileMetadata {
            name: "empty.txt".to_owned(),
            path: None,
            size: 0,
            last_modified: 0,
            is_directory: false,
        }];

        tx.unbounded_send(RdpInputEvent::ClipboardBackend(
            WasmClipboardBackendMessage::InitiateFileCopy { files },
        ))
        .unwrap();

        match rx.try_recv() {
            Ok(RdpInputEvent::ClipboardBackend(WasmClipboardBackendMessage::InitiateFileCopy {
                files: received_files,
            })) => {
                assert_eq!(received_files[0].size, 0);
            }
            _ => panic!("Expected zero-size file"),
        }
    }

    #[test]
    fn test_file_with_special_characters_in_name() {
        let (tx, mut rx) = create_test_input_channel();

        let files = vec![FileMetadata {
            name: "test file (1) [copy].txt".to_owned(),
            path: None,
            size: 100,
            last_modified: 0,
            is_directory: false,
        }];

        tx.unbounded_send(RdpInputEvent::ClipboardBackend(
            WasmClipboardBackendMessage::InitiateFileCopy { files },
        ))
        .unwrap();

        match rx.try_recv() {
            Ok(RdpInputEvent::ClipboardBackend(WasmClipboardBackendMessage::InitiateFileCopy {
                files: received_files,
            })) => {
                assert_eq!(received_files[0].name, "test file (1) [copy].txt");
            }
            _ => panic!("Expected file with special characters"),
        }
    }

    #[test]
    fn test_empty_file_list() {
        let (tx, mut rx) = create_test_input_channel();

        let files: Vec<FileMetadata> = vec![];

        tx.unbounded_send(RdpInputEvent::ClipboardBackend(
            WasmClipboardBackendMessage::InitiateFileCopy { files },
        ))
        .unwrap();

        match rx.try_recv() {
            Ok(RdpInputEvent::ClipboardBackend(WasmClipboardBackendMessage::InitiateFileCopy {
                files: received_files,
            })) => {
                assert!(received_files.is_empty());
            }
            _ => panic!("Expected empty file list"),
        }
    }

    #[test]
    fn test_flags_bits_conversion() {
        let (tx, mut rx) = create_test_input_channel();

        // Test SIZE flag (0x1)
        let size_flags = FileContentsFlags::SIZE;
        assert_eq!(size_flags.bits(), 0x1);

        // Test DATA flag (0x2)
        let data_flags = FileContentsFlags::RANGE;
        assert_eq!(data_flags.bits(), 0x2);

        // Test that flags convert correctly through the channel
        tx.unbounded_send(RdpInputEvent::ClipboardBackend(
            WasmClipboardBackendMessage::FileContentsRequestSend {
                stream_id: 1,
                index: 0,
                flags: FileContentsFlags::from_bits_truncate(0x1),
                position: 0,
                size: 8,
                clip_data_id: None,
            },
        ))
        .unwrap();

        match rx.try_recv() {
            Ok(RdpInputEvent::ClipboardBackend(WasmClipboardBackendMessage::FileContentsRequestSend {
                flags, ..
            })) => {
                assert_eq!(flags.bits(), 0x1);
            }
            _ => panic!("Expected correct flags conversion"),
        }
    }
}
