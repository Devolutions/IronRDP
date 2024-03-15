#[cfg(test)]
mod tests;

use std::io;

use super::{FieldType, Header, PduType, HEADER_SIZE, PDU_WITH_DATA_MAX_SIZE, UNUSED_U8};
use crate::rdp::vc::ChannelError;
use crate::PduParsing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPdu {
    pub channel_id_type: FieldType,
    pub channel_id: u32,
    pub data_size: usize,
}

impl DataPdu {
    pub fn new(channel_id: u32, data_size: usize) -> Self {
        Self {
            channel_id_type: FieldType::U8,
            channel_id,
            data_size,
        }
    }

    pub fn from_buffer(
        mut stream: impl io::Read,
        channel_id_type: FieldType,
        mut data_size: usize,
    ) -> Result<Self, ChannelError> {
        let channel_id = channel_id_type.read_buffer_according_to_type(&mut stream)?;
        data_size -= channel_id_type.get_type_size();

        let expected_max_data_size = PDU_WITH_DATA_MAX_SIZE - (HEADER_SIZE + channel_id_type.get_type_size());

        if data_size > expected_max_data_size {
            Err(ChannelError::InvalidDvcMessageSize)
        } else {
            Ok(Self {
                channel_id_type,
                channel_id,
                data_size,
            })
        }
    }

    pub fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), ChannelError> {
        let dvc_header = Header {
            channel_id_type: self.channel_id_type as u8,
            pdu_dependent: UNUSED_U8,
            pdu_type: PduType::Data,
        };
        dvc_header.to_buffer(&mut stream)?;
        self.channel_id_type
            .to_buffer_according_to_type(&mut stream, self.channel_id)?;

        Ok(())
    }

    pub fn buffer_length(&self) -> usize {
        HEADER_SIZE + self.channel_id_type.get_type_size()
    }
}
