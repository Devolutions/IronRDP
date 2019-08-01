#[cfg(test)]
mod test;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::rdp::CapabilitySetsError;
use crate::PduParsing;

const VIRTUAL_CHANNEL_LENGTH: usize = 8;

const CHANNEL_CHUNK_LENGTH: u32 = 1600;
const CHUNK_SIZE_VALID_VALUE: u32 = 16256;

bitflags! {
    pub struct VirtualChannelFlags: u32 {
        const NO_COMPRESSION = 0;
        const COMPRESSION_SERVER_TO_CLIENT = 1;
        const COMPRESSION_CLIENT_TO_SERVER_8K = 2;
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct VirtualChannel {
    flags: VirtualChannelFlags,
    chunk_size: u32,
}

impl PduParsing for VirtualChannel {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let flags = VirtualChannelFlags::from_bits_truncate(buffer.read_u32::<LittleEndian>()?);

        let chunk_size = buffer.read_u32::<LittleEndian>()?;
        if chunk_size > CHUNK_SIZE_VALID_VALUE || chunk_size < CHANNEL_CHUNK_LENGTH {
            return Err(CapabilitySetsError::InvalidChunkSize);
        }

        Ok(VirtualChannel { flags, chunk_size })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(self.flags.bits())?;
        buffer.write_u32::<LittleEndian>(self.chunk_size)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        VIRTUAL_CHANNEL_LENGTH
    }
}
