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
    /// * `offset` - Byte offset in the input stream where the shortage was detected.
    ///   Callers without stream-cursor access pass `0`.
    ///
    /// # Returns
    ///
    /// A new error instance.
    fn not_enough_bytes(context: &'static str, received: usize, expected: usize, offset: usize) -> Self;
}

/// Helper function to create a "not enough bytes" error.
///
/// This function is a convenience wrapper around the `NotEnoughBytesErr` trait.
pub fn not_enough_bytes_err<T: NotEnoughBytesErr>(
    context: &'static str,
    received: usize,
    expected: usize,
    offset: usize,
) -> T {
    T::not_enough_bytes(context, received, expected, offset)
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
    /// * `offset` - Byte offset in the input stream where the field was decoded.
    ///   Callers without stream-cursor access pass `0`.
    ///
    /// # Returns
    ///
    /// A new error instance.
    fn invalid_field(context: &'static str, field: &'static str, reason: &'static str, offset: usize) -> Self;
}

/// Helper function to create an "invalid field" error with a source.
pub fn invalid_field_err_with_source<T: InvalidFieldErr + WithSource, E: Source>(
    context: &'static str,
    field: &'static str,
    reason: &'static str,
    offset: usize,
    source: E,
) -> T {
    T::invalid_field(context, field, reason, offset).with_source(source)
}

/// Helper function to create an "invalid field" error.
pub fn invalid_field_err<T: InvalidFieldErr>(
    context: &'static str,
    field: &'static str,
    reason: &'static str,
    offset: usize,
) -> T {
    T::invalid_field(context, field, reason, offset)
}

/// Trait for creating "unexpected message type" errors.
pub trait UnexpectedMessageTypeErr {
    /// Creates a new "unexpected message type" error.
    ///
    /// # Arguments
    ///
    /// * `context` - The context in which the error occurred.
    /// * `got` - The unexpected message type received.
    /// * `offset` - Byte offset in the input stream where the type was read.
    ///   Callers without stream-cursor access pass `0`.
    fn unexpected_message_type(context: &'static str, got: u8, offset: usize) -> Self;
}

/// Helper function to create an "unexpected message type" error.
pub fn unexpected_message_type_err<T: UnexpectedMessageTypeErr>(context: &'static str, got: u8, offset: usize) -> T {
    T::unexpected_message_type(context, got, offset)
}

/// Trait for creating "unsupported version" errors.
pub trait UnsupportedVersionErr {
    /// Creates a new "unsupported version" error.
    ///
    /// # Arguments
    ///
    /// * `context` - The context in which the error occurred.
    /// * `got` - The unsupported version received.
    /// * `offset` - Byte offset in the input stream where the version was read.
    ///   Callers without stream-cursor access pass `0`.
    fn unsupported_version(context: &'static str, got: u8, offset: usize) -> Self;
}

/// Helper function to create an "unsupported version" error.
pub fn unsupported_version_err<T: UnsupportedVersionErr>(context: &'static str, got: u8, offset: usize) -> T {
    T::unsupported_version(context, got, offset)
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
    /// * `offset` - Byte offset in the input stream where the value was read.
    ///   Callers without stream-cursor access pass `0`.
    #[cfg(feature = "alloc")]
    fn unsupported_value(context: &'static str, name: &'static str, value: String, offset: usize) -> Self;

    /// Creates a new "unsupported value" error when the "alloc" feature is disabled.
    ///
    /// # Arguments
    ///
    /// * `context` - The context in which the error occurred.
    /// * `name` - The name of the unsupported value.
    /// * `offset` - Byte offset in the input stream where the value was read.
    ///   Callers without stream-cursor access pass `0`.
    #[cfg(not(feature = "alloc"))]
    fn unsupported_value(context: &'static str, name: &'static str, offset: usize) -> Self;
}

/// Helper function to create an "unsupported value" error when the "alloc" feature is enabled.
#[cfg(feature = "alloc")]
pub fn unsupported_value_err<T: UnsupportedValueErr>(
    context: &'static str,
    name: &'static str,
    value: String,
    offset: usize,
) -> T {
    T::unsupported_value(context, name, value, offset)
}

/// Helper function to create an "unsupported value" error.
#[cfg(not(feature = "alloc"))]
pub fn unsupported_value_err<T: UnsupportedValueErr>(context: &'static str, name: &'static str, offset: usize) -> T {
    T::unsupported_value(context, name, offset)
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
