use ironrdp_core::WriteCursor;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::rdp6::{ABgrChannels, ARgbChannels, BgrAChannels, BitmapStreamEncoder, RgbAChannels};
use ironrdp_pdu::bitmap::{self, BitmapData, BitmapUpdateData, Compression};
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_pdu::{invalid_field_err, PduEncode, PduError};

use crate::{BitmapUpdate, PixelOrder};

// PERF: we could also remove the need for this buffer
pub(crate) struct BitmapEncoder {
    buffer: Vec<u8>,
}

impl BitmapEncoder {
    pub(crate) fn new() -> Self {
        Self {
            buffer: vec![0; u16::MAX as usize],
        }
    }

    pub(crate) fn encode(&mut self, bitmap: &BitmapUpdate, output: &mut [u8]) -> Result<usize, PduError> {
        // FIXME: support non-multiple of 4 widths.
        //
        // It’s not clear how to achieve that yet, but generally, server uses multiple of 4-widths,
        // and client has surface capabilities, so this path is unlikely.
        if bitmap.width.get() % 4 != 0 {
            return Err(invalid_field_err!("bitmap", "Width must be a multiple of 4"));
        }

        let bytes_per_pixel = usize::from(bitmap.format.bytes_per_pixel());
        let row_len = usize::from(bitmap.width.get()) * bytes_per_pixel;
        let chunk_height = usize::from(u16::MAX) / row_len;

        let mut cursor = WriteCursor::new(output);
        let chunks = bitmap.data.chunks(bitmap.stride * chunk_height);

        let total = u16::try_from(chunks.size_hint().0).unwrap();
        BitmapUpdateData::encode_header(total, &mut cursor)?;

        for (i, chunk) in chunks.enumerate() {
            let height = chunk.len() / bitmap.stride;
            let top = usize::from(bitmap.top) + i * chunk_height;

            let encoder = BitmapStreamEncoder::new(usize::from(bitmap.width.get()), height);

            let len = match bitmap.order {
                PixelOrder::BottomToTop => {
                    Self::encode_slice(encoder, bitmap.format, &chunk[..row_len], self.buffer.as_mut_slice())
                }

                PixelOrder::TopToBottom => {
                    let pixels = chunk
                        .chunks(bitmap.stride)
                        .map(|row| &row[..row_len])
                        .rev()
                        .flat_map(|row| row.chunks(bytes_per_pixel));

                    Self::encode_iter(encoder, bitmap.format, pixels, self.buffer.as_mut_slice())
                }
            };

            let data = BitmapData {
                rectangle: InclusiveRectangle {
                    left: bitmap.left,
                    top: u16::try_from(top).unwrap(),
                    right: bitmap.left + bitmap.width.get() - 1,
                    bottom: u16::try_from(top + height - 1).unwrap(),
                },
                width: u16::from(bitmap.width),
                height: u16::try_from(height).unwrap(),
                bits_per_pixel: u16::from(bitmap.format.bytes_per_pixel()) * 8,
                compression_flags: Compression::BITMAP_COMPRESSION,
                compressed_data_header: Some(bitmap::CompressedDataHeader {
                    main_body_size: u16::try_from(len).unwrap(),
                    scan_width: u16::from(bitmap.width),
                    uncompressed_size: u16::try_from(height * row_len).unwrap(),
                }),
                bitmap_data: &self.buffer[..len],
            };

            data.encode(&mut cursor)?;
        }

        Ok(cursor.pos())
    }

    fn encode_slice(mut encoder: BitmapStreamEncoder, format: PixelFormat, src: &[u8], dst: &mut [u8]) -> usize {
        match format {
            PixelFormat::ARgb32 | PixelFormat::XRgb32 => encoder.encode_bitmap::<ARgbChannels>(src, dst, true).unwrap(),
            PixelFormat::RgbA32 | PixelFormat::RgbX32 => encoder.encode_bitmap::<RgbAChannels>(src, dst, true).unwrap(),
            PixelFormat::ABgr32 | PixelFormat::XBgr32 => encoder.encode_bitmap::<ABgrChannels>(src, dst, true).unwrap(),
            PixelFormat::BgrA32 | PixelFormat::BgrX32 => encoder.encode_bitmap::<BgrAChannels>(src, dst, true).unwrap(),
        }
    }

    fn encode_iter<'a, P>(mut encoder: BitmapStreamEncoder, format: PixelFormat, src: P, dst: &mut [u8]) -> usize
    where
        P: Iterator<Item = &'a [u8]> + Clone,
    {
        match format {
            PixelFormat::ARgb32 | PixelFormat::XRgb32 => {
                encoder.encode_pixels_stream::<_, ARgbChannels>(src, dst, true).unwrap()
            }
            PixelFormat::RgbA32 | PixelFormat::RgbX32 => {
                encoder.encode_pixels_stream::<_, RgbAChannels>(src, dst, true).unwrap()
            }
            PixelFormat::ABgr32 | PixelFormat::XBgr32 => {
                encoder.encode_pixels_stream::<_, ABgrChannels>(src, dst, true).unwrap()
            }
            PixelFormat::BgrA32 | PixelFormat::BgrX32 => {
                encoder.encode_pixels_stream::<_, BgrAChannels>(src, dst, true).unwrap()
            }
        }
    }
}
