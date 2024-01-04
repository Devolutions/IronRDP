#[cfg(test)]
mod tests;

use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::{FieldType, Header, PduType, HEADER_SIZE, UNUSED_U8};
use crate::rdp::vc::ChannelError;
use crate::{utils, PduParsing};

pub const DVC_CREATION_STATUS_OK: u32 = 0x0000_0000;
pub const DVC_CREATION_STATUS_NO_LISTENER: u32 = 0xC000_0001;

const DVC_CREATION_STATUS_SIZE: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRequestPdu {
    pub channel_id_type: FieldType,
    pub channel_id: u32,
    pub channel_name: String,
}

impl CreateRequestPdu {
    pub fn new(channel_id: u32, channel_name: String) -> Self {
        Self {
            channel_id_type: FieldType::U32,
            channel_id,
            channel_name,
        }
    }

    pub fn from_buffer(
        mut stream: impl io::Read,
        channel_id_type: FieldType,
        mut data_size: usize,
    ) -> Result<Self, ChannelError> {
        let channel_id = channel_id_type.read_buffer_according_to_type(&mut stream)?;

        data_size -= channel_id_type.get_type_size();
        let channel_name = utils::read_string_from_stream(&mut stream, data_size, utils::CharacterSet::Ansi, false)?;

        Ok(Self {
            channel_id_type,
            channel_id,
            channel_name,
        })
    }

    pub fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), ChannelError> {
        let dvc_header = Header {
            channel_id_type: self.channel_id_type as u8,
            pdu_dependent: UNUSED_U8, // because DYNVC_CAPS_VERSION1
            pdu_type: PduType::Create,
        };
        dvc_header.to_buffer(&mut stream)?;
        self.channel_id_type
            .to_buffer_according_to_type(&mut stream, self.channel_id)?;
        stream.write_all(self.channel_name.as_ref())?;
        stream.write_all(b"\0")?;

        Ok(())
    }

    pub fn buffer_length(&self) -> usize {
        HEADER_SIZE + self.channel_id_type.get_type_size() + self.channel_name.len() + "\0".len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateResponsePdu {
    pub channel_id_type: FieldType,
    pub channel_id: u32,
    pub creation_status: u32,
}

impl CreateResponsePdu {
    pub fn from_buffer(mut stream: impl io::Read, channel_id_type: FieldType) -> Result<Self, ChannelError> {
        let channel_id = channel_id_type.read_buffer_according_to_type(&mut stream)?;
        let creation_status = stream.read_u32::<LittleEndian>()?;

        Ok(Self {
            channel_id_type,
            channel_id,
            creation_status,
        })
    }

    pub fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), ChannelError> {
        let dvc_header = Header {
            channel_id_type: self.channel_id_type as u8,
            pdu_dependent: UNUSED_U8,
            pdu_type: PduType::Create,
        };
        dvc_header.to_buffer(&mut stream)?;
        self.channel_id_type
            .to_buffer_according_to_type(&mut stream, self.channel_id)?;
        stream.write_u32::<LittleEndian>(self.creation_status)?;

        Ok(())
    }

    pub fn buffer_length(&self) -> usize {
        HEADER_SIZE + self.channel_id_type.get_type_size() + DVC_CREATION_STATUS_SIZE
    }
}
