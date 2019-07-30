#[cfg(test)]
mod test;

use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::rdp::CapabilitySetsError;
use crate::PduParsing;

const GLYPH_CACHE_NUM: usize = 10;
const GLYPH_CACHE_LENGTH: usize = 48;
const CACHE_DEFINITION_LENGTH: usize = 4;

#[derive(Copy, Clone, Debug, PartialEq, FromPrimitive, ToPrimitive)]
pub enum GlyphSupportLevel {
    None = 0,
    Partial = 1,
    Full = 2,
    Encode = 3,
}

#[derive(Debug, PartialEq, Copy, Clone, Default)]
pub struct CacheDefinition {
    pub entries: u16,
    pub max_cell_size: u16,
}

impl PduParsing for CacheDefinition {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let entries = buffer.read_u16::<LittleEndian>()?;
        let max_cell_size = buffer.read_u16::<LittleEndian>()?;

        Ok(CacheDefinition {
            entries,
            max_cell_size,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(self.entries)?;
        buffer.write_u16::<LittleEndian>(self.max_cell_size)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        CACHE_DEFINITION_LENGTH
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct GlyphCache {
    pub glyph_cache: [CacheDefinition; GLYPH_CACHE_NUM],
    pub frag_cache: CacheDefinition,
    pub glyph_support_level: GlyphSupportLevel,
}

impl PduParsing for GlyphCache {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let mut glyph_cache = [CacheDefinition::default(); GLYPH_CACHE_NUM];

        for glyph in glyph_cache.iter_mut() {
            *glyph = CacheDefinition::from_buffer(&mut buffer)?;
        }

        let frag_cache = CacheDefinition::from_buffer(&mut buffer)?;
        let glyph_support_level = GlyphSupportLevel::from_u16(buffer.read_u16::<LittleEndian>()?)
            .ok_or(CapabilitySetsError::InvalidGlyphSupportLevel)?;
        let _padding = buffer.read_u16::<LittleEndian>()?;

        Ok(GlyphCache {
            glyph_cache,
            frag_cache,
            glyph_support_level,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        for glyph in self.glyph_cache.iter() {
            glyph.to_buffer(&mut buffer)?;
        }

        self.frag_cache.to_buffer(&mut buffer)?;

        buffer.write_u16::<LittleEndian>(self.glyph_support_level.to_u16().unwrap())?;
        buffer.write_u16::<LittleEndian>(0)?; // padding

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        GLYPH_CACHE_LENGTH
    }
}
