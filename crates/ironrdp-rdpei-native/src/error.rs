use core::fmt;

/// Categorises failures returned by the Windows RDPEI input backend.
#[derive(Debug)]
#[non_exhaustive]
pub enum WindowsInputErrorKind {
    /// The caller supplied an invalid RDPEI input value.
    InvalidInput,
    /// The requested operation is not valid in the current negotiation state.
    InvalidState,
    /// A Windows pointer API call failed.
    NativeApi,
    /// The Windows performance counter could not represent a requested timestamp.
    Clock,
}

impl fmt::Display for WindowsInputErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput => f.write_str("invalid Windows pointer input"),
            Self::InvalidState => f.write_str("invalid Windows pointer input state"),
            Self::NativeApi => f.write_str("Windows pointer API call failed"),
            Self::Clock => f.write_str("Windows pointer clock error"),
        }
    }
}

impl core::error::Error for WindowsInputErrorKind {}

/// A typed failure returned by the Windows RDPEI input backend.
pub type WindowsInputError = ironrdp_error::Error<WindowsInputErrorKind>;

/// The result type used by the Windows RDPEI input backend.
pub type WindowsInputResult<T> = Result<T, WindowsInputError>;

pub(crate) fn invalid_input(context: &'static str) -> WindowsInputError {
    WindowsInputError::new(context, WindowsInputErrorKind::InvalidInput)
}

pub(crate) fn invalid_state(context: &'static str) -> WindowsInputError {
    WindowsInputError::new(context, WindowsInputErrorKind::InvalidState)
}

pub(crate) fn clock_error(context: &'static str) -> WindowsInputError {
    WindowsInputError::new(context, WindowsInputErrorKind::Clock)
}

pub(crate) fn native_error<E>(context: &'static str, source: E) -> WindowsInputError
where
    E: ironrdp_error::Source,
{
    WindowsInputError::new(context, WindowsInputErrorKind::NativeApi).with_source(source)
}

pub(crate) fn clock_error_with_source<E>(context: &'static str, source: E) -> WindowsInputError
where
    E: ironrdp_error::Source,
{
    WindowsInputError::new(context, WindowsInputErrorKind::Clock).with_source(source)
}
