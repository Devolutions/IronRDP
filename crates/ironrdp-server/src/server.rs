use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};
use ironrdp_acceptor::{self, Acceptor, AcceptorResult, BeginResult};
use ironrdp_cliprdr::backend::CliprdrBackendFactory;
use ironrdp_cliprdr::CliprdrServer;
use ironrdp_displaycontrol::server::DisplayControlServer;
use ironrdp_dvc as dvc;
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent};
use ironrdp_pdu::input::InputEventPdu;
use ironrdp_pdu::mcs::SendDataRequest;
use ironrdp_pdu::rdp::capability_sets::{BitmapCodecs, CapabilitySet, CmdFlags, GeneralExtraFlags};
use ironrdp_pdu::{self, decode, mcs, nego, rdp, Action, PduResult};
use ironrdp_svc::{impl_as_any, server_encode_svc_messages, StaticChannelSet};
use ironrdp_tokio::{Framed, FramedRead, FramedWrite, TokioFramed};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use crate::display::{DisplayUpdate, RdpServerDisplay};
use crate::encoder::UpdateEncoder;
use crate::handler::RdpServerInputHandler;
use crate::{builder, capabilities};

#[derive(Clone)]
pub struct RdpServerOptions {
    pub addr: SocketAddr,
    pub security: RdpServerSecurity,
}

#[derive(Clone)]
pub enum RdpServerSecurity {
    None,
    Tls(TlsAcceptor),
}

impl RdpServerSecurity {
    pub fn flag(&self) -> nego::SecurityProtocol {
        match self {
            RdpServerSecurity::None => ironrdp_pdu::nego::SecurityProtocol::empty(),
            RdpServerSecurity::Tls(_) => ironrdp_pdu::nego::SecurityProtocol::SSL,
        }
    }
}

struct AInputHandler {
    handler: Arc<Mutex<Box<dyn RdpServerInputHandler>>>,
}

impl_as_any!(AInputHandler);

impl dvc::DvcProcessor for AInputHandler {
    fn channel_name(&self) -> &str {
        ironrdp_ainput::CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<dvc::DvcMessage>> {
        use ironrdp_ainput::{ServerPdu, VersionPdu};

        let pdu = ServerPdu::Version(VersionPdu::default());

        Ok(vec![Box::new(pdu)])
    }

    fn close(&mut self, _channel_id: u32) {}

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<dvc::DvcMessage>> {
        use ironrdp_ainput::ClientPdu;

        match decode(payload)? {
            ClientPdu::Mouse(pdu) => {
                let mut handler = self.handler.lock().unwrap();
                handler.mouse(pdu.into());
            }
        }

        Ok(Vec::new())
    }
}

impl dvc::DvcServerProcessor for AInputHandler {}

/// RDP Server
///
/// A server is created to listen for connections.
/// After the connection sequence is finalized using the provided security mechanism, the server can:
///  - receive display updates from a [`RdpServerDisplay`] and forward them to the client
///  - receive input events from a client and forward them to an [`RdpServerInputHandler`]
///
/// # Example
///
/// ```
/// use ironrdp_server::{RdpServer, RdpServerInputHandler, RdpServerDisplay, RdpServerDisplayUpdates};
///
///# use anyhow::Result;
///# use ironrdp_server::{DisplayUpdate, DesktopSize, KeyboardEvent, MouseEvent};
///# use tokio_rustls::TlsAcceptor;
///# struct NoopInputHandler;
///# impl RdpServerInputHandler for NoopInputHandler {
///#     fn keyboard(&mut self, _: KeyboardEvent) {}
///#     fn mouse(&mut self, _: MouseEvent) {}
///# }
///# struct NoopDisplay;
///# #[async_trait::async_trait]
///# impl RdpServerDisplay for NoopDisplay {
///#     async fn size(&mut self) -> DesktopSize {
///#         todo!()
///#     }
///#     async fn updates(&mut self) -> Result<Box<dyn RdpServerDisplayUpdates>> {
///#         todo!()
///#     }
///# }
///# async fn stub() {
/// fn make_tls_acceptor() -> TlsAcceptor {
///    /* snip */
///#    todo!()
/// }
///
/// fn make_input_handler() -> impl RdpServerInputHandler {
///    /* snip */
///#    NoopInputHandler
/// }
///
/// fn make_display_handler() -> impl RdpServerDisplay {
///    /* snip */
///#    NoopDisplay
/// }
///
/// let tls_acceptor = make_tls_acceptor();
/// let input_handler = make_input_handler();
/// let display_handler = make_display_handler();
///
/// let mut server = RdpServer::builder()
///     .with_addr(([127, 0, 0, 1], 3389))
///     .with_tls(tls_acceptor)
///     .with_input_handler(input_handler)
///     .with_display_handler(display_handler)
///     .build();
///
/// server.run().await;
///# }
/// ```
pub struct RdpServer {
    opts: RdpServerOptions,
    // FIXME: replace with a channel and poll/process the handler?
    handler: Arc<Mutex<Box<dyn RdpServerInputHandler>>>,
    display: Box<dyn RdpServerDisplay>,
    static_channels: StaticChannelSet,
    cliprdr_factory: Option<Box<dyn CliprdrBackendFactory + Send>>,
}

impl RdpServer {
    pub fn new(
        opts: RdpServerOptions,
        handler: Box<dyn RdpServerInputHandler>,
        display: Box<dyn RdpServerDisplay>,
        cliprdr_factory: Option<Box<dyn CliprdrBackendFactory + Send>>,
    ) -> Self {
        Self {
            opts,
            handler: Arc::new(Mutex::new(handler)),
            display,
            static_channels: StaticChannelSet::new(),
            cliprdr_factory,
        }
    }

    pub fn builder() -> builder::RdpServerBuilder<builder::WantsAddr> {
        builder::RdpServerBuilder::new()
    }

    pub async fn run_connection(&mut self, stream: TcpStream) -> Result<()> {
        let framed = TokioFramed::new(stream);

        let size = self.display.size().await;
        let capabilities = capabilities::capabilities(&self.opts, size);
        let mut acceptor = Acceptor::new(self.opts.security.flag(), size, capabilities);

        if let Some(cliprdr_factory) = self.cliprdr_factory.as_deref() {
            let backend = cliprdr_factory.build_cliprdr_backend();

            let cliprdr = CliprdrServer::new(backend);

            acceptor.attach_static_channel(cliprdr);
        }

        let dvc = dvc::DrdynvcServer::new()
            .with_dynamic_channel(AInputHandler {
                handler: Arc::clone(&self.handler),
            })
            .with_dynamic_channel(DisplayControlServer);
        acceptor.attach_static_channel(dvc);

        match ironrdp_acceptor::accept_begin(framed, &mut acceptor).await {
            Ok(BeginResult::ShouldUpgrade(stream)) => {
                let framed = TokioFramed::new(match &self.opts.security {
                    RdpServerSecurity::Tls(acceptor) => acceptor.accept(stream).await?,
                    RdpServerSecurity::None => unreachable!(),
                });

                match ironrdp_acceptor::accept_finalize(framed, &mut acceptor).await {
                    Ok((framed, result)) => self.client_loop(framed, result).await?,
                    Err(error) => error!(?error, "Accept finalize error"),
                };
            }

            Ok(BeginResult::Continue(framed)) => {
                match ironrdp_acceptor::accept_finalize(framed, &mut acceptor).await {
                    Ok((framed, result)) => self.client_loop(framed, result).await?,
                    Err(error) => error!(?error, "Accept finalize error"),
                };
            }

            Err(error) => {
                error!(?error, "Accept begin error");
            }
        }

        Ok(())
    }

    pub async fn run(&mut self) -> Result<()> {
        let listener = TcpListener::bind(self.opts.addr).await?;

        debug!("Listening for connections");
        while let Ok((stream, peer)) = listener.accept().await {
            debug!(?peer, "Received connection");
            if let Err(error) = self.run_connection(stream).await {
                error!(?error, "Connection error");
            }
        }

        Ok(())
    }

    async fn client_loop<S>(&mut self, mut framed: Framed<S>, result: AcceptorResult) -> Result<()>
    where
        S: FramedWrite + FramedRead,
    {
        debug!("Starting client loop");

        if !result.input_events.is_empty() {
            debug!("Handling input event backlog from acceptor sequence");
            self.handle_input_backlog(
                &mut framed,
                result.io_channel_id,
                result.user_channel_id,
                result.input_events,
            )
            .await?;
        }

        self.static_channels = result.static_channels;
        for (_type_id, channel, channel_id) in self.static_channels.iter_mut() {
            debug!(?channel, ?channel_id, "Start");
            let Some(channel_id) = channel_id else {
                continue;
            };
            let svc_responses = channel.start()?;
            let response = server_encode_svc_messages(svc_responses, channel_id, result.user_channel_id)?;
            framed.write_all(&response).await?;
        }

        let mut rfxcodec = None;
        let mut surface_flags = CmdFlags::empty();
        for c in result.capabilities {
            match c {
                CapabilitySet::General(c) => {
                    let fastpath = c.extra_flags.contains(GeneralExtraFlags::FASTPATH_OUTPUT_SUPPORTED);
                    if !fastpath {
                        bail!("Fastpath output not supported!");
                    }
                }
                CapabilitySet::SurfaceCommands(c) => {
                    surface_flags = c.flags;
                }
                CapabilitySet::BitmapCodecs(BitmapCodecs(codecs)) => {
                    for codec in codecs {
                        match codec.property {
                            // FIXME: The encoder operates in image mode only.
                            //
                            // See [MS-RDPRFX] 3.1.1.1 "State Machine" for
                            // implementation of the video mode. which allows to
                            // skip sending Header for each image.
                            //
                            // We should distinguish parameters for both modes,
                            // and somehow choose the "best", instead of picking
                            // the last parsed here.
                            rdp::capability_sets::CodecProperty::RemoteFx(
                                rdp::capability_sets::RemoteFxContainer::ClientContainer(c),
                            ) => {
                                for caps in c.caps_data.0 .0 {
                                    rfxcodec = Some((caps.entropy_bits, codec.id));
                                }
                            }
                            rdp::capability_sets::CodecProperty::ImageRemoteFx(
                                rdp::capability_sets::RemoteFxContainer::ClientContainer(c),
                            ) => {
                                for caps in c.caps_data.0 .0 {
                                    rfxcodec = Some((caps.entropy_bits, codec.id));
                                }
                            }
                            rdp::capability_sets::CodecProperty::NsCodec(_) => (),
                            _ => (),
                        }
                    }
                }
                _ => {}
            }
        }

        let mut buffer = vec![0u8; 4096];
        let mut encoder = UpdateEncoder::new(surface_flags, rfxcodec);

        let mut display_updates = self.display.updates().await?;

        loop {
            tokio::select! {
                frame = framed.read_pdu() => {
                    let Ok((action, bytes)) = frame else {
                        debug!(?frame, "disconnecting");
                        break;
                    };

                    match action {
                        Action::FastPath => {
                            let input = decode(&bytes)?;
                            self.handle_fastpath(input).await;
                        }

                        Action::X224 => {
                            match self.handle_x224(&mut framed, result.io_channel_id, result.user_channel_id, &bytes).await {
                                Ok(disconnect) => {
                                    if disconnect {
                                        debug!("Got disconnect request");
                                        break;
                                    }
                                },

                                Err(error) => {
                                    error!(?error, "X224 input error");
                                }
                            };
                        }
                    }
                },

                Some(update) = display_updates.next_update() => {
                    let fragmenter = match update {
                        DisplayUpdate::Bitmap(bitmap) => encoder.bitmap(bitmap),
                        DisplayUpdate::PointerPosition(pos) => { encoder.pointer_position(pos) },
                        DisplayUpdate::RGBAPointer(ptr) => { encoder.rgba_pointer(ptr) },
                        DisplayUpdate::ColorPointer(ptr) => { encoder.color_pointer(ptr) },
                        DisplayUpdate::HidePointer => { encoder.hide_pointer() },
                        DisplayUpdate::DefaultPointer => { encoder.default_pointer() },
                    };

                    let mut fragmenter = match fragmenter {
                        Ok(fragmenter) => fragmenter,
                        Err(error) => {
                            error!(?error, "Error during update encoding");
                            break;
                        }
                    };

                    if fragmenter.size_hint() > buffer.len() {
                        buffer.resize(fragmenter.size_hint(), 0);
                    }

                    while let Some(len) = fragmenter.next(&mut buffer) {
                        if let Err(error) = framed.write_all(&buffer[..len]).await {
                            error!(?error, "Write display update error");
                            break;
                        };
                    }
                }
            }
        }

        debug!("End of client loop");

        Ok(())
    }

    async fn handle_input_backlog<S>(
        &mut self,
        framed: &mut Framed<S>,
        io_channel_id: u16,
        user_channel_id: u16,
        frames: Vec<Vec<u8>>,
    ) -> Result<()>
    where
        S: FramedWrite,
    {
        for frame in frames {
            match Action::from_fp_output_header(frame[0]) {
                Ok(Action::FastPath) => {
                    let input = decode(&frame)?;
                    self.handle_fastpath(input).await;
                }

                Ok(Action::X224) => {
                    let _ = self.handle_x224(framed, io_channel_id, user_channel_id, &frame).await;
                }

                // the frame here is always valid, because otherwise it would
                // have failed during the acceptor loop
                Err(_) => unreachable!(),
            }
        }

        Ok(())
    }

    async fn handle_fastpath(&mut self, input: FastPathInput) {
        for event in input.0 {
            let mut handler = self.handler.lock().unwrap();
            match event {
                FastPathInputEvent::KeyboardEvent(flags, key) => {
                    handler.keyboard((key, flags).into());
                }

                FastPathInputEvent::UnicodeKeyboardEvent(flags, key) => {
                    handler.keyboard((key, flags).into());
                }

                FastPathInputEvent::SyncEvent(flags) => {
                    handler.keyboard(flags.into());
                }

                FastPathInputEvent::MouseEvent(mouse) => {
                    handler.mouse(mouse.into());
                }

                FastPathInputEvent::MouseEventEx(mouse) => {
                    handler.mouse(mouse.into());
                }

                FastPathInputEvent::MouseEventRel(mouse) => {
                    handler.mouse(mouse.into());
                }

                FastPathInputEvent::QoeEvent(quality) => {
                    warn!("Received QoE: {}", quality);
                }
            }
        }
    }

    async fn handle_io_channel_data(&mut self, data: SendDataRequest<'_>) -> Result<bool> {
        let control: rdp::headers::ShareControlHeader = decode(data.user_data.as_ref())?;

        match control.share_control_pdu {
            rdp::headers::ShareControlPdu::Data(header) => match header.share_data_pdu {
                rdp::headers::ShareDataPdu::Input(pdu) => {
                    self.handle_input_event(pdu).await;
                }

                rdp::headers::ShareDataPdu::ShutdownRequest => {
                    return Ok(true);
                }

                unexpected => {
                    warn!(?unexpected, "Unexpected share data pdu");
                }
            },

            unexpected => {
                warn!(?unexpected, "Unexpected share control");
            }
        }

        Ok(false)
    }

    async fn handle_x224<S>(
        &mut self,
        framed: &mut Framed<S>,
        io_channel_id: u16,
        user_channel_id: u16,
        frame: &[u8],
    ) -> Result<bool>
    where
        S: FramedWrite,
    {
        let message = decode::<mcs::McsMessage<'_>>(frame)?;
        match message {
            mcs::McsMessage::SendDataRequest(data) => {
                debug!(?data, "McsMessage::SendDataRequest");
                if data.channel_id == io_channel_id {
                    return self.handle_io_channel_data(data).await;
                }

                if let Some(svc) = self.static_channels.get_by_channel_id_mut(data.channel_id) {
                    let response_pdus = svc.process(&data.user_data)?;
                    let response = server_encode_svc_messages(response_pdus, data.channel_id, user_channel_id)?;
                    framed.write_all(&response).await?;
                } else {
                    warn!(channel_id = data.channel_id, "Unexpected channel received: ID",);
                }
            }

            mcs::McsMessage::DisconnectProviderUltimatum(disconnect) => {
                if disconnect.reason == mcs::DisconnectReason::UserRequested {
                    return Ok(true);
                }
            }

            unexpected => {
                warn!(name = ironrdp_pdu::name(&unexpected), "Unexpected mcs message");
            }
        }

        Ok(false)
    }

    async fn handle_input_event(&mut self, input: InputEventPdu) {
        for event in input.0 {
            let mut handler = self.handler.lock().unwrap();
            match event {
                ironrdp_pdu::input::InputEvent::ScanCode(key) => {
                    handler.keyboard((key.key_code, key.flags).into());
                }

                ironrdp_pdu::input::InputEvent::Unicode(key) => {
                    handler.keyboard((key.unicode_code, key.flags).into());
                }

                ironrdp_pdu::input::InputEvent::Sync(sync) => {
                    handler.keyboard(sync.flags.into());
                }

                ironrdp_pdu::input::InputEvent::Mouse(mouse) => {
                    handler.mouse(mouse.into());
                }

                ironrdp_pdu::input::InputEvent::MouseX(mouse) => {
                    handler.mouse(mouse.into());
                }

                ironrdp_pdu::input::InputEvent::MouseRel(mouse) => {
                    handler.mouse(mouse.into());
                }

                ironrdp_pdu::input::InputEvent::Unused(_) => {}
            }
        }
    }
}
