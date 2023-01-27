#[macro_use]
extern crate log;

mod utils;

use core::sync::atomic::AtomicBool;
use std::cell::RefCell;
use std::io;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use anyhow::Context as _;
use futures_util::{pin_mut, ready, AsyncRead, AsyncWrite, AsyncWriteExt, Sink, Stream};
use gloo_net::websocket::futures::WebSocket;
use gloo_net::websocket::{Message as WebSocketMessage, WebSocketError};
use ironrdp::geometry::Rectangle;
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::session::connection_sequence::{process_connection_sequence, ConnectionSequenceResult};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStageOutput, ActiveStageProcessor, ErasedWriter, FramedReader, InputConfig, RdpError};
use parking_lot::Mutex;
use sspi::network_client::{NetworkClient, NetworkClientFactory};
use sspi::AuthIdentity;
use wasm_bindgen::prelude::*;

// TODO: proper error reporting

// NOTE: #[wasm_bindgen(start)] didn’t work last time I tried
#[wasm_bindgen]
pub fn ironrdp_init() {
    utils::set_panic_hook();
    console_log::init_with_level(log::Level::Debug).unwrap();
}

#[wasm_bindgen]
#[derive(Clone, Default)]
pub struct SessionBuilder(Rc<RefCell<SessionBuilderInner>>);

#[derive(Default)]
struct SessionBuilderInner {
    username: Option<String>,
    password: Option<String>,
    address: Option<String>,
    auth_token: Option<String>,
    update_callback: Option<js_sys::Function>,
    update_callback_context: Option<JsValue>,
}

#[wasm_bindgen]
impl SessionBuilder {
    pub fn new() -> SessionBuilder {
        Self(Rc::new(RefCell::new(SessionBuilderInner::default())))
    }

    pub fn username(&self, username: String) -> SessionBuilder {
        self.0.borrow_mut().username = Some(username);
        self.clone()
    }

    pub fn password(&self, password: String) -> SessionBuilder {
        self.0.borrow_mut().password = Some(password);
        self.clone()
    }

    pub fn address(&self, address: String) -> SessionBuilder {
        self.0.borrow_mut().address = Some(address);
        self.clone()
    }

    pub fn auth_token(&self, token: String) -> SessionBuilder {
        self.0.borrow_mut().auth_token = Some(token);
        self.clone()
    }

    pub fn update_callback(&self, callback: js_sys::Function) -> SessionBuilder {
        self.0.borrow_mut().update_callback = Some(callback);
        self.clone()
    }

    pub fn update_callback_context(&self, context: JsValue) -> SessionBuilder {
        self.0.borrow_mut().update_callback_context = Some(context);
        self.clone()
    }

    pub async fn connect(&self) -> Result<Session, String> {
        let (username, password, address, auth_token, update_callback, update_callback_context);

        {
            let inner = self.0.borrow();
            username = inner.username.clone().expect("username");
            password = inner.password.clone().expect("password");
            address = inner.address.clone().expect("address");
            auth_token = inner.auth_token.clone().expect("auth_token");
            update_callback = inner.update_callback.clone().expect("update_callback");
            update_callback_context = inner.update_callback_context.clone().expect("update_callback_context");
        }

        info!("Connect to RDP host");

        let input_config = build_input_config(username, password, None);

        let ws = WebSocketCompat::new(WebSocket::open(&address).map_err(|e| e.to_string())?);

        let (connection_sequence_result, rdp_reader, rdp_writer) = process_connection_sequence(
            ws,
            "MY-FQDN",
            auth_token,
            &input_config,
            Box::new(PlaceholderNetworkClientFactory),
        )
        .await
        .map_err(|e| anyhow::Error::new(e).to_string())?;

        info!("Connected!");

        Ok(Session {
            input_config,
            connection_sequence_result,
            update_callback,
            update_callback_context,
            was_down: AtomicBool::new(false),
            rdp_reader: Mutex::new(rdp_reader),
            rdp_writer: Mutex::new(rdp_writer),
        })
    }
}

#[wasm_bindgen]
#[derive(Clone)]
pub struct DesktopSize {
    pub width: u16,
    pub height: u16,
}

struct WebSocketCompat {
    read_buf: Option<Vec<u8>>,
    inner: WebSocket,
}

impl WebSocketCompat {
    fn new(ws: WebSocket) -> Self {
        Self {
            read_buf: None,
            inner: ws,
        }
    }
}

impl AsyncRead for WebSocketCompat {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let read_buf = &mut this.read_buf;
        let inner = &mut this.inner;
        pin_mut!(inner);

        let mut data = if let Some(data) = read_buf.take() {
            data
        } else {
            match ready!(inner.as_mut().poll_next(cx)) {
                Some(Ok(m)) => match m {
                    WebSocketMessage::Text(s) => s.into_bytes(),
                    WebSocketMessage::Bytes(data) => data,
                },
                Some(Err(e)) => return Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e.to_string()))),
                None => return Poll::Ready(Ok(0)),
            }
        };

        let bytes_to_copy = std::cmp::min(buf.len(), data.len());
        buf[..bytes_to_copy].copy_from_slice(&data[..bytes_to_copy]);

        if data.len() > bytes_to_copy {
            data.drain(..bytes_to_copy);
            *read_buf = Some(data);
        }

        Poll::Ready(Ok(bytes_to_copy))
    }
}

impl AsyncWrite for WebSocketCompat {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        macro_rules! try_in_poll {
            ($expr:expr) => {{
                match $expr {
                    Ok(o) => o,
                    // When using `AsyncWriteExt::write_all`, `io::ErrorKind::WriteZero` will be raised.
                    // In this case it means "attempted to write on a closed socket".
                    Err(WebSocketError::ConnectionClose(_)) => return Poll::Ready(Ok(0)),
                    Err(e) => return Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e.to_string()))),
                }
            }};
        }

        let inner = &mut self.get_mut().inner;
        pin_mut!(inner);

        // try flushing preemptively
        let _ = inner.as_mut().poll_flush(cx);

        // make sure sink is ready to send
        try_in_poll!(ready!(inner.as_mut().poll_ready(cx)));

        // actually submit new item
        try_in_poll!(inner.as_mut().start_send(WebSocketMessage::Bytes(buf.to_vec())));
        // ^ if no error occurred, message is accepted and queued when calling `start_send`
        // (that is: `to_vec` is called only once)

        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let inner = &mut self.get_mut().inner;
        pin_mut!(inner);
        let res = ready!(inner.poll_flush(cx));
        Poll::Ready(websocket_to_io_result(res))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let inner = &mut self.get_mut().inner;
        pin_mut!(inner);
        let res = ready!(inner.poll_close(cx));
        Poll::Ready(websocket_to_io_result(res))
    }
}

fn websocket_to_io_result(res: Result<(), WebSocketError>) -> io::Result<()> {
    match res {
        Ok(()) => Ok(()),
        Err(WebSocketError::ConnectionClose(_)) => Ok(()),
        Err(e) => Err(io::Error::new(io::ErrorKind::Other, e.to_string())),
    }
}

enum ButtonState {
    Unchanged,
    Pressed,
    Released,
}

#[wasm_bindgen]
pub struct Session {
    input_config: InputConfig,
    connection_sequence_result: ConnectionSequenceResult,
    update_callback: js_sys::Function,
    update_callback_context: JsValue,
    was_down: AtomicBool,
    rdp_reader: Mutex<FramedReader>,
    rdp_writer: Mutex<ErasedWriter>,
}

#[wasm_bindgen]
impl Session {
    pub async fn run(&self) -> Result<(), String> {
        info!("Start RDP session");

        let mut image = DecodedImage::new(
            PixelFormat::RgbA32,
            u32::from(self.connection_sequence_result.desktop_size.width),
            u32::from(self.connection_sequence_result.desktop_size.height),
        );

        let mut active_stage =
            ActiveStageProcessor::new(self.input_config.clone(), None, self.connection_sequence_result.clone());
        let mut frame_id = 0;

        'outer: loop {
            let frame = self
                .rdp_reader
                .lock()
                .read_frame()
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| RdpError::AccessDenied.to_string())?
                .freeze();

            let outputs = active_stage.process(&mut image, frame).map_err(|e| e.to_string())?;

            for out in outputs {
                match out {
                    ActiveStageOutput::ResponseFrame(frame) => {
                        let mut writer = self.rdp_writer.lock();
                        writer.write_all(&frame).await.map_err(|e| e.to_string())?;
                        writer.flush().await.map_err(|e| e.to_string())?;
                    }
                    ActiveStageOutput::GraphicsUpdate(updated_region) => {
                        let partial_image = extract_partial_image(&image, &updated_region);

                        send_update_rectangle(
                            &self.update_callback,
                            &self.update_callback_context,
                            frame_id,
                            updated_region,
                            partial_image,
                        )
                        .context("Failed to send update rectangle")
                        .map_err(|e| e.to_string())?;

                        frame_id += 1;
                    }
                    ActiveStageOutput::Terminate => break 'outer,
                }
            }
        }

        info!("RPD session terminated");

        Ok(())
    }

    pub fn desktop_size(&self) -> DesktopSize {
        let desktop_width = self.connection_sequence_result.desktop_size.width;
        let desktop_height = self.connection_sequence_result.desktop_size.height;

        DesktopSize {
            width: desktop_width,
            height: desktop_height,
        }
    }

    /// Returns previous state
    fn toggle_down(&self, currently_down: bool) -> anyhow::Result<ButtonState> {
        let was_down = self.was_down.swap(currently_down, core::sync::atomic::Ordering::SeqCst);

        match (currently_down, was_down) {
            (true, false) => Ok(ButtonState::Pressed),
            (false, true) => Ok(ButtonState::Released),
            _ => Ok(ButtonState::Unchanged),
        }
    }

    pub async fn update_mouse(&self, mouse_x: u16, mouse_y: u16, left_click: bool) -> Result<(), String> {
        use ironrdp::core::input::fast_path::{FastPathInput, FastPathInputEvent};
        use ironrdp::core::input::mouse::{ButtonEvents, MovementEvents, WheelEvents};
        use ironrdp::core::input::MousePdu;
        use ironrdp::core::PduParsing;

        let mut inputs = vec![];

        inputs.push(FastPathInputEvent::MouseEvent(MousePdu {
            wheel_events: WheelEvents::empty(),
            movement_events: MovementEvents::MOVE,
            button_events: ButtonEvents::empty(),
            number_of_wheel_rotations: 0,
            x_position: mouse_x,
            y_position: mouse_y,
        }));

        let button_state = self.toggle_down(left_click).map_err(|e| e.to_string())?;

        match button_state {
            ButtonState::Pressed => {
                inputs.push(FastPathInputEvent::MouseEvent(MousePdu {
                    wheel_events: WheelEvents::empty(),
                    movement_events: MovementEvents::empty(),
                    button_events: ButtonEvents::DOWN | ButtonEvents::LEFT_BUTTON,
                    number_of_wheel_rotations: 0,
                    x_position: mouse_x,
                    y_position: mouse_y,
                }));
            }
            ButtonState::Released => {
                inputs.push(FastPathInputEvent::MouseEvent(MousePdu {
                    wheel_events: WheelEvents::empty(),
                    movement_events: MovementEvents::empty(),
                    button_events: ButtonEvents::LEFT_BUTTON,
                    number_of_wheel_rotations: 0,
                    x_position: mouse_x,
                    y_position: mouse_y,
                }));
            }
            ButtonState::Unchanged => {}
        }

        let fastpath_input = FastPathInput(inputs);

        let mut frame = Vec::new();
        fastpath_input.to_buffer(&mut frame).map_err(|e| e.to_string())?;

        let mut writer = self.rdp_writer.lock();
        writer.write_all(&frame).await.map_err(|e| e.to_string())?;
        writer.flush().await.map_err(|e| e.to_string())?;

        Ok(())
    }
}

fn build_input_config(username: String, password: String, domain: Option<String>) -> InputConfig {
    const DEFAULT_WIDTH: u16 = 1280;
    const DEFAULT_HEIGHT: u16 = 720;
    const GLOBAL_CHANNEL_NAME: &str = "GLOBAL";
    const USER_CHANNEL_NAME: &str = "USER";

    InputConfig {
        credentials: AuthIdentity {
            username,
            password,
            domain,
        },
        security_protocol: ironrdp::SecurityProtocol::HYBRID_EX,
        keyboard_type: ironrdp::gcc::KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        width: DEFAULT_WIDTH,
        height: DEFAULT_HEIGHT,
        global_channel_name: GLOBAL_CHANNEL_NAME.to_owned(),
        user_channel_name: USER_CHANNEL_NAME.to_owned(),
        graphics_config: None,
    }
}

fn extract_partial_image(image: &DecodedImage, region: &Rectangle) -> Vec<u8> {
    let pixel_size = usize::from(image.pixel_format().bytes_per_pixel());

    let image_width = usize::try_from(image.width()).unwrap();

    let region_top = usize::from(region.top);
    let region_left = usize::from(region.left);
    let region_width = usize::from(region.width());
    let region_height = usize::from(region.height());

    let dst_buf_size = region_width * region_height * pixel_size;
    let mut dst = vec![0; dst_buf_size];

    let src = image.data();

    let image_stride = image_width * pixel_size;
    let region_stride = region_width * pixel_size;

    for row in 0..region_height {
        let src_begin = image_stride * (region_top + row) + region_left * pixel_size;
        let src_end = src_begin + region_stride;
        let src_slice = &src[src_begin..src_end];

        let target_begin = region_stride * row;
        let target_end = target_begin + region_stride;
        let target_slice = &mut dst[target_begin..target_end];

        target_slice.copy_from_slice(src_slice);
    }

    dst
}

#[wasm_bindgen]
pub struct RectInfo {
    pub frame_id: usize,
    pub top: u16,
    pub left: u16,
    pub right: u16,
    pub bottom: u16,
    pub width: u16,
    pub height: u16,
}

fn send_update_rectangle(
    update_callback: &js_sys::Function,
    callback_context: &JsValue,
    frame_id: usize,
    region: Rectangle,
    buffer: Vec<u8>,
) -> anyhow::Result<()> {
    use js_sys::Uint8ClampedArray;

    let top = region.top;
    let left = region.left;
    let right = region.right;
    let bottom = region.bottom;
    let width = region.width();
    let height = region.height();

    let update_rect = RectInfo {
        frame_id,
        top,
        left,
        right,
        bottom,
        width,
        height,
    };
    let update_rect = JsValue::from(update_rect);

    let js_array = Uint8ClampedArray::new_with_length(buffer.len() as u32);
    js_array.copy_from(&buffer);
    let js_array = JsValue::from(js_array);

    let _ret = update_callback
        .call2(callback_context, &update_rect, &js_array)
        .map_err(|e| anyhow::Error::msg(format!("update callback failed: {:?}", e)))?;

    Ok(())
}

#[derive(Debug, Clone)]
struct PlaceholderNetworkClientFactory;

impl NetworkClientFactory for PlaceholderNetworkClientFactory {
    fn network_client(&self) -> Box<dyn NetworkClient> {
        unimplemented!()
    }

    fn clone(&self) -> Box<dyn NetworkClientFactory> {
        Box::new(Clone::clone(self))
    }
}
