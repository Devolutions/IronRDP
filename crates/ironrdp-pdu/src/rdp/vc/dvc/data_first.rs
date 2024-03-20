use std::io;

use super::{FieldType, Header, PduType, HEADER_SIZE, PDU_WITH_DATA_MAX_SIZE};
use crate::rdp::vc::ChannelError;
use crate::PduParsing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataFirstPdu {
    pub channel_id_type: FieldType,
    pub channel_id: u32,
    pub total_data_size_type: FieldType,
    pub total_data_size: u32,
    pub data_size: usize,
}

impl DataFirstPdu {
    pub fn new(channel_id: u32, total_data_size: u32, data_size: usize) -> Self {
        Self {
            channel_id_type: FieldType::U32,
            channel_id,
            total_data_size_type: FieldType::U32,
            total_data_size,
            data_size,
        }
    }

    pub fn from_buffer(
        mut stream: impl io::Read,
        channel_id_type: FieldType,
        total_data_size_type: FieldType,
        mut data_size: usize,
    ) -> Result<Self, ChannelError> {
        let channel_id = channel_id_type.read_buffer_according_to_type(&mut stream)?;
        let total_data_size = total_data_size_type.read_buffer_according_to_type(&mut stream)?;

        data_size -= channel_id_type.get_type_size() + total_data_size_type.get_type_size();
        if data_size > total_data_size as usize {
            return Err(ChannelError::InvalidDvcTotalMessageSize {
                actual: data_size,
                expected: total_data_size as usize,
            });
        }

        let expected_max_data_size = PDU_WITH_DATA_MAX_SIZE
            - (HEADER_SIZE + channel_id_type.get_type_size() + total_data_size_type.get_type_size());

        if data_size > expected_max_data_size {
            Err(ChannelError::InvalidDvcMessageSize)
        } else {
            Ok(Self {
                channel_id_type,
                channel_id,
                total_data_size_type,
                total_data_size,
                data_size,
            })
        }
    }

    pub fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), ChannelError> {
        let dvc_header = Header {
            channel_id_type: self.channel_id_type as u8,
            pdu_dependent: self.total_data_size_type as u8,
            pdu_type: PduType::DataFirst,
        };
        dvc_header.to_buffer(&mut stream)?;
        self.channel_id_type
            .to_buffer_according_to_type(&mut stream, self.channel_id)?;
        self.total_data_size_type
            .to_buffer_according_to_type(&mut stream, self.total_data_size)?;

        Ok(())
    }

    pub fn buffer_length(&self) -> usize {
        HEADER_SIZE + self.channel_id_type.get_type_size() + self.total_data_size_type.get_type_size()
    }
}
