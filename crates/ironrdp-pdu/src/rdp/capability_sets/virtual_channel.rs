#[cfg(test)]
mod tests;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::rdp::capability_sets::CapabilitySetsError;
use crate::{try_read_optional, try_write_optional, PduParsing};

const FLAGS_FIELD_SIZE: usize = 4;
const CHUNK_SIZE_FIELD_SIZE: usize = 4;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct VirtualChannelFlags: u32 {
        const NO_COMPRESSION = 0;
        const COMPRESSION_SERVER_TO_CLIENT = 1;
        const COMPRESSION_CLIENT_TO_SERVER_8K = 2;
    }
}

/// The VirtualChannel structure is used to advertise virtual channel support characteristics. This capability is sent by both client and server.
///
/// # Fields
///
/// * `flags` - virtual channel compression flags
/// * `chunk_size` - when sent from server to client, this field contains the maximum allowed size of a virtual channel chunk and MUST be greater than or equal to 1600 and less than or equal to 16256.
/// When sent from client to server, the value in this field is ignored by the server. This value is not verified in IronRDP and MUST be verified on the caller's side
///
/// # MSDN
///
/// * [Virtual Channel Capability Set](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/a8593178-80c0-4b80-876c-cb77e62cecfc)
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct VirtualChannel {
    pub flags: VirtualChannelFlags,
    pub chunk_size: Option<u32>,
}

impl PduParsing for VirtualChannel {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let flags = VirtualChannelFlags::from_bits_truncate(buffer.read_u32::<LittleEndian>()?);

        let mut virtual_channel_pdu = Self {
            flags,
            chunk_size: None,
        };

        virtual_channel_pdu.chunk_size = Some(try_read_optional!(
            buffer.read_u32::<LittleEndian>(),
            virtual_channel_pdu
        ));

        Ok(virtual_channel_pdu)
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(self.flags.bits())?;

        try_write_optional!(self.chunk_size, |value: &u32| {
            buffer.write_u32::<LittleEndian>(*value)
        });

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        FLAGS_FIELD_SIZE + self.chunk_size.map(|_| CHUNK_SIZE_FIELD_SIZE).unwrap_or(0)
    }
}
