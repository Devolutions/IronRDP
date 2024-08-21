/// Creates a `PduError` with `UnsupportedValue` kind
#[macro_export]
macro_rules! unsupported_message_err {
    ( $name:expr, class: $class:expr, kind: $kind:expr $(,)? ) => {{
        ironrdp_pdu::unsupported_value_err(
            "NOW-PROTO",
            $name,
            alloc::format!("CLASS({}); KIND({})", $class, $kind)
        )
    }};
    ( class: $class:expr, kind: $kind:expr $(,)? ) => {{
        unsupported_message_err!(Self::NAME, class: $class, kind: $kind)
    }};
}
