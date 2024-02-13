#[cfg(test)]
mod tests;

use bitflags::bitflags;

use crate::cursor::{ReadCursor, WriteCursor};
use crate::{PduDecode, PduEncode, PduResult};

const SOUND_LENGTH: usize = 4;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct SoundFlags: u16 {
        const BEEPS = 1;
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Sound {
    pub flags: SoundFlags,
}

impl Sound {
    const NAME: &'static str = "Sound";

    const FIXED_PART_SIZE: usize = SOUND_LENGTH;
}

impl PduEncode for Sound {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.flags.bits());
        write_padding!(dst, 2);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for Sound {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let flags = SoundFlags::from_bits_truncate(src.read_u16());
        read_padding!(src, 2);

        Ok(Sound { flags })
    }
}
