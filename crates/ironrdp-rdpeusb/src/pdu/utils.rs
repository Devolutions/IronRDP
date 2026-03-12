/// An integer value that indicates the result or status of an operation.
///
/// * [MS-ERREF § 2.1 HRESULT][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-erref/0642cb2f-2075-4469-918c-4441e69c548a
pub type HResult = u32;

/// The [`CANCEL_REQUEST::request_id`] field. Represents the ID of a request previously sent via
/// IO_CONTROL, INTERNAL_IO_CONTROL, TRANSFER_IN_REQUEST, or TRANSFER_OUT_REQUEST message. Think of
/// this like an "umbrella" type for [`RequestIdIoctl`] and [`RequestIdTsUrb`].
pub type RequestId = u32;

/// Represents a request ID that uniquely identifies an `IO_CONTROL` or `INTERNAL_IO_CONTROL`
/// message.
pub type RequestIdIoctl = u32;

/// Represents a request ID that uniquely identifies a `TRANSFER_IN_REQUEST` or
/// `TRANSFER_OUT_REQUEST` message. 31 bits.
#[repr(transparent)]
#[derive(Debug, Clone, Copy)]
pub struct RequestIdTsUrb(u32);

impl From<u32> for RequestIdTsUrb {
    /// Construct a request ID for `TRANSFER_IN_REQUEST` or `TRANSFER_OUT_REQUEST`. Discards
    /// highest bit.
    fn from(value: u32) -> Self {
        Self(value & 0x7F_FF_FF_FF)
    }
}

impl From<RequestIdTsUrb> for u32 {
    fn from(value: RequestIdTsUrb) -> Self {
        value.0
    }
}

// TODO: This could be moved to ironrdp-core.
//
/// Ensures that a buffer has at least the payload size of a struct.
///
/// This macro is a specialized version of `ensure_size` that uses the
/// `PAYLOAD_SIZE` constant of the current struct.
///
/// # Examples
///
/// ```
/// use ironrdp_rdpeusb::ensure_payload_size;
///
/// struct MyStruct {
///     // ... fields
/// }
///
/// impl MyStruct {
///     const PAYLOAD_SIZE: usize = 20;
///
///     fn parse(buf: &[u8]) -> Result<Self, Error> {
///         ensure_payload_size!(in: buf);
///         // ... parsing logic
///     }
/// }
/// ```
///
/// # Note
///
/// This macro assumes that the current struct has a `PAYLOAD_SIZE` constant defined.
#[macro_export]
macro_rules! ensure_payload_size {
    (in: $buf:ident) => {{
        ironrdp_core::ensure_size!(ctx: ironrdp_core::function!(), in: $buf, size: Self::PAYLOAD_SIZE)
    }};
}
