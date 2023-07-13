use std::{io::Cursor, net::SocketAddr};

use anyhow::{Error, Result};
use bytes::BytesMut;
use ironrdp_pdu::{
    input::fast_path::{FastPathInput, FastPathInputEvent},
    Action, PduParsing,
};
use ironrdp_tokio::{Framed, FramedRead, FramedWrite, TokioFramed};
use tokio::{net::TcpListener, select};
use tokio_rustls::TlsAcceptor;

use crate::DisplayUpdate;

use super::{
    acceptor::{self, BeginResult, ServerAcceptor},
    builder,
    display::RdpServerDisplay,
    encoder::{bitmap::UncompressedBitmapHandler, UpdateEncoder},
    handler::RdpServerInputHandler,
};

pub struct RdpServer<H, D> {
    opts: RdpServerOptions,
    handler: H,
    display: D,
}

#[derive(Clone)]
pub struct RdpServerOptions {
    pub addr: SocketAddr,
    pub security: RdpServerSecurity,
}

#[derive(Clone)]
pub enum RdpServerSecurity {
    None,
    SSL(TlsAcceptor),
}

impl<H, D> RdpServer<H, D>
where
    H: RdpServerInputHandler,
    D: RdpServerDisplay,
{
    pub fn new(opts: RdpServerOptions, handler: H, display: D) -> Self {
        Self { opts, handler, display }
    }

    pub fn builder() -> builder::RdpServerBuilder<builder::WantsAddr, H, D> {
        builder::RdpServerBuilder::new()
    }

    pub async fn run(&mut self) -> Result<(), Error> {
        let listener = TcpListener::bind(self.opts.addr).await?;

        while let Ok((stream, _)) = listener.accept().await {
            let size = self.display.size().await;
            let mut acceptor = ServerAcceptor::new(self.opts.clone(), size);

            match acceptor::accept_begin(stream, &mut acceptor).await {
                Ok(BeginResult::ShouldUpgrade(stream)) => {
                    let upgraded = acceptor::upgrade(&self.opts.security, stream).await?;
                    let framed = TokioFramed::new(upgraded);
                    let framed = acceptor::accept_finalize(framed, &mut acceptor).await?;
                    self.client_loop(framed).await?;
                }

                Ok(BeginResult::Continue(framed)) => {
                    let framed = acceptor::accept_finalize(framed, &mut acceptor).await?;
                    self.client_loop(framed).await?;
                }

                Err(e) => {
                    eprintln!("connection error: {:?}", e);
                }
            }
        }

        Ok(())
    }

    async fn client_loop<S>(&mut self, mut framed: Framed<S>) -> Result<()>
    where
        S: FramedWrite + FramedRead,
    {
        let mut buffer = vec![0u8; 8192 * 8192];
        let mut encoder = UpdateEncoder::new(UncompressedBitmapHandler {});

        loop {
            select! {
                frame = framed.read_pdu() => {
                    let Ok((action, bytes)) = frame else {
                        break;
                    };

                    match action {
                        Action::FastPath => {
                            let input = FastPathInput::from_buffer(Cursor::new(&bytes))?;
                            self.handle_fastpath(input).await;
                        },

                        Action::X224 => {
                            if let Err(e) = self.handle_x224(bytes).await {
                                eprintln!("x224 input error: {:?}", e);
                            }
                        },
                    }
                },

                Some(update) = self.display.get_update() => {
                    match update {
                        DisplayUpdate::Bitmap(bitmap) => {
                            if let Some(len) = encoder.encode(bitmap, &mut buffer) {
                                if let Err(e) = framed.write_all(&buffer[..len]).await {
                                    eprintln!("write error: {:?}", e);
                                    break;
                                };
                            };
                        },
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
                    self.handler.keyboard((key as u16, flags).into()).await;
                }

                FastPathInputEvent::UnicodeKeyboardEvent(flags, key) => {
                    self.handler.keyboard((key, flags).into()).await;
                }

                FastPathInputEvent::MouseEvent(mouse) => {
                    self.handler.mouse(mouse.into()).await;
                }

                FastPathInputEvent::MouseEventEx(mouse) => {
                    self.handler.mouse(mouse.into()).await;
                }

                other => println!("{other:?}"),
            }
        }
    }

    async fn handle_x224(&mut self, _frame: BytesMut) -> Result<()> {
        unimplemented!()
    }
}
