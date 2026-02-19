use core::num::NonZeroUsize;

use ironrdp_core::{cast_int, cast_length, Encode as _, WriteCursor};
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

    pub(crate) fn encode(&mut self, bitmap: &BitmapUpdate, output: &mut [u8]) -> Result<usize, BitmapEncodeError> {
        let bytes_per_pixel = u16::from(bitmap.format.bytes_per_pixel());
        let row_len = bitmap.width.get() * bytes_per_pixel;
        let chunk_height = u16::MAX / row_len;

        let mut cursor = WriteCursor::new(output);
        let stride = bitmap.stride.get();
        let chunks = bitmap.data.chunks(stride * usize::from(chunk_height));

        let total = cast_int!("chunks length lower bound", chunks.size_hint().0).map_err(BitmapEncodeError::Encode)?;
        BitmapUpdateData::encode_header(total, &mut cursor).map_err(BitmapEncodeError::Encode)?;

        for (i, chunk) in chunks.enumerate() {
            let height = cast_int!("bitmap height", chunk.len() / stride).map_err(BitmapEncodeError::Encode)?;
            let i: u16 = cast_int!("chunk idx", i).map_err(BitmapEncodeError::Encode)?;
            let top = bitmap.y + i * chunk_height;

            let encoder = BitmapStreamEncoder::new(NonZeroUsize::from(bitmap.width).get(), usize::from(height));

            let len = {
                let pixels = chunk
                    .chunks(stride)
                    .map(|row| &row[..usize::from(row_len)])
                    .rev()
                    .flat_map(|row| row.chunks(usize::from(bytes_per_pixel)));

                Self::encode_iter(encoder, bitmap.format, pixels, self.buffer.as_mut_slice())?
            };

            let data = BitmapData {
                rectangle: InclusiveRectangle {
                    left: bitmap.x,
                    top,
                    right: bitmap.x + bitmap.width.get() - 1,
                    bottom: top + height - 1,
                },
                width: u16::from(bitmap.width),
                height,
                bits_per_pixel: u16::from(bitmap.format.bytes_per_pixel()) * 8,
                compression_flags: Compression::BITMAP_COMPRESSION,
                compressed_data_header: Some(bitmap::CompressedDataHeader {
                    main_body_size: cast_length!("main body size", len).map_err(BitmapEncodeError::Encode)?,
                    scan_width: row_len,
                    uncompressed_size: height * row_len,
                }),
                bitmap_data: &self.buffer[..len],
            };

            data.encode(&mut cursor).map_err(BitmapEncodeError::Encode)?;
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

    use bytes::Bytes;
    use ironrdp_core::decode;
    use ironrdp_graphics::image_processing::PixelFormat;
    use ironrdp_pdu::bitmap::BitmapUpdateData;

    use super::BitmapEncoder;
    use crate::BitmapUpdate;

    #[test]
    fn encode_allows_non_multiple_of_four_pixel_width_for_32bpp() {
        let mut encoder = BitmapEncoder::new();
        let width = NonZeroU16::new(5).expect("non-zero width");
        let height = NonZeroU16::new(3).expect("non-zero height");
        let stride = NonZeroUsize::new(usize::from(width.get()) * 4).expect("non-zero stride");
        let bitmap = BitmapUpdate {
            x: 0,
            y: 0,
            width,
            height,
            format: PixelFormat::ARgb32,
            data: Bytes::from(vec![0xAA; stride.get() * usize::from(height.get())]),
            stride,
        };

        let mut output = vec![0u8; 4096];
        let written = encoder
            .encode(&bitmap, &mut output)
            .expect("encoding should succeed for width 5");
        let payload = &output[..written];

        let update: BitmapUpdateData<'_> = decode(payload).expect("bitmap update should decode");
        assert_eq!(update.rectangles.len(), 1);

        let rectangle = &update.rectangles[0];
        assert_eq!(rectangle.width, width.get());
        assert_eq!(rectangle.height, height.get());
    }

    #[test]
    fn encode_sets_scan_width_in_bytes() {
        let mut encoder = BitmapEncoder::new();
        let width = NonZeroU16::new(5).expect("non-zero width");
        let height = NonZeroU16::new(2).expect("non-zero height");
        let stride = NonZeroUsize::new(usize::from(width.get()) * 4).expect("non-zero stride");
        let bitmap = BitmapUpdate {
            x: 0,
            y: 0,
            width,
            height,
            format: PixelFormat::ARgb32,
            data: Bytes::from(vec![0x55; stride.get() * usize::from(height.get())]),
            stride,
        };

        let mut output = vec![0u8; 4096];
        let written = encoder.encode(&bitmap, &mut output).expect("encoding should succeed");
        let payload = &output[..written];

        let update: BitmapUpdateData<'_> = decode(payload).expect("bitmap update should decode");
        let rectangle = &update.rectangles[0];
        let compressed_header = rectangle
            .compressed_data_header
            .as_ref()
            .expect("compressed header should be present");

        assert_eq!(compressed_header.scan_width, width.get() * 4);
    }
}
