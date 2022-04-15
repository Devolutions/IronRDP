pub mod client;
pub mod server;

use std::io;

use failure::Fail;

use crate::impl_from_error;

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

#[derive(Debug, Fail)]
pub enum CoreDataError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "Invalid version field")]
    InvalidVersion,
    #[fail(display = "Invalid color depth field")]
    InvalidColorDepth,
    #[fail(display = "Invalid post beta color depth field")]
    InvalidPostBetaColorDepth,
    #[fail(display = "Invalid high color depth field")]
    InvalidHighColorDepth,
    #[fail(display = "Invalid supported color depths field")]
    InvalidSupportedColorDepths,
    #[fail(display = "Invalid secure access sequence field")]
    InvalidSecureAccessSequence,
    #[fail(display = "Invalid keyboard type field")]
    InvalidKeyboardType,
    #[fail(display = "Invalid early capability flags field")]
    InvalidEarlyCapabilityFlags,
    #[fail(display = "Invalid connection type field")]
    InvalidConnectionType,
    #[fail(display = "Invalid server security protocol field")]
    InvalidServerSecurityProtocol,
}

impl_from_error!(io::Error, CoreDataError, CoreDataError::IOError);
