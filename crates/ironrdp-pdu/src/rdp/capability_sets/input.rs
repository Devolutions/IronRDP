#[cfg(test)]
mod tests;

use bitflags::bitflags;
use num_traits::{FromPrimitive, ToPrimitive};

use crate::gcc::{KeyboardType, IME_FILE_NAME_SIZE};
use crate::{utils, Decode, DecodeResult, Encode, EncodeResult};
use ironrdp_core::{ensure_fixed_part_size, ReadCursor, WriteCursor};

const INPUT_LENGTH: usize = 84;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct InputFlags: u16 {
        const SCANCODES = 0x0001;
        const MOUSEX = 0x0004;
        const FASTPATH_INPUT = 0x0008;
        const UNICODE = 0x0010;
        const FASTPATH_INPUT_2 = 0x0020;
        const UNUSED_1 = 0x0040;
        const MOUSE_RELATIVE = 0x0080;
        const TS_MOUSE_HWHEEL = 0x0100;
        const TS_QOE_TIMESTAMPS = 0x0200;
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Input {
    pub input_flags: InputFlags,
    pub keyboard_layout: u32,
    pub keyboard_type: Option<KeyboardType>,
    pub keyboard_subtype: u32,
    pub keyboard_function_key: u32,
    pub keyboard_ime_filename: String,
}

impl Input {
    const NAME: &'static str = "Input";

    const FIXED_PART_SIZE: usize = INPUT_LENGTH;
}

impl Encode for Input {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.input_flags.bits());
        write_padding!(dst, 2);
        dst.write_u32(self.keyboard_layout);

        let type_buffer = match self.keyboard_type.as_ref() {
            Some(value) => value.to_u32().unwrap_or(0),
            None => 0,
        };
        dst.write_u32(type_buffer);

        dst.write_u32(self.keyboard_subtype);
        dst.write_u32(self.keyboard_function_key);

        utils::encode_string(
            dst.remaining_mut(),
            &self.keyboard_ime_filename,
            utils::CharacterSet::Unicode,
            true,
        )?;
        dst.advance(IME_FILE_NAME_SIZE);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for Input {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let input_flags = InputFlags::from_bits_truncate(src.read_u16());
        read_padding!(src, 2);
        let keyboard_layout = src.read_u32();

        let keyboard_type = KeyboardType::from_u32(src.read_u32());

        let keyboard_subtype = src.read_u32();
        let keyboard_function_key = src.read_u32();

        let keyboard_ime_filename =
            utils::decode_string(src.read_slice(IME_FILE_NAME_SIZE), utils::CharacterSet::Unicode, false)?;

        Ok(Input {
            input_flags,
            keyboard_layout,
            keyboard_type,
            keyboard_subtype,
            keyboard_function_key,
            keyboard_ime_filename,
        })
    }
}
