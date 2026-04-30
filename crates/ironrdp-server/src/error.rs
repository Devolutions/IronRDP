//! Typed error types for the public API of the `ironrdp-server` crate.
//!
//! Mirrors the shape of [`ironrdp_connector::ConnectorError`]: a thin
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

use ironrdp_core::{DecodeError, EncodeError};

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
    Channel(&'static str),
    /// A feature requested by the client is not supported by this server.
    Unsupported(&'static str),
    /// Generic failure with a runtime description. Prefer a specific variant.
    Reason(String),
    /// Catch-all with no specific cause. Prefer a specific variant.
    General,
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
            Self::Channel(name) => write!(f, "channel error: {name}"),
            Self::Unsupported(feature) => write!(f, "unsupported: {feature}"),
            Self::Reason(reason) => write!(f, "reason: {reason}"),
            Self::General => write!(f, "general error"),
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
            Self::Channel(_) | Self::Unsupported(_) | Self::Reason(_) | Self::General | Self::Custom => None,
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
/// [`ironrdp_connector::ConnectorErrorExt`].
pub trait ServerErrorExt {
    /// Build a [`ServerErrorKind::Encode`] error from an [`EncodeError`].
    fn encode(error: EncodeError) -> Self;
    /// Build a [`ServerErrorKind::Decode`] error from a [`DecodeError`].
    fn decode(error: DecodeError) -> Self;
    /// Build a [`ServerErrorKind::Io`] error with a static context and an
    /// [`io::Error`] source.
    fn io(context: &'static str, error: io::Error) -> Self;
    /// Build a [`ServerErrorKind::Channel`] error tagged with the channel
    /// name.
    fn channel(context: &'static str) -> Self;
    /// Build a [`ServerErrorKind::Unsupported`] error.
    fn unsupported(context: &'static str) -> Self;
    /// Build a [`ServerErrorKind::General`] error with a static context.
    fn general(context: &'static str) -> Self;
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
    fn encode(error: EncodeError) -> Self {
        Self::new("encode error", ServerErrorKind::Encode(error))
    }

    fn decode(error: DecodeError) -> Self {
        Self::new("decode error", ServerErrorKind::Decode(error))
    }

    fn io(context: &'static str, error: io::Error) -> Self {
        Self::new(context, ServerErrorKind::Io(error))
    }

    fn channel(context: &'static str) -> Self {
        Self::new(context, ServerErrorKind::Channel(context))
    }

    fn unsupported(context: &'static str) -> Self {
        Self::new(context, ServerErrorKind::Unsupported(context))
    }

    fn general(context: &'static str) -> Self {
        Self::new(context, ServerErrorKind::General)
    }

    fn reason(context: &'static str, reason: impl Into<String>) -> Self {
        Self::new(context, ServerErrorKind::Reason(reason.into()))
    }

    fn custom<E>(context: &'static str, error: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static,
    {
        Self::new(context, ServerErrorKind::Custom).with_source(error)
    }
}

/// Result-side helpers mirroring [`ironrdp_connector::ConnectorResultExt`].
pub trait ServerResultExt {
    /// Replace the `&'static str` context on any error in `Self`.
    #[must_use]
    fn with_context(self, context: &'static str) -> Self;
    /// Attach a source to any error in `Self`.
    #[must_use]
    fn with_source<E>(self, source: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static;
}

impl<T> ServerResultExt for ServerResult<T> {
    fn with_context(self, context: &'static str) -> Self {
        self.map_err(|mut e| {
            e.set_context(context);
            e
        })
    }

    fn with_source<E>(self, source: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static,
    {
        self.map_err(|e| e.with_source(source))
    }
}
