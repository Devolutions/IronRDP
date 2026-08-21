/// Creates a `SequenceError` with `General` kind
///
/// Shorthand for
/// ```rust
/// <ironrdp_sequence::SequenceError as ironrdp_sequence::SequenceErrorExt>::general(context)
/// ```
#[macro_export]
macro_rules! general_err {
    ( $context:expr $(,)? ) => {{ <$crate::SequenceError as $crate::SequenceErrorExt>::general($context) }};
}

/// Creates a `SequenceError` with `Reason` kind
///
/// Shorthand for
/// ```rust
/// <ironrdp_sequence::SequenceError as ironrdp_sequence::SequenceErrorExt>::reason(context, reason)
/// ```
#[macro_export]
macro_rules! reason_err {
    ( $context:expr, $($arg:tt)* ) => {{
        <$crate::SequenceError as $crate::SequenceErrorExt>::reason($context, format!($($arg)*))
    }};
}

/// Creates a `SequenceError` with `Custom` kind and a source error attached to it
///
/// Shorthand for
/// ```rust
/// <ironrdp_sequence::SequenceError as ironrdp_sequence::SequenceErrorExt>::custom(context, source)
/// ```
#[macro_export]
macro_rules! custom_err {
    ( $context:expr, $source:expr $(,)? ) => {{ <$crate::SequenceError as $crate::SequenceErrorExt>::custom($context, $source) }};
}
