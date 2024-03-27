#[allow(dead_code)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum CodecId {
    None = 0x0,
    RemoteFx = 0x3,
}
