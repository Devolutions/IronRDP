use ironrdp::cliprdr::backend::{ClipboardMessage, CliprdrBackendFactory};
use ironrdp::connector::connection_activation::ConnectionActivationState;
use ironrdp::connector::{ConnectionResult, ConnectorResult};
use ironrdp::displaycontrol::client::DisplayControlClient;
use ironrdp::dvc::DrdynvcClient;
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::pdu::input::fast_path::FastPathInputEvent;
use ironrdp::pdu::write_buf::WriteBuf;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{fast_path, ActiveStage, ActiveStageOutput, GracefulDisconnectReason, SessionResult};
use ironrdp::svc::SvcProcessorMessages;
use ironrdp::{cliprdr, connector, rdpdr, rdpsnd, session};
use ironrdp_tokio::single_sequence_step_read;
use rdpdr::NoopRdpdrBackend;
use smallvec::SmallVec;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use winit::event_loop::EventLoopProxy;

use crate::config::Config;

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
    Resize { width: u16, height: u16 },
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
            let (connection_result, framed) = match connect(&self.config, self.cliprdr_factory.as_deref()).await {
                Ok(result) => result,
                Err(e) => {
                    let _ = self.event_loop_proxy.send_event(RdpOutputEvent::ConnectionFailure(e));
                    break;
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

type UpgradedFramed = ironrdp_tokio::TokioFramed<ironrdp_tls::TlsStream<TcpStream>>;

async fn connect(
    config: &Config,
    cliprdr_factory: Option<&(dyn CliprdrBackendFactory + Send)>,
) -> ConnectorResult<(ConnectionResult, UpgradedFramed)> {
    let server_addr = config
        .destination
        .lookup_addr()
        .map_err(|e| connector::custom_err!("lookup addr", e))?;

    let stream = TcpStream::connect(&server_addr)
        .await
        .map_err(|e| connector::custom_err!("TCP connect", e))?;

    let mut framed = ironrdp_tokio::TokioFramed::new(stream);

    let mut connector = connector::ClientConnector::new(config.connector.clone())
        .with_server_addr(server_addr)
        .with_static_channel(
            ironrdp::dvc::DrdynvcClient::new().with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new()))),
        )
        .with_static_channel(rdpsnd::Rdpsnd::new())
        .with_static_channel(rdpdr::Rdpdr::new(Box::new(NoopRdpdrBackend {}), "IronRDP".to_owned()).with_smartcard(0));

    if let Some(builder) = cliprdr_factory {
        let backend = builder.build_cliprdr_backend();

        let cliprdr = cliprdr::Cliprdr::new(backend);

        connector.attach_static_channel(cliprdr);
    }

    let should_upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector).await?;

    debug!("TLS upgrade");

    // Ensure there is no leftover
    let initial_stream = framed.into_inner_no_leftover();

    let (upgraded_stream, server_public_key) = ironrdp_tls::upgrade(initial_stream, config.destination.name())
        .await
        .map_err(|e| connector::custom_err!("TLS upgrade", e))?;

    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);

    let mut upgraded_framed = ironrdp_tokio::TokioFramed::new(upgraded_stream);

    let mut network_client = crate::network_client::ReqwestNetworkClient::new();
    let connection_result = ironrdp_tokio::connect_finalize(
        upgraded,
        &mut upgraded_framed,
        connector,
        (&config.destination).into(),
        server_public_key,
        Some(&mut network_client),
        None,
    )
    .await?;

    debug!(?connection_result);

    Ok((connection_result, upgraded_framed))
}

async fn active_session(
    mut framed: UpgradedFramed,
    connection_result: ConnectionResult,
    event_loop_proxy: &EventLoopProxy<RdpOutputEvent>,
    input_event_receiver: &mut mpsc::UnboundedReceiver<RdpInputEvent>,
) -> SessionResult<RdpControlFlow> {
    let mut image = DecodedImage::new(
        PixelFormat::RgbA32,
        connection_result.desktop_size.width,
        connection_result.desktop_size.height,
    );

    let mut active_stage = ActiveStage::new(connection_result);

    let disconnect_reason = 'outer: loop {
        let outputs = tokio::select! {
            frame = framed.read_pdu() => {
                let (action, payload) = frame.map_err(|e| session::custom_err!("read frame", e))?;
                trace!(?action, frame_length = payload.len(), "Frame received");

                active_stage.process(&mut image, action, &payload)?
            }
            input_event = input_event_receiver.recv() => {
                let input_event = input_event.ok_or_else(|| session::general_err!("GUI is stopped"))?;

                match input_event {
                    RdpInputEvent::Resize { mut width, mut height } => {
                        // Find the last resize event
                        while let Ok(newer_event) = input_event_receiver.try_recv() {
                            if let RdpInputEvent::Resize { width: newer_width, height: newer_height } = newer_event {
                                width = newer_width;
                                height = newer_height;
                            }
                        }

                        info!(width, height, "resize event");

                        if let Some((display_client, channel_id)) = active_stage.get_dvc_processor::<DisplayControlClient>() {
                            if let Some(channel_id) = channel_id {
                                let svc_messages = display_client.encode_single_primary_monitor(channel_id, width.into(), height.into())
                                    .map_err(|e| session::custom_err!("DisplayControl", e))?;
                                let frame = active_stage.process_svc_processor_messages(SvcProcessorMessages::<DrdynvcClient>::new(svc_messages))?;
                                vec![ActiveStageOutput::ResponseFrame(frame)]
                            } else {
                                // TODO: could add a mechanism that withholds the resize event until the channel is created rather than reconnecting
                                debug!("Display Control Virtual Channel is not yet connected, reconnecting with new size");
                                return Ok(RdpControlFlow::ReconnectWithNewSize { width, height })
                            }
                        } else {
                            // TODO(#271): use the "auto-reconnect cookie": https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/15b0d1c9-2891-4adb-a45e-deb4aeeeab7c
                            debug!("Display Control Virtual Channel is not available, reconnecting with new size");
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
                        if let Some(cliprdr) = active_stage.get_svc_processor::<ironrdp::cliprdr::CliprdrClient>() {
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
                ActiveStageOutput::ResponseFrame(frame) => framed
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
                        let written = single_sequence_step_read(&mut framed, &mut *connection_activation, &mut buf)
                            .await
                            .map_err(|e| session::custom_err!("read deactivation-reactivation sequence step", e))?;

                        if written.size().is_some() {
                            framed.write_all(buf.filled()).await.map_err(|e| {
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
