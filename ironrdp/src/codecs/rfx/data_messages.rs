use std::io::Write;

use bit_field::BitField;
use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use super::{
    BlockHeader, BlockType, CodecChannelHeader, CodecChannelType, RfxError, BLOCK_HEADER_SIZE,
    CODEC_CHANNEL_HEADER_SIZE,
};
use crate::{from_buffer, to_buffer, PduBufferParsing};
const CONTEXT_ID: u8 = 0;
const TILE_SIZE: u16 = 0x0040;
const COLOR_CONVERSION_ICT: u16 = 1;
const CLW_XFORM_DWT_53_A: u16 = 1;
const SCALAR_QUANTIZATION: u16 = 1;
const LRF: bool = true;
const CBT_REGION: u16 = 0xcac1;
const NUMBER_OF_TILESETS: u16 = 1;
const CBT_TILESET: u16 = 0xcac2;
const IDX: u16 = 0;
const IS_LAST_TILESET_FLAG: bool = true;
const QUANT_SIZE: usize = 5;
const RECTANGLE_SIZE: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct ContextPdu {
    pub flags: OperatingMode,
    pub entropy_algorithm: EntropyAlgorithm,
}

impl PduBufferParsing for ContextPdu {
    type Error = RfxError;

    fn from_buffer(mut buffer: &[u8]) -> Result<Self, Self::Error> {
        let header = CodecChannelHeader::from_buffer_with_type(buffer, CodecChannelType::Context)?;
        buffer = &buffer[header.buffer_length()..header.buffer_length() + header.data_length];

        let id = buffer.read_u8()?;
        if id != CONTEXT_ID {
            return Err(RfxError::InvalidContextId(id));
        }

        let tile_size = buffer.read_u16::<LittleEndian>()?;
        if tile_size != TILE_SIZE {
            return Err(RfxError::InvalidTileSize(tile_size));
        }

        let properties = buffer.read_u16::<LittleEndian>()?;
        let flags = OperatingMode::from_bits_truncate(properties.get_bits(0..3));
        let color_conversion_transform = properties.get_bits(3..5);
        if color_conversion_transform != COLOR_CONVERSION_ICT {
            return Err(RfxError::InvalidColorConversionTransform(
                color_conversion_transform,
            ));
        }

        let dwt = properties.get_bits(5..9);
        if dwt != CLW_XFORM_DWT_53_A {
            return Err(RfxError::InvalidDwt(dwt));
        }

        let entropy_algorithm_bits = properties.get_bits(9..13);
        let entropy_algorithm = EntropyAlgorithm::from_u16(entropy_algorithm_bits)
            .ok_or(RfxError::InvalidEntropyAlgorithm(entropy_algorithm_bits))?;

        let quantization_type = properties.get_bits(13..15);
        if quantization_type != SCALAR_QUANTIZATION {
            return Err(RfxError::InvalidQuantizationType(quantization_type));
        }

        let _reserved = properties.get_bit(15);

        Ok(Self {
            flags,
            entropy_algorithm,
        })
    }

    fn to_buffer(&self, mut buffer: &mut [u8]) -> Result<(), Self::Error> {
        let header = CodecChannelHeader {
            ty: CodecChannelType::Context,
            data_length: self.buffer_length() - CODEC_CHANNEL_HEADER_SIZE,
        };

        to_buffer!(header, buffer)?;
        buffer.write_u8(CONTEXT_ID)?;
        buffer.write_u16::<LittleEndian>(TILE_SIZE)?;

        let mut properties: u16 = 0;
        properties.set_bits(0..3, self.flags.bits());
        properties.set_bits(3..5, COLOR_CONVERSION_ICT);
        properties.set_bits(5..9, CLW_XFORM_DWT_53_A);
        properties.set_bits(9..13, self.entropy_algorithm.to_u16().unwrap());
        properties.set_bits(13..15, SCALAR_QUANTIZATION);
        properties.set_bit(15, false); // reserved
        buffer.write_u16::<LittleEndian>(properties)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        CODEC_CHANNEL_HEADER_SIZE + 5
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameBeginPdu {
    pub index: u32,
    pub number_of_regions: i16,
}

impl PduBufferParsing for FrameBeginPdu {
    type Error = RfxError;

    fn from_buffer(mut buffer: &[u8]) -> Result<Self, Self::Error> {
        let header =
            CodecChannelHeader::from_buffer_with_type(buffer, CodecChannelType::FrameBegin)?;
        buffer = &buffer[header.buffer_length()..header.buffer_length() + header.data_length];

        let index = buffer.read_u32::<LittleEndian>()?;
        let number_of_regions = buffer.read_i16::<LittleEndian>()?;

        Ok(Self {
            index,
            number_of_regions,
        })
    }

    fn to_buffer(&self, mut buffer: &mut [u8]) -> Result<(), Self::Error> {
        let header = CodecChannelHeader {
            ty: CodecChannelType::FrameBegin,
            data_length: self.buffer_length() - CODEC_CHANNEL_HEADER_SIZE,
        };

        to_buffer!(header, buffer)?;
        buffer.write_u32::<LittleEndian>(self.index)?;
        buffer.write_i16::<LittleEndian>(self.number_of_regions)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        CODEC_CHANNEL_HEADER_SIZE + 6
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameEndPdu;

impl PduBufferParsing for FrameEndPdu {
    type Error = RfxError;

    fn from_buffer(buffer: &[u8]) -> Result<Self, Self::Error> {
        let _header = CodecChannelHeader::from_buffer_with_type(buffer, CodecChannelType::FrameEnd);

        Ok(Self)
    }

    fn to_buffer(&self, buffer: &mut [u8]) -> Result<(), Self::Error> {
        CodecChannelHeader {
            ty: CodecChannelType::FrameEnd,
            data_length: self.buffer_length() - CODEC_CHANNEL_HEADER_SIZE,
        }
        .to_buffer(buffer)
    }

    fn buffer_length(&self) -> usize {
        CODEC_CHANNEL_HEADER_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegionPdu {
    pub rectangles: Vec<Rectangle>,
}

impl PduBufferParsing for RegionPdu {
    type Error = RfxError;

    fn from_buffer(mut buffer: &[u8]) -> Result<Self, Self::Error> {
        let header = CodecChannelHeader::from_buffer_with_type(buffer, CodecChannelType::Region)?;
        buffer = &buffer[header.buffer_length()..header.buffer_length() + header.data_length];

        let region_flags = buffer.read_u8()?;
        let lrf = region_flags.get_bit(0);
        if lrf != LRF {
            return Err(RfxError::InvalidLrf(lrf));
        }

        let number_of_rectangles = buffer.read_u16::<LittleEndian>()? as usize;
        if buffer.len() < number_of_rectangles * RECTANGLE_SIZE {
            return Err(RfxError::InvalidDataLength {
                expected: number_of_rectangles * RECTANGLE_SIZE,
                actual: buffer.len(),
            });
        }

        let rectangles = (0..number_of_rectangles)
            .map(|_| from_buffer!(Rectangle, buffer))
            .collect::<Result<Vec<_>, _>>()?;

        let region_type = buffer.read_u16::<LittleEndian>()?;
        if region_type != CBT_REGION {
            return Err(RfxError::InvalidRegionType(region_type));
        }

        let number_of_tilesets = buffer.read_u16::<LittleEndian>()?;
        if number_of_tilesets != NUMBER_OF_TILESETS {
            return Err(RfxError::InvalidNumberOfTilesets(number_of_tilesets));
        }

        Ok(Self { rectangles })
    }

    fn to_buffer(&self, mut buffer: &mut [u8]) -> Result<(), Self::Error> {
        let header = CodecChannelHeader {
            ty: CodecChannelType::Region,
            data_length: self.buffer_length() - CODEC_CHANNEL_HEADER_SIZE,
        };

        to_buffer!(header, buffer)?;

        let mut region_flags = 0;
        region_flags.set_bit(0, LRF);
        buffer.write_u8(region_flags)?;

        buffer.write_u16::<LittleEndian>(self.rectangles.len() as u16)?;
        for rectangle in self.rectangles.iter() {
            to_buffer!(rectangle, buffer)?;
        }

        buffer.write_u16::<LittleEndian>(CBT_REGION)?;
        buffer.write_u16::<LittleEndian>(NUMBER_OF_TILESETS)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        CODEC_CHANNEL_HEADER_SIZE
            + 7
            + self
                .rectangles
                .iter()
                .map(PduBufferParsing::buffer_length)
                .sum::<usize>()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TileSetPdu<'a> {
    pub entropy_algorithm: EntropyAlgorithm,
    pub quants: Vec<Quant>,
    pub tiles: Vec<Tile<'a>>,
}

impl<'a> TileSetPdu<'a> {
    pub fn from_buffer(mut buffer: &'a [u8]) -> Result<Self, RfxError> {
        let header =
            CodecChannelHeader::from_buffer_with_type(buffer, CodecChannelType::Extension)?;
        buffer = &buffer[header.buffer_length()..header.buffer_length() + header.data_length];

        let subtype = buffer.read_u16::<LittleEndian>()?;
        if subtype != CBT_TILESET {
            return Err(RfxError::InvalidSubtype(subtype));
        }

        let id_of_context = buffer.read_u16::<LittleEndian>()?;
        if id_of_context != IDX {
            return Err(RfxError::InvalidIdOfContext(id_of_context));
        }

        let properties = buffer.read_u16::<LittleEndian>()?;
        let is_last = properties.get_bit(0);
        if is_last != IS_LAST_TILESET_FLAG {
            return Err(RfxError::InvalidItFlag(is_last));
        }

        // The encoder MUST set `flags` value to the value of flags
        // that is set in the properties field of TS_RFX_CONTEXT.
        // The decoder MUST ignore this flag and MUST use the flags specified
        // in the flags field of the TS_RFX_CONTEXT.

        let color_conversion_transform = properties.get_bits(4..6);
        if color_conversion_transform != COLOR_CONVERSION_ICT {
            return Err(RfxError::InvalidColorConversionTransform(
                color_conversion_transform,
            ));
        }

        let dwt = properties.get_bits(6..10);
        if dwt != CLW_XFORM_DWT_53_A {
            return Err(RfxError::InvalidDwt(dwt));
        }

        let entropy_algorithm_bits = properties.get_bits(10..14);
        let entropy_algorithm = EntropyAlgorithm::from_u16(entropy_algorithm_bits)
            .ok_or(RfxError::InvalidEntropyAlgorithm(entropy_algorithm_bits))?;

        let quantization_type = properties.get_bits(14..16);
        if quantization_type != SCALAR_QUANTIZATION {
            return Err(RfxError::InvalidQuantizationType(quantization_type));
        }

        let number_of_quants = buffer.read_u8()? as usize;

        let tile_size = u16::from(buffer.read_u8()?);
        if tile_size != TILE_SIZE {
            return Err(RfxError::InvalidTileSize(tile_size));
        }

        let number_of_tiles = buffer.read_u16::<LittleEndian>()?;
        let tiles_data_size = buffer.read_u32::<LittleEndian>()? as usize;

        let expected_length = tiles_data_size + number_of_quants * QUANT_SIZE;
        if buffer.len() < expected_length {
            return Err(RfxError::InvalidDataLength {
                expected: expected_length,
                actual: buffer.len(),
            });
        }

        let quants = (0..number_of_quants)
            .map(|_| from_buffer!(Quant, buffer))
            .collect::<Result<Vec<_>, _>>()?;

        let mut tiles_buffer = &buffer[..tiles_data_size];
        let tiles = (0..number_of_tiles)
            .map(|_| from_buffer!(Tile, tiles_buffer))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            entropy_algorithm,
            quants,
            tiles,
        })
    }

    pub fn to_buffer(&self, mut buffer: &mut [u8]) -> Result<(), RfxError> {
        let header = CodecChannelHeader {
            ty: CodecChannelType::Extension,
            data_length: self.buffer_length() - CODEC_CHANNEL_HEADER_SIZE,
        };

        to_buffer!(header, buffer)?;
        buffer.write_u16::<LittleEndian>(CBT_TILESET)?;
        buffer.write_u16::<LittleEndian>(IDX)?;

        let mut properties: u16 = 0;
        properties.set_bit(0, IS_LAST_TILESET_FLAG);
        properties.set_bits(1..4, OperatingMode::empty().bits()); // The decoder MUST ignore this flag
        properties.set_bits(4..6, COLOR_CONVERSION_ICT);
        properties.set_bits(6..10, CLW_XFORM_DWT_53_A);
        properties.set_bits(10..14, self.entropy_algorithm.to_u16().unwrap());
        properties.set_bits(14..16, SCALAR_QUANTIZATION);
        buffer.write_u16::<LittleEndian>(properties)?;

        buffer.write_u8(self.quants.len() as u8)?;
        buffer.write_u8(TILE_SIZE as u8)?;
        buffer.write_u16::<LittleEndian>(self.tiles.len() as u16)?;

        let tiles_data_size = self.tiles.iter().map(|t| t.buffer_length()).sum::<usize>() as u32;
        buffer.write_u32::<LittleEndian>(tiles_data_size)?;

        for quant in self.quants.iter() {
            to_buffer!(quant, buffer)?;
        }

        for tile in self.tiles.iter() {
            to_buffer!(tile, buffer)?;
        }

        Ok(())
    }

    pub fn buffer_length(&self) -> usize {
        CODEC_CHANNEL_HEADER_SIZE
            + 14
            + self
                .quants
                .iter()
                .map(PduBufferParsing::buffer_length)
                .sum::<usize>()
            + self.tiles.iter().map(|t| t.buffer_length()).sum::<usize>()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rectangle {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl PduBufferParsing for Rectangle {
    type Error = RfxError;

    fn from_buffer(mut buffer: &[u8]) -> Result<Self, Self::Error> {
        let x = buffer.read_u16::<LittleEndian>()?;
        let y = buffer.read_u16::<LittleEndian>()?;
        let width = buffer.read_u16::<LittleEndian>()?;
        let height = buffer.read_u16::<LittleEndian>()?;

        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    fn to_buffer(&self, mut buffer: &mut [u8]) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(self.x)?;
        buffer.write_u16::<LittleEndian>(self.y)?;
        buffer.write_u16::<LittleEndian>(self.width)?;
        buffer.write_u16::<LittleEndian>(self.height)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        RECTANGLE_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Quant {
    pub ll3: u8,
    pub lh3: u8,
    pub hl3: u8,
    pub hh3: u8,
    pub lh2: u8,
    pub hl2: u8,
    pub hh2: u8,
    pub lh1: u8,
    pub hl1: u8,
    pub hh1: u8,
}

impl PduBufferParsing for Quant {
    type Error = RfxError;

    fn from_buffer(mut buffer: &[u8]) -> Result<Self, Self::Error> {
        let level3 = buffer.read_u16::<LittleEndian>()?;
        let ll3 = level3.get_bits(0..4) as u8;
        let lh3 = level3.get_bits(4..8) as u8;
        let hl3 = level3.get_bits(8..12) as u8;
        let hh3 = level3.get_bits(12..16) as u8;

        let level2_with_lh1 = buffer.read_u16::<LittleEndian>()?;
        let lh2 = level2_with_lh1.get_bits(0..4) as u8;
        let hl2 = level2_with_lh1.get_bits(4..8) as u8;
        let hh2 = level2_with_lh1.get_bits(8..12) as u8;
        let lh1 = level2_with_lh1.get_bits(12..16) as u8;

        let level1 = buffer.read_u8()?;
        let hl1 = level1.get_bits(0..4);
        let hh1 = level1.get_bits(4..8);

        Ok(Self {
            ll3,
            lh3,
            hl3,
            hh3,
            lh2,
            hl2,
            hh2,
            lh1,
            hl1,
            hh1,
        })
    }

    fn to_buffer(&self, mut buffer: &mut [u8]) -> Result<(), Self::Error> {
        let mut level3 = 0;
        level3.set_bits(0..4, u16::from(self.ll3));
        level3.set_bits(4..8, u16::from(self.lh3));
        level3.set_bits(8..12, u16::from(self.hl3));
        level3.set_bits(12..16, u16::from(self.hh3));

        let mut level2_with_lh1 = 0;
        level2_with_lh1.set_bits(0..4, u16::from(self.lh2));
        level2_with_lh1.set_bits(4..8, u16::from(self.hl2));
        level2_with_lh1.set_bits(8..12, u16::from(self.hh2));
        level2_with_lh1.set_bits(12..16, u16::from(self.lh1));

        let mut level1 = 0;
        level1.set_bits(0..4, self.hl1);
        level1.set_bits(4..8, self.hh1);

        buffer.write_u16::<LittleEndian>(level3)?;
        buffer.write_u16::<LittleEndian>(level2_with_lh1)?;
        buffer.write_u8(level1)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        QUANT_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tile<'a> {
    pub y_quant_index: u8,
    pub cb_quant_index: u8,
    pub cr_quant_index: u8,

    pub x_tile_index: u16,
    pub y_tile_index: u16,

    pub y_data: &'a [u8],
    pub cb_data: &'a [u8],
    pub cr_data: &'a [u8],
}

impl<'a> Tile<'a> {
    pub fn from_buffer(mut buffer: &'a [u8]) -> Result<Self, RfxError> {
        let header = BlockHeader::from_buffer_with_type(buffer, BlockType::Tile)?;
        buffer = &buffer[header.buffer_length()..header.buffer_length() + header.data_length];

        let y_quant_index = buffer.read_u8()?;
        let cb_quant_index = buffer.read_u8()?;
        let cr_quant_index = buffer.read_u8()?;

        let x_tile_index = buffer.read_u16::<LittleEndian>()?;
        let y_tile_index = buffer.read_u16::<LittleEndian>()?;

        let y_component_length = buffer.read_u16::<LittleEndian>()? as usize;
        let cb_component_length = buffer.read_u16::<LittleEndian>()? as usize;
        let cr_component_length = buffer.read_u16::<LittleEndian>()? as usize;

        if buffer.len() < y_component_length + cb_component_length + cr_component_length {
            return Err(RfxError::InvalidDataLength {
                expected: y_component_length + cb_component_length + cr_component_length,
                actual: buffer.len(),
            });
        }

        let y_start = 0;
        let cb_start = y_component_length;
        let cr_start = y_component_length + cb_component_length;

        let y_data = &buffer[y_start..y_component_length];
        let cb_data = &buffer[cb_start..cb_start + cb_component_length];
        let cr_data = &buffer[cr_start..cr_start + cr_component_length];

        Ok(Self {
            y_quant_index,
            cb_quant_index,
            cr_quant_index,

            x_tile_index,
            y_tile_index,

            y_data,
            cb_data,
            cr_data,
        })
    }

    pub fn to_buffer(&self, mut buffer: &mut [u8]) -> Result<(), RfxError> {
        let header = BlockHeader {
            ty: BlockType::Tile,
            data_length: self.buffer_length() - BLOCK_HEADER_SIZE,
        };

        to_buffer!(header, buffer)?;
        buffer.write_u8(self.y_quant_index)?;
        buffer.write_u8(self.cb_quant_index)?;
        buffer.write_u8(self.cr_quant_index)?;

        buffer.write_u16::<LittleEndian>(self.x_tile_index)?;
        buffer.write_u16::<LittleEndian>(self.y_tile_index)?;

        buffer.write_u16::<LittleEndian>(self.y_data.len() as u16)?;
        buffer.write_u16::<LittleEndian>(self.cb_data.len() as u16)?;
        buffer.write_u16::<LittleEndian>(self.cr_data.len() as u16)?;

        buffer.write_all(self.y_data)?;
        buffer.write_all(self.cb_data)?;
        buffer.write_all(self.cr_data)?;

        Ok(())
    }

    pub fn buffer_length(&self) -> usize {
        BLOCK_HEADER_SIZE + 13 + self.y_data.len() + self.cb_data.len() + self.cr_data.len()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
#[repr(u16)]
pub enum EntropyAlgorithm {
    Rlgr1 = 0x01,
    Rlgr3 = 0x04,
}

bitflags! {
    pub struct OperatingMode: u16 {
        const IMAGE_MODE = 0x02; // if not set, the codec is operating in video mode
    }
}
