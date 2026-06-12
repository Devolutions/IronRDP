//! [`WebGfxHandler`] — the `Send` EGFX handler that lives inside the DVC processor. It does no
//! decoding or GPU work: every `GraphicsPipelineHandler` callback is translated into one
//! [`EgfxUpdate`] and forwarded over an mpsc channel to the render loop (see [`super`]).

use futures_channel::mpsc::UnboundedSender;
use ironrdp::pdu::geometry::ExclusiveRectangle;
use ironrdp_egfx::client::{BitmapUpdate, GraphicsPipelineHandler, Surface};
use ironrdp_egfx::pdu::{CapabilitiesV104Flags, CapabilitiesV107Flags, CapabilitiesV81Flags, CapabilitySet};
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
        // Logged when the EGFX DVC channel opens and we advertise. If you see THIS but not
        // "EGFX graphics pipeline active", the server opened EGFX but declined our offer.
        // If you see neither, the server never opened EGFX (pure fast-path RemoteFX).
        info!("EGFX channel opened — advertising AVC (V10.7/10.6 + V8.1) capability");
        // Windows servers only set "AVC available" for clients that advertise the V10.x capsets;
        // a V8.1-only advertisement is treated as non-AVC and the server falls back to RemoteFX
        // (confirmed via RdpCoreTS event 162 "AVC available: 0"). The server confirms the *highest*
        // advertised version it supports, so we offer a descending ladder — V10.7 down to V10.4, then
        // V8.1 — to land on whatever the host tops out at (Server 2022 negotiates V10.6 / 0xA0600).
        // AVC is enabled by leaving AVC_DISABLED clear; the server then picks AVC420 vs AVC444 per its
        // "Prioritize H.264/AVC 444" policy. Our WebCodecs path decodes the AVC420 bitstream
        // (`on_avc420_bitstream`); AVC444 is not yet rendered, so the host should stay in AVC420 mode.
        vec![
            CapabilitySet::V10_7 {
                flags: CapabilitiesV107Flags::SMALL_CACHE,
            },
            CapabilitySet::V10_6 {
                flags: CapabilitiesV104Flags::SMALL_CACHE,
            },
            CapabilitySet::V10_5 {
                flags: CapabilitiesV104Flags::SMALL_CACHE,
            },
            CapabilitySet::V10_4 {
                flags: CapabilitiesV104Flags::SMALL_CACHE,
            },
            CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::AVC420_ENABLED | CapabilitiesV81Flags::SMALL_CACHE,
            },
        ]
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
