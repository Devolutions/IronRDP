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

/// Error returned when a string is too long for a [`FixedString`] field.
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

impl core::error::Error for StringTooLong {}

/// Error returned by [`FixedString::from_utf16le_bytes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixedStringBytesError {
    /// The byte slice has odd length. UTF-16LE requires exactly 2 bytes per code unit.
    OddByteCount,
    /// The content is too long for the field after stripping trailing nulls.
    StringTooLong(StringTooLong),
}

impl fmt::Display for FixedStringBytesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OddByteCount => f.write_str("odd byte count: UTF-16LE requires 2 bytes per code unit"),
            Self::StringTooLong(e) => fmt::Display::fmt(e, f),
        }
    }
}

impl core::error::Error for FixedStringBytesError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::OddByteCount => None,
            Self::StringTooLong(e) => Some(e),
        }
    }
}

impl From<StringTooLong> for FixedStringBytesError {
    fn from(e: StringTooLong) -> Self {
        Self::StringTooLong(e)
    }
}

// ── FixedString ────────────────────────────────────────────────────

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
/// The wire byte size is always [`WIRE_SIZE`](FixedString::WIRE_SIZE) = `WCHAR_COUNT * 2`.
///
/// # Common instantiations
///
/// | Type alias                           | `WCHAR_COUNT` | Wire bytes | Spec field |
/// |--------------------------------------|---------------|------------|------------|
/// | `FixedString<16>`         | 16            | 32         | `clientName` ([MS-RDPBCGR] §2.2.1.3.2) |
/// | `FixedString<32>`         | 32            | 64         | `StandardName`, `DaylightName` ([MS-RDPBCGR] §2.2.1.11.1.1.1) |
/// | `FixedString<260>`        | 260           | 520        | `fileName`, `applicationId` |
///
/// [`to_native`]: FixedString::to_native
/// [`to_native_lossy`]: FixedString::to_native_lossy
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
pub struct FixedString<const WCHAR_COUNT: usize>(
    /// INVARIANT: `utf16_code_units` of the stored string is `< WCHAR_COUNT`.
    StringRepr,
);

impl<const WCHAR_COUNT: usize> FixedString<WCHAR_COUNT> {
    /// Wire byte size: always `WCHAR_COUNT * 2` bytes.
    pub const WIRE_SIZE: usize = {
        assert!(
            WCHAR_COUNT > 0,
            "FixedString<WCHAR_COUNT>: WCHAR_COUNT must be > 0 (at least one slot is required for the null terminator)"
        );
        WCHAR_COUNT * 2
    };

    /// Creates a `FixedString` from UTF-16LE content bytes.
    ///
    /// `bytes` is the string content — it does not need to be padded to `WIRE_SIZE`.
    /// Trailing null code units are stripped before the length check. Returns
    /// [`FixedStringBytesError::OddByteCount`] if `bytes` has odd length, or
    /// [`FixedStringBytesError::StringTooLong`] if the content exceeds `WCHAR_COUNT - 1`
    /// code units after stripping.
    ///
    /// This is a convenience wrapper around [`utf16le_bytes_to_units`] + [`from_wire_units`].
    ///
    /// [`utf16le_bytes_to_units`]: crate::utf16le_bytes_to_units
    /// [`from_wire_units`]: FixedString::from_wire_units
    pub fn from_utf16le_bytes(bytes: &[u8]) -> Result<Self, FixedStringBytesError> {
        let units = crate::utf16le_bytes_to_units(bytes).ok_or(FixedStringBytesError::OddByteCount)?;
        Self::from_wire_units(units).map_err(FixedStringBytesError::StringTooLong)
    }

    /// Creates a `FixedString` from pre-parsed UTF-16 code units.
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

    /// Creates a `FixedString` from a native Rust string, truncating to
    /// `WCHAR_COUNT - 1` UTF-16 code units if the string is too long.
    ///
    /// If the string fits within the field, this is equivalent to [`new`]. If it is too
    /// long, the string is truncated at code-unit boundaries; a dangling high surrogate
    /// at the cut point is also removed to preserve valid surrogate pairs.
    ///
    /// [`new`]: FixedString::new
    #[expect(
        clippy::missing_panics_doc,
        reason = "the expect() is unreachable: truncation to at most WCHAR_COUNT-1 units guarantees from_wire_units succeeds"
    )]
    pub fn new_truncating(s: impl Into<String>) -> Self {
        let s = s.into();

        let max = WCHAR_COUNT.saturating_sub(1);

        // Fast path: string fits — keep the owned String directly, no Vec<u16> needed.
        if utf16_code_units(&s) <= max {
            return Self(StringRepr::from_native(s));
        }

        // Slow path: truncate at a code-unit boundary, then drop a dangling high surrogate.
        let mut units: Vec<u16> = s.encode_utf16().take(max).collect();
        if units.last().is_some_and(|&u| (0xD800..=0xDBFF).contains(&u)) {
            units.pop();
        }
        Self::from_wire_units(units).expect("truncated units cannot exceed WCHAR_COUNT - 1")
    }

    /// Creates a `FixedString` from a native Rust string.
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

impl<const WCHAR_COUNT: usize> TryFrom<FixedString<WCHAR_COUNT>> for String {
    type Error = InvalidUtf16;

    fn try_from(s: FixedString<WCHAR_COUNT>) -> Result<Self, Self::Error> {
        s.0.into_native()
    }
}

impl<const WCHAR_COUNT: usize> fmt::Display for FixedString<WCHAR_COUNT> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.to_native_lossy(), f)
    }
}

impl<const WCHAR_COUNT: usize> fmt::Debug for FixedString<WCHAR_COUNT> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FixedString<{WCHAR_COUNT}>({:?})", self.0)
    }
}

impl<const WCHAR_COUNT: usize> Clone for FixedString<WCHAR_COUNT> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<const WCHAR_COUNT: usize> PartialEq for FixedString<WCHAR_COUNT> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<const WCHAR_COUNT: usize> Eq for FixedString<WCHAR_COUNT> {}

impl<const WCHAR_COUNT: usize> Default for FixedString<WCHAR_COUNT> {
    fn default() -> Self {
        Self(StringRepr::from_native(String::new()))
    }
}

// ── Encode / DecodeOwned ──────────────────────────────────────────────────────

impl<const WCHAR_COUNT: usize> Encode for FixedString<WCHAR_COUNT> {
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
        "FixedString"
    }

    fn size(&self) -> usize {
        Self::WIRE_SIZE
    }
}

impl<const WCHAR_COUNT: usize> DecodeOwned for FixedString<WCHAR_COUNT> {
    fn decode_owned(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::WIRE_SIZE);

        let slice = src.read_slice(Self::WIRE_SIZE);
        let units = crate::repr::le_bytes_to_units_strip_nulls(slice);

        // After stripping trailing nulls from WCHAR_COUNT units, the result must be
        // strictly shorter — if no null was present the field is malformed.
        if units.len() >= WCHAR_COUNT {
            return Err(ironrdp_core::invalid_field_err!(
                "content",
                "fixed-size string field is missing its null terminator"
            ));
        }

        Ok(Self(StringRepr::from_wire_units(units)))
    }
}
