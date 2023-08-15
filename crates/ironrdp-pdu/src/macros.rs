//! Helper macros for PDU encoding and decoding
//!
//! Some are exported and available to external crates

/// Creates a `PduError` with `NotEnoughBytes` kind
///
/// Shorthand for
/// ```rust
/// <crate::PduError as crate::PduErrorExt>::not_enough_bytes(context, received, expected)
/// ```
/// and
/// ```rust
/// <crate::PduError as crate::PduErrorExt>::not_enough_bytes(Self::NAME, received, expected)
/// ```
#[macro_export]
macro_rules! not_enough_bytes_err {
    ( $context:expr, $received:expr , $expected:expr $(,)? ) => {{
        <$crate::PduError as $crate::PduErrorExt>::not_enough_bytes($context, $received, $expected)
    }};
    ( $received:expr , $expected:expr $(,)? ) => {{
        not_enough_bytes_err!(Self::NAME, $received, $expected)
    }};
}

/// Creates a `PduError` with `InvalidMessage` kind
///
/// Shorthand for
/// ```rust
/// <crate::PduError as crate::PduErrorExt>::invalid_message(context, field, reason)
/// ```
/// and
/// ```rust
/// <crate::PduError as crate::PduErrorExt>::invalid_message(Self::NAME, field, reason)
/// ```
#[macro_export]
macro_rules! invalid_message_err {
    ( $context:expr, $field:expr , $reason:expr $(,)? ) => {{
        <$crate::PduError as $crate::PduErrorExt>::invalid_message($context, $field, $reason)
    }};
    ( $field:expr , $reason:expr $(,)? ) => {{
        invalid_message_err!(Self::NAME, $field, $reason)
    }};
}

/// Creates a `PduError` with `UnexpectedMessageType` kind
///
/// Shorthand for
/// ```rust
/// <crate::PduError as crate::PduErrorExt>::unexpected_message_type(context, got)
/// ```
/// and
/// ```rust
/// <crate::PduError as crate::PduErrorExt>::unexpected_message_type(Self::NAME, got)
/// ```
#[macro_export]
macro_rules! unexpected_message_type_err {
    ( $context:expr, $got:expr $(,)? ) => {{
        <$crate::PduError as $crate::PduErrorExt>::unexpected_message_type($context, $got)
    }};
    ( $got:expr $(,)? ) => {{
        unexpected_message_type_err!(Self::NAME, $got)
    }};
}

/// Creates a `PduError` with `UnsupportedVersion` kind
///
/// Shorthand for
/// ```rust
/// <crate::PduError as crate::PduErrorExt>::unsupported_version(context, got)
/// ```
/// and
/// ```rust
/// <crate::PduError as crate::PduErrorExt>::unsupported_version(Self::NAME, got)
/// ```
#[macro_export]
macro_rules! unsupported_version_err {
    ( $context:expr, $got:expr $(,)? ) => {{
        <$crate::PduError as $crate::PduErrorExt>::unsupported_version($context, $got)
    }};
    ( $got:expr $(,)? ) => {{
        unsupported_version_err!(Self::NAME, $got)
    }};
}

/// Creates a `PduError` with `Other` kind
///
/// Shorthand for
/// ```rust
/// <crate::PduError as crate::PduErrorExt>::other(context, description)
/// ```
/// and
/// ```rust
/// <crate::PduError as crate::PduErrorExt>::other(Self::NAME, description)
/// ```
#[macro_export]
macro_rules! other_err {
    ( $context:expr, $description:expr $(,)? ) => {{
        <$crate::PduError as $crate::PduErrorExt>::other($context, $description)
    }};
    ( $description:expr $(,)? ) => {{
        other_err!(Self::NAME, $description)
    }};
}

/// Creates a `PduError` with `Custom` kind and a source error attached to it
///
/// Shorthand for
/// ```rust
/// <crate::PduError as crate::PduErrorExt>::custom(context, source)
/// ```
/// and
/// ```rust
/// <crate::PduError as crate::PduErrorExt>::custom(Self::NAME, source)
/// ```
#[macro_export]
macro_rules! custom_err {
    ( $context:expr, $source:expr $(,)? ) => {{
        <$crate::PduError as $crate::PduErrorExt>::custom($context, $source)
    }};
    ( $source:expr $(,)? ) => {{
        custom_err!(Self::NAME, $source)
    }};
}

#[macro_export]
macro_rules! ensure_size {
    (ctx: $ctx:expr, in: $buf:ident, size: $expected:expr) => {{
        let received = $buf.len();
        let expected = $expected;
        if !(received >= expected) {
            return Err(<$crate::PduError as $crate::PduErrorExt>::not_enough_bytes($ctx, received, expected));
        }
    }};
    (in: $buf:ident, size: $expected:expr) => {{
        $crate::ensure_size!(ctx: Self::NAME, in: $buf, size: $expected)
    }};
}

#[macro_export]
macro_rules! ensure_fixed_part_size {
    (in: $buf:ident) => {{
        $crate::ensure_size!(ctx: Self::NAME, in: $buf, size: Self::FIXED_PART_SIZE)
    }};
}

#[macro_export]
macro_rules! cast_length {
    ($ctx:expr, $field:expr, $len:expr) => {{
        $len.try_into().map_err(|e| {
            <$crate::PduError as $crate::PduErrorExt>::invalid_message($ctx, $field, "too many elements").with_source(e)
        })
    }};
    ($field:expr, $len:expr) => {{
        $crate::cast_length!(<Self as $crate::Pdu>::NAME, $field, $len)
    }};
}

#[macro_export]
macro_rules! cast_int {
    ($ctx:expr, $field:expr, $len:expr) => {{
        $len.try_into().map_err(|e| {
            <$crate::PduError as $crate::PduErrorExt>::invalid_message(
                $ctx,
                $field,
                "out of range integral type conversion",
            )
            .with_source(e)
        })
    }};
    ($field:expr, $len:expr) => {{
        $crate::cast_int!(Self::NAME, $field, $len)
    }};
}

/// Asserts that the traits support dynamic dispatch.
///
/// From <https://docs.rs/static_assertions/latest/src/static_assertions/assert_obj_safe.rs.html#72-76>
#[macro_export]
macro_rules! assert_obj_safe {
    ($($xs:path),+ $(,)?) => {
        $(const _: Option<&dyn $xs> = None;)+
    };
}

/// Implements additional traits for a plain old data structure (POD).
#[macro_export]
macro_rules! impl_pdu_pod {
    ($pdu_ty:ty) => {
        impl $crate::IntoOwnedPdu for $pdu_ty {
            type Owned = Self;

            fn into_owned_pdu(self) -> Self::Owned {
                self
            }
        }

        impl $crate::PduDecodeOwned for $pdu_ty {
            fn decode_owned(src: &mut $crate::cursor::ReadCursor<'_>) -> $crate::PduResult<Self> {
                <Self as $crate::PduDecode>::decode(src)
            }
        }
    };
}

/// Implements additional traits for a borrowing PDU and defines a static-bounded owned version.
#[macro_export]
macro_rules! impl_pdu_borrowing {
    ($pdu_ty:ident, $owned_ty:ident) => {
        pub type $owned_ty = $pdu_ty<'static>;

        impl $crate::PduDecodeOwned for $owned_ty {
            fn decode_owned(src: &mut $crate::cursor::ReadCursor<'_>) -> $crate::PduResult<Self> {
                let pdu = <$pdu_ty as $crate::PduDecode>::decode(src)?;
                Ok($crate::IntoOwnedPdu::into_owned_pdu(pdu))
            }
        }
    };
}

#[macro_export]
macro_rules! try_read_optional {
    ($e:expr, $ret:expr) => {
        match $e {
            Ok(v) => v,
            Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Ok($ret);
            }
            Err(e) => return Err(From::from(e)),
        }
    };
}

#[macro_export]
macro_rules! try_write_optional {
    ($val:expr, $f:expr) => {
        if let Some(ref val) = $val {
            $f(val)?
        } else {
            return Ok(());
        }
    };
}
