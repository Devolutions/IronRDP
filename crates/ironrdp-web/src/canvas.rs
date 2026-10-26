use core::num::NonZeroU32;

#[cfg(target_arch = "wasm32")]
use anyhow::anyhow;
use ironrdp::pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp::session::image::DecodedImage;
use ironrdp_core::WriteBuf;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::{Clamped, JsCast as _};
#[cfg(target_arch = "wasm32")]
use web_sys::ImageData;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use crate::image::extract_partial_image;

/// Web render surface: blits each dirty region to the canvas with `put_image_data`.
pub(crate) struct Canvas {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
}

impl Canvas {
    pub(crate) fn new(render_canvas: HtmlCanvasElement, width: NonZeroU32, height: NonZeroU32) -> anyhow::Result<Self> {
        render_canvas.set_width(width.get());
        render_canvas.set_height(height.get());
        let ctx = context_2d(&render_canvas)?;

        Ok(Self {
            canvas: render_canvas,
            ctx,
        })
    }

    /// Resizes the backing store. Note: this also clears the canvas and resets 2D context state;
    /// the cached `ctx` stays valid.
    pub(crate) fn resize(&mut self, width: NonZeroU32, height: NonZeroU32) {
        self.canvas.set_width(width.get());
        self.canvas.set_height(height.get());
    }

    /// Blits a dirty region with `put_image_data`. Forces alpha opaque first: the framebuffer isn't
    /// guaranteed opaque (zero-init columns, QOI-RGBA) and `put_image_data` stores alpha verbatim.
    pub(crate) fn draw(&self, buffer: &mut [u8], region: InclusiveRectangle) -> anyhow::Result<()> {
        make_opaque(buffer);

        self.blit(buffer, region)
    }

    /// Prepares and uploads a framebuffer update without modifying the source pixels.
    /// Clips the update to the framebuffer; regions with no overlap are ignored.
    /// The caller must clear `buffer` before each update.
    pub(crate) fn draw_image(
        &self,
        image: &DecodedImage,
        region: InclusiveRectangle,
        buffer: &mut WriteBuf,
    ) -> anyhow::Result<()> {
        let Some(region) = clip_region(image, &region) else {
            return Ok(());
        };

        if prefer_fused_copy(image, &region) {
            copy_opaque_rows(image, &region, buffer);
            self.blit(buffer.filled(), region)
        } else {
            let region = extract_partial_image(image, region, buffer);
            self.draw(buffer.filled_mut(), region)
        }
    }

    /// Uploads pixels whose alpha has already been made opaque.
    fn blit(&self, buffer: &[u8], region: InclusiveRectangle) -> anyhow::Result<()> {
        #[cfg(target_arch = "wasm32")]
        {
            let image = ImageData::new_with_u8_clamped_array_and_sh(
                Clamped(buffer),
                u32::from(region.width()),
                u32::from(region.height()),
            )
            .map_err(|err| anyhow!("ImageData::new failed: {err:?}"))?;
            self.ctx
                .put_image_data(&image, f64::from(region.left), f64::from(region.top))
                .map_err(|err| anyhow!("put_image_data failed: {err:?}"))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (&self.ctx, buffer, region);
            unimplemented!("web canvas is only available on wasm32")
        }
    }
}

/// Bounds a decoded update before its dimensions or pixel offsets are calculated.
/// RemoteFX regions can extend past the framebuffer even after decoding succeeds.
pub(crate) fn clip_region(image: &DecodedImage, region: &InclusiveRectangle) -> Option<InclusiveRectangle> {
    let bounds = InclusiveRectangle {
        left: 0,
        top: 0,
        right: image.width().checked_sub(1)?,
        bottom: image.height().checked_sub(1)?,
    };
    region.intersect(&bounds)
}

/// Appends opaque full-width rows without changing existing buffer contents or the framebuffer.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "the source image bounds the row offsets and length"
)]
pub(crate) fn copy_opaque_rows(image: &DecodedImage, region: &InclusiveRectangle, buffer: &mut WriteBuf) {
    debug_assert_eq!(region.left, 0);
    debug_assert_eq!(region.width(), image.width());
    let stride = usize::from(image.width()) * 4;
    let len = stride * usize::from(region.height());
    let begin = usize::from(region.top) * stride;
    copy_opaque_rgba(&image.data()[begin..begin + len], buffer.unfilled_to(len));
    buffer.advance(len);
}

#[expect(clippy::arithmetic_side_effects, reason = "two u16 dimensions fit in a wasm32 usize")]
fn prefer_fused_copy(image: &DecodedImage, region: &InclusiveRectangle) -> bool {
    // Avoid the per-row SIMD overhead observed for narrow updates. Small full-width updates
    // also retain the existing path; this threshold is checked by the browser benchmark.
    const MIN_FUSED_PIXELS: usize = 256 * 1024 / 4;
    region.left == 0
        && region.width() == image.width()
        && MIN_FUSED_PIXELS <= usize::from(image.width()) * usize::from(region.height())
}

/// Copies complete RGBA pixels with opaque alpha, preserving incomplete trailing pixels.
pub(crate) fn copy_opaque_rgba(src: &[u8], dst: &mut [u8]) {
    assert_eq!(src.len(), dst.len());

    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    let (src, dst) = {
        use core::arch::wasm32::{u32x4_splat, v128_load, v128_or, v128_store};

        let alpha = u32x4_splat(0xFF00_0000);
        let mut sources = src.chunks_exact(16);
        let mut targets = dst.chunks_exact_mut(16);
        for (source, target) in sources.by_ref().zip(targets.by_ref()) {
            // SAFETY: Each source owns 16 initialized bytes; SIMD loads accept unaligned pointers.
            let pixels = unsafe { v128_load(source.as_ptr().cast()) };
            // SAFETY: Each target uniquely owns 16 bytes; SIMD stores accept unaligned pointers.
            unsafe { v128_store(target.as_mut_ptr().cast(), v128_or(pixels, alpha)) };
        }
        (sources.remainder(), targets.into_remainder())
    };

    dst.copy_from_slice(src);
    for pixel in dst.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
}

/// Forces complete RGBA pixels opaque, preserving RGB and any incomplete trailing pixel.
pub(crate) fn make_opaque(buffer: &mut [u8]) {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    let buffer = {
        use core::arch::wasm32::{u32x4_splat, v128_load, v128_or, v128_store};

        let alpha = u32x4_splat(0xFF00_0000);
        let mut chunks = buffer.chunks_exact_mut(16);
        for chunk in &mut chunks {
            let ptr = chunk.as_mut_ptr().cast();
            // SAFETY: The chunk contains 16 initialized bytes; v128_load accepts unaligned pointers.
            let pixels = unsafe { v128_load(ptr) };
            let opaque = v128_or(pixels, alpha);
            // SAFETY: The chunk uniquely owns these 16 bytes; v128_store accepts unaligned pointers.
            unsafe { v128_store(ptr, opaque) };
        }
        chunks.into_remainder()
    };

    for pixel in buffer.chunks_exact_mut(4) {
        pixel[3] = 0xFF;
    }
}

/// Acquires the canvas 2D context (wasm only; panics on other targets).
fn context_2d(canvas: &HtmlCanvasElement) -> anyhow::Result<CanvasRenderingContext2d> {
    #[cfg(target_arch = "wasm32")]
    {
        canvas
            .get_context("2d")
            .map_err(|err| anyhow!("get_context(\"2d\") failed: {err:?}"))?
            .ok_or_else(|| anyhow!("canvas has no 2d context"))?
            .dyn_into::<CanvasRenderingContext2d>()
            .map_err(|_| anyhow!("2d context is not a CanvasRenderingContext2d"))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = canvas;
        unimplemented!("web canvas is only available on wasm32")
    }
}
