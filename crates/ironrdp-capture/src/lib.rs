//! Offline decoding support for RDP packet captures.

mod error;
mod gateway;
mod gateway_rpch;
mod tls;
mod transport;

pub use error::CaptureError;
pub use gateway::{extract_tunneled_rdp, is_gateway_tunnel};
pub use gateway_rpch::{extract_rpch_from_flows, extract_rpch_tunneled_rdp};
#[doc(hidden)]
pub use tls::decrypt_tls_streams;
pub use tls::{Plaintext, decrypt_tls};
pub use transport::{Capture, Endpoint, Flow, PacketStream, TlsKeyLog, read_capture};
