use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::rdp::CapabilitySetsError;
use crate::PduParsing;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct MultifragmentUpdate {
    pub max_request_size: u32,
}

impl PduParsing for MultifragmentUpdate {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let max_request_size = buffer.read_u32::<LittleEndian>()?;

        Ok(Self { max_request_size })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(self.max_request_size)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        4
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const MULTIFRAGMENT_UPDATE_PDU_BUFFER: [u8; 4] = [0xf4, 0xf3, 0xf2, 0xf1];
    const MULTIFRAGMENT_UPDATE_PDU: MultifragmentUpdate = MultifragmentUpdate {
        max_request_size: 0xf1f2_f3f4,
    };

    #[test]
    fn from_buffer_correctly_parses_multifragment_update() {
        assert_eq!(
            MULTIFRAGMENT_UPDATE_PDU,
            MultifragmentUpdate::from_buffer(MULTIFRAGMENT_UPDATE_PDU_BUFFER.as_ref()).unwrap()
        );
    }

    #[test]
    fn to_buffer_correctly_serializes_multifragment_update() {
        let expected = MULTIFRAGMENT_UPDATE_PDU_BUFFER.as_ref();
        let mut buffer = Vec::with_capacity(expected.len());

        MULTIFRAGMENT_UPDATE_PDU.to_buffer(&mut buffer).unwrap();
        assert_eq!(expected, buffer.as_slice());
    }

    #[test]
    fn buffer_length_is_correct_for_multifragment_update() {
        assert_eq!(
            MULTIFRAGMENT_UPDATE_PDU_BUFFER.len(),
            MULTIFRAGMENT_UPDATE_PDU.buffer_length()
        );
    }
}
