use ironrdp_core::{ReadCursor, WriteCursor};
use ironrdp_pdu::{Decode, DecodeResult, Encode, EncodeResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NowMessageClass(pub u8);

impl NowMessageClass {
    /// NOW-PROTO: NOW_SYSTEM_MSG_CLASS_ID
    pub const SYSTEM: Self = Self(0x11);

    /// NOW-PROTO: NOW_SESSION_MSG_CLASS_ID
    pub const SESSION: Self = Self(0x12);

    /// NOW-PROTO: NOW_EXEC_MSG_CLASS_ID
    pub const EXEC: Self = Self(0x13);
}

/// The NOW_HEADER structure is the header common to all NOW protocol messages.
///
/// NOW-PROTO: NOW_HEADER
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowHeader {
    pub size: u32,
    pub class: NowMessageClass,
    pub kind: u8,
    pub flags: u16,
}

impl NowHeader {
    const NAME: &'static str = "NOW_HEADER";
    pub const FIXED_PART_SIZE: usize = 8;
}

impl Encode for NowHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.size);
        dst.write_u8(self.class.0);
        dst.write_u8(self.kind);
        dst.write_u16(self.flags);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for NowHeader {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let size = src.read_u32();
        let class = NowMessageClass(src.read_u8());
        let kind = src.read_u8();
        let flags = src.read_u16();

        Ok(NowHeader {
            size,
            class,
            kind,
            flags,
        })
    }
}
