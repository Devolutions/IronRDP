use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::InputEventError;
use crate::PduParsing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MouseRelPdu {
    pub flags: PointerRelFlags,
    pub x_delta: i16,
    pub y_delta: i16,
}

impl PduParsing for MouseRelPdu {
    type Error = InputEventError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let flags = PointerRelFlags::from_bits_truncate(stream.read_u16::<LittleEndian>()?);
        let x_delta = stream.read_i16::<LittleEndian>()?;
        let y_delta = stream.read_i16::<LittleEndian>()?;

        Ok(Self {
            flags,
            x_delta,
            y_delta,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.flags.bits())?;
        stream.write_i16::<LittleEndian>(self.x_delta)?;
        stream.write_i16::<LittleEndian>(self.y_delta)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        6
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct PointerRelFlags: u16 {
        const MOVE = 0x0800;
        const DOWN = 0x8000;
        const BUTTON1 = 0x1000;
        const BUTTON2 = 0x2000;
        const BUTTON3 = 0x4000;
        const XBUTTON1 = 0x0001;
        const XBUTTON2 = 0x0002;
    }
}
