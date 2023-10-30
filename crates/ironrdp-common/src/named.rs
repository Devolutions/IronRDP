/// A type with a static name.
pub trait StaticName {
    /// Name associated to this type.
    const NAME: &'static str;
}

/// A type from which a name can be retrieved.
pub trait GetName {
    /// Returns the name associated to this value.
    fn get_name(&self) -> &'static str;
}

impl<T: StaticName> GetName for T {
    fn get_name(&self) -> &'static str {
        T::NAME
    }
}

assert_obj_safe!(GetName);

/// Gets the name of this value.
pub fn get_name<T: GetName>(value: &T) -> &'static str {
    value.get_name()
}
