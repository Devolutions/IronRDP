#[cfg(test)]
mod test;

use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::{rdp::CapabilitySetsError, PduParsing};

const OFFSCREEN_BITMAP_CACHE_LENGTH: usize = 8;

#[derive(Debug, PartialEq, Clone)]
pub struct OffscreenBitmapCache {
    pub is_supported: bool,
    pub cache_size: u16,
    pub cache_entries: u16,
}

impl PduParsing for OffscreenBitmapCache {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let is_supported = buffer.read_u32::<LittleEndian>()? != 0;
        let cache_size = buffer.read_u16::<LittleEndian>()?;
        let cache_entries = buffer.read_u16::<LittleEndian>()?;

        Ok(OffscreenBitmapCache {
            is_supported,
            cache_size,
            cache_entries,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(u32::from(self.is_supported))?;
        buffer.write_u16::<LittleEndian>(self.cache_size)?;
        buffer.write_u16::<LittleEndian>(self.cache_entries)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        OFFSCREEN_BITMAP_CACHE_LENGTH
    }
}
