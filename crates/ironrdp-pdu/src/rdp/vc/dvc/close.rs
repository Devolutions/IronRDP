use std::io;

use super::{FieldType, Header, PduType, HEADER_SIZE, UNUSED_U8};
use crate::rdp::vc::ChannelError;
use crate::PduParsing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosePdu {
    pub channel_id_type: FieldType,
    pub channel_id: u32,
}

impl ClosePdu {
    pub fn from_buffer(mut stream: impl io::Read, channel_id_type: FieldType) -> Result<Self, ChannelError> {
        let channel_id = channel_id_type.read_buffer_according_to_type(&mut stream)?;

        Ok(Self {
            channel_id_type,
            channel_id,
        })
    }

    pub fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), ChannelError> {
        let dvc_header = Header {
            channel_id_type: self.channel_id_type as u8,
            pdu_dependent: UNUSED_U8,
            pdu_type: PduType::Close,
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
