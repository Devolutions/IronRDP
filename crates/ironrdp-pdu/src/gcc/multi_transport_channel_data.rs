use bitflags::bitflags;

use crate::cursor::{ReadCursor, WriteCursor};
use crate::{PduDecode, PduEncode, PduResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiTransportChannelData {
    pub flags: MultiTransportFlags,
}

impl MultiTransportChannelData {
    const NAME: &'static str = "MultiTransportChannelData";

    const FIXED_PART_SIZE: usize = 4 /* flags */;
}

impl PduEncode for MultiTransportChannelData {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.flags.bits());

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for MultiTransportChannelData {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let flags = MultiTransportFlags::from_bits(src.read_u32())
            .ok_or_else(|| invalid_message_err!("flags", "invalid multitransport flags"))?;

        Ok(Self { flags })
    }
}

impl_pdu_parsing!(MultiTransportChannelData);

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct MultiTransportFlags: u32 {
        const TRANSPORT_TYPE_UDP_FECR = 0x01;
        const TRANSPORT_TYPE_UDP_FECL = 0x04;
        const TRANSPORT_TYPE_UDP_PREFERRED = 0x100;
        const SOFT_SYNC_TCP_TO_UDP = 0x200;
    }
}
