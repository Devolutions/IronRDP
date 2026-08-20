//! Error type for the gateway forwarding crate.

use core::fmt;

/// Result alias for gateway forwarding operations.
pub type Result<T> = core::result::Result<T, ForwardError>;

/// Error kind for gateway forwarding and SOCKS5 operations.
#[derive(Debug)]
#[non_exhaustive]
pub enum ForwardErrorKind {
    /// The gateway tunnel could not be opened or was lost.
    Tunnel,
    /// A local listener could not be bound or accepted a connection.
    Listener,
    /// A SOCKS5 request was malformed or used an unsupported feature.
    Socks5,
    /// The target address could not be parsed or resolved.
    Address,
    /// A generic I/O failure while relaying bytes.
    Io,
}

impl fmt::Display for ForwardErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tunnel => f.write_str("gateway tunnel error"),
            Self::Listener => f.write_str("listener error"),
            Self::Socks5 => f.write_str("socks5 protocol error"),
            Self::Address => f.write_str("address error"),
            Self::Io => f.write_str("io error"),
        }
    }
}

impl core::error::Error for ForwardErrorKind {}

/// Error for gateway forwarding operations.
pub type ForwardError = ironrdp_error::Error<ForwardErrorKind>;
