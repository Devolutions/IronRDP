#[cfg(test)]
mod tests;

use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::rdp::capability_sets::CapabilitySetsError;
use crate::PduParsing;

const BRUSH_LENGTH: usize = 4;

#[derive(Copy, Clone, Debug, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum SupportLevel {
    Default = 0,
    Color8x8 = 1,
    ColorFull = 2,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Brush {
    pub support_level: SupportLevel,
}

impl PduParsing for Brush {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let support_level = SupportLevel::from_u32(buffer.read_u32::<LittleEndian>()?)
            .ok_or(CapabilitySetsError::InvalidBrushSupportLevel)?;

        Ok(Brush { support_level })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(self.support_level.to_u32().unwrap())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        BRUSH_LENGTH
    }
}
