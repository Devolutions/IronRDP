use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::InputEventError;
use crate::PduParsing;

#[derive(Debug, Clone, PartialEq)]
pub struct SyncPdu {
    pub flags: SyncToggleFlags,
}

impl PduParsing for SyncPdu {
    type Error = InputEventError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let _padding = stream.read_u16::<LittleEndian>()?;
        let flags = SyncToggleFlags::from_bits_truncate(stream.read_u32::<LittleEndian>()?);

        Ok(Self { flags })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(0)?; // padding
        stream.write_u32::<LittleEndian>(self.flags.bits())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        6
    }
}

bitflags! {
    pub struct SyncToggleFlags: u32 {
        const SCROLL_LOCK = 0x1;
        const NUM_LOCK = 0x2;
        const CAPS_LOCK = 0x4;
        const KANA_LOCK = 0x8;
    }
}
