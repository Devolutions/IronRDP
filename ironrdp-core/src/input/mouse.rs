use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::InputEventError;
use crate::PduParsing;

const WHEEL_ROTATION_MASK: u16 = 0x00FF;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MousePdu {
    pub wheel_events: WheelEvents,
    pub movement_events: MovementEvents,
    pub button_events: ButtonEvents,
    pub number_of_wheel_rotation_units: i16,
    pub x_position: u16,
    pub y_position: u16,
}

impl PduParsing for MousePdu {
    type Error = InputEventError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let pointer_flags = stream.read_u16::<LittleEndian>()?;

        let wheel_events = WheelEvents::from_bits_truncate(pointer_flags);
        let movement_events = MovementEvents::from_bits_truncate(pointer_flags);
        let button_events = ButtonEvents::from_bits_truncate(pointer_flags);

        let mut number_of_wheel_rotation_units = i16::try_from(pointer_flags & WHEEL_ROTATION_MASK).unwrap();
        if wheel_events.contains(WheelEvents::WHEEL_NEGATIVE) {
            number_of_wheel_rotation_units *= -1;
        }

        let x_position = stream.read_u16::<LittleEndian>()?;
        let y_position = stream.read_u16::<LittleEndian>()?;

        Ok(Self {
            wheel_events,
            movement_events,
            button_events,
            number_of_wheel_rotation_units,
            x_position,
            y_position,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let wheel_negative_flag = if self.number_of_wheel_rotation_units < 0 {
            WheelEvents::WHEEL_NEGATIVE
        } else {
            WheelEvents::empty()
        };
        let number_of_wheel_rotation_units = self.number_of_wheel_rotation_units as u16 & !WHEEL_ROTATION_MASK;

        let flags = self.wheel_events.bits()
            | self.movement_events.bits()
            | self.button_events.bits()
            | wheel_negative_flag.bits()
            | number_of_wheel_rotation_units;

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
    pub struct WheelEvents: u16 {
        const HORIZONTAL_WHEEL = 0x0400;
        const VERTICAL_WHEEL = 0x0200;
        const WHEEL_NEGATIVE = 0x0100;
    }
}

bitflags! {
    pub struct MovementEvents: u16 {
        const MOVE = 0x0800;
    }
}

bitflags! {
    pub struct ButtonEvents: u16 {
        const DOWN = 0x8000;
        const LEFT_BUTTON = 0x1000;
        const RIGHT_BUTTON = 0x2000;
        const MIDDLE_BUTTON_OR_WHEEL = 0x4000;
    }
}
