use std::io::Cursor;
use std::net::SocketAddr;

use anyhow::Result;
use ironrdp_acceptor::{self, Acceptor, AcceptorResult, BeginResult};
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent};
use ironrdp_pdu::input::InputEventPdu;
use ironrdp_pdu::{self, mcs, nego, rdp, Action, PduParsing};
use ironrdp_tokio::{Framed, FramedRead, FramedWrite, TokioFramed};
use tokio::net::TcpListener;
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
/// use ironrdp_server::{RdpServer, RdpServerInputHandler, RdpServerDisplay};
///
///# use ironrdp_server::{DisplayUpdate, DesktopSize, KeyboardEvent, MouseEvent};
///# use tokio_rustls::TlsAcceptor;
///# struct NoopInputHandler;
///# #[async_trait::async_trait]
///# impl RdpServerInputHandler for NoopInputHandler {
///#     async fn keyboard(&mut self, _: KeyboardEvent) {}
///#     async fn mouse(&mut self, _: MouseEvent) {}
///# }
///# struct NoopDisplay;
///# #[async_trait::async_trait]
///# impl RdpServerDisplay for NoopDisplay {
///#     async fn size(&mut self) -> DesktopSize {
///#         todo!()
///#     }
///#     async fn get_update(&mut self) -> Option<DisplayUpdate> {
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
    handler: Box<dyn RdpServerInputHandler>,
    display: Box<dyn RdpServerDisplay>,
}

impl RdpServer {
    pub fn new(
        opts: RdpServerOptions,
        handler: Box<dyn RdpServerInputHandler>,
        display: Box<dyn RdpServerDisplay>,
    ) -> Self {
        Self { opts, handler, display }
    }

    pub fn builder() -> builder::RdpServerBuilder<builder::WantsAddr> {
        builder::RdpServerBuilder::new()
    }

    pub async fn run(&mut self) -> Result<()> {
        let listener = TcpListener::bind(self.opts.addr).await?;

        debug!("Listening for connections");
        while let Ok((stream, peer)) = listener.accept().await {
            debug!(?peer, "Received connection");
            let framed = TokioFramed::new(stream);

            let size = self.display.size().await;
            let capabilities = capabilities::capabilities(&self.opts, size.clone());
            let mut acceptor = Acceptor::new(self.opts.security.flag(), size, capabilities);

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
        }

        Ok(())
    }

    async fn client_loop<S>(&mut self, mut framed: Framed<S>, result: AcceptorResult) -> Result<()>
    where
        S: FramedWrite + FramedRead,
    {
        let mut buffer = vec![0u8; 4096];
        let mut encoder = UpdateEncoder::new();

        if !result.input_events.is_empty() {
            debug!("Handling input event backlog from acceptor sequence");
            self.handle_input_backlog(result.input_events).await?;
        }

        debug!("Starting client loop");

        'main: loop {
            tokio::select! {
                frame = framed.read_pdu() => {
                    let Ok((action, bytes)) = frame else {
                        break;
                    };

                    match action {
                        Action::FastPath => {
                            let input = FastPathInput::from_buffer(Cursor::new(&bytes))?;
                            self.handle_fastpath(input).await;
                        }

                        Action::X224 => {
                            match self.handle_x224(&bytes).await {
                                Ok(disconnect) => {
                                    if disconnect {
                                        break 'main;
                                    }
                                },

                                Err(error) => {
                                    error!(?error, "X224 input error");
                                }
                            };
                        }
                    }
                },

                Some(update) = self.display.get_update() => {
                    let fragmenter = match update {
                        DisplayUpdate::Bitmap(bitmap) => encoder.bitmap(bitmap)
                    };

                    if let Some(mut fragmenter) = fragmenter {
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
        }

        Ok(())
    }

    async fn handle_input_backlog(&mut self, frames: Vec<Vec<u8>>) -> Result<()> {
        for frame in frames {
            match Action::from_fp_output_header(frame[0]) {
                Ok(Action::FastPath) => {
                    let input = FastPathInput::from_buffer(Cursor::new(&frame))?;
                    self.handle_fastpath(input).await;
                }

                Ok(Action::X224) => {
                    let _ = self.handle_x224(&frame).await;
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
            match event {
                FastPathInputEvent::KeyboardEvent(flags, key) => {
                    self.handler.keyboard((key, flags).into()).await;
                }

                FastPathInputEvent::UnicodeKeyboardEvent(flags, key) => {
                    self.handler.keyboard((key, flags).into()).await;
                }

                FastPathInputEvent::SyncEvent(flags) => {
                    self.handler.keyboard(flags.into()).await;
                }

                FastPathInputEvent::MouseEvent(mouse) => {
                    self.handler.mouse(mouse.into()).await;
                }

                FastPathInputEvent::MouseEventEx(mouse) => {
                    self.handler.mouse(mouse.into()).await;
                }

                FastPathInputEvent::QoeEvent(quality) => {
                    warn!("Received QoE: {}", quality);
                }
            }
        }
    }

    async fn handle_x224(&mut self, frame: &[u8]) -> Result<bool> {
        let message = ironrdp_pdu::decode::<mcs::McsMessage<'_>>(frame)?;
        match message {
            mcs::McsMessage::SendDataRequest(data) => {
                let control = rdp::headers::ShareControlHeader::from_buffer(Cursor::new(data.user_data))?;

                match control.share_control_pdu {
                    rdp::headers::ShareControlPdu::Data(header) => match header.share_data_pdu {
                        rdp::headers::ShareDataPdu::Input(pdu) => {
                            self.handle_input_event(pdu).await;
                        }

                        unexpected => {
                            warn!(?unexpected, "Unexpected share data pdu");
                        }
                    },

                    unexpected => {
                        warn!(?unexpected, "Unexpected share control");
                    }
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
            match event {
                ironrdp_pdu::input::InputEvent::ScanCode(key) => {
                    self.handler.keyboard((key.key_code, key.flags).into()).await;
                }

                ironrdp_pdu::input::InputEvent::Unicode(key) => {
                    self.handler.keyboard((key.unicode_code, key.flags).into()).await;
                }

                ironrdp_pdu::input::InputEvent::Sync(sync) => {
                    self.handler.keyboard(sync.flags.into()).await;
                }

                ironrdp_pdu::input::InputEvent::Mouse(mouse) => {
                    self.handler.mouse(mouse.into()).await;
                }

                ironrdp_pdu::input::InputEvent::MouseX(mouse) => {
                    self.handler.mouse(mouse.into()).await;
                }

                ironrdp_pdu::input::InputEvent::Unused(_) => {}
            }
        }
    }
}
