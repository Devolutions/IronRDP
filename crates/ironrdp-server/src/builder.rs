use std::net::SocketAddr;

use tokio_rustls::TlsAcceptor;

use super::display::{DesktopSize, DisplayUpdate, RdpServerDisplay};
use super::handler::{KeyboardEvent, MouseEvent, RdpServerInputHandler};
use super::server::*;

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
            },
        }
    }
}

impl RdpServerBuilder<BuilderDone> {
    pub fn build(self) -> RdpServer {
        RdpServer::new(
            RdpServerOptions {
                addr: self.state.addr,
                security: self.state.security,
            },
            self.state.handler,
            self.state.display,
        )
    }
}

pub struct NoopInputHandler;

#[async_trait::async_trait]
impl RdpServerInputHandler for NoopInputHandler {
    async fn keyboard(&mut self, _: KeyboardEvent) {}
    async fn mouse(&mut self, _: MouseEvent) {}
}

pub struct NoopDisplay;

#[async_trait::async_trait]
impl RdpServerDisplay for NoopDisplay {
    async fn size(&mut self) -> DesktopSize {
        DesktopSize { width: 0, height: 0 }
    }
    async fn get_update(&mut self) -> Option<DisplayUpdate> {
        let () = std::future::pending().await;
        unreachable!()
    }
}
