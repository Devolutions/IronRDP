//! Typed error types for the public API of the `ironrdp-server` crate.
//!
//! Mirrors the shape of `ironrdp_connector::ConnectorError`: a thin
//! [`ironrdp_error::Error`] wrapper around a typed [`ServerErrorKind`] enum,
//! with a static `&'static str` context and an opaque `source` for arbitrary
//! upstream errors. The wrapper provides `with_source` so concrete errors
//! from consumer-supplied components can be attached without forcing the
//! variant taxonomy to encode every possible source type.
//!
//! See [#1209] for the migration discussion.
//!
//! [#1209]: https://github.com/Devolutions/IronRDP/issues/1209

use core::fmt;
use std::io;

use ironrdp_connector::ConnectorError;
use ironrdp_core::{DecodeError, EncodeError};
use ironrdp_pdu::PduError;

/// Categorizes the failure modes the server crate exposes through
/// [`ServerError`].
///
/// Marked `#[non_exhaustive]` so additional variants do not constitute a
/// breaking change.
#[non_exhaustive]
#[derive(Debug)]
pub enum ServerErrorKind {
    /// PDU encoding failed.
    Encode(EncodeError),
    /// PDU decoding failed.
    Decode(DecodeError),
    /// I/O error during TLS setup, listener setup, or client communication.
    Io(io::Error),
    /// A required virtual channel was missing or a channel send failed.
    /// The specific channel and failure mode is named in the [`ServerError`]
    /// context.
    Channel,
    /// A feature requested by the client is not supported by this server.
    /// The specific feature is named in the [`ServerError`] context.
    Unsupported,
    /// The RDP connection sequence (acceptor handshake) failed.
    Connector(ConnectorError),
    /// A static virtual channel failed to encode or decode a PDU.
    Pdu(PduError),
    /// Generic failure with a runtime description. Prefer a specific variant.
    Reason(String),
    /// Custom failure with the actual source attached via
    /// [`ironrdp_error::Error::with_source`].
    Custom,
}

impl fmt::Display for ServerErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode(_) => write!(f, "encode error"),
            Self::Decode(_) => write!(f, "decode error"),
            Self::Io(_) => write!(f, "I/O error"),
            Self::Channel => write!(f, "channel error"),
            Self::Unsupported => write!(f, "unsupported"),
            Self::Connector(_) => write!(f, "connector error"),
            Self::Pdu(_) => write!(f, "PDU error"),
            Self::Reason(reason) => write!(f, "reason: {reason}"),
            Self::Custom => write!(f, "custom error"),
        }
    }
}

impl core::error::Error for ServerErrorKind {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Encode(e) => Some(e),
            Self::Decode(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::Connector(e) => Some(e),
            Self::Pdu(e) => Some(e),
            Self::Channel | Self::Unsupported | Self::Reason(_) | Self::Custom => None,
        }
    }
}

/// Server-side failure type.
///
/// A typed alias of [`ironrdp_error::Error`] specialized to
/// [`ServerErrorKind`]. The wrapper adds a static `&'static str` context and
/// an optional opaque `source` to whichever kind of failure occurred.
pub type ServerError = ironrdp_error::Error<ServerErrorKind>;

/// Convenience alias for `Result<T, ServerError>`.
pub type ServerResult<T> = Result<T, ServerError>;

/// Constructors for [`ServerError`] that match the shape of
/// `ironrdp_connector::ConnectorErrorExt`.
pub trait ServerErrorExt {
    /// Build a [`ServerErrorKind::Encode`] error from an [`EncodeError`].
    fn encode(error: EncodeError) -> Self;
    /// Build a [`ServerErrorKind::Decode`] error from a [`DecodeError`].
    fn decode(error: DecodeError) -> Self;
    /// Build a [`ServerErrorKind::Io`] error with a static context and an
    /// [`io::Error`] source.
    fn io(context: &'static str, error: io::Error) -> Self;
    /// Build a [`ServerErrorKind::Channel`] error with the channel name or
    /// failure description carried in the context.
    fn channel(context: &'static str) -> Self;
    /// Build a [`ServerErrorKind::Unsupported`] error with the unsupported
    /// feature named in the context.
    fn unsupported(context: &'static str) -> Self;
    /// Build a [`ServerErrorKind::Reason`] error with a static context and a
    /// runtime description.
    fn reason(context: &'static str, reason: impl Into<String>) -> Self;
    /// Build a [`ServerErrorKind::Custom`] error with a static context and an
    /// arbitrary source.
    fn custom<E>(context: &'static str, error: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static;
}

impl ServerErrorExt for ServerError {
    #[track_caller]
    fn encode(error: EncodeError) -> Self {
        Self::new("encode error", ServerErrorKind::Encode(error))
    }

    #[track_caller]
    fn decode(error: DecodeError) -> Self {
        Self::new("decode error", ServerErrorKind::Decode(error))
    }

    #[track_caller]
    fn io(context: &'static str, error: io::Error) -> Self {
        Self::new(context, ServerErrorKind::Io(error))
    }

    #[track_caller]
    fn channel(context: &'static str) -> Self {
        Self::new(context, ServerErrorKind::Channel)
    }

    #[track_caller]
    fn unsupported(context: &'static str) -> Self {
        Self::new(context, ServerErrorKind::Unsupported)
    }

    #[track_caller]
    fn reason(context: &'static str, reason: impl Into<String>) -> Self {
        Self::new(context, ServerErrorKind::Reason(reason.into()))
    }

    #[track_caller]
    fn custom<E>(context: &'static str, error: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static,
    {
        Self::new(context, ServerErrorKind::Custom).with_source(error)
    }
}

/// Result-side helpers mirroring `ironrdp_connector::ConnectorResultExt`.
pub trait ServerResultExt {
    /// Replace the `&'static str` context on any error in `Self`.
    #[must_use]
    fn with_context(self, context: &'static str) -> Self;
}

impl<T> ServerResultExt for ServerResult<T> {
    fn with_context(self, context: &'static str) -> Self {
        self.map_err(|mut e| {
            e.set_context(context);
            e
        })
    }
}

/// Bridges an anyhow error at the public API boundary while the migration to
/// typed errors is staged, tagging it with `context` naming the operation
/// that failed.
///
/// Internal call sites still use `anyhow::Result`; conversion happens here so
/// the public signatures can advertise [`ServerResult`] today without forcing
/// every internal site to convert in this PR. Later steps of the migration
/// (see [#1209]) remove the remaining `anyhow` usage and this helper.
///
/// [#1209]: https://github.com/Devolutions/IronRDP/issues/1209
#[track_caller]
pub(crate) fn from_anyhow_with_context(error: anyhow::Error, context: &'static str) -> ServerError {
    ServerError::new(context, ServerErrorKind::Custom).with_source(AnyhowError(error))
}

/// Newtype wrapper that gives [`anyhow::Error`] a `core::error::Error` impl
/// suitable for `ironrdp_error::Error::with_source`.
#[derive(Debug)]
struct AnyhowError(anyhow::Error);

impl fmt::Display for AnyhowError {
    /// Prints only the outermost context. Anyhow's alternate form (`{:#}`)
    /// would flatten the whole cause chain here, and since [`Self::source`]
    /// exposes that same chain, every level below the outermost would then be
    /// printed twice by `ErrorReport`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl core::error::Error for AnyhowError {
    /// Forwards to the wrapped [`anyhow::Error`]'s cause chain so callers
    /// traversing [`core::error::Error::source`] reach the original root
    /// cause rather than stopping at this newtype.
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.0.source()
    }
}
