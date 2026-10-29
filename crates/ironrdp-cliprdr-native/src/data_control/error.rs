//! Errors returned by the clipboard client.

use core::fmt;

/// Errors returned by [`DataControl`](super::DataControl).
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The connection to the Wayland compositor could not be made.
    Connect(String),
    /// The compositor offers neither data-control protocol.
    Unsupported,
    /// The requested protocol is not offered and fallback is disabled.
    ProtocolUnavailable,
    /// The compositor advertises no seat to attach the clipboard to.
    NoSeat,
    /// The compositor reported a protocol error or the connection failed.
    Wayland(String),
    /// The worker thread has stopped, so the request could not be delivered.
    Stopped,
    /// Clipboard data exceeded the size limit.
    TooLarge {
        /// Bytes received when the limit was hit.
        size: usize,
        /// The limit.
        limit: usize,
    },
    /// The source did not deliver its data in time.
    Timeout,
    /// The selection changed while it was being read, so the data is missing or belongs to
    /// another selection.
    SelectionChanged,
    /// An I/O error while creating or reading a pipe.
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(reason) => write!(f, "cannot connect to the Wayland compositor: {reason}"),
            Self::Unsupported => {
                f.write_str("the compositor offers neither ext-data-control-v1 nor wlr-data-control-unstable-v1")
            }
            Self::ProtocolUnavailable => {
                f.write_str("the requested data-control protocol is not offered by the compositor")
            }
            Self::NoSeat => f.write_str("the compositor advertises no seat"),
            Self::Wayland(reason) => write!(f, "Wayland error: {reason}"),
            Self::Stopped => f.write_str("the clipboard worker has stopped"),
            Self::TooLarge { size, limit } => {
                write!(f, "clipboard data of {size} bytes exceeds the {limit} byte limit")
            }
            Self::Timeout => f.write_str("the clipboard source did not deliver its data in time"),
            Self::SelectionChanged => f.write_str("the clipboard selection changed while it was being read"),
            Self::Io(error) => write!(f, "I/O error: {error}"),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Result alias for this module.
pub type Result<T> = core::result::Result<T, Error>;
