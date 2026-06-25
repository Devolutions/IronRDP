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
        for pixel in buffer.chunks_exact_mut(4) {
            pixel[3] = 0xFF;
        }

        #[cfg(target_arch = "wasm32")]
        {
            let image = ImageData::new_with_u8_clamped_array_and_sh(
                Clamped(&*buffer),
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
