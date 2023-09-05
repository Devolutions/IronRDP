use std::rc::Rc;

use ironrdp_graphics::color_conversion::rdp_16bit_to_rgb;
use ironrdp_graphics::image_processing::{ImageRegion, ImageRegionMut, PixelFormat};
use ironrdp_graphics::pointer::DecodedPointer;
use ironrdp_graphics::rectangle_processing::Region;
use ironrdp_pdu::geometry::{InclusiveRectangle, Rectangle as _};

use crate::SessionResult;

const TILE_SIZE: u16 = 64;
const SOURCE_PIXEL_FORMAT: PixelFormat = PixelFormat::BgrX32;
const SOURCE_STRIDE: u16 = TILE_SIZE * SOURCE_PIXEL_FORMAT.bytes_per_pixel() as u16;

pub struct DecodedImage {
    pixel_format: PixelFormat,
    data: Vec<u8>,

    /// Part of the pointer image which should be drawn
    pointer_src_rect: InclusiveRectangle,
    /// X position of the pointer sprite on the screen
    pointer_draw_x: u16,
    /// Y position of the pointer sprite on the screen
    pointer_draw_y: u16,

    pointer_x: u16,
    pointer_y: u16,

    pointer: Option<Rc<DecodedPointer>>,
    /// Image data, overridden by pointer. Used to restore image after pointer was hidden or moved
    pointer_backbuffer: Vec<u8>,
    /// Whether to show pointer or not
    show_pointer: bool,

    width: u16,
    height: u16,
}

enum PointerLayer {
    Background,
    Pointer,
}

struct PointerRenderingState {
    redraw: bool,
    update_rectangle: InclusiveRectangle,
}

#[allow(clippy::too_many_arguments)]
fn copy_cursor_data(
    from: &[u8],
    from_pos: (usize, usize),
    from_stride: usize,
    to: &mut [u8],
    to_stride: usize,
    to_pos: (usize, usize),
    size: (usize, usize),
    dst_size: (usize, usize),
    composite: bool,
) {
    const PIXEL_SIZE: usize = 4;

    if to_pos.0 + size.0 > dst_size.0 || to_pos.1 + size.1 > dst_size.1 {
        // Perform clipping
        return;
    }

    let (from_x, from_y) = from_pos;
    let (to_x, to_y) = to_pos;
    let (width, height) = size;

    for y in 0..height {
        let from_start = (from_y + y) * from_stride + from_x * PIXEL_SIZE;
        let to_start = (to_y + y) * to_stride + to_x * PIXEL_SIZE;

        if composite {
            for pixel in 0..width {
                let dest_r = to[to_start + pixel * PIXEL_SIZE];
                let dest_g = to[to_start + pixel * PIXEL_SIZE + 1];
                let dest_b = to[to_start + pixel * PIXEL_SIZE + 2];

                let src_r = from[from_start + pixel * PIXEL_SIZE];
                let src_g = from[from_start + pixel * PIXEL_SIZE + 1];
                let src_b = from[from_start + pixel * PIXEL_SIZE + 2];
                let src_a = from[from_start + pixel * PIXEL_SIZE + 3];

                // Inverted pixel, this color has a special meaning when encoded by ironrdp-graphics
                if src_a == 0 && src_r == 255 && src_g == 255 && src_b == 255 {
                    to[to_start + pixel * PIXEL_SIZE] = 255 - dest_r;
                    to[to_start + pixel * PIXEL_SIZE + 1] = 255 - dest_g;
                    to[to_start + pixel * PIXEL_SIZE + 2] = 255 - dest_b;
                    to[to_start + pixel * PIXEL_SIZE + 3] = 255;
                    continue;
                }

                // Skip 100% transparent pixels
                if src_a == 0 {
                    continue;
                }

                // Integer alpha blending, source represented as premultiplied alpha color, calculation in floating point
                to[to_start + pixel * PIXEL_SIZE] = src_r + (((dest_r as u16) * (255 - src_a) as u16) >> 8) as u8;
                to[to_start + pixel * PIXEL_SIZE + 1] = src_g + (((dest_g as u16) * (255 - src_a) as u16) >> 8) as u8;
                to[to_start + pixel * PIXEL_SIZE + 2] = src_b + (((dest_b as u16) * (255 - src_a) as u16) >> 8) as u8;
                // Framebuffer is always opaque, so we can skip alpha channel change
            }
        } else {
            to[to_start..to_start + width * PIXEL_SIZE]
                .copy_from_slice(&from[from_start..from_start + width * PIXEL_SIZE]);
        }
    }
}

impl DecodedImage {
    pub fn new(pixel_format: PixelFormat, width: u16, height: u16) -> Self {
        let len = usize::from(width) * usize::from(height) * usize::from(pixel_format.bytes_per_pixel());

        Self {
            pixel_format,
            data: vec![0; len],
            width,
            height,

            pointer_src_rect: InclusiveRectangle {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            pointer_x: 0,
            pointer_y: 0,
            pointer_draw_x: 0,
            pointer_draw_y: 0,
            pointer_backbuffer: Vec::new(),
            pointer: None,
            show_pointer: false,
        }
    }

    pub fn pixel_format(&self) -> PixelFormat {
        self.pixel_format
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    fn apply_pointer_layer(&mut self, layer: PointerLayer) -> SessionResult<Option<InclusiveRectangle>> {
        if self.data.is_empty() {
            return Ok(None);
        }

        let pointer = if let Some(pointer) = &self.pointer {
            pointer
        } else {
            return Ok(None);
        };

        if self.pointer_src_rect.width() == 0 || self.pointer_src_rect.height() == 0 {
            return Ok(None);
        }

        let dest_rect = InclusiveRectangle {
            left: self.pointer_draw_x,
            top: self.pointer_draw_y,
            right: self.pointer_draw_x + self.pointer_src_rect.width() - 1,
            bottom: self.pointer_draw_y + self.pointer_src_rect.height() - 1,
        };

        if dest_rect.width() == 0 || dest_rect.height() == 0 {
            return Ok(None);
        }

        match &layer {
            PointerLayer::Background => {
                if self.pointer_backbuffer.is_empty() {
                    // Backbuffer were previously empty
                    return Ok(None);
                }

                copy_cursor_data(
                    &self.pointer_backbuffer,
                    (0, 0),
                    self.pointer_src_rect.width() as usize * 4,
                    &mut self.data,
                    self.width as usize * 4,
                    (self.pointer_draw_x as usize, self.pointer_draw_y as usize),
                    (
                        self.pointer_src_rect.width() as usize,
                        self.pointer_src_rect.height() as usize,
                    ),
                    (self.width as usize, self.height as usize),
                    false,
                );
            }
            PointerLayer::Pointer => {
                // Copy current background to backbuffer
                let buffer_size = self
                    .pointer_backbuffer
                    .len()
                    .max(self.pointer_src_rect.width() as usize * self.pointer_src_rect.height() as usize * 4);
                self.pointer_backbuffer.resize(buffer_size, 0);

                copy_cursor_data(
                    &self.data,
                    (self.pointer_draw_x as usize, self.pointer_draw_y as usize),
                    self.width as usize * 4,
                    &mut self.pointer_backbuffer,
                    self.pointer_src_rect.width() as usize * 4,
                    (0, 0),
                    (
                        self.pointer_src_rect.width() as usize,
                        self.pointer_src_rect.height() as usize,
                    ),
                    (self.width as usize, self.height as usize),
                    false,
                );

                // Draw pointer (with compositing)
                copy_cursor_data(
                    pointer.bitmap_data.as_slice(),
                    (self.pointer_src_rect.left as usize, self.pointer_src_rect.top as usize),
                    pointer.width * 4,
                    &mut self.data,
                    self.width as usize * 4,
                    (self.pointer_draw_x as usize, self.pointer_draw_y as usize),
                    (
                        self.pointer_src_rect.width() as usize,
                        self.pointer_src_rect.height() as usize,
                    ),
                    (self.width as usize, self.height as usize),
                    true,
                );
            }
        }

        // Request redraw of the changed area
        Ok(Some(dest_rect))
    }

    pub(crate) fn show_pointer(&mut self) -> SessionResult<Option<InclusiveRectangle>> {
        if !self.show_pointer {
            self.show_pointer = true;
            self.apply_pointer_layer(PointerLayer::Pointer)
        } else {
            Ok(None)
        }
    }

    pub(crate) fn hide_pointer(&mut self) -> SessionResult<Option<InclusiveRectangle>> {
        if self.show_pointer {
            self.show_pointer = false;
            self.apply_pointer_layer(PointerLayer::Background)
        } else {
            Ok(None)
        }
    }

    fn recalculate_pointer_geometry(&mut self) {
        let x = self.pointer_x;
        let y = self.pointer_y;

        let pointer = match &self.pointer {
            Some(pointer) if self.show_pointer => pointer,
            _ => return,
        };

        let left_virtual = x as i16 - pointer.hot_spot_x as i16;
        let top_virtual = y as i16 - pointer.hot_spot_y as i16;
        let right_virtual = left_virtual + pointer.width as i16 - 1;
        let bottom_virtual = top_virtual + pointer.height as i16 - 1;

        let (left, draw_x) = if left_virtual < 0 {
            // Cut left side if required
            (pointer.hot_spot_x as u16 - x, 0)
        } else {
            (0, x - pointer.hot_spot_x as u16)
        };

        let (top, draw_y) = if top_virtual < 0 {
            // Cut top side if required
            (pointer.hot_spot_y as u16 - y, 0)
        } else {
            (0, y - pointer.hot_spot_y as u16)
        };

        let right = if right_virtual >= (self.width - 1) as i16 {
            // Cut right side if required
            self.width - draw_x - 1
        } else {
            pointer.width as u16 - 1
        };

        let bottom = if bottom_virtual >= (self.height - 1) as i16 {
            // Cut bottom side if required
            self.height - draw_y - 1
        } else {
            pointer.height as u16 - 1
        };

        let pointer_src_rect = InclusiveRectangle {
            left,
            top,
            right,
            bottom,
        };

        self.pointer_src_rect = pointer_src_rect;
        self.pointer_draw_x = draw_x;
        self.pointer_draw_y = draw_y;
    }

    pub(crate) fn move_pointer(&mut self, x: u16, y: u16) -> SessionResult<Option<InclusiveRectangle>> {
        self.pointer_x = x;
        self.pointer_y = y;

        if self.pointer.is_some() && self.show_pointer {
            let old_rect = self.apply_pointer_layer(PointerLayer::Background)?;
            self.recalculate_pointer_geometry();
            let new_rect = self.apply_pointer_layer(PointerLayer::Pointer)?;

            match (old_rect, new_rect) {
                (None, None) => Ok(None),
                (None, Some(rect)) => Ok(Some(rect)),
                (Some(rect), None) => Ok(Some(rect)),
                (Some(a), Some(b)) => Ok(Some(a.union(&b))),
            }
        } else {
            Ok(None)
        }
    }

    pub(crate) fn update_pointer(&mut self, pointer: Rc<DecodedPointer>) -> SessionResult<Option<InclusiveRectangle>> {
        self.show_pointer = true;

        // Remove old pointer from frame buffer
        let old_rect = if self.pointer.is_some() {
            self.apply_pointer_layer(PointerLayer::Background)?
        } else {
            None
        };

        self.pointer = Some(pointer);
        self.recalculate_pointer_geometry();

        // Draw new pointer
        let new_rect = self.apply_pointer_layer(PointerLayer::Pointer)?;

        match (old_rect, new_rect) {
            (None, None) => Ok(None),
            (None, Some(rect)) => Ok(Some(rect)),
            (Some(rect), None) => Ok(Some(rect)),
            (Some(a), Some(b)) => Ok(Some(a.union(&b))),
        }
    }

    fn is_pointer_redraw_required(&self, update_rectangle: &InclusiveRectangle) -> bool {
        let pointer_dest_rect = InclusiveRectangle {
            left: self.pointer_draw_x,
            top: self.pointer_draw_y,
            right: self.pointer_draw_x + self.pointer_src_rect.width() - 1,
            bottom: self.pointer_draw_y + self.pointer_src_rect.height() - 1,
        };

        update_rectangle.intersect(&pointer_dest_rect).is_some() && self.show_pointer
    }

    /// This method should be called BEFORE and fraebuffer updates, with the update rectangle,
    /// to determine if the pointer needs to be redrawn (overlapping with the update rectangle).
    fn pointer_rendering_begin(
        &mut self,
        update_rectangle: &InclusiveRectangle,
    ) -> SessionResult<PointerRenderingState> {
        if !self.is_pointer_redraw_required(update_rectangle) || self.pointer.is_none() {
            return Ok(PointerRenderingState {
                redraw: false,
                update_rectangle: update_rectangle.clone(),
            });
        }

        let state = self
            .apply_pointer_layer(PointerLayer::Background)?
            .map(|cursor_erase_rect| PointerRenderingState {
                redraw: true,
                update_rectangle: cursor_erase_rect.union(update_rectangle),
            })
            .unwrap_or_else(|| PointerRenderingState {
                redraw: false,
                update_rectangle: update_rectangle.clone(),
            });

        Ok(state)
    }

    fn pointer_rendering_end(
        &mut self,
        pointer_rendering_state: PointerRenderingState,
    ) -> SessionResult<InclusiveRectangle> {
        if !pointer_rendering_state.redraw {
            return Ok(pointer_rendering_state.update_rectangle);
        }

        let update_rectangle = self
            .apply_pointer_layer(PointerLayer::Pointer)?
            .map(|pointer_draw_rectangle| pointer_draw_rectangle.union(&pointer_rendering_state.update_rectangle))
            .unwrap_or_else(|| pointer_rendering_state.update_rectangle);

        Ok(update_rectangle)
    }

    // To apply the buffer, we need to un-apply previously drawn cursor, and then apply it again
    // in other position.

    pub(crate) fn apply_tile(
        &mut self,
        tile_output: &[u8],
        clipping_rectangles: &Region,
        update_rectangle: &InclusiveRectangle,
        width: u16,
    ) -> SessionResult<InclusiveRectangle> {
        debug!("Tile: {:?}", update_rectangle);

        let pointer_rendering_state = self.pointer_rendering_begin(&clipping_rectangles.extents)?;

        let update_region = clipping_rectangles.intersect_rectangle(update_rectangle);
        for region_rectangle in &update_region.rectangles {
            let source_x = region_rectangle.left - update_rectangle.left;
            let source_y = region_rectangle.top - update_rectangle.top;
            let source_image_region = ImageRegion {
                region: InclusiveRectangle {
                    left: source_x,
                    top: source_y,
                    right: source_x + region_rectangle.width() - 1,
                    bottom: source_y + region_rectangle.height() - 1,
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

            source_image_region
                .copy_to(&mut destination_image_region)
                .map_err(|e| custom_err!("copy_to", e))?;
        }

        let update_rectangle = self.pointer_rendering_end(pointer_rendering_state)?;

        Ok(update_rectangle)
    }

    // FIXME: this assumes PixelFormat::RgbA32
    pub(crate) fn apply_rgb16_bitmap(
        &mut self,
        rgb16: &[u8],
        update_rectangle: &InclusiveRectangle,
    ) -> SessionResult<InclusiveRectangle> {
        const SRC_COLOR_DEPTH: usize = 2;
        const DST_COLOR_DEPTH: usize = 4;

        let image_width = self.width as usize;
        let rectangle_width = usize::from(update_rectangle.width());
        let top = usize::from(update_rectangle.top);
        let left = usize::from(update_rectangle.left);

        let pointer_rendering_state = self.pointer_rendering_begin(update_rectangle)?;

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

                        let [r, g, b] = rdp_16bit_to_rgb(rgb16_value);
                        self.data[dst_idx] = r;
                        self.data[dst_idx + 1] = g;
                        self.data[dst_idx + 2] = b;
                        self.data[dst_idx + 3] = 0xff;
                    })
            });

        let update_rectangle = self.pointer_rendering_end(pointer_rendering_state)?;

        Ok(update_rectangle)
    }

    // FIXME: this assumes PixelFormat::RgbA32
    pub(crate) fn apply_rgb24_bitmap(
        &mut self,
        rgb24: &[u8],
        update_rectangle: &InclusiveRectangle,
    ) -> SessionResult<InclusiveRectangle> {
        const SRC_COLOR_DEPTH: usize = 3;
        const DST_COLOR_DEPTH: usize = 4;

        let image_width = self.width as usize;
        let rectangle_width = usize::from(update_rectangle.width());
        let top = usize::from(update_rectangle.top);
        let left = usize::from(update_rectangle.left);

        let pointer_rendering_state = self.pointer_rendering_begin(update_rectangle)?;

        rgb24
            .chunks_exact(rectangle_width * SRC_COLOR_DEPTH)
            .rev()
            .enumerate()
            .for_each(|(row_idx, row)| {
                row.chunks_exact(SRC_COLOR_DEPTH)
                    .enumerate()
                    .for_each(|(col_idx, src_pixel)| {
                        let dst_idx = ((top + row_idx) * image_width + left + col_idx) * DST_COLOR_DEPTH;

                        // Copy RGB channels as is
                        self.data[dst_idx..dst_idx + SRC_COLOR_DEPTH].copy_from_slice(src_pixel);
                        // Set alpha channel to opaque(0xFF)
                        self.data[dst_idx + 3] = 0xFF;
                    })
            });

        let update_rectangle = self.pointer_rendering_end(pointer_rendering_state)?;

        Ok(update_rectangle)
    }
}
