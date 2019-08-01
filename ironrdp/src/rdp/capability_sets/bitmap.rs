#[cfg(test)]
mod test;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::{rdp::CapabilitySetsError, PduParsing};

const BITMAP_LENGTH: usize = 24;

bitflags! {
    pub struct BitmapDrawingFlags: u8 {
        const ALLOW_DYNAMIC_COLOR_FIDELITY = 0x02;
        const ALLOW_COLOR_SUBSAMPLING = 0x04;
        const ALLOW_SKIP_ALPHA = 0x08;
        const UNUSED_FLAG = 0x10;
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct Bitmap {
    pub pref_bits_per_pix: u16,
    pub desktop_width: u16,
    pub desktop_height: u16,
    pub desktop_resize_flag: bool,
    pub drawing_flags: BitmapDrawingFlags,
}

impl PduParsing for Bitmap {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let pref_bits_per_pix = buffer.read_u16::<LittleEndian>()?;

        let _receive_1_bit_per_pixel = buffer.read_u16::<LittleEndian>()? != 0;
        let _receive_4_bit_per_pixel = buffer.read_u16::<LittleEndian>()? != 0;
        let _receive_8_bit_per_pixel = buffer.read_u16::<LittleEndian>()? != 0;
        let desktop_width = buffer.read_u16::<LittleEndian>()?;
        let desktop_height = buffer.read_u16::<LittleEndian>()?;
        let _padding = buffer.read_u16::<LittleEndian>()?;
        let desktop_resize_flag = buffer.read_u16::<LittleEndian>()? != 0;

        let is_bitmap_compress_flag_set = buffer.read_u16::<LittleEndian>()? != 0;
        if !is_bitmap_compress_flag_set {
            return Err(CapabilitySetsError::InvalidCompressionFlag);
        }

        let _high_color_flags = buffer.read_u8()?;
        let drawing_flags = BitmapDrawingFlags::from_bits_truncate(buffer.read_u8()?);

        let is_multiple_rect_supported = buffer.read_u16::<LittleEndian>()? != 0;
        if !is_multiple_rect_supported {
            return Err(CapabilitySetsError::InvalidMultipleRectSupport);
        }

        let _padding = buffer.read_u16::<LittleEndian>()?;

        Ok(Bitmap {
            pref_bits_per_pix,
            desktop_width,
            desktop_height,
            desktop_resize_flag,
            drawing_flags,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(self.pref_bits_per_pix)?;
        buffer.write_u16::<LittleEndian>(1)?; // receive1BitPerPixel
        buffer.write_u16::<LittleEndian>(1)?; // receive4BitsPerPixel
        buffer.write_u16::<LittleEndian>(1)?; // receive8BitsPerPixel
        buffer.write_u16::<LittleEndian>(self.desktop_width)?;
        buffer.write_u16::<LittleEndian>(self.desktop_height)?;
        buffer.write_u16::<LittleEndian>(0)?; // padding
        buffer.write_u16::<LittleEndian>(u16::from(self.desktop_resize_flag))?;
        buffer.write_u16::<LittleEndian>(1)?; // bitmapCompressionFlag
        buffer.write_u8(0)?; // highColorFlags
        buffer.write_u8(self.drawing_flags.bits())?;
        buffer.write_u16::<LittleEndian>(1)?; // multipleRectangleSupport
        buffer.write_u16::<LittleEndian>(0)?; // padding

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        BITMAP_LENGTH
    }
}
