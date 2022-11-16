use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::rdp::CapabilitySetsError;
use crate::PduParsing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameAcknowledge {
    pub max_unacknowledged_frame_count: u32,
}

impl PduParsing for FrameAcknowledge {
    type Error = CapabilitySetsError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let max_unacknowledged_frame_count = stream.read_u32::<LittleEndian>()?;

        Ok(Self {
            max_unacknowledged_frame_count,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(self.max_unacknowledged_frame_count)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        4
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const FRAME_ACKNOWLEDGE_PDU_BUFFER: [u8; 4] = [0xf4, 0xf3, 0xf2, 0xf1];
    const FRAME_ACKNOWLEDGE_PDU: FrameAcknowledge = FrameAcknowledge {
        max_unacknowledged_frame_count: 0xf1f2_f3f4,
    };

    #[test]
    fn from_buffer_correctly_parses_frame_acknowledge() {
        assert_eq!(
            FRAME_ACKNOWLEDGE_PDU,
            FrameAcknowledge::from_buffer(FRAME_ACKNOWLEDGE_PDU_BUFFER.as_ref()).unwrap()
        );
    }

    #[test]
    fn to_buffer_correctly_serializes_frame_acknowledge() {
        let expected = FRAME_ACKNOWLEDGE_PDU_BUFFER.as_ref();
        let mut buffer = Vec::with_capacity(expected.len());

        FRAME_ACKNOWLEDGE_PDU.to_buffer(&mut buffer).unwrap();
        assert_eq!(expected, buffer.as_slice());
    }

    #[test]
    fn buffer_length_is_correct_for_frame_acknowledge() {
        assert_eq!(
            FRAME_ACKNOWLEDGE_PDU_BUFFER.len(),
            FRAME_ACKNOWLEDGE_PDU.buffer_length()
        );
    }
}
