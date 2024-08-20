#[cfg(test)]
mod tests;

use bitflags::bitflags;

use crate::{PduDecode, PduEncode, PduResult};
use ironrdp_core::{ReadCursor, WriteCursor};

const SURFACE_COMMANDS_LENGTH: usize = 8;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct CmdFlags: u32 {
        const SET_SURFACE_BITS = 0x02;
        const FRAME_MARKER = 0x10;
        const STREAM_SURFACE_BITS = 0x40;
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct SurfaceCommands {
    pub flags: CmdFlags,
}

impl SurfaceCommands {
    const NAME: &'static str = "SurfaceCommands";

    const FIXED_PART_SIZE: usize = SURFACE_COMMANDS_LENGTH;
}

impl PduEncode for SurfaceCommands {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.flags.bits());
        dst.write_u32(0); // reserved

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for SurfaceCommands {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let flags = CmdFlags::from_bits_truncate(src.read_u32());
        let _reserved = src.read_u32();

        Ok(SurfaceCommands { flags })
    }
}
