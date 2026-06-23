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

/// Web render surface. Owns the canvas's 2D context and a reusable RGBA scratch buffer: each dirty
/// region's pixels are copied once into the scratch (alpha forced opaque), then blitted with
/// `put_image_data` at the region's origin.
///
/// This replaced a softbuffer-backed path that converted RGBA -> u32 `0RGB` (our pass) and then let
/// softbuffer repack u32 -> RGBA per frame into a freshly allocated buffer — two pixel passes over
/// the whole surface plus a per-frame allocation. The replay benchmark (`src/bench.rs`, feature
/// `bench`) measures the direct path's present an order of magnitude faster at 4K with byte-identical
/// canvas output (FNV-1a over the rendered canvas pixels). Mirrors the same fix in IronVNC.
pub(crate) struct Canvas {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    rgba: Vec<u8>,
}

impl Canvas {
    pub(crate) fn new(render_canvas: HtmlCanvasElement, width: NonZeroU32, height: NonZeroU32) -> anyhow::Result<Self> {
        render_canvas.set_width(width.get());
        render_canvas.set_height(height.get());
        let ctx = context_2d(&render_canvas)?;

        Ok(Self {
            canvas: render_canvas,
            ctx,
            rgba: Vec::new(),
        })
    }

    /// Setting width/height resets the canvas backing store and 2D context state.
    pub(crate) fn resize(&mut self, width: NonZeroU32, height: NonZeroU32) {
        self.canvas.set_width(width.get());
        self.canvas.set_height(height.get());
    }

    /// `buffer` is the region's RGBA sub-image (as produced by `extract_partial_image`).
    pub(crate) fn draw(&mut self, buffer: &[u8], region: InclusiveRectangle) -> anyhow::Result<()> {
        let len = buffer.len();
        if self.rgba.len() < len {
            self.rgba.resize(len, 0);
        }
        let dst = &mut self.rgba[..len];
        dst.copy_from_slice(buffer);

        // Force opaque alpha: most decode paths already write 0xFF, but the QOI path copies source
        // alpha, and `put_image_data` stores alpha verbatim into the canvas.
        for pixel in dst.chunks_exact_mut(4) {
            pixel[3] = 0xFF;
        }

        blit(&self.ctx, dst, &region)
    }
}

/// Acquires the canvas 2D context. Only meaningful on wasm; stubbed elsewhere so the crate still
/// type-checks for host tooling.
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
