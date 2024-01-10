#[cfg(test)]
mod tests;

use bitflags::bitflags;

use crate::cursor::{ReadCursor, WriteCursor};
use crate::{PduDecode, PduEncode, PduResult};

const BITMAP_LENGTH: usize = 24;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct BitmapDrawingFlags: u8 {
        const ALLOW_DYNAMIC_COLOR_FIDELITY = 0x02;
        const ALLOW_COLOR_SUBSAMPLING = 0x04;
        const ALLOW_SKIP_ALPHA = 0x08;
        const UNUSED_FLAG = 0x10;
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Bitmap {
    pub pref_bits_per_pix: u16,
    pub desktop_width: u16,
    pub desktop_height: u16,
    pub desktop_resize_flag: bool,
    pub drawing_flags: BitmapDrawingFlags,
}

impl Bitmap {
    const NAME: &'static str = "Bitmap";

    const FIXED_PART_SIZE: usize = BITMAP_LENGTH;
}

impl PduEncode for Bitmap {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.pref_bits_per_pix);
        dst.write_u16(1); // receive1BitPerPixel
        dst.write_u16(1); // receive4BitsPerPixel
        dst.write_u16(1); // receive8BitsPerPixel
        dst.write_u16(self.desktop_width);
        dst.write_u16(self.desktop_height);
        write_padding!(dst, 2);
        dst.write_u16(u16::from(self.desktop_resize_flag));
        dst.write_u16(1); // bitmapCompressionFlag
        dst.write_u8(0); // highColorFlags
        dst.write_u8(self.drawing_flags.bits());
        dst.write_u16(1); // multipleRectangleSupport
        write_padding!(dst, 2);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for Bitmap {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let pref_bits_per_pix = src.read_u16();

        let _receive_1_bit_per_pixel = src.read_u16() != 0;
        let _receive_4_bit_per_pixel = src.read_u16() != 0;
        let _receive_8_bit_per_pixel = src.read_u16() != 0;
        let desktop_width = src.read_u16();
        let desktop_height = src.read_u16();
        read_padding!(src, 2);
        let desktop_resize_flag = src.read_u16() != 0;

        let is_bitmap_compress_flag_set = src.read_u16() != 0;
        if !is_bitmap_compress_flag_set {
            return Err(invalid_message_err!(
                "isBitmapCompressFlagSet",
                "invalid compression flag"
            ));
        }

        let _high_color_flags = src.read_u8();
        let drawing_flags = BitmapDrawingFlags::from_bits_truncate(src.read_u8());

        let is_multiple_rect_supported = src.read_u16() != 0;
        if !is_multiple_rect_supported {
            return Err(invalid_message_err!(
                "isMultipleRectSupported",
                "invalid multiple rect support"
            ));
        }

        read_padding!(src, 2);

        Ok(Bitmap {
            pref_bits_per_pix,
            desktop_width,
            desktop_height,
            desktop_resize_flag,
            drawing_flags,
        })
    }
}

impl_pdu_parsing!(Bitmap);
