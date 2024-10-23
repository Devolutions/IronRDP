use std::net::SocketAddr;

use anyhow::Result;
use tokio_rustls::TlsAcceptor;

use super::clipboard::CliprdrServerFactory;
use super::display::{DesktopSize, RdpServerDisplay};
use super::handler::{KeyboardEvent, MouseEvent, RdpServerInputHandler};
use super::server::*;
use crate::{DisplayUpdate, RdpServerDisplayUpdates, SoundServerFactory};

pub struct WantsAddr {}
pub struct WantsSecurity {
    addr: SocketAddr,
}
pub struct WantsHandler {
    addr: SocketAddr,
    security: RdpServerSecurity,
}
pub struct WantsDisplay {
    addr: SocketAddr,
    security: RdpServerSecurity,
    handler: Box<dyn RdpServerInputHandler>,
}
pub struct BuilderDone {
    addr: SocketAddr,
    security: RdpServerSecurity,
    handler: Box<dyn RdpServerInputHandler>,
    display: Box<dyn RdpServerDisplay>,
    cliprdr_factory: Option<Box<dyn CliprdrServerFactory>>,
    sound_factory: Option<Box<dyn SoundServerFactory>>,
}

pub struct RdpServerBuilder<State> {
    state: State,
}

impl RdpServerBuilder<WantsAddr> {
    pub fn new() -> Self {
        Self { state: WantsAddr {} }
    }
}

impl Default for RdpServerBuilder<WantsAddr> {
    fn default() -> Self {
        Self::new()
    }
}

impl RdpServerBuilder<WantsAddr> {
    #[allow(clippy::unused_self)] // ensuring state transition from WantsAddr
    pub fn with_addr(self, addr: impl Into<SocketAddr>) -> RdpServerBuilder<WantsSecurity> {
        RdpServerBuilder {
            state: WantsSecurity { addr: addr.into() },
        }
    }
}

impl RdpServerBuilder<WantsSecurity> {
    pub fn with_no_security(self) -> RdpServerBuilder<WantsHandler> {
        RdpServerBuilder {
            state: WantsHandler {
                addr: self.state.addr,
                security: RdpServerSecurity::None,
            },
        }
    }

    pub fn with_tls(self, acceptor: impl Into<TlsAcceptor>) -> RdpServerBuilder<WantsHandler> {
        RdpServerBuilder {
            state: WantsHandler {
                addr: self.state.addr,
                security: RdpServerSecurity::Tls(acceptor.into()),
            },
        }
    }

    pub fn with_hybrid(self, acceptor: impl Into<TlsAcceptor>, pub_key: Vec<u8>) -> RdpServerBuilder<WantsHandler> {
        RdpServerBuilder {
            state: WantsHandler {
                addr: self.state.addr,
                security: RdpServerSecurity::Hybrid((acceptor.into(), pub_key)),
            },
        }
    }
}

impl RdpServerBuilder<WantsHandler> {
    pub fn with_input_handler<H>(self, handler: H) -> RdpServerBuilder<WantsDisplay>
    where
        H: RdpServerInputHandler + 'static,
    {
        RdpServerBuilder {
            state: WantsDisplay {
                addr: self.state.addr,
                security: self.state.security,
                handler: Box::new(handler),
            },
        }
    }

    pub fn with_no_input(self) -> RdpServerBuilder<WantsDisplay> {
        RdpServerBuilder {
            state: WantsDisplay {
                addr: self.state.addr,
                security: self.state.security,
                handler: Box::new(NoopInputHandler),
            },
        }
    }
}

impl RdpServerBuilder<WantsDisplay> {
    pub fn with_display_handler<D>(self, display: D) -> RdpServerBuilder<BuilderDone>
    where
        D: RdpServerDisplay + 'static,
    {
        RdpServerBuilder {
            state: BuilderDone {
                addr: self.state.addr,
                security: self.state.security,
                handler: self.state.handler,
                display: Box::new(display),
                sound_factory: None,
                cliprdr_factory: None,
            },
        }
    }

    pub fn with_no_display(self) -> RdpServerBuilder<BuilderDone> {
        RdpServerBuilder {
            state: BuilderDone {
                addr: self.state.addr,
                security: self.state.security,
                handler: self.state.handler,
                display: Box::new(NoopDisplay),
                sound_factory: None,
                cliprdr_factory: None,
            },
        }
    }
}

impl RdpServerBuilder<BuilderDone> {
    pub fn with_cliprdr_factory(mut self, cliprdr_factory: Option<Box<dyn CliprdrServerFactory>>) -> Self {
        self.state.cliprdr_factory = cliprdr_factory;
        self
    }

    pub fn with_sound_factory(mut self, sound: Option<Box<dyn SoundServerFactory>>) -> Self {
        self.state.sound_factory = sound;
        self
    }

    pub fn build(self) -> RdpServer {
        RdpServer::new(
            RdpServerOptions {
                addr: self.state.addr,
                security: self.state.security,
            },
            self.state.handler,
            self.state.display,
            self.state.sound_factory,
            self.state.cliprdr_factory,
        )
    }
}

struct NoopInputHandler;

impl RdpServerInputHandler for NoopInputHandler {
    fn keyboard(&mut self, _: KeyboardEvent) {}
    fn mouse(&mut self, _: MouseEvent) {}
}

struct NoopDisplayUpdates;

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for NoopDisplayUpdates {
    async fn next_update(&mut self) -> Option<DisplayUpdate> {
        let () = std::future::pending().await;
        unreachable!()
    }
}

struct NoopDisplay;

#[async_trait::async_trait]
impl RdpServerDisplay for NoopDisplay {
    async fn size(&mut self) -> DesktopSize {
        DesktopSize { width: 0, height: 0 }
    }

    async fn updates(&mut self) -> Result<Box<dyn RdpServerDisplayUpdates>> {
        Ok(Box::new(NoopDisplayUpdates {}))
    }
}
