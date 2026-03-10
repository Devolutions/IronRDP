//! `MULTI_SZ` string list.
//!
//! Used for fields like `HardwareIds` and `CompatibilityIds` in MS-RDPEUSB §2.2.4.2.
//!
//! # Internal representation
//!
//! [`MultiSzString`] uses a dual-representation design analogous to `StringRepr`:
//!
//! - **`Wire`**: stores the raw UTF-16 code units for all string segments flat in one
//!   `Vec<u16>`. Each segment ends with its null terminator (`0x0000`), but the final
//!   sentinel null is **not** stored — it is always written by [`Encode`] and stripped
//!   by [`DecodeOwned`]. This means decode is a single allocation (`memcpy`) plus a
//!   one-slot `truncate`, and re-encode is a single bulk bytemuck write plus one
//!   `write_u16(0)` for the sentinel. No per-segment scanning or allocation is needed
//!   until the caller actually iterates the segments.
//!
//! - **`Native`**: stores a `Vec<String>` of Rust strings. UTF-16 encoding is deferred
//!   entirely to encode time.
//!
//! Wire layout for `["foo", "bar"]` (stored units in the `Wire` variant):
//!
//! ```text
//! stored: [f, o, o, 0x0000, b, a, r, 0x0000]   (sentinel excluded)
//! wire:   [u32 cch=9][f,o,o][0x0000][b,a,r][0x0000][0x0000 sentinel]
//! ```
//!
//! [`Encode`]: ironrdp_core::Encode
//! [`DecodeOwned`]: ironrdp_core::DecodeOwned

use alloc::borrow::Cow;
#[cfg(not(feature = "std"))]
use alloc::string::String;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use core::fmt;

use ironrdp_core::{
    DecodeOwned, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_size,
    invalid_field_err,
};

use crate::{EmbeddedNul, InvalidUtf16};

// ── MultiSzSegmentError ───────────────────────────────────────────────────────

/// Error returned by [`MultiSzString::from_utf16le_byte_strings`] when a byte-slice segment
/// is malformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiSzSegmentError {
    /// The byte slice has odd length. UTF-16LE requires exactly 2 bytes per code unit.
    OddByteCount,
    /// The segment contains an embedded `0x0000` (U+0000) code unit, which would corrupt
    /// `MULTI_SZ` segment boundaries and break round-trip semantics.
    EmbeddedNul,
}

impl fmt::Display for MultiSzSegmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OddByteCount => f.write_str("odd byte count: UTF-16LE requires 2 bytes per code unit"),
            Self::EmbeddedNul => f.write_str("embedded nul: MULTI_SZ segment contains a U+0000 code unit"),
        }
    }
}

impl core::error::Error for MultiSzSegmentError {}

// ── MultiSzFlatError ──────────────────────────────────────────────────────────

/// Error returned by [`MultiSzString::from_utf16le_flat`] and
/// [`MultiSzString::from_wire_units_flat`] when the flat buffer is malformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiSzFlatError {
    /// The byte slice has odd length. UTF-16LE requires exactly 2 bytes per code unit.
    ///
    /// Only returned by [`MultiSzString::from_utf16le_flat`].
    OddByteCount,
    /// The buffer does not end with the required sentinel null (`0x0000`).
    MissingSentinel,
    /// After stripping the sentinel, the remaining content is non-empty but does not end
    /// with a per-string null terminator. The last segment would be silently dropped by
    /// iteration.
    UnterminatedLastSegment,
}

impl fmt::Display for MultiSzFlatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OddByteCount => f.write_str("odd byte count: UTF-16LE requires 2 bytes per code unit"),
            Self::MissingSentinel => f.write_str("MULTI_SZ flat buffer is missing the sentinel null"),
            Self::UnterminatedLastSegment => f.write_str("MULTI_SZ last segment is missing its null terminator"),
        }
    }
}

impl core::error::Error for MultiSzFlatError {}

// ── Internal representation ───────────────────────────────────────────────────

/// Internal representation of a [`MultiSzString`].
///
/// | Logical value      | `Wire` stored units                           |
/// |--------------------|-----------------------------------------------|
/// | `[]`               | `[]`                                          |
/// | `["foo"]`          | `[f, o, o, 0x0000]`                           |
/// | `["foo", "bar"]`   | `[f, o, o, 0x0000, b, a, r, 0x0000]`         |
///
/// Each segment ends with its per-string null terminator. The final sentinel null is
/// **not** included — it is implicit and always written by [`Encode`] / stripped by
/// [`DecodeOwned`].
///
/// [`Encode`]: ironrdp_core::Encode
/// [`DecodeOwned`]: ironrdp_core::DecodeOwned
enum MultiSzStringRepr {
    /// Raw UTF-16 code units: all segments, each null-terminated; sentinel excluded.
    Wire(Vec<u16>),
    /// Validated native Rust strings, one per segment.
    Native(Vec<String>),
}

// ── MultiSzString ─────────────────────────────────────────────────────────────

/// A `MULTI_SZ`: a list of UTF-16LE strings, each null-terminated, followed by an extra
/// null, with the whole block prefixed by a `u32` WCHAR count that includes all null
/// terminators.
///
/// Wire layout: `[u32 cch][str1 WCHARs][0x0000][str2 WCHARs][0x0000]...[0x0000]`
///
/// The `u32 cch` counts **all** code units including all null terminators
/// (both per-string and the final sentinel). The minimum valid `cch` for an empty
/// list is 1 (just the final sentinel null).
///
/// Wire data is accepted as-is with no UTF-16 validation at decode time. Call [`iter_native`]
/// for validated conversion, or [`iter_native_lossy`] to accept any byte sequence with
/// lone-surrogate replacement.
///
/// [MS-RDPEUSB] §2.2.4.2
///
/// [MS-RDPEUSB]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/
/// [`iter_native`]: MultiSzString::iter_native
/// [`iter_native_lossy`]: MultiSzString::iter_native_lossy
pub struct MultiSzString(MultiSzStringRepr);

impl MultiSzString {
    /// Creates a `MultiSzString` from an iterator of native Rust strings.
    ///
    /// Returns [`EmbeddedNul`] if any string contains an embedded `\0` (U+0000). MULTI_SZ
    /// uses null as a segment delimiter, so an embedded null would corrupt segment boundaries
    /// and break round-trip semantics.
    pub fn new(strings: impl IntoIterator<Item = impl Into<String>>) -> Result<Self, EmbeddedNul> {
        let strings: Vec<String> = strings.into_iter().map(Into::into).collect();
        if strings.iter().any(|s| s.contains('\0')) {
            return Err(EmbeddedNul);
        }
        Ok(Self(MultiSzStringRepr::Native(strings)))
    }

    /// Creates a `MultiSzString` from an iterator of raw UTF-16LE byte slices, one per
    /// string segment.
    ///
    /// Each byte slice is converted to `u16` code units and null-terminated; the resulting
    /// segments are stored as a flat `Wire` buffer.
    ///
    /// Returns [`MultiSzSegmentError::OddByteCount`] if any slice has odd length, or
    /// [`MultiSzSegmentError::EmbeddedNul`] if any decoded segment contains a `0x0000`
    /// code unit. An embedded null would split the segment into multiple segments on iteration,
    /// breaking the API contract of one byte slice per string.
    #[expect(
        single_use_lifetimes,
        reason = "`'a` is required here because anonymous lifetimes in `impl Trait` are unstable; rustc incorrectly suggests eliding it"
    )]
    pub fn from_utf16le_byte_strings<'a>(
        byte_strings: impl IntoIterator<Item = &'a [u8]>,
    ) -> Result<Self, MultiSzSegmentError> {
        let mut units: Vec<u16> = Vec::new();
        for bytes in byte_strings {
            if !bytes.len().is_multiple_of(2) {
                return Err(MultiSzSegmentError::OddByteCount);
            }
            let segment = crate::repr::le_bytes_to_units(bytes);
            if segment.contains(&0) {
                return Err(MultiSzSegmentError::EmbeddedNul);
            }
            units.extend_from_slice(&segment);
            units.push(0);
        }
        Ok(Self(MultiSzStringRepr::Wire(units)))
    }

    /// Creates a `MultiSzString` from a flat UTF-16LE byte slice containing the complete
    /// `MULTI_SZ` content: all string segments with their per-string null terminators,
    /// followed by the final sentinel null.
    ///
    /// This is the flat-buffer counterpart to [`from_utf16le_byte_strings`]: instead of
    /// one `&[u8]` per segment, the entire content arrives as a single contiguous slice
    /// (e.g. straight from a registry value). The sentinel null is required and stripped
    /// before storage; per-string nulls are retained.
    ///
    /// # Errors
    ///
    /// - [`MultiSzFlatError::OddByteCount`] — `bytes` has odd length.
    /// - [`MultiSzFlatError::MissingSentinel`] — `bytes` does not end with `0x0000`.
    /// - [`MultiSzFlatError::UnterminatedLastSegment`] — after stripping the sentinel, the
    ///   remaining content is non-empty but does not end with a per-string null terminator.
    ///
    /// [`from_utf16le_byte_strings`]: MultiSzString::from_utf16le_byte_strings
    pub fn from_utf16le_flat(bytes: &[u8]) -> Result<Self, MultiSzFlatError> {
        if !bytes.len().is_multiple_of(2) {
            return Err(MultiSzFlatError::OddByteCount);
        }

        let mut units = crate::repr::le_bytes_to_units(bytes);

        // Require and strip the sentinel null.
        if units.last() != Some(&0) {
            return Err(MultiSzFlatError::MissingSentinel);
        }

        units.truncate(units.len() - 1);

        // After stripping the sentinel, the remaining content must either be empty
        // (empty list) or end with a per-string null (last segment is properly terminated).
        if !units.is_empty() && units.last() != Some(&0) {
            return Err(MultiSzFlatError::UnterminatedLastSegment);
        }

        Ok(Self(MultiSzStringRepr::Wire(units)))
    }

    /// Creates a `MultiSzString` from a flat `Vec<u16>` of UTF-16 code units containing
    /// the complete `MULTI_SZ` content: all string segments with their per-string null
    /// terminators, followed by the final sentinel null.
    ///
    /// This is the flat-buffer counterpart to [`from_unit_strings`]: instead of one
    /// `Vec<u16>` per segment, all segments arrive in a single pre-parsed vector. The
    /// sentinel null is required and stripped before storage; per-string nulls are retained.
    ///
    /// # Errors
    ///
    /// - [`MultiSzFlatError::MissingSentinel`] — `units` does not end with `0x0000`
    ///   (including the empty-vector case).
    /// - [`MultiSzFlatError::UnterminatedLastSegment`] — after stripping the sentinel, the
    ///   remaining content is non-empty but does not end with a per-string null terminator.
    ///
    /// [`from_unit_strings`]: MultiSzString::from_unit_strings
    pub fn from_wire_units_flat(units: Vec<u16>) -> Result<Self, MultiSzFlatError> {
        // Require and strip the sentinel null.
        // If `units` is empty, `last()` returns `None` which != Some(&0), so we error.
        if units.last() != Some(&0) {
            return Err(MultiSzFlatError::MissingSentinel);
        }

        let mut units = units;
        units.truncate(units.len() - 1);

        // After stripping the sentinel, the remaining content must either be empty
        // (empty list) or end with a per-string null (last segment is properly terminated).
        if !units.is_empty() && units.last() != Some(&0) {
            return Err(MultiSzFlatError::UnterminatedLastSegment);
        }

        Ok(Self(MultiSzStringRepr::Wire(units)))
    }

    /// Creates a `MultiSzString` from an iterator of pre-parsed UTF-16 code unit vectors,
    /// one `Vec<u16>` per string segment.
    ///
    /// A single trailing `0x0000` in each segment is stripped. The per-segment null
    /// terminator and the final sentinel are always written by [`Encode`] and consumed by
    /// [`DecodeOwned`]. This is the low-level counterpart to [`DecodeOwned`] for callers
    /// that already have units from [`utf16le_bytes_to_units`].
    ///
    /// Returns [`EmbeddedNul`] if any segment contains an **interior** `0x0000` unit (i.e.,
    /// a null that is not the trailing terminator). MULTI_SZ uses null as a segment delimiter,
    /// so an interior null would corrupt segment boundaries and break round-trip semantics.
    ///
    /// [`Encode`]: ironrdp_core::Encode
    /// [`DecodeOwned`]: ironrdp_core::DecodeOwned
    /// [`utf16le_bytes_to_units`]: crate::utf16le_bytes_to_units
    pub fn from_unit_strings(unit_strings: impl IntoIterator<Item = Vec<u16>>) -> Result<Self, EmbeddedNul> {
        let mut units: Vec<u16> = Vec::new();

        for mut segment in unit_strings {
            // Strip a single trailing null.
            if segment.last() == Some(&0) {
                segment.pop();
            }

            // Reject interior nulls — they would silently split this segment into multiple ones.
            if segment.contains(&0) {
                return Err(EmbeddedNul);
            }

            units.extend_from_slice(&segment);
            units.push(0); // per-segment null terminator
        }

        Ok(Self(MultiSzStringRepr::Wire(units)))
    }

    /// Returns an iterator over the string values.
    ///
    /// Returns [`InvalidUtf16`] per entry if the wire data for that entry contains
    /// a lone surrogate. For wire-decoded strings, each successful entry allocates
    /// a `String`.
    pub fn iter_native(&self) -> impl Iterator<Item = Result<Cow<'_, str>, InvalidUtf16>> + '_ {
        MultiSzNativeIter(match &self.0 {
            MultiSzStringRepr::Wire(units) => MultiSzNativeIterInner::Wire(units.as_slice()),
            MultiSzStringRepr::Native(strings) => MultiSzNativeIterInner::Native(strings.iter()),
        })
    }

    /// Returns an iterator over the string values, replacing any lone surrogates with U+FFFD.
    ///
    /// For strings decoded from the wire, each entry allocates a `String`.
    /// For strings constructed from native Rust code, each entry is a zero-cost borrow.
    pub fn iter_native_lossy(&self) -> impl Iterator<Item = Cow<'_, str>> + '_ {
        MultiSzLossyIter(match &self.0 {
            MultiSzStringRepr::Wire(units) => MultiSzLossyIterInner::Wire(units.as_slice()),
            MultiSzStringRepr::Native(strings) => MultiSzLossyIterInner::Native(strings.iter()),
        })
    }

    /// Consumes `self` and returns each string as a validated native `String`.
    ///
    /// Returns [`InvalidUtf16`] if any segment contains a lone surrogate.
    /// Zero-cost per segment when the value was constructed from native Rust strings.
    pub fn into_native(self) -> Result<Vec<String>, InvalidUtf16> {
        match self.0 {
            MultiSzStringRepr::Wire(units) => {
                let mut result: Vec<String> = Vec::new();
                let mut remaining = units.as_slice();

                while !remaining.is_empty() {
                    let Some(null_pos) = remaining.iter().position(|&u| u == 0) else {
                        break;
                    };

                    result.push(String::from_utf16(&remaining[..null_pos]).map_err(|_| InvalidUtf16)?);
                    remaining = &remaining[null_pos + 1..];
                }

                Ok(result)
            }
            MultiSzStringRepr::Native(strings) => Ok(strings),
        }
    }

    /// Returns the total number of UTF-16 code units on the wire, including all null
    /// terminators and the final sentinel null. This is the value written as the `u32 cch`
    /// prefix.
    ///
    /// # Panics
    ///
    /// Panics on arithmetic overflow (requires a pathologically large input that cannot
    /// be represented on this platform).
    pub fn total_cch(&self) -> usize {
        self.checked_total_cch().expect("MULTI_SZ total length overflow")
    }

    /// Like [`total_cch`], but returns `None` on `usize` overflow.
    ///
    /// [`total_cch`]: MultiSzString::total_cch
    fn checked_total_cch(&self) -> Option<usize> {
        match &self.0 {
            // Stored units already include per-segment nulls; add 1 for the sentinel.
            MultiSzStringRepr::Wire(units) => units.len().checked_add(1),
            // Each string contributes its code units + 1 null; start from 1 for the sentinel.
            MultiSzStringRepr::Native(strings) => strings.iter().try_fold(1usize, |acc, s| {
                acc.checked_add(crate::utf16_code_units(s))
                    .and_then(|n| n.checked_add(1))
            }),
        }
    }
}

// ── Iterators ─────────────────────────────────────────────────────────────────

/// Advances `remaining` past the next null-terminated segment and returns that segment.
///
/// Returns `None` when `remaining` is empty (all segments consumed).
fn wire_next_segment<'a>(remaining: &mut &'a [u16]) -> Option<&'a [u16]> {
    if remaining.is_empty() {
        return None;
    }
    let null_pos = remaining.iter().position(|&u| u == 0)?;
    let segment = &remaining[..null_pos];
    *remaining = &remaining[null_pos + 1..];
    Some(segment)
}

struct MultiSzNativeIter<'a>(MultiSzNativeIterInner<'a>);

enum MultiSzNativeIterInner<'a> {
    Wire(&'a [u16]),
    Native(core::slice::Iter<'a, String>),
}

impl<'a> Iterator for MultiSzNativeIter<'a> {
    type Item = Result<Cow<'a, str>, InvalidUtf16>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            MultiSzNativeIterInner::Wire(remaining) => wire_next_segment(remaining)
                .map(|seg| String::from_utf16(seg).map(Cow::Owned).map_err(|_| InvalidUtf16)),
            MultiSzNativeIterInner::Native(iter) => iter.next().map(|s| Ok(Cow::Borrowed(s.as_str()))),
        }
    }
}

struct MultiSzLossyIter<'a>(MultiSzLossyIterInner<'a>);

enum MultiSzLossyIterInner<'a> {
    Wire(&'a [u16]),
    Native(core::slice::Iter<'a, String>),
}

impl<'a> Iterator for MultiSzLossyIter<'a> {
    type Item = Cow<'a, str>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            MultiSzLossyIterInner::Wire(remaining) => {
                wire_next_segment(remaining).map(|seg| Cow::Owned(String::from_utf16_lossy(seg)))
            }
            MultiSzLossyIterInner::Native(iter) => iter.next().map(|s| Cow::Borrowed(s.as_str())),
        }
    }
}

// ── TryFrom, Debug, Clone, PartialEq, Eq ──────────────────────────────────────

impl TryFrom<MultiSzString> for Vec<String> {
    type Error = InvalidUtf16;

    fn try_from(m: MultiSzString) -> Result<Self, Self::Error> {
        m.into_native()
    }
}

impl fmt::Debug for MultiSzString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let strings: Vec<Cow<'_, str>> = self.iter_native_lossy().collect();
        write!(f, "MultiSzString({strings:?})")
    }
}

impl Clone for MultiSzString {
    fn clone(&self) -> Self {
        Self(match &self.0 {
            MultiSzStringRepr::Wire(units) => MultiSzStringRepr::Wire(units.clone()),
            MultiSzStringRepr::Native(strings) => MultiSzStringRepr::Native(strings.clone()),
        })
    }
}

impl PartialEq for MultiSzString {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (MultiSzStringRepr::Wire(a), MultiSzStringRepr::Wire(b)) => a == b,
            (MultiSzStringRepr::Native(a), MultiSzStringRepr::Native(b)) => a == b,
            (MultiSzStringRepr::Wire(units), MultiSzStringRepr::Native(strings))
            | (MultiSzStringRepr::Native(strings), MultiSzStringRepr::Wire(units)) => {
                let native_iter = strings
                    .iter()
                    .flat_map(|s| s.encode_utf16().chain(core::iter::once(0u16)));
                units.iter().copied().eq(native_iter)
            }
        }
    }
}

impl Eq for MultiSzString {}

// ── Encode / DecodeOwned ──────────────────────────────────────────────────────

impl Encode for MultiSzString {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        // Checked arithmetic first: overflow → error, not a silent wrap.
        let total_cch: u32 = cast_length!(
            "cch",
            self.checked_total_cch()
                .ok_or_else(|| invalid_field_err!("cch", "MULTI_SZ total length overflow"))?
        )?;

        ensure_size!(in: dst, size: self.size());
        dst.write_u32(total_cch);

        match &self.0 {
            MultiSzStringRepr::Wire(units) => {
                // Write flat unit buffer as UTF-16LE bytes.
                #[cfg(target_endian = "little")]
                {
                    dst.write_slice(bytemuck::cast_slice(units.as_slice()));
                }
                #[cfg(not(target_endian = "little"))]
                {
                    for &u in units {
                        dst.write_u16(u);
                    }
                }
            }
            MultiSzStringRepr::Native(strings) => {
                for s in strings {
                    for unit in s.encode_utf16() {
                        dst.write_u16(unit);
                    }
                    dst.write_u16(0); // per-string null terminator
                }
            }
        }

        dst.write_u16(0); // final sentinel null

        Ok(())
    }

    fn name(&self) -> &'static str {
        "MultiSzString"
    }

    fn size(&self) -> usize {
        // Use checked arithmetic so overflow panics here rather than silently producing
        // usize::MAX, which would cause encode_vec() to attempt a huge allocation.
        let total_cch = self
            .checked_total_cch()
            .expect("MULTI_SZ total length overflow when computing size()");

        total_cch
            .checked_mul(2)
            .and_then(|bytes_for_units| bytes_for_units.checked_add(4))
            .expect("MULTI_SZ encoded size overflow")
    }
}

impl DecodeOwned for MultiSzString {
    fn decode_owned(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4);
        let total_cch: usize = cast_length!("cch", src.read_u32())?;

        // The minimum valid total_cch is 1 (just the final sentinel null).
        if total_cch == 0 {
            return Err(invalid_field_err!("cch", "zero cch for MULTI_SZ is invalid"));
        }

        let byte_count = total_cch
            .checked_mul(2)
            .ok_or_else(|| invalid_field_err!("cch", "MULTI_SZ byte length overflow"))?;
        ensure_size!(in: src, size: byte_count);

        // One allocation: read all bytes and reinterpret as u16 code units.
        let all_bytes = src.read_slice(byte_count);
        let mut all_units = crate::repr::le_bytes_to_units(all_bytes);

        // The last code unit must be the final sentinel null (0x0000).
        if let Some(&unit) = all_units.last()
            && unit != 0
        {
            return Err(invalid_field_err!("content", "MULTI_SZ must end with a null sentinel"));
        }

        // Strip the sentinel null; per-string null terminators are retained in storage.
        all_units.truncate(all_units.len() - 1);

        // After stripping the sentinel, the remaining content must be empty (empty list)
        // or end with a per-string null (the last segment is properly terminated).
        // Without this check, a last segment without its own null would be silently dropped
        // by the null-scanning iterators.
        if !all_units.is_empty() && all_units.last() != Some(&0) {
            return Err(invalid_field_err!(
                "content",
                "MULTI_SZ last segment is missing its null terminator"
            ));
        }

        Ok(Self(MultiSzStringRepr::Wire(all_units)))
    }
}
