use std::sync::Arc;

use ironrdp_core::assert_impl;
use ironrdp_graphics::color_conversion::{rdp_15bit_to_rgb, rdp_16bit_to_rgb};
use ironrdp_graphics::image_processing::{ImageRegion, ImageRegionMut, PixelFormat};
use ironrdp_graphics::pointer::DecodedPointer;
use ironrdp_graphics::rectangle_processing::Region;
use ironrdp_pdu::geometry::{InclusiveRectangle, Rectangle as _};
use tracing::{debug, trace};

use crate::{SessionResult, custom_err};

const TILE_SIZE: u16 = 64;

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

    pointer: Option<Arc<DecodedPointer>>,
    /// Image data, overridden by pointer. Used to restore image after pointer was hidden or moved
    pointer_backbuffer: Vec<u8>,
    /// Whether to show pointer or not
    show_pointer: bool,
    /// Whether pointer is visible on the screen or its sprite is currently out of bounds
    pointer_visible_on_screen: bool,

    width: u16,
    height: u16,
}

assert_impl!(DecodedImage: Send);

impl core::fmt::Debug for DecodedImage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecodedImage")
            .field("pixel_format", &self.pixel_format)
            .field("data_len", &self.data.len())
            .field("pointer_src_rect", &self.pointer_src_rect)
            .field("pointer_draw_x", &self.pointer_draw_x)
            .field("pointer_draw_y", &self.pointer_draw_y)
            .field("pointer_x", &self.pointer_x)
            .field("pointer_y", &self.pointer_y)
            .field("pointer", &self.pointer)
            .field("pointer_backbuffer", &self.pointer_backbuffer)
            .field("show_pointer", &self.show_pointer)
            .field("pointer_visible_on_screen", &self.pointer_visible_on_screen)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

#[derive(PartialEq, Eq)]
enum PointerLayer {
    Background,
    Pointer,
}

struct PointerRenderingState {
    redraw: bool,
    update_rectangle: InclusiveRectangle,
}

#[expect(clippy::too_many_arguments)]
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

                #[expect(clippy::as_conversions, reason = "(u16 >> 8) fits into u8 + hot loop")]
                {
                    // Integer alpha blending, source represented as premultiplied alpha color, calculation in floating point
                    to[to_start + pixel * PIXEL_SIZE] =
                        src_r + ((u16::from(dest_r) * u16::from(255 - src_a)) >> 8) as u8;
                    to[to_start + pixel * PIXEL_SIZE + 1] =
                        src_g + ((u16::from(dest_g) * u16::from(255 - src_a)) >> 8) as u8;
                    to[to_start + pixel * PIXEL_SIZE + 2] =
                        src_b + ((u16::from(dest_b) * u16::from(255 - src_a)) >> 8) as u8;
                    // Framebuffer is always opaque, so we can skip alpha channel change
                }
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
            pointer_visible_on_screen: true,
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

    pub fn bytes_per_pixel(&self) -> usize {
        usize::from(self.pixel_format.bytes_per_pixel())
    }

    pub fn stride(&self) -> usize {
        usize::from(self.width) * self.bytes_per_pixel()
    }

    pub fn data_for_rect(&self, rect: &InclusiveRectangle) -> &[u8] {
        if !self.rect_fits(rect) {
            debug!(
                "data_for_rect: rect {:?} does not fit in image {}x{}, returning empty slice",
                rect, self.width, self.height,
            );
            return &self.data[0..0];
        }

        let start = usize::from(rect.left) * self.bytes_per_pixel() + usize::from(rect.top) * self.stride();
        let end =
            start + usize::from(rect.height() - 1) * self.stride() + usize::from(rect.width()) * self.bytes_per_pixel();

        debug_assert!(
            end <= self.data.len(),
            "data_for_rect end {end} exceeds data len {}",
            self.data.len()
        );

        &self.data[start..end]
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    /// Returns `true` if the rectangle fits entirely within the image bounds.
    fn rect_fits(&self, rect: &InclusiveRectangle) -> bool {
        rect.right < self.width && rect.bottom < self.height
    }

    fn apply_pointer_layer(&mut self, layer: PointerLayer) -> SessionResult<Option<InclusiveRectangle>> {
        // Pointer is not hidden, but its texture is not visible on the screen, so we don't
        // need to render it
        if layer == PointerLayer::Pointer && !self.pointer_visible_on_screen {
            return Ok(None);
        }

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

        let pointer_src_rect_width = usize::from(self.pointer_src_rect.width());
        let pointer_src_rect_height = usize::from(self.pointer_src_rect.height());
        let pointer_draw_x = usize::from(self.pointer_draw_x);
        let pointer_draw_y = usize::from(self.pointer_draw_y);
        let width = usize::from(self.width);
        let height = usize::from(self.height);

        match &layer {
            PointerLayer::Background => {
                if self.pointer_backbuffer.is_empty() {
                    // Backbuffer were previously empty
                    return Ok(None);
                }

                copy_cursor_data(
                    &self.pointer_backbuffer,
                    (0, 0),
                    pointer_src_rect_width * 4,
                    &mut self.data,
                    width * 4,
                    (pointer_draw_x, pointer_draw_y),
                    (pointer_src_rect_width, pointer_src_rect_height),
                    (width, height),
                    false,
                );
            }
            PointerLayer::Pointer => {
                // Copy current background to backbuffer
                let buffer_size = self
                    .pointer_backbuffer
                    .len()
                    .max(pointer_src_rect_width * pointer_src_rect_height * 4);
                self.pointer_backbuffer.resize(buffer_size, 0);

                copy_cursor_data(
                    &self.data,
                    (pointer_draw_x, pointer_draw_y),
                    width * 4,
                    &mut self.pointer_backbuffer,
                    pointer_src_rect_width * 4,
                    (0, 0),
                    (pointer_src_rect_width, pointer_src_rect_height),
                    (width, height),
                    false,
                );

                // Draw pointer (with compositing)
                copy_cursor_data(
                    pointer.bitmap_data.as_slice(),
                    (
                        usize::from(self.pointer_src_rect.left),
                        usize::from(self.pointer_src_rect.top),
                    ),
                    usize::from(pointer.width) * 4,
                    &mut self.data,
                    width * 4,
                    (pointer_draw_x, pointer_draw_y),
                    (pointer_src_rect_width, pointer_src_rect_height),
                    (width, height),
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

        let left_virtual = i32::from(x) - i32::from(pointer.hotspot_x);
        let top_virtual = i32::from(y) - i32::from(pointer.hotspot_y);
        let right_virtual = left_virtual + i32::from(pointer.width) - 1;
        let bottom_virtual = top_virtual + i32::from(pointer.height) - 1;

        let (left, draw_x) = if left_virtual < 0 {
            // Cut left side if required
            (pointer.hotspot_x - x, 0)
        } else {
            (0, x - pointer.hotspot_x)
        };

        let (top, draw_y) = if top_virtual < 0 {
            // Cut top side if required
            (pointer.hotspot_y - y, 0)
        } else {
            (0, y - pointer.hotspot_y)
        };

        // Cut right side if required
        let right = if right_virtual >= i32::from(self.width - 1) {
            if draw_x + 1 >= self.width {
                // Pointer is completely out of bounds horizontally
                self.pointer_visible_on_screen = false;
                return;
            } else {
                self.width - (draw_x + 1)
            }
        } else {
            pointer.width - 1
        };

        // Cut bottom side if required
        let bottom = if bottom_virtual >= i32::from(self.height - 1) {
            if (draw_y + 1) >= self.height {
                // Pointer is completely out of bounds vertically
                self.pointer_visible_on_screen = false;
                return;
            } else {
                self.height - (draw_y + 1)
            }
        } else {
            pointer.height - 1
        };

        self.pointer_visible_on_screen = true;

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

    pub(crate) fn update_pointer(&mut self, pointer: Arc<DecodedPointer>) -> SessionResult<Option<InclusiveRectangle>> {
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

    /// This method should be called BEFORE and framebuffer updates, with the update rectangle,
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
        pixel_format: PixelFormat,
        clipping_rectangles: &Region,
        update_rectangle: &InclusiveRectangle,
    ) -> SessionResult<InclusiveRectangle> {
        trace!("Tile: {:?}", update_rectangle);

        if !self.rect_fits(&clipping_rectangles.extents) {
            debug!(
                "Skipping tile update {:?} outside image bounds {}x{}",
                clipping_rectangles.extents, self.width, self.height,
            );
            return Ok(InclusiveRectangle::empty());
        }

        let pointer_rendering_state = self.pointer_rendering_begin(&clipping_rectangles.extents)?;

        let update_region = clipping_rectangles.intersect_rectangle(update_rectangle);
        for region_rectangle in &update_region.rectangles {
            let source_x = region_rectangle.left - update_rectangle.left;
            let source_y = region_rectangle.top - update_rectangle.top;
            let stride = u16::from(pixel_format.bytes_per_pixel()) * TILE_SIZE;
            let source_image_region = ImageRegion {
                region: InclusiveRectangle {
                    left: source_x,
                    top: source_y,
                    right: source_x + region_rectangle.width() - 1,
                    bottom: source_y + region_rectangle.height() - 1,
                },
                data: tile_output,
                step: stride,
                pixel_format,
            };

            let mut destination_image_region = ImageRegionMut {
                region: region_rectangle.clone(),
                step: self.width() * u16::from(self.pixel_format.bytes_per_pixel()),
                pixel_format: self.pixel_format,
                data: &mut self.data,
            };

            trace!("Source image region: {:?}", source_image_region.region);
            trace!("Destination image region: {:?}", destination_image_region.region);

            source_image_region
                .copy_to(&mut destination_image_region)
                .map_err(|e| custom_err!("copy_to", e))?;
        }

        let update_rectangle = self.pointer_rendering_end(pointer_rendering_state)?;

        Ok(update_rectangle)
    }

    pub(crate) fn apply_rgb16_bitmap(
        &mut self,
        rgb16: &[u8],
        update_rectangle: &InclusiveRectangle,
        data_stride: u16,
    ) -> SessionResult<InclusiveRectangle> {
        if !self.rect_fits(update_rectangle) {
            debug!(
                "Skipping rgb16 update {:?} outside image bounds {}x{}",
                update_rectangle, self.width, self.height,
            );
            return Ok(InclusiveRectangle::empty());
        }

        const SRC_COLOR_DEPTH: usize = 2;
        const DST_COLOR_DEPTH: usize = 4;

        let image_width = usize::from(self.width);
        let rectangle_width = usize::from(update_rectangle.width());
        let stride_width = usize::from(data_stride);
        let top = usize::from(update_rectangle.top);
        let left = usize::from(update_rectangle.left);
        let [ri, gi, bi, ai] = self.pixel_format.channel_offsets();

        let pointer_rendering_state = self.pointer_rendering_begin(update_rectangle)?;

        rgb16
            .chunks_exact(stride_width * SRC_COLOR_DEPTH)
            .rev()
            .enumerate()
            .for_each(|(row_idx, row)| {
                row.chunks_exact(SRC_COLOR_DEPTH)
                    .take(rectangle_width)
                    .enumerate()
                    .for_each(|(col_idx, src_pixel)| {
                        let rgb16_value = u16::from_le_bytes(
                            src_pixel
                                .try_into()
                                .expect("src_pixel contains exactly two u8 elements"),
                        );
                        let dst_idx = ((top + row_idx) * image_width + left + col_idx) * DST_COLOR_DEPTH;

                        debug_assert!(
                            dst_idx + DST_COLOR_DEPTH <= self.data.len(),
                            "rgb16 dst_idx out of bounds: {dst_idx} + {DST_COLOR_DEPTH} > {}",
                            self.data.len()
                        );
                        let [r, g, b] = rdp_16bit_to_rgb(rgb16_value);
                        self.data[dst_idx + ri] = r;
                        self.data[dst_idx + gi] = g;
                        self.data[dst_idx + bi] = b;
                        self.data[dst_idx + ai] = 0xff;
                    })
            });

        let update_rectangle = self.pointer_rendering_end(pointer_rendering_state)?;

        Ok(update_rectangle)
    }

    /// Apply a 15-bit (RGB555) bitmap. Bottom-up row order, 2 bytes per pixel.
    pub(crate) fn apply_rgb15_bitmap(
        &mut self,
        rgb15: &[u8],
        update_rectangle: &InclusiveRectangle,
        data_stride: u16,
    ) -> SessionResult<InclusiveRectangle> {
        if !self.rect_fits(update_rectangle) {
            debug!(
                "Skipping rgb15 update {:?} outside image bounds {}x{}",
                update_rectangle, self.width, self.height,
            );
            return Ok(InclusiveRectangle::empty());
        }

        const SRC_COLOR_DEPTH: usize = 2;
        const DST_COLOR_DEPTH: usize = 4;

        let image_width = usize::from(self.width);
        let rectangle_width = usize::from(update_rectangle.width());
        let stride_width = usize::from(data_stride);
        let top = usize::from(update_rectangle.top);
        let left = usize::from(update_rectangle.left);
        let [ri, gi, bi, ai] = self.pixel_format.channel_offsets();

        let pointer_rendering_state = self.pointer_rendering_begin(update_rectangle)?;

        rgb15
            .chunks_exact(stride_width * SRC_COLOR_DEPTH)
            .rev()
            .enumerate()
            .for_each(|(row_idx, row)| {
                row.chunks_exact(SRC_COLOR_DEPTH)
                    .take(rectangle_width)
                    .enumerate()
                    .for_each(|(col_idx, src_pixel)| {
                        let rgb15_value = u16::from_le_bytes(
                            src_pixel
                                .try_into()
                                .expect("src_pixel contains exactly two u8 elements"),
                        );
                        let dst_idx = ((top + row_idx) * image_width + left + col_idx) * DST_COLOR_DEPTH;

                        debug_assert!(
                            dst_idx + DST_COLOR_DEPTH <= self.data.len(),
                            "rgb15 dst_idx out of bounds: {dst_idx} + {DST_COLOR_DEPTH} > {}",
                            self.data.len()
                        );
                        let [r, g, b] = rdp_15bit_to_rgb(rgb15_value);
                        self.data[dst_idx + ri] = r;
                        self.data[dst_idx + gi] = g;
                        self.data[dst_idx + bi] = b;
                        self.data[dst_idx + ai] = 0xff;
                    })
            });

        let update_rectangle = self.pointer_rendering_end(pointer_rendering_state)?;

        Ok(update_rectangle)
    }

    /// Apply a 24-bit BGR bitmap. RLE 24bpp decompresses to BGR byte order,
    /// and uncompressed 24bpp bitmaps are also BGR per MS-RDPBCGR.
    /// Bottom-up row order, 3 bytes per pixel.
    pub(crate) fn apply_bgr24_bitmap(
        &mut self,
        bgr24: &[u8],
        update_rectangle: &InclusiveRectangle,
        data_stride: u16,
    ) -> SessionResult<InclusiveRectangle> {
        if !self.rect_fits(update_rectangle) {
            debug!(
                "Skipping bgr24 update {:?} outside image bounds {}x{}",
                update_rectangle, self.width, self.height,
            );
            return Ok(InclusiveRectangle::empty());
        }

        const SRC_COLOR_DEPTH: usize = 3;
        const DST_COLOR_DEPTH: usize = 4;

        let image_width = usize::from(self.width);
        let rectangle_width = usize::from(update_rectangle.width());
        let stride_width = usize::from(data_stride);
        let top = usize::from(update_rectangle.top);
        let left = usize::from(update_rectangle.left);
        let [ri, gi, bi, ai] = self.pixel_format.channel_offsets();

        let pointer_rendering_state = self.pointer_rendering_begin(update_rectangle)?;

        bgr24
            .chunks_exact(stride_width * SRC_COLOR_DEPTH)
            .rev()
            .enumerate()
            .for_each(|(row_idx, row)| {
                row.chunks_exact(SRC_COLOR_DEPTH)
                    .take(rectangle_width)
                    .enumerate()
                    .for_each(|(col_idx, src_pixel)| {
                        let dst_idx = ((top + row_idx) * image_width + left + col_idx) * DST_COLOR_DEPTH;

                        debug_assert!(
                            dst_idx + DST_COLOR_DEPTH <= self.data.len(),
                            "bgr24 dst_idx out of bounds: {dst_idx} + {DST_COLOR_DEPTH} > {}",
                            self.data.len()
                        );
                        // BGR -> RGB channel swap
                        self.data[dst_idx + ri] = src_pixel[2];
                        self.data[dst_idx + gi] = src_pixel[1];
                        self.data[dst_idx + bi] = src_pixel[0];
                        self.data[dst_idx + ai] = 0xff;
                    })
            });

        let update_rectangle = self.pointer_rendering_end(pointer_rendering_state)?;

        Ok(update_rectangle)
    }

    /// Apply an 8-bit palette-indexed bitmap. Each source byte is a palette index.
    /// Bottom-up row order.
    pub(crate) fn apply_rgb8_with_palette(
        &mut self,
        indexed: &[u8],
        update_rectangle: &InclusiveRectangle,
        palette: &[[u8; 3]; 256],
        data_stride: u16,
    ) -> SessionResult<InclusiveRectangle> {
        if !self.rect_fits(update_rectangle) {
            debug!(
                "Skipping rgb8 update {:?} outside image bounds {}x{}",
                update_rectangle, self.width, self.height,
            );
            return Ok(InclusiveRectangle::empty());
        }

        const DST_COLOR_DEPTH: usize = 4;

        let image_width = usize::from(self.width);
        let rectangle_width = usize::from(update_rectangle.width());
        let stride_width = usize::from(data_stride);
        let top = usize::from(update_rectangle.top);
        let left = usize::from(update_rectangle.left);
        let [ri, gi, bi, ai] = self.pixel_format.channel_offsets();

        let pointer_rendering_state = self.pointer_rendering_begin(update_rectangle)?;

        indexed
            .chunks_exact(stride_width)
            .rev()
            .enumerate()
            .for_each(|(row_idx, row)| {
                row.iter()
                    .take(rectangle_width)
                    .enumerate()
                    .for_each(|(col_idx, &index)| {
                        let dst_idx = ((top + row_idx) * image_width + left + col_idx) * DST_COLOR_DEPTH;
                        debug_assert!(
                            dst_idx + DST_COLOR_DEPTH <= self.data.len(),
                            "rgb8 dst_idx out of bounds: {dst_idx} + {DST_COLOR_DEPTH} > {}",
                            self.data.len()
                        );
                        let [r, g, b] = palette[usize::from(index)];
                        self.data[dst_idx + ri] = r;
                        self.data[dst_idx + gi] = g;
                        self.data[dst_idx + bi] = b;
                        self.data[dst_idx + ai] = 0xff;
                    })
            });

        let update_rectangle = self.pointer_rendering_end(pointer_rendering_state)?;

        Ok(update_rectangle)
    }

    fn apply_rgb24_iter<'a, I>(
        &mut self,
        rgb24: I,
        update_rectangle: &InclusiveRectangle,
    ) -> SessionResult<InclusiveRectangle>
    where
        I: Iterator<Item = &'a [u8]>,
    {
        if !self.rect_fits(update_rectangle) {
            debug!(
                "Skipping rgb24 update {:?} outside image bounds {}x{}",
                update_rectangle, self.width, self.height,
            );
            return Ok(InclusiveRectangle::empty());
        }

        const SRC_COLOR_DEPTH: usize = 3;
        const DST_COLOR_DEPTH: usize = 4;

        let image_width = usize::from(self.width);
        let rectangle_width = usize::from(update_rectangle.width());
        let top = usize::from(update_rectangle.top);
        let left = usize::from(update_rectangle.left);
        let [ri, gi, bi, ai] = self.pixel_format.channel_offsets();

        let pointer_rendering_state = self.pointer_rendering_begin(update_rectangle)?;

        let max_rows = usize::from(update_rectangle.height()).min(usize::from(self.height).saturating_sub(top));

        rgb24.enumerate().take(max_rows).for_each(|(row_idx, row)| {
            row.chunks_exact(SRC_COLOR_DEPTH)
                .take(rectangle_width)
                .enumerate()
                .for_each(|(col_idx, src_pixel)| {
                    let dst_idx = ((top + row_idx) * image_width + left + col_idx) * DST_COLOR_DEPTH;

                    debug_assert!(
                        dst_idx + DST_COLOR_DEPTH <= self.data.len(),
                        "rgb24 dst_idx out of bounds: {dst_idx} + {DST_COLOR_DEPTH} > {}",
                        self.data.len()
                    );
                    self.data[dst_idx + ri] = src_pixel[0];
                    self.data[dst_idx + gi] = src_pixel[1];
                    self.data[dst_idx + bi] = src_pixel[2];
                    self.data[dst_idx + ai] = 0xFF;
                })
        });

        let update_rectangle = self.pointer_rendering_end(pointer_rendering_state)?;

        Ok(update_rectangle)
    }

    pub(crate) fn apply_rgb24(
        &mut self,
        rgb24: &[u8],
        update_rectangle: &InclusiveRectangle,
        data_stride: u16,
        flip: bool,
    ) -> SessionResult<InclusiveRectangle> {
        const SRC_COLOR_DEPTH: usize = 3;
        let stride_width = usize::from(data_stride);
        let lines = rgb24.chunks_exact(stride_width * SRC_COLOR_DEPTH);
        if flip {
            self.apply_rgb24_iter(lines.rev(), update_rectangle)
        } else {
            self.apply_rgb24_iter(lines, update_rectangle)
        }
    }

    pub(crate) fn apply_rgb32_bitmap(
        &mut self,
        rgb32: &[u8],
        format: PixelFormat,
        update_rectangle: &InclusiveRectangle,
        data_stride: u16,
    ) -> SessionResult<InclusiveRectangle> {
        if !self.rect_fits(update_rectangle) {
            debug!(
                "Skipping rgb32 update {:?} outside image bounds {}x{}",
                update_rectangle, self.width, self.height,
            );
            return Ok(InclusiveRectangle::empty());
        }

        const SRC_COLOR_DEPTH: usize = 4;
        const DST_COLOR_DEPTH: usize = 4;

        let image_width = usize::from(self.width);
        let rectangle_width = usize::from(update_rectangle.width());
        let stride_width = usize::from(data_stride);
        let top = usize::from(update_rectangle.top);
        let left = usize::from(update_rectangle.left);

        let pointer_rendering_state = self.pointer_rendering_begin(update_rectangle)?;

        if format == self.pixel_format {
            rgb32
                .chunks_exact(stride_width * SRC_COLOR_DEPTH)
                .rev()
                .enumerate()
                .for_each(|(row_idx, row)| {
                    row.chunks_exact(SRC_COLOR_DEPTH)
                        .take(rectangle_width)
                        .enumerate()
                        .for_each(|(col_idx, src_pixel)| {
                            let dst_idx = ((top + row_idx) * image_width + left + col_idx) * DST_COLOR_DEPTH;

                            debug_assert!(
                                dst_idx + SRC_COLOR_DEPTH <= self.data.len(),
                                "rgb32 same-format dst_idx out of bounds: {dst_idx} + {SRC_COLOR_DEPTH} > {}",
                                self.data.len()
                            );
                            self.data[dst_idx..dst_idx + SRC_COLOR_DEPTH].copy_from_slice(src_pixel);
                        })
                });
        } else {
            let [ri, gi, bi, ai] = self.pixel_format.channel_offsets();
            rgb32
                .chunks_exact(stride_width * SRC_COLOR_DEPTH)
                .rev()
                .enumerate()
                .try_for_each(|(row_idx, row)| {
                    row.chunks_exact(SRC_COLOR_DEPTH)
                        .take(rectangle_width)
                        .enumerate()
                        .try_for_each(|(col_idx, src_pixel)| {
                            let dst_idx = ((top + row_idx) * image_width + left + col_idx) * DST_COLOR_DEPTH;

                            debug_assert!(
                                dst_idx + DST_COLOR_DEPTH <= self.data.len(),
                                "rgb32 cross-format dst_idx out of bounds: {dst_idx} + {DST_COLOR_DEPTH} > {}",
                                self.data.len()
                            );
                            let c = format
                                .read_color(src_pixel)
                                .map_err(|err| custom_err!("read color", err))?;

                            self.data[dst_idx + ri] = c.r;
                            self.data[dst_idx + gi] = c.g;
                            self.data[dst_idx + bi] = c.b;
                            self.data[dst_idx + ai] = c.a;

                            Ok(())
                        })?;

                    Ok(())
                })?;
        }

        let update_rectangle = self.pointer_rendering_end(pointer_rendering_state)?;

        Ok(update_rectangle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construct a DecodedImage with a known pixel format and dimensions.
    fn make_image(width: u16, height: u16) -> DecodedImage {
        DecodedImage::new(PixelFormat::RgbA32, width, height)
    }

    /// Regression test for bitmap rendering correctness.
    ///
    /// Verifies that apply_rgb32_bitmap writes pixels to the correct
    /// framebuffer positions using the blit_rect as the row stride.
    /// This validates that the fast_path.rs blit_rect fix results in
    /// correct pixel placement.
    #[test]
    fn apply_rgb32_correct_pixel_placement() {
        let mut image = make_image(10, 10);

        // 3×2 bitmap data in RgbA32 (same format as the framebuffer).
        // 3 pixels per row, 2 rows. Bottom-up order (row 0 = bottom).
        let rect_width: usize = 3;
        let rect_height: usize = 2;
        let bpp: usize = 4;
        let mut bitmap_data = vec![0u8; rect_width * rect_height * bpp];

        // RgbA32 channel layout: [R, G, B, A]
        // Fill row 0 (bottom row in bitmap → last screen row after .rev())
        for col in 0..rect_width {
            let idx = col * bpp;
            bitmap_data[idx] = 0xAA; // R
            bitmap_data[idx + 1] = 0xBB; // G
            bitmap_data[idx + 2] = 0xCC; // B
            bitmap_data[idx + 3] = 0xFF; // A
        }
        // Fill row 1 (top row in bitmap → first screen row after .rev())
        for col in 0..rect_width {
            let idx = (rect_width + col) * bpp;
            bitmap_data[idx] = 0x11; // R
            bitmap_data[idx + 1] = 0x22; // G
            bitmap_data[idx + 2] = 0x33; // B
            bitmap_data[idx + 3] = 0xFF; // A
        }

        let blit_rect = InclusiveRectangle {
            left: 2,
            top: 3,
            right: 4,  // width = 3
            bottom: 4, // height = 2
        };

        // data_stride == rect width (no padding)
        let result = image.apply_rgb32_bitmap(&bitmap_data, PixelFormat::RgbA32, &blit_rect, 3);
        assert!(result.is_ok());

        let stride = 10 * 4; // image width * bpp

        // After .rev(), bitmap row 1 → screen row_idx 0 (y = top = 3).
        // First pixel at (x=2, y=3): should be [0x11, 0x22, 0x33, 0xFF]
        let px = 3 * stride + 2 * 4;
        assert_eq!(image.data[px], 0x11, "R channel at (2,3)");
        assert_eq!(image.data[px + 1], 0x22, "G channel at (2,3)");
        assert_eq!(image.data[px + 2], 0x33, "B channel at (2,3)");
        assert_eq!(image.data[px + 3], 0xFF, "A channel at (2,3)");

        // After .rev(), bitmap row 0 → screen row_idx 1 (y = 4).
        // First pixel at (x=2, y=4): should be [0xAA, 0xBB, 0xCC, 0xFF]
        let px2 = 4 * stride + 2 * 4;
        assert_eq!(image.data[px2], 0xAA, "R channel at (2,4)");
        assert_eq!(image.data[px2 + 1], 0xBB, "G channel at (2,4)");

        // Pixel at column 5 (= left + rect_width) should NOT be written.
        let px_outside = 3 * stride + 5 * 4;
        assert_eq!(image.data[px_outside], 0, "pixel at col 5 should be untouched");
    }

    /// Basic in-bounds write test for apply_rgb16_bitmap.
    ///
    /// Fills the entire 4×4 image with a known RGB565 color and verifies
    /// that pixels are correctly written.
    #[test]
    fn apply_rgb16_basic_in_bounds_write() {
        let mut image = make_image(4, 4);

        let rect = InclusiveRectangle {
            left: 0,
            top: 0,
            right: 3,
            bottom: 3,
        };

        // 4×4 RGB16 bitmap: 2 bytes per pixel, 32 bytes total.
        // Use a known 16-bit color value: 0xFFFF (white in RGB565).
        let bitmap_data = vec![0xFF; 4 * 4 * 2];

        let result = image.apply_rgb16_bitmap(&bitmap_data, &rect, 4);
        assert!(result.is_ok());

        // Verify that pixel (0,0) was written (should be white-ish from RGB565 0xFFFF).
        let px = 0;
        assert_ne!(image.data[px], 0, "pixel (0,0) R should be non-zero");
        assert_eq!(image.data[px + 3], 0xFF, "pixel (0,0) A should be 0xFF");
    }

    /// Regression test: rectangle that does NOT fit should be silently skipped.
    #[test]
    fn apply_rgb16_rect_exceeds_image_returns_empty() {
        let mut image = make_image(4, 4);

        // This rectangle extends past the image (right=5 >= width=4).
        let rect = InclusiveRectangle {
            left: 2,
            top: 2,
            right: 5,
            bottom: 5,
        };

        let bitmap_data = vec![0xFF; 4 * 4 * 2];
        let result = image.apply_rgb16_bitmap(&bitmap_data, &rect, 4);
        assert!(result.is_ok());

        // rect_fits returns false, so InclusiveRectangle::empty() is returned
        // and no pixels are written. Note: InclusiveRectangle::empty() has
        // all fields = 0, so width() = right - left + 1 = 1.
        let update_rect = result.unwrap();
        assert_eq!(update_rect.left, 0);
        assert_eq!(update_rect.top, 0);
        assert_eq!(update_rect.right, 0);
        assert_eq!(update_rect.bottom, 0);
    }

    /// Regression test: data_for_rect returns empty slice for out-of-bounds rect.
    #[test]
    fn data_for_rect_returns_empty_for_oob() {
        let image = make_image(4, 4);

        // Rectangle larger than the image
        let rect = InclusiveRectangle {
            left: 0,
            top: 0,
            right: 10,
            bottom: 10,
        };

        // Should not panic — returns empty slice since rect doesn't fit
        let data = image.data_for_rect(&rect);
        assert_eq!(data.len(), 0, "out-of-bounds rect should return empty slice");
    }

    /// Regression test for asymmetric stride handling (MS-RDPBCGR §2.2.9.1.1.3.1.2.2).
    ///
    /// When bitmap data is wider than the destination rectangle (e.g. xRDP padding
    /// bitmapWidth to a 4-byte alignment boundary), the extra columns must be
    /// discarded. This test constructs a bitmap with data_stride=8 but a dest rect
    /// width of 5. Only the first 5 columns per row should be written.
    #[test]
    fn apply_rgb32_stride_wider_than_rect_discards_extra_columns() {
        let mut image = make_image(10, 10);

        // Bitmap data: 8 pixels wide, 2 rows tall (bottom-up).
        // Dest rect: only 5 pixels wide.
        let data_stride: usize = 8;
        let rect_width: usize = 5;
        let rect_height: usize = 2;
        let bpp: usize = 4;
        let mut bitmap_data = vec![0u8; data_stride * rect_height * bpp];

        // Fill all pixels with a known pattern.
        // Columns 0-4 (within dest rect): 0x11 per channel.
        // Columns 5-7 (padding, should be discarded): 0xEE per channel.
        for row in 0..rect_height {
            for col in 0..data_stride {
                let idx = (row * data_stride + col) * bpp;
                let value = if col < rect_width { 0x11 } else { 0xEE };
                bitmap_data[idx] = value;
                bitmap_data[idx + 1] = value;
                bitmap_data[idx + 2] = value;
                bitmap_data[idx + 3] = 0xFF;
            }
        }

        let blit_rect = InclusiveRectangle {
            left: 1,
            top: 2,
            right: 5,  // width = 5
            bottom: 3, // height = 2
        };

        let result = image.apply_rgb32_bitmap(
            &bitmap_data,
            PixelFormat::RgbA32,
            &blit_rect,
            u16::try_from(data_stride).unwrap(),
        );
        assert!(result.is_ok());

        let img_stride = 10 * 4; // image width * bpp

        // Verify in-bounds columns (0-4 of dest rect, at x=1..5) were written.
        // After .rev(), bitmap row 1 → screen row_idx 0 (y=2).
        for col in 0..rect_width {
            let px = 2 * img_stride + (1 + col) * 4;
            assert_eq!(image.data[px], 0x11, "pixel at col {col} should be 0x11 (within rect)");
        }

        // Verify that column 6 (x = 1 + 5 = 6) was NOT written (padding was discarded).
        let px_padding = 2 * img_stride + 6 * 4;
        assert_eq!(
            image.data[px_padding], 0,
            "pixel at x=6 should be untouched (padding discarded)"
        );
    }
}
