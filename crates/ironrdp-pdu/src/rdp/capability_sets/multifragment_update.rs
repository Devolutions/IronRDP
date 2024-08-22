use crate::{Decode, DecodeResult, Encode, EncodeResult};
use ironrdp_core::{ensure_fixed_part_size, ReadCursor, WriteCursor};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct MultifragmentUpdate {
    pub max_request_size: u32,
}

impl MultifragmentUpdate {
    const NAME: &'static str = "MultifragmentUpdate";

    const FIXED_PART_SIZE: usize = 4;
}

impl Encode for MultifragmentUpdate {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.max_request_size);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for MultifragmentUpdate {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let max_request_size = src.read_u32();

        Ok(Self { max_request_size })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{decode, encode_vec};

    const MULTIFRAGMENT_UPDATE_PDU_BUFFER: [u8; 4] = [0xf4, 0xf3, 0xf2, 0xf1];
    const MULTIFRAGMENT_UPDATE_PDU: MultifragmentUpdate = MultifragmentUpdate {
        max_request_size: 0xf1f2_f3f4,
    };

    #[test]
    fn from_buffer_correctly_parses_multifragment_update() {
        assert_eq!(
            MULTIFRAGMENT_UPDATE_PDU,
            decode(MULTIFRAGMENT_UPDATE_PDU_BUFFER.as_ref()).unwrap()
        );
    }

    #[test]
    fn to_buffer_correctly_serializes_multifragment_update() {
        let expected = MULTIFRAGMENT_UPDATE_PDU_BUFFER.as_ref();

        let buffer = encode_vec(&MULTIFRAGMENT_UPDATE_PDU).unwrap();
        assert_eq!(expected, buffer.as_slice());
    }

    #[test]
    fn buffer_length_is_correct_for_multifragment_update() {
        assert_eq!(MULTIFRAGMENT_UPDATE_PDU_BUFFER.len(), MULTIFRAGMENT_UPDATE_PDU.size());
    }
}
