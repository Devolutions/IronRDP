use crate::{
    cursor::{ReadCursor, WriteCursor},
    PduDecode, PduEncode, PduResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnusedPdu;

impl UnusedPdu {
    const NAME: &'static str = "UnusedPdu";

    const FIXED_PART_SIZE: usize = 6 /* padding */;
}

impl PduEncode for UnusedPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        write_padding!(dst, 6);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for UnusedPdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        read_padding!(src, 6);
        Ok(Self)
    }
}
