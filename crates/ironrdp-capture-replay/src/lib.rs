//! Offline analysis support for direct TCP RDP captures.

mod error;
mod negotiation;
mod routing;
mod tls;
mod transport;

pub use error::ReplayError;
pub use negotiation::{NegotiatedState, StaticChannel, recover_negotiated_state};
pub use routing::{
    CapturedActivation, CapturedDynamicChannel, ReplayDirection, ReplayEvent, ReplayGap, ReplayGapKind, ReplayReport,
    ReplayRoute, ReplayRouter, replay_capture,
};
pub use tls::{Plaintext, decrypt_tls};
pub use transport::{Capture, Endpoint, Flow, PacketStream, TlsKeyLog, read_capture};
