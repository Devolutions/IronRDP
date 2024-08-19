use core::any::Any;

/// Implement [`AsAny`] for the given type.
#[macro_export]
macro_rules! impl_as_any {
    ($t:ty) => {
        impl $crate::AsAny for $t {
            #[inline]
            fn as_any(&self) -> &dyn core::any::Any {
                self
            }

            #[inline]
            fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
                self
            }
        }
    };
}

/// Type information (`TypeId`) may be retrieved at runtime for this type.
pub trait AsAny: 'static {
    /// Returns a reference to the type information for this type.
    fn as_any(&self) -> &dyn Any;

    /// Returns a mutable reference to the type information for this type.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
