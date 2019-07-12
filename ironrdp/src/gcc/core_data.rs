pub mod client;
pub mod server;

use std::io;

use failure::Fail;
use num_derive::{FromPrimitive, ToPrimitive};

const VERSION_SIZE: usize = 4;

#[derive(Copy, Clone, Debug, PartialEq, FromPrimitive, ToPrimitive)]
pub enum RdpVersion {
    V4 = 0x0008_0001,
    V5Plus = 0x0008_0004,
    V10 = 0x0008_0005,
    V10_1 = 0x0008_0006,
    V10_2 = 0x0008_0007,
    V10_3 = 0x0008_0008,
    V10_4 = 0x0008_0009,
    V10_5 = 0x0008_000A,
    V10_6 = 0x0008_000B,
    V10_7 = 0x0008_000C,
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
