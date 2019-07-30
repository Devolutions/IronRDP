#[cfg(test)]
mod test;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::rdp::CapabilitySetsError;
use crate::PduParsing;

const SURFACE_COMMANDS_LENGTH: usize = 8;

bitflags! {
    pub struct CmdFlags: u32 {
        const SET_SURFACE_BITS = 0x02;
        const FRAME_MARKER = 0x10;
        const STREAM_SURFACE_BITS = 0x40;
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct SurfaceCommands {
    flags: CmdFlags,
}

impl PduParsing for SurfaceCommands {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let flags = CmdFlags::from_bits_truncate(buffer.read_u32::<LittleEndian>()?);
        let _reserved = buffer.read_u32::<LittleEndian>()?;

        Ok(SurfaceCommands { flags })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(self.flags.bits())?;
        buffer.write_u32::<LittleEndian>(0)?; // reserved

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        SURFACE_COMMANDS_LENGTH
    }
}
