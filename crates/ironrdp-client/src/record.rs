//! Benchmark capture recording (`--record-*` flags).
//!
//! Records the decrypted server->client RDP byte stream of a live session into an `IRDPREC1`
//! capture so it can later be replayed deterministically through the decode/render pipeline
//! (native, WASM, or .NET) without a live server. See
//! `docs/plans/2026-06-03-ironrdp-benchmark-design.md`.
//!
//! Three sidecar artifacts are produced:
//! * `<name>.irdprec`      — the decrypted server->client **active-session** byte stream (everything
//!   after the connection sequence completes). A replay feeds this into a freshly built `ActiveStage`
//!   reconstructed from the manifest.
//! * `<name>.json` — negotiated session state (channel IDs, desktop size, compression, …)
//!   needed to rebuild the `ActiveStage` on replay.
//! * `<name>.checksum.json` — CRC32 over the final framebuffer, the deterministic correctness gate.
//!
//! Recording stays gated off through the entire connection sequence (CredSSP, MCS, capability
//! exchange) and is flipped on via [`RecordGate`] right after `connect_finalize` returns, so the
//! capture contains exactly the active-session bytes (plus any already buffered at that boundary).

use core::pin::Pin;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::task::{Context, Poll};
use std::fs::File;
use std::io::{Error, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;

use ironrdp::connector::ConnectionResult;
use ironrdp::session::image::DecodedImage;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc::{self, UnboundedSender};
use tracing::{error, info, warn};

/// Queued-but-unwritten byte count past which we warn once that the recorder is falling behind.
/// The capture channel is unbounded, so a slow sink can grow it without limit; this makes that
/// observable before it becomes an out-of-memory condition.
const RECORD_QUEUE_HIGH_WATER_BYTES: usize = 256 * 1024 * 1024;

/// Shared switch that gates whether [`RecordingStream`] tees the bytes it reads.
///
/// Created off; [`RecordGate::open`] is called once `connect_finalize` returns so the capture starts
/// exactly at the active-session byte stream.
#[derive(Clone, Debug)]
pub(crate) struct RecordGate(Arc<AtomicBool>);

impl RecordGate {
    pub(crate) fn new_closed() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    /// Start capturing. Idempotent.
    pub(crate) fn open(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn is_open(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Wraps a transport stream and tees every server->client byte it reads into a capture file once
/// [`RecordGate`] is open. Writes pass through untouched. Modeled on the IronVNC `RecordTraffic`
/// recorder (background writer thread + unbounded channel) so file I/O never blocks the runtime.
pub(crate) struct RecordingStream<S> {
    stream: S,
    gate: RecordGate,
    // `Option` so `Drop` can close the channel (drop the sender) *before* joining the writer thread,
    // which is what lets the writer's `blocking_recv` loop terminate.
    sender: Option<UnboundedSender<Vec<u8>>>,
    writer_thread: Option<JoinHandle<()>>,
    // Total bytes captured so far (post-gate). Shared so the manifest can read it at teardown.
    recorded_bytes: Arc<AtomicUsize>,
    // Bytes queued for writing but not yet flushed, backing the high-water warning.
    queued_bytes: Arc<AtomicUsize>,
    high_water_warned: bool,
}

impl<S> RecordingStream<S> {
    /// Creates the capture file and spawns its background writer. The only fallible step is creating
    /// the file, so this returns an I/O error (which the caller can wrap with `custom_err!`).
    pub(crate) fn new(output_path: &Path, stream: S, gate: RecordGate) -> Result<Self, Error> {
        let mut output_file = File::create(output_path)?;
        let output_path_display = output_path.display().to_string();

        // Unbounded on purpose: `poll_read` runs on the async runtime and cannot block to apply
        // backpressure; lossless capture is preferred. `queued_bytes` + the high-water warning make
        // a runaway backlog observable, and the timed join on `Drop` bounds the lifetime.
        let (sender, mut receiver) = mpsc::unbounded_channel::<Vec<u8>>();
        let queued_bytes = Arc::new(AtomicUsize::new(0));
        let writer_queued_bytes = Arc::clone(&queued_bytes);

        let writer_thread = std::thread::spawn(move || {
            let mut write_error = false;
            while let Some(data) = receiver.blocking_recv() {
                let result = output_file.write_all(&data);
                writer_queued_bytes.fetch_sub(data.len(), Ordering::Relaxed);
                if let Err(e) = result {
                    error!(error = %e, "Error writing to the capture file");
                    write_error = true;
                    break;
                }
            }

            if let Err(e) = output_file.flush() {
                error!(error = %e, "Error flushing the capture file");
                write_error = true;
            }

            if write_error {
                error!(file = %output_path_display, "Capture may be incomplete due to a write error");
            } else {
                info!(file = %output_path_display, "Capture saved");
            }
        });

        Ok(Self {
            stream,
            gate,
            sender: Some(sender),
            writer_thread: Some(writer_thread),
            recorded_bytes: Arc::new(AtomicUsize::new(0)),
            queued_bytes,
            high_water_warned: false,
        })
    }

    /// Handle to the running total of captured bytes, readable at any time (e.g. for the manifest).
    pub(crate) fn recorded_bytes_handle(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.recorded_bytes)
    }

    /// Handle for writing bytes into this capture out-of-band, FIFO-ordered with the stream's own
    /// gated reads. Used to prepend the bytes already buffered at the CredSSP -> MCS boundary.
    pub(crate) fn capture_handle(&self) -> CaptureHandle {
        CaptureHandle {
            sender: self.sender.clone(),
            recorded_bytes: Arc::clone(&self.recorded_bytes),
        }
    }

    fn tee(&mut self, block: Vec<u8>) {
        let block_len = block.len();
        let Some(sender) = self.sender.as_ref() else {
            return;
        };

        // Account before handing off: the writer decrements only after receiving a block, so adding
        // first guarantees the add happens-before the matching sub (no unsigned underflow).
        self.queued_bytes.fetch_add(block_len, Ordering::Relaxed);
        if sender.send(block).is_err() {
            self.queued_bytes.fetch_sub(block_len, Ordering::Relaxed);
            error!("Error sending captured data to the writer thread");
            return;
        }
        self.recorded_bytes.fetch_add(block_len, Ordering::Relaxed);

        let queued = self.queued_bytes.load(Ordering::Relaxed);
        if queued > RECORD_QUEUE_HIGH_WATER_BYTES && !self.high_water_warned {
            self.high_water_warned = true;
            warn!(
                queued_bytes = queued,
                high_water_bytes = RECORD_QUEUE_HIGH_WATER_BYTES,
                "Capture writer is falling behind; the unwritten backlog is growing (slow sink?)"
            );
        }
    }
}

/// Out-of-band writer into a [`RecordingStream`]'s capture, FIFO-ordered with the stream's reads.
#[derive(Clone)]
pub(crate) struct CaptureHandle {
    sender: Option<UnboundedSender<Vec<u8>>>,
    recorded_bytes: Arc<AtomicUsize>,
}

impl CaptureHandle {
    /// Append `bytes` to the capture. No-op if empty.
    pub(crate) fn write(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if let Some(sender) = self.sender.as_ref()
            && sender.send(bytes.to_vec()).is_ok()
        {
            self.recorded_bytes.fetch_add(bytes.len(), Ordering::Relaxed);
        }
    }
}

impl<S> Drop for RecordingStream<S> {
    fn drop(&mut self) {
        // Close the channel first so the writer's `blocking_recv` returns `None` and its loop ends;
        // then join. Order matters: joining before dropping the sender would deadlock.
        self.sender = None;

        if let Some(handle) = self.writer_thread.take()
            && handle.join().is_err()
        {
            error!("Capture writer thread panicked; capture may be incomplete");
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for RecordingStream<S> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<Result<(), Error>> {
        let buf_filled_before = buf.filled().len();
        let this = self.get_mut();

        match Pin::new(&mut this.stream).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                let read = buf.filled().len() - buf_filled_before;
                if read != 0 && this.gate.is_open() {
                    let block = buf.filled()[buf_filled_before..][..read].to_vec();
                    this.tee(block);
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for RecordingStream<S> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, Error>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

/// Paths for the three `IRDPREC1` sidecar artifacts, derived from a single `--record-traffic` path.
#[derive(Clone, Debug)]
pub struct RecordOptions {
    pub traffic_path: PathBuf,
    pub manifest_path: PathBuf,
    pub checksum_path: PathBuf,
}

impl RecordOptions {
    /// Derive the manifest/checksum sidecar paths from the traffic path:
    /// `capture.irdprec` -> `capture.json` / `capture.checksum.json`.
    pub fn from_traffic_path(traffic_path: PathBuf) -> Self {
        let manifest_path = traffic_path.with_extension("json");
        let checksum_path = traffic_path.with_extension("checksum.json");
        Self {
            traffic_path,
            manifest_path,
            checksum_path,
        }
    }
}

/// Recording state carried from connection setup into the active session, so the manifest and
/// checksum can be finalized at teardown (once the byte count and final framebuffer are known).
pub struct RecordContext {
    pub options: RecordOptions,
    pub manifest: SessionManifest,
    pub recorded_bytes: Arc<AtomicUsize>,
}

impl RecordContext {
    /// Stamp the manifest with the captured byte count and write the manifest + framebuffer checksum.
    pub fn finalize(mut self, image: &DecodedImage) -> anyhow::Result<()> {
        let bytes = self.recorded_bytes.load(Ordering::SeqCst);
        self.manifest.finalize(bytes);
        self.manifest.write(&self.options.manifest_path)?;
        write_checksum(&self.options.checksum_path, image)?;
        Ok(())
    }
}

const IRDPREC_FORMAT: &str = "IRDPREC1";
const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// A negotiated static virtual channel: its RDP name and the channel ID the server assigned. Recorded
/// so a replay can rebuild the same channel set with matching IDs (the x224 processor routes
/// slow-path/DVC PDUs by channel ID and rejects unknown ones).
#[derive(Debug, Serialize)]
pub struct ChannelManifest {
    pub name: String,
    pub id: u16,
}

/// `IRDPREC1` session manifest — the negotiated state a replay needs to rebuild the session.
///
/// `ConnectionResult` and `connector::Config` do not derive `Serialize`, so this is a flat mirror of
/// the fields a replay requires.
#[derive(Debug, Serialize)]
pub struct SessionManifest {
    pub schema_version: u32,
    pub format: &'static str,
    pub desktop_width: u16,
    pub desktop_height: u16,
    pub io_channel_id: u16,
    pub user_channel_id: u16,
    pub share_id: u32,
    pub compression_type: Option<String>,
    pub enable_server_pointer: bool,
    pub pointer_software_rendering: bool,
    /// Negotiated static virtual channels (name + assigned ID), in `StaticChannelSet` order.
    pub channels: Vec<ChannelManifest>,
    /// Captured server->client active-session bytes. Filled in at teardown.
    pub stream_bytes: usize,
}

impl SessionManifest {
    /// Snapshot the negotiated state from a live [`ConnectionResult`]. `stream_bytes` is filled in
    /// later via [`SessionManifest::finalize`] once the capture is complete.
    pub fn from_connection_result(result: &ConnectionResult) -> Self {
        let channels = result
            .static_channels
            .iter()
            .filter_map(|(type_id, svc)| {
                let id = result.static_channels.get_channel_id_by_type_id(type_id)?;
                let name = svc.channel_name().as_str()?.to_owned();
                Some(ChannelManifest { name, id })
            })
            .collect();

        Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            format: IRDPREC_FORMAT,
            desktop_width: result.desktop_size.width,
            desktop_height: result.desktop_size.height,
            io_channel_id: result.io_channel_id,
            user_channel_id: result.user_channel_id,
            share_id: result.share_id,
            compression_type: result.compression_type.map(|ct| format!("{ct:?}")),
            enable_server_pointer: result.enable_server_pointer,
            pointer_software_rendering: result.pointer_software_rendering,
            channels,
            stream_bytes: 0,
        }
    }

    pub fn finalize(&mut self, stream_bytes: usize) {
        self.stream_bytes = stream_bytes;
    }

    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        info!(path = %path.display(), "Wrote session manifest");
        Ok(())
    }
}

/// `IRDPREC1` framebuffer checksum — the deterministic correctness gate. Just the CRC32; the
/// framebuffer dimensions and pixel format live in the manifest.
#[derive(Debug, Serialize)]
struct ChecksumManifest {
    /// CRC32 over the canonical framebuffer bytes, 8 lowercase hex digits.
    crc32: String,
}

/// CRC32 over the canonical framebuffer: the `DecodedImage` RGBA buffer with the alpha byte of every
/// pixel masked to `0xFF`, so captures stay comparable across codec paths (most decoders force
/// opaque alpha, but the QOI path copies source alpha).
pub fn framebuffer_crc32(image: &DecodedImage) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    let data = image.data();
    // RgbA32: 4 bytes/pixel, [r, g, b, a]. Hash R,G,B and a canonical 0xFF alpha per pixel.
    let mut pixel = [0u8; 4];
    for chunk in data.chunks_exact(4) {
        pixel[0] = chunk[0];
        pixel[1] = chunk[1];
        pixel[2] = chunk[2];
        pixel[3] = 0xFF;
        hasher.update(&pixel);
    }
    hasher.finalize()
}

/// Computes the canonical framebuffer CRC32 and writes it to `path`.
pub fn write_checksum(path: &Path, image: &DecodedImage) -> anyhow::Result<u32> {
    let crc = framebuffer_crc32(image);
    let manifest = ChecksumManifest {
        crc32: format!("{crc:08x}"),
    };
    let json = serde_json::to_string_pretty(&manifest)?;
    std::fs::write(path, json)?;
    info!(path = %path.display(), crc32 = %format!("{crc:08x}"), "Wrote framebuffer checksum");
    Ok(crc)
}
