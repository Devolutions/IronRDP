#[cfg(test)]
pub mod test;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{impl_from_error, PduParsing};

const REDIRECTION_VERSION_MASK: u32 = 0x0000_003C;

const FLAGS_SIZE: usize = 4;
const REDIRECTED_SESSION_ID_SIZE: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientClusterData {
    pub flags: RedirectionFlags,
    pub redirection_version: RedirectionVersion,
    pub redirected_session_id: u32,
}

impl PduParsing for ClientClusterData {
    type Error = ClusterDataError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let flags_with_version = buffer.read_u32::<LittleEndian>()?;
        let redirected_session_id = buffer.read_u32::<LittleEndian>()?;

        let flags = RedirectionFlags::from_bits(flags_with_version & !REDIRECTION_VERSION_MASK)
            .ok_or(ClusterDataError::InvalidRedirectionFlags)?;
        let redirection_version =
            RedirectionVersion::from_u8(((flags_with_version & REDIRECTION_VERSION_MASK) >> 2) as u8)
                .ok_or(ClusterDataError::InvalidRedirectionFlags)?;

        Ok(Self {
            flags,
            redirection_version,
            redirected_session_id,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        let flags_with_version = self.flags.bits() | (self.redirection_version.to_u32().unwrap() << 2);

        buffer.write_u32::<LittleEndian>(flags_with_version)?;
        buffer.write_u32::<LittleEndian>(self.redirected_session_id)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        FLAGS_SIZE + REDIRECTED_SESSION_ID_SIZE
    }
}

bitflags! {
    pub struct RedirectionFlags: u32 {
        const REDIRECTION_SUPPORTED = 0x0000_0001;
        const REDIRECTED_SESSION_FIELD_VALID = 0x0000_0002;
        const REDIRECTED_SMARTCARD = 0x0000_0040;
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum RedirectionVersion {
    V1 = 0,
    V2 = 1,
    V3 = 2,
    V4 = 3,
    V5 = 4,
    V6 = 5,
}

#[derive(Debug, Fail)]
pub enum ClusterDataError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "Invalid redirection flags field")]
    InvalidRedirectionFlags,
}

impl_from_error!(io::Error, ClusterDataError, ClusterDataError::IOError);
