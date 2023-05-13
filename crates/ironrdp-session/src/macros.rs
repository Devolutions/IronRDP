/// Creates a `SessionError` with `General` kind
///
/// Shorthand for
/// ```rust
/// <crate::SessionError as crate::SessionErrorExt>::general(context)
/// ```
#[macro_export]
macro_rules! general_err {
    ( $context:expr $(,)? ) => {{
        <$crate::SessionError as $crate::SessionErrorExt>::general($context)
    }};
}

/// Creates a `SessionError` with `Reason` kind
///
/// Shorthand for
/// ```rust
/// <crate::SessionError as crate::SessionErrorExt>::reason(context, reason)
/// ```
#[macro_export]
macro_rules! reason_err {
    ( $context:expr, $($arg:tt)* ) => {{
        <$crate::SessionError as $crate::SessionErrorExt>::reason($context, format!($($arg)*))
    }};
}

/// Creates a `SessionError` with `Custom` kind and a source error attached to it
///
/// Shorthand for
/// ```rust
/// <crate::SessionError as crate::SessionErrorExt>::custom(context, source)
/// ```
#[macro_export]
macro_rules! custom_err {
    ( $context:expr, $source:expr $(,)? ) => {{
        <$crate::SessionError as $crate::SessionErrorExt>::custom($context, $source)
    }};
}
