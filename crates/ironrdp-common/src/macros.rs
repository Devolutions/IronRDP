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
