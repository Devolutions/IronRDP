//! Binary wire primitives shared by the IPC message codecs.
//!
//! Everything is little-endian and cursor-based so it composes directly with [`ironrdp_core`]'s
//! `Encode`/`Decode`/`DecodeOwned` traits. Strings (and string-shaped payloads) are length-delimited
//! with a `u32` byte-count prefix.
//!
//! These helpers are `pub` so the [`internal`](crate) feature can expose them for unit testing in
//! the workspace test suite; the [`wire`](crate::wire) module itself is only public under that
//! feature.

// The helpers are unconditionally `pub`; their effective visibility is the `wire` module's, which is
// `pub(crate)` unless the `internal` feature exposes it.
#![cfg_attr(not(feature = "internal"), allow(unreachable_pub))]

pub mod propertyset;

use ironrdp_core::{DecodeResult, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_size};
use ironrdp_input::MouseButton;

/// Size on the wire of a length-prefixed UTF-8 string.
pub fn string_size(value: &str) -> usize {
    4 /* length prefix */ + value.len() /* UTF-8 bytes */
}

/// Size on the wire of an optional length-prefixed UTF-8 string.
pub fn opt_string_size(value: Option<&str>) -> usize {
    1 /* presence flag */ + value.map_or(0, string_size)
}

pub fn write_string(dst: &mut WriteCursor<'_>, value: &str) -> EncodeResult<()> {
    ensure_size!(in: dst, size: string_size(value));
    let len: u32 = cast_length!("string length", value.len())?;
    dst.write_u32(len);
    dst.write_slice(value.as_bytes());
    Ok(())
}

pub fn read_string(src: &mut ReadCursor<'_>) -> DecodeResult<String> {
    ensure_size!(in: src, size: 4);
    let len = src.read_u32();
    let len = usize::try_from(len).map_err(|_| ironrdp_core::other_err!("string", "length does not fit in usize"))?;
    ensure_size!(in: src, size: len);
    let bytes = src.read_slice(len);
    String::from_utf8(bytes.to_vec()).map_err(|_| ironrdp_core::invalid_field_err!("string", "not valid UTF-8"))
}

/// Size on the wire of a length-prefixed raw byte blob.
pub fn bytes_size(value: &[u8]) -> usize {
    4 /* length prefix */ + value.len() /* raw bytes */
}

pub fn write_bytes(dst: &mut WriteCursor<'_>, value: &[u8]) -> EncodeResult<()> {
    ensure_size!(in: dst, size: bytes_size(value));
    let len: u32 = cast_length!("bytes length", value.len())?;
    dst.write_u32(len);
    dst.write_slice(value);
    Ok(())
}

pub fn read_bytes(src: &mut ReadCursor<'_>) -> DecodeResult<Vec<u8>> {
    ensure_size!(in: src, size: 4);
    let len = src.read_u32();
    let len = usize::try_from(len).map_err(|_| ironrdp_core::other_err!("bytes", "length does not fit in usize"))?;
    ensure_size!(in: src, size: len);
    Ok(src.read_slice(len).to_vec())
}

pub fn write_opt_string(dst: &mut WriteCursor<'_>, value: Option<&str>) -> EncodeResult<()> {
    ensure_size!(in: dst, size: 1);
    match value {
        Some(value) => {
            dst.write_u8(1);
            write_string(dst, value)
        }
        None => {
            dst.write_u8(0);
            Ok(())
        }
    }
}

pub fn read_opt_string(src: &mut ReadCursor<'_>) -> DecodeResult<Option<String>> {
    ensure_size!(in: src, size: 1);
    match src.read_u8() {
        0 => Ok(None),
        1 => Ok(Some(read_string(src)?)),
        _ => Err(ironrdp_core::invalid_field_err!(
            "optional string",
            "invalid presence flag"
        )),
    }
}

pub fn write_bool(dst: &mut WriteCursor<'_>, value: bool) -> EncodeResult<()> {
    ensure_size!(in: dst, size: 1);
    dst.write_u8(u8::from(value));
    Ok(())
}

pub fn read_bool(src: &mut ReadCursor<'_>) -> DecodeResult<bool> {
    ensure_size!(in: src, size: 1);
    Ok(src.read_u8() != 0)
}

pub fn write_char(dst: &mut WriteCursor<'_>, value: char) -> EncodeResult<()> {
    ensure_size!(in: dst, size: 4);
    dst.write_u32(u32::from(value));
    Ok(())
}

pub fn read_char(src: &mut ReadCursor<'_>) -> DecodeResult<char> {
    ensure_size!(in: src, size: 4);
    let code = src.read_u32();
    char::from_u32(code).ok_or_else(|| ironrdp_core::invalid_field_err!("char", "not a valid Unicode scalar value"))
}

pub fn write_mouse_button(dst: &mut WriteCursor<'_>, button: MouseButton) -> EncodeResult<()> {
    ensure_size!(in: dst, size: 1);
    let idx: u8 = cast_length!("mouse button index", button.as_idx())?;
    dst.write_u8(idx);
    Ok(())
}

pub fn read_mouse_button(src: &mut ReadCursor<'_>) -> DecodeResult<MouseButton> {
    ensure_size!(in: src, size: 1);
    let idx = src.read_u8();
    MouseButton::from_idx(usize::from(idx))
        .ok_or_else(|| ironrdp_core::invalid_field_err!("mouse button", "unknown button index"))
}

pub fn opt_u16_size(value: Option<u16>) -> usize {
    1 /* presence */ + value.map_or(0, |_| 2)
}

pub fn write_opt_u16(dst: &mut WriteCursor<'_>, value: Option<u16>) -> EncodeResult<()> {
    ensure_size!(in: dst, size: opt_u16_size(value));
    match value {
        Some(value) => {
            dst.write_u8(1);
            dst.write_u16(value);
        }
        None => dst.write_u8(0),
    }
    Ok(())
}

pub fn read_opt_u16(src: &mut ReadCursor<'_>) -> DecodeResult<Option<u16>> {
    ensure_size!(in: src, size: 1);
    match src.read_u8() {
        0 => Ok(None),
        1 => {
            ensure_size!(in: src, size: 2);
            Ok(Some(src.read_u16()))
        }
        _ => Err(ironrdp_core::invalid_field_err!(
            "optional u16",
            "invalid presence flag"
        )),
    }
}
