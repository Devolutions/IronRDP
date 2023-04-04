use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::InputEventError;
use crate::PduParsing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MouseXPdu {
    pub flags: PointerXFlags,
    pub x_position: u16,
    pub y_position: u16,
}

impl PduParsing for MouseXPdu {
    type Error = InputEventError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let flags = PointerXFlags::from_bits_truncate(stream.read_u16::<LittleEndian>()?);
        let x_position = stream.read_u16::<LittleEndian>()?;
        let y_position = stream.read_u16::<LittleEndian>()?;

        Ok(Self {
            flags,
            x_position,
            y_position,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.flags.bits())?;
        stream.write_u16::<LittleEndian>(self.x_position)?;
        stream.write_u16::<LittleEndian>(self.y_position)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        6
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct PointerXFlags: u16 {
        const DOWN = 0x8000;
        const BUTTON1 = 0x0001;
        const BUTTON2 = 0x0002;
    }
}
