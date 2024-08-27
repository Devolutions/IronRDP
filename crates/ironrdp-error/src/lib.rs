#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::boxed::Box;

use core::fmt;

#[cfg(feature = "std")]
pub trait Source: std::error::Error + Sync + Send + 'static {}

#[cfg(feature = "std")]
impl<T> Source for T where T: std::error::Error + Sync + Send + 'static {}

#[cfg(not(feature = "std"))]
pub trait Source: fmt::Display + fmt::Debug + Send + Sync + 'static {}

#[cfg(not(feature = "std"))]
impl<T> Source for T where T: fmt::Display + fmt::Debug + Send + Sync + 'static {}

#[derive(Debug)]
pub struct Error<Kind> {
    pub context: &'static str,
    pub kind: Kind,
    #[cfg(feature = "std")]
    source: Option<Box<dyn std::error::Error + Sync + Send>>,
    #[cfg(all(not(feature = "std"), feature = "alloc"))]
    source: Option<Box<dyn Source>>,
}

impl<Kind> Error<Kind> {
    #[cold]
    #[must_use]
    pub fn new(context: &'static str, kind: Kind) -> Self {
        Self {
            context,
            kind,
            #[cfg(feature = "alloc")]
            source: None,
        }
    }

    #[cold]
    #[must_use]
    pub fn with_source<E>(self, source: E) -> Self
    where
        E: Source,
    {
        #[cfg(feature = "alloc")]
        {
            let mut this = self;
            this.source = Some(Box::new(source));
            this
        }

        // No source when no std and no alloc crates
        #[cfg(not(feature = "alloc"))]
        {
            let _ = source;
            self
        }
    }

    pub fn into_other_kind<OtherKind>(self) -> Error<OtherKind>
    where
        Kind: Into<OtherKind>,
    {
        Error {
            context: self.context,
            kind: self.kind.into(),
            #[cfg(any(feature = "std", feature = "alloc"))]
            source: self.source,
        }
    }

    pub fn kind(&self) -> &Kind {
        &self.kind
    }

    pub fn report(&self) -> ErrorReport<'_, Kind> {
        ErrorReport(self)
    }
}

impl<Kind> fmt::Display for Error<Kind>
where
    Kind: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.context, self.kind)
    }
}

#[cfg(feature = "std")]
impl<Kind> std::error::Error for Error<Kind>
where
    Kind: std::error::Error,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let Some(source) = self.kind.source() {
            Some(source)
        } else {
            // NOTE: we can’t use Option::as_ref here because of type inference
            if let Some(e) = &self.source {
                Some(e.as_ref())
            } else {
                None
            }
        }
    }
}

#[cfg(feature = "std")]
impl<Kind> From<Error<Kind>> for std::io::Error
where
    Kind: std::error::Error + Send + Sync + 'static,
{
    fn from(error: Error<Kind>) -> Self {
        Self::new(std::io::ErrorKind::Other, error)
    }
}

pub struct ErrorReport<'a, Kind>(&'a Error<Kind>);

#[cfg(feature = "std")]
impl<Kind> fmt::Display for ErrorReport<'_, Kind>
where
    Kind: std::error::Error,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use std::error::Error;

        write!(f, "{}", self.0)?;

        let mut next_source = self.0.source();

        while let Some(e) = next_source {
            write!(f, ", caused by: {e}")?;
            next_source = e.source();
        }

        Ok(())
    }
}

#[cfg(not(feature = "std"))]
impl<E> fmt::Display for ErrorReport<'_, E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)?;

        #[cfg(feature = "alloc")]
        if let Some(source) = &self.0.source {
            write!(f, ", caused by: {source}")?;
        }

        Ok(())
    }
}
