pub(crate) mod client;
pub(crate) mod server;

use std::io;

use thiserror::Error;

use crate::PduError;

const VERSION_SIZE: usize = 4;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RdpVersion(pub u32);

impl From<u32> for RdpVersion {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<RdpVersion> for u32 {
    fn from(version: RdpVersion) -> Self {
        version.0
    }
}

impl RdpVersion {
    pub const V4: Self = Self(0x0008_0001);
    pub const V5_PLUS: Self = Self(0x0008_0004);
    pub const V10: Self = Self(0x0008_0005);
    pub const V10_1: Self = Self(0x0008_0006);
    pub const V10_2: Self = Self(0x0008_0007);
    pub const V10_3: Self = Self(0x0008_0008);
    pub const V10_4: Self = Self(0x0008_0009);
    pub const V10_5: Self = Self(0x0008_000A);
    pub const V10_6: Self = Self(0x0008_000B);
    pub const V10_7: Self = Self(0x0008_000C);
    pub const V10_8: Self = Self(0x0008_000D);
    pub const V10_9: Self = Self(0x0008_000E);
    pub const V10_10: Self = Self(0x0008_000F);
    pub const V10_11: Self = Self(0x0008_0010);
    pub const V10_12: Self = Self(0x0008_0011);
}

#[derive(Debug, Error)]
pub enum CoreDataError {
    #[error("IO error")]
    IOError(#[from] io::Error),
    #[error("invalid version field")]
    InvalidVersion,
    #[error("invalid color depth field")]
    InvalidColorDepth,
    #[error("invalid post beta color depth field")]
    InvalidPostBetaColorDepth,
    #[error("invalid high color depth field")]
    InvalidHighColorDepth,
    #[error("invalid supported color depths field")]
    InvalidSupportedColorDepths,
    #[error("invalid secure access sequence field")]
    InvalidSecureAccessSequence,
    #[error("invalid keyboard type field")]
    InvalidKeyboardType,
    #[error("invalid early capability flags field")]
    InvalidEarlyCapabilityFlags,
    #[error("invalid connection type field")]
    InvalidConnectionType,
    #[error("invalid server security protocol field")]
    InvalidServerSecurityProtocol,
    #[error("PDU error: {0}")]
    Pdu(PduError),
}

impl From<PduError> for CoreDataError {
    fn from(e: PduError) -> Self {
        Self::Pdu(e)
    }
}
