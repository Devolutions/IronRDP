use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use thiserror::Error;

use crate::PduParsing;

const FLAGS_SIZE: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiTransportChannelData {
    pub flags: MultiTransportFlags,
}

impl PduParsing for MultiTransportChannelData {
    type Error = MultiTransportChannelDataError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let flags = MultiTransportFlags::from_bits(buffer.read_u32::<LittleEndian>()?)
            .ok_or(MultiTransportChannelDataError::InvalidMultiTransportFlags)?;

        Ok(Self { flags })
    }
    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(self.flags.bits())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        FLAGS_SIZE
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct MultiTransportFlags: u32 {
        const TRANSPORT_TYPE_UDP_FECR = 0x01;
        const TRANSPORT_TYPE_UDP_FECL = 0x04;
        const TRANSPORT_TYPE_UDP_PREFERRED = 0x100;
        const SOFT_SYNC_TCP_TO_UDP = 0x200;
    }
}

#[derive(Debug, Error)]
pub enum MultiTransportChannelDataError {
    #[error("IO error")]
    IOError(#[from] io::Error),
    #[error("invalid flags field")]
    InvalidMultiTransportFlags,
}
