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
/// `context`, and `location` inline. Sources exposed by `Kind` remain available,
/// but [`Error::with_source`] cannot retain an additional source. `no_alloc`
/// targets are supported on a best-effort basis and are not a primary target of
/// this crate. Do not add more inline fields here: the struct should stay lean
/// for stack-constrained environments.
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

    pub fn kind(&self) -> &Kind {
        &self.kind
    }

    /// Returns the static operation label supplied when this error was created.
    pub fn context(&self) -> &'static str {
        #[cfg(feature = "alloc")]
        {
            self.meta.context
        }
        #[cfg(not(feature = "alloc"))]
        {
            self.context
        }
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
        ErrorReport {
            error: self,
            include_locations: false,
        }
    }
}

impl<Kind> fmt::Display for Error<Kind>
where
    Kind: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[cfg(feature = "alloc")]
        let (context, location) = (self.meta.context, self.meta.location);
        #[cfg(not(feature = "alloc"))]
        let (context, location) = (self.context, self.location);

        if f.alternate() {
            write!(f, "[{context} @ {}:{}] {}", location.file(), location.line(), self.kind)
        } else {
            write!(f, "[{context}] {}", self.kind)
        }
    }
}

impl<Kind> core::error::Error for Error<Kind>
where
    Kind: core::error::Error,
{
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        if let Some(source) = self.kind.source() {
            return Some(source);
        }

        #[cfg(feature = "alloc")]
        {
            // NOTE: we can't use Option::as_ref here because of type inference
            if let Some(e) = &self.meta.source {
                return Some(e.as_ref());
            }
        }

        None
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

pub struct ErrorReport<'a, Kind> {
    error: &'a Error<Kind>,
    include_locations: bool,
}

impl<Kind> ErrorReport<'_, Kind> {
    /// Includes source locations when formatting this error report.
    ///
    /// Alternate formatting (`{report:#}`) also includes source locations.
    #[must_use]
    pub fn with_locations(mut self) -> Self {
        self.include_locations = true;
        self
    }
}

#[cfg(feature = "std")]
impl<Kind> fmt::Display for ErrorReport<'_, Kind>
where
    Kind: core::error::Error,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use core::error::Error as _;

        let include_locations = self.include_locations || f.alternate();

        if include_locations {
            write!(f, "{:#}", self.error)?;
        } else {
            write!(f, "{}", self.error)?;
        }

        let mut next_source = self.error.source();

        while let Some(e) = next_source {
            if include_locations {
                write!(f, ", caused by: {e:#}")?;
            } else {
                write!(f, ", caused by: {e}")?;
            }
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
        let include_locations = self.include_locations || f.alternate();

        if include_locations {
            write!(f, "{:#}", self.error)?;
        } else {
            write!(f, "{}", self.error)?;
        }

        #[cfg(feature = "alloc")]
        if let Some(source) = &self.error.meta.source {
            if include_locations {
                write!(f, ", caused by: {source:#}")?;
            } else {
                write!(f, ", caused by: {source}")?;
            }
        }

        Ok(())
    }
}

/// Defines the canonical conversion from an inner error kind to this error kind.
///
/// Implement this trait explicitly for every supported inner kind. No blanket
/// implementation is provided, so error-kind owners retain control over their
/// cross-domain mappings.
pub trait ErrorMapping<InnerKind>: Sized {
    /// Converts an error with `InnerKind` into an error with this kind.
    ///
    /// The conversion is called by [`ResultExt::map_err_as`].
    #[track_caller]
    fn map_error(error: Error<InnerKind>) -> Error<Self>;
}

/// Extension methods for converting [`Error`] values across typed error boundaries.
///
/// Import this trait to use [`ResultExt::map_err_as`], [`ResultExt::map_err_kind`],
/// and [`ResultExt::map_err_source`] on `Result<T, Error<InnerKind>>`.
pub trait ResultExt<T, InnerKind> {
    /// Converts an error using `OuterKind`'s canonical [`ErrorMapping`] implementation.
    #[track_caller]
    fn map_err_as<OuterKind>(self) -> Result<T, Error<OuterKind>>
    where
        OuterKind: ErrorMapping<InnerKind>;

    /// Wraps the complete inner error in an explicitly selected outer error kind.
    ///
    /// Use this when the outer kind has a variant that carries `Error<InnerKind>`.
    #[track_caller]
    fn map_err_kind<OuterKind>(
        self,
        context: &'static str,
        wrap: impl FnOnce(Error<InnerKind>) -> OuterKind,
    ) -> Result<T, Error<OuterKind>>;

    /// Converts an error to a coarse outer kind while retaining the inner error as its source.
    ///
    /// Source chaining follows [`Error::with_source`] behavior and is therefore unavailable
    /// when `Error<InnerKind>` does not implement [`Source`].
    #[track_caller]
    fn map_err_source<OuterKind>(self, context: &'static str, kind: OuterKind) -> Result<T, Error<OuterKind>>
    where
        Error<InnerKind>: Source;
}

impl<T, InnerKind> ResultExt<T, InnerKind> for Result<T, Error<InnerKind>> {
    #[track_caller]
    fn map_err_as<OuterKind>(self) -> Result<T, Error<OuterKind>>
    where
        OuterKind: ErrorMapping<InnerKind>,
    {
        match self {
            Ok(value) => Ok(value),
            Err(error) => Err(OuterKind::map_error(error)),
        }
    }

    #[track_caller]
    fn map_err_kind<OuterKind>(
        self,
        context: &'static str,
        wrap: impl FnOnce(Error<InnerKind>) -> OuterKind,
    ) -> Result<T, Error<OuterKind>> {
        match self {
            Ok(value) => Ok(value),
            Err(error) => Err(Error::new(context, wrap(error))),
        }
    }

    #[track_caller]
    fn map_err_source<OuterKind>(self, context: &'static str, kind: OuterKind) -> Result<T, Error<OuterKind>>
    where
        Error<InnerKind>: Source,
    {
        match self {
            Ok(value) => Ok(value),
            Err(error) => Err(Error::new(context, kind).with_source(error)),
        }
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

#[cfg(test)]
mod tests {
    use super::Error;

    #[test]
    fn context_returns_static_error_label() {
        let error = Error::new("test context", ());

        assert_eq!(error.context(), "test context");
    }
}
