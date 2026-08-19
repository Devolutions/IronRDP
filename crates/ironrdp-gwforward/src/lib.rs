//! Generic TCP forwarding and SOCKS5 proxying over an MS-TSGU RD Gateway tunnel.
//!
//! The RD Gateway tunneling protocol ([MS-TSGU]) relays a TCP byte stream to any
//! reachable target host:port; it is not limited to RDP. This crate uses that to expose
//! two local entry points that generic programs can use to traverse an RD Gateway:
//!
//! - [`run_port_forward`]: a fixed local forward (SSH `-L`-style) to one target.
//! - [`run_socks5`]: a SOCKS5 proxy that opens a tunnel per requested destination.
//!
//! Each inbound connection opens an independent gateway tunnel via
//! [`ironrdp_mstsgu::GwClient`] and relays bytes bidirectionally.

mod error;
mod forward;
mod socks5;
mod tunnel;

pub use error::{ForwardError, ForwardErrorKind, Result};
pub use forward::{run_port_forward, run_socks5};
pub use tunnel::{GatewayTransport, GatewayTunnelConfig, TunnelStream, open_tunnel};
