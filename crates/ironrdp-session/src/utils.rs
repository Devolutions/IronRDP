#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum CodecId {
    None = 0x0,
    RemoteFx = 0x3,
}

impl CodecId {
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            3 => Some(Self::RemoteFx),
            _ => None,
        }
    }
}
