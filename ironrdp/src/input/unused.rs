use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::InputEventError;
use crate::PduParsing;

#[derive(Debug, Clone, PartialEq)]
pub struct UnusedPdu;

impl PduParsing for UnusedPdu {
    type Error = InputEventError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let _padding = stream.read_u32::<LittleEndian>()?;
        let _padding = stream.read_u16::<LittleEndian>()?;

        Ok(Self)
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(0)?; // padding
        stream.write_u16::<LittleEndian>(0)?; // padding

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        6
    }
}
