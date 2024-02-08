use bitflags::bitflags;

use crate::{
    cursor::{ReadCursor, WriteCursor},
    PduDecode, PduEncode, PduResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnicodePdu {
    pub flags: KeyboardFlags,
    pub unicode_code: u16,
}

impl UnicodePdu {
    const NAME: &'static str = "UnicodePdu";

    const FIXED_PART_SIZE: usize = 2 /* flags */ + 2 /* code */ + 2 /* padding */;
}

impl PduEncode for UnicodePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.flags.bits());
        dst.write_u16(self.unicode_code);
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

impl<'de> PduDecode<'de> for UnicodePdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let flags = KeyboardFlags::from_bits_truncate(src.read_u16());
        let unicode_code = src.read_u16();
        read_padding!(src, 2);

        Ok(Self { flags, unicode_code })
    }
}

impl_pdu_parsing!(UnicodePdu);

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct KeyboardFlags: u16 {
        const RELEASE = 0x8000;
    }
}
