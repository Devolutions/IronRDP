#[cfg(test)]
mod tests;

use bitflags::bitflags;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive as _, ToPrimitive as _};

use crate::cursor::{ReadCursor, WriteCursor};
use crate::geometry::ExclusiveRectangle;
use crate::{PduDecode, PduEncode, PduResult};

pub const SURFACE_COMMAND_HEADER_SIZE: usize = 2;

// TS_SURFCMD
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurfaceCommand<'a> {
    SetSurfaceBits(SurfaceBitsPdu<'a>),
    FrameMarker(FrameMarkerPdu),
    StreamSurfaceBits(SurfaceBitsPdu<'a>),
}

impl SurfaceCommand<'_> {
    const NAME: &str = "TS_SURFCMD";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u16>();
}

impl<'en> PduEncode for SurfaceCommand<'en> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        let cmd_type = SurfaceCommandType::from(self);
        dst.write_u16(cmd_type.to_u16().unwrap());

        match self {
            Self::SetSurfaceBits(pdu) | Self::StreamSurfaceBits(pdu) => pdu.encode(dst),
            Self::FrameMarker(pdu) => pdu.encode(dst),
        }?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        SURFACE_COMMAND_HEADER_SIZE
            + match self {
                Self::SetSurfaceBits(pdu) | Self::StreamSurfaceBits(pdu) => pdu.size(),
                Self::FrameMarker(pdu) => pdu.size(),
            }
    }
}

impl<'de> PduDecode<'de> for SurfaceCommand<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let cmd_type = src.read_u16();
        let cmd_type = SurfaceCommandType::from_u16(cmd_type)
            .ok_or_else(|| invalid_message_err!("cmdType", "invalid surface command"))?;

        match cmd_type {
            SurfaceCommandType::SetSurfaceBits => Ok(Self::SetSurfaceBits(SurfaceBitsPdu::decode(src)?)),
            SurfaceCommandType::FrameMarker => Ok(Self::FrameMarker(FrameMarkerPdu::decode(src)?)),
            SurfaceCommandType::StreamSurfaceBits => Ok(Self::StreamSurfaceBits(SurfaceBitsPdu::decode(src)?)),
        }
    }
}

// TS_SURFCMD_STREAM_SURF_BITS and TS_SURFCMD_SET_SURF_BITS
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceBitsPdu<'a> {
    pub destination: ExclusiveRectangle,
    pub extended_bitmap_data: ExtendedBitmapDataPdu<'a>,
}

impl SurfaceBitsPdu<'_> {
    const NAME: &str = "TS_SURFCMD_x_SURFACE_BITS_PDU";
}

impl<'en> PduEncode for SurfaceBitsPdu<'en> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        self.destination.encode(dst)?;
        self.extended_bitmap_data.encode(dst)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.destination.size() + self.extended_bitmap_data.size()
    }
}

impl<'de> PduDecode<'de> for SurfaceBitsPdu<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let destination = ExclusiveRectangle::decode(src)?;
        let extended_bitmap_data = ExtendedBitmapDataPdu::decode(src)?;

        Ok(Self {
            destination,
            extended_bitmap_data,
        })
    }
}

// TS_FRAME_MARKER
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameMarkerPdu {
    pub frame_action: FrameAction,
    pub frame_id: Option<u32>,
}

impl FrameMarkerPdu {
    const NAME: &str = "TS_FRAME_MARKER_PDU";
    const FIXED_PART_SIZE: usize = core::mem::size_of::<u16>() + core::mem::size_of::<u32>();
}

impl PduEncode for FrameMarkerPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.frame_action as u16);
        dst.write_u32(self.frame_id.unwrap_or(0));

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for FrameMarkerPdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_size!(in: src, size: core::mem::size_of::<u16>());

        let frame_action = src.read_u16();

        let frame_action = FrameAction::from_u16(frame_action)
            .ok_or_else(|| invalid_message_err!("frameAction", "invalid frame action"))?;

        let frame_id = if src.is_empty() {
            // Sometimes Windows 10 RDP server sends not complete FrameMarker PDU (without frame ID),
            // so we made frame ID field as optional (not officially)

            None
        } else {
            ensure_size!(in: src, size: core::mem::size_of::<u32>());
            Some(src.read_u32())
        };

        Ok(Self { frame_action, frame_id })
    }
}

// TS_BITMAP_DATA_EX
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedBitmapDataPdu<'a> {
    pub bpp: u8,
    pub codec_id: u8,
    pub width: u16,
    pub height: u16,
    pub header: Option<BitmapDataHeader>,
    pub data: &'a [u8],
}

impl ExtendedBitmapDataPdu<'_> {
    const NAME: &str = "TS_BITMAP_DATA_EX";
    const FIXED_PART_SIZE: usize =
        core::mem::size_of::<u8>() * 4 + core::mem::size_of::<u16>() * 2 + core::mem::size_of::<u32>();
}

impl<'en> PduEncode for ExtendedBitmapDataPdu<'en> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        if self.data.len() > u32::MAX as usize {
            return Err(invalid_message_err!("bitmapDataLength", "bitmap data is too big"));
        }

        dst.write_u8(self.bpp);
        let flags = if self.header.is_some() {
            BitmapDataFlags::COMPRESSED_BITMAP_HEADER_PRESENT
        } else {
            BitmapDataFlags::empty()
        };
        dst.write_u8(flags.bits());
        dst.write_u8(0); // reserved
        dst.write_u8(self.codec_id);
        dst.write_u16(self.width);
        dst.write_u16(self.height);
        dst.write_u32(self.data.len() as u32);
        if let Some(header) = &self.header {
            header.encode(dst)?;
        }
        dst.write_slice(self.data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.header.as_ref().map_or(0, |h| h.size()) + self.data.len()
    }
}

impl<'de> PduDecode<'de> for ExtendedBitmapDataPdu<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let bpp = src.read_u8();
        let flags = BitmapDataFlags::from_bits_truncate(src.read_u8());
        let _reserved = src.read_u8();
        let codec_id = src.read_u8();
        let width = src.read_u16();
        let height = src.read_u16();
        let data_length = src.read_u32() as usize;

        let expected_remaining_size = if flags.contains(BitmapDataFlags::COMPRESSED_BITMAP_HEADER_PRESENT) {
            data_length + BitmapDataHeader::ENCODED_SIZE
        } else {
            data_length
        };

        ensure_size!(in: src, size: expected_remaining_size);

        let header = if flags.contains(BitmapDataFlags::COMPRESSED_BITMAP_HEADER_PRESENT) {
            Some(BitmapDataHeader::decode(src)?)
        } else {
            None
        };

        let data = src.read_slice(data_length);

        Ok(Self {
            bpp,
            codec_id,
            width,
            height,
            header,
            data,
        })
    }
}

// TS_COMPRESSED_BITMAP_HEADER_EX
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitmapDataHeader {
    pub high_unique_id: u32,
    pub low_unique_id: u32,
    pub tm_milliseconds: u64,
    pub tm_seconds: u64,
}

impl BitmapDataHeader {
    const NAME: &str = "TS_COMPRESSED_BITMAP_HEADER_EX";
    const FIXED_PART_SIZE: usize = core::mem::size_of::<u32>() * 2 + core::mem::size_of::<u64>() * 4;

    pub const ENCODED_SIZE: usize = Self::FIXED_PART_SIZE;
}

impl PduEncode for BitmapDataHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.high_unique_id);
        dst.write_u32(self.low_unique_id);
        dst.write_u64(self.tm_milliseconds);
        dst.write_u64(self.tm_seconds);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for BitmapDataHeader {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let high_unique_id = src.read_u32();
        let low_unique_id = src.read_u32();
        let tm_milliseconds = src.read_u64();
        let tm_seconds = src.read_u64();

        Ok(Self {
            high_unique_id,
            low_unique_id,
            tm_milliseconds,
            tm_seconds,
        })
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

/*
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
*/
