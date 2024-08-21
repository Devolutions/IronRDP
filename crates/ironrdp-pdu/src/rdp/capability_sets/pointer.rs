#[cfg(test)]
mod tests;

use crate::{DecodeResult, EncodeResult, PduDecode, PduEncode};
use ironrdp_core::{ReadCursor, WriteCursor};

const POINTER_LENGTH: usize = 6;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Pointer {
    pub color_pointer_cache_size: u16,
    pub pointer_cache_size: u16,
}

impl Pointer {
    const NAME: &'static str = "Pointer";

    const FIXED_PART_SIZE: usize = POINTER_LENGTH;
}

impl PduEncode for Pointer {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(1); // color pointer flag
        dst.write_u16(self.color_pointer_cache_size);
        dst.write_u16(self.pointer_cache_size);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for Pointer {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let _color_pointer_flag = src.read_u16() != 0;
        let color_pointer_cache_size = src.read_u16();
        let pointer_cache_size = src.read_u16();

        Ok(Pointer {
            color_pointer_cache_size,
            pointer_cache_size,
        })
    }
}
