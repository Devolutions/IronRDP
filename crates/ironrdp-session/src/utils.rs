#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum CodecId {
    RemoteFx = 0x3,
}

impl CodecId {
    pub const fn from_u8(value: u8) -> Option<Self> {
        if value == 0x3 {
            Some(Self::RemoteFx)
        } else {
            None
        }
    }
}
