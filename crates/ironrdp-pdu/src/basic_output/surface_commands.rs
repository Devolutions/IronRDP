#[cfg(test)]
mod tests;

use std::io::{self, Write};

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive as _, ToPrimitive as _};
use thiserror::Error;

use crate::geometry::Rectangle;
use crate::utils::SplitTo;
use crate::PduBufferParsing;

pub const SURFACE_COMMAND_HEADER_SIZE: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurfaceCommand<'a> {
    SetSurfaceBits(SurfaceBitsPdu<'a>),
    FrameMarker(FrameMarkerPdu),
    StreamSurfaceBits(SurfaceBitsPdu<'a>),
}

impl<'a> PduBufferParsing<'a> for SurfaceCommand<'a> {
    type Error = SurfaceCommandsError;

    fn from_buffer_consume(buffer: &mut &'a [u8]) -> Result<Self, Self::Error> {
        let cmd_type = buffer.read_u16::<LittleEndian>()?;
        let cmd_type =
            SurfaceCommandType::from_u16(cmd_type).ok_or(SurfaceCommandsError::InvalidSurfaceCommandType(cmd_type))?;

        match cmd_type {
            SurfaceCommandType::SetSurfaceBits => {
                Ok(Self::SetSurfaceBits(SurfaceBitsPdu::from_buffer_consume(buffer)?))
            }
            SurfaceCommandType::FrameMarker => Ok(Self::FrameMarker(FrameMarkerPdu::from_buffer_consume(buffer)?)),
            SurfaceCommandType::StreamSurfaceBits => {
                Ok(Self::StreamSurfaceBits(SurfaceBitsPdu::from_buffer_consume(buffer)?))
            }
        }
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        let cmd_type = SurfaceCommandType::from(self);
        buffer.write_u16::<LittleEndian>(cmd_type.to_u16().unwrap())?;

        match self {
            Self::SetSurfaceBits(pdu) | Self::StreamSurfaceBits(pdu) => pdu.to_buffer_consume(buffer),
            Self::FrameMarker(pdu) => pdu.to_buffer_consume(buffer),
        }
    }

    fn buffer_length(&self) -> usize {
        SURFACE_COMMAND_HEADER_SIZE
            + match self {
                Self::SetSurfaceBits(pdu) | Self::StreamSurfaceBits(pdu) => pdu.buffer_length(),
                Self::FrameMarker(pdu) => pdu.buffer_length(),
            }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceBitsPdu<'a> {
    pub destination: Rectangle,
    pub extended_bitmap_data: ExtendedBitmapDataPdu<'a>,
}

impl<'a> PduBufferParsing<'a> for SurfaceBitsPdu<'a> {
    type Error = SurfaceCommandsError;

    fn from_buffer_consume(mut buffer: &mut &'a [u8]) -> Result<Self, Self::Error> {
        let destination = Rectangle::from_buffer_exclusive(&mut buffer)?;
        let extended_bitmap_data = ExtendedBitmapDataPdu::from_buffer_consume(buffer)?;

        Ok(Self {
            destination,
            extended_bitmap_data,
        })
    }

    fn to_buffer_consume(&self, mut buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        self.destination.to_buffer_exclusive(&mut buffer)?;
        self.extended_bitmap_data.to_buffer_consume(buffer)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        8 + self.extended_bitmap_data.buffer_length()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameMarkerPdu {
    pub frame_action: FrameAction,
    pub frame_id: Option<u32>,
}

impl<'a> PduBufferParsing<'a> for FrameMarkerPdu {
    type Error = SurfaceCommandsError;

    fn from_buffer_consume(buffer: &mut &[u8]) -> Result<Self, Self::Error> {
        let frame_action = buffer.read_u16::<LittleEndian>()?;
        let frame_action =
            FrameAction::from_u16(frame_action).ok_or(SurfaceCommandsError::InvalidFrameAction(frame_action))?;

        let frame_id = if buffer.is_empty() {
            // Sometimes Windows 10 RDP server sends not complete FrameMarker PDU (without frame ID),
            // so we made frame ID field as optional (not officially)

            None
        } else {
            Some(buffer.read_u32::<LittleEndian>()?)
        };

        Ok(Self { frame_action, frame_id })
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(self.frame_action.to_u16().unwrap())?;
        buffer.write_u32::<LittleEndian>(self.frame_id.unwrap_or(0))?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        6
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedBitmapDataPdu<'a> {
    pub bpp: u8,
    pub codec_id: u8,
    pub width: u16,
    pub height: u16,
    pub header: Option<BitmapDataHeader>,
    pub data: &'a [u8],
}

impl<'a> PduBufferParsing<'a> for ExtendedBitmapDataPdu<'a> {
    type Error = SurfaceCommandsError;

    fn from_buffer_consume(buffer: &mut &'a [u8]) -> Result<Self, Self::Error> {
        let bpp = buffer.read_u8()?;
        let flags = BitmapDataFlags::from_bits_truncate(buffer.read_u8()?);
        let _reserved = buffer.read_u8()?;
        let codec_id = buffer.read_u8()?;
        let width = buffer.read_u16::<LittleEndian>()?;
        let height = buffer.read_u16::<LittleEndian>()?;
        let data_length = buffer.read_u32::<LittleEndian>()? as usize;
        let header = if flags.contains(BitmapDataFlags::COMPRESSED_BITMAP_HEADER_PRESENT) {
            Some(BitmapDataHeader::from_buffer_consume(buffer)?)
        } else {
            None
        };

        if buffer.len() < data_length {
            return Err(SurfaceCommandsError::InvalidDataLength {
                expected: data_length,
                actual: buffer.len(),
            });
        }
        let data = buffer.split_to(data_length);

        Ok(Self {
            bpp,
            codec_id,
            width,
            height,
            header,
            data,
        })
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        buffer.write_u8(self.bpp)?;

        let flags = if self.header.is_some() {
            BitmapDataFlags::COMPRESSED_BITMAP_HEADER_PRESENT
        } else {
            BitmapDataFlags::empty()
        };
        buffer.write_u8(flags.bits())?;
        buffer.write_u8(0)?; // reserved
        buffer.write_u8(self.codec_id)?;
        buffer.write_u16::<LittleEndian>(self.width)?;
        buffer.write_u16::<LittleEndian>(self.height)?;
        buffer.write_u32::<LittleEndian>(self.data.len() as u32)?;
        if let Some(ref header) = self.header {
            header.to_buffer_consume(buffer)?;
        }
        buffer.write_all(self.data)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        12 + self.header.as_ref().map(PduBufferParsing::buffer_length).unwrap_or(0) + self.data.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitmapDataHeader {
    pub high_unique_id: u32,
    pub low_unique_id: u32,
    pub tm_milliseconds: u64,
    pub tm_seconds: u64,
}

impl<'a> PduBufferParsing<'a> for BitmapDataHeader {
    type Error = SurfaceCommandsError;

    fn from_buffer_consume(buffer: &mut &[u8]) -> Result<Self, Self::Error> {
        let high_unique_id = buffer.read_u32::<LittleEndian>()?;
        let low_unique_id = buffer.read_u32::<LittleEndian>()?;
        let tm_milliseconds = buffer.read_u64::<LittleEndian>()?;
        let tm_seconds = buffer.read_u64::<LittleEndian>()?;

        Ok(Self {
            high_unique_id,
            low_unique_id,
            tm_milliseconds,
            tm_seconds,
        })
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(self.high_unique_id)?;
        buffer.write_u32::<LittleEndian>(self.low_unique_id)?;
        buffer.write_u64::<LittleEndian>(self.tm_milliseconds)?;
        buffer.write_u64::<LittleEndian>(self.tm_seconds)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        24
    }
}

#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
#[repr(u16)]
enum SurfaceCommandType {
    SetSurfaceBits = 0x01,
    FrameMarker = 0x04,
    StreamSurfaceBits = 0x06,
}

impl<'a> From<&SurfaceCommand<'a>> for SurfaceCommandType {
    fn from(command: &SurfaceCommand<'_>) -> Self {
        match command {
            SurfaceCommand::SetSurfaceBits(_) => Self::SetSurfaceBits,
            SurfaceCommand::FrameMarker(_) => Self::FrameMarker,
            SurfaceCommand::StreamSurfaceBits(_) => Self::StreamSurfaceBits,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
#[repr(u16)]
pub enum FrameAction {
    Begin = 0x00,
    End = 0x01,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    struct BitmapDataFlags: u8 {
        const COMPRESSED_BITMAP_HEADER_PRESENT = 0x01;
    }
}

#[derive(Debug, Error)]
pub enum SurfaceCommandsError {
    #[error("IO error")]
    IOError(#[from] io::Error),
    #[error("Invalid Surface Command type: {0}")]
    InvalidSurfaceCommandType(u16),
    #[error("Invalid Frame Marker action: {0}")]
    InvalidFrameAction(u16),
    #[error("Input buffer is shorter than the data length: {actual} < {expected}")]
    InvalidDataLength { expected: usize, actual: usize },
}
