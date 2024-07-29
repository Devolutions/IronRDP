/// Creates a `PduError` with `UnexpectedMessageType` kind
///
/// Shorthand for
/// ```text
/// <PduError as PduErrorExt>::unexpected_message_type(context, got)
/// ```
/// and
/// ```text
/// <PduError as PduErrorExt>::unexpected_message_type(Self::NAME, got)
/// ```
#[macro_export]
macro_rules! unexpected_message_kind_err {
    ( $name:expr, class: $class:expr, kind: $kind:expr $(,)? ) => {{
        <ironrdp_pdu::PduError as ironrdp_pdu::PduErrorExt>::unsupported_pdu(
            "NOW-PROTO",
            $name,
            alloc::format!("CLASS({}); KIND({})", $class, $kind)
        )
    }};
    ( class: $class:expr, kind: $kind:expr $(,)? ) => {{
        unexpected_message_kind_err!(Self::NAME, class: $class, kind: $kind)
    }};
}
