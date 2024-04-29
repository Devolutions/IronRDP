use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::{BlockHeader, BlockType, RfxError, BLOCK_HEADER_SIZE};
use crate::utils::SplitTo;
use crate::PduBufferParsing;

const SYNC_MAGIC: u32 = 0xCACC_ACCA;
const SYNC_VERSION: u16 = 0x0100;
const CODECS_NUMBER: u8 = 1;
const CODEC_ID: u8 = 1;
const CODEC_VERSION: u16 = 0x0100;
const CHANNEL_ID: u8 = 0;

const CHANNEL_SIZE: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPdu;

impl SyncPdu {
    pub fn from_buffer_consume_with_header(buffer: &mut &[u8], header: BlockHeader) -> Result<Self, RfxError> {
        let mut buffer = buffer.split_to(header.data_length);

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
}

impl PduBufferParsing<'_> for SyncPdu {
    type Error = RfxError;

    fn from_buffer_consume(buffer: &mut &[u8]) -> Result<Self, Self::Error> {
        let header = BlockHeader::from_buffer_consume_with_expected_type(buffer, BlockType::Sync)?;
        Self::from_buffer_consume_with_header(buffer, header)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecVersionsPdu;

impl CodecVersionsPdu {
    pub fn from_buffer_consume_with_header(buffer: &mut &[u8], header: BlockHeader) -> Result<Self, RfxError> {
        let mut buffer = buffer.split_to(header.data_length);

        let codecs_number = buffer.read_u8()?;
        if codecs_number != CODECS_NUMBER {
            return Err(RfxError::InvalidCodecsNumber(codecs_number));
        }

        let _codec_version = CodecVersion::from_buffer(buffer)?;

        Ok(Self)
    }
}

impl PduBufferParsing<'_> for CodecVersionsPdu {
    type Error = RfxError;

    fn from_buffer_consume(buffer: &mut &[u8]) -> Result<Self, Self::Error> {
        let header = BlockHeader::from_buffer_consume_with_expected_type(buffer, BlockType::CodecVersions)?;

        Self::from_buffer_consume_with_header(buffer, header)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelsPdu(pub Vec<RfxChannel>);

impl ChannelsPdu {
    pub fn from_buffer_consume_with_header(buffer: &mut &[u8], header: BlockHeader) -> Result<Self, RfxError> {
        let mut buffer = buffer.split_to(header.data_length);

        let channels_number = usize::from(buffer.read_u8()?);

        if buffer.len() < channels_number * CHANNEL_SIZE {
            return Err(RfxError::InvalidDataLength {
                expected: channels_number * CHANNEL_SIZE,
                actual: buffer.len(),
            });
        }

        let channels = (0..channels_number)
            .map(|_| RfxChannel::from_buffer_consume(&mut buffer))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self(channels))
    }
}

impl PduBufferParsing<'_> for ChannelsPdu {
    type Error = RfxError;

    fn from_buffer_consume(buffer: &mut &[u8]) -> Result<Self, Self::Error> {
        let header = BlockHeader::from_buffer_consume_with_expected_type(buffer, BlockType::Channels)?;

        Self::from_buffer_consume_with_header(buffer, header)
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
        BLOCK_HEADER_SIZE + 1 + self.0.iter().map(PduBufferParsing::buffer_length).sum::<usize>()
    }
}

/// TS_RFX_CHANNELT
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct RfxChannel {
    pub width: RfxChannelWidth,
    pub height: RfxChannelHeight,
}

/// A 16-bit, signed integer within the range of 1 to 4096
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct RfxChannelWidth(i16);

impl RfxChannelWidth {
    pub fn new(value: i16) -> Self {
        Self(value)
    }

    pub fn as_u16(self) -> u16 {
        u16::try_from(self.0).expect("integer within the range of 1 to 4096")
    }

    pub fn get(self) -> i16 {
        self.0
    }
}

/// A 16-bit, signed integer within the range of 1 to 2048
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct RfxChannelHeight(i16);

impl RfxChannelHeight {
    pub fn new(value: i16) -> Self {
        Self(value)
    }

    pub fn as_u16(self) -> u16 {
        u16::try_from(self.0).expect("integer within the range of 1 to 2048")
    }

    pub fn get(self) -> i16 {
        self.0
    }
}

impl PduBufferParsing<'_> for RfxChannel {
    type Error = RfxError;

    fn from_buffer_consume(buffer: &mut &[u8]) -> Result<Self, Self::Error> {
        let id = buffer.read_u8()?;
        if id != CHANNEL_ID {
            return Err(RfxError::InvalidChannelId(id));
        }

        let width = buffer.read_i16::<LittleEndian>()?;
        let width = RfxChannelWidth::new(width);

        let height = buffer.read_i16::<LittleEndian>()?;
        let height = RfxChannelHeight::new(height);

        Ok(Self { width, height })
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        buffer.write_u8(CHANNEL_ID)?;
        buffer.write_i16::<LittleEndian>(self.width.get())?;
        buffer.write_i16::<LittleEndian>(self.height.get())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        CHANNEL_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
struct CodecVersion;

impl PduBufferParsing<'_> for CodecVersion {
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
