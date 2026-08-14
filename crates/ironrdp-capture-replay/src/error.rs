use std::io;

use thiserror::Error;

/// Errors that make an offline replay unreliable.
#[derive(Debug, Error)]
pub enum ReplayError {
    /// The capture could not be read.
    #[error("failed to read capture")]
    Io(#[source] io::Error),
    /// The capture could not be parsed.
    #[error("invalid pcapng capture: {0}")]
    Pcap(String),
    /// The capture does not contain a supported direct TCP flow.
    #[error("capture does not contain an Ethernet direct TCP flow")]
    UnsupportedTransport,
    /// The capture has incomplete or contradictory TCP data.
    #[error("capture does not contain a complete client/server TCP flow")]
    MissingTcpFlow,
}
