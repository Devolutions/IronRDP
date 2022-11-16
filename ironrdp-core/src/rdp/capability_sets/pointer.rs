#[cfg(test)]
mod test;

use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::rdp::CapabilitySetsError;
use crate::PduParsing;

const POINTER_LENGTH: usize = 6;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Pointer {
    pub color_pointer_cache_size: u16,
    pub pointer_cache_size: u16,
}

impl PduParsing for Pointer {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let _color_pointer_flag = buffer.read_u16::<LittleEndian>()? != 0;
        let color_pointer_cache_size = buffer.read_u16::<LittleEndian>()?;
        let pointer_cache_size = buffer.read_u16::<LittleEndian>()?;

        Ok(Pointer {
            color_pointer_cache_size,
            pointer_cache_size,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(1)?; // color pointer flag
        buffer.write_u16::<LittleEndian>(self.color_pointer_cache_size)?;
        buffer.write_u16::<LittleEndian>(self.pointer_cache_size)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        POINTER_LENGTH
    }
}
