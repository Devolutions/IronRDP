//! RDPUDP_FEC_HEADER: the mandatory header for every v1 datagram.
//!
//! MS-RDPEUDP Section 2.2.2.1.
//! Wire layout: `snSourceAck(4)` + `uReceiveWindowSize(2)` + `uFlags(2)` = 8 bytes, little-endian.

use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor};

use super::v1_flags::V1Flags;

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Common header present on every v1 datagram.
///
/// MS-RDPEUDP Section 2.2.2.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FecHeader {
    /// Highest sequence number the remote endpoint has received.
    /// Set to `0xFFFFFFFF` in the initial SYN.
    pub sn_source_ack: u32,

    /// Size of the receiver's buffer (in packets).
    pub receive_window_size: u16,

    /// Bitmap of `V1Flags` indicating optional payloads and features.
    pub flags: V1Flags,
}

impl FecHeader {
    const FIXED_PART_SIZE: usize = 4 + 2 + 2; // snSourceAck + uReceiveWindowSize + uFlags

    const NAME: &'static str = "RDPUDP_FEC_HEADER";
}

impl Encode for FecHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_fixed_part_size!(in: dst);

        dst.write_u32_be(self.sn_source_ack);
        dst.write_u16_be(self.receive_window_size);
        dst.write_u16_be(self.flags.bits());

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for FecHeader {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_core::ensure_fixed_part_size!(in: src);

        let sn_source_ack = src.read_u32_be();
        let receive_window_size = src.read_u16_be();
        let flags_raw = src.read_u16_be();

        let flags = V1Flags::from_bits_truncate(flags_raw);

        Ok(Self {
            sn_source_ack,
            receive_window_size,
            flags,
        })
    }
}
