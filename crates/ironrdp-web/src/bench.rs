//! WASM replay benchmark + correctness gate (feature `bench`).
//!
//! Replays a recorded `IRDPREC1` active-session capture through the **real** IronRDP web render
//! pipeline (decode -> dirty-region extract -> canvas draw) in a headless browser, instruments each
//! stage with `performance.now()`, and verifies the framebuffer CRC32 against the recorded ground
//! truth. The session is rebuilt from the manifest via the shared `ironrdp-replay-core` crate, so
//! this reproduces the exact framebuffer the native and .NET replays do. Mirrors the IronVNC
//! `crates/ironvnc-web/src/bench.rs`. See `docs/plans/2026-06-03-ironrdp-benchmark-design.md`.

use core::num::NonZeroU32;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::io;

use anyhow::Context as _;
use futures_util::io::{AsyncRead, AsyncWrite};
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput};
use ironrdp_futures::LocalFuturesFramed;
use ironrdp_replay_core::{ChannelEntry, ReplayParams, build_connection_result, framebuffer_crc32, parse_compression};
use wasm_bindgen::prelude::*;
use web_sys::{HtmlCanvasElement, Performance};

use crate::canvas::Canvas;

/// In-memory replay transport: serves the recorded capture bytes; discards writes.
struct ReplayStream<'a> {
    data: &'a [u8],
    pos: usize,
}

impl AsyncRead for ReplayStream<'_> {
    fn poll_read(mut self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        let remaining = &self.data[self.pos..];
        let n = remaining.len().min(buf.len());
        buf[..n].copy_from_slice(&remaining[..n]);
        self.pos += n;
        Poll::Ready(Ok(n))
    }
}

impl AsyncWrite for ReplayStream<'_> {
    fn poll_write(self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Per-stage accumulators (milliseconds) for one replay pass.
#[derive(Default, Clone)]
struct Accum {
    read_ms: f64,
    decode_ms: f64,
    extract_ms: f64,
    draw_ms: f64,
    frames: u64,
    rects: u64,
}

struct PassResult {
    acc: Accum,
    total_ms: f64,
    crc: u32,
}

#[expect(clippy::too_many_arguments, reason = "flat WASM entry point mirroring the manifest")]
#[expect(unreachable_pub, reason = "exported to JS via wasm-bindgen")]
#[wasm_bindgen]
pub async fn run_web_bench(
    canvas: HtmlCanvasElement,
    capture: Vec<u8>,
    io_channel_id: u16,
    user_channel_id: u16,
    share_id: u32,
    desktop_width: u16,
    desktop_height: u16,
    enable_server_pointer: bool,
    pointer_software_rendering: bool,
    compression: String,
    channel_names: Vec<String>,
    channel_ids: Vec<u16>,
    passes: u32,
) -> Result<String, JsValue> {
    let params = ReplayParams {
        io_channel_id,
        user_channel_id,
        share_id,
        desktop_width,
        desktop_height,
        enable_server_pointer,
        pointer_software_rendering,
        compression_type: parse_compression(Some(&compression)),
        channels: channel_names
            .into_iter()
            .zip(channel_ids)
            .map(|(name, id)| ChannelEntry { name, id })
            .collect(),
    };

    run(&canvas, &capture, &params, passes.max(1))
        .await
        .map_err(|err| JsValue::from_str(&format!("{err:#}")))
}

async fn run(canvas: &HtmlCanvasElement, capture: &[u8], params: &ReplayParams, passes: u32) -> anyhow::Result<String> {
    let perf = web_sys::window()
        .context("no window")?
        .performance()
        .context("no performance")?;

    // Warmup (discarded) — warm wasm tier-up / caches.
    let _ = run_pass(canvas, capture, params, &perf).await?;

    let mut measured: Vec<PassResult> = Vec::new();
    for _ in 0..passes {
        measured.push(run_pass(canvas, capture, params, &perf).await?);
    }

    let crc = measured.last().map(|p| p.crc).unwrap_or(0);
    let total = median(measured.iter().map(|p| p.total_ms).collect());
    let read = median(measured.iter().map(|p| p.acc.read_ms).collect());
    let decode = median(measured.iter().map(|p| p.acc.decode_ms).collect());
    let extract = median(measured.iter().map(|p| p.acc.extract_ms).collect());
    let draw = median(measured.iter().map(|p| p.acc.draw_ms).collect());
    let p0 = &measured[0];

    let json = format!(
        concat!(
            "{{\"schemaVersion\":1,\"frontend\":\"wasm\",",
            "\"passes\":{{\"warmup\":1,\"measured\":{}}},",
            "\"framebuffer\":{{\"width\":{},\"height\":{}}},",
            "\"counts\":{{\"frames\":{},\"rects\":{},\"captureBytes\":{}}},",
            "\"totalMs\":{:.4},",
            "\"stageMs\":{{\"readPdu\":{:.4},\"decode\":{:.4},\"extract\":{:.4},\"draw\":{:.4}}},",
            "\"canonicalChecksum\":\"{:08x}\"}}"
        ),
        passes,
        params.desktop_width,
        params.desktop_height,
        p0.acc.frames,
        p0.acc.rects,
        capture.len(),
        total,
        read,
        decode,
        extract,
        draw,
        crc,
    );

    Ok(json)
}

async fn run_pass(
    canvas: &HtmlCanvasElement,
    capture: &[u8],
    params: &ReplayParams,
    perf: &Performance,
) -> anyhow::Result<PassResult> {
    let connection_result = build_connection_result(params);
    let mut image = DecodedImage::new(PixelFormat::RgbA32, params.desktop_width, params.desktop_height);
    let mut active_stage = ActiveStage::new(connection_result);

    let mut gui = Canvas::new(
        canvas.clone(),
        NonZeroU32::new(u32::from(params.desktop_width)).context("zero width")?,
        NonZeroU32::new(u32::from(params.desktop_height)).context("zero height")?,
    )
    .await?;

    let mut framed = LocalFuturesFramed::new(ReplayStream { data: capture, pos: 0 });
    let mut acc = Accum::default();

    let t_start = perf.now();
    loop {
        let t_read = perf.now();
        let frame = framed.read_pdu().await;
        acc.read_ms += perf.now() - t_read;

        let (action, payload) = match frame {
            Ok(frame) => frame,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(anyhow::Error::new(e).context("read frame from capture")),
        };
        acc.frames += 1;

        let t_dec = perf.now();
        let outputs = active_stage.process(&mut image, action, &payload).context("decode frame")?;
        acc.decode_ms += perf.now() - t_dec;

        let mut terminate = false;
        for output in outputs {
            match output {
                ActiveStageOutput::GraphicsUpdate(region) => {
                    // The GPU presenter reads the framebuffer in place (no extraction step);
                    // extract_ms stays 0 and any fallback extraction is accounted under draw_ms.
                    let t_draw = perf.now();
                    gui.draw(&image, region).context("draw region")?;
                    acc.draw_ms += perf.now() - t_draw;
                    acc.rects += 1;
                }
                ActiveStageOutput::Terminate(_) => terminate = true,
                // No server on replay: response frames and pointer/other events are dropped.
                _ => {}
            }
        }
        if terminate {
            break;
        }
    }
    let total_ms = perf.now() - t_start;
    let crc = framebuffer_crc32(image.data());

    Ok(PassResult { acc, total_ms, crc })
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let n = v.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}
