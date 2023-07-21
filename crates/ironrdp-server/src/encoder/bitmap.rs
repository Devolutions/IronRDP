use ironrdp_graphics::{
    image_processing::PixelFormat,
    rdp6::{ABgrChannels, ARgbChannels, BgrAChannels, BitmapStreamEncoder, RgbAChannels},
};
use ironrdp_pdu::{
    bitmap::{BitmapData, BitmapUpdateData, Compression},
    fast_path::FastPathUpdate,
    geometry::InclusiveRectangle,
    PduError,
};

use crate::{BitmapUpdate, PixelOrder};

// TODO: this is terrible, improve pdu encoding to avoid unecessary copying

pub struct BitmapEncoder {
    temp: Vec<Vec<u8>>,
    lens: Vec<usize>,
}

impl BitmapEncoder {
    pub fn new() -> Self {
        Self {
            temp: vec![vec![0; u16::MAX as usize]; 128],
            lens: vec![0; 128],
        }
    }

    pub fn encode(&mut self, bitmap: &BitmapUpdate, output: &mut [u8]) -> Result<usize, PduError> {
        let row_len = bitmap.width * bitmap.format.bytes_per_pixel() as u32;
        let chunk_height = u16::MAX as u32 / row_len;

        bitmap
            .data
            .chunks((row_len * chunk_height) as usize)
            .enumerate()
            .for_each(|(i, chunk)| {
                let height = chunk.len() as u32 / row_len;

                let encoder = BitmapStreamEncoder::new(bitmap.width as usize, height as usize);

                match bitmap.order {
                    PixelOrder::BottomToTop => {
                        self.lens[i] = Self::helper(encoder, bitmap.format, chunk, self.temp[i].as_mut_slice());
                    }

                    PixelOrder::TopToBottom => {
                        let bytes_per_pixel = bitmap.format.bytes_per_pixel() as usize;
                        let pixels = chunk
                            .chunks(row_len as usize)
                            .rev()
                            .flat_map(|row| row.chunks(bytes_per_pixel));

                        self.lens[i] = Self::helper_flip(encoder, bitmap.format, pixels, self.temp[i].as_mut_slice());
                    }
                }
            });

        let rectangles = bitmap
            .data
            .chunks((row_len * chunk_height) as usize)
            .enumerate()
            .map(|(i, chunk)| {
                let height = chunk.len() as u32 / row_len;
                let top = bitmap.top + i as u32 * chunk_height;
                let len = self.lens[i];

                BitmapData {
                    rectangle: InclusiveRectangle {
                        left: bitmap.left as u16,
                        top: top as u16,
                        right: (bitmap.left + bitmap.width - 1) as u16,
                        bottom: (top + height - 1) as u16,
                    },
                    width: bitmap.width as u16,
                    height: height as u16,
                    bits_per_pixel: bitmap.format.bytes_per_pixel() as u16 * 8,
                    compression_flags: Compression::BITMAP_COMPRESSION,
                    compressed_data_header: Some(ironrdp_pdu::bitmap::CompressedDataHeader {
                        main_body_size: len as u16,
                        scan_width: bitmap.width as u16,
                        uncompressed_size: chunk.len() as u16,
                    }),
                    bitmap_data: &self.temp[i][..len],
                }
            })
            .collect();

        let update = FastPathUpdate::Bitmap(BitmapUpdateData { rectangles });

        ironrdp_pdu::encode(&update, output)
    }

    fn helper(mut encoder: BitmapStreamEncoder, format: PixelFormat, src: &[u8], dst: &mut [u8]) -> usize {
        match format {
            PixelFormat::ARgb32 | PixelFormat::XRgb32 => encoder.encode_bitmap::<ARgbChannels>(src, dst, true).unwrap(),
            PixelFormat::RgbA32 | PixelFormat::RgbX32 => encoder.encode_bitmap::<RgbAChannels>(src, dst, true).unwrap(),
            PixelFormat::ABgr32 | PixelFormat::XBgr32 => encoder.encode_bitmap::<ABgrChannels>(src, dst, true).unwrap(),
            PixelFormat::BgrA32 | PixelFormat::BgrX32 => encoder.encode_bitmap::<BgrAChannels>(src, dst, true).unwrap(),
        }
    }

    fn helper_flip<'a, P>(mut encoder: BitmapStreamEncoder, format: PixelFormat, src: P, dst: &mut [u8]) -> usize
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
