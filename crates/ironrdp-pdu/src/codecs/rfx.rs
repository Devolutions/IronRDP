mod data_messages;
mod header_messages;

use std::io;

use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive as _, ToPrimitive as _};
use thiserror::Error;

use crate::{Decode, DecodeResult, Encode, EncodeResult, PduBufferParsing};
use ironrdp_core::{ensure_fixed_part_size, ReadCursor, WriteCursor};

#[rustfmt::skip]
pub use self::data_messages::{
    ContextPdu, EntropyAlgorithm, FrameBeginPdu, FrameEndPdu, OperatingMode, Quant, RegionPdu, RfxRectangle, Tile,
    TileSetPdu,
};
pub use self::header_messages::{
    ChannelsPdu, CodecVersionsPdu, RfxChannel, RfxChannelHeight, RfxChannelWidth, SyncPdu,
};

const BLOCK_HEADER_SIZE: usize = 6;
const CODEC_CHANNEL_HEADER_SIZE: usize = 2;

const CODEC_ID: u8 = 1;
const CHANNEL_ID_FOR_CONTEXT: u8 = 0xFF;
const CHANNEL_ID_FOR_OTHER_VALUES: u8 = 0x00;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Headers {
    CodecVersions(CodecVersionsPdu),
    Channels(ChannelsPdu),
    Context(ContextPdu),
}

impl<'a> PduBufferParsing<'a> for Headers {
    type Error = RfxError;

    fn from_buffer_consume(buffer: &mut &'a [u8]) -> Result<Self, Self::Error> {
        let mut temp = *buffer;
        let ty = temp.read_u16::<LittleEndian>()?;
        let ty = BlockType::from_u16(ty).ok_or(RfxError::InvalidBlockType(ty))?;

        match ty {
            BlockType::CodecVersions => Ok(Self::CodecVersions(CodecVersionsPdu::from_buffer_consume(buffer)?)),
            BlockType::Channels => Ok(Self::Channels(ChannelsPdu::from_buffer_consume(buffer)?)),
            BlockType::Context => Ok(Self::Context(ContextPdu::from_buffer_consume(buffer)?)),
            _ => Err(RfxError::InvalidHeaderBlockType(ty)),
        }
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        match self {
            Self::CodecVersions(pdu) => pdu.to_buffer_consume(buffer),
            Self::Channels(pdu) => pdu.to_buffer_consume(buffer),
            Self::Context(pdu) => pdu.to_buffer_consume(buffer),
        }
    }

    fn buffer_length(&self) -> usize {
        match self {
            Self::CodecVersions(pdu) => pdu.buffer_length(),
            Self::Channels(pdu) => pdu.buffer_length(),
            Self::Context(pdu) => pdu.buffer_length(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockHeader {
    pub ty: BlockType,
    pub data_length: usize,
}

impl BlockHeader {
    pub fn from_buffer_consume(buffer: &mut &[u8]) -> Result<Self, RfxError> {
        let ty = BlockType::from_buffer(buffer)?;
        Self::from_buffer_consume_with_type(buffer, ty)
    }

    fn from_buffer_consume_with_type(buffer: &mut &[u8], ty: BlockType) -> Result<Self, RfxError> {
        let block_length = buffer.read_u32::<LittleEndian>()? as usize;

        let block_length_without_header = block_length
            .checked_sub(BLOCK_HEADER_SIZE)
            .ok_or(RfxError::InvalidBlockLength(block_length))?;

        if buffer.len() < block_length_without_header {
            return Err(RfxError::InvalidDataLength {
                expected: block_length_without_header,
                actual: buffer.len(),
            });
        }

        let data_length = block_length
            .checked_sub(headers_length(ty))
            .ok_or(RfxError::InvalidBlockLength(block_length))?;

        Ok(Self { ty, data_length })
    }

    fn from_buffer_consume_with_expected_type(buffer: &mut &[u8], expected_type: BlockType) -> Result<Self, RfxError> {
        let ty = BlockType::from_buffer(buffer)?;
        if ty != expected_type {
            return Err(RfxError::UnexpectedBlockType {
                expected: expected_type,
                actual: ty,
            });
        }

        Self::from_buffer_consume_with_type(buffer, ty)
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), RfxError> {
        buffer.write_u16::<LittleEndian>(self.ty.to_u16().unwrap())?;
        buffer.write_u32::<LittleEndian>((headers_length(self.ty) + self.data_length) as u32)?;

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecChannelHeader;

impl CodecChannelHeader {
    fn from_buffer_consume_with_type(buffer: &mut &[u8], ty: BlockType) -> Result<Self, RfxError> {
        let codec_id = buffer.read_u8()?;
        if codec_id != CODEC_ID {
            return Err(RfxError::InvalidCodecId(codec_id));
        }

        let channel_id = buffer.read_u8()?;
        let expected_channel_id = match ty {
            BlockType::Context => CHANNEL_ID_FOR_CONTEXT,
            _ => CHANNEL_ID_FOR_OTHER_VALUES,
        };
        if channel_id != expected_channel_id {
            return Err(RfxError::InvalidChannelId(channel_id));
        }

        Ok(Self)
    }

    #[allow(clippy::unused_self)] // for symmetry
    fn to_buffer_consume_with_type(&self, buffer: &mut &mut [u8], ty: BlockType) -> Result<(), RfxError> {
        buffer.write_u8(CODEC_ID)?;

        let channel_id = match ty {
            BlockType::Context => CHANNEL_ID_FOR_CONTEXT,
            _ => CHANNEL_ID_FOR_OTHER_VALUES,
        };
        buffer.write_u8(channel_id)?;

        Ok(())
    }
}

fn headers_length(ty: BlockType) -> usize {
    BLOCK_HEADER_SIZE
        + match ty {
            BlockType::Context
            | BlockType::FrameBegin
            | BlockType::FrameEnd
            | BlockType::Region
            | BlockType::Extension => CODEC_CHANNEL_HEADER_SIZE,
            _ => 0,
        }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameAcknowledgePdu {
    pub frame_id: u32,
}

impl FrameAcknowledgePdu {
    const NAME: &'static str = "FrameAcknowledgePdu";

    const FIXED_PART_SIZE: usize = 4 /* frameId */;
}

impl Encode for FrameAcknowledgePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
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

impl<'de> Decode<'de> for FrameAcknowledgePdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let frame_id = src.read_u32();

        Ok(Self { frame_id })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
#[repr(u16)]
pub enum BlockType {
    Tile = 0xCAC3,
    Capabilities = 0xCBC0,
    CapabilitySet = 0xCBC1,
    Sync = 0xCCC0,
    CodecVersions = 0xCCC1,
    Channels = 0xCCC2,
    Context = 0xCCC3,
    FrameBegin = 0xCCC4,
    FrameEnd = 0xCCC5,
    Region = 0xCCC6,
    Extension = 0xCCC7,
}

impl BlockType {
    fn from_buffer(buffer: &mut &[u8]) -> Result<Self, RfxError> {
        let ty = buffer.read_u16::<LittleEndian>()?;
        let ty = BlockType::from_u16(ty).ok_or(RfxError::InvalidBlockType(ty))?;
        Ok(ty)
    }
}

#[derive(Debug, Error)]
pub enum RfxError {
    #[error("IO error")]
    IoError(#[from] io::Error),
    #[error("got invalid block type: {0}")]
    InvalidBlockType(u16),
    #[error("got unexpected Block type: expected ({expected:?}) != actual({actual:?})")]
    UnexpectedBlockType { expected: BlockType, actual: BlockType },
    #[error("got unexpected Block type ({0:?}) while was expected header message")]
    InvalidHeaderBlockType(BlockType),
    #[error("got invalid block length: {0}")]
    InvalidBlockLength(usize),
    #[error("got invalid Sync magic number: {0}")]
    InvalidMagicNumber(u32),
    #[error("got invalid Sync version: {0}")]
    InvalidSyncVersion(u16),
    #[error("got invalid codecs number: {0}")]
    InvalidCodecsNumber(u8),
    #[error("got invalid codec ID: {0}")]
    InvalidCodecId(u8),
    #[error("got invalid codec version: {0}")]
    InvalidCodecVersion(u16),
    #[error("got invalid channel ID: {0}")]
    InvalidChannelId(u8),
    #[error("got invalid context ID: {0}")]
    InvalidContextId(u8),
    #[error("got invalid context tile size: {0}")]
    InvalidTileSize(u16),
    #[error("got invalid conversion transform: {0}")]
    InvalidColorConversionTransform(u16),
    #[error("got invalid DWT: {0}")]
    InvalidDwt(u16),
    #[error("got invalid entropy algorithm: {0}")]
    InvalidEntropyAlgorithm(u16),
    #[error("got invalid quantization type: {0}")]
    InvalidQuantizationType(u16),
    #[error("input buffer is shorter than the data length: {actual} < {expected}")]
    InvalidDataLength { expected: usize, actual: usize },
    #[error("got invalid Region LRF: {0}")]
    InvalidLrf(bool),
    #[error("got invalid Region type: {0}")]
    InvalidRegionType(u16),
    #[error("got invalid number of tilesets: {0}")]
    InvalidNumberOfTilesets(u16),
    #[error("got invalid ID of context: {0}")]
    InvalidIdOfContext(u16),
    #[error("got invalid TileSet subtype: {0}")]
    InvalidSubtype(u16),
    #[error("got invalid IT flag of TileSet: {0}")]
    InvalidItFlag(bool),
    #[error("got invalid channel width: {0}")]
    InvalidChannelWidth(i16),
    #[error("got invalid channel height: {0}")]
    InvalidChannelHeight(i16),
}
