#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(all(feature = "alloc", not(feature = "std")))]
use alloc::boxed::Box;
use core::fmt;

#[cfg(feature = "std")]
pub trait Source: core::error::Error + Sync + Send + 'static {}

#[cfg(feature = "std")]
impl<T> Source for T where T: core::error::Error + Sync + Send + 'static {}

#[cfg(not(feature = "std"))]
pub trait Source: fmt::Display + fmt::Debug + Send + Sync + 'static {}

#[cfg(not(feature = "std"))]
impl<T> Source for T where T: fmt::Display + fmt::Debug + Send + Sync + 'static {}

#[derive(Debug)]
pub struct Error<Kind> {
    context: &'static str,
    kind: Kind,
    #[cfg(feature = "std")]
    source: Option<Box<dyn core::error::Error + Sync + Send>>,
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

    pub fn set_context(&mut self, context: &'static str) {
        self.context = context;
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
impl<Kind> core::error::Error for Error<Kind>
where
    Kind: core::error::Error,
{
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
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
    Kind: core::error::Error + Send + Sync + 'static,
{
    fn from(error: Error<Kind>) -> Self {
        Self::other(error)
    }
}

pub struct ErrorReport<'a, Kind>(&'a Error<Kind>);

#[cfg(feature = "std")]
impl<Kind> fmt::Display for ErrorReport<'_, Kind>
where
    Kind: core::error::Error,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use core::error::Error as _;

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

/// Returns from the enclosing function with an [`Error`] built from a kind variant.
///
/// Three forms are supported:
///
/// - `bail!(kind)` — empty context.
/// - `bail!(context, kind)` — explicit `&'static str` context.
/// - `bail!(context, kind, source: source)` — explicit context plus a chained source error.
///
/// The kind type is inferred from the enclosing function's return type, which must be
/// `Result<_, Error<Kind>>` (or any type alias resolving to it).
///
/// Mirrors the call-site shape of [`anyhow::bail!`] but produces a typed
/// `Error<Kind>` rather than a type-erased `anyhow::Error`.
///
/// [`anyhow::bail!`]: https://docs.rs/anyhow/latest/anyhow/macro.bail.html
#[macro_export]
macro_rules! bail {
    ($kind:expr $(,)?) => {
        return ::core::result::Result::Err($crate::Error::new("", $kind))
    };
    ($context:expr, $kind:expr $(,)?) => {
        return ::core::result::Result::Err($crate::Error::new($context, $kind))
    };
    ($context:expr, $kind:expr, source: $source:expr $(,)?) => {
        return ::core::result::Result::Err($crate::Error::new($context, $kind).with_source($source))
    };
}

/// Returns from the enclosing function with an [`Error`] if the given condition is false.
///
/// Two forms are supported:
///
/// - `ensure!(condition, kind)` — empty context.
/// - `ensure!(condition, context, kind)` — explicit `&'static str` context.
///
/// The kind type is inferred from the enclosing function's return type, which must be
/// `Result<_, Error<Kind>>` (or any type alias resolving to it).
///
/// Mirrors the call-site shape of [`anyhow::ensure!`] but produces a typed
/// `Error<Kind>` rather than a type-erased `anyhow::Error`.
///
/// [`anyhow::ensure!`]: https://docs.rs/anyhow/latest/anyhow/macro.ensure.html
#[macro_export]
macro_rules! ensure {
    ($condition:expr, $kind:expr $(,)?) => {
        if !$condition {
            return ::core::result::Result::Err($crate::Error::new("", $kind));
        }
    };
    ($condition:expr, $context:expr, $kind:expr $(,)?) => {
        if !$condition {
            return ::core::result::Result::Err($crate::Error::new($context, $kind));
        }
    };
}
