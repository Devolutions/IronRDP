#![cfg_attr(not(feature = "std"), no_std)]
#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::string::String;

use core::fmt;

/// Source error which can be stored inside an [`Error`]
///
/// You should write generic code against this trait in order automatically support
/// `std::error::Error` if available while staying `no_std`-compatible otherwise.
#[cfg(feature = "std")]
pub trait Source: std::error::Error + Sync + Send + 'static {}

#[cfg(feature = "std")]
impl<T> Source for T where T: std::error::Error + Sync + Send + 'static {}

/// Source error which can be stored inside an [`Error`].
///
/// You should write generic code against this trait in order automatically support
/// `std::error::Error` if available while staying `no_std`-compatible otherwise.
#[cfg(not(feature = "std"))]
pub trait Source: fmt::Display + fmt::Debug + Send + Sync + 'static {}

#[cfg(not(feature = "std"))]
impl<T> Source for T where T: fmt::Display + fmt::Debug + Send + Sync + 'static {}

/// A flexible error type holding a context string along a domain-specific kind for detailed reporting
#[derive(Debug)]
#[non_exhaustive]
pub struct Error<Kind> {
    /// Context string
    pub context: &'static str,
    /// Domain-specific error kind
    pub kind: Kind,
    #[cfg(feature = "std")]
    source: Option<Box<dyn std::error::Error + Sync + Send>>,
    #[cfg(all(not(feature = "std"), feature = "alloc"))]
    source: Option<alloc::boxed::Box<dyn Source>>,
}

impl<Kind> Error<Kind> {
    /// Creates a new error of the given kind.
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

    /// Attaches a source to this error.
    #[cold]
    #[must_use]
    pub fn with_source<E>(self, source: E) -> Self
    where
        E: Source,
    {
        #[cfg(feature = "alloc")]
        {
            let mut this = self;
            this.source = Some(alloc::boxed::Box::new(source));
            this
        }

        // No source when no std and no alloc crates
        #[cfg(not(feature = "alloc"))]
        {
            let _ = source;
            self
        }
    }

    /// Converts this error into another one with a compatible kind.
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

    /// Returns the error kind
    pub fn kind(&self) -> &Kind {
        &self.kind
    }

    /// Returns a struct for formatting and reporting this error to the user
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

/// The reporting type to use when showing the final error to the user
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

/// New Type wrapper around a [`String`](alloc::string::String) which can be used as an error
#[cfg(feature = "alloc")]
#[derive(Debug, PartialEq, PartialOrd, Eq, Ord)]
pub struct StringError(pub String);

#[cfg(feature = "alloc")]
impl From<String> for StringError {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for StringError {}

#[cfg(feature = "alloc")]
impl fmt::Display for StringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// New Type wrapper around a `&'static str` which can be used as an error
#[derive(Debug, PartialEq, PartialOrd, Eq, Ord)]
pub struct StrError(pub &'static str);

impl From<&'static str> for StrError {
    fn from(value: &'static str) -> Self {
        Self(value)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for StrError {}

impl fmt::Display for StrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Wrapper around the [`format`](::alloc::format) macro, but creates either [`StrError`] or [`StringError`] with the purpose
/// of describing an error.
///
/// If the `alloc` feature is disabled, no formatting happens and a [`StrError`] is created using the first argument
/// as a best effort.
#[macro_export]
macro_rules! err_desc {
    ($($arg:tt)*) => {
        $crate::err_desc_impl!($($arg)*)
    }
}

#[cfg(feature = "alloc")]
#[doc(hidden)]
#[macro_export]
macro_rules! err_desc_impl {
    ($desc:literal) => {
        $crate::StrError($desc)
    };
    ($($arg:tt)*) => {
        $crate::StringError(::alloc::format!($($arg)*))
    }
}

#[cfg(not(feature = "alloc"))]
#[doc(hidden)]
#[macro_export]
macro_rules! err_desc_impl {
    ($desc:literal $($tail:tt)*) => {
        $crate::StrError($desc)
    };
}

/// Temporary compatibility traits to smooth transition from old style
#[cfg(feature = "std")]
#[doc(hidden)]
pub mod legacy {
    #[doc(hidden)]
    pub trait CatchAllKind {
        const CATCH_ALL_VALUE: Self;
    }

    #[doc(hidden)]
    pub trait ErrorContext: std::error::Error {
        fn context(&self) -> &'static str;
    }

    #[doc(hidden)]
    impl<E, Kind> From<E> for crate::Error<Kind>
    where
        E: ErrorContext + Send + Sync + 'static,
        Kind: CatchAllKind,
    {
        #[cold]
        fn from(error: E) -> Self {
            Self::new(error.context(), Kind::CATCH_ALL_VALUE).with_source(error)
        }
    }
}
