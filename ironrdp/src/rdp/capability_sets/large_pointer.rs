use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::rdp::CapabilitySetsError;
use crate::PduParsing;

#[derive(Debug, PartialEq, Clone)]
pub struct LargePointer {
    pub flags: LargePointerSupportFlags,
}

impl PduParsing for LargePointer {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let flags = LargePointerSupportFlags::from_bits_truncate(buffer.read_u16::<LittleEndian>()?);

        Ok(Self { flags })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(self.flags.bits())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        2
    }
}

bitflags! {
    pub struct LargePointerSupportFlags: u16 {
        const UP_TO_96X96_PIXELS = 1;
        const UP_TO_384X384_PIXELS = 2;
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const LARGE_POINTER_PDU_BUFFER: [u8; 2] = [0x01, 0x00];
    const LARGE_POINTER_PDU: LargePointer = LargePointer {
        flags: LargePointerSupportFlags::UP_TO_96X96_PIXELS,
    };

    #[test]
    fn from_buffer_correctly_parses_large_pointer() {
        assert_eq!(
            LARGE_POINTER_PDU,
            LargePointer::from_buffer(LARGE_POINTER_PDU_BUFFER.as_ref()).unwrap()
        );
    }

    #[test]
    fn to_buffer_correctly_serializes_large_pointer() {
        let expected = LARGE_POINTER_PDU_BUFFER.as_ref();
        let mut buffer = Vec::with_capacity(expected.len());

        LARGE_POINTER_PDU.to_buffer(&mut buffer).unwrap();
        assert_eq!(expected, buffer.as_slice());
    }

    #[test]
    fn buffer_length_is_correct_for_large_pointer() {
        assert_eq!(LARGE_POINTER_PDU_BUFFER.len(), LARGE_POINTER_PDU.buffer_length());
    }
}
