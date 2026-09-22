use ironrdp_core::{Encode as _, WriteCursor, cast_int, cast_length, invalid_field_err};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::rdp6::{
    ABgrChannels, ARgbChannels, BgrAChannels, BitmapEncodeError, BitmapStreamEncoder, RgbAChannels,
};
use ironrdp_pdu::bitmap::{self, BitmapData, BitmapUpdateData, Compression};
use ironrdp_pdu::geometry::InclusiveRectangle;

use crate::BitmapUpdate;

// PERF: we could also remove the need for this buffer
#[derive(Clone)]
pub(crate) struct BitmapEncoder {
    buffer: Vec<u8>,
}

impl BitmapEncoder {
    pub(crate) fn new() -> Self {
        Self {
            buffer: vec![0; usize::from(u16::MAX)],
        }
    }

    /// Largest bitmap tile sent in one TS_BITMAP_DATA. FreeRDP decodes
    /// bitmap-update planar data with a 64×64 context and rejects anything
    /// larger, and Windows servers tile at 64×64 as well.
    const TILE: u16 = 64;

    pub(crate) fn encode(&mut self, bitmap: &BitmapUpdate, output: &mut [u8]) -> Result<usize, BitmapEncodeError> {
        if !bitmap.width.get().is_multiple_of(4) {
            return Err(BitmapEncodeError::Encode(invalid_field_err!(
                "bitmap",
                "Width must be a multiple of 4"
            )));
        }

        let bytes_per_pixel = usize::from(bitmap.format.bytes_per_pixel());
        let stride = bitmap.stride.get();
        let (width, height) = (bitmap.width.get(), bitmap.height.get());
        let tiles_x = width.div_ceil(Self::TILE);
        let tiles_y = height.div_ceil(Self::TILE);

        let mut cursor = WriteCursor::new(output);
        let total =
            cast_int!("tile count", usize::from(tiles_x) * usize::from(tiles_y)).map_err(BitmapEncodeError::Encode)?;
        BitmapUpdateData::encode_header(total, &mut cursor).map_err(BitmapEncodeError::Encode)?;

        for ty in 0..tiles_y {
            let top = ty * Self::TILE;
            let tile_h = Self::TILE.min(height - top);
            for tx in 0..tiles_x {
                let left = tx * Self::TILE;
                // `width` is a multiple of 4 and so is TILE, so every tile width is too.
                let tile_w = Self::TILE.min(width - left);
                let row_len = usize::from(tile_w) * bytes_per_pixel;
                let col = usize::from(left) * bytes_per_pixel;

                let encoder = BitmapStreamEncoder::new(usize::from(tile_w), usize::from(tile_h));
                let len = {
                    // Planar bitmaps are bottom-up.
                    let pixels = (usize::from(top)..usize::from(top + tile_h))
                        .rev()
                        .map(|y| &bitmap.data[y * stride + col..y * stride + col + row_len])
                        .flat_map(|row| row.chunks(bytes_per_pixel));
                    Self::encode_iter(encoder, bitmap.format, pixels, self.buffer.as_mut_slice())?
                };

                let data = BitmapData {
                    rectangle: InclusiveRectangle {
                        left: bitmap.x + left,
                        top: bitmap.y + top,
                        right: bitmap.x + left + tile_w - 1,
                        bottom: bitmap.y + top + tile_h - 1,
                    },
                    width: tile_w,
                    height: tile_h,
                    bits_per_pixel: u16::from(bitmap.format.bytes_per_pixel()) * 8,
                    compression_flags: Compression::BITMAP_COMPRESSION,
                    compressed_data_header: Some(bitmap::CompressedDataHeader {
                        main_body_size: cast_length!("main body size", len).map_err(BitmapEncodeError::Encode)?,
                        scan_width: tile_w,
                        uncompressed_size: cast_length!("uncompressed size", usize::from(tile_h) * row_len)
                            .map_err(BitmapEncodeError::Encode)?,
                    }),
                    bitmap_data: &self.buffer[..len],
                };

                data.encode(&mut cursor).map_err(BitmapEncodeError::Encode)?;
            }
        }

        Ok(cursor.pos())
    }

    fn encode_iter<'a, P>(
        mut encoder: BitmapStreamEncoder,
        format: PixelFormat,
        src: P,
        dst: &mut [u8],
    ) -> Result<usize, BitmapEncodeError>
    where
        P: Iterator<Item = &'a [u8]> + Clone,
    {
        let written = match format {
            PixelFormat::ARgb32 | PixelFormat::XRgb32 => {
                encoder.encode_pixels_stream::<_, ARgbChannels>(src, dst, true)?
            }
            PixelFormat::RgbA32 | PixelFormat::RgbX32 => {
                encoder.encode_pixels_stream::<_, RgbAChannels>(src, dst, true)?
            }
            PixelFormat::ABgr32 | PixelFormat::XBgr32 => {
                encoder.encode_pixels_stream::<_, ABgrChannels>(src, dst, true)?
            }
            PixelFormat::BgrA32 | PixelFormat::BgrX32 => {
                encoder.encode_pixels_stream::<_, BgrAChannels>(src, dst, true)?
            }
        };

        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use core::num::{NonZeroU16, NonZeroUsize};

    use ironrdp_core::{ReadCursor, decode_cursor};

    use super::*;

    #[test]
    fn splits_into_64x64_tiles() {
        let (w, h) = (136u16, 70u16);
        let stride = usize::from(w) * 4;
        let data: Vec<u8> = (0..stride * usize::from(h))
            .map(|i| u8::try_from(i % 251).expect("below 251"))
            .collect();
        let update = BitmapUpdate {
            x: 8,
            y: 4,
            width: NonZeroU16::new(w).unwrap(),
            height: NonZeroU16::new(h).unwrap(),
            format: PixelFormat::BgrA32,
            data: data.into(),
            stride: NonZeroUsize::new(stride).unwrap(),
        };
        let mut out = vec![0; 1 << 20];
        let len = BitmapEncoder::new().encode(&update, &mut out).unwrap();
        let mut cursor = ReadCursor::new(&out[..len]);
        let decoded: BitmapUpdateData<'_> = decode_cursor(&mut cursor).unwrap();
        let rects: Vec<_> = decoded
            .rectangles
            .iter()
            .map(|r| (r.rectangle.left, r.rectangle.top, r.width, r.height))
            .collect();
        assert_eq!(
            rects,
            vec![
                (8, 4, 64, 64),
                (72, 4, 64, 64),
                (136, 4, 8, 64),
                (8, 68, 64, 6),
                (72, 68, 64, 6),
                (136, 68, 8, 6)
            ]
        );
    }
}
