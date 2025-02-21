// TODO: lookup the codec id used by connector instead!
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum CodecId {
    None = 0x0,
    RemoteFx = 0x3,
    #[cfg(feature = "qoi")]
    QOI = 0xA0,
}

impl CodecId {
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            3 => Some(Self::RemoteFx),
            #[cfg(feature = "qoi")]
            0x0A => Some(Self::QOI),
            _ => None,
        }
    }
}
