#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(all(feature = "alloc", not(feature = "std")))]
use alloc::boxed::Box;
use core::fmt;

pub trait Source: core::error::Error + Send + Sync + 'static {}

impl<T> Source for T where T: core::error::Error + Send + Sync + 'static {}

/// Diagnostic metadata stored behind a [`Box`] so that `Error<Kind>` stays small.
///
/// All fields here are purely for display and error-chain traversal; none are
/// needed for matching on the error kind. The allocation only occurs when an
/// error is *constructed* — a cold path — so the per-error heap cost is
/// acceptable.
#[cfg(feature = "alloc")]
struct ErrorMeta {
    context: &'static str,
    location: &'static core::panic::Location<'static>,
    source: Option<Box<dyn Source>>,
}

/// A typed error wrapper carrying a `Kind` discriminant plus diagnostic metadata.
///
/// # `no_alloc` platforms
///
/// When compiled without the `alloc` feature, `Error<Kind>` retains `kind`,
/// `context`, and `location` inline. The error source chain is unavailable.
/// `no_alloc` targets are supported on a best-effort basis and are not a
/// primary target of this crate. Do not add more inline fields here: the
/// struct should stay lean for stack-constrained environments.
pub struct Error<Kind> {
    kind: Kind,
    /// Diagnostic metadata. Present only when `alloc` is available.
    #[cfg(feature = "alloc")]
    meta: Box<ErrorMeta>,
    /// Minimal context kept for `no_alloc` targets (no source chain).
    #[cfg(not(feature = "alloc"))]
    context: &'static str,
    #[cfg(not(feature = "alloc"))]
    location: &'static core::panic::Location<'static>,
}

// Manual `Debug` impl that excludes the `location` field. The location is
// captured via `core::panic::Location::caller()` and rendered in `Display`,
// but its `file()` returns platform-native paths (`/` on Unix, `\` on
// Windows). Including it in `Debug` would break cross-platform snapshot
// tests. Consumers needing programmatic access can use `Error::location()`.
impl<Kind: fmt::Debug> fmt::Debug for Error<Kind> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut dbg = f.debug_struct("Error");
        #[cfg(feature = "alloc")]
        dbg.field("context", &self.meta.context)
            .field("kind", &self.kind)
            .field("source", &self.meta.source);
        #[cfg(not(feature = "alloc"))]
        dbg.field("context", &self.context).field("kind", &self.kind);
        dbg.finish()
    }
}

impl<Kind> Error<Kind> {
    #[cold]
    #[must_use]
    #[track_caller]
    pub fn new(context: &'static str, kind: Kind) -> Self {
        Self {
            kind,
            #[cfg(feature = "alloc")]
            meta: Box::new(ErrorMeta {
                context,
                location: core::panic::Location::caller(),
                source: None,
            }),
            #[cfg(not(feature = "alloc"))]
            context,
            #[cfg(not(feature = "alloc"))]
            location: core::panic::Location::caller(),
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
            this.meta.source = Some(Box::new(source));
            this
        }

        // No source when no alloc
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
            kind: self.kind.into(),
            #[cfg(feature = "alloc")]
            meta: self.meta,
            #[cfg(not(feature = "alloc"))]
            context: self.context,
            #[cfg(not(feature = "alloc"))]
            location: self.location,
        }
    }

    pub fn kind(&self) -> &Kind {
        &self.kind
    }

    /// Returns the source code location at which this error was constructed.
    ///
    /// Captured automatically by [`Error::new`] via [`core::panic::Location::caller`]
    /// and `#[track_caller]`. Useful for diagnostic logging and error reporting
    /// when the variant alone does not narrow down the call site enough.
    pub fn location(&self) -> &'static core::panic::Location<'static> {
        #[cfg(feature = "alloc")]
        {
            self.meta.location
        }
        #[cfg(not(feature = "alloc"))]
        {
            self.location
        }
    }

    pub fn set_context(&mut self, context: &'static str) {
        #[cfg(feature = "alloc")]
        {
            self.meta.context = context;
        }
        #[cfg(not(feature = "alloc"))]
        {
            self.context = context;
        }
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
        #[cfg(feature = "alloc")]
        {
            write!(
                f,
                "[{} @ {}:{}] {}",
                self.meta.context,
                self.meta.location.file(),
                self.meta.location.line(),
                self.kind
            )
        }
        #[cfg(not(feature = "alloc"))]
        {
            write!(
                f,
                "[{} @ {}:{}] {}",
                self.context,
                self.location.file(),
                self.location.line(),
                self.kind
            )
        }
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
            // NOTE: we can't use Option::as_ref here because of type inference
            if let Some(e) = &self.meta.source {
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
        if let Some(source) = &self.0.meta.source {
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
        if !($condition) {
            return ::core::result::Result::Err($crate::Error::new("", $kind));
        }
    };
    ($condition:expr, $context:expr, $kind:expr $(,)?) => {
        if !($condition) {
            return ::core::result::Result::Err($crate::Error::new($context, $kind));
        }
    };
}
