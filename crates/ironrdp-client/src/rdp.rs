use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::pdu::input::fast_path::FastPathInputEvent;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput};
use ironrdp::{connector, session};
use smallvec::SmallVec;
use sspi::network_client::reqwest_network_client::RequestClientFactory;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use winit::event_loop::EventLoopProxy;

use crate::config::Config;

#[derive(Debug)]
pub enum RdpOutputEvent {
    Image { buffer: Vec<u32>, width: u16, height: u16 },
    ConnectionFailure(connector::Error),
    Terminated(session::Result<()>),
}

#[derive(Debug)]
pub enum RdpInputEvent {
    Resize { width: u16, height: u16 },
    FastPath(SmallVec<[FastPathInputEvent; 2]>),
    Close,
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
}

impl RdpClient {
    pub async fn run(mut self) {
        loop {
            let (connection_result, framed) = match connect(&self.config).await {
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

type UpgradedFramed = ironrdp_async::Framed<ironrdp_async::TokioCompat<ironrdp_tls::TlsStream<TcpStream>>>;

async fn connect(config: &Config) -> connector::Result<(connector::ConnectionResult, UpgradedFramed)> {
    let server_addr = config
        .destination
        .lookup_addr()
        .map_err(|e| connector::Error::new("lookup addr").with_custom(e))?;

    let stream = TcpStream::connect(&server_addr)
        .await
        .map_err(|e| connector::Error::new("TCP connect").with_custom(e))?;

    let mut framed = ironrdp_async::Framed::tokio_new(stream);

    let mut connector = connector::ClientConnector::new(config.connector.clone())
        .with_server_addr(server_addr)
        .with_server_name(&config.destination)
        .with_credssp_client_factory(Box::new(RequestClientFactory));

    let should_upgrade = ironrdp_async::connect_begin(&mut framed, &mut connector).await?;

    debug!("TLS upgrade");

    // Ensure there is no leftover
    let initial_stream = framed.tokio_into_inner_no_leftover();

    let (upgraded_stream, server_public_key) = ironrdp_tls::upgrade(initial_stream, config.destination.name())
        .await
        .map_err(|e| connector::Error::new("TLS upgrade").with_custom(e))?;

    let upgraded = ironrdp_async::mark_as_upgraded(should_upgrade, &mut connector, server_public_key);

    let mut upgraded_framed = ironrdp_async::Framed::tokio_new(upgraded_stream);

    let connection_result = ironrdp_async::connect_finalize(upgraded, &mut upgraded_framed, connector).await?;

    Ok((connection_result, upgraded_framed))
}

async fn active_session(
    mut framed: UpgradedFramed,
    connection_result: connector::ConnectionResult,
    event_loop_proxy: &EventLoopProxy<RdpOutputEvent>,
    input_event_receiver: &mut mpsc::UnboundedReceiver<RdpInputEvent>,
) -> session::Result<RdpControlFlow> {
    let mut image = DecodedImage::new(
        PixelFormat::RgbA32,
        connection_result.desktop_size.width,
        connection_result.desktop_size.height,
    );

    let mut active_stage = ActiveStage::new(connection_result, None);

    'outer: loop {
        tokio::select! {
            frame = framed.read_pdu() => {
                let (action, payload) = frame.map_err(|e| session::Error::new("read frame").with_custom(e))?;
                trace!(?action, frame_length = payload.len(), "Frame received");

                let outputs = active_stage.process(&mut image, action, &payload)?;

                for out in outputs {
                    match out {
                        ActiveStageOutput::ResponseFrame(frame) => framed.write_all(&frame).await.map_err(|e| session::Error::new("write response").with_custom(e))?,
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
                                .map_err(|e| session::Error::new("event_loop_proxy").with_custom(e))?;
                        }
                        ActiveStageOutput::Terminate => break 'outer,
                    }
                }
            }
            input_event = input_event_receiver.recv() => {
                let input_event = input_event.ok_or(session::Error::new("GUI is stopped"))?;

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
                        use ironrdp::pdu::input::fast_path::FastPathInput;
                        use ironrdp::pdu::PduParsing as _;

                        trace!(?events);

                        // PERF: unnecessary copy
                        let fastpath_input = FastPathInput(events.into_vec());

                        let mut frame = Vec::new();
                        fastpath_input
                            .to_buffer(&mut frame)
                            .map_err(|e| session::Error::new("FastPathInput encode").with_custom(e))?;

                        framed.write_all(&frame).await.map_err(|e| session::Error::new("write FastPathInput PDU").with_custom(e))?;
                    }
                    RdpInputEvent::Close => {
                        // TODO: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/27915739-8f77-487e-9927-55008af7fd68
                        break 'outer;
                    }
                }
            }
        }
    }

    Ok(RdpControlFlow::TerminatedGracefully)
}
