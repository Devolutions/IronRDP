#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![forbid(unsafe_code)]

pub mod error;
pub mod framed;
pub mod multitransport;
pub mod transport;

pub(crate) mod clock;
pub(crate) mod driver;
pub(crate) mod stream;
pub(crate) mod tls;
pub(crate) mod tunnel;

pub use self::error::{DriverError, DriverErrorKind, UdpTransportError, UdpTransportErrorKind};
pub use self::multitransport::MultitransportBootstrap;
pub use self::transport::{UdpAcceptConfig, UdpTransport, UdpTransportConfig, accept_udp, connect_udp};
