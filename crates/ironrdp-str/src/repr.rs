//! Internal dual-representation string value.
//!
//! This module contains `StringRepr`, the common backing store for all string types in
//! this crate.

use alloc::borrow::Cow;
#[cfg(not(feature = "std"))]
use alloc::string::String;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use core::fmt;

use crate::InvalidUtf16;

// ── StringRepr ────────────────────────────────────────────────────────────────

/// The internal representation of an RDP string field value.
///
/// All string types in this crate use `StringRepr` as their internal storage.
/// The choice of variant is an implementation detail invisible to callers.
///
/// - `Wire`: produced by decoding from the wire. Stores UTF-16 code unit values as `u16`;
///   re-encoding to wire bytes is zero-cost on little-endian targets via `bytemuck`.
/// - `Native`: produced by construction from a Rust `String` or `&str`.
#[derive(Clone)]
pub(crate) enum StringRepr {
    /// UTF-16 code unit values decoded from the wire, stored as `u16`.
    ///
    /// INVARIANT: null terminators are never stored here (callers strip them on decode).
    /// NOTE: code units are NOT validated; lone surrogates may be present.
    Wire(Vec<u16>),

    /// Native UTF-8 string from Rust caller code.
    Native(String),
}

impl StringRepr {
    /// Creates a `Wire` variant directly from UTF-16 code units.
    ///
    /// The caller must ensure that null terminators have already been stripped.
    pub(crate) fn from_wire_units(units: Vec<u16>) -> Self {
        Self::Wire(units)
    }

    /// Creates a `Native` variant from a Rust string.
    pub(crate) fn from_native(s: String) -> Self {
        Self::Native(s)
    }

    /// Tries to return the string content as a Rust `str`.
    ///
    /// Returns [`InvalidUtf16`] if the wire data contains a lone surrogate.
    /// For `Wire` this allocates a new `String`. For `Native` this is a zero-cost borrow.
    pub(crate) fn to_native(&self) -> Result<Cow<'_, str>, InvalidUtf16> {
        match self {
            Self::Wire(units) => String::from_utf16(units).map(Cow::Owned).map_err(|_| InvalidUtf16),
            Self::Native(s) => Ok(Cow::Borrowed(s.as_str())),
        }
    }

    /// Returns the string content, replacing any lone surrogates with U+FFFD.
    ///
    /// For `Wire` this allocates a new `String`. For `Native` this is a zero-cost borrow.
    pub(crate) fn to_native_lossy(&self) -> Cow<'_, str> {
        match self {
            Self::Wire(units) => Cow::Owned(String::from_utf16_lossy(units)),
            Self::Native(s) => Cow::Borrowed(s.as_str()),
        }
    }

    /// Returns an iterator over the UTF-16 code units of this string.
    ///
    /// Zero-allocation for both variants.
    pub(crate) fn utf16_units(&self) -> Utf16Units<'_> {
        match self {
            Self::Wire(units) => Utf16Units(Utf16UnitsInner::Wire(units.iter().copied())),
            Self::Native(s) => Utf16Units(Utf16UnitsInner::Native(s.encode_utf16())),
        }
    }

    /// Returns the number of UTF-16 code units (WCHARs) in this string.
    ///
    /// O(1) for the `Wire` variant, O(n) for the `Native` variant.
    pub(crate) fn utf16_len(&self) -> usize {
        match self {
            Self::Wire(units) => units.len(),
            Self::Native(s) => s.encode_utf16().count(),
        }
    }

    /// Returns the wire byte length of the UTF-16LE encoding (`utf16_len() * 2`).
    pub(crate) fn utf16_byte_len(&self) -> usize {
        match self {
            Self::Wire(units) => units.len() * 2,
            Self::Native(s) => s.encode_utf16().count() * 2,
        }
    }

    /// Returns the raw bytes of the wire representation.
    ///
    /// For `Wire` on little-endian targets, this is a zero-cost borrow via `bytemuck`.
    /// For `Native`, or on big-endian targets, this encodes to UTF-16LE and allocates.
    pub(crate) fn as_wire_bytes(&self) -> Cow<'_, [u8]> {
        match self {
            Self::Wire(units) => {
                #[cfg(target_endian = "little")]
                {
                    Cow::Borrowed(bytemuck::cast_slice(units.as_slice()))
                }
                #[cfg(not(target_endian = "little"))]
                {
                    Cow::Owned(units.iter().flat_map(|u| u.to_le_bytes()).collect())
                }
            }
            Self::Native(s) => Cow::Owned(s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()),
        }
    }

    /// Returns the UTF-16 code units of this string.
    ///
    /// For `Wire`, this is a zero-cost borrow of the stored code units.
    /// For `Native`, this encodes the string to UTF-16 and allocates a `Vec<u16>`.
    pub(crate) fn to_wire_units(&self) -> Cow<'_, [u16]> {
        match self {
            Self::Wire(units) => Cow::Borrowed(units.as_slice()),
            Self::Native(s) => Cow::Owned(s.encode_utf16().collect()),
        }
    }

    /// Consumes `self` and returns the UTF-16 code units.
    ///
    /// For `Wire`, this is a zero-cost move of the stored `Vec<u16>`.
    /// For `Native`, this encodes the string to UTF-16 and allocates a `Vec<u16>`.
    pub(crate) fn into_wire_units(self) -> Vec<u16> {
        match self {
            Self::Wire(units) => units,
            Self::Native(s) => s.encode_utf16().collect(),
        }
    }

    /// Consumes `self` and returns a validated native `String`.
    ///
    /// For `Native`, this is a zero-cost unwrap of the stored `String`.
    /// For `Wire`, this validates the UTF-16 sequence and allocates a new `String`.
    /// Returns [`InvalidUtf16`] if the wire data contains a lone surrogate.
    pub(crate) fn into_native(self) -> Result<String, InvalidUtf16> {
        match self {
            Self::Native(s) => Ok(s),
            Self::Wire(units) => String::from_utf16(&units).map_err(|_| InvalidUtf16),
        }
    }

    /// Consumes `self` and returns the raw UTF-16LE bytes.
    ///
    /// For `Wire` on little-endian targets, this is a zero-cost move via `bytemuck::cast_vec`.
    /// For `Native`, or on big-endian targets, this encodes to UTF-16LE and allocates.
    pub(crate) fn into_wire(self) -> Vec<u8> {
        match self {
            Self::Wire(units) => {
                #[cfg(target_endian = "little")]
                {
                    bytemuck::cast_vec(units)
                }
                #[cfg(not(target_endian = "little"))]
                {
                    units.iter().flat_map(|u| u.to_le_bytes()).collect()
                }
            }
            Self::Native(s) => s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect(),
        }
    }
}

impl PartialEq for StringRepr {
    fn eq(&self, other: &Self) -> bool {
        // Compare by UTF-16 code unit sequence. This is correct because two strings
        // with identical code unit sequences represent the same wire bytes.
        self.utf16_units().eq(other.utf16_units())
    }
}

impl Eq for StringRepr {}

impl core::hash::Hash for StringRepr {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        // Must be consistent with PartialEq: hash the UTF-16 code unit sequence.
        self.utf16_units().for_each(|u| u.hash(state));
    }
}

impl fmt::Debug for StringRepr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(units) => match String::from_utf16(units) {
                Ok(s) => write!(f, "Wire({s:?})"),
                Err(_) => write!(f, "Wire(<invalid utf-16: {units:?}>)"),
            },
            Self::Native(s) => write!(f, "Native({s:?})"),
        }
    }
}

// ── Utf16Units iterator ───────────────────────────────────────────────────────

/// Zero-allocation iterator over UTF-16 code units.
pub(crate) struct Utf16Units<'a>(Utf16UnitsInner<'a>);

enum Utf16UnitsInner<'a> {
    Wire(core::iter::Copied<core::slice::Iter<'a, u16>>),
    Native(core::str::EncodeUtf16<'a>),
}

impl Iterator for Utf16Units<'_> {
    type Item = u16;

    fn next(&mut self) -> Option<u16> {
        match &mut self.0 {
            Utf16UnitsInner::Wire(it) => it.next(),
            Utf16UnitsInner::Native(it) => it.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.0 {
            Utf16UnitsInner::Wire(it) => it.size_hint(),
            Utf16UnitsInner::Native(it) => it.size_hint(),
        }
    }
}

// ── Wire-byte conversion helpers ──────────────────────────────────────────────

/// Converts a slice of UTF-16LE bytes into a `Vec<u16>` of code unit values.
///
/// On little-endian targets, uses `bytemuck::try_cast_slice` to reinterpret the bytes as
/// `u16` values without per-element endian conversion, then bulk-copies into a `Vec<u16>`.
/// If the input is misaligned the bytes are copied per element as a fallback.
/// On big-endian targets the bytes are always byte-swapped per element.
///
/// In all cases an allocation is performed; the optimization on little-endian targets is
/// avoiding per-element byte-swapping rather than eliminating the allocation.
///
/// # Panics
///
/// Panics in debug builds if `bytes.len()` is odd.
pub(crate) fn le_bytes_to_units(bytes: &[u8]) -> Vec<u16> {
    debug_assert!(bytes.len().is_multiple_of(2), "le_bytes_to_units: odd byte count");

    #[cfg(target_endian = "little")]
    if let Ok(units) = bytemuck::try_cast_slice::<u8, u16>(bytes) {
        return units.to_vec();
    }

    bytes
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect()
}

/// Converts a slice of UTF-16LE bytes into a `Vec<u16>`, stripping trailing null code units.
///
/// On little-endian targets, uses `bytemuck::try_cast_slice` to reinterpret the bytes as
/// `u16` values without per-element endian conversion, then bulk-copies into a `Vec<u16>`.
/// If the input is misaligned the bytes are copied per element as a fallback.
/// On big-endian targets the bytes are always byte-swapped per element.
/// In all cases, trailing `0x0000` code units are stripped before returning.
///
/// In all cases an allocation is performed; the optimization on little-endian targets is
/// avoiding per-element byte-swapping rather than eliminating the allocation.
///
/// # Panics
///
/// Panics in debug builds if `bytes.len()` is odd.
pub(crate) fn le_bytes_to_units_strip_nulls(bytes: &[u8]) -> Vec<u16> {
    debug_assert!(
        bytes.len().is_multiple_of(2),
        "le_bytes_to_units_strip_nulls: odd byte count"
    );

    #[cfg(target_endian = "little")]
    if let Ok(units) = bytemuck::try_cast_slice::<u8, u16>(bytes) {
        let end = units.iter().rposition(|&u| u != 0).map_or(0, |i| i + 1);
        return units[..end].to_vec();
    }

    let mut units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();

    let end = units.iter().rposition(|&u| u != 0).map_or(0, |i| i + 1);
    units.truncate(end);
    units
}
