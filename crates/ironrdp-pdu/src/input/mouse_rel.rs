use bitflags::bitflags;

use crate::{
    cursor::{ReadCursor, WriteCursor},
    PduDecode, PduEncode, PduResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MouseRelPdu {
    pub flags: PointerRelFlags,
    pub x_delta: i16,
    pub y_delta: i16,
}

impl MouseRelPdu {
    const NAME: &'static str = "MouseRelPdu";

    const FIXED_PART_SIZE: usize = 2 /* flags */ + 2 /* x */ + 2 /* y */;
}

impl PduEncode for MouseRelPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.flags.bits());
        dst.write_i16(self.x_delta);
        dst.write_i16(self.y_delta);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for MouseRelPdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let flags = PointerRelFlags::from_bits_truncate(src.read_u16());
        let x_delta = src.read_i16();
        let y_delta = src.read_i16();

        Ok(Self {
            flags,
            x_delta,
            y_delta,
        })
    }
}

impl_pdu_parsing!(MouseRelPdu);

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct PointerRelFlags: u16 {
        const MOVE = 0x0800;
        const DOWN = 0x8000;
        const BUTTON1 = 0x1000;
        const BUTTON2 = 0x2000;
        const BUTTON3 = 0x4000;
        const XBUTTON1 = 0x0001;
        const XBUTTON2 = 0x0002;
    }
}
