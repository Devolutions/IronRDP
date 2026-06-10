//! Native EGFX (MS-RDPEGFX) H.264 graphics: in-process **CPU** decode via openh264, composited into
//! the client's `DecodedImage`.
//!
//! The DVC framework runs the [`GraphicsPipelineHandler`] inside `ActiveStage::process` (a `Send`
//! context) which does not have access to the render loop's `image`. So the handler decodes (the
//! EGFX client owns an `openh264` `H264Decoder` and hands us RGBA via `on_bitmap_updated`) and
//! queues output-space [`Blit`]s into a shared [`NativeGfxState`]; the render loop drains them into
//! `image` after each `process` and presents.
//!
//! [`NativeGfxState`] is plain data (no EGFX types) so it can be threaded through the connect/
//! session signatures unconditionally; only [`NativeGfxHandler`] and its registration depend on the
//! `egfx` feature (and thus on `ironrdp-egfx` / openh264).

use std::sync::{Arc, Mutex};

/// One composited region, in output (framebuffer) coordinates, as tightly-packed RGBA.
pub struct Blit {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub data: Vec<u8>,
}

#[derive(Default)]
struct Inner {
    /// surface id → output origin (`MapSurfaceToOutput`).
    surfaces: std::collections::BTreeMap<u16, (u32, u32)>,
    /// Pending composited regions, drained by the render loop.
    blits: Vec<Blit>,
}

/// Shared EGFX compositor state. Cloneable handle (`Arc`); the handler writes, the render loop
/// drains. Always available regardless of the `egfx` feature (empty when the feature is off).
#[derive(Clone, Default)]
pub struct NativeGfxState(Arc<Mutex<Inner>>);

impl NativeGfxState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Removes and returns all pending blits (call after `ActiveStage::process`).
    pub fn take_blits(&self) -> Vec<Blit> {
        core::mem::take(&mut self.0.lock().expect("egfx state poisoned").blits)
    }

    /// Drops all surface/blit state (on Deactivation-Reactivation).
    pub fn reset(&self) {
        let mut inner = self.0.lock().expect("egfx state poisoned");
        inner.surfaces.clear();
        inner.blits.clear();
    }
}

#[cfg(feature = "egfx")]
mod handler {
    use ironrdp_egfx::client::{BitmapUpdate, GraphicsPipelineHandler, Surface};
    use ironrdp_egfx::pdu::{CapabilitiesV81Flags, CapabilitySet};

    use super::{Blit, NativeGfxState};

    /// EGFX handler that composites decoded (openh264, CPU) RGBA bitmaps into [`NativeGfxState`].
    pub struct NativeGfxHandler {
        state: NativeGfxState,
    }

    impl NativeGfxHandler {
        pub fn new(state: NativeGfxState) -> Self {
            Self { state }
        }

        fn origin(&self, surface_id: u16) -> (u32, u32) {
            self.state
                .0
                .lock()
                .expect("egfx state poisoned")
                .surfaces
                .get(&surface_id)
                .copied()
                .unwrap_or((0, 0))
        }
    }

    fn clamp_u16(value: u32) -> u16 {
        u16::try_from(value).unwrap_or(u16::MAX)
    }

    impl GraphicsPipelineHandler for NativeGfxHandler {
        fn capabilities(&self) -> Vec<CapabilitySet> {
            // AVC420 only — see the web handler's rationale (avoid the server picking a codec we
            // don't render and blanking the screen; non-AVC servers stay on fast-path RemoteFX).
            vec![CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::AVC420_ENABLED | CapabilitiesV81Flags::SMALL_CACHE,
            }]
        }

        fn on_reset_graphics(&mut self, _width: u32, _height: u32) {
            self.state.reset();
        }

        fn on_surface_created(&mut self, surface: &Surface) {
            self.state
                .0
                .lock()
                .expect("egfx state poisoned")
                .surfaces
                .insert(surface.id, (0, 0));
        }

        fn on_surface_mapped(&mut self, surface_id: u16, origin_x: u32, origin_y: u32) {
            self.state
                .0
                .lock()
                .expect("egfx state poisoned")
                .surfaces
                .insert(surface_id, (origin_x, origin_y));
        }

        fn on_surface_deleted(&mut self, surface_id: u16) {
            self.state
                .0
                .lock()
                .expect("egfx state poisoned")
                .surfaces
                .remove(&surface_id);
        }

        fn on_bitmap_updated(&mut self, update: &BitmapUpdate) {
            let (ox, oy) = self.origin(update.surface_id);
            let blit = Blit {
                x: clamp_u16(ox + u32::from(update.destination_rectangle.left)),
                y: clamp_u16(oy + u32::from(update.destination_rectangle.top)),
                width: update.width,
                height: update.height,
                data: update.data.clone(),
            };
            self.state.0.lock().expect("egfx state poisoned").blits.push(blit);
        }
    }
}

#[cfg(feature = "egfx")]
pub use handler::NativeGfxHandler;
