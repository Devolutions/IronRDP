mod data_messages;
mod header_messages;
#[cfg(test)]
mod tests;

pub use self::{
    data_messages::{
        ContextPdu, EntropyAlgorithm, FrameBeginPdu, FrameEndPdu, OperatingMode, Quant, RegionPdu,
        Tile, TileSetPdu,
    },
    header_messages::{Channel, ChannelsPdu, CodecVersionsPdu, SyncPdu},
};

use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::impl_from_error;

const BLOCK_HEADER_SIZE: usize = 6;
const CODEC_CHANNEL_HEADER_SIZE: usize = 8;

const CODEC_ID: u8 = 1;
const CHANNEL_ID_FOR_CONTEXT: u8 = 0xFF;
const CHANNEL_ID_FOR_OTHER_VALUES: u8 = 0x00;

#[derive(Debug, Clone, PartialEq)]
struct BlockHeader {
    pub ty: BlockType,
    pub data_length: usize,
}

impl BlockHeader {
    fn from_buffer_with_type(
        mut buffer: &[u8],
        expected_type: BlockType,
    ) -> Result<Self, RfxError> {
        let ty = BlockType::from_u16(buffer.read_u16::<LittleEndian>()?)
            .ok_or(RfxError::InvalidBlockType)?;
        if ty != expected_type {
            return Err(RfxError::UnexpectedBlockType {
                expected: expected_type,
                actual: ty,
            });
        }

        let block_length = buffer.read_u32::<LittleEndian>()? as usize;

        let data_length = block_length
            .checked_sub(BLOCK_HEADER_SIZE)
            .ok_or(RfxError::InvalidBlockLength(block_length))?;

        if buffer.len() < data_length {
            return Err(RfxError::InvalidDataLength {
                expected: data_length,
                actual: buffer.len(),
            });
        }

        Ok(Self { ty, data_length })
    }

    fn to_buffer(&self, mut buffer: &mut [u8]) -> Result<(), RfxError> {
        buffer.write_u16::<LittleEndian>(self.ty.to_u16().unwrap())?;
        buffer.write_u32::<LittleEndian>((self.buffer_length() + self.data_length) as u32)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        BLOCK_HEADER_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
struct CodecChannelHeader {
    pub ty: CodecChannelType,
    pub data_length: usize,
}

impl CodecChannelHeader {
    fn from_buffer_with_type(
        mut buffer: &[u8],
        expected_type: CodecChannelType,
    ) -> Result<Self, RfxError> {
        let ty = CodecChannelType::from_u16(buffer.read_u16::<LittleEndian>()?)
            .ok_or(RfxError::InvalidCodecChannelType)?;
        if ty != expected_type {
            return Err(RfxError::UnexpectedCodecChannelType {
                expected: expected_type,
                actual: ty,
            });
        }

        let block_length = buffer.read_u32::<LittleEndian>()? as usize;

        let data_length = block_length
            .checked_sub(CODEC_CHANNEL_HEADER_SIZE)
            .ok_or(RfxError::InvalidBlockLength(block_length))?;

        let codec_id = buffer.read_u8()?;
        if codec_id != CODEC_ID {
            return Err(RfxError::InvalidCodecId(codec_id));
        }

        let channel_id = buffer.read_u8()?;
        let expected_channel_id = match ty {
            CodecChannelType::Context => CHANNEL_ID_FOR_CONTEXT,
            _ => CHANNEL_ID_FOR_OTHER_VALUES,
        };
        if channel_id != expected_channel_id {
            return Err(RfxError::InvalidChannelId(channel_id));
        }

        if buffer.len() < data_length {
            return Err(RfxError::InvalidDataLength {
                expected: data_length,
                actual: buffer.len(),
            });
        }

        Ok(Self { ty, data_length })
    }

    fn to_buffer(&self, mut buffer: &mut [u8]) -> Result<(), RfxError> {
        buffer.write_u16::<LittleEndian>(self.ty.to_u16().unwrap())?;
        buffer.write_u32::<LittleEndian>((self.buffer_length() + self.data_length) as u32)?;
        buffer.write_u8(CODEC_ID)?;

        let channel_id = match self.ty {
            CodecChannelType::Context => CHANNEL_ID_FOR_CONTEXT,
            _ => CHANNEL_ID_FOR_OTHER_VALUES,
        };
        buffer.write_u8(channel_id)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        CODEC_CHANNEL_HEADER_SIZE
    }
}

#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
#[repr(u16)]
pub enum BlockType {
    Tile = 0xCAC3,
    Capabilities = 0xCBC0,
    CapabilitySet = 0xCBC1,
    Sync = 0xCCC0,
    CodecVersions = 0xCCC1,
    Channels = 0xCCC2,
}

#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
#[repr(u16)]
pub enum CodecChannelType {
    Context = 0xCCC3,
    FrameBegin = 0xCCC4,
    FrameEnd = 0xCCC5,
    Region = 0xCCC6,
    Extension = 0xCCC7,
}

#[derive(Debug, Fail)]
pub enum RfxError {
    #[fail(display = "IO error: {}", _0)]
    IoError(#[fail(cause)] io::Error),
    #[fail(display = "Got invalid block type")]
    InvalidBlockType,
    #[fail(
        display = "Got unexpected Block type: expected ({:?}) != actual({:?})",
        expected, actual
    )]
    UnexpectedBlockType {
        expected: BlockType,
        actual: BlockType,
    },
    #[fail(display = "Got invalid block length: {}", _0)]
    InvalidBlockLength(usize),
    #[fail(display = "Got invalid Sync magic number: {}", _0)]
    InvalidMagicNumber(u32),
    #[fail(display = "Got invalid Sync version: {}", _0)]
    InvalidSyncVersion(u16),
    #[fail(display = "Got invalid codecs number: {}", _0)]
    InvalidCodecsNumber(u8),
    #[fail(display = "Got invalid codec ID: {}", _0)]
    InvalidCodecId(u8),
    #[fail(display = "Got invalid codec version: {}", _0)]
    InvalidCodecVersion(u16),
    #[fail(display = "Got invalid channel ID: {}", _0)]
    InvalidChannelId(u8),
    #[fail(display = "Got invalid context ID: {}", _0)]
    InvalidContextId(u8),
    #[fail(display = "Got invalid context tile size: {}", _0)]
    InvalidTileSize(u16),
    #[fail(display = "Got invalid conversion transform: {}", _0)]
    InvalidColorConversionTransform(u16),
    #[fail(display = "Got invalid DWT: {}", _0)]
    InvalidDwt(u16),
    #[fail(display = "Got invalid entropy algorithm: {}", _0)]
    InvalidEntropyAlgorithm(u16),
    #[fail(display = "Got invalid quantization type: {}", _0)]
    InvalidQuantizationType(u16),
    #[fail(display = "Got invalid codec channel type")]
    InvalidCodecChannelType,
    #[fail(
        display = "Got unexpected codec channel type: expected ({:?}) != actual ({:?})",
        expected, actual
    )]
    UnexpectedCodecChannelType {
        expected: CodecChannelType,
        actual: CodecChannelType,
    },
    #[fail(
        display = "Input buffer is shorter then the data length: {} < {}",
        actual, expected
    )]
    InvalidDataLength { expected: usize, actual: usize },
    #[fail(display = "Got invalid Region LRF: {}", _0)]
    InvalidLrf(bool),
    #[fail(display = "Got invalid Region type: {}", _0)]
    InvalidRegionType(u16),
    #[fail(display = "Got invalid number of tilesets: {}", _0)]
    InvalidNumberOfTilesets(u16),
    #[fail(display = "Got invalid ID of context: {}", _0)]
    InvalidIdOfContext(u16),
    #[fail(display = "Got invalid TileSet subtype: {}", _0)]
    InvalidSubtype(u16),
    #[fail(display = "Got invalid IT flag of TileSet: {}", _0)]
    InvalidItFlag(bool),
}

impl_from_error!(io::Error, RfxError, RfxError::IoError);
