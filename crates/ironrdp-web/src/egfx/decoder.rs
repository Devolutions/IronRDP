//! [`WebCodecsH264Decoder`] — hardware H.264 decode via the browser `VideoDecoder` (WebCodecs).
//!
//! Lives on the render-loop side (`!Send`: owns JS objects). It is deliberately **not** an
//! `ironrdp_egfx::decode::H264Decoder` impl — that trait is synchronous + `Send`, which the async
//! WebCodecs decoder cannot satisfy. Instead the EGFX client runs in `with_external_avc_decode()`
//! mode and hands us the raw bitstream; we submit it to `VideoDecoder` and the decoded `VideoFrame`s
//! arrive asynchronously on the `output` callback, which forwards them as [`DecodedVideo`].
//!
//! WebCodecs preserves decode order, so a FIFO of `(surface_id, dst)` pushed at submit time and
//! popped on output correlates each frame with its destination.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use anyhow::anyhow;
use ironrdp::pdu::geometry::ExclusiveRectangle;
use tracing::{error, info, warn};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;
use web_sys::{
    EncodedVideoChunk, EncodedVideoChunkInit, EncodedVideoChunkType, VideoDecoder, VideoDecoderConfig,
    VideoDecoderInit, VideoFrame,
};

/// A decoded frame plus where it goes (output-space rectangle; only the top-left is used for import).
pub(crate) struct DecodedVideo {
    pub(crate) surface_id: u16,
    pub(crate) dst: ExclusiveRectangle,
    pub(crate) frame: VideoFrame,
}

type Correlation = Rc<RefCell<VecDeque<(u16, ExclusiveRectangle)>>>;
/// Frames decoded asynchronously land here; the render loop drains and imports them at present time.
/// Shared (same-thread `Rc`) between the `VideoDecoder` output callback and the render loop — never
/// crosses a thread, so the `!Send` `VideoFrame` never enters a `Send` channel.
pub(crate) type DecodedVideoQueue = Rc<RefCell<Vec<DecodedVideo>>>;

pub(crate) struct WebCodecsH264Decoder {
    decoder: VideoDecoder,
    // Kept alive for the decoder's lifetime; dropping them would dangle the JS callbacks.
    _output_cb: Closure<dyn FnMut(VideoFrame)>,
    _error_cb: Closure<dyn FnMut(web_sys::DomException)>,
    pending: Correlation,
    configured: bool,
    timestamp: i32,
    annexb: Vec<u8>,
    samples_logged: u8,
}

impl WebCodecsH264Decoder {
    pub(crate) fn new(decoded: DecodedVideoQueue) -> anyhow::Result<Self> {
        let pending: Correlation = Rc::new(RefCell::new(VecDeque::new()));

        let pending_out = Rc::clone(&pending);
        let output_cb = Closure::wrap(Box::new(move |frame: VideoFrame| {
            // Decode order == submit order, so the front of the FIFO is this frame's destination.
            if let Some((surface_id, dst)) = pending_out.borrow_mut().pop_front() {
                decoded.borrow_mut().push(DecodedVideo { surface_id, dst, frame });
            } else {
                warn!("WebCodecs produced a frame with no pending destination; closing it");
                frame.close();
            }
        }) as Box<dyn FnMut(VideoFrame)>);

        let error_cb = Closure::wrap(Box::new(move |e: web_sys::DomException| {
            error!(message = %e.message(), "WebCodecs VideoDecoder error");
        }) as Box<dyn FnMut(web_sys::DomException)>);

        // VideoDecoderInit::new(error, output) — error first (verified against the web-sys binding).
        let init = VideoDecoderInit::new(error_cb.as_ref().unchecked_ref(), output_cb.as_ref().unchecked_ref());
        let decoder = VideoDecoder::new(&init).map_err(|e| anyhow!("VideoDecoder::new failed: {e:?}"))?;

        Ok(Self {
            decoder,
            _output_cb: output_cb,
            _error_cb: error_cb,
            pending,
            configured: false,
            timestamp: 0,
            annexb: Vec::new(),
            samples_logged: 0,
        })
    }

    /// Submit one AVC420 frame (AVC format: 4-byte BE length-prefixed NALs) destined for `out_dst`
    /// (output-space). Configures lazily on the first SPS; frames before that are dropped.
    pub(crate) fn decode(&mut self, surface_id: u16, out_dst: ExclusiveRectangle, bitstream: &[u8]) {
        let sampling = self.samples_logged < 4;
        let mut sps: Option<Vec<u8>> = None;
        let mut has_idr = false;
        let mut nal_types: Vec<u8> = Vec::new();
        for_each_nal(bitstream, |nal| {
            let t = nal_type(nal);
            if sampling {
                nal_types.push(t);
            }
            match t {
                NAL_SPS => sps = Some(nal.to_vec()),
                NAL_IDR => has_idr = true,
                _ => {}
            }
        });
        if sampling {
            // One-shot wire-format diagnostic: confirms Annex-B vs AVC framing and which NAL types
            // actually arrive (so we can see whether the server ever sends an SPS/IDR keyframe).
            let head: String = bitstream.iter().take(20).map(|b| format!("{b:02x} ")).collect();
            info!(len = bitstream.len(), annexb = is_annex_b(bitstream), ?nal_types, head = %head.trim_end(), "AVC420 bitstream sample");
            self.samples_logged += 1;
        }

        if !self.configured {
            let Some(sps) = sps.as_deref() else {
                warn!("AVC420 frame before SPS; waiting for a keyframe");
                return;
            };
            let codec = avc_codec_string(sps);
            let config = VideoDecoderConfig::new(&codec);
            config.set_optimize_for_latency(true);
            self.decoder.configure(&config);
            self.configured = true;
            // If you see this, H.264/AVC420 is actually being received and hardware-decoded.
            info!(%codec, "EGFX H.264 (WebCodecs) decoder configured — hardware video active");
        }

        to_annex_b(bitstream, &mut self.annexb);
        let data = js_sys::Uint8Array::from(self.annexb.as_slice());
        let chunk_type = if has_idr {
            EncodedVideoChunkType::Key
        } else {
            EncodedVideoChunkType::Delta
        };
        let init = EncodedVideoChunkInit::new_with_u8_array(&data, self.timestamp, chunk_type);
        self.timestamp = self.timestamp.wrapping_add(1);

        match EncodedVideoChunk::new(&init) {
            Ok(chunk) => {
                self.pending.borrow_mut().push_back((surface_id, out_dst));
                self.decoder.decode(&chunk);
            }
            Err(e) => error!(error = ?e, "failed to build EncodedVideoChunk"),
        }
    }

    /// Resets to the unconfigured state (on EGFX `ResetGraphics`); pending correlations are dropped.
    pub(crate) fn reset(&mut self) {
        self.decoder.reset();
        self.configured = false;
        self.pending.borrow_mut().clear();
    }
}

const NAL_IDR: u8 = 5;
const NAL_SPS: u8 = 7;

fn nal_type(nal: &[u8]) -> u8 {
    nal.first().map(|b| b & 0x1f).unwrap_or(0)
}

/// True if the buffer begins with an Annex-B start code (`00 00 01` or `00 00 00 01`).
fn is_annex_b(data: &[u8]) -> bool {
    data.starts_with(&[0, 0, 0, 1]) || data.starts_with(&[0, 0, 1])
}

/// Visits each NAL unit, auto-detecting framing: Annex-B (start codes) vs AVC (4-byte BE
/// length-prefixed). MS-RDPEGFX AVC420 framing is not reliably one or the other across servers, so
/// we detect rather than trust a fixed format.
fn for_each_nal(data: &[u8], f: impl FnMut(&[u8])) {
    if is_annex_b(data) {
        annexb_each_nal(data, f);
    } else {
        avc_each_nal(data, f);
    }
}

/// Visits each NAL of a 4-byte BE length-prefixed (AVC / `avcC`) buffer.
fn avc_each_nal(data: &[u8], mut f: impl FnMut(&[u8])) {
    let mut offset = 0;
    while offset + 4 <= data.len() {
        let len = u32::from_be_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]) as usize;
        offset += 4;
        let Some(end) = offset.checked_add(len) else {
            break;
        };
        if end > data.len() {
            break;
        }
        f(&data[offset..end]);
        offset = end;
    }
}

/// Visits each NAL of an Annex-B byte stream (NALs separated by `00 00 01` / `00 00 00 01`).
fn annexb_each_nal(data: &[u8], mut f: impl FnMut(&[u8])) {
    let n = data.len();
    let mut i = 0;
    // Advance to the first start code.
    while i + 3 <= n && !(data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1) {
        i += 1;
    }
    if i + 3 > n {
        return;
    }
    let mut nal_start = i + 3;
    i = nal_start;
    while i + 3 <= n {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            let mut end = i;
            // A 4-byte start code (`00 00 00 01`) leaves a trailing zero on the previous NAL.
            if end > nal_start && data[end - 1] == 0 {
                end -= 1;
            }
            if end > nal_start {
                f(&data[nal_start..end]);
            }
            nal_start = i + 3;
            i = nal_start;
        } else {
            i += 1;
        }
    }
    if nal_start < n {
        f(&data[nal_start..n]);
    }
}

/// Produces an Annex-B byte stream (`00 00 00 01` start codes) for `VideoDecoder` (configured without
/// an `avcC` description). Pass-through if already Annex-B, otherwise convert from length-prefixed.
fn to_annex_b(data: &[u8], out: &mut Vec<u8>) {
    out.clear();
    if is_annex_b(data) {
        out.extend_from_slice(data);
        return;
    }
    avc_each_nal(data, |nal| {
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
    });
}

/// `avc1.PPCCLL` codec string from the SPS NAL (`[1]`=profile, `[2]`=constraints, `[3]`=level).
fn avc_codec_string(sps: &[u8]) -> String {
    let profile = sps.get(1).copied().unwrap_or(0x42);
    let constraints = sps.get(2).copied().unwrap_or(0);
    let level = sps.get(3).copied().unwrap_or(0x1e);
    format!("avc1.{profile:02X}{constraints:02X}{level:02X}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annex_b_replaces_length_prefixes_with_start_codes() {
        // Two NALs: lengths 2 and 3.
        let avc = [0, 0, 0, 2, 0x67, 0x42, 0, 0, 0, 3, 0x65, 0x88, 0x99];
        let mut out = Vec::new();
        to_annex_b(&avc, &mut out);
        assert_eq!(out, [0, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x65, 0x88, 0x99]);
    }

    #[test]
    fn codec_string_from_sps() {
        // NAL header 0x67, profile 0x42 (baseline), constraints 0xE0, level 0x1F.
        assert_eq!(avc_codec_string(&[0x67, 0x42, 0xE0, 0x1F]), "avc1.42E01F");
    }

    #[test]
    fn truncated_nal_is_ignored() {
        let avc = [0, 0, 0, 9, 0x67, 0x42]; // claims 9 bytes, only 2 present
        let mut seen = 0;
        for_each_nal(&avc, |_| seen += 1);
        assert_eq!(seen, 0);
    }
}
