use std::num::NonZeroU32;

use ironrdp::pdu::geometry::{InclusiveRectangle, Rectangle as _};
use softbuffer::{NoDisplayHandle, NoWindowHandle};
use web_sys::HtmlCanvasElement;

pub(crate) struct Canvas {
    width: u32,
    surface: softbuffer::Surface<NoDisplayHandle, NoWindowHandle>,
}

impl Canvas {
    pub(crate) fn new(render_canvas: HtmlCanvasElement, width: u32, height: u32) -> anyhow::Result<Self> {
        render_canvas.set_width(width);
        render_canvas.set_height(height);

        #[cfg(target_arch = "wasm32")]
        let mut surface = {
            use softbuffer::SurfaceExtWeb as _;
            softbuffer::Surface::from_canvas(render_canvas).expect("surface")
        };

        #[cfg(not(target_arch = "wasm32"))]
        let mut surface = {
            fn stub(_: HtmlCanvasElement) -> softbuffer::Surface<NoDisplayHandle, NoWindowHandle> {
                unimplemented!()
            }

            stub(render_canvas)
        };

        surface
            .resize(NonZeroU32::new(width).unwrap(), NonZeroU32::new(height).unwrap())
            .expect("surface resize");

        Ok(Self { width, surface })
    }

    pub(crate) fn draw(&mut self, buffer: &[u8], region: InclusiveRectangle) -> anyhow::Result<()> {
        let region_width = region.width();
        let region_height = region.height();

        let mut src = buffer.chunks_exact(4).map(|pixel| {
            let r = pixel[0];
            let g = pixel[1];
            let b = pixel[2];
            u32::from_be_bytes([0, r, g, b])
        });

        let mut dst = self.surface.buffer_mut().expect("surface buffer");

        {
            // Copy src into dst

            let region_top_usize = usize::from(region.top);
            let region_height_usize = usize::from(region_height);
            let region_left_usize = usize::from(region.left);
            let region_width_usize = usize::from(region_width);

            for dst_row in dst
                .chunks_exact_mut(self.width as usize)
                .skip(region_top_usize)
                .take(region_height_usize)
            {
                let src_row = src.by_ref().take(region_width_usize);

                dst_row
                    .iter_mut()
                    .skip(region_left_usize)
                    .take(region_width_usize)
                    .zip(src_row)
                    .for_each(|(dst, src)| *dst = src);
            }
        }

        let damage_rect = softbuffer::Rect {
            x: u32::from(region.left),
            y: u32::from(region.top),
            width: NonZeroU32::new(u32::from(region_width)).unwrap(),
            height: NonZeroU32::new(u32::from(region_height)).unwrap(),
        };

        dst.present_with_damage(&[damage_rect]).expect("buffer present");

        Ok(())
    }
}
