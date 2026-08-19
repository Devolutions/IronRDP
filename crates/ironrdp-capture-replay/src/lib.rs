//! Offline analysis support for direct TCP RDP captures.

mod error;
mod gateway;
pub(crate) mod gateway_rpch;
mod negotiation;
mod output;
mod routing;
mod tls;
mod transport;

pub use error::ReplayError;
pub use gateway::{extract_tunneled_rdp, is_gateway_tunnel};
pub use negotiation::{NegotiatedState, StaticChannel, recover_negotiated_state};
pub use output::{ExportError, ExportOptions, ExportSummary, export_capture};
pub use routing::{
    CapturedActivation, CapturedDynamicChannel, ReplayDirection, ReplayEvent, ReplayGap, ReplayGapKind, ReplayReport,
    ReplayRoute, ReplayRouter, replay_capture,
};
pub use tls::{Plaintext, decrypt_tls, decrypt_tls_streams};
pub use transport::{Capture, Endpoint, Flow, PacketStream, TlsKeyLog, read_capture};
