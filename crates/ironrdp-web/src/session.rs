// https://github.com/rustwasm/wasm-bindgen/issues/4080
#![allow(non_snake_case)]

use core::cell::RefCell;
use core::num::NonZeroU32;
use core::time::Duration;
use std::borrow::Cow;
use std::rc::Rc;

use anyhow::Context as _;
use base64::Engine as _;
use futures_channel::mpsc;
use futures_util::io::{ReadHalf, WriteHalf};
use futures_util::{select, AsyncWriteExt as _, FutureExt as _, StreamExt as _};
use gloo_net::websocket;
use gloo_net::websocket::futures::WebSocket;
use iron_remote_desktop::{CursorStyle, DesktopSize, IronErrorKind};
use ironrdp::cliprdr::backend::ClipboardMessage;
use ironrdp::cliprdr::CliprdrClient;
use ironrdp::connector::connection_activation::ConnectionActivationState;
use ironrdp::connector::credssp::KerberosConfig;
use ironrdp::connector::{self, ClientConnector, Credentials};
use ironrdp::displaycontrol::client::DisplayControlClient;
use ironrdp::dvc::DrdynvcClient;
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::pdu::input::fast_path::FastPathInputEvent;
use ironrdp::pdu::rdp::client_info::PerformanceFlags;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{fast_path, ActiveStage, ActiveStageOutput, GracefulDisconnectReason};
use ironrdp_core::WriteBuf;
use ironrdp_futures::{single_sequence_step_read, FramedWrite};
use rgb::AsPixels as _;
use serde::{Deserialize, Serialize};
use tap::prelude::*;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlCanvasElement;

use crate::canvas::Canvas;
use crate::clipboard;
use crate::clipboard::{ClipboardTransaction, WasmClipboard, WasmClipboardBackend, WasmClipboardBackendMessage};
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
    remote_received_format_list_callback: Option<js_sys::Function>,
    force_clipboard_update_callback: Option<js_sys::Function>,

    use_display_control: bool,
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
            remote_received_format_list_callback: None,
            force_clipboard_update_callback: None,

            use_display_control: false,
        }
    }
}

impl iron_remote_desktop::SessionBuilder for SessionBuilder {
    type Session = Session;
    type Error = IronError;

    fn init() -> Self {
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
    fn remote_received_format_list_callback(&self, callback: js_sys::Function) -> Self {
        self.0.borrow_mut().remote_received_format_list_callback = Some(callback);
        self.clone()
    }

    /// Optional
    fn force_clipboard_update_callback(&self, callback: js_sys::Function) -> Self {
        self.0.borrow_mut().force_clipboard_update_callback = Some(callback);
        self.clone()
    }

    fn extension(&self, value: JsValue) -> Self {
        match serde_wasm_bindgen::from_value::<Extension>(value) {
            Ok(value) => match value {
                Extension::KdcProxyUrl(kdc_proxy_url) => self.0.borrow_mut().kdc_proxy_url = Some(kdc_proxy_url),
                Extension::Pcb(pcb) => self.0.borrow_mut().pcb = Some(pcb),
                Extension::DisplayControl(use_display_control) => {
                    self.0.borrow_mut().use_display_control = use_display_control
                }
            },
            Err(error) => error!(%error, "Unsupported extension value"),
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
            remote_received_format_list_callback,
            force_clipboard_update_callback,
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
            remote_received_format_list_callback = inner.remote_received_format_list_callback.clone();
            force_clipboard_update_callback = inner.force_clipboard_update_callback.clone();
        }

        info!("Connect to RDP host");

        let config = build_config(username, password, server_domain, client_name, desktop_size);

        let (input_events_tx, input_events_rx) = mpsc::unbounded();

        let clipboard = remote_clipboard_changed_callback.clone().map(|callback| {
            WasmClipboard::new(
                clipboard::WasmClipboardMessageProxy::new(input_events_tx.clone()),
                clipboard::JsClipboardCallbacks {
                    on_remote_clipboard_changed: callback,
                    on_remote_received_format_list: remote_received_format_list_callback,
                    on_force_clipboard_update: force_clipboard_update_callback,
                },
            )
        });

        let ws = WebSocket::open(&proxy_address).context("Couldn’t open WebSocket")?;

        // NOTE: ideally, when the WebSocket can’t be opened, the above call should fail with details on why is that
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

        spawn_local(writer_task(writer_rx, rdp_writer));

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

#[derive(Debug, Serialize, Deserialize)]
enum Extension {
    KdcProxyUrl(String),
    Pcb(String),
    DisplayControl(bool),
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
    type ClipboardTransaction = ClipboardTransaction;
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

        let mut gui = Canvas::new(
            self.render_canvas.clone(),
            u32::from(connection_result.desktop_size.width),
            u32::from(connection_result.desktop_size.height),
        )
        .context("canvas initialization")?;

        debug!("Canvas initialized");

        info!("Start RDP session");

        let mut image = DecodedImage::new(
            PixelFormat::RgbA32,
            connection_result.desktop_size.width,
            connection_result.desktop_size.height,
        );

        let mut active_stage = ActiveStage::new(connection_result);

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
                            if let Some(cliprdr) = active_stage.get_svc_processor::<CliprdrClient>() {
                                if let Some(svc_messages) = match message {
                                    ClipboardMessage::SendInitiateCopy(formats) => Some(
                                        cliprdr.initiate_copy(&formats)
                                            .context("CLIPRDR initiate copy")?
                                    ),
                                    ClipboardMessage::SendFormatData(response) => Some(
                                        cliprdr.submit_format_data(response)
                                            .context("CLIPRDR submit format data")?
                                    ),
                                    ClipboardMessage::SendInitiatePaste(format) => Some(
                                        cliprdr.initiate_paste(format)
                                            .context("CLIPRDR initiate paste")?
                                    ),
                                    ClipboardMessage::Error(e) => {
                                        error!("Clipboard backend error: {}", e);
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
                            if let Some(clipboard) = &mut clipboard {
                                clipboard.process_event(event)?;
                            }
                            // No RDP output frames for backend event processing
                            Vec::new()
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
                                self.render_canvas.set_width(width);
                                self.render_canvas.set_height(height);
                                gui.resize(NonZeroU32::new(width).unwrap(), NonZeroU32::new(height).unwrap());
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
                            #[allow(clippy::arithmetic_side_effects)]
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
                                .write_image_data(rgba_buffer.as_ref())
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
                                no_server_pointer,
                                pointer_software_rendering,
                            } = box_connection_activation.state
                            {
                                debug!("Deactivation-Reactivation Sequence completed");
                                image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
                                // Create a new [`FastPathProcessor`] with potentially updated
                                // io/user channel ids.
                                active_stage.set_fastpath_processor(
                                    fast_path::ProcessorBuilder {
                                        io_channel_id,
                                        user_channel_id,
                                        no_server_pointer,
                                        pointer_software_rendering,
                                    }
                                    .build(),
                                );
                                active_stage.set_no_server_pointer(no_server_pointer);
                                break 'activation_seq;
                            }
                        }
                    }
                    ActiveStageOutput::Terminate(reason) => break 'outer reason,
                }
            }
        };

        info!(%disconnect_reason, "RPD session terminated");

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
        let fastpath_input = FastPathInput(vec![event]);

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

    async fn on_clipboard_paste(&self, content: Self::ClipboardTransaction) -> Result<(), Self::Error> {
        self.input_events_tx
            .unbounded_send(RdpInputEvent::ClipboardBackend(
                WasmClipboardBackendMessage::LocalClipboardChanged(content),
            ))
            .context("Send clipboard backend event")?;

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
        self.input_events_tx
            .unbounded_send(RdpInputEvent::Resize {
                width,
                height,
                scale_factor,
                physical_size: physical_width.and_then(|width| physical_height.map(|height| (width, height))),
            })
            .expect("send resize event to writer task");
    }

    fn supports_unicode_keyboard_shortcuts(&self) -> bool {
        // RDP does not support Unicode keyboard shortcuts (When key combinations are executed, only
        // plain scancode events are allowed to function correctly).
        false
    }

    fn extension_call(_value: JsValue) -> Result<JsValue, Self::Error> {
        Ok(JsValue::null())
    }
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
        }),
        #[allow(clippy::arithmetic_side_effects)] // fine unless we end up with an insanely big version
        client_build: semver::Version::parse(env!("CARGO_PKG_VERSION"))
            .map(|version| version.major * 100 + version.minor * 10 + version.patch)
            .unwrap_or(0)
            .pipe(u32::try_from)
            .unwrap(),
        client_name,
        // NOTE: hardcode this value like in freerdp
        // https://github.com/FreeRDP/FreeRDP/blob/4e24b966c86fdf494a782f0dfcfc43a057a2ea60/libfreerdp/core/settings.c#LL49C34-L49C70
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),
        platform: ironrdp::pdu::rdp::capability_sets::MajorPlatformType::UNSPECIFIED,
        no_server_pointer: false,
        autologon: false,
        no_audio_playback: true,
        request_data: None,
        pointer_software_rendering: false,
        performance_flags: PerformanceFlags::default(),
        desktop_scale_factor: 0,
        hardware_id: None,
        license_cache: None,
    }
}

async fn writer_task(rx: mpsc::UnboundedReceiver<Vec<u8>>, rdp_writer: WriteHalf<WebSocket>) {
    debug!("writer task started");

    async fn inner(
        mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
        mut rdp_writer: WriteHalf<WebSocket>,
    ) -> anyhow::Result<()> {
        while let Some(frame) = rx.next().await {
            rdp_writer.write_all(&frame).await.context("Couldn’t write frame")?;
            rdp_writer.flush().await.context("Couldn’t flush")?;
        }

        Ok(())
    }

    match inner(rx, rdp_writer).await {
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

    let mut connector = ClientConnector::new(config);

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
        &mut framed,
        connector,
        (&destination).into(),
        server_public_key,
        Some(&mut WasmNetworkClient),
        url::Url::parse(kdc_proxy_url.unwrap_or_default().as_str()) // if kdc_proxy_url does not exit, give url parser a empty string, it will fail anyway and map to a None
            .ok()
            .map(|url| KerberosConfig {
                kdc_proxy_url: Some(url),
                // HACK: It’s supposed to be the computer name of the client, but since it’s not easy to retrieve this information in the browser,
                // we set the destination hostname instead because it happens to work.
                hostname: Some(destination),
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
            .context("couldn’t write RDCleanPath request")?;
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

        let (x224_connection_response, server_cert_chain, server_addr) =
            match rdcleanpath_res.into_enum().context("invalid RDCleanPath PDU")? {
                ironrdp_rdcleanpath::RDCleanPath::Request { .. } => {
                    return Err(anyhow::Error::msg("received an unexpected RDCleanPath type (request)").into());
                }
                ironrdp_rdcleanpath::RDCleanPath::Response {
                    x224_connection_response,
                    server_cert_chain,
                    server_addr,
                } => (x224_connection_response, server_cert_chain, server_addr),
                ironrdp_rdcleanpath::RDCleanPath::Err(error) => {
                    return Err(
                        IronError::from(anyhow::Error::new(error).context("received an RDCleanPath error"))
                            .with_kind(IronErrorKind::RDCleanPath),
                    );
                }
            };

        let server_addr = server_addr
            .parse()
            .context("failed to parse server address sent by proxy")?;

        connector.attach_client_addr(server_addr);

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

#[allow(clippy::cast_sign_loss)]
#[allow(clippy::cast_possible_truncation)]
fn f64_to_u16_saturating_cast(value: f64) -> u16 {
    value as u16
}
