#[cfg(test)]
mod tests;

use std::io;

use super::{FieldType, Header, PduType, HEADER_SIZE, PDU_WITH_DATA_MAX_SIZE};
use crate::{rdp::vc::ChannelError, PduParsing};

#[derive(Debug, Clone, PartialEq)]
pub struct DataFirstPdu {
    pub channel_id_type: FieldType,
    pub channel_id: u32,
    pub data_length_type: FieldType,
    pub data_length: u32,
    pub dvc_data: Vec<u8>,
}

impl DataFirstPdu {
    pub fn from_buffer(
        mut stream: impl io::Read,
        channel_id_type: FieldType,
        data_length_type: FieldType,
    ) -> Result<Self, ChannelError> {
        let channel_id = channel_id_type.read_buffer_according_to_type(&mut stream)?;
        let data_length = data_length_type.read_buffer_according_to_type(&mut stream)?;
        let mut dvc_data = Vec::new();
        stream.read_to_end(&mut dvc_data)?;

        if dvc_data.len() >= data_length as usize {
            return Err(ChannelError::InvalidDvcTotalMessageSize);
        }

        let expected_max_data_size = PDU_WITH_DATA_MAX_SIZE
            - (HEADER_SIZE + channel_id_type.get_type_size() + data_length_type.get_type_size());

        if dvc_data.len() > expected_max_data_size {
            Err(ChannelError::InvalidDvcMessageSize)
        } else {
            Ok(Self {
                channel_id_type,
                channel_id,
                data_length_type,
                data_length,
                dvc_data,
            })
        }
    }

    pub fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), ChannelError> {
        let dvc_header = Header {
            channel_id_type: self.channel_id_type as u8,
            pdu_dependent: self.data_length_type as u8,
            pdu_type: PduType::DataFirst,
        };
        dvc_header.to_buffer(&mut stream)?;
        self.channel_id_type
            .to_buffer_according_to_type(&mut stream, self.channel_id)?;
        self.data_length_type
            .to_buffer_according_to_type(&mut stream, self.data_length)?;
        stream.write_all(self.dvc_data.as_ref())?;

        Ok(())
    }

    pub fn buffer_length(&self) -> usize {
        HEADER_SIZE
            + self.channel_id_type.get_type_size()
            + self.data_length_type.get_type_size()
            + self.dvc_data.len()
    }
}
