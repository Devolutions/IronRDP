//! Helper macros for PDU encoding and decoding
//!
//! Some are exported and available to external crates

#[macro_export]
macro_rules! ensure_size {
    (name: $name:expr, in: $buf:ident, size: $expected:expr) => {{
        let received = $buf.len();
        let expected = $expected;
        if !(received >= expected) {
            return Err($crate::Error::NotEnoughBytes {
                name: $name,
                received,
                expected,
            });
        }
    }};
    (in: $buf:ident, size: $expected:expr) => {{
        $crate::ensure_size!(name: Self::NAME, in: $buf, size: $expected)
    }};
}

#[macro_export]
macro_rules! ensure_fixed_part_size {
    (in: $buf:ident) => {{
        $crate::ensure_size!(name: Self::NAME, in: $buf, size: Self::FIXED_PART_SIZE)
    }};
}

#[macro_export]
macro_rules! cast_length {
    ($len:expr, $name:expr, $field:expr) => {{
        $len.try_into().map_err(|_| $crate::Error::InvalidMessage {
            name: $name,
            field: $field,
            reason: "too many elements",
        })
    }};
    ($len:expr, $field:expr) => {{
        $crate::cast_length!($len, <Self as $crate::Pdu>::NAME, $field)
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
            fn decode_owned(src: &mut $crate::cursor::ReadCursor<'_>) -> $crate::Result<Self> {
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
            fn decode_owned(src: &mut $crate::cursor::ReadCursor<'_>) -> $crate::Result<Self> {
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
