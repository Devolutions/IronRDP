//! V2 data payloads.
//!
//! `DataHeader Payload` (MS-RDPEUDP2 Section 2.2.1.2.5) and
//! `DataBody Payload` (MS-RDPEUDP2 Section 2.2.1.2.7).
//!
//! Both are present when the DATA flag (0x004) is set.
//! DataHeader always precedes DataBody in the packet layout.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor};

// ── DataHeader Payload ──

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Header portion of a data payload, carrying the data sequence number.
///
/// MS-RDPEUDP2 Section 2.2.1.2.5.
/// Wire layout: `DataSeqNum(2)` = 2 bytes.
///
/// `DataSeqNum` is the lower 16 bits of the coded sequence number.
/// This value changes on retransmit: a retransmitted packet gets
/// a new DataSeqNum but preserves its ChannelSeqNum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataHeader {
    /// Lower 16 bits of the data sequence number.
    pub data_seq_num: u16,
}

impl DataHeader {
    const FIXED_PART_SIZE: usize = 2;
    const NAME: &'static str = "RDP-UDP2 DataHeader Payload";
}

impl Encode for DataHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_fixed_part_size!(in: dst);
        dst.write_u16(self.data_seq_num);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for DataHeader {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_core::ensure_fixed_part_size!(in: src);
        let data_seq_num = src.read_u16();
        Ok(Self { data_seq_num })
    }
}

// ── DataBody Payload ──

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Application data payload, containing the channel sequence number
/// and the actual RDP data from higher layers.
///
/// MS-RDPEUDP2 Section 2.2.1.2.7.
/// Wire layout: `ChannelSeqNum(2)` + `Data(variable)`.
///
/// `ChannelSeqNum` is preserved across retransmits: the reliability
/// controller uses it to match a retransmitted packet to the original.
/// This is the sequence number that higher layers care about for
/// ordering and reassembly. See MS-RDPEUDP2 Section 3.1.1.2.4.
///
/// The `Data` field contains the RDP protocol data from upper layers
/// and extends to the end of the packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataBody {
    /// Lower 16 bits of the channel sequence number.
    /// Stable across retransmits.
    pub channel_seq_num: u16,

    /// Application data from higher RDP layers.
    pub data: Vec<u8>,
}

impl DataBody {
    const FIXED_PART_SIZE: usize = 2; // ChannelSeqNum
    const NAME: &'static str = "RDP-UDP2 DataBody Payload";
}

impl Encode for DataBody {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_size!(in: dst, size: self.size());
        dst.write_u16(self.channel_seq_num);
        dst.write_slice(&self.data);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.data.len()
    }
}

impl<'de> Decode<'de> for DataBody {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ironrdp_core::ensure_fixed_part_size!(in: src);
        let channel_seq_num = src.read_u16();
        let data = src.read_remaining().to_vec();
        Ok(Self { channel_seq_num, data })
    }
}
