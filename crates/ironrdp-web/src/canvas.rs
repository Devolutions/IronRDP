use core::num::NonZeroU32;

#[cfg(target_arch = "wasm32")]
use anyhow::anyhow;
use ironrdp::pdu::geometry::InclusiveRectangle;
#[cfg(target_arch = "wasm32")]
use ironrdp::pdu::geometry::Rectangle as _;
use ironrdp::session::image::DecodedImage;
#[cfg(target_arch = "wasm32")]
use tracing::{debug, warn};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::{Clamped, JsCast as _};
#[cfg(target_arch = "wasm32")]
use web_sys::ImageData;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use crate::image::extract_partial_image;

/// Web render surface.
///
/// Primary path (wasm32): softblit (WebGPU). The session's `DecodedImage` bytes are uploaded per
/// dirty region straight out of the framebuffer (`write_texture` with the framebuffer's stride —
/// no extraction copy), and the X/garbage alpha channel is forced opaque on the GPU. One CPU copy
/// per dirty byte total.
///
/// Fallback path (WebGPU unavailable): the previous Canvas2D presenter — extract the dirty
/// region into a scratch buffer, force alpha opaque on the CPU, `put_image_data`.
pub(crate) enum Canvas {
    #[cfg(target_arch = "wasm32")]
    Gpu(Box<GpuCanvas>),
    Canvas2d(Canvas2d),
}

impl Canvas {
    pub(crate) async fn new(
        render_canvas: HtmlCanvasElement,
        width: NonZeroU32,
        height: NonZeroU32,
    ) -> anyhow::Result<Self> {
        render_canvas.set_width(width.get());
        render_canvas.set_height(height.get());

        #[cfg(target_arch = "wasm32")]
        {
            match softblit::Surface::new(
                softblit::SurfaceTarget::Canvas(render_canvas.clone()),
                softblit::SurfaceDescriptor {
                    source_size: (width.get(), height.get()),
                    // The session decodes into `PixelFormat::RgbA32` whose alpha is not reliable
                    // (e.g. the QOI path copies source alpha); Rgbx8 forces alpha to 1 in the blit.
                    format: softblit::PixelFormat::Rgbx8,
                    // Source and target are kept at the same size (the canvas element is scaled
                    // by CSS, as before); Stretch keeps full coverage if they ever diverge.
                    scaling: softblit::ScalingMode::Stretch,
                },
            )
            .await
            {
                Ok(surface) => {
                    debug!("softblit WebGPU presenter initialized");
                    return Ok(Self::Gpu(Box::new(GpuCanvas {
                        canvas: render_canvas,
                        surface,
                    })));
                }
                Err(softblit::Error::WebGpuUnavailable { reason }) => {
                    warn!(reason = %reason, "WebGPU unavailable; falling back to Canvas2D presenter");
                }
                Err(e) => return Err(anyhow::anyhow!("softblit surface creation failed: {e}")),
            }
        }

        Ok(Self::Canvas2d(Canvas2d::new(render_canvas)?))
    }

    /// Setting width/height resets the canvas backing store.
    pub(crate) fn resize(&mut self, width: NonZeroU32, height: NonZeroU32) {
        match self {
            #[cfg(target_arch = "wasm32")]
            Self::Gpu(gpu) => gpu.resize(width, height),
            Self::Canvas2d(c2d) => c2d.resize(width, height),
        }
    }

    /// Presents the damaged `regions` of `image` in one shot. The GPU path uploads each rect
    /// individually (softblit coalesces internally), so scattered updates upload far less than
    /// a single union bounding box would.
    pub(crate) fn draw(&mut self, image: &DecodedImage, regions: &[InclusiveRectangle]) -> anyhow::Result<()> {
        match self {
            #[cfg(target_arch = "wasm32")]
            Self::Gpu(gpu) => gpu.draw(image, regions),
            Self::Canvas2d(c2d) => {
                for region in regions {
                    let (region, buffer) = extract_partial_image(image, region.clone());
                    c2d.draw(&buffer, region)?;
                }
                Ok(())
            }
        }
    }
}

/// softblit-backed presenter: persistent GPU texture + per-region in-place framebuffer uploads.
#[cfg(target_arch = "wasm32")]
pub(crate) struct GpuCanvas {
    canvas: HtmlCanvasElement,
    surface: softblit::Surface,
}

#[cfg(target_arch = "wasm32")]
impl GpuCanvas {
    fn draw(&mut self, image: &DecodedImage, regions: &[InclusiveRectangle]) -> anyhow::Result<()> {
        let format = softblit_format(image.pixel_format())?;
        if self.surface.format() != format {
            self.surface.set_format(format);
        }
        // The session recreates `DecodedImage` on Deactivation-Reactivation; stay in sync.
        let source_size = (u32::from(image.width()), u32::from(image.height()));
        if self.surface.source_size() != source_size {
            self.surface.resize_source(source_size.0, source_size.1);
        }

        let rects: Vec<softblit::Rect> = regions
            .iter()
            .map(|region| {
                softblit::Rect::new(
                    u32::from(region.left),
                    u32::from(region.top),
                    u32::from(region.width()),
                    u32::from(region.height()),
                )
            })
            .collect();
        self.surface
            .present_external(image.data(), &rects)
            .map_err(|e| anyhow!("softblit present failed: {e}"))?;
        Ok(())
    }

    fn resize(&mut self, width: NonZeroU32, height: NonZeroU32) {
        self.canvas.set_width(width.get());
        self.canvas.set_height(height.get());
        self.surface.resize_source(width.get(), height.get());
        self.surface.resize_target(width.get(), height.get());
    }
}

#[cfg(target_arch = "wasm32")]
fn softblit_format(format: ironrdp::graphics::image_processing::PixelFormat) -> anyhow::Result<softblit::PixelFormat> {
    use ironrdp::graphics::image_processing::PixelFormat as Pf;

    // Alpha is forced opaque in both cases, matching the Canvas2D path's unconditional 0xFF.
    Ok(match format {
        Pf::RgbA32 | Pf::RgbX32 => softblit::PixelFormat::Rgbx8,
        Pf::BgrA32 | Pf::BgrX32 => softblit::PixelFormat::Bgrx8,
        other => anyhow::bail!("pixel format {other:?} is not supported by the GPU presenter"),
    })
}

/// Canvas2D fallback presenter. Owns the canvas's 2D context and a reusable RGBA scratch buffer:
/// each dirty region's pixels are copied once into the scratch (alpha forced opaque), then
/// blitted with `put_image_data` at the region's origin.
pub(crate) struct Canvas2d {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    rgba: Vec<u8>,
}

impl Canvas2d {
    fn new(render_canvas: HtmlCanvasElement) -> anyhow::Result<Self> {
        let ctx = context_2d(&render_canvas)?;

        Ok(Self {
            canvas: render_canvas,
            ctx,
            rgba: Vec::new(),
        })
    }

    /// Setting width/height resets the canvas backing store; the 2D context persists.
    fn resize(&mut self, width: NonZeroU32, height: NonZeroU32) {
        self.canvas.set_width(width.get());
        self.canvas.set_height(height.get());
    }

    /// `buffer` is the region's RGBA sub-image (as produced by `extract_partial_image`).
    fn draw(&mut self, buffer: &[u8], region: InclusiveRectangle) -> anyhow::Result<()> {
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
