use std::io::Cursor;
use std::net::SocketAddr;

use anyhow::Result;
use bytes::BytesMut;
use ironrdp_acceptor::{self, Acceptor, BeginResult};
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
/// ```ignore
/// use ironrdp_server::{RdpServer, RdpServerInputHandler, RdpServerDisplay};
///
/// async fn main() {
///     let tls_acceptor = todo!();
///     let input_handler = todo!();
///     let display_handler = todo!();
///
///     let mut server = RdpServer::builder()
///         .with_addr(([127, 0, 0, 1], 3389))
///         .with_tls(tls_acceptor)
///         .with_input_handler(input_handler)
///         .with_display_handler(display_handler)
///         .build();
///
///     server.run().await;
/// }
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

        while let Ok((stream, peer)) = listener.accept().await {
            debug!(?peer, "received connection");
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
                    let (framed, _) = ironrdp_acceptor::accept_finalize(framed, &mut acceptor).await?;
                    self.client_loop(framed).await?;
                }

                Ok(BeginResult::Continue(framed)) => {
                    let (framed, _) = ironrdp_acceptor::accept_finalize(framed, &mut acceptor).await?;
                    self.client_loop(framed).await?;
                }

                Err(e) => {
                    error!("connection error: {:?}", e);
                }
            }
        }

        Ok(())
    }

    async fn client_loop<S>(&mut self, mut framed: Framed<S>) -> Result<()>
    where
        S: FramedWrite + FramedRead,
    {
        let mut buffer = vec![0u8; 4096];
        let mut encoder = UpdateEncoder::new();

        debug!("starting client loop");

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
                            match self.handle_x224(bytes).await {
                                Ok(disconnect) => {
                                    if disconnect {
                                        break 'main;
                                    }
                                },

                                Err(e) => {
                                    error!("x224 input error: {:?}", e);
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
                            if let Err(e) = framed.write_all(&buffer[..len]).await {
                                error!("write error: {:?}", e);
                                break;
                            };
                        }
                    }
                }
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
                    warn!("received QoE: {}", quality);
                }
            }
        }
    }

    async fn handle_x224(&mut self, frame: BytesMut) -> Result<bool> {
        let message = ironrdp_pdu::decode::<mcs::McsMessage>(&frame)?;
        match message {
            mcs::McsMessage::SendDataRequest(data) => {
                let control = rdp::headers::ShareControlHeader::from_buffer(Cursor::new(data.user_data))?;

                match control.share_control_pdu {
                    rdp::headers::ShareControlPdu::Data(header) => match header.share_data_pdu {
                        rdp::headers::ShareDataPdu::Input(pdu) => {
                            self.handle_input_event(pdu).await;
                        }

                        unexpected => {
                            warn!("unexpected share data pdu {:?}", unexpected);
                        }
                    },

                    unexpected => {
                        warn!("unexpected share control {:?}", unexpected);
                    }
                }
            }

            mcs::McsMessage::DisconnectProviderUltimatum(disconnect) => {
                if disconnect.reason == mcs::DisconnectReason::UserRequested {
                    return Ok(true);
                }
            }

            unexpected => {
                warn!("unexpected mcs message {:?}", ironrdp_pdu::name(&unexpected));
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
