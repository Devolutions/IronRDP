use std::{marker::PhantomData, net::SocketAddr};
use tokio_rustls::TlsAcceptor;

use super::display::RdpServerDisplay;
use super::handler::RdpServerInputHandler;
use super::server::*;

pub struct WantsAddr {}
pub struct WantsSecurity {
    addr: SocketAddr,
}
pub struct WantsHandler {
    addr: SocketAddr,
    security: RdpServerSecurity,
}
pub struct WantsDisplay<H> {
    addr: SocketAddr,
    security: RdpServerSecurity,
    handler: H,
}
pub struct BuilderDone<H, D> {
    addr: SocketAddr,
    security: RdpServerSecurity,
    handler: H,
    display: D,
}

pub struct RdpServerBuilder<State, H, D> {
    state: State,
    _handler: PhantomData<H>,
    _display: PhantomData<D>,
}

impl<H, D> RdpServerBuilder<WantsAddr, H, D> {
    pub fn new() -> Self {
        Self {
            state: WantsAddr {},
            _handler: PhantomData,
            _display: PhantomData,
        }
    }
}

impl<H, D> Default for RdpServerBuilder<WantsAddr, H, D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H, D> RdpServerBuilder<WantsAddr, H, D> {
    pub fn with_addr<T: Into<SocketAddr>>(self, addr: T) -> RdpServerBuilder<WantsSecurity, H, D> {
        RdpServerBuilder {
            state: WantsSecurity { addr: addr.into() },
            _handler: PhantomData,
            _display: PhantomData,
        }
    }
}

impl<H, D> RdpServerBuilder<WantsSecurity, H, D> {
    pub fn with_no_security(self) -> RdpServerBuilder<WantsHandler, H, D> {
        RdpServerBuilder {
            state: WantsHandler {
                addr: self.state.addr,
                security: RdpServerSecurity::None,
            },
            _handler: PhantomData,
            _display: PhantomData,
        }
    }

    pub fn with_ssl<T: Into<TlsAcceptor>>(self, acceptor: T) -> RdpServerBuilder<WantsHandler, H, D> {
        RdpServerBuilder {
            state: WantsHandler {
                addr: self.state.addr,
                security: RdpServerSecurity::SSL(acceptor.into()),
            },
            _handler: PhantomData,
            _display: PhantomData,
        }
    }
}

impl<H, D> RdpServerBuilder<WantsHandler, H, D> {
    pub fn with_io_handler(self, handler: H) -> RdpServerBuilder<WantsDisplay<H>, H, D> {
        RdpServerBuilder {
            state: WantsDisplay {
                addr: self.state.addr,
                security: self.state.security,
                handler,
            },
            _handler: PhantomData,
            _display: PhantomData,
        }
    }
}

impl<H, D> RdpServerBuilder<WantsDisplay<H>, H, D> {
    pub fn with_display_handler(self, display: D) -> RdpServerBuilder<BuilderDone<H, D>, H, D> {
        RdpServerBuilder {
            state: BuilderDone {
                addr: self.state.addr,
                security: self.state.security,
                handler: self.state.handler,
                display,
            },
            _handler: PhantomData,
            _display: PhantomData,
        }
    }
}

impl<H, D> RdpServerBuilder<BuilderDone<H, D>, H, D>
where
    H: RdpServerInputHandler + 'static,
    D: RdpServerDisplay + 'static,
{
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
