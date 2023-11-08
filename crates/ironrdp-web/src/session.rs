use core::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use anyhow::Context as _;
use futures_channel::mpsc;
use futures_util::io::{ReadHalf, WriteHalf};
use futures_util::{select, AsyncReadExt as _, AsyncWriteExt as _, FutureExt as _, StreamExt as _};
use gloo_net::websocket;
use gloo_net::websocket::futures::WebSocket;
use ironrdp::connector::{self, ClientConnector, Credentials, KerberosConfig};
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::pdu::input::fast_path::FastPathInputEvent;
use ironrdp::pdu::write_buf::WriteBuf;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput};
use tap::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlCanvasElement;

use crate::canvas::Canvas;
use crate::error::{IronRdpError, IronRdpErrorKind};
use crate::image::extract_partial_image;
use crate::input::InputTransaction;
use crate::network_client::WasmNetworkClient;
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
    kdc_proxy_url: Option<String>,
    client_name: String,
    desktop_size: DesktopSize,

    render_canvas: Option<HtmlCanvasElement>,
    hide_pointer_callback: Option<js_sys::Function>,
    hide_pointer_callback_context: Option<JsValue>,
    show_pointer_callback: Option<js_sys::Function>,
    show_pointer_callback_context: Option<JsValue>,
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
            hide_pointer_callback: None,
            hide_pointer_callback_context: None,
            show_pointer_callback: None,
            show_pointer_callback_context: None,
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

    /// Optional
    pub fn render_canvas(&self, canvas: HtmlCanvasElement) -> SessionBuilder {
        self.0.borrow_mut().render_canvas = Some(canvas);
        self.clone()
    }

    /// Required
    pub fn hide_pointer_callback(&self, callback: js_sys::Function) -> SessionBuilder {
        self.0.borrow_mut().hide_pointer_callback = Some(callback);
        self.clone()
    }

    /// Required
    pub fn hide_pointer_callback_context(&self, context: JsValue) -> SessionBuilder {
        self.0.borrow_mut().hide_pointer_callback_context = Some(context);
        self.clone()
    }

    /// Required
    pub fn show_pointer_callback(&self, callback: js_sys::Function) -> SessionBuilder {
        self.0.borrow_mut().show_pointer_callback = Some(callback);
        self.clone()
    }

    /// Required
    pub fn show_pointer_callback_context(&self, context: JsValue) -> SessionBuilder {
        self.0.borrow_mut().show_pointer_callback_context = Some(context);
        self.clone()
    }

    /// Optional
    pub fn kdc_proxy_url(&self, kdc_proxy_url: Option<String>) -> SessionBuilder {
        self.0.borrow_mut().kdc_proxy_url = kdc_proxy_url;
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
            render_canvas,
            hide_pointer_callback,
            hide_pointer_callback_context,
            show_pointer_callback,
            show_pointer_callback_context,
            kdc_proxy_url,
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
            client_name = inner.client_name.clone();
            desktop_size = inner.desktop_size.clone();

            kdc_proxy_url = inner.kdc_proxy_url.clone();

            render_canvas = inner.render_canvas.clone().context("render_canvas missing")?;

            hide_pointer_callback = inner
                .hide_pointer_callback
                .clone()
                .context("hide_pointer_callback missing")?;
            hide_pointer_callback_context = inner
                .hide_pointer_callback_context
                .clone()
                .context("show_pointer_callback_context missing")?;
            show_pointer_callback = inner
                .show_pointer_callback
                .clone()
                .context("hide_pointer_callback missing")?;
            show_pointer_callback_context = inner
                .show_pointer_callback_context
                .clone()
                .context("show_pointer_callback_context missing")?;
        }

        info!("Connect to RDP host");

        let config = build_config(username, password, server_domain, client_name, desktop_size);

        let ws = WebSocket::open(&proxy_address).context("Couldn’t open WebSocket")?;

        // NOTE: ideally, when the WebSocket can’t be opened, the above call should fail with details on why is that
        // (e.g., the proxy hostname could not be resolved, proxy service is not running), but errors are neved
        // bubbled up in practice, so instead we poll the WebSocket state until we know its connected (i.e., the
        // WebSocket handshake is a success and user data can be exchanged).
        loop {
            match ws.state() {
                websocket::State::Closing | websocket::State::Closed => {
                    return Err(IronRdpError::from(anyhow::anyhow!(
                        "Failed to connect to {proxy_address} (WebSocket is `{:?}`)",
                        ws.state()
                    ))
                    .with_kind(IronRdpErrorKind::ProxyConnect));
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

        let ws = WebSocketCompat::new(ws);

        let (connection_result, ws) = connect(ws, config, auth_token, destination, pcb, kdc_proxy_url).await?;

        info!("Connected!");

        let (rdp_reader, rdp_writer) = ws.split();

        let (writer_tx, writer_rx) = mpsc::unbounded();

        let (input_events_tx, input_events_rx) = mpsc::unbounded();

        spawn_local(writer_task(writer_rx, rdp_writer));

        Ok(Session {
            desktop_size: connection_result.desktop_size.clone(),
            input_database: RefCell::new(ironrdp::input::Database::new()),
            writer_tx,
            input_events_tx,

            render_canvas,
            hide_pointer_callback,
            hide_pointer_callback_context,
            show_pointer_callback,
            show_pointer_callback_context,

            input_events_rx: RefCell::new(Some(input_events_rx)),
            rdp_reader: RefCell::new(Some(rdp_reader)),
            connection_result: RefCell::new(Some(connection_result)),
        })
    }
}

type FastPathInputEvents = smallvec::SmallVec<[FastPathInputEvent; 2]>;

#[wasm_bindgen]
pub struct Session {
    desktop_size: connector::DesktopSize,
    input_database: RefCell<ironrdp::input::Database>,
    writer_tx: mpsc::UnboundedSender<Vec<u8>>,
    input_events_tx: mpsc::UnboundedSender<FastPathInputEvents>,

    render_canvas: HtmlCanvasElement,
    hide_pointer_callback: js_sys::Function,
    hide_pointer_callback_context: JsValue,
    show_pointer_callback: js_sys::Function,
    show_pointer_callback_context: JsValue,

    // Consumed when `run` is called
    input_events_rx: RefCell<Option<mpsc::UnboundedReceiver<FastPathInputEvents>>>,
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

        let mut fastpath_input_events = self
            .input_events_rx
            .borrow_mut()
            .take()
            .context("RDP session can be started only once")?;

        let connection_result = self
            .connection_result
            .borrow_mut()
            .take()
            .expect("run called only once");

        let mut framed = ironrdp_futures::SingleThreadedFuturesFramed::new(rdp_reader);

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

        let mut active_stage = ActiveStage::new(connection_result, None);

        'outer: loop {
            let outputs = select! {
                frame = framed.read_pdu().fuse() => {
                    let (action, payload) = frame.context("read frame")?;
                    trace!(?action, frame_length = payload.len(), "Frame received");

                    active_stage.process(&mut image, action, &payload)?
                }
                input_events = fastpath_input_events.next() => {
                    let events = input_events.context("read next fastpath input events")?;

                    active_stage.process_fastpath_input(&mut image, &events).context("Fast path input events processing")?
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
                        let _ret = self
                            .show_pointer_callback
                            .call0(&self.show_pointer_callback_context)
                            .map_err(|e| anyhow::Error::msg(format!("show pointer callback failed: {e:?}")))?;
                    }
                    ActiveStageOutput::PointerHidden => {
                        let _ret = self
                            .hide_pointer_callback
                            .call0(&self.hide_pointer_callback_context)
                            .map_err(|e| anyhow::Error::msg(format!("hide pointer callback failed: {e:?}")))?;
                    }
                    ActiveStageOutput::PointerPosition { .. } => {
                        // Not applicable for web
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

    fn h_send_inputs(&self, inputs: smallvec::SmallVec<[FastPathInputEvent; 2]>) -> Result<(), IronRdpError> {
        if !inputs.is_empty() {
            trace!("Inputs: {inputs:?}");

            self.input_events_tx
                .unbounded_send(inputs)
                .context("Send input events to writer task")?;
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

    #[allow(clippy::unused_self)] // FIXME: not yet implemented
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
        credentials: Credentials::UsernamePassword { username, password },
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
    }
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
    kdc_proxy_url: Option<String>,
) -> Result<(connector::ConnectionResult, WebSocketCompat), IronRdpError> {
    let mut framed = ironrdp_futures::SingleThreadedFuturesFramed::new(ws);

    let mut connector = connector::ClientConnector::new(config);

    let (upgraded, server_public_key) =
        connect_rdcleanpath(&mut framed, &mut connector, destination.clone(), proxy_auth_token, pcb).await?;

    info!("kdc url = {:?}", &kdc_proxy_url);

    let mut network_client = WasmNetworkClient::new();

    let connection_result = ironrdp_futures::connect_finalize(
        upgraded,
        &mut framed,
        (&destination).into(),
        server_public_key,
        Some(&mut network_client),
        connector,
        Some(KerberosConfig {
            kdc_proxy_url: kdc_proxy_url
                .map(|url| url::Url::parse(&url))
                .transpose()
                .context("invalid KDC URL")?,
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
) -> Result<(ironrdp_futures::Upgraded, Vec<u8>), IronRdpError>
where
    S: ironrdp_futures::FramedRead + ironrdp_futures::FramedWrite,
{
    use ironrdp::connector::Sequence as _;
    use x509_cert::der::Decode as _;

    #[derive(Clone, Copy, Debug)]
    struct RDCleanPathHint;

    const RDCLEANPATH_HINT: RDCleanPathHint = RDCleanPathHint;

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

        let upgraded = ironrdp_futures::mark_as_upgraded(should_upgrade, connector);

        Ok((upgraded, server_public_key))
    }
}
