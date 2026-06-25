use core::num::NonZeroU32;

#[cfg(target_arch = "wasm32")]
use anyhow::anyhow;
use ironrdp::pdu::geometry::InclusiveRectangle;
#[cfg(target_arch = "wasm32")]
use ironrdp::pdu::geometry::Rectangle as _;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::{Clamped, JsCast as _};
#[cfg(target_arch = "wasm32")]
use web_sys::ImageData;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

/// Web render surface. Owns the canvas's 2D context; each dirty region is blitted directly with
/// `put_image_data` at the region's origin, after forcing its alpha opaque in place. The region
/// buffer is the caller's throwaway copy, so no scratch buffer is kept here.
///
/// This replaced a softbuffer-backed path that converted RGBA -> u32 `0RGB` (our pass) and then let
/// softbuffer repack u32 -> RGBA per frame into a freshly allocated buffer — two pixel passes over
/// the whole surface plus a per-frame allocation. The direct path drops the u32 round-trip and the
/// per-frame allocation, measuring an order of magnitude faster present at 4K with byte-identical
/// canvas output. Mirrors the same fix in IronVNC.
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

    /// Resizes the canvas backing store to `width` x `height`. Setting width/height clears the
    /// canvas and resets 2D context state (transform, styles, ...); the cached `ctx` handle stays
    /// valid. Callers must not rely on prior canvas content or context configuration surviving.
    pub(crate) fn resize(&mut self, width: NonZeroU32, height: NonZeroU32) {
        self.canvas.set_width(width.get());
        self.canvas.set_height(height.get());
    }

    /// Blits one dirty region. `buffer` is the region's RGBA sub-image — a throwaway copy produced
    /// by `extract_partial_image` — mutated in place to force opaque alpha before upload.
    pub(crate) fn draw(&mut self, buffer: &mut [u8], region: InclusiveRectangle) -> anyhow::Result<()> {
        // Force opaque alpha in place. The decoded framebuffer leaves the alpha channel as
        // don't-care (it starts at 0 and the decode loop skips it), but `put_image_data` stores
        // alpha verbatim, so the region would otherwise render transparent on the canvas.
        for pixel in buffer.chunks_exact_mut(4) {
            pixel[3] = 0xFF;
        }

        blit(&self.ctx, buffer, &region)
    }
}

/// Acquires the canvas 2D context. Only meaningful on wasm; on other targets it exists solely so
/// host tooling type-checks, and panics if called.
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

/// Blits `rgba` (a `region`-sized RGBA buffer) onto the canvas at the region's origin.
fn blit(ctx: &CanvasRenderingContext2d, rgba: &[u8], region: &InclusiveRectangle) -> anyhow::Result<()> {
    #[cfg(target_arch = "wasm32")]
    {
        let image = ImageData::new_with_u8_clamped_array_and_sh(
            Clamped(rgba),
            u32::from(region.width()),
            u32::from(region.height()),
        )
        .map_err(|err| anyhow!("ImageData::new failed: {err:?}"))?;
        ctx.put_image_data(&image, f64::from(region.left), f64::from(region.top))
            .map_err(|err| anyhow!("put_image_data failed: {err:?}"))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (ctx, rgba, region);
        unimplemented!("web canvas is only available on wasm32")
    }
}
