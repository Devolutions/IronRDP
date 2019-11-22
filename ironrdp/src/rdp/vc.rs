pub mod dvc;

#[cfg(test)]
mod tests;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;

use crate::{impl_from_error, PduParsing};

pub const DRDYNVC_CHANNEL_NAME: &str = "drdynvc";

const CHANNEL_PDU_HEADER_SIZE: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelPduHeader {
    /// The total length in bytes of the uncompressed channel data, excluding this header
    pub total_length: u32,
    pub flags: ChannelControlFlags,
}

impl PduParsing for ChannelPduHeader {
    type Error = ChannelError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let total_length = stream.read_u32::<LittleEndian>()?;
        let flags = ChannelControlFlags::from_bits_truncate(stream.read_u32::<LittleEndian>()?);

        Ok(Self {
            total_length,
            flags,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(self.total_length)?;
        stream.write_u32::<LittleEndian>(self.flags.bits())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        CHANNEL_PDU_HEADER_SIZE
    }
}

bitflags! {
    pub struct ChannelControlFlags: u32 {
        const FLAG_FIRST = 0x0000_0001;
        const FLAG_LAST = 0x0000_0002;
        const FLAG_SHOW_PROTOCOL = 0x0000_0010;
        const FLAG_SUSPEND = 0x0000_0020;
        const FLAG_RESUME  = 0x0000_0040;
        const FLAG_SHADOW_PERSISTENT = 0x0000_0080;
        const PACKET_COMPRESSED = 0x0020_0000;
        const PACKET_AT_FRONT = 0x0040_0000;
        const PACKET_FLUSHED = 0x0080_0000;
        const COMPRESSION_TYPE_MASK = 0x000F_0000;
    }
}

#[derive(Debug, Fail)]
pub enum ChannelError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "Utf8 error: {}", _0)]
    Utf8Error(#[fail(cause)] std::string::FromUtf8Error),
    #[fail(display = "Invalid channel PDU header")]
    InvalidChannelPduHeader,
    #[fail(display = "Invalid channel total data length")]
    InvalidChannelTotalDataLength,
    #[fail(display = "Invalid DVC PDU type")]
    InvalidDvcPduType,
    #[fail(display = "Invalid DVC id length value")]
    InvalidDVChannelIdLength,
    #[fail(display = "Invalid DVC data length value")]
    InvalidDvcDataLength,
    #[fail(display = "Invalid DVC capabilities version")]
    InvalidDvcCapabilitiesVersion,
    #[fail(display = "Invalid DVC message size")]
    InvalidDvcMessageSize,
    #[fail(display = "Invalid DVC total message size")]
    InvalidDvcTotalMessageSize,
}

impl_from_error!(io::Error, ChannelError, ChannelError::IOError);
impl_from_error!(
    std::string::FromUtf8Error,
    ChannelError,
    ChannelError::Utf8Error
);

impl From<ChannelError> for io::Error {
    fn from(e: ChannelError) -> io::Error {
        io::Error::new(
            io::ErrorKind::Other,
            format!("Virtual channel error: {}", e),
        )
    }
}
