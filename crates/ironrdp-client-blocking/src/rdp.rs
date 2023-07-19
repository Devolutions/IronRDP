use crate::config::Config;
use ironrdp::connector::{ConnectionResult, ConnectorResult};
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::pdu::input::fast_path::FastPathInputEvent;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput, SessionResult};
use ironrdp::{connector, session};
use mio::net::TcpStream;
use native_tls::{HandshakeError, MidHandshakeTlsStream, TlsStream};
use smallvec::SmallVec;
use sspi::network_client::reqwest_network_client::RequestClientFactory;
use std::io;
use std::net::TcpStream as StdTcpStream;
use std::sync::Arc;
use winit::event_loop::EventLoopProxy;

#[derive(Debug)]
pub enum RdpOutputEvent {
    Image { buffer: Vec<u32>, width: u16, height: u16 },
    ConnectionFailure(connector::ConnectorError),
    Terminated(SessionResult<()>),
}

#[derive(Debug)]
pub enum RdpInputEvent {
    Resize {
        width: u16,
        height: u16,
    },
    FastPath(SmallVec<[FastPathInputEvent; 2]>),
    Close,
    /// DevNull is a special event that is used to check if the input event channel is closed.
    DevNull,
}

impl RdpInputEvent {
    pub fn create_channel() -> (crossbeam::channel::Sender<Self>, crossbeam::channel::Receiver<Self>) {
        crossbeam::channel::unbounded()
    }
}

pub struct RdpClient {
    pub config: Config,
    pub event_loop_proxy: EventLoopProxy<RdpOutputEvent>,
    pub input_event_receiver: crossbeam::channel::Receiver<RdpInputEvent>,
}

impl RdpClient {
    pub fn run(mut self) {
        loop {
            let (connection_result, framed) = match connect(&self.config) {
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
            ) {
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

type UpgradedFramed = ironrdp_blocking::Framed<TlsStream<TcpStream>>;

fn connect(config: &Config) -> ConnectorResult<(ConnectionResult, UpgradedFramed)> {
    let server_addr = config
        .destination
        .lookup_addr()
        .map_err(|e| connector::custom_err!("lookup addr", e))?;

    let stream = StdTcpStream::connect(server_addr).map_err(|e| connector::custom_err!("TCP connect", e))?;

    let mut framed = ironrdp_blocking::Framed::new(stream);

    let mut connector = connector::ClientConnector::new(config.connector.clone())
        .with_server_addr(server_addr)
        .with_server_name(&config.destination)
        .with_credssp_client_factory(Box::new(RequestClientFactory));

    let should_upgrade = ironrdp_blocking::connect_begin(&mut framed, &mut connector)?;

    debug!("TLS upgrade");

    // Ensure there is no leftover
    let initial_stream = framed.into_inner_no_leftover();
    // Set the stream to non-blocking so that it can be read/written
    // from/to in multiple threads without causing a deadlock.
    initial_stream
        .set_nonblocking(true)
        .map_err(|e| connector::custom_err!("set nonblocking", e))?;
    let initial_stream = TcpStream::from_std(initial_stream);

    let (upgraded_stream, server_public_key) = upgrade_blocking(initial_stream, config.destination.name())
        .map_err(|e| connector::custom_err!("TLS upgrade", e))?;

    let upgraded = ironrdp_blocking::mark_as_upgraded(should_upgrade, &mut connector, server_public_key);

    let mut upgraded_framed = ironrdp_blocking::Framed::new(upgraded_stream);

    let connection_result = ironrdp_blocking::connect_finalize(upgraded, &mut upgraded_framed, connector)?;

    Ok((connection_result, upgraded_framed))
}

// TODO: should probably be moved to ironrdp-tls
fn upgrade_blocking(stream: TcpStream, server_name: &str) -> io::Result<(TlsStream<TcpStream>, Vec<u8>)> {
    let tls_connector = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true) // TODO: this should probably be optional
        .use_sni(false) // TODO: I'm not sure why this is here, just copied from ironrdp-tls
        .build()
        .map_err(|e| connector::custom_err!("create TlsConnector", e))?;

    let tls_stream = match tls_connector.connect(server_name, stream) {
        Ok(stream) => Ok(stream),
        Err(HandshakeError::WouldBlock(s)) => handle_midhandshake_tls_stream(s),
        Err(e) => Err(connector::custom_err!("TLS handshake", e))?,
    }?;

    let server_public_key = {
        let cert = tls_stream
            .peer_certificate()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "peer certificate is missing"))?;
        let cert = cert.to_der().map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        extract_tls_server_public_key(&cert)?
    };

    Ok((tls_stream, server_public_key))
}

fn handle_midhandshake_tls_stream(s: MidHandshakeTlsStream<TcpStream>) -> io::Result<native_tls::TlsStream<TcpStream>> {
    use mio::{Events, Interest, Poll, Token};
    use std::time::Duration;

    let mut s = s;
    let mut events = Events::with_capacity(128);

    // Create a `Poll` instance to monitor our stream
    let mut poll = Poll::new()?;

    // Register the stream with `Poll`. We want to know when the stream is ready for reading or writing.
    poll.registry()
        .register(s.get_mut(), Token(0), Interest::READABLE | Interest::WRITABLE)?;

    loop {
        // Wait for the stream to become ready
        poll.poll(&mut events, Some(Duration::from_secs(1)))?;

        match s.handshake() {
            Ok(stream) => return Ok(stream),
            Err(HandshakeError::WouldBlock(s2)) => s = s2,
            Err(HandshakeError::Failure(e)) => return Err(connector::custom_err!("TLS handshake", e))?,
        }
    }
}

// TODO: taken from ironrdp-tls, remove x509_cert from Cargo.toml when this is removed
fn extract_tls_server_public_key(cert: &[u8]) -> io::Result<Vec<u8>> {
    use x509_cert::der::Decode as _;

    let cert = x509_cert::Certificate::from_der(cert).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let server_public_key = cert
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "subject public key BIT STRING is not aligned"))?
        .to_owned();

    Ok(server_public_key)
}

fn active_session(
    framed: UpgradedFramed,
    connection_result: ConnectionResult,
    event_loop_proxy: &EventLoopProxy<RdpOutputEvent>,
    input_event_receiver: &mut crossbeam::channel::Receiver<RdpInputEvent>,
) -> SessionResult<RdpControlFlow> {
    let mut image = DecodedImage::new(
        PixelFormat::RgbA32,
        connection_result.desktop_size.width,
        connection_result.desktop_size.height,
    );

    let mut active_stage = ActiveStage::new(connection_result, None);

    let (pdu_tx, pdu_rx) = crossbeam::channel::unbounded();
    let framed = Arc::new(std::sync::Mutex::new(framed));
    let framed_clone = framed.clone();

    std::thread::spawn(move || {
        trace!("Spawned frame reading thread");
        loop {
            trace!("Waiting for lock in frame reading thread");
            let mut locked_frame = framed.lock().unwrap();
            trace!("Got lock in frame reading thread");
            trace!("Waiting for read_pdu in frame reading thread");
            let frame = locked_frame.read_pdu();
            trace!("Got read_pdu in frame reading thread");
            trace!("Sending frame to session thread");
            match pdu_tx.send(frame) {
                Ok(()) => {
                    trace!("Frame successfully sent to session thread");
                }
                Err(crossbeam::channel::SendError(_)) => {
                    trace!("Failed to send frame to session thread, this may be due to a resize event, returning");
                    return;
                }
            }
        }
    });

    'outer: loop {
        crossbeam::channel::select! {
            recv(pdu_rx) -> frame_recv_result => {
                let frame = frame_recv_result.map_err(|e| session::custom_err!("recv frame", e))?;
                let (action, payload) = frame.map_err(|e| session::custom_err!("read frame", e))?;
                trace!(?action, frame_length = payload.len(), "Frame received");

                let outputs = active_stage.process(&mut image, action, &payload)?;

                for out in outputs {
                    match out {
                        ActiveStageOutput::ResponseFrame(frame) => {
                            trace!("Waiting for lock in session thread (pdu_rx)");
                            let mut framed_clone_locked = framed_clone.lock().unwrap();
                            trace!("Got lock in session thread (pdu_rx)");
                            trace!("Writing response frame in session thread (pdu_rx)");
                            framed_clone_locked.write_all(&frame).map_err(|e| session::custom_err!("write response", e))?;
                            trace!("Wrote response frame in session thread (pdu_rx)");
                        },
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
                        ActiveStageOutput::Terminate => break 'outer,
                    }
                }
            },
            recv(input_event_receiver) -> input_event => {
                let input_event = input_event.map_err(|_| session::general_err!("GUI is stopped"))?;

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
                            .map_err(|e| session::custom_err!("FastPathInput encode", e))?;
                            trace!("Waiting for lock in session thread (input_event_receiver)");
                            let mut framed_clone_locked = framed_clone.lock().unwrap();
                            trace!("Got lock in session thread (input_event_receiver)");
                            trace!("Writing response frame in session thread (input_event_receiver)");
                            framed_clone_locked.write_all(&frame).map_err(|e| session::custom_err!("write FastPathInput PDU", e))?;
                            trace!("Wrote response frame in session thread (input_event_receiver)");
                    }
                    RdpInputEvent::Close => {
                        // TODO: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/27915739-8f77-487e-9927-55008af7fd68
                        break 'outer;
                    }
                    RdpInputEvent::DevNull => {}
                }
            }
        }
    }

    Ok(RdpControlFlow::TerminatedGracefully)
}
