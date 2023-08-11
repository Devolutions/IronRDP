use core::cell::RefCell;
use std::rc::Rc;

use anyhow::Context as _;
use futures_channel::mpsc;
use futures_util::io::{ReadHalf, WriteHalf};
use futures_util::{AsyncReadExt as _, AsyncWriteExt as _, StreamExt as _};
use gloo_net::websocket;
use gloo_net::websocket::futures::WebSocket;
use ironrdp::connector::{self, ClientConnector};
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::pdu::geometry::Rectangle;
use ironrdp::pdu::write_buf::WriteBuf;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput};
use tap::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::error::{IronRdpError, IronRdpErrorKind};
use crate::image::{extract_partial_image, RectInfo};
use crate::input::InputTransaction;
use crate::network_client::PlaceholderNetworkClientFactory;
use crate::websocket::WebSocketCompat;
use crate::DesktopSize;

const DEFAULT_WIDTH: u16 = 1280;
const DEFAULT_HEIGHT: u16 = 720;

#[wasm_bindgen]
#[derive(Clone, Default)]
pub struct SessionBuilder(Rc<RefCell<SessionBuilderInner>>);

struct SessionBuilderInner {
    username: Option<String>,
    destination: Option<String>,
    server_domain: Option<String>,
    password: Option<String>,
    proxy_address: Option<String>,
    auth_token: Option<String>,
    pcb: Option<String>,
    client_name: String,
    desktop_size: DesktopSize,
    update_callback: Option<js_sys::Function>,
    update_callback_context: Option<JsValue>,
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
            client_name: "ironrdp-web".to_owned(),
            desktop_size: DesktopSize {
                width: DEFAULT_WIDTH,
                height: DEFAULT_HEIGHT,
            },
            update_callback: None,
            update_callback_context: None,
        }
    }
}

#[wasm_bindgen]
impl SessionBuilder {
    pub fn new() -> SessionBuilder {
        Self(Rc::new(RefCell::new(SessionBuilderInner::default())))
    }

    /// Required
    pub fn username(&self, username: String) -> SessionBuilder {
        self.0.borrow_mut().username = Some(username);
        self.clone()
    }

    /// Required
    pub fn destination(&self, destination: String) -> SessionBuilder {
        self.0.borrow_mut().destination = Some(destination);
        self.clone()
    }

    /// Optional
    pub fn server_domain(&self, server_domain: String) -> SessionBuilder {
        self.0.borrow_mut().server_domain = if server_domain.is_empty() {
            None
        } else {
            Some(server_domain)
        };
        self.clone()
    }

    /// Required
    pub fn password(&self, password: String) -> SessionBuilder {
        self.0.borrow_mut().password = Some(password);
        self.clone()
    }

    /// Required
    pub fn proxy_address(&self, address: String) -> SessionBuilder {
        self.0.borrow_mut().proxy_address = Some(address);
        self.clone()
    }

    /// Required
    pub fn auth_token(&self, token: String) -> SessionBuilder {
        self.0.borrow_mut().auth_token = Some(token);
        self.clone()
    }

    /// Optional
    pub fn pcb(&self, pcb: String) -> SessionBuilder {
        self.0.borrow_mut().pcb = Some(pcb);
        self.clone()
    }

    /// Optional
    pub fn desktop_size(&self, desktop_size: DesktopSize) -> SessionBuilder {
        self.0.borrow_mut().desktop_size = desktop_size;
        self.clone()
    }

    /// Required
    pub fn update_callback(&self, callback: js_sys::Function) -> SessionBuilder {
        self.0.borrow_mut().update_callback = Some(callback);
        self.clone()
    }

    /// Required
    pub fn update_callback_context(&self, context: JsValue) -> SessionBuilder {
        self.0.borrow_mut().update_callback_context = Some(context);
        self.clone()
    }

    pub async fn connect(&self) -> Result<Session, IronRdpError> {
        let (
            username,
            destination,
            server_domain,
            password,
            proxy_address,
            auth_token,
            pcb,
            client_name,
            desktop_size,
            update_callback,
            update_callback_context,
        );

        {
            let inner = self.0.borrow();
            username = inner.username.clone().expect("username");
            destination = inner.destination.clone().expect("destination");
            server_domain = inner.server_domain.clone();
            password = inner.password.clone().expect("password");
            proxy_address = inner.proxy_address.clone().expect("proxy_address");
            auth_token = inner.auth_token.clone().expect("auth_token");
            pcb = inner.pcb.clone();
            client_name = inner.client_name.clone();
            desktop_size = inner.desktop_size.clone();
            update_callback = inner.update_callback.clone().expect("update_callback");
            update_callback_context = inner.update_callback_context.clone().expect("update_callback_context");
        }

        info!("Connect to RDP host");

        let config = build_config(username, password, server_domain, client_name, desktop_size);

        let ws = WebSocket::open(&proxy_address).context("Couldn’t open WebSocket")?;

        if matches!(ws.state(), websocket::State::Closing | websocket::State::Closed) {
            return Err(IronRdpError::from(anyhow::anyhow!(
                "Failed to connect to {proxy_address} (WebSocket is in state {:?})",
                ws.state()
            ))
            .with_kind(IronRdpErrorKind::ProxyConnect));
        }

        let ws = WebSocketCompat::new(ws);

        let (connection_result, ws) = connect(ws, config, auth_token, destination, pcb).await?;

        info!("Connected!");

        let (rdp_reader, rdp_writer) = ws.split();

        let (writer_tx, writer_rx) = mpsc::unbounded();

        spawn_local(writer_task(writer_rx, rdp_writer));

        Ok(Session {
            desktop_size: connection_result.desktop_size.clone(),
            update_callback,
            update_callback_context,
            input_database: RefCell::new(ironrdp::input::Database::new()),
            writer_tx,

            rdp_reader: RefCell::new(Some(rdp_reader)),
            connection_result: RefCell::new(Some(connection_result)),
        })
    }
}

#[wasm_bindgen]
pub struct Session {
    desktop_size: connector::DesktopSize,
    update_callback: js_sys::Function,
    update_callback_context: JsValue,
    input_database: RefCell<ironrdp::input::Database>,
    writer_tx: mpsc::UnboundedSender<Vec<u8>>,

    // Consumed when `run` is called
    connection_result: RefCell<Option<connector::ConnectionResult>>,
    rdp_reader: RefCell<Option<ReadHalf<WebSocketCompat>>>,
}

#[wasm_bindgen]
impl Session {
    pub async fn run(&self) -> Result<(), IronRdpError> {
        let rdp_reader = self
            .rdp_reader
            .borrow_mut()
            .take()
            .context("RDP session can be started only once")?;

        let connection_result = self
            .connection_result
            .borrow_mut()
            .take()
            .expect("run called only once");

        let mut framed = ironrdp_futures::FuturesFramed::new(rdp_reader);

        info!("Start RDP session");

        let mut image = DecodedImage::new(PixelFormat::RgbA32, self.desktop_size.width, self.desktop_size.height);

        let mut active_stage = ActiveStage::new(connection_result, None);
        let mut frame_id = 0;

        'outer: loop {
            let outputs = {
                let (action, frame) = framed.read_pdu().await.context("read next frame")?;

                active_stage
                    .process(&mut image, action, &frame)
                    .context("Active stage processing")?
            };

            for out in outputs {
                match out {
                    ActiveStageOutput::ResponseFrame(frame) => {
                        // PERF: unnecessary copy
                        self.writer_tx
                            .unbounded_send(frame.to_vec())
                            .context("Send frame to writer task")?;
                    }
                    ActiveStageOutput::GraphicsUpdate(updated_region) => {
                        let (partial_image_rectangle, partial_image) = extract_partial_image(&image, updated_region);

                        send_update_rectangle(
                            &self.update_callback,
                            &self.update_callback_context,
                            frame_id,
                            partial_image_rectangle,
                            partial_image,
                        )
                        .context("Failed to send update rectangle")?;

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
        DesktopSize {
            width: self.desktop_size.width,
            height: self.desktop_size.height,
        }
    }

    pub fn apply_inputs(&self, transaction: InputTransaction) -> Result<(), IronRdpError> {
        let inputs = self.input_database.borrow_mut().apply(transaction);
        self.h_send_inputs(inputs)
    }

    pub fn release_all_inputs(&self) -> Result<(), IronRdpError> {
        let inputs = self.input_database.borrow_mut().release_all();
        self.h_send_inputs(inputs)
    }

    fn h_send_inputs(
        &self,
        inputs: smallvec::SmallVec<[ironrdp::pdu::input::fast_path::FastPathInputEvent; 2]>,
    ) -> Result<(), IronRdpError> {
        use ironrdp::pdu::input::fast_path::FastPathInput;
        use ironrdp::pdu::PduParsing as _;

        trace!("Inputs: {inputs:?}");

        if !inputs.is_empty() {
            // PERF: unnecessary copy
            let fastpath_input = FastPathInput(inputs.into_vec());

            let mut frame = Vec::new();
            fastpath_input.to_buffer(&mut frame).context("FastPathInput encoding")?;

            self.writer_tx
                .unbounded_send(frame)
                .context("Send frame to writer task")?;
        }

        Ok(())
    }

    pub fn synchronize_lock_keys(
        &self,
        scroll_lock: bool,
        num_lock: bool,
        caps_lock: bool,
        kana_lock: bool,
    ) -> Result<(), IronRdpError> {
        use ironrdp::pdu::input::fast_path::FastPathInput;
        use ironrdp::pdu::PduParsing as _;

        let event = ironrdp::input::synchronize_event(scroll_lock, num_lock, caps_lock, kana_lock);
        let fastpath_input = FastPathInput(vec![event]);

        let mut frame = Vec::new();
        fastpath_input.to_buffer(&mut frame).context("FastPathInput encoding")?;

        self.writer_tx
            .unbounded_send(frame)
            .context("Send frame to writer task")?;

        Ok(())
    }

    pub fn shutdown(&self) -> Result<(), IronRdpError> {
        // TODO: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/27915739-8f77-487e-9927-55008af7fd68
        Ok(())
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
        username,
        password,
        domain,
        security_protocol: ironrdp::pdu::nego::SecurityProtocol::HYBRID,
        keyboard_type: ironrdp::pdu::gcc::KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: connector::DesktopSize {
            width: desktop_size.width,
            height: desktop_size.height,
        },
        graphics: None,
        bitmap: Some(connector::BitmapConfig {
            color_depth: 16,
            lossy_compression: true,
        }),
        client_build: semver::Version::parse(env!("CARGO_PKG_VERSION"))
            .map(|version| version.major * 100 + version.minor * 10 + version.patch)
            .unwrap_or(0)
            .pipe(u32::try_from)
            .unwrap(),
        client_name,
        // NOTE: hardcode this value like in freerdp
        // https://github.com/FreeRDP/FreeRDP/blob/4e24b966c86fdf494a782f0dfcfc43a057a2ea60/libfreerdp/core/settings.c#LL49C34-L49C70
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),
        platform: ironrdp::pdu::rdp::capability_sets::MajorPlatformType::Unspecified,
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

async fn writer_task(rx: mpsc::UnboundedReceiver<Vec<u8>>, rdp_writer: WriteHalf<WebSocketCompat>) {
    debug!("writer task started");

    async fn inner(
        mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
        mut rdp_writer: WriteHalf<WebSocketCompat>,
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

async fn connect(
    ws: WebSocketCompat,
    config: connector::Config,
    proxy_auth_token: String,
    destination: String,
    pcb: Option<String>,
) -> Result<(connector::ConnectionResult, WebSocketCompat), IronRdpError> {
    let mut framed = ironrdp_futures::FuturesFramed::new(ws);

    let mut connector = connector::ClientConnector::new(config)
        .with_server_name(&destination)
        .with_credssp_network_client(PlaceholderNetworkClientFactory)
        .with_static_channel(ironrdp::dvc::Drdynvc::new());

    let upgraded = connect_rdcleanpath(&mut framed, &mut connector, destination, proxy_auth_token, pcb).await?;

    let connection_result = ironrdp_futures::connect_finalize(upgraded, &mut framed, connector).await?;

    let ws = framed.into_inner_no_leftover();

    Ok((connection_result, ws))
}

async fn connect_rdcleanpath<S>(
    framed: &mut ironrdp_futures::Framed<S>,
    connector: &mut ClientConnector,
    destination: String,
    proxy_auth_token: String,
    pcb: Option<String>,
) -> Result<ironrdp_futures::Upgraded, IronRdpError>
where
    S: ironrdp_futures::FramedRead + ironrdp_futures::FramedWrite,
{
    use ironrdp::connector::Sequence as _;
    use x509_cert::der::Decode as _;

    #[derive(Clone, Copy, Debug)]
    pub struct RDCleanPathHint;

    pub const RDCLEANPATH_HINT: RDCleanPathHint = RDCleanPathHint;

    impl ironrdp::pdu::PduHint for RDCleanPathHint {
        fn find_size(&self, bytes: &[u8]) -> ironrdp::pdu::PduResult<Option<usize>> {
            match ironrdp_rdcleanpath::RDCleanPathPdu::detect(bytes) {
                ironrdp_rdcleanpath::DetectionResult::Detected { total_length, .. } => Ok(Some(total_length)),
                ironrdp_rdcleanpath::DetectionResult::NotEnoughBytes => Ok(None),
                ironrdp_rdcleanpath::DetectionResult::Failed => Err(ironrdp::pdu::other_err!(
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

        let ironrdp::connector::ClientConnectorState::ConnectionInitiationSendRequest = connector.state else {
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
                        IronRdpError::from(anyhow::anyhow!("received an RDCleanPath error: {error}"))
                            .with_kind(IronRdpErrorKind::RDCleanPath),
                    );
                }
            };

        let server_addr = server_addr
            .parse()
            .context("failed to parse server address sent by proxy")?;

        connector.attach_server_addr(server_addr);

        let ironrdp::connector::ClientConnectorState::ConnectionInitiationWaitConfirm = connector.state else {
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

        // At this point, proxy established the TLS session

        let upgraded = ironrdp_futures::mark_as_upgraded(should_upgrade, connector, server_public_key);

        Ok(upgraded)
    }
}
