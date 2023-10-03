use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::rdp6::{ABgrChannels, ARgbChannels, BgrAChannels, BitmapStreamEncoder, RgbAChannels};
use ironrdp_pdu::bitmap::{self, BitmapData, BitmapUpdateData, Compression};
use ironrdp_pdu::cursor::WriteCursor;
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_pdu::{PduEncode, PduError};

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
        let row_len = bitmap.width * u32::from(bitmap.format.bytes_per_pixel());
        let chunk_height = u32::from(u16::MAX) / row_len;

        let mut cursor = WriteCursor::new(output);
        let chunks = bitmap.data.chunks((row_len * chunk_height) as usize);

        let total = u16::try_from(chunks.clone().count()).unwrap();
        BitmapUpdateData::encode_header(total, &mut cursor)?;

        for (i, chunk) in chunks.enumerate() {
            let height = u32::try_from(chunk.len()).unwrap() / row_len;
            let top = bitmap.top + u32::try_from(i).unwrap() * chunk_height;

            let encoder = BitmapStreamEncoder::new(bitmap.width as usize, height as usize);

            let len = match bitmap.order {
                PixelOrder::BottomToTop => {
                    Self::encode_slice(encoder, bitmap.format, chunk, self.buffer.as_mut_slice())
                }

                PixelOrder::TopToBottom => {
                    let bytes_per_pixel = bitmap.format.bytes_per_pixel() as usize;
                    let pixels = chunk
                        .chunks(row_len as usize)
                        .rev()
                        .flat_map(|row| row.chunks(bytes_per_pixel));

                    Self::encode_iter(encoder, bitmap.format, pixels, self.buffer.as_mut_slice())
                }
            };

            let data = BitmapData {
                rectangle: InclusiveRectangle {
                    left: u16::try_from(bitmap.left).unwrap(),
                    top: u16::try_from(top).unwrap(),
                    right: u16::try_from(bitmap.left + bitmap.width - 1).unwrap(),
                    bottom: u16::try_from(top + height - 1).unwrap(),
                },
                width: u16::try_from(bitmap.width).unwrap(),
                height: u16::try_from(height).unwrap(),
                bits_per_pixel: u16::from(bitmap.format.bytes_per_pixel()) * 8,
                compression_flags: Compression::BITMAP_COMPRESSION,
                compressed_data_header: Some(bitmap::CompressedDataHeader {
                    main_body_size: u16::try_from(len).unwrap(),
                    scan_width: u16::try_from(bitmap.width).unwrap(),
                    uncompressed_size: u16::try_from(chunk.len()).unwrap(),
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
