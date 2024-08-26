//! Helper macros for PDU encoding and decoding
//!
//! Some are exported and available to external crates

/// Finds the name of the function in which this macro is expanded
#[macro_export]
macro_rules! function {
    // Taken from https://stackoverflow.com/a/40234666
    () => {{
        fn f() {}
        fn type_name_of<T>(_: T) -> &'static str {
            core::any::type_name::<T>()
        }
        let name = type_name_of(f);
        name.strip_suffix("::f").unwrap()
    }};
}

/// Creates a `PduError` with `NotEnoughBytes` kind
///
/// Shorthand for
/// ```rust
/// <ironrdp_pdu::PduError as ironrdp_pdu::PduErrorExt>::not_enough_bytes(context, received, expected)
/// ```
/// and
/// ```rust
/// <ironrdp_pdu::PduError as ironrdp_pdu::PduErrorExt>::not_enough_bytes($crate::function!(), received, expected)
/// ```
#[macro_export]
macro_rules! not_enough_bytes_err {
    ( $context:expr, $received:expr , $expected:expr $(,)? ) => {{
        <$crate::PduError as $crate::PduErrorExt>::not_enough_bytes($context, $received, $expected)
    }};
    ( $received:expr , $expected:expr $(,)? ) => {{
        not_enough_bytes_err!($crate::function!(), $received, $expected)
    }};
}

/// Creates a `PduError` with `InvalidMessage` kind
///
/// Shorthand for
/// ```rust
/// <ironrdp_pdu::PduError as ironrdp_pdu::PduErrorExt>::invalid_message(context, field, reason)
/// ```
/// and
/// ```rust
/// <ironrdp_pdu::PduError as ironrdp_pdu::PduErrorExt>::invalid_message($crate::function!(), field, reason)
/// ```
#[macro_export]
macro_rules! invalid_message_err {
    ( $context:expr, $field:expr , $reason:expr $(,)? ) => {{
        <$crate::PduError as $crate::PduErrorExt>::invalid_message($context, $field, $reason)
    }};
    ( $field:expr , $reason:expr $(,)? ) => {{
        invalid_message_err!($crate::function!(), $field, $reason)
    }};
}

/// Creates a `PduError` with `UnexpectedMessageType` kind
///
/// Shorthand for
/// ```rust
/// <ironrdp_pdu::PduError as ironrdp_pdu::PduErrorExt>::unexpected_message_type(context, got)
/// ```
/// and
/// ```rust
/// <ironrdp_pdu::PduError as ironrdp_pdu::PduErrorExt>::unexpected_message_type($crate::function!(), got)
/// ```
#[macro_export]
macro_rules! unexpected_message_type_err {
    ( $context:expr, $got:expr $(,)? ) => {{
        <$crate::PduError as $crate::PduErrorExt>::unexpected_message_type($context, $got)
    }};
    ( $got:expr $(,)? ) => {{
        unexpected_message_type_err!($crate::function!(), $got)
    }};
}

/// Creates a `PduError` with `UnsupportedVersion` kind
///
/// Shorthand for
/// ```rust
/// <ironrdp_pdu::PduError as ironrdp_pdu::PduErrorExt>::unsupported_version(context, got)
/// ```
/// and
/// ```rust
/// <ironrdp_pdu::PduError as ironrdp_pdu::PduErrorExt>::unsupported_version($crate::function!(), got)
/// ```
#[macro_export]
macro_rules! unsupported_version_err {
    ( $context:expr, $got:expr $(,)? ) => {{
        <$crate::PduError as $crate::PduErrorExt>::unsupported_version($context, $got)
    }};
    ( $got:expr $(,)? ) => {{
        unsupported_version_err!($crate::function!(), $got)
    }};
}

/// Creates a `PduError` with `UnsupportedPdu` kind
/// Shorthand for
/// ```rust
/// <ironrdp_pdu::PduError as ironrdp_pdu::PduErrorExt>::unsupported_pdu(context, name, value)
/// ```
/// and
/// ```rust
/// <ironrdp_pdu::PduError as ironrdp_pdu::PduErrorExt>::unsupported_pdu($crate::function!(), name, value)
/// ```
#[macro_export]
macro_rules! unsupported_pdu_err {
    ( $context:expr, $name:expr, $value:expr $(,)? ) => {{
        <$crate::PduError as $crate::PduErrorExt>::unsupported_pdu($context, $name, $value)
    }};
    ( $name:expr, $value:expr $(,)? ) => {{
        unsupported_pdu_err!($crate::function!(), $name, $value)
    }};
}

/// Creates a `PduError` with `Other` kind
///
/// Shorthand for
/// ```rust
/// <ironrdp_pdu::PduError as ironrdp_pdu::PduErrorExt>::other(context, description)
/// ```
/// and
/// ```rust
/// <ironrdp_pdu::PduError as ironrdp_pdu::PduErrorExt>::other($crate::function!(), description)
/// ```
#[macro_export]
macro_rules! other_err {
    ( $context:expr, $description:expr $(,)? ) => {{
        <$crate::PduError as $crate::PduErrorExt>::other($context, $description)
    }};
    ( $description:expr $(,)? ) => {{
        other_err!($crate::function!(), $description)
    }};
}

/// Creates a `PduError` with `Custom` kind and a source error attached to it
///
/// Shorthand for
/// ```rust
/// <ironrdp_pdu::PduError as ironrdp_pdu::PduErrorExt>::custom(context, source)
/// ```
/// and
/// ```rust
/// <ironrdp_pdu::PduError as ironrdp_pdu::PduErrorExt>::custom($crate::function!(), source)
/// ```
#[macro_export]
macro_rules! custom_err {
    ( $context:expr, $source:expr $(,)? ) => {{
        <$crate::PduError as $crate::PduErrorExt>::custom($context, $source)
    }};
    ( $source:expr $(,)? ) => {{
        custom_err!($crate::function!(), $source)
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
        $crate::ensure_size!(ctx: $crate::function!(), in: $buf, size: $expected)
    }};
}

#[macro_export]
macro_rules! ensure_fixed_part_size {
    (in: $buf:ident) => {{
        $crate::ensure_size!(ctx: $crate::function!(), in: $buf, size: Self::FIXED_PART_SIZE)
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
        $crate::cast_length!($crate::function!(), $field, $len)
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
        $crate::cast_int!($crate::function!(), $field, $len)
    }};
}

/// Asserts that constant expressions evaluate to `true`.
///
/// From <https://docs.rs/static_assertions/1.1.0/src/static_assertions/const_assert.rs.html#51-57>
#[macro_export]
macro_rules! const_assert {
    ($x:expr $(,)?) => {
        #[allow(unknown_lints, eq_op)]
        const _: [(); 0 - !{
            const ASSERT: bool = $x;
            ASSERT
        } as usize] = [];
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
            fn decode_owned(src: &mut ReadCursor<'_>) -> $crate::PduResult<Self> {
                <Self as $crate::PduDecode>::decode(src)
            }
        }
    };
}

/// Implements additional traits for a borrowing PDU and defines a static-bounded owned version.
#[macro_export]
macro_rules! impl_pdu_borrowing {
    ($pdu_ty:ident $(<$($lt:lifetime),+>)?, $owned_ty:ident) => {
        pub type $owned_ty = $pdu_ty<'static>;

        impl $crate::PduDecodeOwned for $owned_ty {
            fn decode_owned(src: &mut ReadCursor<'_>) -> $crate::PduResult<Self> {
                let pdu = <$pdu_ty $(<$($lt),+>)? as $crate::PduDecode>::decode(src)?;
                Ok($crate::IntoOwnedPdu::into_owned_pdu(pdu))
            }
        }
    };
}

/// Writes zeroes using as few `write_u*` calls as possible.
///
/// This is similar to `ironrdp_pdu::padding::write`, but the loop is optimized out when a single
/// operation is enough.
#[macro_export]
macro_rules! write_padding {
    ($dst:expr, 1) => {
        $dst.write_u8(0)
    };
    ($dst:expr, 2) => {
        $dst.write_u16(0)
    };
    ($dst:expr, 4) => {
        $dst.write_u32(0)
    };
    ($dst:expr, 8) => {
        $dst.write_u64(0)
    };
    ($dst:expr, $n:expr) => {
        $crate::padding::write($dst, $n)
    };
}

/// Moves read cursor, ignoring padding bytes.
///
/// This is similar to `ironrdp_pdu::padding::read`, only exists for consistency with `write_padding!`.
#[macro_export]
macro_rules! read_padding {
    ($src:expr, $n:expr) => {
        $crate::padding::read($src, $n)
    };
}

// FIXME: legacy macros below

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
            // This is a workaround for clippy false positive because
            // of macro expansion.
            #[allow(clippy::redundant_closure_call)]
            $f(val)?
        } else {
            return Ok(());
        }
    };
}
