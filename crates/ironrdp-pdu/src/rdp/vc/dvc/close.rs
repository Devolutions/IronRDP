#[cfg(test)]
mod tests;

use super::{FieldType, Header, PduType, HEADER_SIZE, UNUSED_U8};
use crate::cursor::{ReadCursor, WriteCursor};
use crate::{PduEncode, PduResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosePdu {
    pub channel_id_type: FieldType,
    pub channel_id: u32,
}

impl ClosePdu {
    const NAME: &'static str = "DvcClosePdu";

    pub(crate) fn decode(src: &mut ReadCursor<'_>, channel_id_type: FieldType) -> PduResult<Self> {
        let channel_id = channel_id_type.read_according_to_type(src)?;

        Ok(Self {
            channel_id_type,
            channel_id,
        })
    }
}

impl PduEncode for ClosePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        let dvc_header = Header {
            channel_id_type: self.channel_id_type as u8,
            pdu_dependent: UNUSED_U8,
            pdu_type: PduType::Close,
        };
        dvc_header.encode(dst)?;
        self.channel_id_type.write_according_to_type(dst, self.channel_id)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        HEADER_SIZE + self.channel_id_type.size()
    }
}
