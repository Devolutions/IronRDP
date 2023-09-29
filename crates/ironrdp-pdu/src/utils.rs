use std::io;

use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::ToPrimitive as _;

use crate::cursor::{ReadCursor, WriteCursor};
use crate::PduResult;

pub fn split_u64(value: u64) -> (u32, u32) {
    let bytes = value.to_le_bytes();
    let (low, high) = bytes.split_at(std::mem::size_of::<u32>());
    (
        u32::from_le_bytes(low.try_into().unwrap()),
        u32::from_le_bytes(high.try_into().unwrap()),
    )
}

pub fn combine_u64(lo: u32, hi: u32) -> u64 {
    let mut position_bytes = [0u8; std::mem::size_of::<u64>()];
    position_bytes[..std::mem::size_of::<u32>()].copy_from_slice(&lo.to_le_bytes());
    position_bytes[std::mem::size_of::<u32>()..].copy_from_slice(&hi.to_le_bytes());
    u64::from_le_bytes(position_bytes)
}

pub fn to_utf16_bytes(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .flat_map(|i| i.to_le_bytes().to_vec())
        .collect::<Vec<u8>>()
}

pub fn from_utf16_bytes(mut value: &[u8]) -> String {
    let mut value_u16 = vec![0x00; value.len() / 2];
    value
        .read_u16_into::<LittleEndian>(value_u16.as_mut())
        .expect("read_u16_into cannot fail at this point");

    String::from_utf16_lossy(value_u16.as_ref())
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum CharacterSet {
    Ansi = 1,
    Unicode = 2,
}

pub fn read_string_from_cursor(
    cursor: &mut ReadCursor<'_>,
    character_set: CharacterSet,
    read_null_terminator: bool,
) -> PduResult<String> {
    let size = if character_set == CharacterSet::Unicode {
        let code_units = if read_null_terminator {
            // Find null or read all if null is not found
            cursor
                .remaining()
                .chunks_exact(2)
                .position(|chunk| chunk[0] == 0 && chunk[1] == 0)
                .map(|code_units| code_units + 1) // Read null code point
                .unwrap_or(cursor.len() / 2)
        } else {
            // UTF16 uses 2 bytes per code unit, so we need to read an even number of bytes
            cursor.len() / 2
        };

        code_units * 2
    } else if read_null_terminator {
        // Find null or read all if null is not found
        cursor
            .remaining()
            .iter()
            .position(|&i| i == 0)
            .map(|code_units| code_units + 1) // Read null code point
            .unwrap_or(cursor.len())
    } else {
        // Read all
        cursor.len()
    };

    // Empty string, nothing to do
    if size == 0 {
        return Ok(String::new());
    }

    let result = match character_set {
        CharacterSet::Unicode => {
            ensure_size!(ctx: "Decode string (UTF-16)", in: cursor, size: size);
            let mut slice = cursor.read_slice(size);

            let str_buffer = &mut slice;
            let mut u16_buffer = vec![0u16; str_buffer.len() / 2];

            str_buffer
                .read_u16_into::<LittleEndian>(u16_buffer.as_mut())
                .expect("BUG: str_buffer is always even for UTF16");

            String::from_utf16(&u16_buffer)
                .map_err(|_| invalid_message_err!("UTF16 decode", "buffer", "Failed to decode UTF16 string"))?
        }
        CharacterSet::Ansi => {
            ensure_size!(ctx: "Decode string (UTF-8)", in: cursor, size: size);
            let slice = cursor.read_slice(size);
            String::from_utf8(slice.to_vec())
                .map_err(|_| invalid_message_err!("UTF8 decode", "buffer", "Failed to decode UTF8 string"))?
        }
    };

    Ok(result.trim_end_matches('\0').into())
}

pub fn write_string_to_cursor(
    cursor: &mut WriteCursor<'_>,
    value: &str,
    character_set: CharacterSet,
    write_null_terminator: bool,
) -> PduResult<()> {
    let (buffer, ctx) = match character_set {
        CharacterSet::Unicode => {
            let mut buffer = to_utf16_bytes(value);
            if write_null_terminator {
                buffer.push(0);
                buffer.push(0);
            }
            (buffer, "Encode string (UTF-16)")
        }
        CharacterSet::Ansi => {
            let mut buffer = value.as_bytes().to_vec();
            if write_null_terminator {
                buffer.push(0);
            }
            (buffer, "Encode string (UTF-8)")
        }
    };

    ensure_size!(ctx: ctx, in: cursor, size: buffer.len());
    cursor.write_slice(&buffer);
    Ok(())
}

/// Returns the length in bytes of the encoded value
/// based on the passed CharacterSet and with_null_terminator flag.
pub fn encoded_str_len(value: &str, character_set: CharacterSet, with_null_terminator: bool) -> usize {
    match character_set {
        CharacterSet::Ansi => value.len() + if with_null_terminator { 1 } else { 0 },
        CharacterSet::Unicode => value.encode_utf16().count() * 2 + if with_null_terminator { 2 } else { 0 },
    }
}

pub(crate) fn read_string(
    mut stream: impl io::Read,
    size: usize,
    character_set: CharacterSet,
    read_null_terminator: bool,
) -> io::Result<String> {
    let size = size
        + if read_null_terminator {
            character_set.to_usize().unwrap()
        } else {
            0
        };
    let mut buffer = vec![0; size];
    stream.read_exact(&mut buffer)?;

    let result = match character_set {
        CharacterSet::Unicode => from_utf16_bytes(buffer.as_slice()),
        CharacterSet::Ansi => String::from_utf8(buffer)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("the string is not utf8: {e}")))?,
    };

    Ok(result.trim_end_matches('\0').into())
}

pub(crate) fn write_string_with_null_terminator(
    mut stream: impl io::Write,
    value: &str,
    character_set: CharacterSet,
) -> io::Result<()> {
    match character_set {
        CharacterSet::Unicode => {
            stream.write_all(to_utf16_bytes(value).as_ref())?;
            stream.write_u16::<LittleEndian>(0)
        }
        CharacterSet::Ansi => {
            stream.write_all(value.as_bytes())?;
            stream.write_u8(0)
        }
    }
}

pub trait SplitTo {
    #[must_use]
    fn split_to(&mut self, n: usize) -> Self;
}

impl<T> SplitTo for &[T] {
    fn split_to(&mut self, n: usize) -> Self {
        assert!(n <= self.len());

        let (a, b) = self.split_at(n);
        *self = b;

        a
    }
}

impl<T> SplitTo for &mut [T] {
    fn split_to(&mut self, n: usize) -> Self {
        assert!(n <= self.len());

        let (a, b) = std::mem::take(self).split_at_mut(n);
        *self = b;

        a
    }
}
