#[cfg(test)]
mod test;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{
    gcc::{KeyboardType, IME_FILE_NAME_SIZE},
    rdp::CapabilitySetsError,
    PduParsing,
};

const INPUT_LENGTH: usize = 84;

bitflags! {
    pub struct InputFlags: u16 {
        const SCANCODES = 0x0001;
        const MOUSEX = 0x0004;
        const FASTPATH_INPUT = 0x0008;
        const UNICODE = 0x0010;
        const FASTPATH_INPUT_2 = 0x0020;
        const UNUSED_1 = 0x0040;
        const UNUSED_2 = 0x0080;
        const TS_MOUSE_HWHEEL = 0x0100;
        const TS_QOE_TIMESTAMPS = 0x0200;
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct Input {
    pub input_flags: InputFlags,
    pub keyboard_layout: u32,
    pub keyboard_type: Option<KeyboardType>,
    pub keyboard_subtype: u32,
    pub keyboard_function_key: u32,
    pub keyboard_ime_filename: String,
}

impl PduParsing for Input {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let input_flags = InputFlags::from_bits_truncate(buffer.read_u16::<LittleEndian>()?);
        let _padding = buffer.read_u16::<LittleEndian>()?;
        let keyboard_layout = buffer.read_u32::<LittleEndian>()?;

        let keyboard_type = KeyboardType::from_u32(buffer.read_u32::<LittleEndian>()?);

        let keyboard_subtype = buffer.read_u32::<LittleEndian>()?;
        let keyboard_function_key = buffer.read_u32::<LittleEndian>()?;

        let mut ime_buffer = [0; IME_FILE_NAME_SIZE];
        buffer.read_exact(&mut ime_buffer)?;
        let keyboard_ime_filename = sspi::utils::bytes_to_utf16_string(ime_buffer.as_ref())
            .trim_end_matches('\u{0}')
            .into();

        Ok(Input {
            input_flags,
            keyboard_layout,
            keyboard_type,
            keyboard_subtype,
            keyboard_function_key,
            keyboard_ime_filename,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(self.input_flags.bits())?;
        buffer.write_u16::<LittleEndian>(0)?; // padding
        buffer.write_u32::<LittleEndian>(self.keyboard_layout)?;

        let type_buffer = match self.keyboard_type.as_ref() {
            Some(value) => match value.to_u32() {
                Some(n) => n,
                None => 0,
            },
            None => 0,
        };
        buffer.write_u32::<LittleEndian>(type_buffer)?;

        buffer.write_u32::<LittleEndian>(self.keyboard_subtype)?;
        buffer.write_u32::<LittleEndian>(self.keyboard_function_key)?;

        let mut keyboard_ime_file_name_buffer =
            sspi::utils::string_to_utf16(self.keyboard_ime_filename.as_ref());
        keyboard_ime_file_name_buffer.resize(IME_FILE_NAME_SIZE - 2, 0);
        buffer.write_all(keyboard_ime_file_name_buffer.as_ref())?;
        buffer.write_u16::<LittleEndian>(0)?; // ime file name null terminator

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        INPUT_LENGTH
    }
}
