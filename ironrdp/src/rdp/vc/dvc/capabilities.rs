#[cfg(test)]
mod test;

use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use super::{Header, PduType, HEADER_SIZE, UNUSED_U8};
use crate::{rdp::vc::ChannelError, PduParsing};

const DVC_CAPABILITIES_PAD_SIZE: usize = 1;
const DVC_CAPABILITIES_VERSION_SIZE: usize = 2;
const DVC_CAPABILITIES_CHARGE_SIZE: usize = 2;
const DVC_CAPABILITIES_CHARGE_COUNT: usize = 4;

#[repr(u16)]
#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum CapsVersion {
    V1 = 0x0001,
    V2 = 0x0002,
    V3 = 0x0003,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CapabilitiesRequestPdu {
    V1,
    V2 {
        charges: [u16; DVC_CAPABILITIES_CHARGE_COUNT],
    },
    V3 {
        charges: [u16; DVC_CAPABILITIES_CHARGE_COUNT],
    },
}

impl PduParsing for CapabilitiesRequestPdu {
    type Error = ChannelError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let _pad = stream.read_u8()?;
        let version = CapsVersion::from_u16(stream.read_u16::<LittleEndian>()?)
            .ok_or(ChannelError::InvalidDvcCapabilitiesVersion)?;

        match version {
            CapsVersion::V1 => Ok(Self::V1),
            CapsVersion::V2 => {
                let mut charges = [0; DVC_CAPABILITIES_CHARGE_COUNT];
                stream.read_u16_into::<LittleEndian>(&mut charges)?;
                Ok(Self::V2 { charges })
            }
            CapsVersion::V3 => {
                let mut charges = [0; DVC_CAPABILITIES_CHARGE_COUNT];
                stream.read_u16_into::<LittleEndian>(&mut charges)?;
                Ok(Self::V3 { charges })
            }
        }
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let dvc_header = Header {
            channel_id_type: UNUSED_U8,
            pdu_dependent: UNUSED_U8,
            pdu_type: PduType::Capabilities,
        };
        dvc_header.to_buffer(&mut stream)?;
        stream.write_u8(UNUSED_U8)?;

        match self {
            CapabilitiesRequestPdu::V1 => {
                stream.write_u16::<LittleEndian>(CapsVersion::V1.to_u16().unwrap())?
            }
            CapabilitiesRequestPdu::V2 { charges } => {
                stream.write_u16::<LittleEndian>(CapsVersion::V2.to_u16().unwrap())?;
                for charge in charges.iter() {
                    stream.write_u16::<LittleEndian>(*charge)?;
                }
            }
            CapabilitiesRequestPdu::V3 { charges } => {
                stream.write_u16::<LittleEndian>(CapsVersion::V3.to_u16().unwrap())?;
                for charge in charges.iter() {
                    stream.write_u16::<LittleEndian>(*charge)?;
                }
            }
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let charges_length = match self {
            CapabilitiesRequestPdu::V1 => 0,
            CapabilitiesRequestPdu::V2 { charges } | CapabilitiesRequestPdu::V3 { charges } => {
                charges.len() * DVC_CAPABILITIES_CHARGE_SIZE
            }
        };

        HEADER_SIZE + DVC_CAPABILITIES_PAD_SIZE + DVC_CAPABILITIES_VERSION_SIZE + charges_length
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CapabilitiesResponsePdu {
    pub version: CapsVersion,
}

impl PduParsing for CapabilitiesResponsePdu {
    type Error = ChannelError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let _pad = stream.read_u8()?;
        let version = CapsVersion::from_u16(stream.read_u16::<LittleEndian>()?)
            .ok_or(ChannelError::InvalidDvcCapabilitiesVersion)?;

        Ok(Self { version })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let dvc_header = Header {
            channel_id_type: UNUSED_U8,
            pdu_dependent: UNUSED_U8,
            pdu_type: PduType::Capabilities,
        };
        dvc_header.to_buffer(&mut stream)?;
        stream.write_u8(UNUSED_U8)?;
        stream.write_u16::<LittleEndian>(self.version.to_u16().unwrap())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        HEADER_SIZE + DVC_CAPABILITIES_PAD_SIZE + DVC_CAPABILITIES_VERSION_SIZE
    }
}
