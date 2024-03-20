#[cfg(test)]
mod tests;

use crate::cursor::{ReadCursor, WriteCursor};
use crate::{PduDecode, PduEncode, PduResult};

const OFFSCREEN_BITMAP_CACHE_LENGTH: usize = 8;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct OffscreenBitmapCache {
    pub is_supported: bool,
    pub cache_size: u16,
    pub cache_entries: u16,
}

impl OffscreenBitmapCache {
    const NAME: &'static str = "OffscreenBitmapCache";

    const FIXED_PART_SIZE: usize = OFFSCREEN_BITMAP_CACHE_LENGTH;
}

impl PduEncode for OffscreenBitmapCache {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(u32::from(self.is_supported));
        dst.write_u16(self.cache_size);
        dst.write_u16(self.cache_entries);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for OffscreenBitmapCache {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let is_supported = src.read_u32() != 0;
        let cache_size = src.read_u16();
        let cache_entries = src.read_u16();

        Ok(OffscreenBitmapCache {
            is_supported,
            cache_size,
            cache_entries,
        })
    }
}
