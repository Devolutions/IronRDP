/// Asserts that the traits support dynamic dispatch.
///
/// From <https://docs.rs/static_assertions/1.1.0/src/static_assertions/assert_obj_safe.rs.html#72-76>
#[macro_export]
macro_rules! assert_obj_safe {
    ($($xs:path),+ $(,)?) => {
        $(const _: Option<&dyn $xs> = None;)+
    };
}

/// Asserts that the type implements _all_ of the given traits.
///
/// From <https://docs.rs/static_assertions/1.1.0/src/static_assertions/assert_impl.rs.html#113-121>
#[macro_export]
macro_rules! assert_impl {
    ($type:ty: $($trait:path),+ $(,)?) => {
        const _: fn() = || {
            // Only callable when `$type` implements all traits in `$($trait)+`.
            fn assert_impl_all<T: ?Sized $(+ $trait)+>() {}
            assert_impl_all::<$type>();
        };
    };
}

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

/// Creates a "not enough bytes" error with context information.
///
/// This macro generates an error indicating that there weren't enough bytes
/// in a buffer for a particular operation.
///
/// # Arguments
///
/// * `context` - The context in which the error occurred (optional)
/// * `received` - The number of bytes actually received
/// * `expected` - The number of bytes expected
///
/// # Examples
///
/// ```
/// use ironrdp_core::not_enough_bytes_err;
///
/// let err = not_enough_bytes_err!("parsing header", 5, 10);
/// ```
///
/// # Note
///
/// If the context is not provided, it will use the current function name.
#[macro_export]
macro_rules! not_enough_bytes_err {
    // offset extracted from cursor.pos()
    ( $context:expr, $received:expr, $expected:expr, in: $cursor:expr $(,)? ) => {{
        $crate::not_enough_bytes_err($context, $received, $expected, $cursor.pos())
    }};
    ( $received:expr, $expected:expr, in: $cursor:expr $(,)? ) => {{
        $crate::not_enough_bytes_err!($crate::function!(), $received, $expected, in: $cursor)
    }};
    // explicit offset (use 0 when the producer has no stream-cursor access)
    ( $context:expr, $received:expr, $expected:expr, at: $offset:expr $(,)? ) => {{
        $crate::not_enough_bytes_err($context, $received, $expected, $offset)
    }};
    ( $received:expr, $expected:expr, at: $offset:expr $(,)? ) => {{
        $crate::not_enough_bytes_err!($crate::function!(), $received, $expected, at: $offset)
    }};
}

/// Creates an "invalid field" error with context information.
///
/// This macro generates an error indicating that a field in a data structure
/// or input is invalid for some reason.
///
/// # Arguments
///
/// * `context` - The context in which the error occurred (optional)
/// * `field` - The name of the invalid field
/// * `reason` - The reason why the field is invalid
///
/// # Examples
///
/// ```
/// use ironrdp_core::invalid_field_err;
///
/// let err = invalid_field_err!("user input", "Age", "must be positive");
/// ```
///
/// # Note
///
/// If the context is not provided, it will use the current function name.
#[macro_export]
macro_rules! invalid_field_err {
    ( $context:expr, $field:expr, $reason:expr, in: $cursor:expr $(,)? ) => {{
        $crate::invalid_field_err($context, $field, $reason, $cursor.pos())
    }};
    ( $field:expr, $reason:expr, in: $cursor:expr $(,)? ) => {{
        $crate::invalid_field_err!($crate::function!(), $field, $reason, in: $cursor)
    }};
    ( $context:expr, $field:expr, $reason:expr, at: $offset:expr $(,)? ) => {{
        $crate::invalid_field_err($context, $field, $reason, $offset)
    }};
    ( $field:expr, $reason:expr, at: $offset:expr $(,)? ) => {{
        $crate::invalid_field_err!($crate::function!(), $field, $reason, at: $offset)
    }};
}

/// Creates an "unexpected message type" error with context information.
///
/// This macro generates an error indicating that an unexpected message type
/// was received in a particular context.
///
/// # Arguments
///
/// * `context` - The context in which the error occurred (optional)
/// * `got` - The unexpected message type that was received
///
/// # Examples
///
/// ```
/// use ironrdp_core::unexpected_message_type_err;
///
/// let err = unexpected_message_type_err!("Erase");
/// ```
///
/// # Note
///
/// If the context is not provided, it will use the current function name.
#[macro_export]
macro_rules! unexpected_message_type_err {
    ( $context:expr, $got:expr, in: $cursor:expr $(,)? ) => {{
        $crate::unexpected_message_type_err($context, $got, $cursor.pos())
    }};
    ( $got:expr, in: $cursor:expr $(,)? ) => {{
        $crate::unexpected_message_type_err!($crate::function!(), $got, in: $cursor)
    }};
    ( $context:expr, $got:expr, at: $offset:expr $(,)? ) => {{
        $crate::unexpected_message_type_err($context, $got, $offset)
    }};
    ( $got:expr, at: $offset:expr $(,)? ) => {{
        $crate::unexpected_message_type_err!($crate::function!(), $got, at: $offset)
    }};
}

/// Creates an "unsupported version" error with context information.
///
/// This macro generates an error indicating that an unsupported version
/// was encountered in a particular context.
///
/// # Arguments
///
/// * `context` - The context in which the error occurred (optional)
/// * `got` - The unsupported version that was encountered
///
/// # Examples
///
/// ```
/// use ironrdp_core::unsupported_version_err;
///
/// let err = unsupported_version_err!("protocol version", 12);
/// ```
///
/// # Note
///
/// If the context is not provided, it will use the current function name.
#[macro_export]
macro_rules! unsupported_version_err {
    ( $context:expr, $got:expr, in: $cursor:expr $(,)? ) => {{
        $crate::unsupported_version_err($context, $got, $cursor.pos())
    }};
    ( $got:expr, in: $cursor:expr $(,)? ) => {{
        $crate::unsupported_version_err!($crate::function!(), $got, in: $cursor)
    }};
    ( $context:expr, $got:expr, at: $offset:expr $(,)? ) => {{
        $crate::unsupported_version_err($context, $got, $offset)
    }};
    ( $got:expr, at: $offset:expr $(,)? ) => {{
        $crate::unsupported_version_err!($crate::function!(), $got, at: $offset)
    }};
}

/// Creates an "unsupported value" error with context information.
///
/// This macro generates an error indicating that an unsupported value
/// was encountered for a specific named parameter or field.
///
/// # Arguments
///
/// * `context` - The context in which the error occurred (optional)
/// * `name` - The name of the parameter or field with the unsupported value
/// * `value` - The unsupported value that was encountered
///
/// # Examples
///
/// ```
/// use ironrdp_core::unsupported_value_err;
///
/// let err = unsupported_value_err!("configuration", "log_level", "EXTREME");
/// ```
///
/// # Note
///
/// If the context is not provided, it will use the current function name.
#[macro_export]
macro_rules! unsupported_value_err {
    ( $context:expr, $name:expr, $value:expr, in: $cursor:expr $(,)? ) => {{
        $crate::unsupported_value_err($context, $name, $value, $cursor.pos())
    }};
    ( $name:expr, $value:expr, in: $cursor:expr $(,)? ) => {{
        $crate::unsupported_value_err!($crate::function!(), $name, $value, in: $cursor)
    }};
    ( $context:expr, $name:expr, $value:expr, at: $offset:expr $(,)? ) => {{
        $crate::unsupported_value_err($context, $name, $value, $offset)
    }};
    ( $name:expr, $value:expr, at: $offset:expr $(,)? ) => {{
        $crate::unsupported_value_err!($crate::function!(), $name, $value, at: $offset)
    }};
}

/// Creates a generic "other" error with optional context and source information.
///
/// This macro generates a generic error that can include a description, context,
/// and an optional source error. It's useful for creating custom errors or
/// wrapping other errors with additional context.
///
/// # Arguments
///
/// * `description` - A description of the error (optional)
/// * `context` - The context in which the error occurred (optional)
/// * `source` - The source error, if this error is wrapping another (optional)
///
/// # Examples
///
/// ```
/// use ironrdp_core::other_err;
///
/// // With description and source
/// let source_err = std::io::Error::new(std::io::ErrorKind::Other, "Source error");
/// let err = other_err!("Something went wrong", source: source_err);
///
/// // With context and description
/// let err = other_err!("parsing input", "Unexpected end of file");
///
/// // With only description
/// let err = other_err!("Operation failed");
///
/// // With only source
/// let err = other_err!(source: std::io::Error::new(std::io::ErrorKind::Other, "IO error"));
/// ```
///
/// # Note
///
/// If the context is not provided, it will use the current function name.
#[macro_export]
macro_rules! other_err {
    ( $context:expr, source: $source:expr $(,)? ) => {{
        $crate::other_err_with_source($context, "", $source)
    }};
    ( $context:expr, $description:expr $(,)? ) => {{
        $crate::other_err($context, $description)
    }};
    ( source: $source:expr $(,)? ) => {{
        $crate::other_err!($crate::function!(), source: $source)
    }};
    ( $description:expr $(,)? ) => {{
        $crate::other_err!($crate::function!(), $description)
    }};
}

/// Ensures that a buffer has at least the expected size.
///
/// This macro checks if the buffer length is greater than or equal to the expected size.
/// If not, it returns a "not enough bytes" error.
///
/// # Arguments
///
/// * `ctx` - The context for the error message (optional)
/// * `buf` - The buffer to check
/// * `expected` - The expected minimum size of the buffer
///
/// # Examples
///
/// ```
/// use ironrdp_core::ensure_size;
///
/// fn parse_data(buf: &[u8]) -> Result<(), Error> {
///     ensure_size!(in: buf, size: 10);
///     // ... rest of the parsing logic
///     Ok(())
/// }
/// ```
///
/// # Note
///
/// If the context is not provided, it will use the current function name.
#[macro_export]
macro_rules! ensure_size {
    (ctx: $ctx:expr, in: $buf:ident, size: $expected:expr) => {{
        let received = $buf.len();
        let expected = $expected;
        if !(received >= expected) {
            // `$buf` is always a `ReadCursor` or `WriteCursor` in practice;
            // both expose `.pos()`. The previous `encode_string` slice path
            // was refactored to take a `WriteCursor` so this invariant holds.
            return Err($crate::not_enough_bytes_err($ctx, received, expected, $buf.pos()));
        }
    }};
    (in: $buf:ident, size: $expected:expr) => {{
        $crate::ensure_size!(ctx: $crate::function!(), in: $buf, size: $expected)
    }};
}

/// Ensures that a buffer has at least the fixed part size of a struct.
///
/// This macro is a specialized version of `ensure_size` that uses the
/// `FIXED_PART_SIZE` constant of the current struct.
///
/// # Examples
///
/// ```
/// use ironrdp_core::ensure_fixed_part_size;
///
/// struct MyStruct {
///     // ... fields
/// }
///
/// impl MyStruct {
///     const FIXED_PART_SIZE: usize = 20;
///
///     fn parse(buf: &[u8]) -> Result<Self, Error> {
///         ensure_fixed_part_size!(in: buf);
///         // ... parsing logic
///     }
/// }
/// ```
///
/// # Note
///
/// This macro assumes that the current struct has a `FIXED_PART_SIZE` constant defined.
#[macro_export]
macro_rules! ensure_fixed_part_size {
    (in: $buf:ident) => {{
        $crate::ensure_size!(ctx: $crate::function!(), in: $buf, size: Self::FIXED_PART_SIZE)
    }};
}

/// Safely casts a length to a different integer type.
///
/// This macro attempts to convert a length value to a different integer type,
/// returning an error if the conversion fails due to overflow.
///
/// # Arguments
///
/// * `ctx` - The context for the error message (optional)
/// * `field` - The name of the field being cast
/// * `len` - The length value to cast
/// * `in: $cursor` or `at: $offset` - cursor whose `pos()` to read, or an
///   explicit byte offset (`0` if the producer has no stream-cursor access)
///
/// # Examples
///
/// ```ignore
/// // Inside a decode method with a `src: &mut ReadCursor<'_>` parameter:
/// let len: u16 = cast_length!("data length", data.len(), in: src)?;
///
/// // Outside any decode context (e.g. a getter on a decoded struct):
/// let len: u16 = cast_length!("data length", data.len(), at: 0)?;
/// ```
///
/// # Note
///
/// If the context is not provided, it will use the current function name.
#[macro_export]
macro_rules! cast_length {
    // offset extracted from cursor.pos()
    ($ctx:expr, $field:expr, $len:expr, in: $cursor:expr $(,)?) => {{
        let __offset = $cursor.pos();
        $len.try_into()
            .map_err(|e| $crate::invalid_field_err_with_source($ctx, $field, "too many elements", __offset, e))
    }};
    ($field:expr, $len:expr, in: $cursor:expr $(,)?) => {{
        $crate::cast_length!($crate::function!(), $field, $len, in: $cursor)
    }};
    // explicit offset (use 0 only when the producer has no stream-cursor access)
    ($ctx:expr, $field:expr, $len:expr, at: $offset:expr $(,)?) => {{
        $len.try_into()
            .map_err(|e| $crate::invalid_field_err_with_source($ctx, $field, "too many elements", $offset, e))
    }};
    ($field:expr, $len:expr, at: $offset:expr $(,)?) => {{
        $crate::cast_length!($crate::function!(), $field, $len, at: $offset)
    }};
}

/// Safely casts an integer to a different integer type.
///
/// This macro attempts to convert an integer value to a different integer type,
/// returning an error if the conversion fails due to out-of-range issues.
///
/// # Arguments
///
/// * `ctx` - The context for the error message (optional)
/// * `field` - The name of the field being cast
/// * `len` - The integer value to cast
/// * `in: $cursor` or `at: $offset` - cursor whose `pos()` to read, or an
///   explicit byte offset (`0` if the producer has no stream-cursor access)
///
/// # Examples
///
/// ```ignore
/// // Inside a decode method with a `src: &mut ReadCursor<'_>` parameter:
/// let v: i32 = cast_int!("input value", value, in: src)?;
///
/// // Outside any decode context:
/// let v: i32 = cast_int!("input value", value, at: 0)?;
/// ```
///
/// # Note
///
/// If the context is not provided, it will use the current function name.
#[macro_export]
macro_rules! cast_int {
    // offset extracted from cursor.pos()
    ($ctx:expr, $field:expr, $len:expr, in: $cursor:expr $(,)?) => {{
        let __offset = $cursor.pos();
        $len.try_into().map_err(|e| {
            $crate::invalid_field_err_with_source($ctx, $field, "out of range integral type conversion", __offset, e)
        })
    }};
    ($field:expr, $len:expr, in: $cursor:expr $(,)?) => {{
        $crate::cast_int!($crate::function!(), $field, $len, in: $cursor)
    }};
    // explicit offset (use 0 only when the producer has no stream-cursor access)
    ($ctx:expr, $field:expr, $len:expr, at: $offset:expr $(,)?) => {{
        $len.try_into().map_err(|e| {
            $crate::invalid_field_err_with_source($ctx, $field, "out of range integral type conversion", $offset, e)
        })
    }};
    ($field:expr, $len:expr, at: $offset:expr $(,)?) => {{
        $crate::cast_int!($crate::function!(), $field, $len, at: $offset)
    }};
}

/// Writes zeroes using as few `write_u*` calls as possible.
///
/// This is similar to `ironrdp_core::padding::write`, but the loop is optimized out when a single
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
        $crate::write_padding($dst, $n)
    };
}

/// Moves read cursor, ignoring padding bytes.
///
/// This is similar to `ironrdp_pdu::padding::read`, only exists for consistency with `write_padding!`.
#[macro_export]
macro_rules! read_padding {
    ($src:expr, $n:expr) => {
        $crate::read_padding($src, $n)
    };
}
