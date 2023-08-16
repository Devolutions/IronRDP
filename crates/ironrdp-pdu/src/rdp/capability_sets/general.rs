#[cfg(test)]
mod tests;

use std::{fmt, io};

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::rdp::capability_sets::CapabilitySetsError;
use crate::PduParsing;

const GENERAL_LENGTH: usize = 20;
const PROTOCOL_VER: u16 = 0x0200;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MajorPlatformType(u16);

impl fmt::Debug for MajorPlatformType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match *self {
            Self::UNSPECIFIED => "UNSPECIFIED",
            Self::WINDOWS => "WINDOWS",
            Self::OS2 => "OS2",
            Self::MACINTOSH => "MACINTOSH",
            Self::UNIX => "UNIX",
            Self::IOS => "IOS",
            Self::OSX => "OSX",
            Self::ANDROID => "ANDROID",
            Self::CHROMEOS => "CHROMEOS",
            _ => "UNKNOWN",
        };

        write!(f, "MajorPlatformType(0x{:02X}-{name})", self.0)
    }
}

impl MajorPlatformType {
    pub const UNSPECIFIED: Self = Self(0);
    pub const WINDOWS: Self = Self(1);
    pub const OS2: Self = Self(2);
    pub const MACINTOSH: Self = Self(3);
    pub const UNIX: Self = Self(4);
    pub const IOS: Self = Self(5);
    pub const OSX: Self = Self(6);
    pub const ANDROID: Self = Self(7);
    pub const CHROMEOS: Self = Self(8);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MinorPlatformType(u16);

impl fmt::Debug for MinorPlatformType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match *self {
            Self::UNSPECIFIED => "UNSPECIFIED",
            Self::WINDOWS_31X => "WINDOWS_31X",
            Self::WINDOWS_95 => "WINDOWS_95",
            Self::WINDOWS_NT => "WINDOWS_NT",
            Self::OS2V21 => "OS2_V21",
            Self::POWER_PC => "POWER_PC",
            Self::MACINTOSH => "MACINTOSH",
            Self::NATIVE_XSERVER => "NATIVE_XSERVER",
            Self::PSEUDO_XSERVER => "PSEUDO_XSERVER",
            Self::WINDOWS_RT => "WINDOWS_RT",
            _ => "UNKNOWN",
        };

        write!(f, "MinorPlatformType(0x{:02X}-{name})", self.0)
    }
}

impl MinorPlatformType {
    pub const UNSPECIFIED: Self = Self(0);
    pub const WINDOWS_31X: Self = Self(1);
    pub const WINDOWS_95: Self = Self(2);
    pub const WINDOWS_NT: Self = Self(3);
    pub const OS2V21: Self = Self(4);
    pub const POWER_PC: Self = Self(5);
    pub const MACINTOSH: Self = Self(6);
    pub const NATIVE_XSERVER: Self = Self(7);
    pub const PSEUDO_XSERVER: Self = Self(8);
    pub const WINDOWS_RT: Self = Self(9);
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct GeneralExtraFlags: u16 {
        const FASTPATH_OUTPUT_SUPPORTED = 0x0001;
        const NO_BITMAP_COMPRESSION_HDR = 0x0400;
        const LONG_CREDENTIALS_SUPPORTED = 0x0004;
        const AUTORECONNECT_SUPPORTED = 0x0008;
        const ENC_SALTED_CHECKSUM = 0x0010;
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
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
        let major_platform_type = MajorPlatformType(buffer.read_u16::<LittleEndian>()?);
        let minor_platform_type = MinorPlatformType(buffer.read_u16::<LittleEndian>()?);

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
        buffer.write_u16::<LittleEndian>(self.major_platform_type.0)?;
        buffer.write_u16::<LittleEndian>(self.minor_platform_type.0)?;
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
