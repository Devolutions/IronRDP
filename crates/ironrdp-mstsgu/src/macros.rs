/// Creates a [`crate::Error`] with `Custom` kind and a source error attached to it
#[macro_export]
macro_rules! custom_err {
    ( $context:expr, $source:expr $(,)? ) => {{
        <$crate::Error as $crate::GwErrorExt>::custom($context, $source)
    }};
}
