//! Free-standing helpers for ANSI (UTF-8) wire strings.
//!
//! RDP uses two string encodings side-by-side:
//! - **UTF-16LE** (the primary Unicode encoding) — handled by the typed primitives
//!   in [`crate::fixed`], [`crate::prefixed`], and [`crate::multi_sz`].
//! - **ANSI** (historically the Windows ANSI code page, but always UTF-8 / ASCII in
//!   modern practice) — handled by this module.
//!
//! Unlike the UTF-16LE types this module does **not** define new `Encode`/`Decode`
//! types; instead it provides free-standing functions that mirror the three operations
//! every ANSI field site needs:
//!
//! | Operation | Function |
//! |-----------|----------|
//! | Decode from byte slice | [`decode_ansi`] |
//! | Decode from cursor (null-terminated) | [`read_ansi_null_term`] |
//! | Byte length, null included | [`encoded_ansi_len_with_null`] |
//! | Byte length, null excluded | [`encoded_ansi_len_without_null`] |
//! | Write to cursor, null appended | [`write_ansi_with_null`] |
//! | Write to cursor, no null | [`write_ansi_without_null`] |

#[cfg(not(feature = "std"))]
use alloc::string::String;

use ironrdp_core::{DecodeResult, EncodeResult, ReadCursor, WriteCursor, ensure_size, invalid_field_err};

/// Decodes a byte slice as a UTF-8 (ANSI) string, stopping at the first `\0` byte.
///
/// Bytes after the first NUL are ignored, matching null-terminated field semantics.
/// Returns the decoded `String` on success, or the raw [`core::str::Utf8Error`] on
/// failure so that the call site can attach appropriate field context via
/// [`invalid_field_err!`].
#[inline]
pub fn decode_ansi(bytes: &[u8]) -> Result<String, core::str::Utf8Error> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    core::str::from_utf8(&bytes[..end]).map(|s| s.to_owned())
}

/// Reads a null-terminated ANSI (UTF-8) string from a cursor.
///
/// Scans the cursor for the first `0x00` byte. If found, reads up to and **including**
/// the null (advancing the cursor past it) and returns the content **without** the
/// null. If no null is found, reads to the end of the cursor.
///
/// Returns an error if the content is not valid UTF-8.
pub fn read_ansi_null_term(src: &mut ReadCursor<'_>) -> DecodeResult<String> {
    let null_pos = src.remaining().iter().position(|&b| b == 0);
    // Consume up to and including the null (if present).
    let consume_len = null_pos.map_or(src.len(), |p| p + 1);
    ensure_size!(ctx: "ansi null-terminated string", in: src, size: consume_len);
    let bytes = src.read_slice(consume_len);
    // The string content is everything before the null.
    let content = &bytes[..null_pos.unwrap_or(bytes.len())];
    core::str::from_utf8(content)
        .map(String::from)
        .map_err(|_| invalid_field_err!("ansi string", "invalid UTF-8"))
}

// ── Length helpers ──────────────────────────────────────────────────────────

#[inline]
fn impl_encoded_ansi_len(s: &str, with_null: bool) -> usize {
    s.len() + usize::from(with_null)
}

/// Encoded byte length of `s` **including** a one-byte null terminator.
#[inline]
#[must_use]
pub fn encoded_ansi_len_with_null(s: &str) -> usize {
    impl_encoded_ansi_len(s, true)
}

/// Encoded byte length of `s` **without** a null terminator.
#[inline]
#[must_use]
pub fn encoded_ansi_len_without_null(s: &str) -> usize {
    impl_encoded_ansi_len(s, false)
}

// ── Write helpers ───────────────────────────────────────────────────────────

fn impl_write_ansi(dst: &mut WriteCursor<'_>, s: &str, with_null: bool) -> EncodeResult<()> {
    let total = impl_encoded_ansi_len(s, with_null);
    ensure_size!(ctx: "ansi string", in: dst, size: total);
    dst.write_slice(s.as_bytes());
    if with_null {
        dst.write_u8(0);
    }
    Ok(())
}

/// Writes `s` as an ANSI (UTF-8) string to `dst`, appending a `0x00` null terminator.
pub fn write_ansi_with_null(dst: &mut WriteCursor<'_>, s: &str) -> EncodeResult<()> {
    impl_write_ansi(dst, s, true)
}

/// Writes `s` as an ANSI (UTF-8) string to `dst` **without** a null terminator.
pub fn write_ansi_without_null(dst: &mut WriteCursor<'_>, s: &str) -> EncodeResult<()> {
    impl_write_ansi(dst, s, false)
}
