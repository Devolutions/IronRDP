#[macro_use]
extern crate tracing;

pub mod image;
pub mod legacy;
pub mod rfx; // FIXME: maybe this module should not be in this crate

mod active_stage;
mod fast_path;
mod utils;
mod x224;

use core::fmt;

pub use active_stage::{ActiveStage, ActiveStageOutput};

pub type Result<T> = std::result::Result<T, Error>;

#[non_exhaustive]
#[derive(Debug)]
pub enum ErrorKind {
    Pdu(ironrdp_pdu::Error),
    Custom(Box<dyn std::error::Error + Sync + Send + 'static>),
    General,
}

#[derive(Debug)]
pub struct Error {
    pub context: &'static str,
    pub kind: ErrorKind,
    pub reason: Option<String>,
}

impl Error {
    pub const fn new(context: &'static str) -> Self {
        Self {
            context,
            kind: ErrorKind::General,
            reason: None,
        }
    }

    pub fn with_kind(mut self, kind: ErrorKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn with_custom<E>(mut self, custom_error: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static,
    {
        self.kind = ErrorKind::Custom(Box::new(custom_error));
        self
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            ErrorKind::Pdu(e) => Some(e),
            ErrorKind::Custom(e) => Some(e.as_ref()),
            ErrorKind::General => None,
        }
    }
}

impl From<Error> for std::io::Error {
    fn from(error: Error) -> Self {
        std::io::Error::new(std::io::ErrorKind::Other, error)
    }
}

impl From<ironrdp_pdu::Error> for Error {
    fn from(value: ironrdp_pdu::Error) -> Self {
        Self {
            context: "invalid payload",
            kind: ErrorKind::Pdu(value),
            reason: None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.context)?;

        match &self.kind {
            ErrorKind::Pdu(e) => {
                if f.alternate() {
                    write!(f, ": {e}")?;
                }
            }
            ErrorKind::Custom(e) => {
                if f.alternate() {
                    write!(f, ": {e}")?;

                    let mut next_source = e.source();
                    while let Some(e) = next_source {
                        write!(f, ", caused by: {e}")?;
                        next_source = e.source();
                    }
                }
            }
            ErrorKind::General => {}
        }

        if let Some(reason) = &self.reason {
            write!(f, " ({reason})")?;
        }

        Ok(())
    }
}

pub trait SessionResultExt {
    fn with_context(self, context: &'static str) -> Self;
    fn with_kind(self, kind: ErrorKind) -> Self;
    fn with_custom<E>(self, custom_error: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static;
    fn with_reason(self, reason: impl Into<String>) -> Self;
}

impl<T> SessionResultExt for Result<T> {
    fn with_context(self, context: &'static str) -> Self {
        self.map_err(|mut e| {
            e.context = context;
            e
        })
    }

    fn with_kind(self, kind: ErrorKind) -> Self {
        self.map_err(|mut e| {
            e.kind = kind;
            e
        })
    }

    fn with_custom<E>(self, custom_error: E) -> Self
    where
        E: std::error::Error + Sync + Send + 'static,
    {
        self.map_err(|mut e| {
            e.kind = ErrorKind::Custom(Box::new(custom_error));
            e
        })
    }

    fn with_reason(self, reason: impl Into<String>) -> Self {
        self.map_err(|mut e| {
            e.reason = Some(reason.into());
            e
        })
    }
}
