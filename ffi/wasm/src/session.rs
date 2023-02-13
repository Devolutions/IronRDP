use core::cell::RefCell;
use std::rc::Rc;

use anyhow::Context as _;
use futures_channel::mpsc;
use futures_util::{AsyncWriteExt as _, StreamExt as _};
use gloo_net::websocket::futures::WebSocket;
use ironrdp::geometry::Rectangle;
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::session::connection_sequence::{process_connection_sequence, ConnectionSequenceResult};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStageOutput, ActiveStageProcessor, ErasedWriter, FramedReader, InputConfig, RdpError};
use sspi::AuthIdentity;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::image::RectInfo;
use crate::input::InputTransaction;
use crate::network_client::PlaceholderNetworkClientFactory;
use crate::websocket::WebSocketCompat;
use crate::DesktopSize;

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

        let (writer_tx, writer_rx) = mpsc::unbounded();

        spawn_local(writer_task(writer_rx, rdp_writer));

        Ok(Session {
            input_config,
            connection_sequence_result,
            update_callback,
            update_callback_context,
            input_database: RefCell::new(ironrdp_input::Database::new()),
            rdp_reader: RefCell::new(Some(rdp_reader)),
            writer_tx,
        })
    }
}

#[wasm_bindgen]
pub struct Session {
    input_config: InputConfig,
    connection_sequence_result: ConnectionSequenceResult,
    update_callback: js_sys::Function,
    update_callback_context: JsValue,
    input_database: RefCell<ironrdp_input::Database>,
    rdp_reader: RefCell<Option<FramedReader>>,
    writer_tx: mpsc::UnboundedSender<Vec<u8>>,
}

#[wasm_bindgen]
impl Session {
    pub async fn run(&self) -> Result<(), String> {
        let mut rdp_reader = self
            .rdp_reader
            .borrow_mut()
            .take()
            .ok_or_else(|| "RDP session can be started only once".to_owned())?;

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
            let outputs = {
                let frame = rdp_reader
                    .read_frame()
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| RdpError::AccessDenied.to_string())?
                    .freeze();

                active_stage.process(&mut image, frame).map_err(|e| e.to_string())?
            };

            for out in outputs {
                match out {
                    ActiveStageOutput::ResponseFrame(frame) => {
                        // PERF: unnecessary copy
                        self.writer_tx
                            .unbounded_send(frame.to_vec())
                            .map_err(|e| e.to_string())?;
                    }
                    ActiveStageOutput::GraphicsUpdate(_updated_region) => {
                        // FIXME: atm sending a partial is not working
                        // let partial_image = extract_partial_image(&image, &updated_region);

                        send_update_rectangle(
                            &self.update_callback,
                            &self.update_callback_context,
                            frame_id,
                            Rectangle {
                                left: 0,
                                top: 0,
                                right: image.width() as u16,
                                bottom: image.height() as u16,
                            },
                            image.data().to_vec(),
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

    pub fn apply_inputs(&self, transaction: InputTransaction) -> Result<(), String> {
        let inputs = self.input_database.borrow_mut().apply(transaction);
        self.h_send_inputs(inputs)
    }

    pub fn release_all_inputs(&self) -> Result<(), String> {
        let inputs = self.input_database.borrow_mut().release_all();
        self.h_send_inputs(inputs)
    }

    fn h_send_inputs(
        &self,
        inputs: smallvec::SmallVec<[ironrdp::core::input::fast_path::FastPathInputEvent; 2]>,
    ) -> Result<(), String> {
        use ironrdp::core::input::fast_path::FastPathInput;
        use ironrdp::core::PduParsing as _;

        trace!("Inputs: {inputs:?}");

        if !inputs.is_empty() {
            // PERF: unnecessary copy
            let fastpath_input = FastPathInput(inputs.into_vec());

            let mut frame = Vec::new();
            fastpath_input.to_buffer(&mut frame).map_err(|e| e.to_string())?;

            self.writer_tx.unbounded_send(frame).map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    pub fn synchronize_lock_keys(
        &self,
        scroll_lock: bool,
        num_lock: bool,
        caps_lock: bool,
        kana_lock: bool,
    ) -> Result<(), String> {
        use ironrdp::core::input::fast_path::FastPathInput;
        use ironrdp::core::PduParsing as _;

        let event = ironrdp_input::synchronize_event(scroll_lock, num_lock, caps_lock, kana_lock);
        let fastpath_input = FastPathInput(vec![event]);

        let mut frame = Vec::new();
        fastpath_input.to_buffer(&mut frame).map_err(|e| e.to_string())?;

        self.writer_tx.unbounded_send(frame).map_err(|e| e.to_string())?;

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
        .map_err(|e| anyhow::Error::msg(format!("update callback failed: {e:?}")))?;

    Ok(())
}

async fn writer_task(rx: mpsc::UnboundedReceiver<Vec<u8>>, rdp_writer: ErasedWriter) {
    debug!("writer task started");

    async fn inner(mut rx: mpsc::UnboundedReceiver<Vec<u8>>, mut rdp_writer: ErasedWriter) -> anyhow::Result<()> {
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
