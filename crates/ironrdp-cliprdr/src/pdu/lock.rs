use ironrdp_core::{ReadCursor, WriteCursor};
use ironrdp_pdu::{cast_int, ensure_fixed_part_size, impl_pdu_pod, Decode, DecodeResult, Encode, EncodeResult};

use crate::pdu::PartialHeader;

/// Represents `CLIPRDR_LOCK_CLIPDATA`/`CLIPRDR_UNLOCK_CLIPDATA`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockDataId(pub u32);

impl_pdu_pod!(LockDataId);

impl LockDataId {
    const NAME: &'static str = "CLIPRDR_(UN)LOCK_CLIPDATA";
    const FIXED_PART_SIZE: usize = 4 /* Id */;
}

impl Encode for LockDataId {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
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

impl<'de> Decode<'de> for LockDataId {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let _header = PartialHeader::decode(src)?;

        ensure_fixed_part_size!(in: src);
        let id = src.read_u32();

        Ok(Self(id))
    }
}
