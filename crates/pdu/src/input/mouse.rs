use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::InputEventError;
use crate::PduParsing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MousePdu {
    pub flags: PointerFlags,
    pub number_of_wheel_rotation_units: i16,
    pub x_position: u16,
    pub y_position: u16,
}

impl PduParsing for MousePdu {
    type Error = InputEventError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let flags_raw = stream.read_u16::<LittleEndian>()?;

        let flags = PointerFlags::from_bits_truncate(flags_raw);

        let wheel_rotations_bits = flags_raw as u8; // truncate

        let number_of_wheel_rotation_units = if flags.contains(PointerFlags::WHEEL_NEGATIVE) {
            -i16::from(wheel_rotations_bits)
        } else {
            i16::from(wheel_rotations_bits)
        };

        let x_position = stream.read_u16::<LittleEndian>()?;
        let y_position = stream.read_u16::<LittleEndian>()?;

        Ok(Self {
            flags,
            number_of_wheel_rotation_units,
            x_position,
            y_position,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let wheel_negative_bit = if self.number_of_wheel_rotation_units < 0 {
            PointerFlags::WHEEL_NEGATIVE.bits()
        } else {
            PointerFlags::empty().bits()
        };

        let wheel_rotations_bits = u16::from(self.number_of_wheel_rotation_units as u8); // truncate

        let flags = self.flags.bits() | wheel_negative_bit | wheel_rotations_bits;

        stream.write_u16::<LittleEndian>(flags)?;
        stream.write_u16::<LittleEndian>(self.x_position)?;
        stream.write_u16::<LittleEndian>(self.y_position)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        6
    }
}

bitflags! {
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
