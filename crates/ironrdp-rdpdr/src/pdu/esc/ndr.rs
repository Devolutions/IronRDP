//! Request/response messages are nested structs with fields, encoded as NDR (network data
//! representation).
//!
//! Fixed-size fields are encoded in-line as they appear in the struct.
//!
//! Variable-sized fields (strings, byte arrays, sometimes structs) are encoded as pointers:
//! - in place of the field in the struct, a "pointer" is written
//! - the pointer value is 0x0002xxxx, where xxxx is an "index" in increments of 4
//! - for example, first pointer is 0x0002_0000, second is 0x0002_0004, third is 0x0002_0008 etc.
//! - the actual values are then appended at the end of the message, in the same order as their
//!   pointers appeared
//! - in the code below, "*_ptr" is the pointer value and "*_value" the actual data
//! - note that some fields (like arrays) will have a length prefix before the pointer and also
//!   before the actual data at the end of the message
//!
//! To deal with this, fixed-size structs only have encode/decode methods, while variable-size ones
//! have encode_ptr/decode_ptr and encode_value/decode_value methods. Messages are parsed linearly,
//! so decode_ptr/decode_value are called at different stages (same for encoding).
//!
//! Most of the above was reverse-engineered from FreeRDP:
//! https://github.com/FreeRDP/FreeRDP/blob/master/channels/smartcard/client/smartcard_pack.c

use std::mem::size_of;

use ironrdp_pdu::{
    cursor::{ReadCursor, WriteCursor},
    ensure_size, invalid_message_err,
    utils::{self, CharacterSet},
    PduResult,
};

use super::ReaderStateCommonCall;

/// [2.2.1.1 REDIR_SCARDCONTEXT]
///
/// [2.2.1.1 REDIR_SCARDCONTEXT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/060abee1-e520-4149-9ef7-ce79eb500a59
#[derive(Debug)]
pub struct ScardContext {
    pub length: u32,
    // Shortcut: we always create 4-byte context values.
    // The spec allows this field to have variable length.
    pub value: u32,
}

impl ScardContext {
    const NAME: &'static str = "REDIR_SCARDCONTEXT";

    pub fn new(value: u32) -> Self {
        Self {
            length: size_of::<u32>() as u32,
            value,
        }
    }

    pub fn encode_ptr(&self, index: &mut u32, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        encode_ptr(Some(self.length), index, dst)
    }

    pub fn encode_value(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        dst.write_u32(self.length);
        dst.write_u32(self.value);
        Ok(())
    }

    pub fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> PduResult<Self> {
        ensure_size!(in: src, size: size_of::<u32>());
        let length = src.read_u32();
        let _ptr = decode_ptr(src, index)?;
        Ok(Self { length, value: 0 })
    }

    pub fn decode_value(&mut self, src: &mut ReadCursor<'_>) -> PduResult<()> {
        ensure_size!(in: src, size: size_of::<u32>()*2);
        let length = src.read_u32();
        if length != self.length {
            Err(invalid_message_err!(
                "decode_value",
                "mismatched length in ScardContext reference and value"
            ))
        } else {
            self.value = src.read_u32();
            Ok(())
        }
    }

    pub fn size(&self) -> usize {
        ptr_size(true) + size_of::<u32>() * 2
    }
}

/// [2.2.1.7 ReaderStateW]
///
/// [2.2.1.7 ReaderStateW]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/0ba03cd2-bed0-495b-adbe-3d2cde61980c
#[derive(Debug)]
pub struct ReaderState {
    pub reader: String,
    pub common: ReaderStateCommonCall,
}

impl ReaderState {
    pub fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> PduResult<Self> {
        let _reader_ptr = decode_ptr(src, index)?;
        let common = ReaderStateCommonCall::decode(src)?;
        Ok(Self {
            reader: String::new(),
            common,
        })
    }

    pub fn decode_value(&mut self, src: &mut ReadCursor<'_>) -> PduResult<()> {
        self.reader = read_string_from_cursor(src)?;
        Ok(())
    }
}

pub fn encode_ptr(length: Option<u32>, index: &mut u32, dst: &mut WriteCursor<'_>) -> PduResult<()> {
    ensure_size!(ctx: "encode_ptr", in: dst, size: ptr_size(length.is_some()));
    if let Some(length) = length {
        dst.write_u32(length);
    }

    dst.write_u32(0x0002_0000 + *index * 4);
    *index += 1;
    Ok(())
}

pub fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> PduResult<u32> {
    ensure_size!(ctx: "decode_ptr", in: src, size: size_of::<u32>());
    let ptr = src.read_u32();
    if ptr == 0 {
        // NULL pointer is OK. Don't update index.
        return Ok(ptr);
    }
    let expect_ptr = 0x0002_0000 + *index * 4;
    *index += 1;
    if ptr != expect_ptr {
        Err(invalid_message_err!("decode_ptr", "ptr", "ptr != expect_ptr"))
    } else {
        Ok(ptr)
    }
}

pub fn ptr_size(with_length: bool) -> usize {
    if with_length {
        size_of::<u32>() * 2
    } else {
        size_of::<u32>()
    }
}

/// A special read_string_from_cursor which reads and ignores the additional length and
/// offset fields prefixing the string, as well as any extra padding for a 4-byte aligned
/// NULL-terminated string.
pub fn read_string_from_cursor(cursor: &mut ReadCursor<'_>) -> PduResult<String> {
    ensure_size!(ctx: "ndr::read_string_from_cursor", in: cursor, size: size_of::<u32>() * 3);
    let length = cursor.read_u32();
    let _offset = cursor.read_u32();
    let _length2 = cursor.read_u32();

    let string = utils::read_string_from_cursor(cursor, CharacterSet::Unicode, true)?;

    // Skip padding for 4-byte aligned NULL-terminated string.
    if length % 2 != 0 {
        ensure_size!(ctx: "ndr::read_string_from_cursor", in: cursor, size: size_of::<u16>());
        let _padding = cursor.read_u16();
    }

    Ok(string)
}
