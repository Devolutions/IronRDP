pub mod dvc;

#[cfg(test)]
mod tests;

use core::fmt;
use std::{io, str};

use bitflags::bitflags;
use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size};

use crate::PduError;

const CHANNEL_PDU_HEADER_SIZE: usize = 8;

/// Channel PDU Header (CHANNEL_PDU_HEADER)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelPduHeader {
    /// The total length in bytes of the uncompressed channel data, excluding this header
    ///
    /// The data can span multiple Virtual Channel PDUs and the individual chunks will need to be
    /// reassembled in that case (section 3.1.5.2.2 of MS-RDPBCGR).
    pub length: u32,
    pub flags: ChannelControlFlags,
}

impl ChannelPduHeader {
    const NAME: &'static str = "ChannelPduHeader";

    const FIXED_PART_SIZE: usize = CHANNEL_PDU_HEADER_SIZE;
}

impl Encode for ChannelPduHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.length);
        dst.write_u32(self.flags.bits());
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for ChannelPduHeader {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let total_length = src.read_u32();
        let flags = ChannelControlFlags::from_bits_retain(src.read_u32());
        Ok(Self {
            length: total_length,
            flags,
        })
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ChannelControlFlags: u32 {
        const FLAG_FIRST = 0x0000_0001;
        const FLAG_LAST = 0x0000_0002;
        const FLAG_SHOW_PROTOCOL = 0x0000_0010;
        const FLAG_SUSPEND = 0x0000_0020;
        const FLAG_RESUME  = 0x0000_0040;
        const FLAG_SHADOW_PERSISTENT = 0x0000_0080;
        const PACKET_COMPRESSED = 0x0020_0000;
        const PACKET_AT_FRONT = 0x0040_0000;
        const PACKET_FLUSHED = 0x0080_0000;
        const COMPRESSION_TYPE_MASK = 0x000F_0000;

        const _ = !0;
    }
}

#[derive(Debug)]
pub enum ChannelError {
    IOError(io::Error),
    FromUtf8Error(std::string::FromUtf8Error),
    InvalidChannelPduHeader,
    InvalidChannelTotalDataLength,
    InvalidDvcPduType,
    InvalidDVChannelIdLength,
    InvalidDvcDataLength,
    InvalidDvcCapabilitiesVersion,
    InvalidDvcMessageSize,
    InvalidDvcTotalMessageSize { actual: usize, expected: usize },
    Pdu(PduError),
}

impl fmt::Display for ChannelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IOError(_) => f.write_str("IO error"),
            Self::FromUtf8Error(_) => f.write_str("from UTF-8 error"),
            Self::InvalidChannelPduHeader => f.write_str("invalid channel PDU header"),
            Self::InvalidChannelTotalDataLength => f.write_str("invalid channel total data length"),
            Self::InvalidDvcPduType => f.write_str("invalid DVC PDU type"),
            Self::InvalidDVChannelIdLength => f.write_str("invalid DVC id length value"),
            Self::InvalidDvcDataLength => f.write_str("invalid DVC data length value"),
            Self::InvalidDvcCapabilitiesVersion => f.write_str("invalid DVC capabilities version"),
            Self::InvalidDvcMessageSize => f.write_str("invalid DVC message size"),
            Self::InvalidDvcTotalMessageSize { actual, expected } => {
                write!(
                    f,
                    "invalid DVC total message size: actual ({actual}) > expected ({expected})"
                )
            }
            Self::Pdu(e) => write!(f, "PDU error: {e}"),
        }
    }
}

impl core::error::Error for ChannelError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::IOError(e) => Some(e),
            Self::FromUtf8Error(e) => Some(e),
            Self::InvalidChannelPduHeader
            | Self::InvalidChannelTotalDataLength
            | Self::InvalidDvcPduType
            | Self::InvalidDVChannelIdLength
            | Self::InvalidDvcDataLength
            | Self::InvalidDvcCapabilitiesVersion
            | Self::InvalidDvcMessageSize
            | Self::InvalidDvcTotalMessageSize { .. }
            | Self::Pdu(_) => None,
        }
    }
}

impl From<io::Error> for ChannelError {
    fn from(e: io::Error) -> Self {
        Self::IOError(e)
    }
}

impl From<std::string::FromUtf8Error> for ChannelError {
    fn from(e: std::string::FromUtf8Error) -> Self {
        Self::FromUtf8Error(e)
    }
}

impl From<PduError> for ChannelError {
    fn from(e: PduError) -> Self {
        Self::Pdu(e)
    }
}

impl From<ChannelError> for io::Error {
    fn from(e: ChannelError) -> io::Error {
        io::Error::other(format!("Virtual channel error: {e}"))
    }
}
