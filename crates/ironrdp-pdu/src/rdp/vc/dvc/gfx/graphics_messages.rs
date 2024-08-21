mod client;
mod server;

mod avc_messages;
use bitflags::bitflags;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive as _, ToPrimitive as _};

#[rustfmt::skip] // do not re-order this
pub use avc_messages::{Avc420BitmapStream, Avc444BitmapStream, Encoding, QuantQuality};
pub use client::{CacheImportReplyPdu, CapabilitiesAdvertisePdu, FrameAcknowledgePdu, QueueDepth};
pub use server::{
    CacheToSurfacePdu, CapabilitiesConfirmPdu, Codec1Type, Codec2Type, CreateSurfacePdu, DeleteEncodingContextPdu,
    DeleteSurfacePdu, EndFramePdu, EvictCacheEntryPdu, MapSurfaceToOutputPdu, MapSurfaceToScaledOutputPdu,
    MapSurfaceToScaledWindowPdu, PixelFormat, ResetGraphicsPdu, SolidFillPdu, StartFramePdu, SurfaceToCachePdu,
    SurfaceToSurfacePdu, Timestamp, WireToSurface1Pdu, WireToSurface2Pdu,
};

use super::RDP_GFX_HEADER_SIZE;
use crate::{DecodeResult, EncodeResult, PduDecode, PduEncode};
use ironrdp_core::{ReadCursor, WriteCursor};

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

impl CapabilitySet {
    const NAME: &'static str = "GfxCapabilitySet";

    const FIXED_PART_SIZE: usize = CAPABILITY_SET_HEADER_SIZE;
}

impl PduEncode for CapabilitySet {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.version().to_u32().unwrap());
        dst.write_u32(cast_length!("dataLength", self.size() - CAPABILITY_SET_HEADER_SIZE)?);

        match self {
            CapabilitySet::V8 { flags } => dst.write_u32(flags.bits()),
            CapabilitySet::V8_1 { flags } => dst.write_u32(flags.bits()),
            CapabilitySet::V10 { flags } => dst.write_u32(flags.bits()),
            CapabilitySet::V10_1 => dst.write_u128(V10_1_RESERVED),
            CapabilitySet::V10_2 { flags } => dst.write_u32(flags.bits()),
            CapabilitySet::V10_3 { flags } => dst.write_u32(flags.bits()),
            CapabilitySet::V10_4 { flags } => dst.write_u32(flags.bits()),
            CapabilitySet::V10_5 { flags } => dst.write_u32(flags.bits()),
            CapabilitySet::V10_6 { flags } => dst.write_u32(flags.bits()),
            CapabilitySet::V10_6Err { flags } => dst.write_u32(flags.bits()),
            CapabilitySet::V10_7 { flags } => dst.write_u32(flags.bits()),
            CapabilitySet::Unknown(data) => dst.write_slice(data),
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
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

impl<'de> PduDecode<'de> for CapabilitySet {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let version = CapabilityVersion::from_u32(src.read_u32())
            .ok_or_else(|| invalid_field_err!("version", "unhandled version"))?;
        let data_length: usize = cast_length!("dataLength", src.read_u32())?;

        ensure_size!(in: src, size: data_length);
        let data = src.read_slice(data_length);
        let mut cur = ReadCursor::new(data);

        let size = match version {
            CapabilityVersion::V8
            | CapabilityVersion::V8_1
            | CapabilityVersion::V10
            | CapabilityVersion::V10_2
            | CapabilityVersion::V10_3
            | CapabilityVersion::V10_4
            | CapabilityVersion::V10_5
            | CapabilityVersion::V10_6
            | CapabilityVersion::V10_6Err
            | CapabilityVersion::V10_7 => 4,
            CapabilityVersion::V10_1 => 16,
            CapabilityVersion::Unknown => 0,
        };

        ensure_size!(in: cur, size: size);
        match version {
            CapabilityVersion::V8 => Ok(CapabilitySet::V8 {
                flags: CapabilitiesV8Flags::from_bits_truncate(cur.read_u32()),
            }),
            CapabilityVersion::V8_1 => Ok(CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::from_bits_truncate(cur.read_u32()),
            }),
            CapabilityVersion::V10 => Ok(CapabilitySet::V10 {
                flags: CapabilitiesV10Flags::from_bits_truncate(cur.read_u32()),
            }),
            CapabilityVersion::V10_1 => {
                cur.read_u128();

                Ok(CapabilitySet::V10_1)
            }
            CapabilityVersion::V10_2 => Ok(CapabilitySet::V10_2 {
                flags: CapabilitiesV10Flags::from_bits_truncate(cur.read_u32()),
            }),
            CapabilityVersion::V10_3 => Ok(CapabilitySet::V10_3 {
                flags: CapabilitiesV103Flags::from_bits_truncate(cur.read_u32()),
            }),
            CapabilityVersion::V10_4 => Ok(CapabilitySet::V10_4 {
                flags: CapabilitiesV104Flags::from_bits_truncate(cur.read_u32()),
            }),
            CapabilityVersion::V10_5 => Ok(CapabilitySet::V10_5 {
                flags: CapabilitiesV104Flags::from_bits_truncate(cur.read_u32()),
            }),
            CapabilityVersion::V10_6 => Ok(CapabilitySet::V10_6 {
                flags: CapabilitiesV104Flags::from_bits_truncate(cur.read_u32()),
            }),
            CapabilityVersion::V10_6Err => Ok(CapabilitySet::V10_6Err {
                flags: CapabilitiesV104Flags::from_bits_truncate(cur.read_u32()),
            }),
            CapabilityVersion::V10_7 => Ok(CapabilitySet::V10_7 {
                flags: CapabilitiesV107Flags::from_bits_truncate(cur.read_u32()),
            }),
            CapabilityVersion::Unknown => Ok(CapabilitySet::Unknown(data.to_vec())),
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

impl Color {
    const NAME: &'static str = "GfxColor";

    const FIXED_PART_SIZE: usize = 4 /* BGRA */;
}

impl PduEncode for Color {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u8(self.b);
        dst.write_u8(self.g);
        dst.write_u8(self.r);
        dst.write_u8(self.xa);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for Color {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let b = src.read_u8();
        let g = src.read_u8();
        let r = src.read_u8();
        let xa = src.read_u8();

        Ok(Self { b, g, r, xa })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Point {
    pub x: u16,
    pub y: u16,
}

impl Point {
    const NAME: &'static str = "GfxPoint";

    const FIXED_PART_SIZE: usize = 2 /* X */ + 2 /* Y */;
}

impl PduEncode for Point {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.x);
        dst.write_u16(self.y);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for Point {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let x = src.read_u16();
        let y = src.read_u16();

        Ok(Self { x, y })
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
