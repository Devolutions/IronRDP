#[cfg(test)]
pub mod test;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use tap::Pipe as _;

use super::{CoreDataError, RdpVersion, VERSION_SIZE};
use crate::{nego, try_read_optional, try_write_optional, PduParsing};

const CLIENT_REQUESTED_PROTOCOL_SIZE: usize = 4;
const EARLY_CAPABILITY_FLAGS_SIZE: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCoreData {
    pub version: RdpVersion,
    pub optional_data: ServerCoreOptionalData,
}

impl PduParsing for ServerCoreData {
    type Error = CoreDataError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let version = buffer.read_u32::<LittleEndian>()?.pipe(RdpVersion);
        let optional_data = ServerCoreOptionalData::from_buffer(&mut buffer)?;

        Ok(Self { version, optional_data })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(self.version.0)?;
        self.optional_data.to_buffer(&mut buffer)
    }

    fn buffer_length(&self) -> usize {
        VERSION_SIZE + self.optional_data.buffer_length()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ServerCoreOptionalData {
    pub client_requested_protocols: Option<nego::SecurityProtocol>,
    pub early_capability_flags: Option<ServerEarlyCapabilityFlags>,
}

impl PduParsing for ServerCoreOptionalData {
    type Error = CoreDataError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let mut optional_data = Self::default();

        optional_data.client_requested_protocols = Some(
            nego::SecurityProtocol::from_bits(try_read_optional!(buffer.read_u32::<LittleEndian>(), optional_data))
                .ok_or(CoreDataError::InvalidServerSecurityProtocol)?,
        );

        optional_data.early_capability_flags = Some(
            ServerEarlyCapabilityFlags::from_bits(try_read_optional!(buffer.read_u32::<LittleEndian>(), optional_data))
                .ok_or(CoreDataError::InvalidEarlyCapabilityFlags)?,
        );

        Ok(optional_data)
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        try_write_optional!(self.client_requested_protocols, |value: &nego::SecurityProtocol| {
            buffer.write_u32::<LittleEndian>(value.bits())
        });

        try_write_optional!(self.early_capability_flags, |value: &ServerEarlyCapabilityFlags| buffer
            .write_u32::<LittleEndian>(value.bits()));

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let mut size = 0;

        if self.client_requested_protocols.is_some() {
            size += CLIENT_REQUESTED_PROTOCOL_SIZE;
        }
        if self.early_capability_flags.is_some() {
            size += EARLY_CAPABILITY_FLAGS_SIZE;
        }

        size
    }
}

bitflags! {
    pub struct ServerEarlyCapabilityFlags: u32 {
        const EDGE_ACTIONS_SUPPORTED_V1 = 0x0000_0001;
        const DYNAMIC_DST_SUPPORTED = 0x0000_0002;
        const EDGE_ACTIONS_SUPPORTED_V2 = 0x0000_0004;
    }
}
