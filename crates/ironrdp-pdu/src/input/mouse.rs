use bitflags::bitflags;

use crate::{
    cursor::{ReadCursor, WriteCursor},
    PduDecode, PduEncode, PduResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MousePdu {
    pub flags: PointerFlags,
    pub number_of_wheel_rotation_units: i16,
    pub x_position: u16,
    pub y_position: u16,
}

impl MousePdu {
    const NAME: &'static str = "MousePdu";

    const FIXED_PART_SIZE: usize = 2 /* flags */ + 2 /* x */ + 2 /* y */;
}

impl PduEncode for MousePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        let wheel_negative_bit = if self.number_of_wheel_rotation_units < 0 {
            PointerFlags::WHEEL_NEGATIVE.bits()
        } else {
            PointerFlags::empty().bits()
        };

        let wheel_rotations_bits = u16::from(self.number_of_wheel_rotation_units as u8); // truncate

        let flags = self.flags.bits() | wheel_negative_bit | wheel_rotations_bits;

        dst.write_u16(flags);
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

impl<'de> PduDecode<'de> for MousePdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let flags_raw = src.read_u16();

        let flags = PointerFlags::from_bits_truncate(flags_raw);

        let wheel_rotations_bits = flags_raw as u8; // truncate

        let number_of_wheel_rotation_units = if flags.contains(PointerFlags::WHEEL_NEGATIVE) {
            -i16::from(wheel_rotations_bits)
        } else {
            i16::from(wheel_rotations_bits)
        };

        let x_position = src.read_u16();
        let y_position = src.read_u16();

        Ok(Self {
            flags,
            number_of_wheel_rotation_units,
            x_position,
            y_position,
        })
    }
}
impl_pdu_parsing!(MousePdu);

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct PointerFlags: u16 {
        const WHEEL_NEGATIVE = 0x0100;
        const VERTICAL_WHEEL = 0x0200;
        const HORIZONTAL_WHEEL = 0x0400;
        const MOVE = 0x0800;
        const LEFT_BUTTON = 0x1000;
        const RIGHT_BUTTON = 0x2000;
        const MIDDLE_BUTTON_OR_WHEEL = 0x4000;
        const DOWN = 0x8000;
    }
}
