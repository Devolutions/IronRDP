//! Fixed-size Unicode string fields.
//!
//! Used for RDP fields whose wire representation occupies a statically-known number
//! of WCHARs, such as `clientName` (16 WCHARs = 32 bytes) or `fileName` (260 WCHARs = 520 bytes).

use alloc::borrow::Cow;
#[cfg(not(feature = "std"))]
use alloc::string::String;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use core::fmt;

use ironrdp_core::{DecodeOwned, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_size};

use crate::repr::StringRepr;
use crate::{InvalidUtf16, check_invariant, utf16_code_units};

// ── Error type ────────────────────────────────────────────────────────────────

/// Error returned when a string is too long for a [`FixedSizeUnicodeString`] field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringTooLong {
    /// Maximum number of UTF-16 code units the field can hold (excluding the null terminator slot).
    pub max_code_units: usize,
    /// Actual number of UTF-16 code units in the string.
    pub actual_code_units: usize,
}

impl fmt::Display for StringTooLong {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "string too long: {} code units, maximum is {}",
            self.actual_code_units, self.max_code_units
        )
    }
}

#[cfg(feature = "std")]
impl core::error::Error for StringTooLong {}

// ── FixedSizeUnicodeString ────────────────────────────────────────────────────

/// A UTF-16LE string occupying exactly `WCHAR_COUNT` code units on the wire, zero-padded
/// if shorter.
///
/// Strings requiring more than `WCHAR_COUNT - 1` code units are rejected on construction
/// (one slot is reserved for the null terminator). Trailing null terminators and zero
/// padding are stripped on decode.
///
/// Wire data is accepted as-is with no UTF-16 validation at decode time. Call [`to_native`]
/// to validate and convert to a Rust `str`, or [`to_native_lossy`] to accept any byte
/// sequence with lone-surrogate replacement.
///
/// The wire byte size is always [`WIRE_SIZE`](FixedSizeUnicodeString::WIRE_SIZE) = `WCHAR_COUNT * 2`.
///
/// # Common instantiations
///
/// | Type alias                           | `WCHAR_COUNT` | Wire bytes | Spec field |
/// |--------------------------------------|---------------|------------|------------|
/// | `FixedSizeUnicodeString<16>`         | 16            | 32         | `clientName` ([MS-RDPBCGR] §2.2.1.3.2) |
/// | `FixedSizeUnicodeString<32>`         | 32            | 64         | `StandardName`, `DaylightName` ([MS-RDPBCGR] §2.2.1.11.1.1.1) |
/// | `FixedSizeUnicodeString<260>`        | 260           | 520        | `fileName`, `applicationId` |
///
/// [`to_native`]: FixedSizeUnicodeString::to_native
/// [`to_native_lossy`]: FixedSizeUnicodeString::to_native_lossy
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
pub struct FixedSizeUnicodeString<const WCHAR_COUNT: usize>(
    /// INVARIANT: `utf16_code_units` of the stored string is `< WCHAR_COUNT`.
    StringRepr,
);

impl<const WCHAR_COUNT: usize> FixedSizeUnicodeString<WCHAR_COUNT> {
    /// Wire byte size: always `WCHAR_COUNT * 2` bytes.
    pub const WIRE_SIZE: usize = WCHAR_COUNT * 2;

    /// Creates a `FixedSizeUnicodeString` from raw UTF-16LE wire bytes.
    ///
    /// Returns `None` if `bytes` has odd length, or `Err(`[`StringTooLong`]`)` if the
    /// content exceeds `WCHAR_COUNT - 1` code units after stripping trailing nulls.
    /// This is a convenience wrapper around [`utf16le_bytes_to_units`] + [`from_wire_units`].
    ///
    /// [`utf16le_bytes_to_units`]: crate::utf16le_bytes_to_units
    /// [`from_wire_units`]: FixedSizeUnicodeString::from_wire_units
    pub fn from_utf16le_bytes(bytes: &[u8]) -> Option<Result<Self, StringTooLong>> {
        crate::utf16le_bytes_to_units(bytes).map(Self::from_wire_units)
    }

    /// Creates a `FixedSizeUnicodeString` from pre-parsed UTF-16 code units.
    ///
    /// Trailing null and zero-padding code units are stripped. Returns [`StringTooLong`]
    /// if the content exceeds `WCHAR_COUNT - 1` code units after stripping. This is
    /// the low-level counterpart to [`decode_owned`] for callers that already have units
    /// from [`utf16le_bytes_to_units`].
    ///
    /// [`decode_owned`]: ironrdp_core::DecodeOwned::decode_owned
    /// [`utf16le_bytes_to_units`]: crate::utf16le_bytes_to_units
    pub fn from_wire_units(units: Vec<u16>) -> Result<Self, StringTooLong> {
        let mut units = units;
        let end = units.iter().rposition(|&u| u != 0).map_or(0, |i| i + 1);
        units.truncate(end);

        let actual = units.len();

        check_invariant(actual < WCHAR_COUNT).ok_or_else(|| StringTooLong {
            max_code_units: WCHAR_COUNT.saturating_sub(1),
            actual_code_units: actual,
        })?;

        Ok(Self(StringRepr::from_wire_units(units)))
    }

    /// Creates a `FixedSizeUnicodeString` from a native Rust string.
    ///
    /// Returns [`StringTooLong`] if the string requires more than `WCHAR_COUNT - 1`
    /// UTF-16 code units (one slot is reserved for the null terminator).
    pub fn new(s: impl Into<String>) -> Result<Self, StringTooLong> {
        let s = s.into();
        let actual = utf16_code_units(&s);

        check_invariant(actual < WCHAR_COUNT).ok_or_else(|| StringTooLong {
            max_code_units: WCHAR_COUNT.saturating_sub(1),
            actual_code_units: actual,
        })?;

        Ok(Self(StringRepr::from_native(s)))
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
    /// The returned units do not include a null terminator or zero-padding.
    pub fn to_wire_units(&self) -> Cow<'_, [u16]> {
        self.0.to_wire_units()
    }

    /// Consumes `self` and returns the UTF-16 code units of this string.
    ///
    /// Zero-cost when the value was decoded from the wire (moves the internal buffer).
    /// Encodes and allocates when the value was constructed from a native string.
    /// The returned units do not include a null terminator or zero-padding.
    pub fn into_wire_units(self) -> Vec<u16> {
        self.0.into_wire_units()
    }

    /// Consumes `self` and returns the raw UTF-16LE bytes of the string content.
    ///
    /// Zero-cost when the value was decoded from the wire (moves the internal buffer).
    /// Encodes to UTF-16LE and allocates when the value was constructed from a native string.
    /// The returned bytes do not include a null terminator or zero-padding.
    pub fn into_wire(self) -> Vec<u8> {
        self.0.into_wire()
    }
}

impl<const WCHAR_COUNT: usize> TryFrom<FixedSizeUnicodeString<WCHAR_COUNT>> for String {
    type Error = InvalidUtf16;

    fn try_from(s: FixedSizeUnicodeString<WCHAR_COUNT>) -> Result<Self, Self::Error> {
        s.0.into_native()
    }
}

impl<const WCHAR_COUNT: usize> fmt::Display for FixedSizeUnicodeString<WCHAR_COUNT> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.to_native_lossy(), f)
    }
}

impl<const WCHAR_COUNT: usize> fmt::Debug for FixedSizeUnicodeString<WCHAR_COUNT> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FixedSizeUnicodeString<{WCHAR_COUNT}>({:?})", self.0)
    }
}

impl<const WCHAR_COUNT: usize> Clone for FixedSizeUnicodeString<WCHAR_COUNT> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<const WCHAR_COUNT: usize> PartialEq for FixedSizeUnicodeString<WCHAR_COUNT> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<const WCHAR_COUNT: usize> Eq for FixedSizeUnicodeString<WCHAR_COUNT> {}

// ── Encode / DecodeOwned ──────────────────────────────────────────────────────

impl<const WCHAR_COUNT: usize> Encode for FixedSizeUnicodeString<WCHAR_COUNT> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: Self::WIRE_SIZE);

        let wire_bytes = self.0.as_wire_bytes();
        dst.write_slice(&wire_bytes);
        let written_units = wire_bytes.len() / 2;

        // Zero-pad remaining slots (null terminator + any additional padding).
        for _ in written_units..WCHAR_COUNT {
            dst.write_u16(0);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "FixedSizeUnicodeString"
    }

    fn size(&self) -> usize {
        Self::WIRE_SIZE
    }
}

impl<const WCHAR_COUNT: usize> DecodeOwned for FixedSizeUnicodeString<WCHAR_COUNT> {
    fn decode_owned(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: WCHAR_COUNT * 2);

        let slice = src.read_slice(WCHAR_COUNT * 2);
        let units = crate::repr::le_bytes_to_units_strip_nulls(slice);

        Ok(Self(StringRepr::from_wire_units(units)))
    }
}
