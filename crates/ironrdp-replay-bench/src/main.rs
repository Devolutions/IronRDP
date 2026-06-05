//! Deterministic replay benchmark + correctness gate for `IRDPREC1` captures.
//!
//! Replays a recorded active-session byte stream (produced by `ironrdp-viewer --record-traffic`)
//! through the real IronRDP decode pipeline with no live server, then compares the resulting
//! framebuffer CRC32 against the recorded ground truth. Each iteration builds a **fresh**
//! `ActiveStage` + `DecodedImage` and replays from the start of the capture (RDP codecs are
//! stateful, so reuse would desync). See `docs/plans/2026-06-03-ironrdp-benchmark-design.md`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as _, bail};
use clap::Parser;
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput};
use ironrdp_replay_core::{ChannelEntry, ReplayParams, build_connection_result, framebuffer_crc32, parse_compression};
use ironrdp_tokio::TokioFramed;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[derive(Parser, Debug)]
#[command(about = "Replay an IRDPREC1 capture and verify the framebuffer CRC32")]
struct Args {
    /// Path to the `.irdprec` capture (manifest/checksum sidecars are derived from it).
    #[arg(long)]
    input: PathBuf,

    /// Measured iterations (each builds a fresh session and replays from the start).
    #[arg(long, default_value_t = 1)]
    iterations: u32,

    /// Warmup iterations, discarded from timing.
    #[arg(long, default_value_t = 1)]
    warmup: u32,

    /// Write the result JSON to this path.
    #[arg(long)]
    json: Option<PathBuf>,
}

/// Mirror of `ironrdp_client::record::ChannelManifest`.
#[derive(Debug, Deserialize)]
struct ChannelManifest {
    name: String,
    id: u16,
}

/// Mirror of `ironrdp_client::record::SessionManifest` (deserialize side).
#[derive(Debug, Deserialize)]
struct SessionManifest {
    desktop_width: u16,
    desktop_height: u16,
    io_channel_id: u16,
    user_channel_id: u16,
    share_id: u32,
    compression_type: Option<String>,
    enable_server_pointer: bool,
    pointer_software_rendering: bool,
    #[serde(default)]
    channels: Vec<ChannelManifest>,
}

impl SessionManifest {
    fn to_replay_params(&self) -> ReplayParams {
        ReplayParams {
            io_channel_id: self.io_channel_id,
            user_channel_id: self.user_channel_id,
            share_id: self.share_id,
            desktop_width: self.desktop_width,
            desktop_height: self.desktop_height,
            enable_server_pointer: self.enable_server_pointer,
            pointer_software_rendering: self.pointer_software_rendering,
            compression_type: parse_compression(self.compression_type.as_deref()),
            channels: self
                .channels
                .iter()
                .map(|c| ChannelEntry {
                    name: c.name.clone(),
                    id: c.id,
                })
                .collect(),
        }
    }
}

/// Mirror of the recorded `*.checksum.json` ground truth.
#[derive(Debug, Deserialize)]
struct ChecksumFile {
    crc32: String,
}

/// In-memory replay transport: serves the recorded capture bytes to the reader; discards writes.
struct ReplayStream<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ReplayStream<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
}

impl AsyncRead for ReplayStream<'_> {
    fn poll_read(mut self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let remaining = &self.data[self.pos..];
        let n = remaining.len().min(buf.remaining());
        // n == 0 leaves the buffer untouched, which AsyncRead treats as EOF.
        buf.put_slice(&remaining[..n]);
        self.pos += n;
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for ReplayStream<'_> {
    fn poll_write(self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// One replay pass: fresh session, replay the whole capture, return (decode wall-time, framebuffer
/// CRC32, decoded PDU count).
async fn run_pass(capture: &[u8], params: &ReplayParams) -> anyhow::Result<(Duration, u32, u64)> {
    let connection_result = build_connection_result(params);
    let mut image = DecodedImage::new(
        PixelFormat::RgbA32,
        connection_result.desktop_size.width,
        connection_result.desktop_size.height,
    );
    let mut active_stage = ActiveStage::new(connection_result);
    let mut framed = TokioFramed::new(ReplayStream::new(capture));

    let mut pdus = 0u64;
    let start = Instant::now();
    loop {
        let (action, payload) = match framed.read_pdu().await {
            Ok(frame) => frame,
            // Clean end of capture.
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e).context("read frame from capture"),
        };

        let outputs = active_stage
            .process(&mut image, action, &payload)
            .context("decode frame")?;
        pdus += 1;

        // No server on replay: response frames are dropped. Stop on a graceful terminate.
        if outputs.iter().any(|o| matches!(o, ActiveStageOutput::Terminate(_))) {
            break;
        }
    }
    let elapsed = start.elapsed();
    let crc = framebuffer_crc32(image.data());
    Ok((elapsed, crc, pdus))
}

#[derive(Debug, Serialize)]
struct ResultJson {
    schema_version: u32,
    capture: String,
    frontend: &'static str,
    iterations: u32,
    warmup: u32,
    pdus: u64,
    decode_ms_min: f64,
    decode_ms_median: f64,
    decode_ms_max: f64,
    canonical_checksum: String,
    expected_checksum: String,
    matches_ground_truth: bool,
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

fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> anyhow::Result<T> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    let manifest_path = args.input.with_extension("json");
    let checksum_path = args.input.with_extension("checksum.json");

    let capture = std::fs::read(&args.input).with_context(|| format!("read capture {}", args.input.display()))?;
    let manifest: SessionManifest = load_json(&manifest_path)?;
    let checksum: ChecksumFile = load_json(&checksum_path)?;
    let params = manifest.to_replay_params();

    println!(
        "Replaying {} ({} bytes, {}x{}) — {} warmup + {} measured iterations",
        args.input.display(),
        capture.len(),
        manifest.desktop_width,
        manifest.desktop_height,
        args.warmup,
        args.iterations,
    );

    for _ in 0..args.warmup {
        run_pass(&capture, &params).await?;
    }

    let mut timings_ms = Vec::with_capacity(args.iterations as usize);
    let mut last_crc = 0u32;
    let mut pdus = 0u64;
    for _ in 0..args.iterations.max(1) {
        let (elapsed, crc, n) = run_pass(&capture, &params).await?;
        timings_ms.push(elapsed.as_secs_f64() * 1000.0);
        last_crc = crc;
        pdus = n;
    }

    let canonical_checksum = format!("{last_crc:08x}");
    let matches = canonical_checksum == checksum.crc32;

    let min = timings_ms.iter().copied().fold(f64::INFINITY, f64::min);
    let max = timings_ms.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let med = median(timings_ms);

    println!(
        "PDUs: {pdus} | decode ms: min={min:.3} median={med:.3} max={max:.3}\n\
         checksum: replay={canonical_checksum} expected={} -> {}",
        checksum.crc32,
        if matches { "MATCH" } else { "MISMATCH" },
    );

    if let Some(json_path) = &args.json {
        let result = ResultJson {
            schema_version: 1,
            capture: args.input.display().to_string(),
            frontend: "native",
            iterations: args.iterations,
            warmup: args.warmup,
            pdus,
            decode_ms_min: min,
            decode_ms_median: med,
            decode_ms_max: max,
            canonical_checksum: canonical_checksum.clone(),
            expected_checksum: checksum.crc32.clone(),
            matches_ground_truth: matches,
        };
        std::fs::write(json_path, serde_json::to_string_pretty(&result)?)
            .with_context(|| format!("write {}", json_path.display()))?;
        println!("Wrote {}", json_path.display());
    }

    if !matches {
        bail!("checksum mismatch: replay={canonical_checksum} expected={}", checksum.crc32);
    }

    Ok(())
}
