//! Externally-lengthed Unicode string fields.
//!
//! Used for strings whose wire length is given by a sibling field, not adjacent to
//! the string itself. The length is provided externally at decode time, either as a
//! WCHAR count (via [`UnframedString::decode`]) or as a byte length
//! (via [`UnframedString::decode_from_byte_len`]).

use alloc::borrow::Cow;
#[cfg(not(feature = "std"))]
use alloc::string::String;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use core::fmt;

use ironrdp_core::{DecodeResult, EncodeResult, ReadCursor, WriteCursor, ensure_size, invalid_field_err};

use crate::InvalidUtf16;
use crate::repr::StringRepr;

/// A UTF-16LE string with no self-describing length prefix on the wire.
///
/// The length must be provided externally (typically from a sibling field in the same PDU).
/// Trailing null code units are stripped on decode.
///
/// Wire data is accepted as-is with no UTF-16 validation at decode time. Call [`to_native`]
/// to validate and convert to a Rust `str`, or [`to_native_lossy`] to accept any byte
/// sequence with lone-surrogate replacement.
///
/// Use [`UnframedString::decode`] or [`UnframedString::decode_from_byte_len`]
/// to decode, and [`UnframedString::encode_into`] / [`UnframedString::wire_size`]
/// to encode.
///
/// This type intentionally does not implement [`ironrdp_core::Encode`] or
/// [`ironrdp_core::DecodeOwned`]: there is no self-describing length, so the standard
/// encode/decode interface does not apply.
///
/// [`to_native`]: UnframedString::to_native
/// [`to_native_lossy`]: UnframedString::to_native_lossy
pub struct UnframedString(StringRepr);

impl UnframedString {
    /// Creates an `UnframedString` from a native Rust string.
    pub fn new(s: impl Into<String>) -> Self {
        Self(StringRepr::from_native(s.into()))
    }

    /// Creates an `UnframedString` from raw UTF-16LE wire bytes.
    ///
    /// Returns `None` if `bytes` has odd length. Trailing null code units are stripped.
    /// This is a convenience wrapper around [`utf16le_bytes_to_units`] + [`from_wire_units`].
    ///
    /// [`utf16le_bytes_to_units`]: crate::utf16le_bytes_to_units
    /// [`from_wire_units`]: UnframedString::from_wire_units
    pub fn from_utf16le_bytes(bytes: &[u8]) -> Option<Self> {
        crate::utf16le_bytes_to_units(bytes).map(Self::from_wire_units)
    }

    /// Creates an `UnframedString` from pre-parsed UTF-16 code units.
    ///
    /// Trailing null code units are stripped. This is the low-level counterpart to
    /// [`decode`] for callers that already have units from [`utf16le_bytes_to_units`].
    ///
    /// [`decode`]: UnframedString::decode
    /// [`utf16le_bytes_to_units`]: crate::utf16le_bytes_to_units
    pub fn from_wire_units(units: Vec<u16>) -> Self {
        let mut units = units;
        let end = units.iter().rposition(|&u| u != 0).map_or(0, |i| i + 1);
        units.truncate(end);
        Self(StringRepr::from_wire_units(units))
    }

    /// Tries to return the string content as a Rust `str`.
    ///
    /// Returns [`InvalidUtf16`] if the wire data contains a lone surrogate.
    /// For strings decoded from the wire, this allocates a new `String`.
    /// For strings constructed from native Rust code, this is a zero-cost borrow.
    pub fn to_native(&self) -> Result<Cow<'_, str>, InvalidUtf16> {
        self.0.to_native()
    }

    /// Returns the string content, replacing any lone surrogates with U+FFFD.
    ///
    /// For strings decoded from the wire, this allocates a new `String`.
    /// For strings constructed from native Rust code, this is a zero-cost borrow.
    pub fn to_native_lossy(&self) -> Cow<'_, str> {
        self.0.to_native_lossy()
    }

    /// Returns the number of UTF-16 code units (WCHARs) in this string.
    ///
    /// O(1) for wire-decoded strings, O(n) for natively-constructed strings.
    pub fn utf16_len(&self) -> usize {
        self.0.utf16_len()
    }

    /// Returns the wire byte length of this string (`utf16_len() * 2`).
    ///
    /// Does **not** include a null terminator or any length prefix.
    /// The caller is responsible for tracking this value alongside the string.
    pub fn wire_size(&self) -> usize {
        self.0.utf16_byte_len()
    }

    /// Consumes `self` and returns a validated native `String`.
    ///
    /// Zero-cost when the value was constructed from a native Rust string.
    /// Validates and allocates when the value was decoded from the wire.
    /// Returns [`InvalidUtf16`] if the wire data contains a lone surrogate.
    pub fn into_native(self) -> Result<String, InvalidUtf16> {
        self.0.into_native()
    }

    /// Returns the UTF-16 code units of this string.
    ///
    /// For wire-decoded strings, this is a zero-cost borrow of the stored units.
    /// For strings constructed from native Rust code, this encodes and allocates.
    pub fn to_wire_units(&self) -> Cow<'_, [u16]> {
        self.0.to_wire_units()
    }

    /// Consumes `self` and returns the UTF-16 code units of this string.
    ///
    /// Zero-cost when the value was decoded from the wire (moves the internal buffer).
    /// Encodes and allocates when the value was constructed from a native string.
    pub fn into_wire_units(self) -> Vec<u16> {
        self.0.into_wire_units()
    }

    /// Consumes `self` and returns the raw UTF-16LE bytes of the string content.
    ///
    /// Zero-cost when the value was decoded from the wire (moves the internal buffer).
    /// Encodes to UTF-16LE and allocates when the value was constructed from a native string.
    pub fn into_wire(self) -> Vec<u8> {
        self.0.into_wire()
    }

    /// Decodes a UTF-16LE string from the next `wchar_count` code units in `src`.
    ///
    /// Trailing null code units are stripped. Returns a `DecodeError` if the source
    /// contains fewer than `wchar_count * 2` bytes.
    pub fn decode(src: &mut ReadCursor<'_>, wchar_count: usize) -> DecodeResult<Self> {
        let byte_count = wchar_count
            .checked_mul(2)
            .ok_or_else(|| invalid_field_err!("wchar_count", "character count overflow"))?;
        ensure_size!(in: src, size: byte_count);

        let slice = src.read_slice(byte_count);
        let units = crate::repr::le_bytes_to_units_strip_nulls(slice);
        Ok(Self(StringRepr::from_wire_units(units)))
    }

    /// Decodes a UTF-16LE string from the next `byte_len` bytes in `src`.
    ///
    /// Returns a `DecodeError` if `byte_len` is odd (UTF-16LE is always 2 bytes per code unit).
    /// Otherwise equivalent to `decode(src, byte_len / 2)`.
    pub fn decode_from_byte_len(src: &mut ReadCursor<'_>, byte_len: usize) -> DecodeResult<Self> {
        if byte_len % 2 != 0 {
            return Err(invalid_field_err!("byte_len", "odd byte count for utf-16 string field"));
        }
        Self::decode(src, byte_len / 2)
    }

    /// Encodes the string content into `dst` as UTF-16LE code units.
    ///
    /// Does **not** write a null terminator or any length prefix.
    /// Returns `EncodeResult` for consistency with the rest of the crate.
    pub fn encode_into(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.wire_size());
        let wire_bytes = self.0.as_wire_bytes();
        dst.write_slice(&wire_bytes);
        Ok(())
    }
}

impl From<String> for UnframedString {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for UnframedString {
    fn from(s: &str) -> Self {
        Self::new(s.to_owned())
    }
}

impl TryFrom<UnframedString> for String {
    type Error = InvalidUtf16;

    fn try_from(s: UnframedString) -> Result<Self, Self::Error> {
        s.0.into_native()
    }
}

impl fmt::Display for UnframedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.to_native_lossy(), f)
    }
}

impl fmt::Debug for UnframedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UnframedString({:?})", self.0)
    }
}

impl Clone for UnframedString {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl PartialEq for UnframedString {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for UnframedString {}
