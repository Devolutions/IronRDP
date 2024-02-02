use std::{fmt, mem};

use bit_field::BitField;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use super::{CapabilitySet, Color, Point, RDP_GFX_HEADER_SIZE};
use crate::cursor::{ReadCursor, WriteCursor};
use crate::gcc::Monitor;
use crate::geometry::InclusiveRectangle;
use crate::{decode_cursor, PduDecode, PduEncode, PduResult};

pub(crate) const RESET_GRAPHICS_PDU_SIZE: usize = 340;

const MAX_RESET_GRAPHICS_WIDTH_HEIGHT: u32 = 32_766;
const MONITOR_COUNT_MAX: u32 = 16;

#[derive(Clone, PartialEq, Eq)]
pub struct WireToSurface1Pdu {
    pub surface_id: u16,
    pub codec_id: Codec1Type,
    pub pixel_format: PixelFormat,
    pub destination_rectangle: InclusiveRectangle,
    pub bitmap_data: Vec<u8>,
}

impl fmt::Debug for WireToSurface1Pdu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WireToSurface1Pdu")
            .field("surface_id", &self.surface_id)
            .field("codec_id", &self.codec_id)
            .field("pixel_format", &self.pixel_format)
            .field("destination_rectangle", &self.destination_rectangle)
            .field("bitmap_data_length", &self.bitmap_data.len())
            .finish()
    }
}

impl WireToSurface1Pdu {
    const NAME: &'static str = "WireToSurface1Pdu";

    const FIXED_PART_SIZE: usize = 2 /* SurfaceId */ + 2 /* CodecId */ + 1 /* PixelFormat */ + InclusiveRectangle::FIXED_PART_SIZE /* Dest */ + 4 /* BitmapDataLen */;
}

impl PduEncode for WireToSurface1Pdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.surface_id);
        dst.write_u16(self.codec_id.to_u16().unwrap());
        dst.write_u8(self.pixel_format.to_u8().unwrap());
        self.destination_rectangle.encode(dst)?;
        dst.write_u32(cast_length!("BitmapDataLen", self.bitmap_data.len())?);
        dst.write_slice(&self.bitmap_data);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.bitmap_data.len()
    }
}

impl<'a> PduDecode<'a> for WireToSurface1Pdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let surface_id = src.read_u16();
        let codec_id =
            Codec1Type::from_u16(src.read_u16()).ok_or_else(|| invalid_message_err!("CodecId", "invalid codec ID"))?;
        let pixel_format = PixelFormat::from_u8(src.read_u8())
            .ok_or_else(|| invalid_message_err!("PixelFormat", "invalid pixel format"))?;
        let destination_rectangle = InclusiveRectangle::decode(src)?;
        let bitmap_data_length = cast_length!("BitmapDataLen", src.read_u32())?;

        ensure_size!(in: src, size: bitmap_data_length);
        let bitmap_data = src.read_slice(bitmap_data_length).to_vec();

        Ok(Self {
            surface_id,
            codec_id,
            pixel_format,
            destination_rectangle,
            bitmap_data,
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct WireToSurface2Pdu {
    pub surface_id: u16,
    pub codec_id: Codec2Type,
    pub codec_context_id: u32,
    pub pixel_format: PixelFormat,
    pub bitmap_data: Vec<u8>,
}

impl fmt::Debug for WireToSurface2Pdu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WireToSurface2Pdu")
            .field("surface_id", &self.surface_id)
            .field("codec_id", &self.codec_id)
            .field("codec_context_id", &self.codec_context_id)
            .field("pixel_format", &self.pixel_format)
            .field("bitmap_data_length", &self.bitmap_data.len())
            .finish()
    }
}

impl WireToSurface2Pdu {
    const NAME: &'static str = "WireToSurface2Pdu";

    const FIXED_PART_SIZE: usize = 2 /* SurfaceId */ + 2 /* CodecId */ + 4 /* ContextId */ + 1 /* PixelFormat */ + 4 /* BitmapDataLen */;
}

impl PduEncode for WireToSurface2Pdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.surface_id);
        dst.write_u16(self.codec_id.to_u16().unwrap());
        dst.write_u32(self.codec_context_id);
        dst.write_u8(self.pixel_format.to_u8().unwrap());
        dst.write_u32(cast_length!("BitmapDataLen", self.bitmap_data.len())?);
        dst.write_slice(&self.bitmap_data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.bitmap_data.len()
    }
}

impl<'a> PduDecode<'a> for WireToSurface2Pdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let surface_id = src.read_u16();
        let codec_id =
            Codec2Type::from_u16(src.read_u16()).ok_or_else(|| invalid_message_err!("CodecId", "invalid codec ID"))?;
        let codec_context_id = src.read_u32();
        let pixel_format = PixelFormat::from_u8(src.read_u8())
            .ok_or_else(|| invalid_message_err!("PixelFormat", "invalid pixel format"))?;
        let bitmap_data_length = cast_length!("BitmapDataLen", src.read_u32())?;

        ensure_size!(in: src, size: bitmap_data_length);
        let bitmap_data = src.read_slice(bitmap_data_length).to_vec();

        Ok(Self {
            surface_id,
            codec_id,
            codec_context_id,
            pixel_format,
            bitmap_data,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteEncodingContextPdu {
    pub surface_id: u16,
    pub codec_context_id: u32,
}

impl DeleteEncodingContextPdu {
    const NAME: &'static str = "DeleteEncodingContextPdu";

    const FIXED_PART_SIZE: usize = 2 /* SurfaceId */ + 4 /* CodecContextId */;
}

impl PduEncode for DeleteEncodingContextPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.surface_id);
        dst.write_u32(self.codec_context_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'a> PduDecode<'a> for DeleteEncodingContextPdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let surface_id = src.read_u16();
        let codec_context_id = src.read_u32();

        Ok(Self {
            surface_id,
            codec_context_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolidFillPdu {
    pub surface_id: u16,
    pub fill_pixel: Color,
    pub rectangles: Vec<InclusiveRectangle>,
}

impl SolidFillPdu {
    const NAME: &'static str = "CacheToSurfacePdu";

    const FIXED_PART_SIZE: usize = 2 /* SurfaceId */ + Color::FIXED_PART_SIZE /* Color */ + 2 /* RectCount */;
}

impl PduEncode for SolidFillPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.surface_id);
        self.fill_pixel.encode(dst)?;
        dst.write_u16(self.rectangles.len() as u16);

        for rectangle in self.rectangles.iter() {
            rectangle.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.rectangles.iter().map(|r| r.size()).sum::<usize>()
    }
}

impl<'a> PduDecode<'a> for SolidFillPdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let surface_id = src.read_u16();
        let fill_pixel = Color::decode(src)?;
        let rectangles_count = src.read_u16();

        ensure_size!(in: src, size: usize::from(rectangles_count) * InclusiveRectangle::FIXED_PART_SIZE);
        let rectangles = (0..rectangles_count)
            .map(|_| InclusiveRectangle::decode(src))
            .collect::<Result<_, _>>()?;

        Ok(Self {
            surface_id,
            fill_pixel,
            rectangles,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceToSurfacePdu {
    pub source_surface_id: u16,
    pub destination_surface_id: u16,
    pub source_rectangle: InclusiveRectangle,
    pub destination_points: Vec<Point>,
}

impl SurfaceToSurfacePdu {
    const NAME: &'static str = "SurfaceToSurfacePdu";

    const FIXED_PART_SIZE: usize = 2 /* SourceId */ + 2 /* DestId */ + InclusiveRectangle::FIXED_PART_SIZE /* SourceRect */ + 2 /* DestPointsCount */;
}

impl PduEncode for SurfaceToSurfacePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.source_surface_id);
        dst.write_u16(self.destination_surface_id);
        self.source_rectangle.encode(dst)?;

        dst.write_u16(cast_length!("DestinationPoints", self.destination_points.len())?);
        for rectangle in self.destination_points.iter() {
            rectangle.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.destination_points.iter().map(|r| r.size()).sum::<usize>()
    }
}

impl<'a> PduDecode<'a> for SurfaceToSurfacePdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let source_surface_id = src.read_u16();
        let destination_surface_id = src.read_u16();
        let source_rectangle = InclusiveRectangle::decode(src)?;
        let destination_points_count = src.read_u16();

        let destination_points = (0..destination_points_count)
            .map(|_| Point::decode(src))
            .collect::<Result<_, _>>()?;

        Ok(Self {
            source_surface_id,
            destination_surface_id,
            source_rectangle,
            destination_points,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceToCachePdu {
    pub surface_id: u16,
    pub cache_key: u64,
    pub cache_slot: u16,
    pub source_rectangle: InclusiveRectangle,
}

impl SurfaceToCachePdu {
    const NAME: &'static str = "SurfaceToCachePdu";

    const FIXED_PART_SIZE: usize = 2 /* SurfaceId */ + 8 /* CacheKey */ + 2 /* CacheSlot */ + InclusiveRectangle::FIXED_PART_SIZE /* SourceRect */;
}

impl PduEncode for SurfaceToCachePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.surface_id);
        dst.write_u64(self.cache_key);
        dst.write_u16(self.cache_slot);
        self.source_rectangle.encode(dst)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'a> PduDecode<'a> for SurfaceToCachePdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let surface_id = src.read_u16();
        let cache_key = src.read_u64();
        let cache_slot = src.read_u16();
        let source_rectangle = InclusiveRectangle::decode(src)?;

        Ok(Self {
            surface_id,
            cache_key,
            cache_slot,
            source_rectangle,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheToSurfacePdu {
    pub cache_slot: u16,
    pub surface_id: u16,
    pub destination_points: Vec<Point>,
}

impl CacheToSurfacePdu {
    const NAME: &'static str = "CacheToSurfacePdu";

    const FIXED_PART_SIZE: usize = mem::size_of::<u16>() * 3;
}

impl PduEncode for CacheToSurfacePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.cache_slot);
        dst.write_u16(self.surface_id);
        dst.write_u16(cast_length!("npoints", self.destination_points.len())?);
        for point in self.destination_points.iter() {
            point.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.destination_points.iter().map(|p| p.size()).sum::<usize>()
    }
}

impl<'de> PduDecode<'de> for CacheToSurfacePdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let cache_slot = src.read_u16();
        let surface_id = src.read_u16();
        let destination_points_count = src.read_u16();

        let destination_points = (0..destination_points_count)
            .map(|_| decode_cursor(src))
            .collect::<Result<_, _>>()?;

        Ok(Self {
            cache_slot,
            surface_id,
            destination_points,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSurfacePdu {
    pub surface_id: u16,
    pub width: u16,
    pub height: u16,
    pub pixel_format: PixelFormat,
}

impl CreateSurfacePdu {
    const NAME: &'static str = "CreateSurfacePdu";

    const FIXED_PART_SIZE: usize = 2 /* SurfaceId */ + 2 /* Width */ + 2 /* Height */ + 1 /* PixelFormat */;
}

impl PduEncode for CreateSurfacePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.surface_id);
        dst.write_u16(self.width);
        dst.write_u16(self.height);
        dst.write_u8(self.pixel_format.to_u8().unwrap());

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'a> PduDecode<'a> for CreateSurfacePdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let surface_id = src.read_u16();
        let width = src.read_u16();
        let height = src.read_u16();
        let pixel_format = PixelFormat::from_u8(src.read_u8())
            .ok_or_else(|| invalid_message_err!("pixelFormat", "invalid pixel format"))?;

        Ok(Self {
            surface_id,
            width,
            height,
            pixel_format,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteSurfacePdu {
    pub surface_id: u16,
}

impl DeleteSurfacePdu {
    const NAME: &'static str = "DeleteSurfacePdu";

    const FIXED_PART_SIZE: usize = 2 /* SurfaceId */;
}

impl PduEncode for DeleteSurfacePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.surface_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'a> PduDecode<'a> for DeleteSurfacePdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let surface_id = src.read_u16();

        Ok(Self { surface_id })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetGraphicsPdu {
    pub width: u32,
    pub height: u32,
    pub monitors: Vec<Monitor>,
}

impl ResetGraphicsPdu {
    const NAME: &'static str = "ResetGraphicsPdu";

    const FIXED_PART_SIZE: usize = 4 /* Width */ + 4 /* Height */;
}

impl PduEncode for ResetGraphicsPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.width);
        dst.write_u32(self.height);
        dst.write_u32(cast_length!("nMonitors", self.monitors.len())?);

        for monitor in self.monitors.iter() {
            monitor.encode(dst)?;
        }

        write_padding!(dst, self.padding_size());

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        RESET_GRAPHICS_PDU_SIZE - RDP_GFX_HEADER_SIZE
    }
}

impl<'a> PduDecode<'a> for ResetGraphicsPdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let width = src.read_u32();
        if width > MAX_RESET_GRAPHICS_WIDTH_HEIGHT {
            return Err(invalid_message_err!("width", "invalid reset graphics width"));
        }

        let height = src.read_u32();
        if height > MAX_RESET_GRAPHICS_WIDTH_HEIGHT {
            return Err(invalid_message_err!("height", "invalid reset graphics height"));
        }

        let monitor_count = src.read_u32();
        if monitor_count > MONITOR_COUNT_MAX {
            return Err(invalid_message_err!("height", "invalid reset graphics monitor count"));
        }

        let monitors = (0..monitor_count)
            .map(|_| Monitor::decode(src))
            .collect::<Result<Vec<_>, _>>()?;

        let pdu = Self {
            width,
            height,
            monitors,
        };

        read_padding!(src, pdu.padding_size());

        Ok(pdu)
    }
}

impl ResetGraphicsPdu {
    fn padding_size(&self) -> usize {
        RESET_GRAPHICS_PDU_SIZE - RDP_GFX_HEADER_SIZE - 12 - self.monitors.iter().map(|m| m.size()).sum::<usize>()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapSurfaceToOutputPdu {
    pub surface_id: u16,
    pub output_origin_x: u32,
    pub output_origin_y: u32,
}

impl MapSurfaceToOutputPdu {
    const NAME: &'static str = "MapSurfaceToOutputPdu";

    const FIXED_PART_SIZE: usize = 2 /* surfaceId */ + 2 /* reserved */ + 4 /* OutOriginX */ + 4 /* OutOriginY */;
}

impl PduEncode for MapSurfaceToOutputPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.surface_id);
        dst.write_u16(0); // reserved
        dst.write_u32(self.output_origin_x);
        dst.write_u32(self.output_origin_y);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'a> PduDecode<'a> for MapSurfaceToOutputPdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let surface_id = src.read_u16();
        let _reserved = src.read_u16();
        let output_origin_x = src.read_u32();
        let output_origin_y = src.read_u32();

        Ok(Self {
            surface_id,
            output_origin_x,
            output_origin_y,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapSurfaceToScaledOutputPdu {
    pub surface_id: u16,
    pub output_origin_x: u32,
    pub output_origin_y: u32,
    pub target_width: u32,
    pub target_height: u32,
}

impl MapSurfaceToScaledOutputPdu {
    const NAME: &'static str = "MapSurfaceToScaledOutputPdu";

    const FIXED_PART_SIZE: usize = 2 /* SurfaceId */ + 2 /* reserved */ + 4 /* OutOriginX */ + 4 /* OutOriginY */ + 4 /* TargetWidth */ + 4 /* TargetHeight */;
}

impl PduEncode for MapSurfaceToScaledOutputPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.surface_id);
        dst.write_u16(0); // reserved
        dst.write_u32(self.output_origin_x);
        dst.write_u32(self.output_origin_y);
        dst.write_u32(self.target_width);
        dst.write_u32(self.target_height);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'a> PduDecode<'a> for MapSurfaceToScaledOutputPdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let surface_id = src.read_u16();
        let _reserved = src.read_u16();
        let output_origin_x = src.read_u32();
        let output_origin_y = src.read_u32();
        let target_width = src.read_u32();
        let target_height = src.read_u32();

        Ok(Self {
            surface_id,
            output_origin_x,
            output_origin_y,
            target_width,
            target_height,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapSurfaceToScaledWindowPdu {
    pub surface_id: u16,
    pub window_id: u64,
    pub mapped_width: u32,
    pub mapped_height: u32,
    pub target_width: u32,
    pub target_height: u32,
}

impl MapSurfaceToScaledWindowPdu {
    const NAME: &'static str = "MapSurfaceToScaledWindowPdu";

    const FIXED_PART_SIZE: usize = 2 /* SurfaceId */ + 8 /* WindowId */ + 4 /* MappedWidth */ + 4 /* MappedHeight */ + 4 /* TargetWidth */ + 4 /* TargetHeight */;
}

impl PduEncode for MapSurfaceToScaledWindowPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        dst.write_u16(self.surface_id);
        dst.write_u64(self.window_id); // reserved
        dst.write_u32(self.mapped_width);
        dst.write_u32(self.mapped_height);
        dst.write_u32(self.target_width);
        dst.write_u32(self.target_height);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'a> PduDecode<'a> for MapSurfaceToScaledWindowPdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let surface_id = src.read_u16();
        let window_id = src.read_u64();
        let mapped_width = src.read_u32();
        let mapped_height = src.read_u32();
        let target_width = src.read_u32();
        let target_height = src.read_u32();

        Ok(Self {
            surface_id,
            window_id,
            mapped_width,
            mapped_height,
            target_width,
            target_height,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvictCacheEntryPdu {
    pub cache_slot: u16,
}

impl EvictCacheEntryPdu {
    const NAME: &'static str = "EvictCacheEntryPdu";

    const FIXED_PART_SIZE: usize = 2;
}

impl PduEncode for EvictCacheEntryPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.cache_slot);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'a> PduDecode<'a> for EvictCacheEntryPdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let cache_slot = src.read_u16();

        Ok(Self { cache_slot })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartFramePdu {
    pub timestamp: Timestamp,
    pub frame_id: u32,
}

impl StartFramePdu {
    const NAME: &'static str = "StartFramePdu";

    const FIXED_PART_SIZE: usize = Timestamp::FIXED_PART_SIZE + 4 /* FrameId */;
}

impl PduEncode for StartFramePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        self.timestamp.encode(dst)?;
        dst.write_u32(self.frame_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'a> PduDecode<'a> for StartFramePdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let timestamp = Timestamp::decode(src)?;
        let frame_id = src.read_u32();

        Ok(Self { timestamp, frame_id })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndFramePdu {
    pub frame_id: u32,
}

impl EndFramePdu {
    const NAME: &'static str = "EndFramePdu";

    const FIXED_PART_SIZE: usize = 4;
}

impl PduEncode for EndFramePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.frame_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'a> PduDecode<'a> for EndFramePdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let frame_id = src.read_u32();

        Ok(Self { frame_id })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitiesConfirmPdu(pub CapabilitySet);

impl CapabilitiesConfirmPdu {
    const NAME: &'static str = "CapabilitiesConfirmPdu";
}

impl PduEncode for CapabilitiesConfirmPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        self.0.encode(dst)
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.0.size()
    }
}

impl<'a> PduDecode<'a> for CapabilitiesConfirmPdu {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        let capability_set = CapabilitySet::decode(src)?;

        Ok(Self(capability_set))
    }
}

#[repr(u16)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum Codec1Type {
    Uncompressed = 0x0,
    RemoteFx = 0x3,
    ClearCodec = 0x8,
    Planar = 0xa,
    Avc420 = 0xb,
    Alpha = 0xc,
    Avc444 = 0xe,
    Avc444v2 = 0xf,
}

#[repr(u16)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum Codec2Type {
    RemoteFxProgressive = 0x9,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum PixelFormat {
    XRgb = 0x20,
    ARgb = 0x21,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Timestamp {
    pub milliseconds: u16,
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u16,
}

impl Timestamp {
    const NAME: &'static str = "Timestamp";

    const FIXED_PART_SIZE: usize = 4;
}

impl PduEncode for Timestamp {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        let mut timestamp: u32 = 0;

        timestamp.set_bits(..10, u32::from(self.milliseconds));
        timestamp.set_bits(10..16, u32::from(self.seconds));
        timestamp.set_bits(16..22, u32::from(self.minutes));
        timestamp.set_bits(22.., u32::from(self.hours));

        dst.write_u32(timestamp);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'a> PduDecode<'a> for Timestamp {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let timestamp = src.read_u32();

        let milliseconds = timestamp.get_bits(..10) as u16;
        let seconds = timestamp.get_bits(10..16) as u8;
        let minutes = timestamp.get_bits(16..22) as u8;
        let hours = timestamp.get_bits(22..) as u16;

        Ok(Self {
            milliseconds,
            seconds,
            minutes,
            hours,
        })
    }
}
