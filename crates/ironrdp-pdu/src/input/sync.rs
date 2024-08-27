use bitflags::bitflags;

use crate::{Decode, DecodeResult, Encode, EncodeResult};
use ironrdp_core::{ensure_fixed_part_size, ReadCursor, WriteCursor};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPdu {
    pub flags: SyncToggleFlags,
}

impl SyncPdu {
    const NAME: &'static str = "SyncPdu";

    const FIXED_PART_SIZE: usize = 2 /* padding */ + 4 /* flags */;
}

impl Encode for SyncPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        write_padding!(dst, 2);
        dst.write_u32(self.flags.bits());

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for SyncPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        read_padding!(src, 2);
        let flags = SyncToggleFlags::from_bits_truncate(src.read_u32());

        Ok(Self { flags })
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct SyncToggleFlags: u32 {
        const SCROLL_LOCK = 0x1;
        const NUM_LOCK = 0x2;
        const CAPS_LOCK = 0x4;
        const KANA_LOCK = 0x8;
    }
}
