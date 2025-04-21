use ironrdp::cliprdr::backend::{ClipboardMessage, CliprdrBackendFactory};
use ironrdp::connector::connection_activation::ConnectionActivationState;
use ironrdp::connector::{ConnectionResult, ConnectorResult};
use ironrdp::displaycontrol::client::DisplayControlClient;
use ironrdp::displaycontrol::pdu::MonitorLayoutEntry;
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::pdu::input::fast_path::FastPathInputEvent;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{fast_path, ActiveStage, ActiveStageOutput, GracefulDisconnectReason, SessionResult};
use ironrdp::{cliprdr, connector, rdpdr, rdpsnd, session};
use ironrdp_core::WriteBuf;
use ironrdp_rdpsnd_native::cpal;
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{single_sequence_step_read, split_tokio_framed, FramedWrite};
use rdpdr::NoopRdpdrBackend;
use smallvec::SmallVec;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use winit::event_loop::EventLoopProxy;

use crate::config::{Config, RDCleanPathConfig};

#[derive(Debug)]
pub enum RdpOutputEvent {
    Image { buffer: Vec<u32>, width: u16, height: u16 },
    ConnectionFailure(connector::ConnectorError),
    PointerDefault,
    PointerHidden,
    PointerPosition { x: u16, y: u16 },
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
}

impl RdpInputEvent {
    pub fn create_channel() -> (mpsc::UnboundedSender<Self>, mpsc::UnboundedReceiver<Self>) {
        mpsc::unbounded_channel()
    }
}

pub struct RdpClient {
    pub config: Config,
    pub event_loop_proxy: EventLoopProxy<RdpOutputEvent>,
    pub input_event_receiver: mpsc::UnboundedReceiver<RdpInputEvent>,
    pub cliprdr_factory: Option<Box<dyn CliprdrBackendFactory + Send>>,
}

impl RdpClient {
    pub async fn run(mut self) {
        loop {
            let (connection_result, framed) = if let Some(rdcleanpath) = self.config.rdcleanpath.as_ref() {
                match connect_ws(&self.config, rdcleanpath, self.cliprdr_factory.as_deref()).await {
                    Ok(result) => result,
                    Err(e) => {
                        let _ = self.event_loop_proxy.send_event(RdpOutputEvent::ConnectionFailure(e));
                        break;
                    }
                }
            } else {
                match connect(&self.config, self.cliprdr_factory.as_deref()).await {
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
) -> ConnectorResult<(ConnectionResult, UpgradedFramed)> {
    let dest = format!("{}:{}", config.destination.name(), config.destination.port());

    let stream = TcpStream::connect(dest)
        .await
        .map_err(|e| connector::custom_err!("TCP connect", e))?;

    let server_addr = stream
        .peer_addr()
        .map_err(|e| connector::custom_err!("Peer address", e))?;

    let mut framed = ironrdp_tokio::TokioFramed::new(stream);

    let mut connector = connector::ClientConnector::new(config.connector.clone())
        .with_client_addr(server_addr)
        .with_static_channel(
            ironrdp::dvc::DrdynvcClient::new().with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new()))),
        )
        .with_static_channel(rdpsnd::client::Rdpsnd::new(Box::new(cpal::RdpsndBackend::new())))
        .with_static_channel(rdpdr::Rdpdr::new(Box::new(NoopRdpdrBackend {}), "IronRDP".to_owned()).with_smartcard(0));

    if let Some(builder) = cliprdr_factory {
        let backend = builder.build_cliprdr_backend();

        let cliprdr = cliprdr::Cliprdr::new(backend);

        connector.attach_static_channel(cliprdr);
    }

    let should_upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector).await?;

    debug!("TLS upgrade");

    // Ensure there is no leftover
    let (initial_stream, leftover_bytes) = framed.into_inner();

    let (upgraded_stream, server_public_key) = ironrdp_tls::upgrade(initial_stream, config.destination.name())
        .await
        .map_err(|e| connector::custom_err!("TLS upgrade", e))?;

    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);

    let erased_stream = Box::new(upgraded_stream) as Box<dyn AsyncReadWrite + Unpin + Send + Sync>;
    let mut upgraded_framed = ironrdp_tokio::TokioFramed::new_with_leftover(erased_stream, leftover_bytes);

    let connection_result = ironrdp_tokio::connect_finalize(
        upgraded,
        &mut upgraded_framed,
        connector,
        (&config.destination).into(),
        server_public_key,
        Some(&mut ReqwestNetworkClient::new()),
        None,
    )
    .await?;

    debug!(?connection_result);

    Ok((connection_result, upgraded_framed))
}

async fn connect_ws(
    config: &Config,
    rdcleanpath: &RDCleanPathConfig,
    cliprdr_factory: Option<&(dyn CliprdrBackendFactory + Send)>,
) -> ConnectorResult<(ConnectionResult, UpgradedFramed)> {
    let (ws, _) = tokio_tungstenite::connect_async(&rdcleanpath.url)
        .await
        .map_err(|e| connector::custom_err!("WS connect", e))?;

    let ws = crate::ws::websocket_compat(ws);

    let mut framed = ironrdp_tokio::TokioFramed::new(ws);

    let mut connector = connector::ClientConnector::new(config.connector.clone())
        .with_static_channel(
            ironrdp::dvc::DrdynvcClient::new().with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new()))),
        )
        .with_static_channel(rdpsnd::client::Rdpsnd::new(Box::new(cpal::RdpsndBackend::new())))
        .with_static_channel(rdpdr::Rdpdr::new(Box::new(NoopRdpdrBackend {}), "IronRDP".to_owned()).with_smartcard(0));

    if let Some(builder) = cliprdr_factory {
        let backend = builder.build_cliprdr_backend();

        let cliprdr = cliprdr::Cliprdr::new(backend);

        connector.attach_static_channel(cliprdr);
    }

    let destination = format!("{}:{}", config.destination.name(), config.destination.port());

    let (upgraded, server_public_key) = connect_rdcleanpath(
        &mut framed,
        &mut connector,
        destination,
        rdcleanpath.auth_token.clone(),
        None,
    )
    .await?;

    let connection_result = ironrdp_tokio::connect_finalize(
        upgraded,
        &mut framed,
        connector,
        (&config.destination).into(),
        server_public_key,
        Some(&mut ReqwestNetworkClient::new()),
        None,
    )
    .await?;

    let (ws, leftover_bytes) = framed.into_inner();
    let erased_stream = Box::new(ws) as Box<dyn AsyncReadWrite + Unpin + Send + Sync>;
    let upgraded_framed = ironrdp_tokio::TokioFramed::new_with_leftover(erased_stream, leftover_bytes);

    Ok((connection_result, upgraded_framed))
}

async fn connect_rdcleanpath<S>(
    framed: &mut ironrdp_tokio::Framed<S>,
    connector: &mut connector::ClientConnector,
    destination: String,
    proxy_auth_token: String,
    pcb: Option<String>,
) -> ConnectorResult<(ironrdp_tokio::Upgraded, Vec<u8>)>
where
    S: ironrdp_tokio::FramedRead + FramedWrite,
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
            return Err(connector::general_err!("invalid connector state (send request)"));
        };

        debug_assert!(connector.next_pdu_hint().is_none());

        let written = connector.step_no_input(&mut buf)?;
        let x224_pdu_len = written.size().expect("written size");
        debug_assert_eq!(x224_pdu_len, buf.filled_len());
        let x224_pdu = buf.filled().to_vec();

        let rdcleanpath_req =
            ironrdp_rdcleanpath::RDCleanPathPdu::new_request(x224_pdu, destination, proxy_auth_token, pcb)
                .map_err(|e| connector::custom_err!("new RDCleanPath request", e))?;
        debug!(message = ?rdcleanpath_req, "Send RDCleanPath request");
        let rdcleanpath_req = rdcleanpath_req
            .to_der()
            .map_err(|e| connector::custom_err!("RDCleanPath request encode", e))?;

        framed
            .write_all(&rdcleanpath_req)
            .await
            .map_err(|e| connector::custom_err!("couldn’t write RDCleanPath request", e))?;
    }

    {
        // RDCleanPath response

        let rdcleanpath_res = framed
            .read_by_hint(Box::new(&RDCLEANPATH_HINT))
            .await
            .map_err(|e| connector::custom_err!("read RDCleanPath request", e))?;

        let rdcleanpath_res = ironrdp_rdcleanpath::RDCleanPathPdu::from_der(&rdcleanpath_res)
            .map_err(|e| connector::custom_err!("RDCleanPath response decode", e))?;

        debug!(message = ?rdcleanpath_res, "Received RDCleanPath PDU");

        let (x224_connection_response, server_cert_chain, server_addr) = match rdcleanpath_res
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
                server_addr,
            } => (x224_connection_response, server_cert_chain, server_addr),
            ironrdp_rdcleanpath::RDCleanPath::Err(error) => {
                return Err(connector::custom_err!("received an RDCleanPath error", error));
            }
        };

        let server_addr = server_addr
            .parse()
            .map_err(|e| connector::custom_err!("failed to parse server address sent by proxy", e))?;

        connector.attach_client_addr(server_addr);

        let connector::ClientConnectorState::ConnectionInitiationWaitConfirm { .. } = connector.state else {
            return Err(connector::general_err!("invalid connector state (wait confirm)"));
        };

        debug_assert!(connector.next_pdu_hint().is_some());

        buf.clear();
        let written = connector.step(x224_connection_response.as_bytes(), &mut buf)?;

        debug_assert!(written.is_nothing());

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

        let should_upgrade = ironrdp_tokio::skip_connect_begin(connector);

        // At this point, proxy established the TLS session.

        let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, connector);

        Ok((upgraded, server_public_key))
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
                        let (width, height) = MonitorLayoutEntry::adjust_display_size(width.into(), height.into());
                        debug!(width, height, "Adjusted display size");
                        if let Some(response_frame) = active_stage.encode_resize(width, height, Some(scale_factor), physical_size) {
                            vec![ActiveStageOutput::ResponseFrame(response_frame?)]
                        } else {
                            // TODO(#271): use the "auto-reconnect cookie": https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/15b0d1c9-2891-4adb-a45e-deb4aeeeab7c
                            debug!("Reconnecting with new size");
                            return Ok(RdpControlFlow::ReconnectWithNewSize { width: width.try_into().unwrap(), height: height.try_into().unwrap() })
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
                            width: image.width(),
                            height: image.height(),
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
                ActiveStageOutput::PointerBitmap(_) => {
                    // Not applicable, because we use the software cursor rendering.
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
                            no_server_pointer,
                            pointer_software_rendering,
                        } = connection_activation.state
                        {
                            debug!(?desktop_size, "Deactivation-Reactivation Sequence completed");
                            // Update image size with the new desktop size.
                            image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
                            // Update the active stage with the new channel IDs and pointer settings.
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

    Ok(RdpControlFlow::TerminatedGracefully(disconnect_reason))
}
