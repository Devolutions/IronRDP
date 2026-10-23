//! Offline replay support for direct TCP RDP captures.

#![expect(
    unused_crate_dependencies,
    reason = "the replay binary uses zeroize while the library owns the package manifest"
)]

mod error;
mod negotiation;
mod output;
mod routing;

pub use error::ReplayError;
pub use negotiation::{NegotiatedState, StaticChannel, recover_negotiated_state};
pub use output::{ExportError, ExportOptions, ExportSummary, export_capture};
pub use routing::{
    CapturedActivation, CapturedDynamicChannel, ReplayDirection, ReplayEvent, ReplayGap, ReplayGapKind, ReplayReport,
    ReplayRoute, ReplayRouter, replay_capture,
};
