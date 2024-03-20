use bitflags::bitflags;

use crate::{
    cursor::{ReadCursor, WriteCursor},
    PduDecode, PduEncode, PduResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MouseXPdu {
    pub flags: PointerXFlags,
    pub x_position: u16,
    pub y_position: u16,
}

impl MouseXPdu {
    const NAME: &'static str = "MouseXPdu";

    const FIXED_PART_SIZE: usize = 2 /* flags */ + 2 /* x */ + 2 /* y */;
}

impl PduEncode for MouseXPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.flags.bits());
        dst.write_u16(self.x_position);
        dst.write_u16(self.y_position);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for MouseXPdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let flags = PointerXFlags::from_bits_truncate(src.read_u16());
        let x_position = src.read_u16();
        let y_position = src.read_u16();

        Ok(Self {
            flags,
            x_position,
            y_position,
        })
    }
}

impl_pdu_parsing!(MouseXPdu);

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct PointerXFlags: u16 {
        const DOWN = 0x8000;
        const BUTTON1 = 0x0001;
        const BUTTON2 = 0x0002;
    }
}
