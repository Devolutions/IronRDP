#[cfg(test)]
mod tests;

use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{PduDecode, PduEncode, PduResult};
use ironrdp_core::{ReadCursor, WriteCursor};

const BRUSH_LENGTH: usize = 4;

#[derive(Copy, Clone, Debug, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum SupportLevel {
    Default = 0,
    Color8x8 = 1,
    ColorFull = 2,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Brush {
    pub support_level: SupportLevel,
}

impl Brush {
    const NAME: &'static str = "Brush";

    const FIXED_PART_SIZE: usize = BRUSH_LENGTH;
}

impl PduEncode for Brush {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.support_level.to_u32().unwrap());

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for Brush {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let support_level = SupportLevel::from_u32(src.read_u32())
            .ok_or_else(|| invalid_message_err!("supportLevel", "invalid brush support level"))?;

        Ok(Brush { support_level })
    }
}
