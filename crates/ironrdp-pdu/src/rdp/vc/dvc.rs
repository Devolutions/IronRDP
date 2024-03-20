use std::io;

use bit_field::BitField;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use super::ChannelError;
use crate::cursor::{ReadCursor, WriteCursor};
use crate::{PduParsing, PduResult};

#[cfg(test)]
mod tests;

pub mod display;
pub mod gfx;

mod capabilities;
mod close;
mod create;
mod data;
mod data_first;

pub use self::capabilities::{CapabilitiesRequestPdu, CapabilitiesResponsePdu, CapsVersion};
pub use self::close::ClosePdu;
pub use self::create::{CreateRequestPdu, CreateResponsePdu, DVC_CREATION_STATUS_NO_LISTENER, DVC_CREATION_STATUS_OK};
pub use self::data::DataPdu;
pub use self::data_first::DataFirstPdu;

const HEADER_SIZE: usize = 1;
const PDU_WITH_DATA_MAX_SIZE: usize = 1600;

const UNUSED_U8: u8 = 0;

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum PduType {
    Create = 0x01,
    DataFirst = 0x02,
    Data = 0x03,
    Close = 0x04,
    Capabilities = 0x05,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerPdu {
    CapabilitiesRequest(CapabilitiesRequestPdu),
    CreateRequest(CreateRequestPdu),
    DataFirst(DataFirstPdu),
    Data(DataPdu),
    CloseRequest(ClosePdu),
}

impl ServerPdu {
    pub fn from_buffer(mut stream: impl io::Read, mut dvc_data_size: usize) -> Result<Self, ChannelError> {
        let dvc_header = Header::from_buffer(&mut stream)?;
        let channel_id_type =
            FieldType::from_u8(dvc_header.channel_id_type).ok_or(ChannelError::InvalidDVChannelIdLength)?;

        dvc_data_size -= HEADER_SIZE;

        match dvc_header.pdu_type {
            PduType::Capabilities => Ok(ServerPdu::CapabilitiesRequest(CapabilitiesRequestPdu::from_buffer(
                &mut stream,
            )?)),
            PduType::Create => Ok(ServerPdu::CreateRequest(CreateRequestPdu::from_buffer(
                &mut stream,
                channel_id_type,
                dvc_data_size,
            )?)),
            PduType::DataFirst => {
                let data_length_type =
                    FieldType::from_u8(dvc_header.pdu_dependent).ok_or(ChannelError::InvalidDvcDataLength)?;

                Ok(ServerPdu::DataFirst(DataFirstPdu::from_buffer(
                    &mut stream,
                    channel_id_type,
                    data_length_type,
                    dvc_data_size,
                )?))
            }
            PduType::Data => Ok(ServerPdu::Data(DataPdu::from_buffer(
                &mut stream,
                channel_id_type,
                dvc_data_size,
            )?)),
            PduType::Close => Ok(ServerPdu::CloseRequest(ClosePdu::from_buffer(
                &mut stream,
                channel_id_type,
            )?)),
        }
    }

    pub fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), ChannelError> {
        match self {
            ServerPdu::CapabilitiesRequest(caps_request) => caps_request.to_buffer(&mut stream)?,
            ServerPdu::CreateRequest(create_request) => create_request.to_buffer(&mut stream)?,
            ServerPdu::DataFirst(data_first) => data_first.to_buffer(&mut stream)?,
            ServerPdu::Data(data) => data.to_buffer(&mut stream)?,
            ServerPdu::CloseRequest(close_request) => close_request.to_buffer(&mut stream)?,
        };

        Ok(())
    }

    pub fn buffer_length(&self) -> usize {
        match self {
            ServerPdu::CapabilitiesRequest(caps_request) => caps_request.buffer_length(),
            ServerPdu::CreateRequest(create_request) => create_request.buffer_length(),
            ServerPdu::DataFirst(data_first) => data_first.buffer_length(),
            ServerPdu::Data(data) => data.buffer_length(),
            ServerPdu::CloseRequest(close_request) => close_request.buffer_length(),
        }
    }

    pub fn as_short_name(&self) -> &str {
        match self {
            ServerPdu::CapabilitiesRequest(_) => "Capabilities Request PDU",
            ServerPdu::CreateRequest(_) => "Create Request PDU",
            ServerPdu::DataFirst(_) => "Data First PDU",
            ServerPdu::Data(_) => "Data PDU",
            ServerPdu::CloseRequest(_) => "Close Request PDU",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientPdu {
    CapabilitiesResponse(CapabilitiesResponsePdu),
    CreateResponse(CreateResponsePdu),
    DataFirst(DataFirstPdu),
    Data(DataPdu),
    CloseResponse(ClosePdu),
}

impl ClientPdu {
    pub fn from_buffer(mut stream: impl io::Read, mut dvc_data_size: usize) -> Result<Self, ChannelError> {
        let dvc_header = Header::from_buffer(&mut stream)?;
        let channel_id_type =
            FieldType::from_u8(dvc_header.channel_id_type).ok_or(ChannelError::InvalidDVChannelIdLength)?;

        dvc_data_size -= HEADER_SIZE;

        match dvc_header.pdu_type {
            PduType::Capabilities => Ok(ClientPdu::CapabilitiesResponse(CapabilitiesResponsePdu::from_buffer(
                &mut stream,
            )?)),
            PduType::Create => Ok(ClientPdu::CreateResponse(CreateResponsePdu::from_buffer(
                &mut stream,
                channel_id_type,
            )?)),
            PduType::DataFirst => {
                let data_length_type =
                    FieldType::from_u8(dvc_header.pdu_dependent).ok_or(ChannelError::InvalidDvcDataLength)?;

                Ok(ClientPdu::DataFirst(DataFirstPdu::from_buffer(
                    &mut stream,
                    channel_id_type,
                    data_length_type,
                    dvc_data_size,
                )?))
            }
            PduType::Data => Ok(ClientPdu::Data(DataPdu::from_buffer(
                &mut stream,
                channel_id_type,
                dvc_data_size,
            )?)),
            PduType::Close => Ok(ClientPdu::CloseResponse(ClosePdu::from_buffer(
                &mut stream,
                channel_id_type,
            )?)),
        }
    }

    pub fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), ChannelError> {
        match self {
            ClientPdu::CapabilitiesResponse(caps_request) => caps_request.to_buffer(&mut stream)?,
            ClientPdu::CreateResponse(create_request) => create_request.to_buffer(&mut stream)?,
            ClientPdu::DataFirst(data_first) => data_first.to_buffer(&mut stream)?,
            ClientPdu::Data(data) => data.to_buffer(&mut stream)?,
            ClientPdu::CloseResponse(close_response) => close_response.to_buffer(&mut stream)?,
        };

        Ok(())
    }

    pub fn buffer_length(&self) -> usize {
        match self {
            ClientPdu::CapabilitiesResponse(caps_request) => caps_request.buffer_length(),
            ClientPdu::CreateResponse(create_request) => create_request.buffer_length(),
            ClientPdu::DataFirst(data_first) => data_first.buffer_length(),
            ClientPdu::Data(data) => data.buffer_length(),
            ClientPdu::CloseResponse(close_response) => close_response.buffer_length(),
        }
    }

    pub fn as_short_name(&self) -> &str {
        match self {
            ClientPdu::CapabilitiesResponse(_) => "Capabilities Response PDU",
            ClientPdu::CreateResponse(_) => "Create Response PDU",
            ClientPdu::DataFirst(_) => "Data First PDU",
            ClientPdu::Data(_) => "Data PDU",
            ClientPdu::CloseResponse(_) => "Close Response PDU",
        }
    }
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum FieldType {
    U8 = 0x00,
    U16 = 0x01,
    U32 = 0x02,
}

impl FieldType {
    pub fn read_according_to_type(self, src: &mut ReadCursor<'_>) -> PduResult<u32> {
        ensure_size!(in: src, size: self.size());

        let value = match self {
            FieldType::U8 => u32::from(src.read_u8()),
            FieldType::U16 => u32::from(src.read_u16()),
            FieldType::U32 => src.read_u32(),
        };

        Ok(value)
    }

    pub fn write_according_to_type(self, dst: &mut WriteCursor<'_>, value: u32) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        match self {
            FieldType::U8 => dst.write_u8(value as u8),
            FieldType::U16 => dst.write_u16(value as u16),
            FieldType::U32 => dst.write_u32(value),
        };

        Ok(())
    }

    pub fn size(self) -> usize {
        match self {
            FieldType::U8 => 1,
            FieldType::U16 => 2,
            FieldType::U32 => 4,
        }
    }

    pub fn read_buffer_according_to_type(self, mut stream: impl io::Read) -> Result<u32, io::Error> {
        let value = match self {
            FieldType::U8 => u32::from(stream.read_u8()?),
            FieldType::U16 => u32::from(stream.read_u16::<LittleEndian>()?),
            FieldType::U32 => stream.read_u32::<LittleEndian>()?,
        };

        Ok(value)
    }

    pub fn to_buffer_according_to_type(self, mut stream: impl io::Write, value: u32) -> Result<(), io::Error> {
        match self {
            FieldType::U8 => stream.write_u8(value as u8)?,
            FieldType::U16 => stream.write_u16::<LittleEndian>(value as u16)?,
            FieldType::U32 => stream.write_u32::<LittleEndian>(value)?,
        };

        Ok(())
    }

    pub fn get_type_size(self) -> usize {
        self.size()
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Header {
    channel_id_type: u8, // 2 bit
    pdu_dependent: u8,   // 2 bit
    pdu_type: PduType,   // 4 bit
}

impl PduParsing for Header {
    type Error = ChannelError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let dvc_header = stream.read_u8()?;
        let channel_id_type = dvc_header.get_bits(0..2);
        let pdu_dependent = dvc_header.get_bits(2..4);
        let pdu_type = PduType::from_u8(dvc_header.get_bits(4..8)).ok_or(ChannelError::InvalidDvcPduType)?;

        Ok(Self {
            channel_id_type,
            pdu_dependent,
            pdu_type,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let mut dvc_header: u8 = 0;
        dvc_header.set_bits(0..2, self.channel_id_type);
        dvc_header.set_bits(2..4, self.pdu_dependent);
        dvc_header.set_bits(4..8, self.pdu_type.to_u8().unwrap());
        stream.write_u8(dvc_header)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        HEADER_SIZE
    }
}
