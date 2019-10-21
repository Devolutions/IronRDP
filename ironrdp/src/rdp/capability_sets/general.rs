#[cfg(test)]
mod test;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{rdp::CapabilitySetsError, PduParsing};

const GENERAL_LENGTH: usize = 20;
const PROTOCOL_VER: u16 = 0x0200;

#[derive(Copy, Clone, Debug, PartialEq, FromPrimitive, ToPrimitive)]
pub enum MajorPlatformType {
    Unspecified = 0,
    Windows = 1,
    Os2 = 2,
    Macintosh = 3,
    Unix = 4,
    IOs = 5,
    OsX = 6,
    Android = 7,
    ChromeOs = 8,
}

#[derive(Copy, Clone, Debug, PartialEq, FromPrimitive, ToPrimitive)]
pub enum MinorPlatformType {
    Unspecified = 0,
    Windows31X = 1,
    Windows95 = 2,
    WindowsNT = 3,
    Os2V21 = 4,
    PowerPc = 5,
    Macintosh = 6,
    NativeXServer = 7,
    PseudeXServer = 8,
    WindowsRt = 9,
}

bitflags! {
    pub struct GeneralExtraFlags: u16 {
        const FASTPATH_OUTPUT_SUPPORTED = 0x0001;
        const NO_BITMAP_COMPRESSION_HDR = 0x0400;
        const LONG_CREDENTIALS_SUPPORTED = 0x0004;
        const AUTORECONNECT_SUPPORTED = 0x0008;
        const ENC_SALTED_CHECKSUM = 0x0010;
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct General {
    pub major_platform_type: MajorPlatformType,
    pub minor_platform_type: MinorPlatformType,
    pub extra_flags: GeneralExtraFlags,
    pub refresh_rect_support: bool,
    pub suppress_output_support: bool,
}

impl PduParsing for General {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let major_platform_type = MajorPlatformType::from_u16(buffer.read_u16::<LittleEndian>()?)
            .ok_or(CapabilitySetsError::InvalidMajorPlatformType)?;
        let minor_platform_type = MinorPlatformType::from_u16(buffer.read_u16::<LittleEndian>()?)
            .ok_or(CapabilitySetsError::InvalidMinorPlatformType)?;

        let protocol_ver = buffer.read_u16::<LittleEndian>()?;
        if protocol_ver != PROTOCOL_VER {
            return Err(CapabilitySetsError::InvalidProtocolVersion);
        }

        let _padding = buffer.read_u16::<LittleEndian>()?;

        let compression_types = buffer.read_u16::<LittleEndian>()?;
        if compression_types != 0 {
            return Err(CapabilitySetsError::InvalidCompressionTypes);
        }

        let extra_flags = GeneralExtraFlags::from_bits_truncate(buffer.read_u16::<LittleEndian>()?);

        let update_cap_flags = buffer.read_u16::<LittleEndian>()?;
        if update_cap_flags != 0 {
            return Err(CapabilitySetsError::InvalidUpdateCapFlag);
        }

        let remote_unshare_flag = buffer.read_u16::<LittleEndian>()?;
        if remote_unshare_flag != 0 {
            return Err(CapabilitySetsError::InvalidRemoteUnshareFlag);
        }

        let compression_level = buffer.read_u16::<LittleEndian>()?;
        if compression_level != 0 {
            return Err(CapabilitySetsError::InvalidCompressionLevel);
        }

        let refresh_rect_support = buffer.read_u8()? != 0;
        let suppress_output_support = buffer.read_u8()? != 0;

        Ok(General {
            major_platform_type,
            minor_platform_type,
            extra_flags,
            refresh_rect_support,
            suppress_output_support,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(self.major_platform_type.to_u16().unwrap())?;
        buffer.write_u16::<LittleEndian>(self.minor_platform_type.to_u16().unwrap())?;
        buffer.write_u16::<LittleEndian>(PROTOCOL_VER)?;
        buffer.write_u16::<LittleEndian>(0)?; // padding
        buffer.write_u16::<LittleEndian>(0)?; // generalCompressionTypes
        buffer.write_u16::<LittleEndian>(self.extra_flags.bits())?;
        buffer.write_u16::<LittleEndian>(0)?; // updateCapabilityFlag
        buffer.write_u16::<LittleEndian>(0)?; // remoteUnshareFlag
        buffer.write_u16::<LittleEndian>(0)?; // generalCompressionLevel
        buffer.write_u8(u8::from(self.refresh_rect_support))?;
        buffer.write_u8(u8::from(self.suppress_output_support))?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        GENERAL_LENGTH
    }
}
