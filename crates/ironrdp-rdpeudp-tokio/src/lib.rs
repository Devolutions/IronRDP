#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![forbid(unsafe_code)]

pub mod error;
pub mod framed;
#[cfg(feature = "ice")]
pub mod ice;
pub mod multitransport;
pub mod stun;
pub mod transport;

pub(crate) mod clock;
pub(crate) mod driver;
pub(crate) mod stream;
pub(crate) mod tls;
pub(crate) mod tunnel;

pub use self::error::{DriverError, DriverErrorKind, UdpTransportError, UdpTransportErrorKind};
#[cfg(feature = "ice")]
pub use self::ice::{
    IceCredentials, IceGatherConfig, IceGatheredCandidates, IceRole, IceServerConfig, IceTransportError,
    RemoteIceDescription, accept_udp_with_ice, connect_udp_with_ice, gather_ice_candidates,
};
pub use self::multitransport::MultitransportBootstrap;
pub use self::transport::{
    TunnelMessage, UdpAcceptConfig, UdpTlsConfig, UdpTransport, UdpTransportConfig, UdpTransportSender, accept_udp,
    connect_udp, connect_udp_with_socket,
};

#[cfg(test)]
use {rcgen as _, rstest as _, turn as _};
