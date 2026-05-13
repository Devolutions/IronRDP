pub(crate) mod client;
pub(crate) mod server;

use core::fmt;
use std::io;

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

#[derive(Debug)]
pub enum CoreDataError {
    IOError(io::Error),
    InvalidVersion,
    InvalidColorDepth,
    InvalidPostBetaColorDepth,
    InvalidHighColorDepth,
    InvalidSupportedColorDepths,
    InvalidSecureAccessSequence,
    InvalidKeyboardType,
    InvalidEarlyCapabilityFlags,
    InvalidConnectionType,
    InvalidServerSecurityProtocol,
    Pdu(PduError),
}

impl fmt::Display for CoreDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IOError(_) => f.write_str("IO error"),
            Self::InvalidVersion => f.write_str("invalid version field"),
            Self::InvalidColorDepth => f.write_str("invalid color depth field"),
            Self::InvalidPostBetaColorDepth => f.write_str("invalid post beta color depth field"),
            Self::InvalidHighColorDepth => f.write_str("invalid high color depth field"),
            Self::InvalidSupportedColorDepths => f.write_str("invalid supported color depths field"),
            Self::InvalidSecureAccessSequence => f.write_str("invalid secure access sequence field"),
            Self::InvalidKeyboardType => f.write_str("invalid keyboard type field"),
            Self::InvalidEarlyCapabilityFlags => f.write_str("invalid early capability flags field"),
            Self::InvalidConnectionType => f.write_str("invalid connection type field"),
            Self::InvalidServerSecurityProtocol => f.write_str("invalid server security protocol field"),
            Self::Pdu(e) => write!(f, "PDU error: {e}"),
        }
    }
}

impl core::error::Error for CoreDataError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::IOError(e) => Some(e),
            Self::InvalidVersion
            | Self::InvalidColorDepth
            | Self::InvalidPostBetaColorDepth
            | Self::InvalidHighColorDepth
            | Self::InvalidSupportedColorDepths
            | Self::InvalidSecureAccessSequence
            | Self::InvalidKeyboardType
            | Self::InvalidEarlyCapabilityFlags
            | Self::InvalidConnectionType
            | Self::InvalidServerSecurityProtocol
            | Self::Pdu(_) => None,
        }
    }
}

impl From<io::Error> for CoreDataError {
    fn from(e: io::Error) -> Self {
        Self::IOError(e)
    }
}

impl From<PduError> for CoreDataError {
    fn from(e: PduError) -> Self {
        Self::Pdu(e)
    }
}
