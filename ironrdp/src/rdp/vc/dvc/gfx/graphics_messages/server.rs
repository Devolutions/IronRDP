use std::io;

use bit_field::BitField;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use super::{CapabilitySet, Color, GraphicsMessagesError, Point, Rectangle, RDP_GFX_HEADER_SIZE};
use crate::{
    gcc::{Monitor, MonitorDataError},
    PduParsing,
};

pub const RESET_GRAPHICS_PDU_SIZE: usize = 340;

const MAX_RESET_GRAPHICS_WIDTH_HEIGHT: u32 = 32_766;
const MONITOR_COUNT_MAX: u32 = 16;

#[derive(Debug, Clone, PartialEq)]
pub struct WireToSurface1Pdu {
    pub surface_id: u16,
    pub codec_id: Codec1Type,
    pub pixel_format: PixelFormat,
    pub destination_rectangle: Rectangle,
    pub bitmap_data_length: usize,
}

impl PduParsing for WireToSurface1Pdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let surface_id = stream.read_u16::<LittleEndian>()?;
        let codec_id = Codec1Type::from_u16(stream.read_u16::<LittleEndian>()?)
            .ok_or(GraphicsMessagesError::InvalidCodec1Id)?;
        let pixel_format = PixelFormat::from_u8(stream.read_u8()?)
            .ok_or(GraphicsMessagesError::InvalidFixelFormat)?;
        let destination_rectangle = Rectangle::from_buffer(&mut stream)?;
        let bitmap_data_length = stream.read_u32::<LittleEndian>()? as usize;

        Ok(Self {
            surface_id,
            codec_id,
            pixel_format,
            destination_rectangle,
            bitmap_data_length,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.surface_id)?;
        stream.write_u16::<LittleEndian>(self.codec_id.to_u16().unwrap())?;
        stream.write_u8(self.pixel_format.to_u8().unwrap())?;
        self.destination_rectangle.to_buffer(&mut stream)?;
        stream.write_u32::<LittleEndian>(self.bitmap_data_length as u32)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        17
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WireToSurface2Pdu {
    pub surface_id: u16,
    pub codec_id: Codec2Type,
    pub codec_context_id: u32,
    pub pixel_format: PixelFormat,
    pub bitmap_data_length: usize,
}

impl PduParsing for WireToSurface2Pdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let surface_id = stream.read_u16::<LittleEndian>()?;
        let codec_id = Codec2Type::from_u16(stream.read_u16::<LittleEndian>()?)
            .ok_or(GraphicsMessagesError::InvalidCodec2Id)?;
        let codec_context_id = stream.read_u32::<LittleEndian>()?;
        let pixel_format = PixelFormat::from_u8(stream.read_u8()?)
            .ok_or(GraphicsMessagesError::InvalidFixelFormat)?;
        let bitmap_data_length = stream.read_u32::<LittleEndian>()? as usize;

        Ok(Self {
            surface_id,
            codec_id,
            codec_context_id,
            pixel_format,
            bitmap_data_length,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.surface_id)?;
        stream.write_u16::<LittleEndian>(self.codec_id.to_u16().unwrap())?;
        stream.write_u32::<LittleEndian>(self.codec_context_id)?;
        stream.write_u8(self.pixel_format.to_u8().unwrap())?;
        stream.write_u32::<LittleEndian>(self.bitmap_data_length as u32)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        13
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeleteEncodingContextPdu {
    pub surface_id: u16,
    pub codec_context_id: u32,
}

impl PduParsing for DeleteEncodingContextPdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let surface_id = stream.read_u16::<LittleEndian>()?;
        let codec_context_id = stream.read_u32::<LittleEndian>()?;

        Ok(Self {
            surface_id,
            codec_context_id,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.surface_id)?;
        stream.write_u32::<LittleEndian>(self.codec_context_id)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        6
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SolidFillPdu {
    pub surface_id: u16,
    pub fill_pixel: Color,
    pub rectangles: Vec<Rectangle>,
}

impl PduParsing for SolidFillPdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let surface_id = stream.read_u16::<LittleEndian>()?;
        let fill_pixel = Color::from_buffer(&mut stream)?;
        let rectangles_count = stream.read_u16::<LittleEndian>()?;

        let rectangles = (0..rectangles_count)
            .map(|_| Rectangle::from_buffer(&mut stream))
            .collect::<Result<Vec<_>, Self::Error>>()?;

        Ok(Self {
            surface_id,
            fill_pixel,
            rectangles,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.surface_id)?;
        self.fill_pixel.to_buffer(&mut stream)?;
        stream.write_u16::<LittleEndian>(self.rectangles.len() as u16)?;
        for rectangle in self.rectangles.iter() {
            rectangle.to_buffer(&mut stream)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        8 + self
            .rectangles
            .iter()
            .map(|r| r.buffer_length())
            .sum::<usize>()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceToSurfacePdu {
    pub source_surface_id: u16,
    pub destination_surface_id: u16,
    pub source_rectangle: Rectangle,
    pub destination_points: Vec<Point>,
}

impl PduParsing for SurfaceToSurfacePdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let source_surface_id = stream.read_u16::<LittleEndian>()?;
        let destination_surface_id = stream.read_u16::<LittleEndian>()?;
        let source_rectangle = Rectangle::from_buffer(&mut stream)?;
        let destination_points_count = stream.read_u16::<LittleEndian>()?;

        let destination_points = (0..destination_points_count)
            .map(|_| Point::from_buffer(&mut stream))
            .collect::<Result<Vec<_>, Self::Error>>()?;

        Ok(Self {
            source_surface_id,
            destination_surface_id,
            source_rectangle,
            destination_points,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.source_surface_id)?;
        stream.write_u16::<LittleEndian>(self.destination_surface_id)?;
        self.source_rectangle.to_buffer(&mut stream)?;

        stream.write_u16::<LittleEndian>(self.destination_points.len() as u16)?;
        for rectangle in self.destination_points.iter() {
            rectangle.to_buffer(&mut stream)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        6 + self.source_rectangle.buffer_length()
            + self
                .destination_points
                .iter()
                .map(|r| r.buffer_length())
                .sum::<usize>()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceToCachePdu {
    pub surface_id: u16,
    pub cache_key: u64,
    pub cache_slot: u16,
    pub source_rectangle: Rectangle,
}

impl PduParsing for SurfaceToCachePdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let surface_id = stream.read_u16::<LittleEndian>()?;
        let cache_key = stream.read_u64::<LittleEndian>()?;
        let cache_slot = stream.read_u16::<LittleEndian>()?;
        let source_rectangle = Rectangle::from_buffer(&mut stream)?;

        Ok(Self {
            surface_id,
            cache_key,
            cache_slot,
            source_rectangle,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.surface_id)?;
        stream.write_u64::<LittleEndian>(self.cache_key)?;
        stream.write_u16::<LittleEndian>(self.cache_slot)?;
        self.source_rectangle.to_buffer(&mut stream)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        12 + self.source_rectangle.buffer_length()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CacheToSurfacePdu {
    pub cache_slot: u16,
    pub surface_id: u16,
    pub destination_points: Vec<Point>,
}

impl PduParsing for CacheToSurfacePdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let cache_slot = stream.read_u16::<LittleEndian>()?;
        let surface_id = stream.read_u16::<LittleEndian>()?;
        let destination_points_count = stream.read_u16::<LittleEndian>()?;

        let destination_points = (0..destination_points_count)
            .map(|_| Point::from_buffer(&mut stream))
            .collect::<Result<Vec<_>, Self::Error>>()?;

        Ok(Self {
            cache_slot,
            surface_id,
            destination_points,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.cache_slot)?;
        stream.write_u16::<LittleEndian>(self.surface_id)?;
        stream.write_u16::<LittleEndian>(self.destination_points.len() as u16)?;
        for point in self.destination_points.iter() {
            point.to_buffer(&mut stream)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        6 + self
            .destination_points
            .iter()
            .map(|p| p.buffer_length())
            .sum::<usize>()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateSurfacePdu {
    pub surface_id: u16,
    pub width: u16,
    pub height: u16,
    pub pixel_format: PixelFormat,
}

impl PduParsing for CreateSurfacePdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let surface_id = stream.read_u16::<LittleEndian>()?;
        let width = stream.read_u16::<LittleEndian>()?;
        let height = stream.read_u16::<LittleEndian>()?;
        let pixel_format = PixelFormat::from_u8(stream.read_u8()?)
            .ok_or(GraphicsMessagesError::InvalidFixelFormat)?;

        Ok(Self {
            surface_id,
            width,
            height,
            pixel_format,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.surface_id)?;
        stream.write_u16::<LittleEndian>(self.width)?;
        stream.write_u16::<LittleEndian>(self.height)?;
        stream.write_u8(self.pixel_format.to_u8().unwrap())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        7
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeleteSurfacePdu {
    pub surface_id: u16,
}

impl PduParsing for DeleteSurfacePdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let surface_id = stream.read_u16::<LittleEndian>()?;

        Ok(Self { surface_id })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.surface_id)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        2
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResetGraphicsPdu {
    pub width: u32,
    pub height: u32,
    pub monitors: Vec<Monitor>,
}

impl ResetGraphicsPdu {
    fn padding_size(&self) -> usize {
        RESET_GRAPHICS_PDU_SIZE
            - RDP_GFX_HEADER_SIZE
            - 12
            - self
                .monitors
                .iter()
                .map(|m| m.buffer_length())
                .sum::<usize>()
    }
}

impl PduParsing for ResetGraphicsPdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let width = stream.read_u32::<LittleEndian>()?;
        if width > MAX_RESET_GRAPHICS_WIDTH_HEIGHT {
            return Err(GraphicsMessagesError::InvalidResetGraphicsPduWidth {
                actual: width,
                max: MAX_RESET_GRAPHICS_WIDTH_HEIGHT,
            });
        }

        let height = stream.read_u32::<LittleEndian>()?;
        if height > MAX_RESET_GRAPHICS_WIDTH_HEIGHT {
            return Err(GraphicsMessagesError::InvalidResetGraphicsPduHeight {
                actual: height,
                max: MAX_RESET_GRAPHICS_WIDTH_HEIGHT,
            });
        }

        let monitor_count = stream.read_u32::<LittleEndian>()?;
        if monitor_count > MONITOR_COUNT_MAX {
            return Err(
                GraphicsMessagesError::InvalidResetGraphicsPduMonitorsCount {
                    actual: monitor_count,
                    max: MAX_RESET_GRAPHICS_WIDTH_HEIGHT,
                },
            );
        }

        let monitors = (0..monitor_count)
            .map(|_| Monitor::from_buffer(&mut stream))
            .collect::<Result<Vec<_>, MonitorDataError>>()?;

        let pdu = Self {
            width,
            height,
            monitors,
        };

        let mut padding = vec![0; pdu.padding_size()];
        stream.read_exact(padding.as_mut())?;

        Ok(pdu)
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(self.width)?;
        stream.write_u32::<LittleEndian>(self.height)?;
        stream.write_u32::<LittleEndian>(self.monitors.len() as u32)?;

        for monitor in self.monitors.iter() {
            monitor.to_buffer(&mut stream)?;
        }

        let padding = vec![0; self.padding_size()];
        stream.write_all(padding.as_slice())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        RESET_GRAPHICS_PDU_SIZE - RDP_GFX_HEADER_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapSurfaceToOutputPdu {
    pub surface_id: u16,
    pub output_origin_x: u32,
    pub output_origin_y: u32,
}

impl PduParsing for MapSurfaceToOutputPdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let surface_id = stream.read_u16::<LittleEndian>()?;
        let _reserved = stream.read_u16::<LittleEndian>()?;
        let output_origin_x = stream.read_u32::<LittleEndian>()?;
        let output_origin_y = stream.read_u32::<LittleEndian>()?;

        Ok(Self {
            surface_id,
            output_origin_x,
            output_origin_y,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.surface_id)?;
        stream.write_u16::<LittleEndian>(0)?; // reserved
        stream.write_u32::<LittleEndian>(self.output_origin_x)?;
        stream.write_u32::<LittleEndian>(self.output_origin_y)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        12
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvictCacheEntryPdu {
    pub cache_slot: u16,
}

impl PduParsing for EvictCacheEntryPdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let cache_slot = stream.read_u16::<LittleEndian>()?;

        Ok(Self { cache_slot })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.cache_slot)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        2
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StartFramePdu {
    pub timestamp: Timestamp,
    pub frame_id: u32,
}

impl PduParsing for StartFramePdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let timestamp = Timestamp::from_buffer(&mut stream)?;
        let frame_id = stream.read_u32::<LittleEndian>()?;

        Ok(Self {
            timestamp,
            frame_id,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        self.timestamp.to_buffer(&mut stream)?;
        stream.write_u32::<LittleEndian>(self.frame_id)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        8
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EndFramePdu {
    pub frame_id: u32,
}

impl PduParsing for EndFramePdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let frame_id = stream.read_u32::<LittleEndian>()?;

        Ok(Self { frame_id })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(self.frame_id)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        4
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CapabilitiesConfirmPdu(pub CapabilitySet);

impl PduParsing for CapabilitiesConfirmPdu {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let capability_set = CapabilitySet::from_buffer(&mut stream)?;

        Ok(Self(capability_set))
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        self.0.to_buffer(&mut stream)
    }

    fn buffer_length(&self) -> usize {
        self.0.buffer_length()
    }
}

#[repr(u16)]
#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
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
#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum Codec2Type {
    RemoteFxProgressive = 0x9,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum PixelFormat {
    XRgb = 0x20,
    ARgb = 0x21,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Timestamp {
    pub milliseconds: u16,
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u16,
}

impl PduParsing for Timestamp {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let timestamp = stream.read_u32::<LittleEndian>()?;

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

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let mut timestamp: u32 = 0;

        timestamp.set_bits(..10, u32::from(self.milliseconds));
        timestamp.set_bits(10..16, u32::from(self.seconds));
        timestamp.set_bits(16..22, u32::from(self.minutes));
        timestamp.set_bits(22.., u32::from(self.hours));

        stream.write_u32::<LittleEndian>(timestamp)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        4
    }
}
