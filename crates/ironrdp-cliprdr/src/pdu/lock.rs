use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::{cast_int, ensure_fixed_part_size, PduDecode, PduEncode, PduResult};

use crate::pdu::PartialHeader;

/// Represents `CLIPRDR_LOCK_CLIPDATA`/`CLIPRDR_UNLOCK_CLIPDATA`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockDataId(pub u32);

impl LockDataId {
    const NAME: &str = "CLIPRDR_(UN)LOCK_CLIPDATA";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u32>();
}

impl PduEncode for LockDataId {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = PartialHeader::new(cast_int!("dataLen", Self::FIXED_PART_SIZE)?);
        header.encode(dst)?;

        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.0);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        PartialHeader::SIZE + Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for LockDataId {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let _header = PartialHeader::decode(src)?;

        ensure_fixed_part_size!(in: src);
        let id = src.read_u32();

        Ok(Self(id))
    }
}
