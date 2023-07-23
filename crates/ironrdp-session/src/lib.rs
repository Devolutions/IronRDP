#[macro_use]
extern crate tracing;

#[macro_use]
mod macros;

pub mod fast_path;
pub mod image;
pub mod legacy;
pub mod pointer;
pub mod rfx; // FIXME: maybe this module should not be in this crate
pub mod utils;
pub mod x224;

mod active_stage;

use core::fmt;

pub use active_stage::{ActiveStage, ActiveStageOutput};

pub type SessionResult<T> = std::result::Result<T, SessionError>;

#[non_exhaustive]
#[derive(Debug)]
pub enum SessionErrorKind {
    Pdu(ironrdp_pdu::PduError),
    Reason(String),
    General,
    Custom,
}

impl fmt::Display for SessionErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self {
            SessionErrorKind::Pdu(_) => write!(f, "PDU error"),
            SessionErrorKind::Reason(description) => write!(f, "reason: {description}"),
            SessionErrorKind::General => write!(f, "general error"),
            SessionErrorKind::Custom => write!(f, "custom error"),
        }
    }
}

impl std::error::Error for SessionErrorKind {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self {
            SessionErrorKind::Pdu(e) => Some(e),
            SessionErrorKind::Reason(_) => None,
            SessionErrorKind::General => None,
            SessionErrorKind::Custom => None,
        }
    }
}

pub type SessionError = ironrdp_error::Error<SessionErrorKind>;

pub trait SessionErrorExt {
    fn pdu(error: ironrdp_pdu::PduError) -> Self;
    fn general(context: &'static str) -> Self;
    fn reason(context: &'static str, reason: impl Into<String>) -> Self;
    fn custom<E>(context: &'static str, e: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static;
}

impl SessionErrorExt for SessionError {
    fn pdu(error: ironrdp_pdu::PduError) -> Self {
        Self::new("invalid payload", SessionErrorKind::Pdu(error))
    }

    fn general(context: &'static str) -> Self {
        Self::new(context, SessionErrorKind::General)
    }

    fn reason(context: &'static str, reason: impl Into<String>) -> Self {
        Self::new(context, SessionErrorKind::Reason(reason.into()))
    }

    fn custom<E>(context: &'static str, e: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static,
    {
        Self::new(context, SessionErrorKind::Custom).with_source(e)
    }
}

pub trait SessionResultExt {
    fn with_context(self, context: &'static str) -> Self;
    fn with_source<E>(self, source: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static;
}

impl<T> SessionResultExt for SessionResult<T> {
    fn with_context(self, context: &'static str) -> Self {
        self.map_err(|mut e| {
            e.context = context;
            e
        })
    }

    fn with_source<E>(self, source: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static,
    {
        self.map_err(|e| e.with_source(source))
    }
}
