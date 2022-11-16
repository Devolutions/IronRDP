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
}
