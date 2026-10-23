//! AVC444 and AVC444v2 view combination
//!
//! An AVC444 picture travels as two YUV 4:2:0 pictures decoded by one H.264 decoder
//! ([MS-RDPEGFX] 2.2.4.5, "RFX_AVC444_BITMAP_STREAM"): the main view carries luma and
//! subsampled chroma, and the auxiliary view carries the remaining chroma samples.
//! [`Yuv444Frame`] holds the combined picture of one surface. A luma-only or chroma-only
//! update combines with what earlier updates left, as [MS-RDPEGFX] 3.3.8.3.2 ("YUV420p
//! Stream Combination for YUV444 mode") and 3.3.8.3.3 ("YUV420p Stream Combination for
//! YUV444v2 mode") require.
//!
//! The same type splits a picture into the two views on the encoding side.
//!
//! [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0

use ironrdp_pdu::geometry::ExclusiveRectangle;

use crate::decode::{DecoderResult, Yuv420Frame};

/// Pixel alignment of H.264 macroblocks, to which views are padded.
const MACROBLOCK: usize = 16;

/// Largest change the reverse chroma filter may make before it is applied ([MS-RDPEGFX] 3.3.8.3.2).
///
/// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
const REVERSE_FILTER_THRESHOLD: u8 = 30;

/// How the auxiliary view packs the chroma samples the main view leaves out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Avc444Layout {
    /// `RFX_AVC444_BITMAP_STREAM`: odd chroma rows interleaved per macroblock ([MS-RDPEGFX] 3.3.8.3.2).
    ///
    /// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
    V1,
    /// `RFX_AVC444V2_BITMAP_STREAM`: odd chroma columns side by side ([MS-RDPEGFX] 3.3.8.3.3).
    ///
    /// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
    V2,
}

/// A YUV 4:4:4 picture in the MS-RDPEGFX color space
///
/// The planes are padded to whole macroblocks.
/// Between the reverse chroma filter's inputs, the even-row, even-column chroma sample holds
/// the main view's subsampled value, and the filter runs when the picture is converted to RGBA.
#[derive(Clone)]
pub struct Yuv444Frame {
    width: u16,
    height: u16,
    /// Bytes per plane row, `width` rounded up to a whole macroblock.
    stride: usize,
    /// INVARIANT: `y`, `u` and `v` are `stride * rows` bytes, and `rows` is `height` rounded up to a whole macroblock.
    rows: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl Yuv444Frame {
    /// Creates a black picture.
    pub fn new(width: u16, height: u16) -> Self {
        let stride = usize::from(width).next_multiple_of(MACROBLOCK);
        let rows = usize::from(height).next_multiple_of(MACROBLOCK);
        let size = stride * rows;
        Self {
            width,
            height,
            stride,
            rows,
            y: vec![0; size],
            u: vec![128; size],
            v: vec![128; size],
        }
    }

    /// Converts tightly packed RGBA pixels with the forward transform of [MS-RDPEGFX] 3.3.8.3.1
    /// ("Color Conversion").
    ///
    /// Like a decoded picture, the even-row, even-column chroma samples hold their 2x2 block's
    /// average, which [`Self::to_rgba`] reverses.
    ///
    /// Returns `None` when `rgba` is shorter than `width * height * 4` bytes.
    ///
    /// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
    pub fn from_rgba(rgba: &[u8], width: u16, height: u16) -> Option<Self> {
        let row_bytes = usize::from(width) * 4;
        if rgba.len() < row_bytes * usize::from(height) {
            return None;
        }
        let mut frame = Self::new(width, height);
        if row_bytes == 0 {
            return Some(frame);
        }
        for (row, pixels) in rgba.chunks_exact(row_bytes).take(usize::from(height)).enumerate() {
            let line = row * frame.stride;
            for (col, pixel) in pixels.chunks_exact(4).enumerate() {
                let (y, u, v) = rgb_to_yuv(pixel[0], pixel[1], pixel[2]);
                frame.y[line + col] = y;
                frame.u[line + col] = u;
                frame.v[line + col] = v;
            }
        }
        for row in (0..frame.rows).step_by(2) {
            for col in (0..frame.stride).step_by(2) {
                let at = row * frame.stride + col;
                let below = at + frame.stride;
                frame.u[at] = average(frame.u[at], frame.u[at + 1], frame.u[below], frame.u[below + 1]);
                frame.v[at] = average(frame.v[at], frame.v[at + 1], frame.v[below], frame.v[below + 1]);
            }
        }
        Some(frame)
    }

    /// Bytes [`Self::new`] allocates for a `width` by `height` picture.
    pub fn allocation_size(width: u16, height: u16) -> usize {
        let plane = usize::from(width).next_multiple_of(MACROBLOCK) * usize::from(height).next_multiple_of(MACROBLOCK);
        plane // Y
            + plane // U
            + plane // V
    }

    /// Bytes this picture holds.
    pub fn allocated_bytes(&self) -> usize {
        self.y.len() + self.u.len() + self.v.len()
    }

    /// Picture width in pixels.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Picture height in pixels.
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Takes luma and subsampled chroma from a main view, within `rects`.
    ///
    /// Inside the rectangles the picture becomes the 4:2:0 picture of the view until an
    /// auxiliary view covers them, as for macroblocks in a received luma subframe. Luma is
    /// taken within the rectangles only; chroma for every 2x2 block they touch, since a
    /// block shares its subsampled value.
    pub fn apply_main_view(&mut self, view: &Yuv420Frame, rects: &[ExclusiveRectangle]) {
        for rect in rects {
            let Some(area) = self.chroma_block_area(rect) else {
                continue;
            };
            let luma_rows = usize::from(rect.top)..usize::from(rect.bottom.min(self.height));
            let luma_left = usize::from(rect.left);
            for row in area.top..area.bottom {
                let (Some(luma), Some(u), Some(v)) = (view.luma_row(row), view.u_row(row / 2), view.v_row(row / 2))
                else {
                    break;
                };
                let line = row * self.stride;
                let right = area.right.min(luma.len());
                if right <= area.left {
                    break;
                }
                let luma_right = right.min(usize::from(rect.right));
                if luma_rows.contains(&row) && luma_left < luma_right {
                    self.y[line + luma_left..line + luma_right].copy_from_slice(&luma[luma_left..luma_right]);
                }
                for col in area.left..right {
                    self.u[line + col] = u[col / 2];
                    self.v[line + col] = v[col / 2];
                }
            }
        }
    }

    /// Takes the chroma samples the main view leaves out from an auxiliary view, within `rects`.
    ///
    /// The luma and subsampled chroma are the ones the main views left there.
    pub fn apply_auxiliary_view(&mut self, layout: Avc444Layout, view: &Yuv420Frame, rects: &[ExclusiveRectangle]) {
        for rect in rects {
            let Some(area) = self.chroma_block_area(rect) else {
                continue;
            };
            match layout {
                Avc444Layout::V1 => self.apply_auxiliary_v1(view, &area),
                Avc444Layout::V2 => self.apply_auxiliary_v2(view, &area),
            }
        }
    }

    /// Converts `rect` to tightly packed RGBA with the reverse transform of [MS-RDPEGFX]
    /// 3.3.8.3.1 ("Color Conversion"), after the reverse chroma filter of 3.3.8.3.2.
    ///
    /// `rect` is clipped to the picture.
    ///
    /// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
    pub fn to_rgba(&self, rect: &ExclusiveRectangle) -> Vec<u8> {
        let right = usize::from(rect.right.min(self.width));
        let bottom = usize::from(rect.bottom.min(self.height));
        let (left, top) = (usize::from(rect.left), usize::from(rect.top));
        if !(left < right && top < bottom) {
            return Vec::new();
        }
        let mut rgba = Vec::with_capacity((right - left) * (bottom - top) * 4);
        for row in top..bottom {
            let line = row * self.stride;
            for col in left..right {
                let at = line + col;
                let (mut u, mut v) = (self.u[at], self.v[at]);
                if row % 2 == 0 && col % 2 == 0 {
                    // The block's other three samples are inside the padded planes: `stride` and
                    // `rows` are even, and so are `row` and `col`.
                    let below = at + self.stride;
                    u = reverse_filter(u, self.u[at + 1], self.u[below], self.u[below + 1]);
                    v = reverse_filter(v, self.v[at + 1], self.v[below], self.v[below + 1]);
                }
                rgba.extend_from_slice(&yuv_to_rgba(self.y[at], u, v));
            }
        }
        rgba
    }

    /// Splits the picture into its main and auxiliary views.
    ///
    /// Both views cover the padded picture. The main view carries the average of each 2x2
    /// chroma block, held in its even-row, even-column sample, which the reverse chroma filter
    /// undoes.
    pub fn split(&self, layout: Avc444Layout) -> DecoderResult<(Yuv420Frame, Yuv420Frame)> {
        let (half_stride, half_rows) = (self.stride / 2, self.rows / 2);
        let mut main_u = vec![0; half_stride * half_rows];
        let mut main_v = vec![0; half_stride * half_rows];
        for row in 0..half_rows {
            for col in 0..half_stride {
                let at = 2 * row * self.stride + 2 * col;
                main_u[row * half_stride + col] = self.u[at];
                main_v[row * half_stride + col] = self.v[at];
            }
        }

        let mut aux_y = vec![0; self.stride * self.rows];
        let mut aux_u = vec![0; half_stride * half_rows];
        let mut aux_v = vec![0; half_stride * half_rows];
        match layout {
            Avc444Layout::V1 => {
                // B4 and B5: in each macroblock row, auxiliary luma rows 0 to 7 carry the odd U
                // rows and rows 8 to 15 the odd V rows.
                for aux_row in 0..self.rows {
                    let macroblock = aux_row - aux_row % MACROBLOCK;
                    let within = aux_row % MACROBLOCK;
                    let (plane, row) = if within < MACROBLOCK / 2 {
                        (&self.u, macroblock + 2 * within + 1)
                    } else {
                        (&self.v, macroblock + 2 * (within - MACROBLOCK / 2) + 1)
                    };
                    aux_y[aux_row * self.stride..][..self.stride]
                        .copy_from_slice(&plane[row * self.stride..][..self.stride]);
                }
                // B6 and B7: odd columns of even rows.
                for row in 0..half_rows {
                    for col in 0..half_stride {
                        let at = 2 * row * self.stride + 2 * col + 1;
                        aux_u[row * half_stride + col] = self.u[at];
                        aux_v[row * half_stride + col] = self.v[at];
                    }
                }
            }
            Avc444Layout::V2 => {
                // B4 and B5: odd U columns, then odd V columns.
                for row in 0..self.rows {
                    for col in 0..half_stride {
                        let at = row * self.stride + 2 * col + 1;
                        aux_y[row * self.stride + col] = self.u[at];
                        aux_y[row * self.stride + half_stride + col] = self.v[at];
                    }
                }
                // B6 to B9: every other even column of odd rows.
                let quarter_stride = self.stride / 4;
                for row in 0..half_rows {
                    for col in 0..quarter_stride {
                        let at = (2 * row + 1) * self.stride + 4 * col;
                        let out = row * half_stride + col;
                        aux_u[out] = self.u[at];
                        aux_u[out + quarter_stride] = self.v[at];
                        aux_v[out] = self.u[at + 2];
                        aux_v[out + quarter_stride] = self.v[at + 2];
                    }
                }
            }
        }

        let (width, height) = self.padded_size()?;
        let main = Yuv420Frame::new(width, height, self.y.clone(), self.stride, main_u, main_v, half_stride)?;
        let aux = Yuv420Frame::new(width, height, aux_y, self.stride, aux_u, aux_v, half_stride)?;
        Ok((main, aux))
    }

    fn apply_auxiliary_v1(&mut self, view: &Yuv420Frame, area: &Area) {
        for row in area.top..area.bottom {
            let line = row * self.stride;
            if row % 2 == 1 {
                // B4 and B5: the whole odd row comes from one auxiliary luma row, U from the first
                // half of its macroblock row and V from the second.
                let u_row = row - row % MACROBLOCK + (row % MACROBLOCK) / 2;
                let (Some(u), Some(v)) = (view.luma_row(u_row), view.luma_row(u_row + MACROBLOCK / 2)) else {
                    continue;
                };
                let right = area.right.min(u.len());
                if right <= area.left {
                    continue;
                }
                self.u[line + area.left..line + right].copy_from_slice(&u[area.left..right]);
                self.v[line + area.left..line + right].copy_from_slice(&v[area.left..right]);
            } else {
                // B6 and B7: odd columns of even rows.
                let (Some(u), Some(v)) = (view.u_row(row / 2), view.v_row(row / 2)) else {
                    continue;
                };
                for col in (area.left + 1..area.right).step_by(2) {
                    let (Some(&u), Some(&v)) = (u.get(col / 2), v.get(col / 2)) else {
                        break;
                    };
                    self.u[line + col] = u;
                    self.v[line + col] = v;
                }
            }
        }
    }

    fn apply_auxiliary_v2(&mut self, view: &Yuv420Frame, area: &Area) {
        let half_width = v2_view_width(view) / 2;
        let quarter_width = half_width / 2;
        for row in area.top..area.bottom {
            let line = row * self.stride;
            // B4 and B5: odd columns, U on the left half of the auxiliary luma row and V on the right.
            if let Some(luma) = view.luma_row(row) {
                for col in (area.left + 1..area.right).step_by(2) {
                    let (Some(&u), Some(&v)) = (luma.get(col / 2), luma.get(half_width + col / 2)) else {
                        break;
                    };
                    self.u[line + col] = u;
                    self.v[line + col] = v;
                }
            }
            if row % 2 == 0 {
                continue;
            }
            // B6 to B9: even columns of odd rows, multiples of four from the auxiliary U plane and
            // the others from the auxiliary V plane, each with U on the left and V on the right.
            let (Some(first), Some(second)) = (view.u_row(row / 2), view.v_row(row / 2)) else {
                continue;
            };
            for col in (area.left..area.right).step_by(2) {
                let source = if col % 4 == 0 { first } else { second };
                let (Some(&u), Some(&v)) = (source.get(col / 4), source.get(quarter_width + col / 4)) else {
                    break;
                };
                self.u[line + col] = u;
                self.v[line + col] = v;
            }
        }
    }

    /// The part of `rect` inside the picture, widened to whole 2x2 chroma blocks.
    ///
    /// INVARIANT: the returned area is within `stride` columns and `rows` rows.
    fn chroma_block_area(&self, rect: &ExclusiveRectangle) -> Option<Area> {
        let left = usize::from(rect.left) & !1;
        let top = usize::from(rect.top) & !1;
        let right = usize::from(rect.right.min(self.width)).next_multiple_of(2);
        let bottom = usize::from(rect.bottom.min(self.height)).next_multiple_of(2);
        (left < right && top < bottom).then_some(Area {
            left,
            top,
            right,
            bottom,
        })
    }

    fn padded_size(&self) -> DecoderResult<(u16, u16)> {
        let (Ok(width), Ok(height)) = (u16::try_from(self.stride), u16::try_from(self.rows)) else {
            return Err(crate::decode::DecoderError::msg(
                "padded AVC444 picture exceeds 65535 pixels",
            ));
        };
        Ok((width, height))
    }
}

impl core::fmt::Debug for Yuv444Frame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Yuv444Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// A pixel rectangle with exclusive `right` and `bottom`.
struct Area {
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
}

/// The view width W of [MS-RDPEGFX] 3.3.8.3.3, which places V after U in AVC444v2 auxiliary rows.
///
/// Coded H.264 pictures are whole macroblocks wide, and some decoders report the width after
/// cropping, so the decoded width is rounded up to a whole macroblock.
///
/// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
// UNTESTED: that Windows servers place V at the coded picture width over two. The tests split
// views with this same width, so they cannot catch a server using another one (FreeRDP rounds the
// surface width to 32 instead). A wrong width shows as wrong colors on surfaces whose width is an
// odd number of macroblocks; check such a surface against a Windows server when changing this.
fn v2_view_width(view: &Yuv420Frame) -> usize {
    usize::from(view.width()).next_multiple_of(MACROBLOCK)
}

/// Recovers an even-row, even-column chroma sample from the main view's 2x2 average and the
/// block's other three samples ([MS-RDPEGFX] 3.3.8.3.2).
///
/// A change below the threshold is quantization noise, so the average is kept.
///
/// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
fn reverse_filter(average: u8, right: u8, below: u8, diagonal: u8) -> u8 {
    let reversed = clamp_channel(4 * i32::from(average) - i32::from(right) - i32::from(below) - i32::from(diagonal));
    if reversed.abs_diff(average) < REVERSE_FILTER_THRESHOLD {
        average
    } else {
        reversed
    }
}

fn average(a: u8, b: u8, c: u8, d: u8) -> u8 {
    clamp_channel((i32::from(a) + i32::from(b) + i32::from(c) + i32::from(d) + 2) >> 2)
}

/// Converts a pixel with the reverse transform of [MS-RDPEGFX] 3.3.8.3.1 ("Color Conversion").
///
/// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
pub(crate) fn yuv_to_rgba(y: u8, u: u8, v: u8) -> [u8; 4] {
    let luma = 256 * i32::from(y);
    let u = i32::from(u) - 128;
    let v = i32::from(v) - 128;
    [
        clamp_channel((luma + 403 * v) >> 8),
        clamp_channel((luma - 48 * u - 120 * v) >> 8),
        clamp_channel((luma + 475 * u) >> 8),
        0xFF,
    ]
}

/// Converts a pixel with the forward transform of [MS-RDPEGFX] 3.3.8.3.1 ("Color Conversion").
///
/// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
fn rgb_to_yuv(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    (
        clamp_channel((54 * r + 183 * g + 18 * b) >> 8),
        clamp_channel(((-29 * r - 99 * g + 128 * b) >> 8) + 128),
        clamp_channel(((128 * r - 116 * g - 12 * b) >> 8) + 128),
    )
}

fn clamp_channel(value: i32) -> u8 {
    // The clamped value is its own low byte.
    let [low, ..] = value.clamp(0, 255).to_le_bytes();
    low
}
