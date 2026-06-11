//! [`WebGfxHandler`] — the `Send` EGFX handler that lives inside the DVC processor. It does no
//! decoding or GPU work: every `GraphicsPipelineHandler` callback is translated into one
//! [`EgfxUpdate`] and forwarded over an mpsc channel to the render loop (see [`super`]).

use futures_channel::mpsc::UnboundedSender;
use ironrdp::pdu::geometry::ExclusiveRectangle;
use ironrdp_egfx::client::{BitmapUpdate, GraphicsPipelineHandler, Surface};
use ironrdp_egfx::pdu::{CapabilitiesV81Flags, CapabilitySet};
use tracing::{info, trace};

use super::EgfxUpdate;
use crate::session::RdpInputEvent;

/// Forwards EGFX events to the render loop over the session's input-event channel (as
/// [`RdpInputEvent::Egfx`]). `EgfxUpdate` is `Send`, so this handler stays `Send` as the DVC
/// framework requires; decoded `VideoFrame`s (`!Send`) travel separately, never through here.
pub(crate) struct WebGfxHandler {
    tx: UnboundedSender<RdpInputEvent>,
}

impl WebGfxHandler {
    pub(crate) fn new(tx: UnboundedSender<RdpInputEvent>) -> Self {
        Self { tx }
    }

    fn send(&self, update: EgfxUpdate) {
        // The render loop owns the receiver for the session's lifetime; a send error only means the
        // session is tearing down, in which case dropping the update is correct.
        if self.tx.unbounded_send(RdpInputEvent::Egfx(update)).is_err() {
            trace!("EGFX update dropped: render loop receiver gone");
        }
    }
}

impl GraphicsPipelineHandler for WebGfxHandler {
    fn capabilities(&self) -> Vec<CapabilitySet> {
        // Advertise **only** AVC420 — the one codec we can WebCodecs-decode. Deliberately omit the
        // no-AVC (V8) and AVC444 (V10.x) sets: if the server can't do AVC420 it then won't activate
        // EGFX at all and graphics keep flowing over fast-path RemoteFX, rather than the server
        // filling an EGFX surface with a codec we don't render (RFX Progressive / AVC444) and
        // leaving the screen blank.
        vec![CapabilitySet::V8_1 {
            flags: CapabilitiesV81Flags::AVC420_ENABLED | CapabilitiesV81Flags::SMALL_CACHE,
        }]
    }

    fn on_capabilities_confirmed(&mut self, caps: &CapabilitySet) {
        // If you see this, the server activated the EGFX graphics pipeline (H.264 should follow).
        // If it never appears, the server is using fast-path RemoteFX, not EGFX — enable H.264/AVC
        // on the host (e.g. the "Prioritize H.264/AVC 444 Graphics mode" GPO).
        info!(?caps, "EGFX graphics pipeline active");
    }

    fn on_reset_graphics(&mut self, width: u32, height: u32) {
        self.send(EgfxUpdate::ResetGraphics { width, height });
    }

    fn on_surface_created(&mut self, surface: &Surface) {
        self.send(EgfxUpdate::SurfaceCreated {
            id: surface.id,
            width: surface.width,
            height: surface.height,
        });
    }

    fn on_surface_mapped(&mut self, surface_id: u16, origin_x: u32, origin_y: u32) {
        self.send(EgfxUpdate::SurfaceMapped {
            id: surface_id,
            origin_x,
            origin_y,
        });
    }

    fn on_surface_deleted(&mut self, surface_id: u16) {
        self.send(EgfxUpdate::SurfaceDeleted { id: surface_id });
    }

    fn on_bitmap_updated(&mut self, update: &BitmapUpdate) {
        self.send(EgfxUpdate::Bitmap {
            surface_id: update.surface_id,
            dst: update.destination_rectangle.clone(),
            width: update.width,
            height: update.height,
            data: update.data.clone(),
        });
    }

    fn on_avc420_bitstream(&mut self, surface_id: u16, destination_rectangle: &ExclusiveRectangle, h264_data: &[u8]) {
        self.send(EgfxUpdate::Avc420 {
            surface_id,
            dst: destination_rectangle.clone(),
            bitstream: h264_data.to_vec(),
        });
    }

    fn on_frame_complete(&mut self, frame_id: u32) {
        self.send(EgfxUpdate::FrameComplete { frame_id });
    }

    fn on_close(&mut self) {
        self.send(EgfxUpdate::Close);
    }
}
