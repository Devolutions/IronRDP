//! AVC444 frame reconstruction
//!
//! In AVC444 mode the server splits each YUV444 frame into two YUV420p
//! frames, sent as two H.264 streams: the main (luma) view and the auxiliary
//! (chroma) view. [`Yuv444Frame`] holds the reconstructed YUV444 frame and
//! applies each decoded view to it, following [MS-RDPEGFX] 3.3.8.3.2 (YUV444
//! mode) and 3.3.8.3.3 ([YUV444v2 mode][1]).
//!
//! The frame persists between updates because a PDU may carry only one of the
//! views: a luma-only update is shown at YUV420 quality until the matching
//! chroma arrives, and a chroma-only update refines the last luma.
//!
//! Chroma at even columns of even rows exists only in the main view, where the
//! encoder stored a 2x2 average. It is used as-is; the specification's reverse
//! filter is optional and not applied.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/781406c3-5e24-4f2b-b6ff-42b76bf64f6d

use ironrdp_pdu::geometry::ExclusiveRectangle;

use crate::decode::{DecoderError, DecoderResult};

/// Borrowed planes of a decoded YUV420p frame (8-bit, 4:2:0).
#[derive(Debug, Clone, Copy)]
pub struct Yuv420Planes<'a> {
    pub y: &'a [u8],
    pub y_stride: usize,
    pub u: &'a [u8],
    pub u_stride: usize,
    pub v: &'a [u8],
    pub v_stride: usize,
}

/// A YUV444 frame with tightly packed planes (stride equals width).
#[derive(Clone)]
pub struct Yuv444Frame {
    width: usize,
    height: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl core::fmt::Debug for Yuv444Frame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Yuv444Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl Yuv444Frame {
    /// Create a frame the size of the decoded views, after any cropping.
    pub fn new(width: usize, height: usize) -> DecoderResult<Self> {
        if width == 0 || height == 0 {
            return Err(DecoderError::msg("AVC444 frame is empty"));
        }
        let len = width
            .checked_mul(height)
            .ok_or_else(|| DecoderError::msg("AVC444 frame size overflows"))?;
        Ok(Self {
            width,
            height,
            y: vec![0; len],
            u: vec![128; len],
            v: vec![128; len],
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// The Y plane, `width * height` bytes.
    pub fn y(&self) -> &[u8] {
        &self.y
    }

    /// The U plane, `width * height` bytes.
    pub fn u(&self) -> &[u8] {
        &self.u
    }

    /// The V plane, `width * height` bytes.
    pub fn v(&self) -> &[u8] {
        &self.v
    }

    /// Apply a decoded main (luma) view inside `regions` (areas B1-B3).
    ///
    /// Y is copied. Each main-view U/V sample fills its whole 2x2 block, so
    /// the region shows at YUV420 quality until an auxiliary view refines its
    /// chroma.
    pub fn apply_main_view(&mut self, main: &Yuv420Planes<'_>, regions: &[ExclusiveRectangle]) -> DecoderResult<()> {
        let (chroma_width, chroma_height) = (self.width.div_ceil(2), self.height.div_ceil(2));
        check_planes(main, (self.width, self.height), (chroma_width, chroma_height))?;
        for region in regions {
            let (left, top, right, bottom) = self.macroblock_bounds(region);
            for y in top..bottom {
                let src = y * main.y_stride;
                let dst = y * self.width;
                self.y[dst + left..dst + right].copy_from_slice(&main.y[src + left..src + right]);
            }
            // Row slices rather than per-sample indexing: bounds are checked
            // once per row, with no division per sample. `left` is a
            // macroblock edge unless the region is empty; when the frame width
            // is odd, the last column is a block of its own.
            let (from, to) = (left / 2, right / 2);
            for y in top..bottom {
                let (src_u, src_v) = ((y / 2) * main.u_stride, (y / 2) * main.v_stride);
                let dst = y * self.width;
                for (pair, &u) in self.u[dst + 2 * from..dst + 2 * to]
                    .chunks_exact_mut(2)
                    .zip(&main.u[src_u + from..src_u + to])
                {
                    pair.fill(u);
                }
                for (pair, &v) in self.v[dst + 2 * from..dst + 2 * to]
                    .chunks_exact_mut(2)
                    .zip(&main.v[src_v + from..src_v + to])
                {
                    pair.fill(v);
                }
                if right % 2 == 1 && left < right {
                    self.u[dst + right - 1] = main.u[src_u + to];
                    self.v[dst + right - 1] = main.v[src_v + to];
                }
            }
        }
        Ok(())
    }

    /// Apply a decoded auxiliary (chroma) view in YUV444v2 layout inside
    /// `regions` (areas B4-B9).
    ///
    /// - Auxiliary Y, left half: U at odd columns, every row. Right half: V.
    /// - Auxiliary U, left quarter: U at columns 4x of odd rows. Right
    ///   quarter: V.
    /// - Auxiliary V, left quarter: U at columns 4x+2 of odd rows. Right
    ///   quarter: V.
    ///
    /// The halves and quarters are of the coded width, before cropping. It
    /// is taken as the frame width rounded up to a whole macroblock, as an
    /// encoder pads a frame for H.264 and crops the padding off.
    pub fn apply_auxiliary_view_v2(
        &mut self,
        aux: &Yuv420Planes<'_>,
        regions: &[ExclusiveRectangle],
    ) -> DecoderResult<()> {
        let coded_width = self.width.next_multiple_of(16);
        let (half, quarter) = (coded_width / 2, coded_width / 4);
        check_planes(
            aux,
            (half + self.width / 2, self.height),
            (quarter + self.width.div_ceil(4), self.height / 2),
        )?;
        for region in regions {
            let (left, top, right, bottom) = self.macroblock_bounds(region);

            // B4, B5: odd columns of every row.
            let (from, to) = (left / 2, right / 2);
            for y in top..bottom {
                let src = y * aux.y_stride;
                let dst = y * self.width;
                for (pair, &u) in self.u[dst + 2 * from..dst + 2 * to]
                    .chunks_exact_mut(2)
                    .zip(&aux.y[src + from..src + to])
                {
                    pair[1] = u;
                }
                for (pair, &v) in self.v[dst + 2 * from..dst + 2 * to]
                    .chunks_exact_mut(2)
                    .zip(&aux.y[src + half + from..src + half + to])
                {
                    pair[1] = v;
                }
            }

            // B6-B9: even columns of odd rows.
            let (from, to) = (left / 4, right / 4);
            for y in (top / 2)..(bottom / 2) {
                let (src_u, src_v) = (y * aux.u_stride, y * aux.v_stride);
                let dst = (2 * y + 1) * self.width;
                let u_samples = aux.u[src_u + from..src_u + to]
                    .iter()
                    .zip(&aux.v[src_v + from..src_v + to]);
                for (quad, (&at_0, &at_2)) in self.u[dst + 4 * from..dst + 4 * to].chunks_exact_mut(4).zip(u_samples) {
                    quad[0] = at_0;
                    quad[2] = at_2;
                }
                let v_samples = aux.u[src_u + quarter + from..src_u + quarter + to]
                    .iter()
                    .zip(&aux.v[src_v + quarter + from..src_v + quarter + to]);
                for (quad, (&at_0, &at_2)) in self.v[dst + 4 * from..dst + 4 * to].chunks_exact_mut(4).zip(v_samples) {
                    quad[0] = at_0;
                    quad[2] = at_2;
                }
                // The last group when the width isn't a multiple of 4, which
                // may have no column 2. A region clamped to an empty one at
                // that edge covers no group.
                if right % 4 != 0 && left < right {
                    let x = to;
                    self.u[dst + 4 * x] = aux.u[src_u + x];
                    self.v[dst + 4 * x] = aux.u[src_u + quarter + x];
                    if 4 * x + 2 < right {
                        self.u[dst + 4 * x + 2] = aux.v[src_v + x];
                        self.v[dst + 4 * x + 2] = aux.v[src_v + quarter + x];
                    }
                }
            }
        }
        Ok(())
    }

    /// Convert the area `rect` to RGBA (4 bytes per pixel, rows packed),
    /// reading the samples as full-range BT.709, the format MS-RDPEGFX
    /// specifies for AVC.
    pub fn to_rgba(&self, rect: &ExclusiveRectangle) -> DecoderResult<Vec<u8>> {
        let (left, top) = (usize::from(rect.left), usize::from(rect.top));
        let (right, bottom) = (usize::from(rect.right), usize::from(rect.bottom));
        if left >= right || top >= bottom || right > self.width || bottom > self.height {
            return Err(DecoderError::msg("RGBA area is empty or exceeds the AVC444 frame"));
        }
        let (width, height) = (right - left, bottom - top);
        let to_u32 = |n: usize| u32::try_from(n).map_err(|_| DecoderError::msg("AVC444 frame too large"));
        let stride = to_u32(self.width)?;
        let start = top * self.width + left;
        let mut rgba = vec![0u8; width * height * 4];
        yuv::yuv444_to_rgba(
            &yuv::YuvPlanarImage {
                y_plane: &self.y[start..],
                y_stride: stride,
                u_plane: &self.u[start..],
                u_stride: stride,
                v_plane: &self.v[start..],
                v_stride: stride,
                width: to_u32(width)?,
                height: to_u32(height)?,
            },
            &mut rgba,
            to_u32(width * 4)?,
            yuv::YuvRange::Full,
            yuv::YuvStandardMatrix::Bt709,
        )
        .map_err(|e| DecoderError::new("failed to convert YUV444 to RGBA", e))?;
        Ok(rgba)
    }

    /// The region expanded to whole 16x16 macroblocks and clipped to the
    /// frame. The specification converts whole macroblocks; the caller
    /// crops to the region afterwards.
    fn macroblock_bounds(&self, region: &ExclusiveRectangle) -> (usize, usize, usize, usize) {
        let left = usize::from(region.left) / 16 * 16;
        let top = usize::from(region.top) / 16 * 16;
        let right = usize::from(region.right)
            .div_ceil(16)
            .saturating_mul(16)
            .min(self.width);
        let bottom = usize::from(region.bottom)
            .div_ceil(16)
            .saturating_mul(16)
            .min(self.height);
        (left.min(right), top.min(bottom), right, bottom)
    }
}

/// Check that each plane holds the `(row length, rows)` read from it at its
/// stride: `luma` from Y, `chroma` from U and V.
fn check_planes(planes: &Yuv420Planes<'_>, luma: (usize, usize), chroma: (usize, usize)) -> DecoderResult<()> {
    if plane_fits(planes.y, planes.y_stride, luma)
        && plane_fits(planes.u, planes.u_stride, chroma)
        && plane_fits(planes.v, planes.v_stride, chroma)
    {
        Ok(())
    } else {
        Err(DecoderError::msg("decoded AVC444 view is smaller than the frame"))
    }
}

/// Whether `plane` holds `rows` rows of `row_len` samples at `stride`.
pub(crate) fn plane_fits(plane: &[u8], stride: usize, (row_len, rows): (usize, usize)) -> bool {
    rows == 0
        || stride >= row_len
            && stride
                .checked_mul(rows - 1)
                .and_then(|n| n.checked_add(row_len))
                .is_some_and(|needed| plane.len() >= needed)
}
