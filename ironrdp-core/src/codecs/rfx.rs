mod data_messages;
mod header_messages;

#[cfg(test)]
mod tests;

use std::io;

use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive as _, ToPrimitive as _};
use thiserror::Error;

pub use self::data_messages::{
    ContextPdu, EntropyAlgorithm, FrameBeginPdu, FrameEndPdu, OperatingMode, Quant, RegionPdu, RfxRectangle, Tile,
    TileSetPdu,
};
pub use self::header_messages::{Channel, ChannelsPdu, CodecVersionsPdu, SyncPdu};
use crate::{PduBufferParsing, PduParsing};

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
        let ty = buffer.read_u16::<LittleEndian>()?;
        let ty = BlockType::from_u16(ty).ok_or(RfxError::InvalidBlockType(ty))?;
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

impl PduParsing for FrameAcknowledgePdu {
    type Error = io::Error;

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

#[derive(Debug, Error)]
pub enum RfxError {
    #[error("IO error")]
    IoError(#[from] io::Error),
    #[error("Got invalid block type: {0}")]
    InvalidBlockType(u16),
    #[error("Got unexpected Block type: expected ({expected:?}) != actual({actual:?})")]
    UnexpectedBlockType { expected: BlockType, actual: BlockType },
    #[error("Got unexpected Block type ({0:?}) while was expected header message")]
    InvalidHeaderBlockType(BlockType),
    #[error("Got invalid block length: {0}")]
    InvalidBlockLength(usize),
    #[error("Got invalid Sync magic number: {0}")]
    InvalidMagicNumber(u32),
    #[error("Got invalid Sync version: {0}")]
    InvalidSyncVersion(u16),
    #[error("Got invalid codecs number: {0}")]
    InvalidCodecsNumber(u8),
    #[error("Got invalid codec ID: {0}")]
    InvalidCodecId(u8),
    #[error("Got invalid codec version: {0}")]
    InvalidCodecVersion(u16),
    #[error("Got invalid channel ID: {0}")]
    InvalidChannelId(u8),
    #[error("Got invalid context ID: {0}")]
    InvalidContextId(u8),
    #[error("Got invalid context tile size: {0}")]
    InvalidTileSize(u16),
    #[error("Got invalid conversion transform: {0}")]
    InvalidColorConversionTransform(u16),
    #[error("Got invalid DWT: {0}")]
    InvalidDwt(u16),
    #[error("Got invalid entropy algorithm: {0}")]
    InvalidEntropyAlgorithm(u16),
    #[error("Got invalid quantization type: {0}")]
    InvalidQuantizationType(u16),
    #[error("Input buffer is shorter than the data length: {actual} < {expected}")]
    InvalidDataLength { expected: usize, actual: usize },
    #[error("Got invalid Region LRF: {0}")]
    InvalidLrf(bool),
    #[error("Got invalid Region type: {0}")]
    InvalidRegionType(u16),
    #[error("Got invalid number of tilesets: {0}")]
    InvalidNumberOfTilesets(u16),
    #[error("Got invalid ID of context: {0}")]
    InvalidIdOfContext(u16),
    #[error("Got invalid TileSet subtype: {0}")]
    InvalidSubtype(u16),
    #[error("Got invalid IT flag of TileSet: {0}")]
    InvalidItFlag(bool),
}
