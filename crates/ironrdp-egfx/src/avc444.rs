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
    /// Create a frame of the coded size of both views. Both dimensions must be
    /// multiples of 16, as MS-RDPEGFX requires for AVC bitstreams.
    pub fn new(width: usize, height: usize) -> DecoderResult<Self> {
        if width == 0 || height == 0 || !width.is_multiple_of(16) || !height.is_multiple_of(16) {
            return Err(DecoderError::msg("AVC444 frame size must be a nonzero multiple of 16"));
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
        check_planes(main, self.width, self.height)?;
        for region in regions {
            let (left, top, right, bottom) = self.macroblock_bounds(region);
            for y in top..bottom {
                let src = y * main.y_stride;
                let dst = y * self.width;
                self.y[dst + left..dst + right].copy_from_slice(&main.y[src + left..src + right]);
            }
            for y in top..bottom {
                let (src_u, src_v) = ((y / 2) * main.u_stride, (y / 2) * main.v_stride);
                let dst = y * self.width;
                for x in left..right {
                    self.u[dst + x] = main.u[src_u + x / 2];
                    self.v[dst + x] = main.v[src_v + x / 2];
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
    pub fn apply_auxiliary_view_v2(
        &mut self,
        aux: &Yuv420Planes<'_>,
        regions: &[ExclusiveRectangle],
    ) -> DecoderResult<()> {
        check_planes(aux, self.width, self.height)?;
        let (half, quarter) = (self.width / 2, self.width / 4);
        for region in regions {
            let (left, top, right, bottom) = self.macroblock_bounds(region);

            // B4, B5: odd columns of every row.
            for y in top..bottom {
                let src = y * aux.y_stride;
                let dst = y * self.width;
                for x in (left / 2)..(right / 2) {
                    self.u[dst + 2 * x + 1] = aux.y[src + x];
                    self.v[dst + 2 * x + 1] = aux.y[src + half + x];
                }
            }

            // B6-B9: even columns of odd rows.
            for y in (top / 2)..(bottom / 2) {
                let (src_u, src_v) = (y * aux.u_stride, y * aux.v_stride);
                let dst = (2 * y + 1) * self.width;
                for x in (left / 4)..(right / 4) {
                    self.u[dst + 4 * x] = aux.u[src_u + x];
                    self.v[dst + 4 * x] = aux.u[src_u + quarter + x];
                    self.u[dst + 4 * x + 2] = aux.v[src_v + x];
                    self.v[dst + 4 * x + 2] = aux.v[src_v + quarter + x];
                }
            }
        }
        Ok(())
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

/// Check that each plane is large enough for a `width` x `height` YUV420p
/// frame at its stride.
fn check_planes(planes: &Yuv420Planes<'_>, width: usize, height: usize) -> DecoderResult<()> {
    let (half_width, half_height) = (width / 2, height / 2);
    let fits = |plane: &[u8], stride: usize, row_len: usize, rows: usize| {
        stride >= row_len
            && stride
                .checked_mul(rows - 1)
                .and_then(|n| n.checked_add(row_len))
                .is_some_and(|needed| plane.len() >= needed)
    };
    if fits(planes.y, planes.y_stride, width, height)
        && fits(planes.u, planes.u_stride, half_width, half_height)
        && fits(planes.v, planes.v_stride, half_width, half_height)
    {
        Ok(())
    } else {
        Err(DecoderError::msg("decoded AVC444 view is smaller than the frame"))
    }
}
