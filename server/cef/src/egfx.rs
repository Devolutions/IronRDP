use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ironrdp_dvc::{DvcMessageBatch, encode_dvc_messages};
use ironrdp_egfx::pdu::{Avc420Region, CapabilitiesAdvertisePdu, CapabilitySet};
use ironrdp_egfx::server::{GraphicsPipelineHandler, GraphicsPipelineServer};
use ironrdp_pdu::geometry::ExclusiveRectangle;
use ironrdp_server::{GfxContext, GfxDvcBridge, GfxServerFactory, GfxServerHandle, ServerEvent, ServerEventSender};
use ironrdp_svc::ChannelFlags;
use tokio::sync::{Notify, Semaphore, mpsc::UnboundedSender, oneshot};

use crate::rdp::Frame;

const AVC420_QP: u8 = 22;

#[derive(Clone)]
pub(crate) struct EgfxOutput {
    shared: Arc<Mutex<Shared>>,
}

struct Shared {
    events: Option<UnboundedSender<ServerEvent>>,
    connection_token: Arc<AtomicBool>,
    notify: Arc<Notify>,
    epoch: Arc<AtomicU64>,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            events: None,
            connection_token: Arc::new(AtomicBool::new(true)),
            notify: Arc::new(Notify::new()),
            epoch: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl EgfxOutput {
    pub(crate) fn new() -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared::default())),
        }
    }

    pub(crate) fn disconnected(&self) {
        let mut shared = self.shared.lock().expect("egfx state poisoned");
        shared.connection_token.store(false, Ordering::Release);
        shared.connection_token = Arc::new(AtomicBool::new(true));
        shared.notify = Arc::new(Notify::new());
        shared.epoch = Arc::new(AtomicU64::new(0));
    }

    pub(crate) fn presenter(&self, handle: Option<GfxServerHandle>) -> EgfxPresenter {
        let shared = self.shared.lock().expect("egfx state poisoned");
        EgfxPresenter {
            handle,
            events: shared.events.clone(),
            token: Arc::clone(&shared.connection_token),
            notify: Arc::clone(&shared.notify),
            epoch: Arc::clone(&shared.epoch),
            permits: Arc::new(Semaphore::new(2)),
            started: Instant::now(),
            surface: None,
            previous_bgra: Vec::new(),
            h264: None,
            pending_write: None,
            presented_epoch: 0,
            backpressured: false,
        }
    }

    pub(crate) fn factory(&self) -> Box<dyn GfxServerFactory> {
        Box::new(self.clone())
    }
}

impl ServerEventSender for EgfxOutput {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        self.shared.lock().expect("egfx state poisoned").events = Some(sender);
    }
}

impl GfxServerFactory for EgfxOutput {
    fn build_gfx_handler(&self, _ctx: GfxContext) -> Box<dyn GraphicsPipelineHandler> {
        let shared = self.shared.lock().expect("egfx state poisoned");
        Box::new(Handler {
            notify: Arc::clone(&shared.notify),
            epoch: Arc::clone(&shared.epoch),
        })
    }

    fn build_server_with_handle(&self, ctx: GfxContext) -> Option<(GfxDvcBridge, GfxServerHandle)> {
        let server = GraphicsPipelineServer::new(self.build_gfx_handler(ctx));
        let handle = Arc::new(Mutex::new(server));
        Some((GfxDvcBridge::new(Arc::clone(&handle)), handle))
    }
}

struct Handler {
    notify: Arc<Notify>,
    epoch: Arc<AtomicU64>,
}

impl GraphicsPipelineHandler for Handler {
    fn capabilities_advertise(&mut self, _pdu: &CapabilitiesAdvertisePdu) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }

    fn on_ready(&mut self, _negotiated: &CapabilitySet) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.notify.notify_waiters();
    }

    fn on_frame_ack(&mut self, _frame_id: u32, _queue_depth: u32, _total_frames_decoded: u32) {
        self.notify.notify_waiters();
    }

    fn on_close(&mut self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.notify.notify_waiters();
    }
}

pub(crate) enum PresentOutcome {
    NotReady,
    Backpressured,
    Unchanged,
    Queued,
}

struct H264EncoderState {
    encoder: openh264::encoder::Encoder,
    y_plane: Vec<u8>,
    u_plane: Vec<u8>,
    v_plane: Vec<u8>,
    aligned_width: usize,
    aligned_height: usize,
}

pub(crate) struct EgfxPresenter {
    handle: Option<GfxServerHandle>,
    events: Option<UnboundedSender<ServerEvent>>,
    token: Arc<AtomicBool>,
    notify: Arc<Notify>,
    epoch: Arc<AtomicU64>,
    permits: Arc<Semaphore>,
    started: Instant,
    surface: Option<(u16, u16, u16)>,
    previous_bgra: Vec<u8>,
    h264: Option<H264EncoderState>,
    pending_write: Option<oneshot::Receiver<()>>,
    presented_epoch: u64,
    backpressured: bool,
}

impl EgfxPresenter {
    pub(crate) fn has_pending_write(&self) -> bool {
        self.pending_write.is_some()
    }

    pub(crate) async fn finish_pending_write(&mut self) {
        if let Some(rx) = &mut self.pending_write {
            let _ = rx.await;
            self.pending_write = None;
        }
    }

    pub(crate) fn is_attached(&self) -> bool {
        self.handle.is_some()
    }

    pub(crate) async fn notified(&self) {
        self.notify.notified().await;
    }

    pub(crate) fn needs_repaint(&self) -> bool {
        let Some(handle) = self.handle.as_ref() else {
            return false;
        };
        let gfx = handle.lock().expect("egfx handle poisoned");
        if !gfx.is_ready() || gfx.should_backpressure() || self.permits.available_permits() == 0 {
            return false;
        }
        let current_epoch = self.epoch.load(Ordering::Acquire);
        self.backpressured || (current_epoch != 0 && self.presented_epoch != current_epoch)
    }

    pub(crate) fn present(&mut self, frame: &Frame) -> PresentOutcome {
        let Some(handle) = self.handle.as_ref() else {
            return PresentOutcome::NotReady;
        };
        let Some(events) = self.events.as_ref() else {
            return PresentOutcome::NotReady;
        };
        if !self.token.load(Ordering::Acquire) {
            return PresentOutcome::NotReady;
        }
        let expected_len = usize::from(frame.width) * usize::from(frame.height) * 4;
        if frame.width == 0 || frame.height == 0 || frame.pixels.len() != expected_len {
            return PresentOutcome::NotReady;
        }

        let target_epoch = self.epoch.load(Ordering::Acquire);
        let (channel_id, use_avc420, surface_id, need_reset) = {
            let mut gfx = handle.lock().expect("egfx handle poisoned");
            if !gfx.is_ready() {
                self.presented_epoch = 0;
                self.surface = None;
                return PresentOutcome::NotReady;
            }
            let Some(channel_id) = gfx.channel_id() else {
                return PresentOutcome::NotReady;
            };
            if gfx.should_backpressure() {
                self.backpressured = true;
                return PresentOutcome::Backpressured;
            }
            let epoch_changed = self.presented_epoch != target_epoch;
            let existing = (!epoch_changed)
                .then_some(self.surface)
                .flatten()
                .filter(|(sid, width, height)| {
                    (*width, *height) == (frame.width, frame.height) && gfx.get_surface(*sid).is_some()
                });
            let (surface_id, need_reset) = match existing {
                Some((sid, _, _)) => (sid, false),
                None => {
                    gfx.resize(frame.width, frame.height);
                    let Some(sid) = gfx.create_surface(frame.width, frame.height) else {
                        return PresentOutcome::NotReady;
                    };
                    gfx.map_surface_to_output(sid, 0, 0);
                    self.surface = Some((sid, frame.width, frame.height));
                    (sid, true)
                }
            };
            let use_avc420 = gfx.supports_avc420();
            (channel_id, use_avc420, surface_id, need_reset)
        };

        if need_reset {
            self.previous_bgra.clear();
            self.h264 = None;
        }

        let Ok(permit) = Arc::clone(&self.permits).try_acquire_owned() else {
            self.backpressured = true;
            return PresentOutcome::Backpressured;
        };

        let timestamp_ms = u32::try_from(self.started.elapsed().as_millis() & u128::from(u32::MAX)).unwrap_or(0);
        let mut encoded_avc = None;
        if use_avc420 {
            match encode_bgra_avc420(&mut self.h264, frame.width, frame.height, &frame.pixels, need_reset) {
                Ok(bitstream) => encoded_avc = Some(bitstream),
                Err(error) => {
                    tracing::warn!(%error, "OpenH264 EGFX encode failed; falling back to ZGFX surface region");
                    self.h264 = None;
                }
            }
        }

        let dvc_messages = {
            let mut gfx = handle.lock().expect("egfx handle poisoned");
            if !gfx.is_ready() || gfx.channel_id() != Some(channel_id) {
                return PresentOutcome::NotReady;
            }
            if gfx.should_backpressure() {
                self.backpressured = true;
                return PresentOutcome::Backpressured;
            }
            if let Some(bitstream) = encoded_avc {
                if bitstream.is_empty() {
                    self.backpressured = true;
                    return PresentOutcome::Backpressured;
                }
                let region = Avc420Region::full_frame(frame.width, frame.height, AVC420_QP);
                if gfx
                    .send_avc420_frame(surface_id, &bitstream, &[region], timestamp_ms)
                    .is_none()
                {
                    self.backpressured = true;
                    return PresentOutcome::Backpressured;
                }
                // Invalidate the uncompressed dirty-region baseline whenever AVC updates the surface.
                self.previous_bgra.clear();
                gfx.drain_output()
            } else {
                match extract_dirty_region(&self.previous_bgra, &frame.pixels, frame.width, frame.height) {
                    Some((rect, dirty_bgra)) => {
                        if gfx
                            .try_send_uncompressed_region(surface_id, &dirty_bgra, rect, timestamp_ms)
                            .is_err()
                        {
                            self.backpressured = true;
                            return PresentOutcome::Backpressured;
                        }
                        self.previous_bgra.clear();
                        self.previous_bgra.extend_from_slice(&frame.pixels);
                        gfx.drain_output()
                    }
                    None => {
                        let pending = gfx.drain_output();
                        if pending.is_empty() {
                            self.presented_epoch = target_epoch;
                            self.backpressured = false;
                            return PresentOutcome::Unchanged;
                        }
                        pending
                    }
                }
            }
        };

        if dvc_messages.is_empty() {
            self.presented_epoch = target_epoch;
            self.backpressured = false;
            return PresentOutcome::Unchanged;
        }
        let Ok(messages) = encode_dvc_messages(channel_id, dvc_messages, ChannelFlags::SHOW_PROTOCOL) else {
            return PresentOutcome::NotReady;
        };
        let Ok(batch) = DvcMessageBatch::try_new(channel_id, messages) else {
            return PresentOutcome::NotReady;
        };
        let (completion, receiver) = oneshot::channel();
        let token = Arc::clone(&self.token);
        if events
            .send(ServerEvent::DynamicChannelWithCompletion {
                batch,
                completion,
                permit,
                is_current: Box::new(move || token.load(Ordering::Acquire)),
            })
            .is_err()
        {
            return PresentOutcome::NotReady;
        }
        // Store the receiver in `self` so `tokio::select!` cancelling `next_update`
        // to run `RdpServer`'s event loop does not drop `receiver` and cancel the batch.
        self.pending_write = Some(receiver);
        self.presented_epoch = target_epoch;
        self.backpressured = false;
        PresentOutcome::Queued
    }
}

fn encode_bgra_avc420(
    slot: &mut Option<H264EncoderState>,
    width: u16,
    height: u16,
    bgra: &[u8],
    force_idr: bool,
) -> anyhow::Result<Vec<u8>> {
    let w = usize::from(width);
    let h = usize::from(height);
    // MS-RDPEGFX 2.2.4.4: AVC420 streams are coded at 16-pixel macroblock multiples
    // and cropped on the client through the region rectangle.
    let aligned_w = w.next_multiple_of(16);
    let aligned_h = h.next_multiple_of(16);
    let state = match slot {
        Some(existing) if (existing.aligned_width, existing.aligned_height) == (aligned_w, aligned_h) => existing,
        _ => {
            let config = openh264::encoder::EncoderConfig::new().skip_frames(false);
            let encoder = openh264::encoder::Encoder::with_api_config(openh264::OpenH264API::from_source(), config)?;
            let luma_len = aligned_w * aligned_h;
            let chroma_len = (aligned_w / 2) * (aligned_h / 2);
            *slot = Some(H264EncoderState {
                encoder,
                y_plane: vec![0; luma_len],
                u_plane: vec![128; chroma_len],
                v_plane: vec![128; chroma_len],
                aligned_width: aligned_w,
                aligned_height: aligned_h,
            });
            slot.as_mut().expect("initialized H264 encoder")
        }
    };
    if force_idr {
        state.encoder.force_intra_frame();
    }
    state.fill_bt709_full_i420(bgra, w, h);
    let yuv = openh264::formats::YUVSlices::new(
        (&state.y_plane, &state.u_plane, &state.v_plane),
        (aligned_w, aligned_h),
        (aligned_w, aligned_w / 2, aligned_w / 2),
    );
    Ok(state.encoder.encode(&yuv)?.to_vec())
}

impl H264EncoderState {
    /// Converts BGRA pixels into 16-pixel-padded full-range BT.709 I420 planes per MS-RDPEGFX 3.3.8.3.1.
    fn fill_bt709_full_i420(&mut self, bgra: &[u8], width: usize, height: usize) {
        let src_stride = width * 4;
        let aligned_w = self.aligned_width;
        let half_w = aligned_w / 2;
        let half_h = self.aligned_height / 2;

        for by in 0..half_h {
            let y0 = (by * 2).min(height - 1);
            let y1 = (by * 2 + 1).min(height - 1);
            let row0 = &bgra[y0 * src_stride..(y0 + 1) * src_stride];
            let row1 = &bgra[y1 * src_stride..(y1 + 1) * src_stride];
            let luma_row0 = by * 2 * aligned_w;
            let luma_row1 = (by * 2 + 1) * aligned_w;
            let chroma_row = by * half_w;

            for bx in 0..half_w {
                let x0 = (bx * 2).min(width - 1) * 4;
                let x1 = (bx * 2 + 1).min(width - 1) * 4;

                let (y00, u00, v00) = bt709_full_rgb_to_yuv(row0[x0 + 2], row0[x0 + 1], row0[x0]);
                let (y01, u01, v01) = bt709_full_rgb_to_yuv(row0[x1 + 2], row0[x1 + 1], row0[x1]);
                let (y10, u10, v10) = bt709_full_rgb_to_yuv(row1[x0 + 2], row1[x0 + 1], row1[x0]);
                let (y11, u11, v11) = bt709_full_rgb_to_yuv(row1[x1 + 2], row1[x1 + 1], row1[x1]);

                self.y_plane[luma_row0 + bx * 2] = y00;
                self.y_plane[luma_row0 + bx * 2 + 1] = y01;
                self.y_plane[luma_row1 + bx * 2] = y10;
                self.y_plane[luma_row1 + bx * 2 + 1] = y11;

                self.u_plane[chroma_row + bx] = average4(u00, u01, u10, u11);
                self.v_plane[chroma_row + bx] = average4(v00, v01, v10, v11);
            }
        }
    }
}

fn bt709_full_rgb_to_yuv(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    (
        clamp_u8((54 * r + 183 * g + 18 * b) >> 8),
        clamp_u8(((-29 * r - 99 * g + 128 * b) >> 8) + 128),
        clamp_u8(((128 * r - 116 * g - 12 * b) >> 8) + 128),
    )
}

fn average4(a: u8, b: u8, c: u8, d: u8) -> u8 {
    let sum = u16::from(a) + u16::from(b) + u16::from(c) + u16::from(d) + 2;
    u8::try_from(sum >> 2).expect("average of four u8 values fits in u8")
}

fn clamp_u8(value: i32) -> u8 {
    let [low, ..] = value.clamp(0, 255).to_le_bytes();
    low
}

fn extract_dirty_region(
    previous: &[u8],
    current: &[u8],
    width: u16,
    height: u16,
) -> Option<(ExclusiveRectangle, Vec<u8>)> {
    let w = usize::from(width);
    let h = usize::from(height);
    let stride = w * 4;
    if previous.len() != current.len() {
        return Some((
            ExclusiveRectangle {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            },
            current.to_vec(),
        ));
    }

    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0usize;
    let mut max_y = 0usize;

    for y in 0..h {
        let row_offset = y * stride;
        let prev_row = &previous[row_offset..row_offset + stride];
        let curr_row = &current[row_offset..row_offset + stride];
        if prev_row == curr_row {
            continue;
        }
        min_y = min_y.min(y);
        max_y = y + 1;
        for x in 0..w {
            let px = x * 4;
            if prev_row[px..px + 4] != curr_row[px..px + 4] {
                min_x = min_x.min(x);
                break;
            }
        }
        for x in (min_x..w).rev() {
            let px = x * 4;
            if prev_row[px..px + 4] != curr_row[px..px + 4] {
                max_x = max_x.max(x + 1);
                break;
            }
        }
    }

    if min_x >= max_x || min_y >= max_y {
        return None;
    }

    let rect = ExclusiveRectangle {
        left: u16::try_from(min_x).expect("bounded dirty left"),
        top: u16::try_from(min_y).expect("bounded dirty top"),
        right: u16::try_from(max_x).expect("bounded dirty right"),
        bottom: u16::try_from(max_y).expect("bounded dirty bottom"),
    };
    let rect_stride = (max_x - min_x) * 4;
    let mut packed = Vec::with_capacity((max_y - min_y) * rect_stride);
    for y in min_y..max_y {
        let start = y * stride + min_x * 4;
        packed.extend_from_slice(&current[start..start + rect_stride]);
    }
    Some((rect, packed))
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use ironrdp_core::encode_vec;
    use ironrdp_dvc::DvcProcessor as _;
    use ironrdp_egfx::pdu::{CapabilitiesV8Flags, GfxPdu};
    use tokio::sync::mpsc::unbounded_channel;

    use super::*;

    // oracle: mutant sending full-screen buffers or miscomputing row strides for interior dirty rectangles.
    #[test]
    fn dirty_region_isolates_changed_subrectangle_and_skips_identical_frames() {
        let prev = vec![0x11u8; 4 * 4 * 4];
        assert!(extract_dirty_region(&prev, &prev, 4, 4).is_none());

        let mut curr = prev.clone();
        // Mutate pixel (2, 1) and pixel (3, 2) -> bounding box [2..4, 1..3] (2x2 pixels = 16 bytes).
        curr[(4 + 2) * 4..((4 + 2) * 4) + 4].copy_from_slice(&[1, 2, 3, 255]);
        curr[(8 + 3) * 4..((8 + 3) * 4) + 4].copy_from_slice(&[4, 5, 6, 255]);

        let (rect, data) = extract_dirty_region(&prev, &curr, 4, 4).expect("dirty rect");
        assert_eq!(
            rect,
            ExclusiveRectangle {
                left: 2,
                top: 1,
                right: 4,
                bottom: 3,
            }
        );
        assert_eq!(data.len(), 2 * 2 * 4);
        assert_eq!(&data[..4], &[1, 2, 3, 255]);
        assert_eq!(&data[12..16], &[4, 5, 6, 255]);
    }

    // oracle: spec MS-RDPEGFX §§2.2.2.1 and 3.3.5; mutant failing to create/map surface, encode AVC420, or invalidate on disconnect.
    #[test]
    fn presenter_falls_back_before_ready_streams_avc420_when_negotiated_and_invalidates_on_disconnect() {
        let mut output = EgfxOutput::new();
        let (sender, mut events) = unbounded_channel();
        output.set_sender(sender);
        let epoch = Arc::clone(&output.shared.lock().unwrap().epoch);
        let server = Arc::new(Mutex::new(GraphicsPipelineServer::new(Box::new(Handler {
            notify: Arc::new(Notify::new()),
            epoch,
        }))));
        let mut presenter = output.presenter(Some(Arc::clone(&server)));
        let frame = Frame {
            width: 16,
            height: 16,
            pixels: Bytes::from(vec![0x80; 16 * 16 * 4]),
        };

        assert!(matches!(presenter.present(&frame), PresentOutcome::NotReady));

        {
            let mut gfx = server.lock().unwrap();
            gfx.start(7).unwrap();
            let advertise =
                GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V8_1 {
                    flags: ironrdp_egfx::pdu::CapabilitiesV81Flags::AVC420_ENABLED
                        | ironrdp_egfx::pdu::CapabilitiesV81Flags::SMALL_CACHE,
                }]));
            gfx.process(7, &encode_vec(&advertise).unwrap()).unwrap();
            assert!(gfx.is_ready());
            assert!(gfx.supports_avc420());
        }

        assert!(matches!(presenter.present(&frame), PresentOutcome::Queued));
        assert!(presenter.has_pending_write());
        let ServerEvent::DynamicChannelWithCompletion {
            batch,
            completion,
            is_current,
            ..
        } = events.try_recv().expect("queued EGFX batch")
        else {
            panic!("expected DynamicChannelWithCompletion");
        };
        assert!(
            !completion.is_closed(),
            "presenter must retain completion receiver across select polls"
        );
        assert_eq!(batch.channel_id(), 7);
        assert!(is_current());

        output.disconnected();
        assert!(!is_current(), "disconnect must invalidate queued EGFX frame batches");

        // Also check V8 uncompressed dirty-region path and static-screen recovery on capability re-advertisement.
        let v8_epoch = Arc::clone(&output.shared.lock().unwrap().epoch);
        let v8_notify = Arc::clone(&output.shared.lock().unwrap().notify);
        let v8_server = Arc::new(Mutex::new(GraphicsPipelineServer::new(Box::new(Handler {
            notify: v8_notify,
            epoch: v8_epoch,
        }))));
        let advertise = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V8 {
            flags: CapabilitiesV8Flags::SMALL_CACHE,
        }]));
        let advertise_bytes = encode_vec(&advertise).unwrap();
        let mut v8_presenter = output.presenter(Some(Arc::clone(&v8_server)));
        {
            let mut gfx = v8_server.lock().unwrap();
            gfx.start(9).unwrap();
            gfx.process(9, &advertise_bytes).unwrap();
            assert!(!gfx.supports_avc420());
        }
        assert!(v8_presenter.needs_repaint());
        assert!(matches!(v8_presenter.present(&frame), PresentOutcome::Queued));
        assert!(!v8_presenter.needs_repaint());
        assert!(matches!(v8_presenter.present(&frame), PresentOutcome::Unchanged));

        // Client re-advertising capabilities clears server surfaces; presenter must detect the new epoch and repaint.
        v8_server.lock().unwrap().process(9, &advertise_bytes).unwrap();
        assert!(v8_presenter.needs_repaint());
        assert!(matches!(v8_presenter.present(&frame), PresentOutcome::Queued));

        // Non-16-aligned dimensions (21x19) must pad to 32x32 for AVC420 without failing.
        let mut slot = None;
        let odd_bgra = [0u8, 255, 0, 255].repeat(21 * 19);
        let bitstream = encode_bgra_avc420(&mut slot, 21, 19, &odd_bgra, true).expect("padded AVC420 encode");
        assert!(!bitstream.is_empty());
        assert_eq!(slot.as_ref().unwrap().aligned_width, 32);
        assert_eq!(slot.as_ref().unwrap().aligned_height, 32);
    }
}
