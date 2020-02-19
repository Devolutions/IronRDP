use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::{BlockHeader, BlockType, RfxError, BLOCK_HEADER_SIZE};
use crate::{split_to, PduBufferParsing};

const SYNC_MAGIC: u32 = 0xCACC_ACCA;
const SYNC_VERSION: u16 = 0x0100;
const CODECS_NUMBER: u8 = 1;
const CODEC_ID: u8 = 1;
const CODEC_VERSION: u16 = 0x0100;
const CHANNEL_ID: u8 = 0;

const CHANNEL_SIZE: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct SyncPdu;

impl PduBufferParsing for SyncPdu {
    type Error = RfxError;

    fn from_buffer_consume(buffer: &mut &[u8]) -> Result<Self, Self::Error> {
        let header = BlockHeader::from_buffer_consume_with_type(buffer, BlockType::Sync)?;
        let mut buffer = split_to!(*buffer, header.data_length);

        let magic = buffer.read_u32::<LittleEndian>()?;
        if magic != SYNC_MAGIC {
            return Err(RfxError::InvalidMagicNumber(magic));
        }
        let version = buffer.read_u16::<LittleEndian>()?;
        if version != SYNC_VERSION {
            Err(RfxError::InvalidSyncVersion(version))
        } else {
            Ok(Self)
        }
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        let header = BlockHeader {
            ty: BlockType::Sync,
            data_length: self.buffer_length() - BLOCK_HEADER_SIZE,
        };

        header.to_buffer_consume(buffer)?;
        buffer.write_u32::<LittleEndian>(SYNC_MAGIC)?;
        buffer.write_u16::<LittleEndian>(SYNC_VERSION)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        BLOCK_HEADER_SIZE + 6
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodecVersionsPdu;

impl PduBufferParsing for CodecVersionsPdu {
    type Error = RfxError;

    fn from_buffer_consume(buffer: &mut &[u8]) -> Result<Self, Self::Error> {
        let header = BlockHeader::from_buffer_consume_with_type(buffer, BlockType::CodecVersions)?;
        let mut buffer = split_to!(*buffer, header.data_length);

        let codecs_number = buffer.read_u8()?;
        if codecs_number != CODECS_NUMBER {
            return Err(RfxError::InvalidCodecsNumber(codecs_number));
        }

        let _codec_version = CodecVersion::from_buffer(buffer)?;

        Ok(Self)
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        let header = BlockHeader {
            ty: BlockType::CodecVersions,
            data_length: self.buffer_length() - BLOCK_HEADER_SIZE,
        };

        header.to_buffer_consume(buffer)?;
        buffer.write_u8(CODECS_NUMBER)?;

        CodecVersion.to_buffer_consume(buffer)
    }

    fn buffer_length(&self) -> usize {
        BLOCK_HEADER_SIZE + 1 + CodecVersion.buffer_length()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelsPdu(pub Vec<Channel>);

impl PduBufferParsing for ChannelsPdu {
    type Error = RfxError;

    fn from_buffer_consume(buffer: &mut &[u8]) -> Result<Self, Self::Error> {
        let header = BlockHeader::from_buffer_consume_with_type(buffer, BlockType::Channels)?;
        let mut buffer = split_to!(*buffer, header.data_length);

        let channels_number = buffer.read_u8()? as usize;

        if buffer.len() < channels_number * CHANNEL_SIZE {
            return Err(RfxError::InvalidDataLength {
                expected: channels_number * CHANNEL_SIZE,
                actual: buffer.len(),
            });
        }

        let channels = (0..channels_number)
            .map(|_| Channel::from_buffer_consume(&mut buffer))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self(channels))
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        let header = BlockHeader {
            ty: BlockType::Channels,
            data_length: self.buffer_length() - BLOCK_HEADER_SIZE,
        };

        header.to_buffer_consume(buffer)?;
        buffer.write_u8(self.0.len() as u8)?;

        for channel in self.0.iter() {
            channel.to_buffer_consume(buffer)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        BLOCK_HEADER_SIZE
            + 1
            + self
                .0
                .iter()
                .map(PduBufferParsing::buffer_length)
                .sum::<usize>()
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Channel {
    pub width: i16,
    pub height: i16,
}

impl PduBufferParsing for Channel {
    type Error = RfxError;

    fn from_buffer_consume(buffer: &mut &[u8]) -> Result<Self, Self::Error> {
        let id = buffer.read_u8()?;
        if id != CHANNEL_ID {
            return Err(RfxError::InvalidChannelId(id));
        }

        let width = buffer.read_i16::<LittleEndian>()?;
        let height = buffer.read_i16::<LittleEndian>()?;

        Ok(Self { width, height })
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        buffer.write_u8(CHANNEL_ID)?;
        buffer.write_i16::<LittleEndian>(self.width)?;
        buffer.write_i16::<LittleEndian>(self.height)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        CHANNEL_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
struct CodecVersion;

impl PduBufferParsing for CodecVersion {
    type Error = RfxError;

    fn from_buffer_consume(buffer: &mut &[u8]) -> Result<Self, Self::Error> {
        let id = buffer.read_u8()?;
        if id != CODEC_ID {
            return Err(RfxError::InvalidCodecId(id));
        }

        let version = buffer.read_u16::<LittleEndian>()?;
        if version != CODEC_VERSION {
            Err(RfxError::InvalidCodecVersion(version))
        } else {
            Ok(Self)
        }
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        buffer.write_u8(CODEC_ID)?;
        buffer.write_u16::<LittleEndian>(CODEC_VERSION)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        3
    }
}
