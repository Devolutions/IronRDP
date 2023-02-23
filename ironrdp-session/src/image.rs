use ironrdp_core::geometry::Rectangle;
use ironrdp_graphics::image_processing::{ImageRegion, ImageRegionMut, PixelFormat};
use ironrdp_graphics::rectangle_processing::Region;

use crate::RdpError;

const TILE_SIZE: u16 = 64;
const SOURCE_PIXEL_FORMAT: PixelFormat = PixelFormat::BgrX32;
const SOURCE_STRIDE: u16 = TILE_SIZE * SOURCE_PIXEL_FORMAT.bytes_per_pixel() as u16;

pub struct DecodedImage {
    pixel_format: PixelFormat,
    data: Vec<u8>,
    width: u32,
    height: u32,
}

impl DecodedImage {
    pub fn new(pixel_format: PixelFormat, width: u32, height: u32) -> Self {
        let len = usize::try_from(width).unwrap()
            * usize::try_from(height).unwrap()
            * usize::from(pixel_format.bytes_per_pixel());

        Self {
            pixel_format,
            data: vec![0; len],
            width,
            height,
        }
    }

    pub fn pixel_format(&self) -> PixelFormat {
        self.pixel_format
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub(crate) fn apply_tile(
        &mut self,
        tile_output: &[u8],
        clipping_rectangles: &Region,
        update_rectangle: &Rectangle,
        width: u16,
    ) -> Result<(), RdpError> {
        debug!("Tile: {:?}", update_rectangle);

        let update_region = clipping_rectangles.intersect_rectangle(update_rectangle);
        for region_rectangle in &update_region.rectangles {
            let source_x = region_rectangle.left - update_rectangle.left;
            let source_y = region_rectangle.top - update_rectangle.top;
            let source_image_region = ImageRegion {
                region: Rectangle {
                    left: source_x,
                    top: source_y,
                    right: source_x + region_rectangle.width(),
                    bottom: source_y + region_rectangle.height(),
                },
                step: SOURCE_STRIDE,
                pixel_format: SOURCE_PIXEL_FORMAT,
                data: tile_output,
            };

            let mut destination_image_region = ImageRegionMut {
                region: region_rectangle.clone(),
                step: width * u16::from(self.pixel_format.bytes_per_pixel()),
                pixel_format: self.pixel_format,
                data: &mut self.data,
            };

            debug!("Source image region: {:?}", source_image_region.region);
            debug!("Destination image region: {:?}", destination_image_region.region);

            source_image_region.copy_to(&mut destination_image_region)?;
        }

        Ok(())
    }

    pub(crate) fn apply_rgb16_bitmap(&mut self, rgb16: &[u8], update_rectangle: &Rectangle) {
        const SRC_COLOR_DEPTH: usize = 2;
        const DST_COLOR_DEPTH: usize = 4;

        let image_width = self.width as usize;
        let rectangle_width = usize::from(update_rectangle.width()) + 1;
        let top = usize::from(update_rectangle.top);
        let left = usize::from(update_rectangle.left);

        rgb16
            .chunks_exact(rectangle_width * SRC_COLOR_DEPTH)
            .rev()
            .enumerate()
            .for_each(|(row_idx, row)| {
                row.chunks_exact(SRC_COLOR_DEPTH)
                    .enumerate()
                    .for_each(|(col_idx, src_pixel)| {
                        let rgb16_value = u16::from_le_bytes(src_pixel.try_into().unwrap());
                        let dst_idx = ((top + row_idx) * image_width + left + col_idx) * DST_COLOR_DEPTH;

                        self.data[dst_idx] = (((((rgb16_value >> 11) & 0x1f) * 527) + 23) >> 6) as u8;
                        self.data[dst_idx + 1] = (((((rgb16_value >> 5) & 0x3f) * 259) + 33) >> 6) as u8;
                        self.data[dst_idx + 2] = ((((rgb16_value & 0x1f) * 527) + 23) >> 6) as u8;
                        self.data[dst_idx + 3] = 0xff;
                    })
            });
    }
}
