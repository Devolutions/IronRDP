use ironrdp::connector::sspi::network_client::reqwest_network_client::RequestClientFactory;
use ironrdp::connector::{ConnectionResult, ConnectorResult};
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::pdu::input::fast_path::FastPathInputEvent;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput, SessionResult};
use ironrdp::{connector, session};
use smallvec::SmallVec;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use winit::event_loop::EventLoopProxy;

use crate::config::Config;
use ironrdp::cliprdr::backend::{ClipboardMessage, CliprdrBackendFactory};
use ironrdp::cliprdr::Cliprdr;

#[derive(Debug)]
pub enum RdpOutputEvent {
    Image { buffer: Vec<u32>, width: u16, height: u16 },
    ConnectionFailure(connector::ConnectorError),
    PointerDefault,
    PointerHidden,
    PointerPosition { x: usize, y: usize },
    Terminated(SessionResult<()>),
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
    pub cliprdr_builder: Option<Box<dyn CliprdrBackendFactory + Send>>,
}

impl RdpClient {
    pub async fn run(mut self) {
        loop {
            let (connection_result, framed) = match connect(&self.config, self.cliprdr_builder.as_deref()).await {
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
                Ok(RdpControlFlow::TerminatedGracefully) => {
                    let _ = self.event_loop_proxy.send_event(RdpOutputEvent::Terminated(Ok(())));
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
    TerminatedGracefully,
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
        .with_server_name(&config.destination)
        .with_credssp_network_client(RequestClientFactory)
        // .with_static_channel(ironrdp::dvc::Drdynvc::new()) // FIXME: drdynvc is not working
        .with_static_channel(ironrdp::rdpsnd::Rdpsnd::new())
        .with_static_channel(ironrdp::rdpdr::Rdpdr::default());

    if let Some(builder) = cliprdr_factory {
        let backend = builder.build_cliprdr_backend();

        let cliprdr = Cliprdr::new(backend);

        connector = connector.with_static_channel(cliprdr);
    }

    let should_upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector).await?;

    debug!("TLS upgrade");

    // Ensure there is no leftover
    let initial_stream = framed.into_inner_no_leftover();

    let (upgraded_stream, server_public_key) = ironrdp_tls::upgrade(initial_stream, config.destination.name())
        .await
        .map_err(|e| connector::custom_err!("TLS upgrade", e))?;

    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector, server_public_key);

    let mut upgraded_framed = ironrdp_tokio::TokioFramed::new(upgraded_stream);

    let connection_result = ironrdp_tokio::connect_finalize(upgraded, &mut upgraded_framed, connector).await?;

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

    let mut active_stage = ActiveStage::new(connection_result, None);

    'outer: loop {
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
                        // TODO: Add support for Display Update Virtual Channel Extension
                        // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/d2954508-f487-48bc-8731-39743e0854a9
                        // One approach when this extension is not available is to perform a connection from scratch again.

                        // Find the last resize event
                        while let Ok(newer_event) = input_event_receiver.try_recv() {
                            if let RdpInputEvent::Resize { width: newer_width, height: newer_height } = newer_event {
                                width = newer_width;
                                height = newer_height;
                            }
                        }

                        // TODO: use the "auto-reconnect cookie": https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/15b0d1c9-2891-4adb-a45e-deb4aeeeab7c

                        info!(width, height, "resize event");

                        return Ok(RdpControlFlow::ReconnectWithNewSize { width, height })
                    },
                    RdpInputEvent::FastPath(events) => {
                        trace!(?events);
                        active_stage.process_fastpath_input(&mut image, &events)?
                    }
                    RdpInputEvent::Close => {
                        // TODO: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/27915739-8f77-487e-9927-55008af7fd68
                        break 'outer;
                    }
                    RdpInputEvent::Clipboard(event) => {
                        if let Some(cliprdr) = active_stage.get_svc_processor_downcast_ref::<ironrdp::cliprdr::Cliprdr>() {
                            let svc_request = match event {
                                ClipboardMessage::SendInitiateCopy(formats) => {
                                    let request = cliprdr.initiate_copy(&formats)
                                        .map_err(|e| session::custom_err!("CLIPRDR", e))?;
                                    Some(request)
                                }
                                ClipboardMessage::SendFormatData(response) => {
                                    let request = cliprdr.sumbit_format_data(response)
                                        .map_err(|e| session::custom_err!("CLIPRDR", e))?;
                                    Some(request)
                                }
                                ClipboardMessage::SendInitiatePaste(format) => {
                                    let request = cliprdr.initiate_paste(format)
                                        .map_err(|e| session::custom_err!("CLIPRDR", e))?;
                                    Some(request)
                                }
                                ClipboardMessage::Error(e) => {
                                    error!("Clipboard backend error: {}", e);
                                    None
                                }
                            };

                            if let Some(request) = svc_request {
                                let frame = active_stage.process_user_svc_request(request)?;
                                vec![ActiveStageOutput::ResponseFrame(frame)]
                            } else {
                                vec![]
                            }
                        } else  {
                            warn!("Clipboard event received, but Cliprdr is not available");
                            vec![]
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
                ActiveStageOutput::Terminate => break 'outer,
            }
        }
    }

    Ok(RdpControlFlow::TerminatedGracefully)
}
