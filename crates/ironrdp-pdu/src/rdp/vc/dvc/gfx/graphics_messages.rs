mod client;
mod server;

use std::io;
mod avc_messages;
use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive as _, ToPrimitive as _};
use thiserror::Error;

#[rustfmt::skip] // do not re-order this
pub use avc_messages::{Avc420BitmapStream, Avc444BitmapStream, Encoding, QuantQuality};
pub use client::{CacheImportReplyPdu, CapabilitiesAdvertisePdu, FrameAcknowledgePdu, QueueDepth};
pub(crate) use server::RESET_GRAPHICS_PDU_SIZE;
pub use server::{
    CacheToSurfacePdu, CapabilitiesConfirmPdu, Codec1Type, Codec2Type, CreateSurfacePdu, DeleteEncodingContextPdu,
    DeleteSurfacePdu, EndFramePdu, EvictCacheEntryPdu, MapSurfaceToOutputPdu, MapSurfaceToScaledOutputPdu,
    MapSurfaceToScaledWindowPdu, PixelFormat, ResetGraphicsPdu, SolidFillPdu, StartFramePdu, SurfaceToCachePdu,
    SurfaceToSurfacePdu, Timestamp, WireToSurface1Pdu, WireToSurface2Pdu,
};

use super::RDP_GFX_HEADER_SIZE;
use crate::gcc::MonitorDataError;
use crate::PduParsing;

const CAPABILITY_SET_HEADER_SIZE: usize = 8;

const V10_1_RESERVED: u128 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitySet {
    V8 { flags: CapabilitiesV8Flags },
    V8_1 { flags: CapabilitiesV81Flags },
    V10 { flags: CapabilitiesV10Flags },
    V10_1,
    V10_2 { flags: CapabilitiesV10Flags },
    V10_3 { flags: CapabilitiesV103Flags },
    V10_4 { flags: CapabilitiesV104Flags },
    V10_5 { flags: CapabilitiesV104Flags },
    V10_6 { flags: CapabilitiesV104Flags },
    V10_6Err { flags: CapabilitiesV104Flags },
    V10_7 { flags: CapabilitiesV107Flags },
    Unknown(Vec<u8>),
}

impl CapabilitySet {
    fn version(&self) -> CapabilityVersion {
        match self {
            CapabilitySet::V8 { .. } => CapabilityVersion::V8,
            CapabilitySet::V8_1 { .. } => CapabilityVersion::V8_1,
            CapabilitySet::V10 { .. } => CapabilityVersion::V10,
            CapabilitySet::V10_1 { .. } => CapabilityVersion::V10_1,
            CapabilitySet::V10_2 { .. } => CapabilityVersion::V10_2,
            CapabilitySet::V10_3 { .. } => CapabilityVersion::V10_3,
            CapabilitySet::V10_4 { .. } => CapabilityVersion::V10_4,
            CapabilitySet::V10_5 { .. } => CapabilityVersion::V10_5,
            CapabilitySet::V10_6 { .. } => CapabilityVersion::V10_6,
            CapabilitySet::V10_6Err { .. } => CapabilityVersion::V10_6Err,
            CapabilitySet::V10_7 { .. } => CapabilityVersion::V10_7,
            CapabilitySet::Unknown { .. } => CapabilityVersion::Unknown,
        }
    }
}

impl PduParsing for CapabilitySet {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, GraphicsMessagesError> {
        let version = CapabilityVersion::from_u32(stream.read_u32::<LittleEndian>()?)
            .ok_or(GraphicsMessagesError::InvalidCapabilitiesVersion)?;
        let data_length = stream.read_u32::<LittleEndian>()?;

        let mut data = vec![0; data_length as usize];
        stream.read_exact(data.as_mut())?;

        match version {
            CapabilityVersion::V8 => Ok(CapabilitySet::V8 {
                flags: CapabilitiesV8Flags::from_bits_truncate(data.as_slice().read_u32::<LittleEndian>()?),
            }),
            CapabilityVersion::V8_1 => Ok(CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::from_bits_truncate(data.as_slice().read_u32::<LittleEndian>()?),
            }),
            CapabilityVersion::V10 => Ok(CapabilitySet::V10 {
                flags: CapabilitiesV10Flags::from_bits_truncate(data.as_slice().read_u32::<LittleEndian>()?),
            }),
            CapabilityVersion::V10_1 => {
                data.as_slice().read_u128::<LittleEndian>()?;

                Ok(CapabilitySet::V10_1)
            }
            CapabilityVersion::V10_2 => Ok(CapabilitySet::V10_2 {
                flags: CapabilitiesV10Flags::from_bits_truncate(data.as_slice().read_u32::<LittleEndian>()?),
            }),
            CapabilityVersion::V10_3 => Ok(CapabilitySet::V10_3 {
                flags: CapabilitiesV103Flags::from_bits_truncate(data.as_slice().read_u32::<LittleEndian>()?),
            }),
            CapabilityVersion::V10_4 => Ok(CapabilitySet::V10_4 {
                flags: CapabilitiesV104Flags::from_bits_truncate(data.as_slice().read_u32::<LittleEndian>()?),
            }),
            CapabilityVersion::V10_5 => Ok(CapabilitySet::V10_5 {
                flags: CapabilitiesV104Flags::from_bits_truncate(data.as_slice().read_u32::<LittleEndian>()?),
            }),
            CapabilityVersion::V10_6 => Ok(CapabilitySet::V10_6 {
                flags: CapabilitiesV104Flags::from_bits_truncate(data.as_slice().read_u32::<LittleEndian>()?),
            }),
            CapabilityVersion::V10_6Err => Ok(CapabilitySet::V10_6Err {
                flags: CapabilitiesV104Flags::from_bits_truncate(data.as_slice().read_u32::<LittleEndian>()?),
            }),
            CapabilityVersion::V10_7 => Ok(CapabilitySet::V10_7 {
                flags: CapabilitiesV107Flags::from_bits_truncate(data.as_slice().read_u32::<LittleEndian>()?),
            }),
            CapabilityVersion::Unknown => Ok(CapabilitySet::Unknown(data)),
        }
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), GraphicsMessagesError> {
        stream.write_u32::<LittleEndian>(self.version().to_u32().unwrap())?;
        stream.write_u32::<LittleEndian>((self.buffer_length() - CAPABILITY_SET_HEADER_SIZE) as u32)?;

        match self {
            CapabilitySet::V8 { flags } => stream.write_u32::<LittleEndian>(flags.bits())?,
            CapabilitySet::V8_1 { flags } => stream.write_u32::<LittleEndian>(flags.bits())?,
            CapabilitySet::V10 { flags } => stream.write_u32::<LittleEndian>(flags.bits())?,
            CapabilitySet::V10_1 => stream.write_u128::<LittleEndian>(V10_1_RESERVED)?,
            CapabilitySet::V10_2 { flags } => stream.write_u32::<LittleEndian>(flags.bits())?,
            CapabilitySet::V10_3 { flags } => stream.write_u32::<LittleEndian>(flags.bits())?,
            CapabilitySet::V10_4 { flags } => stream.write_u32::<LittleEndian>(flags.bits())?,
            CapabilitySet::V10_5 { flags } => stream.write_u32::<LittleEndian>(flags.bits())?,
            CapabilitySet::V10_6 { flags } => stream.write_u32::<LittleEndian>(flags.bits())?,
            CapabilitySet::V10_6Err { flags } => stream.write_u32::<LittleEndian>(flags.bits())?,
            CapabilitySet::V10_7 { flags } => stream.write_u32::<LittleEndian>(flags.bits())?,
            CapabilitySet::Unknown(data) => stream.write_all(data)?,
        }

        Ok(())
    }
    fn buffer_length(&self) -> usize {
        CAPABILITY_SET_HEADER_SIZE
            + match self {
                CapabilitySet::V8 { .. }
                | CapabilitySet::V8_1 { .. }
                | CapabilitySet::V10 { .. }
                | CapabilitySet::V10_2 { .. }
                | CapabilitySet::V10_3 { .. }
                | CapabilitySet::V10_4 { .. }
                | CapabilitySet::V10_5 { .. }
                | CapabilitySet::V10_6 { .. }
                | CapabilitySet::V10_6Err { .. }
                | CapabilitySet::V10_7 { .. } => 4,
                CapabilitySet::V10_1 { .. } => 16,
                CapabilitySet::Unknown(data) => data.len(),
            }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Color {
    pub b: u8,
    pub g: u8,
    pub r: u8,
    pub xa: u8,
}

impl PduParsing for Color {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let b = stream.read_u8()?;
        let g = stream.read_u8()?;
        let r = stream.read_u8()?;
        let xa = stream.read_u8()?;

        Ok(Self { b, g, r, xa })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u8(self.b)?;
        stream.write_u8(self.g)?;
        stream.write_u8(self.r)?;
        stream.write_u8(self.xa)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        4
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Point {
    pub x: u16,
    pub y: u16,
}

impl PduParsing for Point {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let x = stream.read_u16::<LittleEndian>()?;
        let y = stream.read_u16::<LittleEndian>()?;

        Ok(Self { x, y })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.x)?;
        stream.write_u16::<LittleEndian>(self.y)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        4
    }
}

#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub(crate) enum CapabilityVersion {
    V8 = 0x8_0004,
    V8_1 = 0x8_0105,
    V10 = 0xa_0002,
    V10_1 = 0xa_0100,
    V10_2 = 0xa_0200,
    V10_3 = 0xa_0301,
    V10_4 = 0xa_0400,
    V10_5 = 0xa_0502,
    V10_6 = 0xa_0600,    // [MS-RDPEGFX-errata]
    V10_6Err = 0xa_0601, // defined similar to FreeRDP to maintain best compatibility
    V10_7 = 0xa_0701,
    Unknown = 0xa_0702,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct CapabilitiesV8Flags: u32  {
        const THIN_CLIENT = 0x1;
        const SMALL_CACHE = 0x2;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct CapabilitiesV81Flags: u32  {
        const THIN_CLIENT = 0x01;
        const SMALL_CACHE = 0x02;
        const AVC420_ENABLED = 0x10;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct CapabilitiesV10Flags: u32 {
        const SMALL_CACHE = 0x02;
        const AVC_DISABLED = 0x20;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct CapabilitiesV103Flags: u32  {
        const AVC_DISABLED = 0x20;
        const AVC_THIN_CLIENT = 0x40;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct CapabilitiesV104Flags: u32  {
        const SMALL_CACHE = 0x02;
        const AVC_DISABLED = 0x20;
        const AVC_THIN_CLIENT = 0x40;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct CapabilitiesV107Flags: u32  {
        const SMALL_CACHE = 0x02;
        const AVC_DISABLED = 0x20;
        const AVC_THIN_CLIENT = 0x40;
        const SCALEDMAP_DISABLE = 0x80;
    }
}

#[derive(Debug, Error)]
pub enum GraphicsMessagesError {
    #[error("IO error")]
    IOError(#[from] io::Error),
    #[error("invalid codec ID version 1")]
    InvalidCodec1Id,
    #[error("invalid codec ID version 2")]
    InvalidCodec2Id,
    #[error("invalid pixel format")]
    InvalidPixelFormat,
    #[error("monitor error")]
    MonitorError(#[from] MonitorDataError),
    #[error("invalid ResetGraphics PDU width: {} > MAX ({})", actual, max)]
    InvalidResetGraphicsPduWidth { actual: u32, max: u32 },
    #[error("invalid ResetGraphics PDU height: {} > MAX ({})", actual, max)]
    InvalidResetGraphicsPduHeight { actual: u32, max: u32 },
    #[error("invalid ResetGraphics PDU monitors count: {} > MAX ({})", actual, max)]
    InvalidResetGraphicsPduMonitorsCount { actual: u32, max: u32 },
    #[error("invalid capabilities version")]
    InvalidCapabilitiesVersion,
    #[error("both luma and chroma packets specified but length is missing")]
    InvalidAvcEncoding,
}
