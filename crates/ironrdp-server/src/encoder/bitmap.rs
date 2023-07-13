use ironrdp_pdu::{
    bitmap::{BitmapData, BitmapUpdateData, Compression},
    fast_path::FastPathUpdate,
    geometry::Rectangle,
    rdp::{client_info::CompressionType, headers::CompressionFlags},
};

use crate::BitmapUpdate;

use super::UpdateHandler;

pub struct UncompressedBitmapHandler {}

impl UpdateHandler for UncompressedBitmapHandler {
    fn handle<'a>(&mut self, bitmap: &'a BitmapUpdate) -> Option<FastPathUpdate<'a>> {
        let row_len = bitmap.width * bitmap.bits_per_pixel as u32 / 8;
        let chunk_height = u16::MAX as u32 / row_len;

        Some(FastPathUpdate::Bitmap(BitmapUpdateData {
            rectangles: bitmap
                .data
                .chunks((row_len * chunk_height) as usize)
                .enumerate()
                .map(|(i, chunk)| {
                    let height = chunk.len() as u32 / row_len;
                    let top = bitmap.top + i as u32 * chunk_height;

                    BitmapData {
                        rectangle: Rectangle {
                            left: bitmap.left as u16,
                            top: top as u16,
                            right: (bitmap.left + bitmap.width - 1) as u16,
                            bottom: (top + height - 1) as u16,
                        },
                        width: bitmap.width as u16,
                        height: height as u16,
                        bits_per_pixel: bitmap.bits_per_pixel,
                        compression_flags: Compression::empty(),
                        bitmap_data_length: chunk.len(),
                        compressed_data_header: None,
                        bitmap_data: chunk,
                    }
                })
                .collect(),
        }))
    }

    fn compression(&self) -> Option<(CompressionFlags, CompressionType)> {
        None
    }
}
