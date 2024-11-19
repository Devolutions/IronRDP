#[cfg(feature = "alloc")]
use alloc::string::String;

use ironrdp_error::{Error, Source};

/// Trait for adding a source to an error type.
pub trait WithSource {
    /// Adds a source to the error.
    ///
    /// # Arguments
    ///
    /// * `source` - The source error to add.
    ///
    /// # Returns
    ///
    /// The error with the added source.
    #[must_use]
    fn with_source<E: Source>(self, source: E) -> Self;
}

impl<T> WithSource for Error<T> {
    fn with_source<E: Source>(self, source: E) -> Self {
        self.with_source(source)
    }
}

/// Trait for creating "not enough bytes" errors.
pub trait NotEnoughBytesErr {
    /// Creates a new "not enough bytes" error.
    ///
    /// # Arguments
    ///
    /// * `context` - The context in which the error occurred.
    /// * `received` - The number of bytes received.
    /// * `expected` - The number of bytes expected.
    ///
    /// # Returns
    ///
    /// A new error instance.
    fn not_enough_bytes(context: &'static str, received: usize, expected: usize) -> Self;
}

/// Helper function to create a "not enough bytes" error.
///
/// This function is a convenience wrapper around the `NotEnoughBytesErr` trait.
///
/// # Arguments
///
/// * `context` - The context in which the error occurred.
/// * `received` - The number of bytes received.
/// * `expected` - The number of bytes expected.
///
/// # Returns
///
/// A new error instance of type `T` that implements `NotEnoughBytesErr`.
pub fn not_enough_bytes_err<T: NotEnoughBytesErr>(context: &'static str, received: usize, expected: usize) -> T {
    T::not_enough_bytes(context, received, expected)
}

/// Trait for creating "invalid field" errors.
pub trait InvalidFieldErr {
    /// Creates a new "invalid field" error.
    ///
    /// # Arguments
    ///
    /// * `context` - The context in which the error occurred.
    /// * `field` - The name of the invalid field.
    /// * `reason` - The reason why the field is invalid.
    ///
    /// # Returns
    ///
    /// A new error instance.
    fn invalid_field(context: &'static str, field: &'static str, reason: &'static str) -> Self;
}

/// Helper function to create an "invalid field" error with a source.
///
/// This function is a convenience wrapper around the `InvalidFieldErr` and `WithSource` traits.
///
/// # Arguments
///
/// * `context` - The context in which the error occurred.
/// * `field` - The name of the invalid field.
/// * `reason` - The reason why the field is invalid.
/// * `source` - The source error to add.
///
/// # Returns
///
/// A new error instance of type `T` that implements both `InvalidFieldErr` and `WithSource`.
pub fn invalid_field_err_with_source<T: InvalidFieldErr + WithSource, E: Source>(
    context: &'static str,
    field: &'static str,
    reason: &'static str,
    source: E,
) -> T {
    T::invalid_field(context, field, reason).with_source(source)
}

/// Helper function to create an "invalid field" error.
pub fn invalid_field_err<T: InvalidFieldErr>(context: &'static str, field: &'static str, reason: &'static str) -> T {
    T::invalid_field(context, field, reason)
}

/// Trait for creating "unexpected message type" errors.
pub trait UnexpectedMessageTypeErr {
    /// Creates a new "unexpected message type" error.
    ///
    /// # Arguments
    ///
    /// * `context` - The context in which the error occurred.
    /// * `got` - The unexpected message type received.
    ///
    /// # Returns
    ///
    /// A new error instance.
    fn unexpected_message_type(context: &'static str, got: u8) -> Self;
}

/// Helper function to create an "unexpected message type" error.
///
/// This function is a convenience wrapper around the `UnexpectedMessageTypeErr` trait.
///
/// # Arguments
///
/// * `context` - The context in which the error occurred.
/// * `got` - The unexpected message type received.
///
/// # Returns
///
/// A new error instance of type `T` that implements `UnexpectedMessageTypeErr`.
pub fn unexpected_message_type_err<T: UnexpectedMessageTypeErr>(context: &'static str, got: u8) -> T {
    T::unexpected_message_type(context, got)
}

/// Trait for creating "unsupported version" errors.
pub trait UnsupportedVersionErr {
    /// Creates a new "unsupported version" error.
    ///
    /// # Arguments
    ///
    /// * `context` - The context in which the error occurred.
    /// * `got` - The unsupported version received.
    ///
    /// # Returns
    ///
    /// A new error instance.
    fn unsupported_version(context: &'static str, got: u8) -> Self;
}

/// Helper function to create an "unsupported version" error.
///
/// This function is a convenience wrapper around the `UnsupportedVersionErr` trait.
///
/// # Arguments
///
/// * `context` - The context in which the error occurred.
/// * `got` - The unsupported version received.
///
/// # Returns
///
/// A new error instance of type `T` that implements `UnsupportedVersionErr`.
pub fn unsupported_version_err<T: UnsupportedVersionErr>(context: &'static str, got: u8) -> T {
    T::unsupported_version(context, got)
}

/// Trait for creating "unsupported value" errors.
pub trait UnsupportedValueErr {
    /// Creates a new "unsupported value" error when the "alloc" feature is enabled.
    ///
    /// # Arguments
    ///
    /// * `context` - The context in which the error occurred.
    /// * `name` - The name of the unsupported value.
    /// * `value` - The unsupported value.
    ///
    /// # Returns
    ///
    /// A new error instance.
    #[cfg(feature = "alloc")]
    fn unsupported_value(context: &'static str, name: &'static str, value: String) -> Self;

    /// Creates a new "unsupported value" error when the "alloc" feature is disabled.
    ///
    /// # Arguments
    ///
    /// * `context` - The context in which the error occurred.
    /// * `name` - The name of the unsupported value.
    ///
    /// # Returns
    ///
    /// A new error instance.
    #[cfg(not(feature = "alloc"))]
    fn unsupported_value(context: &'static str, name: &'static str) -> Self;
}

/// Helper function to create an "unsupported value" error when the "alloc" feature is enabled.
///
/// This function is a convenience wrapper around the `UnsupportedValueErr` trait.
///
/// # Arguments
///
/// * `context` - The context in which the error occurred.
/// * `name` - The name of the unsupported value.
/// * `value` - The unsupported value.
///
/// # Returns
///
/// A new error instance of type `T` that implements `UnsupportedValueErr`.]
#[cfg(feature = "alloc")]
pub fn unsupported_value_err<T: UnsupportedValueErr>(context: &'static str, name: &'static str, value: String) -> T {
    T::unsupported_value(context, name, value)
}

/// Helper function to create an "unsupported value" error.
#[cfg(not(feature = "alloc"))]
pub fn unsupported_value_err<T: UnsupportedValueErr>(context: &'static str, name: &'static str) -> T {
    T::unsupported_value(context, name)
}

/// Trait for creating generic "other" errors.
pub trait OtherErr {
    /// Creates a new generic "other" error.
    ///
    /// # Arguments
    ///
    /// * `context` - The context in which the error occurred.
    /// * `description` - A description of the error.
    ///
    /// # Returns
    ///
    /// A new error instance.
    fn other(context: &'static str, description: &'static str) -> Self;
}

/// Helper function to create a generic "other" error.
///
/// This function is a convenience wrapper around the `OtherErr` trait.
///
/// # Arguments
///
/// * `context` - The context in which the error occurred.
/// * `description` - A description of the error.
///
/// # Returns
///
/// A new error instance of type `T` that implements `OtherErr`.
pub fn other_err<T: OtherErr>(context: &'static str, description: &'static str) -> T {
    T::other(context, description)
}

/// Helper function to create a generic "other" error with a source.
///
/// This function is a convenience wrapper around the `OtherErr` and `WithSource` traits.
///
/// # Arguments
///
/// * `context` - The context in which the error occurred.
/// * `description` - A description of the error.
/// * `source` - The source error to add.
///
/// # Returns
///
/// A new error instance of type `T` that implements both `OtherErr` and `WithSource`.
pub fn other_err_with_source<T: OtherErr + WithSource, E: Source>(
    context: &'static str,
    description: &'static str,
    source: E,
) -> T {
    T::other(context, description).with_source(source)
}
