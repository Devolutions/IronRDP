//! [`EgfxCompositor`] — the render-loop-side state for EGFX: a surface registry (surface id →
//! output origin) and the [`WebCodecsH264Decoder`]. It turns [`EgfxUpdate`]s into either a CPU write
//! into the shared `DecodedImage` (returning the output-space dirty rectangle) or an async AVC
//! decode dispatch. Decoded video arrives separately as [`DecodedVideo`] and is imported by the
//! render loop straight into softblit's GPU texture.
//!
//! Surface coordinates are surface-local; output/framebuffer coordinates add the surface's map
//! origin (`MapSurfaceToOutput`). v1 assumes the common case of surface(s) mapped into the desktop;
//! offscreen surface caching (`SurfaceToCache`/`CacheToSurface`) is not yet modeled.

use std::collections::BTreeMap;

use ironrdp::pdu::geometry::{ExclusiveRectangle, InclusiveRectangle};
use ironrdp::session::image::DecodedImage;
use tracing::{debug, trace};

use super::EgfxUpdate;
use super::decoder::{DecodedVideoQueue, WebCodecsH264Decoder};

struct MappedSurface {
    origin_x: u32,
    origin_y: u32,
}

pub(crate) struct EgfxCompositor {
    surfaces: BTreeMap<u16, MappedSurface>,
    decoder: WebCodecsH264Decoder,
}

impl EgfxCompositor {
    pub(crate) fn new(decoded: DecodedVideoQueue) -> anyhow::Result<Self> {
        Ok(Self {
            surfaces: BTreeMap::new(),
            decoder: WebCodecsH264Decoder::new(decoded)?,
        })
    }

    /// Applies one EGFX update.
    ///
    /// Returns `Some(rect)` (output-space, inclusive) when a CPU bitmap was composited into `image`
    /// and the render loop should mark it dirty. AVC frames return `None` — their pixels arrive
    /// asynchronously as [`DecodedVideo`] and are imported directly to the GPU.
    pub(crate) fn apply_update(&mut self, update: EgfxUpdate, image: &mut DecodedImage) -> Option<InclusiveRectangle> {
        match update {
            EgfxUpdate::ResetGraphics { width, height } => {
                debug!(width, height, "EGFX ResetGraphics");
                self.surfaces.clear();
                self.decoder.reset();
                None
            }
            EgfxUpdate::SurfaceCreated { id, width, height } => {
                trace!(id, width, height, "EGFX surface created");
                self.surfaces.insert(id, MappedSurface { origin_x: 0, origin_y: 0 });
                None
            }
            EgfxUpdate::SurfaceMapped { id, origin_x, origin_y } => {
                trace!(id, origin_x, origin_y, "EGFX surface mapped");
                if let Some(surface) = self.surfaces.get_mut(&id) {
                    surface.origin_x = origin_x;
                    surface.origin_y = origin_y;
                }
                None
            }
            EgfxUpdate::SurfaceDeleted { id } => {
                self.surfaces.remove(&id);
                None
            }
            EgfxUpdate::Bitmap {
                surface_id,
                dst,
                width,
                height,
                data,
            } => {
                let (ox, oy) = self.origin(surface_id);
                let x = clamp_u16(ox + u32::from(dst.left));
                let y = clamp_u16(oy + u32::from(dst.top));
                image.composite_rect(x, y, width, height, &data);
                Some(output_rect(x, y, width, height, image))
            }
            EgfxUpdate::Avc420 {
                surface_id,
                dst,
                bitstream,
            } => {
                let (ox, oy) = self.origin(surface_id);
                let out_dst = ExclusiveRectangle {
                    left: clamp_u16(ox + u32::from(dst.left)),
                    top: clamp_u16(oy + u32::from(dst.top)),
                    right: clamp_u16(ox + u32::from(dst.right)),
                    bottom: clamp_u16(oy + u32::from(dst.bottom)),
                };
                self.decoder.decode(surface_id, out_dst, &bitstream);
                None
            }
            EgfxUpdate::FrameComplete { .. } => None,
            EgfxUpdate::Close => {
                self.surfaces.clear();
                None
            }
        }
    }

    /// Drops all surface state and resets the decoder. Called on a Deactivation-Reactivation
    /// sequence, when the framebuffer is recreated and old surface coordinates no longer apply.
    pub(crate) fn reset(&mut self) {
        self.surfaces.clear();
        self.decoder.reset();
    }

    fn origin(&self, surface_id: u16) -> (u32, u32) {
        self.surfaces
            .get(&surface_id)
            .map(|s| (s.origin_x, s.origin_y))
            .unwrap_or((0, 0))
    }
}

fn clamp_u16(value: u32) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

/// Output-space inclusive dirty rect for a `width`×`height` blit at `(x, y)`, clamped to the image.
fn output_rect(x: u16, y: u16, width: u16, height: u16, image: &DecodedImage) -> InclusiveRectangle {
    let max_x = u32::from(image.width()).saturating_sub(1);
    let max_y = u32::from(image.height()).saturating_sub(1);
    let right = (u32::from(x) + u32::from(width)).saturating_sub(1).min(max_x);
    let bottom = (u32::from(y) + u32::from(height)).saturating_sub(1).min(max_y);
    InclusiveRectangle {
        left: x,
        top: y,
        right: clamp_u16(right),
        bottom: clamp_u16(bottom),
    }
}
