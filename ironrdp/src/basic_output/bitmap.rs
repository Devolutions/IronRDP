#[cfg(test)]
mod tests;

use std::fmt::{self, Debug};
use std::io::{self, Write};

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;

use crate::utils::{Rectangle, SplitTo};
use crate::{impl_from_error, PduBufferParsing, PduParsing};

pub const COMPRESSED_DATA_HEADER_SIZE: usize = 8;
pub const BITMAP_DATA_MAIN_DATA_SIZE: usize = 12;
pub const FIRST_ROW_SIZE_VALUE: u16 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap<'a> {
    pub rectangles_number: usize,
    pub rectangles: Vec<BitmapData<'a>>,
}

impl<'a> PduBufferParsing<'a> for Bitmap<'a> {
    type Error = BitmapError;

    fn from_buffer_consume(buffer: &mut &'a [u8]) -> Result<Self, Self::Error> {
        let update_type = BitmapFlags::from_bits_truncate(buffer.read_u16::<LittleEndian>()?);
        if !update_type.contains(BitmapFlags::BITMAP_UPDATE_TYPE) {
            return Err(BitmapError::InvalidUpdateType);
        }

        let rectangles_number = buffer.read_u16::<LittleEndian>()? as usize;
        let mut rectangles = Vec::with_capacity(rectangles_number);

        for _ in 0..rectangles_number {
            rectangles.push(BitmapData::from_buffer_consume(buffer)?);
        }

        Ok(Bitmap {
            rectangles_number,
            rectangles,
        })
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(BitmapFlags::BITMAP_UPDATE_TYPE.bits())?;
        buffer.write_u16::<LittleEndian>(self.rectangles_number as u16)?;
        for bitmap_data in self.rectangles.iter() {
            bitmap_data.to_buffer_consume(buffer)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        self.rectangles.iter().map(|b| b.buffer_length()).sum::<usize>()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct BitmapData<'a> {
    pub rectangle: Rectangle,
    pub width: u16,
    pub height: u16,
    pub bits_per_pixel: u16,
    pub compression_flags: Compression,
    pub bitmap_data_length: usize,
    pub compressed_data_header: Option<CompressedDataHeader>,
    pub bitmap_data: &'a [u8],
}

impl<'a> PduBufferParsing<'a> for BitmapData<'a> {
    type Error = BitmapError;

    fn from_buffer_consume(mut buffer: &mut &'a [u8]) -> Result<Self, Self::Error> {
        let rectangle = Rectangle::from_buffer(&mut buffer)?;

        let width = buffer.read_u16::<LittleEndian>()?;
        let height = buffer.read_u16::<LittleEndian>()?;

        let bits_per_pixel = buffer.read_u16::<LittleEndian>()?;
        let compression_flags = Compression::from_bits_truncate(buffer.read_u16::<LittleEndian>()?);
        let bitmap_data_length = buffer.read_u16::<LittleEndian>()? as usize;

        if buffer.len() < bitmap_data_length {
            return Err(BitmapError::InvalidDataLength {
                actual: buffer.len(),
                expected: bitmap_data_length,
            });
        }

        let compressed_data_header = if !compression_flags.contains(Compression::NOT_COMPRESSED) {
            Some(CompressedDataHeader::from_buffer_consume(buffer)?)
        } else {
            None
        };

        let bitmap_data = if compressed_data_header.is_some() {
            buffer.split_to(bitmap_data_length - COMPRESSED_DATA_HEADER_SIZE)
        } else {
            buffer.split_to(bitmap_data_length)
        };

        Ok(BitmapData {
            rectangle,
            width,
            height,
            bits_per_pixel,
            compression_flags,
            bitmap_data_length,
            compressed_data_header,
            bitmap_data,
        })
    }

    fn to_buffer_consume(&self, mut buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        self.rectangle.to_buffer(&mut buffer)?;
        buffer.write_u16::<LittleEndian>(self.width)?;
        buffer.write_u16::<LittleEndian>(self.height)?;
        buffer.write_u16::<LittleEndian>(self.bits_per_pixel)?;
        buffer.write_u16::<LittleEndian>(self.compression_flags.bits())?;
        buffer.write_u16::<LittleEndian>(self.bitmap_data_length as u16)?;
        if let Some(ref compressed_data_header) = self.compressed_data_header {
            compressed_data_header.to_buffer_consume(buffer)?;
        };
        buffer.write_all(self.bitmap_data)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        BITMAP_DATA_MAIN_DATA_SIZE + self.bitmap_data_length
    }
}

impl Debug for BitmapData<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "BitmapData:[{:?},
            width - {},
            height - {},
            bits_per_pixel - {},
            compression_flags - ({:?}),
            compressed_data_header - {:?}
            \nReceived Bitmap data buffer with length - {}\n]",
            self.rectangle,
            self.width,
            self.height,
            self.bits_per_pixel,
            self.compression_flags,
            self.compressed_data_header,
            self.bitmap_data.len()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressedDataHeader {
    pub main_body_size: u16,
    pub scan_width: u16,
    pub uncompressed_size: u16,
}

impl<'a> PduBufferParsing<'a> for CompressedDataHeader {
    type Error = BitmapError;

    fn from_buffer_consume(buffer: &mut &[u8]) -> Result<Self, Self::Error> {
        let size = buffer.read_u16::<LittleEndian>()?;
        if size != FIRST_ROW_SIZE_VALUE {
            return Err(BitmapError::InvalidFirstRowSize {
                actual: size as usize,
                expected: FIRST_ROW_SIZE_VALUE as usize,
            });
        }

        let main_body_size = buffer.read_u16::<LittleEndian>()?;
        let scan_width = buffer.read_u16::<LittleEndian>()?;
        if scan_width % 4 != 0 {
            return Err(BitmapError::InvalidScanWidth);
        }
        let uncompressed_size = buffer.read_u16::<LittleEndian>()?;

        Ok(Self {
            main_body_size,
            scan_width,
            uncompressed_size,
        })
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(FIRST_ROW_SIZE_VALUE)?;
        buffer.write_u16::<LittleEndian>(self.main_body_size)?;
        buffer.write_u16::<LittleEndian>(self.scan_width)?;
        buffer.write_u16::<LittleEndian>(self.uncompressed_size)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        COMPRESSED_DATA_HEADER_SIZE
    }
}

bitflags! {
    pub struct BitmapFlags: u16{
        const BITMAP_UPDATE_TYPE = 0x0001;
    }
}

bitflags! {
    pub struct Compression: u16 {
       const COMPRESSED_HDR = 0x0001;
       const NOT_COMPRESSED = 0x0400;
    }
}

#[derive(Debug, Fail)]
pub enum BitmapError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "Invalid update type for Bitmap Update")]
    InvalidUpdateType,
    #[fail(
        display = "Input buffer len is shorter than the data length: {} < {}",
        actual, expected
    )]
    InvalidDataLength { actual: usize, expected: usize },
    #[fail(display = "Compression is not supported for Bitmap data")]
    NotSupportedCompression,
    #[fail(display = "Invalid first row size, expected: {}, but got: {}", actual, expected)]
    InvalidFirstRowSize { actual: usize, expected: usize },
    #[fail(display = "The width of the bitmap must be divisible by 4")]
    InvalidScanWidth,
}

impl_from_error!(io::Error, BitmapError, BitmapError::IOError);
