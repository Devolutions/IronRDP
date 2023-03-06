use futures_util::io::AsyncWriteExt as _;
use ironrdp::core::input::fast_path::FastPathInputEvent;
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::session::connection_sequence::{process_connection_sequence, Address};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStageOutput, ActiveStageProcessor, RdpError};
use smallvec::SmallVec;
use sspi::network_client::reqwest_network_client::RequestClientFactory;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_util::compat::TokioAsyncReadCompatExt as _;
use winit::event_loop::EventLoopProxy;

use crate::config::Config;
use crate::tls::establish_tls;

#[derive(Debug)]
pub enum RdpOutputEvent {
    Image { buffer: Vec<u32>, width: u16, height: u16 },
    Terminated(Result<(), RdpError>),
}

#[derive(Debug)]
pub enum RdpInputEvent {
    Resize { width: u16, height: u16 },
    FastPath(SmallVec<[FastPathInputEvent; 2]>),
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
            match run_impl(&self.config, &self.event_loop_proxy, &mut self.input_event_receiver).await {
                Ok(RdpControlFlow::ReconnectWithNewSize { width, height }) => {
                    self.config.input.width = width;
                    self.config.input.height = height;
                }
                Ok(RdpControlFlow::TerminatedGracefully) => {
                    let _ = self.event_loop_proxy.send_event(RdpOutputEvent::Terminated(Ok(())));
                }
                Err(e) => {
                    let _ = self.event_loop_proxy.send_event(RdpOutputEvent::Terminated(Err(e)));
                }
            }
        }
    }
}

enum RdpControlFlow {
    ReconnectWithNewSize { width: u16, height: u16 },
    TerminatedGracefully,
}

async fn run_impl(
    config: &Config,
    event_loop_proxy: &EventLoopProxy<RdpOutputEvent>,
    input_event_receiver: &mut mpsc::UnboundedReceiver<RdpInputEvent>,
) -> Result<RdpControlFlow, RdpError> {
    let addr = Address::lookup_addr(config.addr.clone())?;

    let stream = TcpStream::connect(addr.sock).await.map_err(RdpError::Connection)?;

    let (connection_sequence_result, mut reader, mut writer) = process_connection_sequence(
        stream.compat(),
        &addr,
        &config.input,
        establish_tls,
        Box::new(RequestClientFactory),
    )
    .await?;

    let mut image = DecodedImage::new(
        PixelFormat::RgbA32,
        connection_sequence_result.desktop_size.width,
        connection_sequence_result.desktop_size.height,
    );

    let mut active_stage = ActiveStageProcessor::new(config.input.clone(), None, connection_sequence_result);

    'outer: loop {
        tokio::select! {
            frame = reader.read_frame() => {
                let frame = frame?.ok_or(RdpError::AccessDenied)?.freeze();
                let outputs = active_stage.process(&mut image, frame)?;
                for out in outputs {
                    match out {
                        ActiveStageOutput::ResponseFrame(frame) => writer.write_all(&frame).await?,
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
                                .map_err(|_| RdpError::Send("event_loop_proxy".to_owned()))?;
                        }
                        ActiveStageOutput::Terminate => break 'outer,
                    }
                }
            }
            input_event = input_event_receiver.recv() => {
                let input_event = input_event.ok_or_else(|| RdpError::Receive("gui_event_receiver".to_owned()))?;

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

                        return Ok(RdpControlFlow::ReconnectWithNewSize { width, height })
                    },
                    RdpInputEvent::FastPath(events) => {
                        use ironrdp::core::input::fast_path::FastPathInput;
                        use ironrdp::core::PduParsing as _;

                        trace!("Inputs: {events:?}");

                        // PERF: unnecessary copy
                        let fastpath_input = FastPathInput(events.into_vec());

                        let mut frame = Vec::new();
                        fastpath_input
                            .to_buffer(&mut frame)
                            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("Unable to encode FastPathInput: {e}")))?;

                        writer.write_all(&frame).await?;
                    }
                }
            }
        }
    }

    Ok(RdpControlFlow::TerminatedGracefully)
}
