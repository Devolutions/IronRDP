#[cfg(test)]
mod tests;

use super::{FieldType, Header, PduType, HEADER_SIZE, PDU_WITH_DATA_MAX_SIZE};
use crate::cursor::{ReadCursor, WriteCursor};
use crate::{PduEncode, PduResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataFirstPdu {
    pub channel_id_type: FieldType,
    pub channel_id: u32,
    pub total_data_size_type: FieldType,
    pub total_data_size: u32,
    pub data_size: usize,
}

impl DataFirstPdu {
    const NAME: &'static str = "DvcDataFirstPdu";

    pub fn new(channel_id: u32, total_data_size: u32, data_size: usize) -> Self {
        Self {
            channel_id_type: FieldType::U32,
            channel_id,
            total_data_size_type: FieldType::U32,
            total_data_size,
            data_size,
        }
    }
}

impl PduEncode for DataFirstPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let dvc_header = Header {
            channel_id_type: self.channel_id_type as u8,
            pdu_dependent: self.total_data_size_type as u8,
            pdu_type: PduType::DataFirst,
        };
        dvc_header.encode(dst)?;
        self.channel_id_type.write_according_to_type(dst, self.channel_id)?;
        self.total_data_size_type
            .write_according_to_type(dst, self.total_data_size)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        HEADER_SIZE + self.channel_id_type.size() + self.total_data_size_type.size()
    }
}

impl DataFirstPdu {
    pub(crate) fn decode(
        src: &mut ReadCursor<'_>,
        channel_id_type: FieldType,
        total_data_size_type: FieldType,
        mut data_size: usize,
    ) -> PduResult<Self> {
        let channel_id = channel_id_type.read_according_to_type(src)?;
        let total_data_size = total_data_size_type.read_according_to_type(src)?;

        data_size -= channel_id_type.size() + total_data_size_type.size();
        if data_size > total_data_size as usize {
            return Err(not_enough_bytes_err!(total_data_size as usize, data_size));
        }

        let expected_max_data_size =
            PDU_WITH_DATA_MAX_SIZE - (HEADER_SIZE + channel_id_type.size() + total_data_size_type.size());

        if data_size > expected_max_data_size {
            Err(invalid_message_err!("DvcDataFirst", "invalid message size"))
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
}
