mod dvc;
#[cfg(test)]
mod test;

pub use self::dvc::{
    DvcCapabilitiesRequestPdu, DvcCapabilitiesResponsePdu, DvcCreateRequestPdu,
    DvcCreateResponsePdu, DvcPdu, DynamicVirtualChannelHeader,
};

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;

use crate::{impl_from_error, PduParsing};

const CHANNEL_PDU_HEADER_SIZE: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelPduHeader {
    // The total length in bytes of the uncompressed channel data, excluding this header
    pub total_length: u32,
    pub flags: ChannelControlFlags,
}

impl ChannelPduHeader {
    pub fn new(total_length: u32, flags: ChannelControlFlags) -> Self {
        Self {
            total_length,
            flags,
        }
    }
}

impl PduParsing for ChannelPduHeader {
    type Error = ChannelError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let total_length = stream.read_u32::<LittleEndian>()?;
        let flags = ChannelControlFlags::from_bits(stream.read_u32::<LittleEndian>()?)
            .ok_or(ChannelError::InvalidChannelPduHeader)?;

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

#[derive(Debug, Clone, PartialEq)]
pub enum VirtualChannelData {
    ChannelData(Vec<u8>),
    DynamicVCPdu(DvcPdu),
}

impl VirtualChannelData {
    pub fn as_short_name(&self) -> &str {
        match self {
            VirtualChannelData::ChannelData(_) => "Virtual channel data",
            VirtualChannelData::DynamicVCPdu(_) => "Dynamic virtual channel PDU",
        }
    }
}

impl PduParsing for VirtualChannelData {
    type Error = ChannelError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let mut vc_data = Vec::new();
        stream.read_to_end(&mut vc_data)?;

        Ok(VirtualChannelData::ChannelData(vc_data))
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        match self {
            VirtualChannelData::DynamicVCPdu(dvc_pdu) => {
                match dvc_pdu {
                    DvcPdu::CapabilitiesRequest(pdu) => pdu.to_buffer(&mut stream)?,
                    DvcPdu::CapabilitiesResponse(pdu) => pdu.to_buffer(&mut stream)?,
                    DvcPdu::CreateRequest(pdu) => pdu.to_buffer(&mut stream)?,
                    DvcPdu::CreateResponse(pdu) => pdu.to_buffer(&mut stream)?,
                };
            }
            VirtualChannelData::ChannelData(data) => stream.write_all(data.as_ref())?,
        };

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        match self {
            VirtualChannelData::ChannelData(data) => data.len(),
            VirtualChannelData::DynamicVCPdu(dvc_pdu) => match dvc_pdu {
                DvcPdu::CapabilitiesRequest(pdu) => pdu.buffer_length(),
                DvcPdu::CapabilitiesResponse(pdu) => pdu.buffer_length(),
                DvcPdu::CreateRequest(pdu) => pdu.buffer_length(),
                DvcPdu::CreateResponse(pdu) => pdu.buffer_length(),
            },
        }
    }
}

pub struct VirtualChannelPdu {
    pub vc_header: ChannelPduHeader,
    pub vc_data: VirtualChannelData,
}

impl VirtualChannelPdu {
    pub fn new(vc_header: ChannelPduHeader, vc_data: VirtualChannelData) -> Self {
        Self { vc_header, vc_data }
    }
}

impl PduParsing for VirtualChannelPdu {
    type Error = ChannelError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let vc_header = ChannelPduHeader::from_buffer(&mut stream)?;
        let vc_data = VirtualChannelData::from_buffer(&mut stream)?;

        Ok(Self { vc_header, vc_data })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        self.vc_header.to_buffer(&mut stream)?;
        self.vc_data.to_buffer(&mut stream)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        self.vc_header.buffer_length() + self.vc_data.buffer_length()
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
    #[fail(display = "Invalid сhannel pdu header")]
    InvalidChannelPduHeader,
    #[fail(display = "Invalid dynamic virtual сhannel id size")]
    InvalidDVChannelIdSize,
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
