//! Padding handling helpers
//!
//! For maximum compatibility, messages should be generated with padding set to zero,
//! and message recipients should not assume padding has any particular
//! value.

use crate::{DecodeResult, ReadCursor, WriteCursor, ensure_size};

/// Writes zeroes using as few `write_u*` calls as possible.
pub fn write_padding(dst: &mut WriteCursor<'_>, mut n: usize) {
    loop {
        match n {
            0 => break,
            1 => {
                dst.write_u8(0);
                n -= 1;
            }
            2..=3 => {
                dst.write_u16(0);
                n -= 2;
            }
            4..=7 => {
                dst.write_u32(0);
                n -= 4;
            }
            _ => {
                dst.write_u64(0);
                n -= 8;
            }
        }
    }
}

/// Moves read cursor, ignoring padding bytes.
///
/// This does not check that `n` bytes remain: the caller is responsible for it (e.g. with `ensure_size!`).
/// Prefer [`try_read_padding`] or `read_padding!`, which return an error when the buffer is too short.
#[inline]
pub fn read_padding(src: &mut ReadCursor<'_>, n: usize) {
    src.advance(n);
}

/// Moves read cursor, ignoring padding bytes, after checking that `n` bytes remain.
///
/// # Errors
///
/// Returns a "not enough bytes" error, leaving the cursor untouched, when fewer than `n` bytes remain.
#[inline]
pub fn try_read_padding(src: &mut ReadCursor<'_>, n: usize) -> DecodeResult<()> {
    ensure_size!(ctx: "padding", in: src, size: n);
    src.advance(n);
    Ok(())
}
