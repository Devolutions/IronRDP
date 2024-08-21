use super::CapabilitySet;
use crate::{DecodeResult, EncodeResult, PduDecode, PduEncode};
use ironrdp_core::{ReadCursor, WriteCursor};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitiesAdvertisePdu(pub Vec<CapabilitySet>);

impl CapabilitiesAdvertisePdu {
    const NAME: &'static str = "CapabilitiesAdvertisePdu";

    const FIXED_PART_SIZE: usize  = 2 /* Count */;
}

impl PduEncode for CapabilitiesAdvertisePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(cast_length!("Count", self.0.len())?);

        for capability_set in self.0.iter() {
            capability_set.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.0.iter().map(|c| c.size()).sum::<usize>()
    }
}

impl<'a> PduDecode<'a> for CapabilitiesAdvertisePdu {
    fn decode(src: &mut ReadCursor<'a>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let capabilities_count = cast_length!("Count", src.read_u16())?;

        ensure_size!(in: src, size: capabilities_count * CapabilitySet::FIXED_PART_SIZE);

        let capabilities = (0..capabilities_count)
            .map(|_| CapabilitySet::decode(src))
            .collect::<Result<_, _>>()?;

        Ok(Self(capabilities))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameAcknowledgePdu {
    pub queue_depth: QueueDepth,
    pub frame_id: u32,
    pub total_frames_decoded: u32,
}

impl FrameAcknowledgePdu {
    const NAME: &'static str = "FrameAcknowledgePdu";

    const FIXED_PART_SIZE: usize = 4 /* QueueDepth */ + 4 /* FrameId */ + 4 /* TotalFramesDecoded */;
}

impl PduEncode for FrameAcknowledgePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.queue_depth.to_u32());
        dst.write_u32(self.frame_id);
        dst.write_u32(self.total_frames_decoded);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'a> PduDecode<'a> for FrameAcknowledgePdu {
    fn decode(src: &mut ReadCursor<'a>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let queue_depth = QueueDepth::from_u32(src.read_u32());
        let frame_id = src.read_u32();
        let total_frames_decoded = src.read_u32();

        Ok(Self {
            queue_depth,
            frame_id,
            total_frames_decoded,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheImportReplyPdu {
    pub cache_slots: Vec<u16>,
}

impl CacheImportReplyPdu {
    const NAME: &'static str = "CacheImportReplyPdu";

    const FIXED_PART_SIZE: usize = 2 /* Count */;
}

impl PduEncode for CacheImportReplyPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(cast_length!("Count", self.cache_slots.len())?);

        for cache_slot in self.cache_slots.iter() {
            dst.write_u16(*cache_slot);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.cache_slots.iter().map(|_| 2).sum::<usize>()
    }
}

impl<'a> PduDecode<'a> for CacheImportReplyPdu {
    fn decode(src: &mut ReadCursor<'a>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let entries_count = src.read_u16();

        let cache_slots = (0..entries_count).map(|_| src.read_u16()).collect();

        Ok(Self { cache_slots })
    }
}

#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum QueueDepth {
    Unavailable,
    AvailableBytes(u32),
    Suspend,
}

impl QueueDepth {
    pub fn from_u32(v: u32) -> Self {
        match v {
            0x0000_0000 => Self::Unavailable,
            0x0000_0001..=0xFFFF_FFFE => Self::AvailableBytes(v),
            0xFFFF_FFFF => Self::Suspend,
        }
    }

    pub fn to_u32(self) -> u32 {
        match self {
            Self::Unavailable => 0x0000_0000,
            Self::AvailableBytes(v) => v,
            Self::Suspend => 0xFFFF_FFFF,
        }
    }
}
