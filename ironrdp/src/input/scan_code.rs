use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::InputEventError;
use crate::PduParsing;

#[derive(Debug, Clone, PartialEq)]
pub struct ScanCodePdu {
    pub flags: KeyboardFlags,
    pub key_code: u16,
}

impl PduParsing for ScanCodePdu {
    type Error = InputEventError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let flags = KeyboardFlags::from_bits_truncate(stream.read_u16::<LittleEndian>()?);
        let key_code = stream.read_u16::<LittleEndian>()?;
        let _padding = stream.read_u16::<LittleEndian>()?;

        Ok(Self { flags, key_code })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.flags.bits())?;
        stream.write_u16::<LittleEndian>(self.key_code)?;
        stream.write_u16::<LittleEndian>(0)?; // padding

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        6
    }
}

bitflags! {
    pub struct KeyboardFlags: u16 {
        const EXTENDED = 0x0100;
        const EXTENDED_1 = 0x0200;
        const DOWN = 0x4000;
        const RELEASE = 0x8000;
    }
}
