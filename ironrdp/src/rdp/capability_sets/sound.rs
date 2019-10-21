#[cfg(test)]
mod test;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::{rdp::CapabilitySetsError, PduParsing};

const SOUND_LENGTH: usize = 4;

bitflags! {
    pub struct SoundFlags: u16 {
        const BEEPS = 1;
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct Sound {
    pub flags: SoundFlags,
}

impl PduParsing for Sound {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let flags = SoundFlags::from_bits_truncate(buffer.read_u16::<LittleEndian>()?);
        let _padding = buffer.read_u16::<LittleEndian>()?;

        Ok(Sound { flags })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(self.flags.bits())?;
        buffer.write_u16::<LittleEndian>(0)?; // padding

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        SOUND_LENGTH
    }
}
