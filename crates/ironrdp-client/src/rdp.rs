use core::num::NonZeroU16;
use std::sync::Arc;

use ironrdp::cliprdr::backend::{ClipboardMessage, CliprdrBackendFactory};
use ironrdp::connector::connection_activation::ConnectionActivationState;
use ironrdp::connector::{ClientConnector, ConnectionResult, ConnectorCore, ConnectorResult};
use ironrdp::displaycontrol::client::DisplayControlClient;
use ironrdp::displaycontrol::pdu::MonitorLayoutEntry;
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::graphics::pointer::DecodedPointer;
use ironrdp::pdu::input::fast_path::FastPathInputEvent;
use ironrdp::pdu::{pdu_other_err, PduResult};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{fast_path, ActiveStage, ActiveStageOutput, GracefulDisconnectReason, SessionResult};
use ironrdp::svc::SvcMessage;
use ironrdp::{cliprdr, connector, rdpdr, rdpsnd, session};
use ironrdp_core::WriteBuf;
use ironrdp_dvc_pipe_proxy::DvcNamedPipeProxy;
use ironrdp_rdpsnd_native::cpal;
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{
    mark_pcb_sent_by_rdclean_path, perform_credssp, run_until_handover, send_pcb, single_sequence_step_read,
    split_tokio_framed, vm_connector_take_over, CredSSPFinished, Framed, FramedRead, FramedWrite, TokioStream,
    Upgraded,
};
use ironrdp_vmconnect::VmClientConnector;
use rdpdr::NoopRdpdrBackend;
use smallvec::SmallVec;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::{debug, error, trace, warn};
use winit::event_loop::EventLoopProxy;
use x509_cert::der::asn1::OctetString;

use crate::config::{Config, PreconnectionBlobPayload, RDCleanPathConfig};

#[derive(Debug)]
pub enum RdpOutputEvent {
    Image {
        buffer: Vec<u32>,
        width: NonZeroU16,
        height: NonZeroU16,
    },
    ConnectionFailure(connector::ConnectorError),
    PointerDefault,
    PointerHidden,
    PointerPosition {
        x: u16,
        y: u16,
    },
    PointerBitmap(Arc<DecodedPointer>),
    Terminated(SessionResult<GracefulDisconnectReason>),
}

#[derive(Debug)]
pub enum RdpInputEvent {
    Resize {
        width: u16,
        height: u16,
        scale_factor: u32,
        /// The physical size of the display in millimeters (width, height).
        physical_size: Option<(u32, u32)>,
    },
    FastPath(SmallVec<[FastPathInputEvent; 2]>),
    Close,
    Clipboard(ClipboardMessage),
    SendDvcMessages {
        channel_id: u32,
        messages: Vec<SvcMessage>,
    },
}

impl RdpInputEvent {
    pub fn create_channel() -> (mpsc::UnboundedSender<Self>, mpsc::UnboundedReceiver<Self>) {
        mpsc::unbounded_channel()
    }
}

pub struct DvcPipeProxyFactory {
    rdp_input_sender: mpsc::UnboundedSender<RdpInputEvent>,
}

impl DvcPipeProxyFactory {
    pub fn new(rdp_input_sender: mpsc::UnboundedSender<RdpInputEvent>) -> Self {
        Self { rdp_input_sender }
    }

    pub fn create(&self, channel_name: String, pipe_name: String) -> DvcNamedPipeProxy {
        let rdp_input_sender = self.rdp_input_sender.clone();

        DvcNamedPipeProxy::new(&channel_name, &pipe_name, move |channel_id, messages| {
            rdp_input_sender
                .send(RdpInputEvent::SendDvcMessages { channel_id, messages })
                .map_err(|_error| pdu_other_err!("send DVC messages to the event loop",))?;

            Ok(())
        })
    }
}

pub type WriteDvcMessageFn = Box<dyn Fn(u32, SvcMessage) -> PduResult<()> + Send + 'static>;

pub struct RdpClient {
    pub config: Config,
    pub event_loop_proxy: EventLoopProxy<RdpOutputEvent>,
    pub input_event_receiver: mpsc::UnboundedReceiver<RdpInputEvent>,
    pub cliprdr_factory: Option<Box<dyn CliprdrBackendFactory + Send>>,
    pub dvc_pipe_proxy_factory: DvcPipeProxyFactory,
}

impl RdpClient {
    pub async fn run(mut self) {
        loop {
            let (connection_result, framed) = if let Some(rdcleanpath) = self.config.rdcleanpath.as_ref() {
                match connect_ws(
                    &self.config,
                    rdcleanpath,
                    self.cliprdr_factory.as_deref(),
                    &self.dvc_pipe_proxy_factory,
                )
                .await
                {
                    Ok(result) => result,
                    Err(e) => {
                        let _ = self.event_loop_proxy.send_event(RdpOutputEvent::ConnectionFailure(e));
                        break;
                    }
                }
            } else {
                match connect(
                    &self.config,
                    self.cliprdr_factory.as_deref(),
                    &self.dvc_pipe_proxy_factory,
                )
                .await
                {
                    Ok(result) => result,
                    Err(e) => {
                        let _ = self.event_loop_proxy.send_event(RdpOutputEvent::ConnectionFailure(e));
                        break;
                    }
                }
            };

            match active_session(
                framed,
                connection_result,
                &self.event_loop_proxy,
                &mut self.input_event_receiver,
            )
            .await
            {
                Ok(RdpControlFlow::ReconnectWithNewSize { width, height }) => {
                    self.config.connector.desktop_size.width = width;
                    self.config.connector.desktop_size.height = height;
                }
                Ok(RdpControlFlow::TerminatedGracefully(reason)) => {
                    let _ = self.event_loop_proxy.send_event(RdpOutputEvent::Terminated(Ok(reason)));
                    break;
                }
                Err(e) => {
                    let _ = self.event_loop_proxy.send_event(RdpOutputEvent::Terminated(Err(e)));
                    break;
                }
            }
        }
    }
}

enum RdpControlFlow {
    ReconnectWithNewSize { width: u16, height: u16 },
    TerminatedGracefully(GracefulDisconnectReason),
}

trait AsyncReadWrite: AsyncRead + AsyncWrite {}

impl<T> AsyncReadWrite for T where T: AsyncRead + AsyncWrite {}

type UpgradedFramed = ironrdp_tokio::TokioFramed<Box<dyn AsyncReadWrite + Unpin + Send + Sync>>;

async fn connect(
    config: &Config,
    cliprdr_factory: Option<&(dyn CliprdrBackendFactory + Send)>,
    dvc_pipe_proxy_factory: &DvcPipeProxyFactory,
) -> ConnectorResult<(ConnectionResult, UpgradedFramed)> {
    let dest = format!("{}:{}", config.destination.name(), config.destination.port());

    let (client_addr, stream) = if let Some(ref gw_config) = config.gw {
        let (gw, client_addr) = ironrdp_mstsgu::GwClient::connect(gw_config, &config.connector.client_name)
            .await
            .map_err(|e| connector::custom_err!("GW Connect", e))?;
        (client_addr, tokio_util::either::Either::Left(gw))
    } else {
        let stream = TcpStream::connect(dest)
            .await
            .map_err(|e| connector::custom_err!("TCP connect", e))?;
        let client_addr = stream
            .local_addr()
            .map_err(|e| connector::custom_err!("get socket local address", e))?;
        (client_addr, tokio_util::either::Either::Right(stream))
    };
    let mut framed = ironrdp_tokio::TokioFramed::new(stream);

    let mut drdynvc =
        ironrdp::dvc::DrdynvcClient::new().with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new())));

    // Instantiate all DVC proxies
    for proxy in config.dvc_pipe_proxies.iter() {
        let channel_name = proxy.channel_name.clone();
        let pipe_name = proxy.pipe_name.clone();

        trace!(%channel_name, %pipe_name, "Creating DVC proxy");

        drdynvc = drdynvc.with_dynamic_channel(dvc_pipe_proxy_factory.create(channel_name, pipe_name));
    }

    let mut connector = ClientConnector::new(config.connector.clone(), client_addr)
        .with_static_channel(drdynvc)
        .with_static_channel(rdpsnd::client::Rdpsnd::new(Box::new(cpal::RdpsndBackend::new())))
        .with_static_channel(rdpdr::Rdpdr::new(Box::new(NoopRdpdrBackend {}), "IronRDP".to_owned()).with_smartcard(0));

    if let Some(builder) = cliprdr_factory {
        let backend = builder.build_cliprdr_backend();
        let cliprdr = cliprdr::Cliprdr::new(backend);
        connector.attach_static_channel(cliprdr);
    }

    let mut connector: Box<dyn ConnectorCore> =
        if let Some(PreconnectionBlobPayload::VmConnect(vmconnect)) = &config.pcb {
            let pcb_sent = send_pcb(&mut framed, vmconnect.to_owned()).await?;
            let connector = vm_connector_take_over(pcb_sent, connector)?;
            Box::new(connector)
        } else {
            Box::new(connector)
        };

    let should_upgrade = ironrdp_tokio::connect_begin(&mut framed, connector.as_mut()).await?;
    let (mut upgraded_framed, server_public_key) = upgrade(framed, config.destination.name()).await?;
    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, connector.as_mut());

    let server_name = (&config.destination).into();

    let mut credssp_finished = perform_credssp(
        upgraded,
        connector.as_mut(),
        &mut upgraded_framed,
        server_name,
        server_public_key,
        Some(&mut ReqwestNetworkClient::new()),
        None,
    )
    .await?;

    let connector = downcast_back_to_client_connector(connector, &mut credssp_finished, &mut upgraded_framed).await?;
    let connection_result = ironrdp_tokio::connect_finalize(credssp_finished, &mut upgraded_framed, connector).await?;
    debug!(?connection_result);

    return Ok((connection_result, upgraded_framed));

    async fn upgrade<S>(
        framed: Framed<TokioStream<S>>,
        server_name: &str,
    ) -> ConnectorResult<(
        Framed<TokioStream<Box<dyn AsyncReadWrite + Sync + Send + Unpin + 'static>>>,
        Vec<u8>,
    )>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    {
        let (initial_stream, leftover_bytes) = framed.into_inner();

        let (upgraded_stream, tls_cert) = ironrdp_tls::upgrade(initial_stream, server_name)
            .await
            .map_err(|e| connector::custom_err!("TLS upgrade", e))?;

        let server_public_key = ironrdp_tls::extract_tls_server_public_key(&tls_cert)
            .ok_or_else(|| connector::general_err!("unable to extract tls server public key"))?
            .to_owned();

        let erased_stream: Box<dyn AsyncReadWrite + Unpin + Send + Sync> = Box::new(upgraded_stream);
        let upgraded_framed = ironrdp_tokio::TokioFramed::new_with_leftover(erased_stream, leftover_bytes);

        Ok((upgraded_framed, server_public_key))
    }
}

async fn connect_ws(
    config: &Config,
    rdcleanpath: &RDCleanPathConfig,
    cliprdr_factory: Option<&(dyn CliprdrBackendFactory + Send)>,
    dvc_pipe_proxy_factory: &DvcPipeProxyFactory,
) -> ConnectorResult<(ConnectionResult, UpgradedFramed)> {
    let hostname = rdcleanpath
        .url
        .host_str()
        .ok_or_else(|| connector::general_err!("host missing from the URL"))?;

    let port = rdcleanpath.url.port_or_known_default().unwrap_or(443);

    let socket = TcpStream::connect((hostname, port))
        .await
        .map_err(|e| connector::custom_err!("TCP connect", e))?;

    socket
        .set_nodelay(true)
        .map_err(|e| connector::custom_err!("set TCP_NODELAY", e))?;

    let client_addr = socket
        .local_addr()
        .map_err(|e| connector::custom_err!("get socket local address", e))?;

    let (ws, _) = tokio_tungstenite::client_async_tls(rdcleanpath.url.as_str(), socket)
        .await
        .map_err(|e| connector::custom_err!("WS connect", e))?;

    let ws = crate::ws::websocket_compat(ws);

    let mut framed = ironrdp_tokio::TokioFramed::new(ws);

    let mut drdynvc =
        ironrdp::dvc::DrdynvcClient::new().with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new())));

    // Instantiate all DVC proxies
    for proxy in config.dvc_pipe_proxies.iter() {
        let channel_name = proxy.channel_name.clone();
        let pipe_name = proxy.pipe_name.clone();

        trace!(%channel_name, %pipe_name, "Creating DVC proxy");

        drdynvc = drdynvc.with_dynamic_channel(dvc_pipe_proxy_factory.create(channel_name, pipe_name));
    }

    let mut connector = ClientConnector::new(config.connector.clone(), client_addr)
        .with_static_channel(drdynvc)
        .with_static_channel(rdpsnd::client::Rdpsnd::new(Box::new(cpal::RdpsndBackend::new())))
        .with_static_channel(rdpdr::Rdpdr::new(Box::new(NoopRdpdrBackend {}), "IronRDP".to_owned()).with_smartcard(0));

    if let Some(builder) = cliprdr_factory {
        let backend = builder.build_cliprdr_backend();

        let cliprdr = cliprdr::Cliprdr::new(backend);

        connector.attach_static_channel(cliprdr);
    }

    let destination = format!("{}:{}", config.destination.name(), config.destination.port());

    let (upgraded, server_public_key, mut connector) = connect_rdcleanpath(
        &mut framed,
        connector,
        destination,
        rdcleanpath.auth_token.clone(),
        &config.pcb,
    )
    .await?;

    let (ws, leftover_bytes) = framed.into_inner();
    let erased_stream: Box<dyn AsyncReadWrite + Unpin + Send + Sync> = Box::new(ws);
    let mut upgraded_framed = ironrdp_tokio::TokioFramed::new_with_leftover(erased_stream, leftover_bytes);

    let server_name = (&config.destination).into();

    let mut credssp_done = perform_credssp(
        upgraded,
        connector.as_mut(),
        &mut upgraded_framed,
        server_name,
        server_public_key,
        Some(&mut ReqwestNetworkClient::new()),
        None,
    )
    .await?;

    let connector = downcast_back_to_client_connector(connector, &mut credssp_done, &mut upgraded_framed).await?;

    let connection_result = ironrdp_tokio::connect_finalize(credssp_done, &mut upgraded_framed, connector).await?;

    Ok((connection_result, upgraded_framed))
}

async fn connect_rdcleanpath<S>(
    framed: &mut Framed<S>,
    mut connector: ClientConnector,
    destination: String,
    proxy_auth_token: String,
    pcb: &Option<PreconnectionBlobPayload>,
) -> ConnectorResult<(Upgraded, Vec<u8>, Box<dyn ConnectorCore>)>
where
    S: FramedRead + FramedWrite,
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

    debug!(?pcb, "Begin connection procedure");

    // let connector::ClientConnectorState::ConnectionInitiationSendRequest = connector.state() else {
    //     return Err(connector::general_err!("invalid connector state (send request)"));
    // };

    debug_assert!(connector.next_pdu_hint().is_none());
    let (rdcleanpath_request, mut connector): (ironrdp_rdcleanpath::RDCleanPathPdu, Box<dyn ConnectorCore>) =
        if let Some(PreconnectionBlobPayload::VmConnect(vm_id)) = pcb {
            let rdcleanpath_req = ironrdp_rdcleanpath::RDCleanPathPdu::new_request(
                None,
                destination,
                proxy_auth_token,
                Some(vm_id.to_owned()),
            )
            .map_err(|e| connector::custom_err!("new RDCleanPath request", e))?;

            debug!(message = ?rdcleanpath_req, "Send RDCleanPath request for VMConnect");

            let pcb_sent = mark_pcb_sent_by_rdclean_path();
            let connector = vm_connector_take_over(pcb_sent, connector)?;
            let connector: Box<dyn ConnectorCore> = Box::new(connector);
            (rdcleanpath_req, connector)
        } else {
            let written = connector.step_no_input(&mut buf)?;
            let x224_pdu_len = written.size().expect("written size");
            debug_assert_eq!(x224_pdu_len, buf.filled_len());
            let x224_pdu = buf.filled().to_vec();
            let general_pcb = pcb.as_ref().and_then(|pcb| pcb.general());
            // RDCleanPath request

            let rdcleanpath_req = ironrdp_rdcleanpath::RDCleanPathPdu::new_request(
                Some(x224_pdu),
                destination,
                proxy_auth_token,
                general_pcb.map(str::to_string),
            )
            .map_err(|e| connector::custom_err!("new RDCleanPath request", e))?;
            let connector: Box<dyn ConnectorCore> = Box::new(connector);
            (rdcleanpath_req, connector)
        };

    let rdcleanpath_request = rdcleanpath_request
        .to_der()
        .map_err(|e| connector::custom_err!("RDCleanPath request encode", e))?;

    framed
        .write_all(&rdcleanpath_request)
        .await
        .map_err(|e| connector::custom_err!("couldn't write RDCleanPath request", e))?;

    let rdcleanpath_result = framed
        .read_by_hint(&RDCLEANPATH_HINT)
        .await
        .map_err(|e| connector::custom_err!("read RDCleanPath request", e))?;

    let rdcleanpath_result = ironrdp_rdcleanpath::RDCleanPathPdu::from_der(&rdcleanpath_result)
        .map_err(|e| connector::custom_err!("RDCleanPath response decode", e))?;

    debug!(message = ?rdcleanpath_result, "Received RDCleanPath PDU");

    let (x224_connection_response, server_cert_chain) = match rdcleanpath_result
        .into_enum()
        .map_err(|e| connector::custom_err!("invalid RDCleanPath PDU", e))?
    {
        ironrdp_rdcleanpath::RDCleanPath::Request { .. } => {
            return Err(connector::general_err!(
                "received an unexpected RDCleanPath type (request)",
            ));
        }
        ironrdp_rdcleanpath::RDCleanPath::Response {
            x224_connection_response,
            server_cert_chain,
            server_addr: _,
        } => (x224_connection_response, server_cert_chain),
        ironrdp_rdcleanpath::RDCleanPath::GeneralErr(error) => {
            return Err(connector::custom_err!("received an RDCleanPath error", error));
        }
        ironrdp_rdcleanpath::RDCleanPath::NegotiationErr {
            x224_connection_response,
        } => {
            if let Ok(x224_confirm) = ironrdp_core::decode::<
                ironrdp::pdu::x224::X224<ironrdp::pdu::nego::ConnectionConfirm>,
            >(&x224_connection_response)
            {
                if let ironrdp::pdu::nego::ConnectionConfirm::Failure { code } = x224_confirm.0 {
                    let negotiation_failure = connector::NegotiationFailure::from(code);
                    return Err(connector::ConnectorError::new(
                        "RDP negotiation failed",
                        connector::ConnectorErrorKind::Negotiation(negotiation_failure),
                    ));
                }
            }

            return Err(connector::general_err!("received an RDCleanPath negotiation error"));
        }
    };

    buf.clear();
    if let Some(x224_connection_response) = x224_connection_response {
        debug_assert!(connector.next_pdu_hint().is_some());
        // Write the X.224 connection response PDU
        let written = connector.step(x224_connection_response.as_bytes(), &mut buf)?;
        debug_assert!(written.is_nothing());
    }

    let server_public_key = extract_server_public_key(server_cert_chain)?;

    let should_upgrade = ironrdp_tokio::skip_connect_begin(connector.as_mut());

    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, connector.as_mut());

    return Ok((upgraded, server_public_key, connector));

    fn extract_server_public_key(server_cert_chain: Vec<OctetString>) -> ConnectorResult<Vec<u8>> {
        let server_cert = server_cert_chain
            .into_iter()
            .next()
            .ok_or_else(|| connector::general_err!("server cert chain missing from rdcleanpath response"))?;

        let cert = x509_cert::Certificate::from_der(server_cert.as_bytes())
            .map_err(|e| connector::custom_err!("server cert chain missing from rdcleanpath response", e))?;

        let server_public_key = cert
            .tbs_certificate
            .subject_public_key_info
            .subject_public_key
            .as_bytes()
            .ok_or_else(|| connector::general_err!("subject public key BIT STRING is not aligned"))?
            .to_owned();

        Ok(server_public_key)
    }
}

async fn active_session(
    framed: UpgradedFramed,
    connection_result: ConnectionResult,
    event_loop_proxy: &EventLoopProxy<RdpOutputEvent>,
    input_event_receiver: &mut mpsc::UnboundedReceiver<RdpInputEvent>,
) -> SessionResult<RdpControlFlow> {
    let (mut reader, mut writer) = split_tokio_framed(framed);
    let mut image = DecodedImage::new(
        PixelFormat::RgbA32,
        connection_result.desktop_size.width,
        connection_result.desktop_size.height,
    );

    let mut active_stage = ActiveStage::new(connection_result);

    let disconnect_reason = 'outer: loop {
        let outputs = tokio::select! {
            frame = reader.read_pdu() => {
                let (action, payload) = frame.map_err(|e| session::custom_err!("read frame", e))?;
                trace!(?action, frame_length = payload.len(), "Frame received");

                active_stage.process(&mut image, action, &payload)?
            }
            input_event = input_event_receiver.recv() => {
                let input_event = input_event.ok_or_else(|| session::general_err!("GUI is stopped"))?;

                match input_event {
                    RdpInputEvent::Resize { width, height, scale_factor, physical_size } => {
                        trace!(width, height, "Resize event");
                        let width = u32::from(width);
                        let height = u32::from(height);
                        // TODO: Make adjust_display_size take and return width and height as u16.
                        // From the function's doc comment, the width and height values must be less than or equal to 8192 pixels.
                        // Therefore, we can remove unnecessary casts from u16 to u32 and back.
                        let (width, height) = MonitorLayoutEntry::adjust_display_size(width, height);
                        debug!(width, height, "Adjusted display size");
                        if let Some(response_frame) = active_stage.encode_resize(width, height, Some(scale_factor), physical_size) {
                            vec![ActiveStageOutput::ResponseFrame(response_frame?)]
                        } else {
                            // TODO(#271): use the "auto-reconnect cookie": https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/15b0d1c9-2891-4adb-a45e-deb4aeeeab7c
                            debug!("Reconnecting with new size");
                            let width = u16::try_from(width).expect("always in the range");
                            let height = u16::try_from(height).expect("always in the range");
                            return Ok(RdpControlFlow::ReconnectWithNewSize { width, height })
                        }
                    },
                    RdpInputEvent::FastPath(events) => {
                        trace!(?events);
                        active_stage.process_fastpath_input(&mut image, &events)?
                    }
                    RdpInputEvent::Close => {
                        active_stage.graceful_shutdown()?
                    }
                    RdpInputEvent::Clipboard(event) => {
                        if let Some(cliprdr) = active_stage.get_svc_processor::<cliprdr::CliprdrClient>() {
                            if let Some(svc_messages) = match event {
                                ClipboardMessage::SendInitiateCopy(formats) => {
                                    Some(cliprdr.initiate_copy(&formats)
                                        .map_err(|e| session::custom_err!("CLIPRDR", e))?)
                                }
                                ClipboardMessage::SendFormatData(response) => {
                                    Some(cliprdr.submit_format_data(response)
                                    .map_err(|e| session::custom_err!("CLIPRDR", e))?)
                                }
                                ClipboardMessage::SendInitiatePaste(format) => {
                                    Some(cliprdr.initiate_paste(format)
                                        .map_err(|e| session::custom_err!("CLIPRDR", e))?)
                                }
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
                    RdpInputEvent::SendDvcMessages { channel_id, messages } => {
                        trace!(channel_id, ?messages, "Send DVC messages");

                        let frame = active_stage.encode_dvc_messages(messages)?;
                        vec![ActiveStageOutput::ResponseFrame(frame)]
                    }
                }
            }
        };

        for out in outputs {
            match out {
                ActiveStageOutput::ResponseFrame(frame) => writer
                    .write_all(&frame)
                    .await
                    .map_err(|e| session::custom_err!("write response", e))?,
                ActiveStageOutput::GraphicsUpdate(_region) => {
                    let buffer: Vec<u32> = image
                        .data()
                        .chunks_exact(4)
                        .map(|pixel| {
                            let r = pixel[0];
                            let g = pixel[1];
                            let b = pixel[2];
                            u32::from_be_bytes([0, r, g, b])
                        })
                        .collect();

                    event_loop_proxy
                        .send_event(RdpOutputEvent::Image {
                            buffer,
                            width: NonZeroU16::new(image.width())
                                .ok_or_else(|| session::general_err!("width is zero"))?,
                            height: NonZeroU16::new(image.height())
                                .ok_or_else(|| session::general_err!("height is zero"))?,
                        })
                        .map_err(|e| session::custom_err!("event_loop_proxy", e))?;
                }
                ActiveStageOutput::PointerDefault => {
                    event_loop_proxy
                        .send_event(RdpOutputEvent::PointerDefault)
                        .map_err(|e| session::custom_err!("event_loop_proxy", e))?;
                }
                ActiveStageOutput::PointerHidden => {
                    event_loop_proxy
                        .send_event(RdpOutputEvent::PointerHidden)
                        .map_err(|e| session::custom_err!("event_loop_proxy", e))?;
                }
                ActiveStageOutput::PointerPosition { x, y } => {
                    event_loop_proxy
                        .send_event(RdpOutputEvent::PointerPosition { x, y })
                        .map_err(|e| session::custom_err!("event_loop_proxy", e))?;
                }
                ActiveStageOutput::PointerBitmap(pointer) => {
                    event_loop_proxy
                        .send_event(RdpOutputEvent::PointerBitmap(pointer))
                        .map_err(|e| session::custom_err!("event_loop_proxy", e))?;
                }
                ActiveStageOutput::DeactivateAll(mut connection_activation) => {
                    // Execute the Deactivation-Reactivation Sequence:
                    // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dfc234ce-481a-4674-9a5d-2a7bafb14432
                    debug!("Received Server Deactivate All PDU, executing Deactivation-Reactivation Sequence");
                    let mut buf = WriteBuf::new();
                    'activation_seq: loop {
                        let written = single_sequence_step_read(&mut reader, &mut *connection_activation, &mut buf)
                            .await
                            .map_err(|e| session::custom_err!("read deactivation-reactivation sequence step", e))?;

                        if written.size().is_some() {
                            writer.write_all(buf.filled()).await.map_err(|e| {
                                session::custom_err!("write deactivation-reactivation sequence step", e)
                            })?;
                        }

                        if let ConnectionActivationState::Finalized {
                            io_channel_id,
                            user_channel_id,
                            desktop_size,
                            enable_server_pointer,
                            pointer_software_rendering,
                        } = connection_activation.connection_activation_state()
                        {
                            debug!(?desktop_size, "Deactivation-Reactivation Sequence completed");
                            // Update image size with the new desktop size.
                            image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
                            // Update the active stage with the new channel IDs and pointer settings.
                            active_stage.set_fastpath_processor(
                                fast_path::ProcessorBuilder {
                                    io_channel_id,
                                    user_channel_id,
                                    enable_server_pointer,
                                    pointer_software_rendering,
                                }
                                .build(),
                            );
                            active_stage.set_enable_server_pointer(enable_server_pointer);
                            break 'activation_seq;
                        }
                    }
                }
                ActiveStageOutput::Terminate(reason) => break 'outer reason,
            }
        }
    };

    Ok(RdpControlFlow::TerminatedGracefully(disconnect_reason))
}

pub async fn downcast_back_to_client_connector(
    connector: Box<dyn ConnectorCore>, // `ConnectorCore: Any`
    credssp_finished: &mut CredSSPFinished,
    framed: &mut Framed<impl FramedRead + FramedWrite>,
) -> ConnectorResult<ClientConnector> {
    let connector: Box<dyn core::any::Any> = connector;

    let client = match connector.downcast::<VmClientConnector>() {
        Ok(vm_connector) => run_until_handover(credssp_finished, framed, *vm_connector).await?,
        Err(err) => match err.downcast::<ClientConnector>() {
            Ok(c) => *c,
            Err(_) => {
                return Err(connector::general_err!(
                    "connector is neither ClientConnector nor VmClientConnector"
                ))
            }
        },
    };

    Ok(client)
}
