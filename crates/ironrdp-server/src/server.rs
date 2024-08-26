use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use ironrdp_acceptor::{self, Acceptor, AcceptorResult, BeginResult, DesktopSize};
use ironrdp_async::bytes;
use ironrdp_cliprdr::backend::ClipboardMessage;
use ironrdp_cliprdr::CliprdrServer;
use ironrdp_core::impl_as_any;
use ironrdp_displaycontrol::pdu::DisplayControlMonitorLayout;
use ironrdp_displaycontrol::server::{DisplayControlHandler, DisplayControlServer};
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent};
use ironrdp_pdu::input::InputEventPdu;
use ironrdp_pdu::mcs::{SendDataIndication, SendDataRequest};
use ironrdp_pdu::rdp::capability_sets::{BitmapCodecs, CapabilitySet, CmdFlags, GeneralExtraFlags};
use ironrdp_pdu::rdp::headers::{ServerDeactivateAll, ShareControlPdu};
use ironrdp_pdu::{self, decode, encode_vec, mcs, nego, rdp, Action, PduResult};
use ironrdp_svc::{server_encode_svc_messages, StaticChannelId, StaticChannelSet, SvcProcessor};
use ironrdp_tokio::{Framed, FramedRead, FramedWrite, TokioFramed};
use rdpsnd::server::{RdpsndServer, RdpsndServerMessage};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio::task;
use tokio_rustls::TlsAcceptor;
use {ironrdp_dvc as dvc, ironrdp_rdpsnd as rdpsnd};

use crate::clipboard::CliprdrServerFactory;
use crate::display::{DisplayUpdate, RdpServerDisplay};
use crate::encoder::UpdateEncoder;
use crate::handler::RdpServerInputHandler;
use crate::{builder, capabilities, SoundServerFactory};

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
            RdpServerSecurity::None => nego::SecurityProtocol::empty(),
            RdpServerSecurity::Tls(_) => nego::SecurityProtocol::SSL,
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
                let handler = Arc::clone(&self.handler);
                task::spawn_blocking(move || {
                    handler.blocking_lock().mouse(pdu.into());
                });
            }
        }

        Ok(Vec::new())
    }
}

impl dvc::DvcServerProcessor for AInputHandler {}

struct DisplayControlBackend {
    display: Arc<Mutex<Box<dyn RdpServerDisplay>>>,
}

impl DisplayControlBackend {
    fn new(display: Arc<Mutex<Box<dyn RdpServerDisplay>>>) -> Self {
        Self { display }
    }
}

impl DisplayControlHandler for DisplayControlBackend {
    fn monitor_layout(&self, layout: DisplayControlMonitorLayout) {
        let display = Arc::clone(&self.display);
        task::spawn_blocking(move || display.blocking_lock().request_layout(layout));
    }
}

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
    display: Arc<Mutex<Box<dyn RdpServerDisplay>>>,
    static_channels: StaticChannelSet,
    sound_factory: Option<Box<dyn SoundServerFactory>>,
    cliprdr_factory: Option<Box<dyn CliprdrServerFactory>>,
    ev_sender: mpsc::UnboundedSender<ServerEvent>,
    ev_receiver: mpsc::UnboundedReceiver<ServerEvent>,
}

#[derive(Debug)]
pub enum ServerEvent {
    Quit(String),
    Clipboard(ClipboardMessage),
    Rdpsnd(RdpsndServerMessage),
}

pub trait ServerEventSender {
    fn set_sender(&mut self, sender: mpsc::UnboundedSender<ServerEvent>);
}

impl ServerEvent {
    pub fn create_channel() -> (mpsc::UnboundedSender<Self>, mpsc::UnboundedReceiver<Self>) {
        mpsc::unbounded_channel()
    }
}

#[derive(Debug, PartialEq)]
enum RunState {
    Continue,
    Disconnect,
    DeactivationReactivation { desktop_size: DesktopSize },
}

impl RdpServer {
    pub fn new(
        opts: RdpServerOptions,
        handler: Box<dyn RdpServerInputHandler>,
        display: Box<dyn RdpServerDisplay>,
        mut sound_factory: Option<Box<dyn SoundServerFactory>>,
        mut cliprdr_factory: Option<Box<dyn CliprdrServerFactory>>,
    ) -> Self {
        let (ev_sender, ev_receiver) = ServerEvent::create_channel();
        if let Some(cliprdr) = cliprdr_factory.as_mut() {
            cliprdr.set_sender(ev_sender.clone());
        }
        if let Some(snd) = sound_factory.as_mut() {
            snd.set_sender(ev_sender.clone());
        }
        Self {
            opts,
            handler: Arc::new(Mutex::new(handler)),
            display: Arc::new(Mutex::new(display)),
            static_channels: StaticChannelSet::new(),
            sound_factory,
            cliprdr_factory,
            ev_sender,
            ev_receiver,
        }
    }

    pub fn builder() -> builder::RdpServerBuilder<builder::WantsAddr> {
        builder::RdpServerBuilder::new()
    }

    pub fn event_sender(&self) -> &mpsc::UnboundedSender<ServerEvent> {
        &self.ev_sender
    }

    fn attach_channels(&mut self, acceptor: &mut Acceptor) {
        if let Some(cliprdr_factory) = self.cliprdr_factory.as_deref() {
            let backend = cliprdr_factory.build_cliprdr_backend();

            let cliprdr = CliprdrServer::new(backend);

            acceptor.attach_static_channel(cliprdr);
        }

        if let Some(factory) = self.sound_factory.as_deref() {
            let backend = factory.build_backend();

            acceptor.attach_static_channel(RdpsndServer::new(backend));
        }

        let dcs_backend = DisplayControlBackend::new(Arc::clone(&self.display));
        let dvc = dvc::DrdynvcServer::new()
            .with_dynamic_channel(AInputHandler {
                handler: Arc::clone(&self.handler),
            })
            .with_dynamic_channel(DisplayControlServer::new(Box::new(dcs_backend)));
        acceptor.attach_static_channel(dvc);
    }

    pub async fn run_connection(&mut self, stream: TcpStream) -> Result<()> {
        let framed = TokioFramed::new(stream);

        let size = self.display.lock().await.size().await;
        let capabilities = capabilities::capabilities(&self.opts, size);
        let mut acceptor = Acceptor::new(self.opts.security.flag(), size, capabilities);

        self.attach_channels(&mut acceptor);

        let res = ironrdp_acceptor::accept_begin(framed, &mut acceptor)
            .await
            .context("accept_begin failed")?;

        match res {
            BeginResult::ShouldUpgrade(stream) => {
                let framed = TokioFramed::new(match &self.opts.security {
                    RdpServerSecurity::Tls(acceptor) => acceptor.accept(stream).await?,
                    RdpServerSecurity::None => unreachable!(),
                });

                self.accept_finalize(framed, acceptor).await?;
            }

            BeginResult::Continue(framed) => {
                self.accept_finalize(framed, acceptor).await?;
            }
        };

        Ok(())
    }

    pub async fn run(&mut self) -> Result<()> {
        let listener = TcpListener::bind(self.opts.addr).await?;

        debug!("Listening for connections");
        loop {
            tokio::select! {
                Some(event) = self.ev_receiver.recv() => {
                    match event {
                        ServerEvent::Quit(reason) => {
                            debug!("Got quit event {reason}");
                            break;
                        }
                        ev => {
                            debug!("Unexpected event {:?}", ev);
                        }
                    }
                },
                Ok((stream, peer)) = listener.accept() => {
                    debug!(?peer, "Received connection");
                    if let Err(error) = self.run_connection(stream).await {
                        error!(?error, "Connection error");
                    }
                    self.static_channels = StaticChannelSet::new();
                }
                else => break,
            }
        }

        Ok(())
    }

    pub fn get_svc_processor<T: SvcProcessor + 'static>(&mut self) -> Option<&mut T> {
        self.static_channels
            .get_by_type_mut::<T>()
            .and_then(|svc| svc.channel_processor_downcast_mut())
    }

    pub fn get_channel_id_by_type<T: SvcProcessor + 'static>(&self) -> Option<StaticChannelId> {
        self.static_channels.get_channel_id_by_type::<T>()
    }

    async fn dispatch_pdu<S>(
        &mut self,
        action: Action,
        bytes: bytes::BytesMut,
        framed: &mut Framed<S>,
        io_channel_id: u16,
        user_channel_id: u16,
    ) -> Result<RunState>
    where
        S: FramedWrite + FramedRead,
    {
        match action {
            Action::FastPath => {
                let input = decode(&bytes)?;
                self.handle_fastpath(input).await;
            }

            Action::X224 => {
                if self
                    .handle_x224(framed, io_channel_id, user_channel_id, &bytes)
                    .await
                    .context("X224 input error")?
                {
                    debug!("Got disconnect request");
                    return Ok(RunState::Disconnect);
                }
            }
        }

        Ok(RunState::Continue)
    }

    async fn dispatch_display_update<S>(
        &mut self,
        update: DisplayUpdate,
        framed: &mut Framed<S>,
        user_channel_id: u16,
        io_channel_id: u16,
        buffer: &mut Vec<u8>,
        encoder: &mut UpdateEncoder,
    ) -> Result<RunState>
    where
        S: FramedWrite + FramedRead,
    {
        let mut fragmenter = match update {
            DisplayUpdate::Bitmap(bitmap) => encoder.bitmap(bitmap),
            DisplayUpdate::PointerPosition(pos) => encoder.pointer_position(pos),
            DisplayUpdate::Resize(desktop_size) => {
                debug!(?desktop_size, "Display resize");
                let pdu = ShareControlPdu::ServerDeactivateAll(ServerDeactivateAll);
                let pdu = rdp::headers::ShareControlHeader {
                    share_id: 0,
                    pdu_source: io_channel_id,
                    share_control_pdu: pdu,
                };
                let user_data = encode_vec(&pdu)?.into();
                let pdu = SendDataIndication {
                    initiator_id: user_channel_id,
                    channel_id: io_channel_id,
                    user_data,
                };
                let msg = encode_vec(&pdu)?;
                framed.write_all(&msg).await?;
                return Ok(RunState::DeactivationReactivation { desktop_size });
            }
            DisplayUpdate::RGBAPointer(ptr) => encoder.rgba_pointer(ptr),
            DisplayUpdate::ColorPointer(ptr) => encoder.color_pointer(ptr),
            DisplayUpdate::HidePointer => encoder.hide_pointer(),
            DisplayUpdate::DefaultPointer => encoder.default_pointer(),
        }
        .context("error during update encoding")?;

        if fragmenter.size_hint() > buffer.len() {
            buffer.resize(fragmenter.size_hint(), 0);
        }

        while let Some(len) = fragmenter.next(buffer) {
            framed
                .write_all(&buffer[..len])
                .await
                .context("failed to write display update")?;
        }

        Ok(RunState::Continue)
    }

    async fn dispatch_server_events<S>(
        &mut self,
        events: &mut Vec<ServerEvent>,
        framed: &mut Framed<S>,
        user_channel_id: u16,
    ) -> Result<RunState>
    where
        S: FramedWrite + FramedRead,
    {
        // Avoid wave message queuing up and causing extra delays.
        // This is a naive solution, better solutions should compute the actual delay, add IO priority, encode audio, use UDP etc.
        // 4 frames should roughly corresponds to hundreds of ms in regular setups.
        let mut wave_limit = 4;
        for event in events.drain(..) {
            match event {
                ServerEvent::Quit(reason) => {
                    debug!("Got quit event: {reason}");
                    return Ok(RunState::Disconnect);
                }
                ServerEvent::Rdpsnd(s) => {
                    let Some(rdpsnd) = self.get_svc_processor::<RdpsndServer>() else {
                        warn!("No rdpsnd channel, dropping event");
                        continue;
                    };
                    let msgs = match s {
                        RdpsndServerMessage::Wave(data, ts) => {
                            if wave_limit == 0 {
                                continue;
                            }
                            wave_limit -= 1;
                            rdpsnd.wave(data, ts)
                        }
                        RdpsndServerMessage::Close => rdpsnd.close(),
                        RdpsndServerMessage::Error(error) => {
                            error!(?error, "Handling rdpsnd event");
                            continue;
                        }
                    }
                    .context("failed to send rdpsnd event")?;
                    let channel_id = self
                        .get_channel_id_by_type::<RdpsndServer>()
                        .ok_or_else(|| anyhow!("SVC channel not found"))?;
                    let data = server_encode_svc_messages(msgs.into(), channel_id, user_channel_id)?;
                    framed.write_all(&data).await?;
                }
                ServerEvent::Clipboard(c) => {
                    let Some(cliprdr) = self.get_svc_processor::<CliprdrServer>() else {
                        warn!("No clipboard channel, dropping event");
                        continue;
                    };
                    let msgs = match c {
                        ClipboardMessage::SendInitiateCopy(formats) => cliprdr.initiate_copy(&formats),
                        ClipboardMessage::SendFormatData(data) => cliprdr.submit_format_data(data),
                        ClipboardMessage::SendInitiatePaste(format) => cliprdr.initiate_paste(format),
                        ClipboardMessage::Error(error) => {
                            error!(?error, "Handling clipboard event");
                            continue;
                        }
                    }
                    .context("failed to send clipboard event")?;
                    let channel_id = self
                        .get_channel_id_by_type::<CliprdrServer>()
                        .ok_or_else(|| anyhow!("SVC channel not found"))?;
                    let data = server_encode_svc_messages(msgs.into(), channel_id, user_channel_id)?;
                    framed.write_all(&data).await?;
                }
            }
        }

        Ok(RunState::Continue)
    }

    async fn client_loop<S>(
        &mut self,
        framed: &mut Framed<S>,
        io_channel_id: u16,
        user_channel_id: u16,
        mut encoder: UpdateEncoder,
    ) -> Result<RunState>
    where
        S: FramedWrite + FramedRead,
    {
        debug!("Starting client loop");

        let mut buffer = vec![0u8; 4096];
        let mut display_updates = self.display.lock().await.updates().await?;
        let mut events = Vec::with_capacity(100);
        let mut state = RunState::Continue;

        while state == RunState::Continue {
            tokio::select! {
                frame = framed.read_pdu() => {
                    let Ok((action, bytes)) = frame else {
                        debug!(?frame, "disconnecting");
                        state = RunState::Disconnect;
                        break;
                    };
                    state = self.dispatch_pdu(action, bytes, framed, io_channel_id, user_channel_id).await?;
                },

                Some(update) = display_updates.next_update() => {
                    state = self.dispatch_display_update(update, framed, user_channel_id, io_channel_id, &mut buffer, &mut encoder).await?;
                }

                nevents = self.ev_receiver.recv_many(&mut events, 100) => {
                    if nevents == 0 {
                        debug!("No sever events.. stopping");
                        state = RunState::Disconnect;
                        break;
                    }
                    while let Ok(ev) = self.ev_receiver.try_recv() {
                        events.push(ev);
                    }
                    state = self.dispatch_server_events(&mut events, framed, user_channel_id).await?;
                }

                else => {
                    debug!("All streams closed, disconnecting");
                    state = RunState::Disconnect;
                }
            }
        }

        debug!("End of client loop");
        Ok(state)
    }

    async fn client_accepted<S>(&mut self, framed: &mut Framed<S>, result: AcceptorResult) -> Result<RunState>
    where
        S: FramedWrite + FramedRead,
    {
        debug!("Client accepted");

        if !result.input_events.is_empty() {
            debug!("Handling input event backlog from acceptor sequence");
            self.handle_input_backlog(
                framed,
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

        let encoder = UpdateEncoder::new(surface_flags, rfxcodec);

        let state = self
            .client_loop(framed, result.io_channel_id, result.user_channel_id, encoder)
            .await
            .context("client loop failure")?;

        Ok(state)
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
            let mut handler = self.handler.lock().await;
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
            let mut handler = self.handler.lock().await;
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

    async fn accept_finalize<S>(&mut self, mut framed: TokioFramed<S>, mut acceptor: Acceptor) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Sync + Send + Unpin,
    {
        let mut other_pdus = None;

        loop {
            let (new_framed, result) = ironrdp_acceptor::accept_finalize(framed, &mut acceptor, other_pdus.as_mut())
                .await
                .context("failed to accept client during finalize")?;

            let (stream, mut leftover) = new_framed.into_inner();

            if let Some(pdus) = other_pdus.take() {
                let unmatched_frames = pdus.into_iter().flatten();
                let previous_leftover = leftover.split();
                leftover.extend(unmatched_frames);
                leftover.extend_from_slice(&previous_leftover);
            }

            framed = TokioFramed::new_with_leftover(stream, leftover);

            match self.client_accepted(&mut framed, result).await? {
                RunState::Continue => {
                    unreachable!();
                }
                RunState::DeactivationReactivation { desktop_size } => {
                    other_pdus = Some(Vec::new());
                    acceptor = Acceptor::new_deactivation_reactivation(acceptor, desktop_size);
                    self.attach_channels(&mut acceptor);
                    continue;
                }
                RunState::Disconnect => break,
            }
        }

        Ok(())
    }
}
