//! Error type produced by [`Sequence`](crate::Sequence) implementors.
//!
//! `SequenceError` is the sspi-free error type returned while driving a single
//! PDU state machine (a [`Sequence`](crate::Sequence) impl and its helpers). It
//! never carries an `sspi::Error` and never needs to know about CredSSP or
//! access-denied semantics: those are connect-flow-level concerns owned by
//! `ironrdp-connector`'s `ConnectorError`, which nests a `SequenceError` in its
//! own `Sequence` variant at each connect boundary.

use core::fmt;

#[cfg(not(feature = "std"))]
use alloc::string::String;

use crate::negotiation_failure::NegotiationFailure;

pub type SequenceResult<T> = Result<T, SequenceError>;

#[non_exhaustive]
#[derive(Debug)]
pub enum SequenceErrorKind {
    Encode(ironrdp_core::EncodeError),
    Decode(ironrdp_core::DecodeError),
    Reason(String),
    General,
    Custom,
    Negotiation(NegotiationFailure),
}

impl fmt::Display for SequenceErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self {
            SequenceErrorKind::Encode(_) => write!(f, "encode error"),
            SequenceErrorKind::Decode(_) => write!(f, "decode error"),
            SequenceErrorKind::Reason(description) => write!(f, "reason: {description}"),
            SequenceErrorKind::General => write!(f, "general error"),
            SequenceErrorKind::Custom => write!(f, "custom error"),
            SequenceErrorKind::Negotiation(failure) => write!(f, "negotiation failure: {failure}"),
        }
    }
}

impl core::error::Error for SequenceErrorKind {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match &self {
            SequenceErrorKind::Encode(e) => Some(e),
            SequenceErrorKind::Decode(e) => Some(e),
            SequenceErrorKind::Reason(_) => None,
            SequenceErrorKind::General => None,
            SequenceErrorKind::Custom => None,
            SequenceErrorKind::Negotiation(failure) => Some(failure),
        }
    }
}

pub type SequenceError = ironrdp_error::Error<SequenceErrorKind>;

pub trait SequenceErrorExt {
    fn encode(error: ironrdp_core::EncodeError) -> Self;
    fn decode(error: ironrdp_core::DecodeError) -> Self;
    fn general(context: &'static str) -> Self;
    fn reason(context: &'static str, reason: impl Into<String>) -> Self;
    fn custom<E>(context: &'static str, e: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static;
    fn negotiation(context: &'static str, failure: NegotiationFailure) -> Self;
}

impl SequenceErrorExt for SequenceError {
    #[track_caller]
    fn encode(error: ironrdp_core::EncodeError) -> Self {
        Self::new("encode error", SequenceErrorKind::Encode(error))
    }

    #[track_caller]
    fn decode(error: ironrdp_core::DecodeError) -> Self {
        Self::new("decode error", SequenceErrorKind::Decode(error))
    }

    #[track_caller]
    fn general(context: &'static str) -> Self {
        Self::new(context, SequenceErrorKind::General)
    }

    #[track_caller]
    fn reason(context: &'static str, reason: impl Into<String>) -> Self {
        Self::new(context, SequenceErrorKind::Reason(reason.into()))
    }

    #[track_caller]
    fn custom<E>(context: &'static str, e: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static,
    {
        Self::new(context, SequenceErrorKind::Custom).with_source(e)
    }

    #[track_caller]
    fn negotiation(context: &'static str, failure: NegotiationFailure) -> Self {
        Self::new(context, SequenceErrorKind::Negotiation(failure))
    }
}

pub trait SequenceResultExt {
    #[must_use]
    fn with_context(self, context: &'static str) -> Self;
    #[must_use]
    fn with_source<E>(self, source: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static;
}

impl<T> SequenceResultExt for SequenceResult<T> {
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
