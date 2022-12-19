#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

use core::future::Future;
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use anyhow::Context as _;
use bytes::BytesMut;
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::input::fast_path::FastPathInput;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ConnectionSequenceResult, ErasedWriter, FramedReader, InputConfig};
use ironrdp::geometry::Rectangle;
use ironrdp::session::{process_connection_sequence, ActiveStageOutput, ActiveStageProcessor, RdpError, UpgradedStream};
use serde::Serialize;
use sspi::AuthIdentity;
use tauri::{Manager as _, State};
use tokio::io::AsyncWriteExt as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_util::compat::TokioAsyncReadCompatExt as _;
use x509_parser::prelude::{FromDer as _, X509Certificate};

const DEFAULT_WIDTH: u16 = 1280;
const DEFAULT_HEIGHT: u16 = 720;
const GLOBAL_CHANNEL_NAME: &str = "GLOBAL";
const USER_CHANNEL_NAME: &str = "USER";

type TlsStream = tokio_util::compat::Compat<tokio_rustls::client::TlsStream<TcpStream>>;

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            close_splashscreen,
            init,
            connect,
            update_mouse
        ])
        .setup(|app| {
            if let Some(splashscreen) = app.get_window("splashscreen") {
                splashscreen.set_always_on_top(false).unwrap();
            }
            app.manage(SessionManager::new());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[derive(Clone, Serialize)]
struct DesktopSize {
    width: u16,
    height: u16,
}

#[derive(Clone, Serialize)]
struct ResizeEvent {
    session_id: usize,
    desktop_size: DesktopSize,
}

#[derive(Clone, Serialize)]
struct NewSessionInfo {
    session_id: usize,
    websocket_port: u16,
    initial_desktop_size: DesktopSize,
}

struct SessionManager {
    next_session_id: AtomicUsize,
    sessions: Mutex<HashMap<usize, Session>>,
}

enum ButtonState {
    Unchanged,
    Pressed,
    Released,
}

impl SessionManager {
    fn new() -> Self {
        Self {
            next_session_id: AtomicUsize::new(0),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn send_message(&self, session_id: usize, msg: SessionMessage) -> anyhow::Result<()> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions.get(&session_id).context("session not found")?;
        session.msg_tx.send(msg).ok().context("couldn’t send session message")?;
        Ok(())
    }

    /// Returns previous state
    fn toggle_down(&self, session_id: usize, currently_down: bool) -> anyhow::Result<ButtonState> {
        let mut sessions = self.sessions.lock().unwrap();
        let mut session = sessions.get_mut(&session_id).context("session not found")?;

        match (currently_down, session.was_down) {
            (true, false) => {
                session.was_down = true;
                Ok(ButtonState::Pressed)
            }
            (false, true) => {
                session.was_down = false;
                Ok(ButtonState::Released)
            }
            _ => Ok(ButtonState::Unchanged),
        }
    }

    fn register_session(&self, session: Session) -> usize {
        let session_id = self.next_session_id.fetch_add(1, Ordering::SeqCst);
        self.sessions.lock().unwrap().insert(session_id, session);
        session_id
    }
}

type MessageSender = mpsc::UnboundedSender<SessionMessage>;

type MessageReceiver = mpsc::UnboundedReceiver<SessionMessage>;

struct Session {
    msg_tx: MessageSender,
    was_down: bool,
}

impl Session {
    fn new() -> (Self, MessageReceiver) {
        let (tx, rx) = mpsc::unbounded_channel();
        let session = Self {
            msg_tx: tx,
            was_down: false,
        };
        (session, rx)
    }
}

enum SessionMessage {
    Inputs(FastPathInput),
    ResponseFrame(BytesMut),
}

#[tauri::command]
fn init() {
    // do nothing for now
}

#[tauri::command]
async fn close_splashscreen(window: tauri::Window) {
    if let Some(splashscreen) = window.get_window("splashscreen") {
        splashscreen.close().unwrap();
        window.get_window("main").unwrap().show().unwrap();
    }
}

#[tauri::command]
#[allow(non_snake_case)]
async fn update_mouse(
    sessionId: usize,
    mouseX: u16,
    mouseY: u16,
    leftClick: bool,
    session_manager: State<'_, SessionManager>,
) -> Result<(), String> {
    use ironrdp::input::fast_path::FastPathInputEvent;
    use ironrdp::input::mouse::{ButtonEvents, MovementEvents, WheelEvents};
    use ironrdp::input::MousePdu;

    let session_id = sessionId;
    let mouse_x = mouseX;
    let mouse_y = mouseY;
    let left_click = leftClick;

    let mut inputs = vec![];

    inputs.push(FastPathInputEvent::MouseEvent(MousePdu {
        wheel_events: WheelEvents::empty(),
        movement_events: MovementEvents::MOVE,
        button_events: ButtonEvents::empty(),
        number_of_wheel_rotations: 0,
        x_position: mouse_x,
        y_position: mouse_y,
    }));

    let button_state = session_manager
        .toggle_down(session_id, left_click)
        .map_err(|e| e.to_string())?;

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

    session_manager
        .send_message(session_id, SessionMessage::Inputs(fastpath_input))
        .unwrap();

    Ok(())
}

#[tauri::command]
async fn connect(
    username: String,
    password: String,
    address: String,
    session_manager: State<'_, SessionManager>,
) -> Result<NewSessionInfo, String> {
    let input_config = build_input_config(username, password, None);

    let address = SocketAddr::from_str(&address).unwrap();

    println!("Connect to RDP host");

    let tcp_stream = TcpStream::connect(&address)
        .await
        .map_err(RdpError::ConnectionError)
        .map_err(|e| e.to_string())?;

    let (connection_sequence_result, rdp_reader, rdp_writer) =
        process_connection_sequence(tcp_stream.compat(), &address, &input_config, establish_tls)
            .await
            .map_err(|e| e.to_string())?;

    let desktop_width = connection_sequence_result.desktop_size.width;
    let desktop_height = connection_sequence_result.desktop_size.height;

    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
    let websocket_port = listener.local_addr().unwrap().port();

    let initial_desktop_size = DesktopSize {
        width: desktop_width,
        height: desktop_height,
    };

    let (session, msg_rx) = Session::new();
    let msg_tx = session.msg_tx.clone();
    let session_id = session_manager.register_session(session);
    let new_session_info = NewSessionInfo {
        session_id,
        websocket_port,
        initial_desktop_size,
    };

    start_rdp_session(
        msg_tx,
        msg_rx,
        listener,
        rdp_reader,
        rdp_writer,
        input_config,
        connection_sequence_result,
    );

    Ok(new_session_info)
}

fn build_input_config(username: String, password: String, domain: Option<String>) -> InputConfig {
    InputConfig {
        credentials: AuthIdentity {
            username,
            password,
            domain,
        },
        security_protocol: ironrdp::nego::SecurityProtocol::HYBRID_EX,
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

fn start_rdp_session(
    msg_tx: MessageSender,
    msg_rx: MessageReceiver,
    listener: TcpListener,
    rdp_reader: FramedReader,
    rdp_writer: ErasedWriter,
    input_config: InputConfig,
    connection_sequence_result: ConnectionSequenceResult,
) {
    spawn_task(rdp_session_task(
        msg_tx,
        listener,
        rdp_reader,
        input_config,
        connection_sequence_result,
    ));
    spawn_task(message_writer_task(msg_rx, rdp_writer));
}

async fn message_writer_task(mut msg_rx: MessageReceiver, mut rdp_writer: ErasedWriter) -> anyhow::Result<()> {
    use futures_util::AsyncWriteExt as _;
    use ironrdp::PduParsing;

    while let Some(message) = msg_rx.recv().await {
        match message {
            SessionMessage::ResponseFrame(frame) => {
                rdp_writer.write_all(&frame).await?;
                rdp_writer.flush().await?;
            }
            SessionMessage::Inputs(inputs) => {
                let mut frame = Vec::new();
                inputs.to_buffer(&mut frame).unwrap();
                rdp_writer.write_all(&frame).await?;
                rdp_writer.flush().await?;
            }
        }
    }

    Ok(())
}

async fn rdp_session_task(
    msg_tx: MessageSender,
    listener: TcpListener,
    mut rdp_reader: FramedReader,
    input_config: InputConfig,
    connection_sequence_result: ConnectionSequenceResult,
) -> anyhow::Result<()> {
    println!("RDP session task started");

    // Waiting for frontend to connect via WebSocket
    let (tcp_stream, _) = listener.accept().await?;
    let mut ws_stream = tokio_tungstenite::accept_async(tcp_stream).await?;

    let mut image = DecodedImage::new(
        PixelFormat::RgbA32,
        u32::from(connection_sequence_result.desktop_size.width),
        u32::from(connection_sequence_result.desktop_size.height),
    );

    let mut active_stage = ActiveStageProcessor::new(input_config, None, connection_sequence_result);
    let mut frame_id = 0;

    'outer: loop {
        // FIXME: remove unwraps
        let frame = rdp_reader
            .read_frame()
            .await
            .unwrap()
            .ok_or(RdpError::AccessDenied)
            .unwrap();
        let outputs = active_stage.process(&mut image, frame).await.unwrap();

        for out in outputs {
            match out {
                ActiveStageOutput::ResponseFrame(frame) => {
                    if msg_tx.send(SessionMessage::ResponseFrame(frame)).is_err() {
                        println!("writer task is terminated");
                        break 'outer;
                    };
                }
                ActiveStageOutput::GraphicsUpdate(updated_region) => {
                    let partial_image = extract_partial_image(&image, &updated_region);

                    send_update_rectangle(&mut ws_stream, frame_id, updated_region, partial_image)
                        .await
                        .context("Failed to send update rectangle")?;

                    frame_id += 1;
                }
                ActiveStageOutput::Terminate => break 'outer,
            }
        }
    }

    println!("RPD session terminated");

    Ok(())
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

async fn send_update_rectangle(
    stream: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    frame_id: usize,
    region: Rectangle,
    buffer: Vec<u8>,
) -> anyhow::Result<()> {
    use futures_util::SinkExt as _;
    use tokio_tungstenite::tungstenite::Message;

    let top = region.top;
    let left = region.left;
    let right = region.right;
    let bottom = region.bottom;
    let width = region.width();
    let height = region.height();
    let msg_len = buffer.len();

    stream.feed(Message::Text(format!(
        r#"{{"top": {top}, "left": {left}, "right": {right}, "bottom": {bottom}, "width": {width}, "height": {height}, "frame_id": {frame_id}, "msg_len": {msg_len}}}"#
    ))).await.unwrap();

    stream.feed(Message::Binary(buffer)).await?;

    Ok(())
}

// TODO: this can be refactored in a separate `ironrdp-tls` crate (all native clients will do the same TLS dance)
async fn establish_tls(stream: tokio_util::compat::Compat<TcpStream>) -> Result<UpgradedStream<TlsStream>, RdpError> {
    let stream = stream.into_inner();

    let mut tls_stream = {
        let mut client_config = rustls::client::ClientConfig::builder()
            .with_safe_defaults()
            .with_custom_certificate_verifier(std::sync::Arc::new(danger::NoCertificateVerification))
            .with_no_client_auth();
        // This adds support for the SSLKEYLOGFILE env variable (https://wiki.wireshark.org/TLS#using-the-pre-master-secret)
        client_config.key_log = std::sync::Arc::new(rustls::KeyLogFile::new());
        let rc_config = std::sync::Arc::new(client_config);
        let example_com = "stub_string".try_into().unwrap();
        let connector = tokio_rustls::TlsConnector::from(rc_config);
        connector.connect(example_com, stream).await?
    };

    tls_stream.flush().await?;

    let server_public_key = {
        let cert = tls_stream
            .get_ref()
            .1
            .peer_certificates()
            .ok_or(RdpError::MissingPeerCertificate)?[0]
            .as_ref();
        get_tls_peer_pubkey(cert.to_vec())?
    };

    Ok(UpgradedStream {
        stream: tls_stream.compat(),
        server_public_key,
    })
}

pub fn get_tls_peer_pubkey(cert: Vec<u8>) -> io::Result<Vec<u8>> {
    let res = X509Certificate::from_der(&cert[..])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid der certificate."))?;
    let public_key = res.1.tbs_certificate.subject_pki.subject_public_key;

    Ok(public_key.data.to_vec())
}

mod danger {
    use std::time::SystemTime;

    use rustls::client::ServerCertVerified;
    use rustls::{Certificate, Error, ServerName};

    pub struct NoCertificateVerification;

    impl rustls::client::ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &Certificate,
            _intermediates: &[Certificate],
            _server_name: &ServerName,
            _scts: &mut dyn Iterator<Item = &[u8]>,
            _ocsp_response: &[u8],
            _now: SystemTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(rustls::client::ServerCertVerified::assertion())
        }
    }
}

fn spawn_task<F, T>(task: F)
where
    F: Future<Output = anyhow::Result<T>> + Send + 'static,
    F::Output: Send + 'static,
    T: 'static,
{
    tauri::async_runtime::spawn(async move {
        match task.await {
            Ok(_) => {}
            Err(e) => println!("Task failed: {e:?}"),
        }
    });
}

// TODO: resize event
#[allow(dead_code)]
fn send_resize_event(app: &tauri::AppHandle, session_id: usize, desktop_size: DesktopSize) -> anyhow::Result<()> {
    app.emit_all(
        "resize",
        ResizeEvent {
            session_id,
            desktop_size,
        },
    )?;
    Ok(())
}
