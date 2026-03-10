#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![cfg_attr(not(feature = "std"), no_std)]
#![warn(clippy::std_instead_of_alloc)]
#![warn(clippy::std_instead_of_core)]
#![cfg_attr(doc, warn(missing_docs))]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::borrow::Cow;
#[cfg(all(feature = "alloc", not(feature = "std")))]
use alloc::vec::Vec;

#[cfg(feature = "alloc")]
mod repr;

#[cfg(feature = "alloc")]
pub mod fixed;

#[cfg(feature = "alloc")]
pub mod prefixed;

#[cfg(feature = "alloc")]
pub mod multi_sz;

#[cfg(feature = "alloc")]
pub mod unframed;

/// Error returned when a wire string contains an invalid UTF-16 sequence (lone surrogate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidUtf16;

/// Error returned when a string passed to a `MULTI_SZ` constructor contains an embedded
/// `NUL` (`\0` / U+0000 / `0x0000`).
///
/// `MULTI_SZ` uses null as a segment delimiter, so an embedded null would corrupt segment
/// boundaries and break round-trip semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddedNul;

impl core::fmt::Display for InvalidUtf16 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("invalid utf-16: lone surrogate in wire data")
    }
}

impl core::error::Error for InvalidUtf16 {}

impl core::fmt::Display for EmbeddedNul {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("embedded nul: MULTI_SZ segment contains a U+0000 code unit")
    }
}

impl core::error::Error for EmbeddedNul {}

/// Converts a slice of UTF-16LE wire bytes into a `Vec` of UTF-16 code unit values.
///
/// Each consecutive pair of bytes is interpreted as one little-endian `u16` code unit.
/// Returns `None` if `bytes` has odd length (which is always a malformed UTF-16LE sequence).
///
/// This is the correct way to hand off raw wire bytes to APIs that work with `&[u16]` or
/// `Vec<u16>`.
#[cfg(feature = "alloc")]
#[inline]
#[must_use]
pub fn utf16le_bytes_to_units(bytes: &[u8]) -> Option<Vec<u16>> {
    bytes.len().is_multiple_of(2).then(|| repr::le_bytes_to_units(bytes))
}

/// Converts a slice of UTF-16 code unit values to a UTF-16LE byte representation.
///
/// On **little-endian** targets this is a **zero-cost borrow**: the returned [`Cow`] points
/// directly into `units` without any allocation or copying.
/// On big-endian targets the bytes are swapped and a new `Vec<u8>` is allocated.
///
/// The `bytemuck` crate is used internally for the zero-copy path.
///
/// [`Cow`]: alloc::borrow::Cow
#[cfg(feature = "alloc")]
#[inline]
#[must_use]
pub fn utf16_units_to_le_bytes(units: &[u16]) -> Cow<'_, [u8]> {
    #[cfg(target_endian = "little")]
    {
        Cow::Borrowed(bytemuck::cast_slice(units))
    }
    #[cfg(not(target_endian = "little"))]
    {
        Cow::Owned(units.iter().flat_map(|u| u.to_le_bytes()).collect())
    }
}

/// Number of UTF-16 code units (WCHARs) required to encode `s`, without null terminator.
///
/// This is what every `cch`-prefixed RDP field counts.
/// For non-BMP characters (U+10000+), each encodes as a surrogate pair and counts as 2.
///
/// **Never substitute `s.chars().count()` or `s.len()` for this.**
#[inline]
#[must_use]
pub fn utf16_code_units(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Byte length of the UTF-16LE wire encoding of `s`, without null terminator.
///
/// This is what every `cb`-prefixed RDP field counts when the null is excluded.
#[inline]
#[must_use]
pub fn utf16_byte_len(s: &str) -> usize {
    s.encode_utf16().count() * 2
}

/// Byte length of the UTF-16LE wire encoding of `s`, including a UTF-16 null terminator
/// (2 bytes: `0x00 0x00`).
///
/// This is what every `cb`-prefixed RDP field counts when the null is included.
#[inline]
#[must_use]
pub fn utf16_byte_len_with_null(s: &str) -> usize {
    (s.encode_utf16().count() + 1) * 2
}

/// Use this when establishing invariants.
#[inline]
#[must_use]
fn check_invariant(condition: bool) -> Option<()> {
    condition.then_some(())
}
