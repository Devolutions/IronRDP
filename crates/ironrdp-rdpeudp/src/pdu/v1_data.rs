//! MS-RDPEUDP 2.2.2.4 `RDPUDP_SOURCE_PAYLOAD_HEADER` and the data payload that
//! follows it in a version 1/2 Source Packet.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor};

/// `RDPUDP_SOURCE_PAYLOAD_HEADER` (2.2.2.4): `snCoded(4)` + `snSourceStart(4)`.
///
/// `snCoded` numbers every datagram put on the wire (a retransmit gets a new
/// one); `snSourceStart` numbers the payload and is what acknowledgements
/// refer to (3.1.1.2).
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePayloadHeader {
    pub sn_coded: u32,
    pub sn_source_start: u32,
}

impl SourcePayloadHeader {
    pub const FIXED_PART_SIZE: usize = 4 + 4;
    const NAME: &'static str = "RDPUDP_SOURCE_PAYLOAD_HEADER";
}

impl Encode for SourcePayloadHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_fixed_part_size!(in: dst);
        dst.write_u32_be(self.sn_coded);
        dst.write_u32_be(self.sn_source_start);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for SourcePayloadHeader {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_core::ensure_fixed_part_size!(in: src);
        let sn_coded = src.read_u32_be();
        let sn_source_start = src.read_u32_be();
        Ok(Self {
            sn_coded,
            sn_source_start,
        })
    }
}

/// A Source Packet's header plus its payload, which runs to the end of the
/// datagram (3.1.5.1.4 step 3).
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceData {
    pub header: SourcePayloadHeader,
    pub payload: Vec<u8>,
}

impl SourceData {
    pub fn size(&self) -> usize {
        SourcePayloadHeader::FIXED_PART_SIZE + self.payload.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_payload_header_roundtrip() {
        // From the MS-RDPEUDP 4.2.1 capture.
        let bytes = [0xec, 0x47, 0x1a, 0xe4, 0xec, 0x47, 0x1a, 0xe4];
        let header: SourcePayloadHeader = ironrdp_core::decode(&bytes).unwrap();
        assert_eq!(header.sn_coded, 0xec47_1ae4);
        assert_eq!(header.sn_source_start, 0xec47_1ae4);
        assert_eq!(ironrdp_core::encode_vec(&header).unwrap(), bytes);
    }
}
