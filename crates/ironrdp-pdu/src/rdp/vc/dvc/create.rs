#[cfg(test)]
mod tests;

use super::{FieldType, Header, PduType, HEADER_SIZE, UNUSED_U8};
use crate::cursor::{ReadCursor, WriteCursor};
use crate::{utils, PduEncode, PduResult};

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
    const NAME: &'static str = "DvcCreateRequestPdu";

    pub fn new(channel_id: u32, channel_name: String) -> Self {
        Self {
            channel_id_type: FieldType::U32,
            channel_id,
            channel_name,
        }
    }

    pub(crate) fn decode(
        src: &mut ReadCursor<'_>,
        channel_id_type: FieldType,
        mut data_size: usize,
    ) -> PduResult<Self> {
        let channel_id = channel_id_type.read_according_to_type(src)?;

        data_size -= channel_id_type.size();
        ensure_size!(in: src, size: data_size);
        let channel_name = utils::decode_string(src.read_slice(data_size), utils::CharacterSet::Ansi, false)?;

        Ok(Self {
            channel_id_type,
            channel_id,
            channel_name,
        })
    }
}

impl PduEncode for CreateRequestPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        let dvc_header = Header {
            channel_id_type: self.channel_id_type as u8,
            pdu_dependent: UNUSED_U8, // because DYNVC_CAPS_VERSION1
            pdu_type: PduType::Create,
        };
        dvc_header.encode(dst)?;
        self.channel_id_type.write_according_to_type(dst, self.channel_id)?;
        dst.write_slice(self.channel_name.as_ref());
        dst.write_u8(0);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        HEADER_SIZE + self.channel_id_type.size() + self.channel_name.len() + "\0".len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateResponsePdu {
    pub channel_id_type: FieldType,
    pub channel_id: u32,
    pub creation_status: u32,
}

impl CreateResponsePdu {
    const NAME: &'static str = "DvcCreateResponsePdu";

    pub(crate) fn decode(src: &mut ReadCursor<'_>, channel_id_type: FieldType) -> PduResult<Self> {
        let channel_id = channel_id_type.read_according_to_type(src)?;

        ensure_size!(in: src, size: 4);
        let creation_status = src.read_u32();

        Ok(Self {
            channel_id_type,
            channel_id,
            creation_status,
        })
    }
}

impl PduEncode for CreateResponsePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        let dvc_header = Header {
            channel_id_type: self.channel_id_type as u8,
            pdu_dependent: UNUSED_U8,
            pdu_type: PduType::Create,
        };
        dvc_header.encode(dst)?;
        self.channel_id_type.write_according_to_type(dst, self.channel_id)?;
        dst.write_u32(self.creation_status);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        HEADER_SIZE + self.channel_id_type.size() + DVC_CREATION_STATUS_SIZE
    }
}
