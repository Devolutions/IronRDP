//! The browser-independent part of the WebCodecs H.264 backend: which chunks reach the
//! `VideoDecoder` and when, how a flush is recovered from, which configurations are known not to
//! work, the pictures awaiting the graphics pipeline, and the memory they hold.
//!
//! The `VideoDecoder` sits behind [`Sink`], so all of this runs and is tested without a browser.

use core::cell::Cell;
use core::time::Duration;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;

use ironrdp_core::MonotonicInstant;
use ironrdp_egfx::annex_b::{AccessUnit, ParameterSets};
use ironrdp_egfx::decode::{DecodedPicture, DecoderError, DecoderResult, Yuv420Frame};
use tracing::warn;

/// Upper bound on one access unit; a 4K IDR picture is well below.
pub(crate) const MAX_ACCESS_UNIT_BYTES: usize = 4 << 20;

/// Upper bound on the access units kept since the last IDR picture, to decode them again after a flush.
const MAX_HISTORY_UNITS: usize = 300;

/// Upper bound on the bytes the kept access units hold, counting [`HISTORY_ENTRY_OVERHEAD`] each.
const MAX_HISTORY_BYTES: usize = 16 << 20;

/// What keeping one access unit costs besides its bytes: its reference count and allocation header.
const HISTORY_ENTRY_OVERHEAD: usize = 64;

/// Upper bound on the chunks given to the `VideoDecoder` and not output yet, replayed ones included.
const MAX_OUTSTANDING_CHUNKS: u32 = 8;

/// Upper bound on the chunks given to the `VideoDecoder` in one call, so a replay spreads over turns.
const MAX_CHUNKS_PER_TURN: u32 = 4;

/// How long a replay may make no progress while the `VideoDecoder` is full before it is given up.
/// A replay that keeps progressing slowly is bounded by the graphics pipeline's own deadline instead.
const REPLAY_STALL: Duration = Duration::from_secs(1);

/// Upper bound on the pictures submitted and not polled yet.
pub(crate) const MAX_PICTURES_IN_FLIGHT: u64 = 32;

/// Upper bound on the configurations remembered as unsupported.
const MAX_UNSUPPORTED_CONFIGS: usize = 16;

/// A `VideoDecoder` configuration, which identifies what a support decision applies to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Config {
    pub(crate) codec: String,
    /// `codedWidth` and `codedHeight`, when known from the stream.
    pub(crate) size: Option<(u32, u32)>,
}

/// Why a `VideoDecoder` stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The configuration is not supported (`NotSupportedError`).
    Unsupported,
    /// The browser ran out of a resource (`QuotaExceededError`).
    Resource,
    /// The data could not be decoded (`EncodingError`, `DataError` and the rest).
    Corrupt,
}

impl Failure {
    /// The failure a `DOMException` name reports.
    pub(crate) fn from_name(name: &str) -> Self {
        match name {
            "NotSupportedError" => Self::Unsupported,
            "QuotaExceededError" => Self::Resource,
            _ => Self::Corrupt,
        }
    }
}

/// The `VideoDecoder` operations the stream drives.
pub(crate) trait Sink {
    /// Creates and configures a new `VideoDecoder`, closing any previous one.
    fn open(&mut self, config: &Config) -> Result<(), Failure>;
    /// Gives a chunk to the `VideoDecoder`.
    fn decode(&mut self, chunk: &[u8], key: bool, timestamp: f64) -> Result<(), Failure>;
    /// Asks the `VideoDecoder` to output everything it holds.
    fn flush(&mut self) -> Result<(), Failure>;
    /// Closes the `VideoDecoder`.
    fn close(&mut self);
}

/// A chunk waiting for room in the `VideoDecoder`.
#[derive(Debug, Clone)]
struct Queued {
    data: Rc<[u8]>,
    key: bool,
    /// The picture number of a live chunk, or `None` for a replayed one, whose picture is discarded.
    number: Option<u64>,
}

/// A replay running ahead of its live chunk.
#[derive(Debug, Clone, Copy)]
struct Replay {
    /// The output count when progress was last seen.
    outputs: u64,
    /// When the `VideoDecoder` last took a chunk or output a picture during the replay.
    progress_at: MonotonicInstant,
    /// A flush was asked for during the replay, which it would start over; it is made once the
    /// live chunk is given.
    flush_deferred: bool,
}

/// The access units given to the `VideoDecoder` since the last IDR picture given, that one first and self-contained.
#[derive(Debug, Default)]
struct History {
    units: Vec<Rc<[u8]>>,
    /// INVARIANT: the sum of each unit's length and [`HISTORY_ENTRY_OVERHEAD`].
    bytes: usize,
}

impl History {
    fn starting_with(key: Rc<[u8]>) -> Option<Self> {
        let mut history = Self::default();
        history.push(key).then_some(history)
    }

    /// Keeps `unit`, or returns `false` when that would pass a bound.
    fn push(&mut self, unit: Rc<[u8]>) -> bool {
        let bytes = self.bytes + unit.len() + HISTORY_ENTRY_OVERHEAD;
        if !(self.units.len() < MAX_HISTORY_UNITS && bytes <= MAX_HISTORY_BYTES) {
            return false;
        }
        self.bytes = bytes;
        self.units.push(unit);
        true
    }
}

/// The chunk side of one decoder: what was given to the `VideoDecoder` and what still waits.
pub(crate) struct Stream {
    parameter_sets: ParameterSets,
    /// The configuration of the open `VideoDecoder`, `None` until an IDR picture opens one.
    open: Option<Config>,
    /// The `VideoDecoder` failed or was flushed, so its next chunk must be a key chunk.
    key_required: bool,
    /// `None` once a bound was passed, until the next IDR picture is given.
    history: Option<History>,
    queue: VecDeque<Queued>,
    /// INVARIANT: chunks given to the `VideoDecoder` since it opened, minus the outputs it returned.
    outstanding: u32,
    /// Outputs the `VideoDecoder`s returned, live or replayed.
    outputs: u64,
    replay: Option<Replay>,
    /// The configuration whose last replay stalled, until a replay completes.
    stalled: Option<Config>,
    probed_codec: String,
    unsupported: BTreeSet<Config>,
    /// Bumped whenever the `VideoDecoder` is closed or replaced, which loses what it was given.
    decoder_generation: u64,
    /// Whether the sequence parameter set of each IDR picture declares full-range samples, by the
    /// number of that picture, which is the first one it applies to.
    /// INVARIANT: holds no entry older than the one for the oldest picture still in flight.
    full_range: BTreeMap<u64, bool>,
}

impl Stream {
    /// A stream whose decoders fall back on `probed_codec`, the codec string the support probe
    /// accepted, when the stream's own configuration fails.
    pub(crate) fn new(probed_codec: &str) -> Self {
        Self {
            parameter_sets: ParameterSets::default(),
            open: None,
            key_required: false,
            history: None,
            queue: VecDeque::new(),
            outstanding: 0,
            outputs: 0,
            replay: None,
            stalled: None,
            probed_codec: probed_codec.to_owned(),
            unsupported: BTreeSet::new(),
            decoder_generation: 0,
            full_range: BTreeMap::new(),
        }
    }

    /// Queues the Annex B access unit `data` as picture `number`, opening a `VideoDecoder` on an IDR
    /// picture when none is open or the stream's configuration changed.
    /// Returns the pictures still queued for a `VideoDecoder` this replaced, which will never come out.
    pub(crate) fn submit(&mut self, sink: &mut dyn Sink, data: &[u8], number: u64) -> DecoderResult<Vec<u64>> {
        if MAX_ACCESS_UNIT_BYTES < data.len() {
            return Err(DecoderError::msg("an H.264 access unit past 4 MiB"));
        }
        let unit = AccessUnit::parse(data);
        if unit.malformed {
            return Err(DecoderError::msg("a malformed H.264 access unit"));
        }
        self.parameter_sets.update(&unit);
        if unit.is_idr() {
            let key = self
                .parameter_sets
                .key_access_unit(data)
                .ok_or_else(|| DecoderError::msg("an IDR picture without its parameter sets"))?;
            if MAX_ACCESS_UNIT_BYTES < key.data.len() {
                return Err(DecoderError::msg("an H.264 access unit past 4 MiB"));
            }
            let config = Config {
                codec: key.sps.codec_string(),
                size: Some((key.sps.width, key.sps.height)),
            };
            let mut lost = Vec::new();
            if self.open.is_none() || self.open.as_ref() != Some(&config) {
                lost = self.queue.drain(..).filter_map(|chunk| chunk.number).collect();
                self.reopen(sink, config)?;
            }
            self.full_range.insert(number, key.sps.full_range == Some(true));
            // Pictures this far back were polled already, so no output needs their entry any more.
            let oldest_in_flight = number.saturating_sub(MAX_PICTURES_IN_FLIGHT);
            if let Some(floor) = self.full_range.range(..=oldest_in_flight).next_back().map(|(&n, _)| n) {
                self.full_range = self.full_range.split_off(&floor);
            }
            self.queue.push_back(Queued {
                data: key.data.into(),
                key: true,
                number: Some(number),
            });
            Ok(lost)
        } else {
            if self.open.is_none() {
                return Err(DecoderError::msg("waiting for an IDR picture"));
            }
            let data: Rc<[u8]> = data.into();
            self.queue.push_back(Queued {
                data,
                key: false,
                number: Some(number),
            });
            Ok(Vec::new())
        }
    }

    /// Opens a `VideoDecoder` for `stream_config`, or for the probed codec when the stream's
    /// configuration is known not to work.
    fn reopen(&mut self, sink: &mut dyn Sink, stream_config: Config) -> DecoderResult<()> {
        self.decoder_generation += 1;
        self.queue.clear();
        self.outstanding = 0;
        self.key_required = false;
        self.history = None;
        self.replay = None;
        let fallback = Config {
            codec: self.probed_codec.clone(),
            size: None,
        };
        for config in [stream_config, fallback] {
            if self.unsupported.contains(&config) {
                continue;
            }
            match sink.open(&config) {
                Ok(()) => {
                    self.open = Some(config);
                    return Ok(());
                }
                Err(Failure::Unsupported) => self.remember_unsupported(config),
                Err(_) => break,
            }
        }
        self.open = None;
        Err(DecoderError::msg("the browser cannot decode this H.264 stream"))
    }

    fn remember_unsupported(&mut self, config: Config) {
        if MAX_UNSUPPORTED_CONFIGS <= self.unsupported.len() {
            self.unsupported.pop_first();
        }
        self.unsupported.insert(config);
    }

    /// Gives queued chunks to the `VideoDecoder`, a few at a time and while it has room.
    ///
    /// After a flush, the access units since the last IDR picture go first, so the decoder holds
    /// the reference pictures the next one needs.
    /// A replay that makes no progress for [`REPLAY_STALL`] while the decoder is full is given up,
    /// since a flush would start it over. A configuration whose replay stalls twice in a row is
    /// remembered as unsupported, so the next IDR picture falls back on the probed codec.
    /// Returns the pictures that will never come out, on a failure.
    pub(crate) fn pump(&mut self, sink: &mut dyn Sink, now: MonotonicInstant) -> Vec<u64> {
        if let Some(replay) = self.replay.as_mut()
            && replay.outputs != self.outputs
        {
            replay.outputs = self.outputs;
            replay.progress_at = now;
        }
        let mut given = 0;
        while given < MAX_CHUNKS_PER_TURN && self.outstanding < MAX_OUTSTANDING_CHUNKS {
            if self.key_required && self.queue.front().is_some_and(|chunk| !chunk.key) {
                let Some(history) = self.history.as_ref().filter(|history| !history.units.is_empty()) else {
                    return self.fail(sink, Failure::Corrupt);
                };
                let replay = history.units.iter().enumerate().map(|(index, unit)| Queued {
                    data: Rc::clone(unit),
                    key: index == 0,
                    number: None,
                });
                let rest = core::mem::take(&mut self.queue);
                self.queue = replay.chain(rest).collect();
                self.replay = Some(Replay {
                    outputs: self.outputs,
                    progress_at: now,
                    flush_deferred: false,
                });
            }
            let Some(chunk) = self.queue.pop_front() else {
                break;
            };
            #[expect(
                clippy::as_conversions,
                clippy::cast_precision_loss,
                reason = "picture numbers stay far below 2^53"
            )]
            let timestamp = chunk.number.map_or(-1.0, |number| number as f64);
            if let Err(failure) = sink.decode(&chunk.data, chunk.key, timestamp) {
                let mut lost = self.fail(sink, failure);
                lost.extend(chunk.number);
                return lost;
            }
            self.key_required = false;
            self.outstanding += 1;
            given += 1;
            if chunk.number.is_none() {
                if let Some(replay) = self.replay.as_mut() {
                    replay.progress_at = now;
                }
                continue;
            }
            self.update_history(&chunk);
            if let Some(replay) = self.replay.take() {
                self.stalled = None;
                if replay.flush_deferred {
                    return self.flush_now(sink);
                }
            }
        }
        if let Some(replay) = self.replay
            && MAX_OUTSTANDING_CHUNKS <= self.outstanding
            && REPLAY_STALL <= now.duration_since(replay.progress_at)
        {
            return self.stall(sink);
        }
        Vec::new()
    }

    /// Keeps the live chunk just given in the history, which follows what the `VideoDecoder` was
    /// given, so a delta still queued behind an IDR picture stays in its own group of pictures.
    fn update_history(&mut self, chunk: &Queued) {
        if chunk.key {
            self.history = History::starting_with(Rc::clone(&chunk.data));
        } else if !self
            .history
            .as_mut()
            .is_some_and(|history| history.push(Rc::clone(&chunk.data)))
        {
            self.history = None;
        }
    }

    /// Gives up a replay that made no progress, and the configuration when its last replay stalled too.
    fn stall(&mut self, sink: &mut dyn Sink) -> Vec<u64> {
        let again = self.open.is_some() && self.stalled == self.open;
        self.stalled = self.open.clone();
        if again {
            warn!("H.264 replays stalled twice with this decoder configuration; falling back");
            self.fail(sink, Failure::Unsupported)
        } else {
            warn!("An H.264 replay made no progress; waiting for an IDR picture");
            self.fail(sink, Failure::Corrupt)
        }
    }

    /// Whether the sequence parameter set picture `number` was encoded with declares full-range
    /// samples, as MS-RDPEGFX §3.3.8.3.1 requires.
    pub(crate) fn declares_full_range(&self, number: u64) -> bool {
        self.full_range
            .range(..=number)
            .next_back()
            .is_some_and(|(_, &full_range)| full_range)
    }

    /// Changes whenever the `VideoDecoder` is closed or replaced: the pictures it was given are lost.
    pub(crate) fn decoder_generation(&self) -> u64 {
        self.decoder_generation
    }

    /// Records that the `VideoDecoder` output a picture, live or replayed.
    pub(crate) fn on_output(&mut self) {
        self.outstanding = self.outstanding.saturating_sub(1);
        self.outputs += 1;
    }

    /// Asks the `VideoDecoder` to output what it holds, when it holds something.
    ///
    /// During a replay, a flush would start the replay over, so it is made once the live chunk
    /// the replay leads to is given.
    /// Returns the pictures that will never come out, on a failure.
    pub(crate) fn flush(&mut self, sink: &mut dyn Sink) -> Vec<u64> {
        if let Some(replay) = self.replay.as_mut() {
            replay.flush_deferred = true;
            return Vec::new();
        }
        if self.open.is_none() || self.outstanding == 0 {
            return Vec::new();
        }
        self.flush_now(sink)
    }

    fn flush_now(&mut self, sink: &mut dyn Sink) -> Vec<u64> {
        match sink.flush() {
            Ok(()) => {
                // WebCodecs requires a key chunk after a flush.
                self.key_required = true;
                Vec::new()
            }
            Err(failure) => self.fail(sink, failure),
        }
    }

    /// Records that the `VideoDecoder` failed, closing it.
    ///
    /// An unsupported configuration is remembered, so the next IDR picture falls back on another.
    /// Returns the pictures still queued, which will never come out.
    pub(crate) fn fail(&mut self, sink: &mut dyn Sink, failure: Failure) -> Vec<u64> {
        if let (Failure::Unsupported, Some(config)) = (failure, self.open.clone()) {
            self.remember_unsupported(config);
        }
        sink.close();
        self.decoder_generation += 1;
        self.open = None;
        self.outstanding = 0;
        self.history = None;
        self.replay = None;
        self.queue.drain(..).filter_map(|chunk| chunk.number).collect()
    }

    /// Discards everything for a new stream, the support decisions included.
    pub(crate) fn reset(&mut self, sink: &mut dyn Sink) {
        sink.close();
        self.decoder_generation += 1;
        self.parameter_sets.clear();
        self.open = None;
        self.key_required = false;
        self.history = None;
        self.replay = None;
        self.stalled = None;
        self.queue.clear();
        self.outstanding = 0;
        self.unsupported.clear();
        self.full_range.clear();
    }
}

/// The bytes of decoded pictures a session may hold outside the graphics pipeline, across all its
/// decoders: copies in progress and pictures not polled yet.
#[derive(Debug)]
pub(crate) struct Budget {
    limit: usize,
    used: Cell<usize>,
}

impl Budget {
    pub(crate) fn new(limit: usize) -> Rc<Self> {
        Rc::new(Self {
            limit,
            used: Cell::new(0),
        })
    }

    #[cfg(all(test, target_arch = "wasm32"))]
    pub(crate) fn used(&self) -> usize {
        self.used.get()
    }

    /// Reserves `bytes` until the returned reservation is dropped, or returns `None` past the limit.
    pub(crate) fn reserve(self: &Rc<Self>, bytes: usize) -> Option<Reservation> {
        let used = self.used.get().checked_add(bytes).filter(|&used| used <= self.limit)?;
        self.used.set(used);
        Some(Reservation {
            budget: Rc::clone(self),
            bytes,
        })
    }
}

/// Bytes held in a [`Budget`], released exactly once, when dropped.
#[derive(Debug)]
pub(crate) struct Reservation {
    budget: Rc<Budget>,
    bytes: usize,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.used.set(self.budget.used.get() - self.bytes);
    }
}

/// The pictures of one decoder, by submission number, as the graphics pipeline polls them.
#[derive(Debug, Default)]
pub(crate) struct Pictures {
    /// The number of the next submission.
    pub(crate) submitted: u64,
    /// The number of the next picture to poll.
    /// INVARIANT: `polled <= submitted`.
    pub(crate) polled: u64,
    /// Results with the memory their pictures hold.
    /// INVARIANT: keys in `polled..submitted`.
    finished: BTreeMap<u64, (DecoderResult<DecodedPicture>, Option<Reservation>)>,
    /// Bumped by each reset, so a copy started before it is discarded when it settles.
    pub(crate) generation: u64,
}

impl Pictures {
    /// The number the next submission takes once accepted.
    pub(crate) fn next_number(&self) -> DecoderResult<u64> {
        if MAX_PICTURES_IN_FLIGHT <= self.submitted - self.polled {
            return Err(DecoderError::msg("too many H.264 pictures in flight"));
        }
        Ok(self.submitted)
    }

    /// Whether picture `number` of `generation` is still awaited, so a copy for it is worth finishing.
    pub(crate) fn awaits(&self, generation: u64, number: u64) -> bool {
        generation == self.generation
            && self.polled <= number
            && number < self.submitted
            && !self.finished.contains_key(&number)
    }

    /// Files the result of picture `number` of `generation`, with the memory it holds.
    /// A result no longer awaited is dropped, its reservation with it. Returns whether it was filed.
    pub(crate) fn finish(
        &mut self,
        generation: u64,
        number: u64,
        result: DecoderResult<DecodedPicture>,
        reservation: Option<Reservation>,
    ) -> bool {
        if !self.awaits(generation, number) {
            return false;
        }
        let reservation = if result.is_ok() { reservation } else { None };
        self.finished.insert(number, (result, reservation));
        true
    }

    /// Ends pictures `numbers` with an error, since they will never come out.
    pub(crate) fn fail(&mut self, numbers: impl IntoIterator<Item = u64>, reason: &str) {
        let generation = self.generation;
        for number in numbers {
            self.finish(generation, number, Err(DecoderError::msg(reason.to_owned())), None);
        }
    }

    /// Ends every picture not returned yet with an error.
    pub(crate) fn fail_all(&mut self, reason: &str) {
        self.fail(self.polled..self.submitted, reason);
    }

    /// The next picture in submission order, once returned; its memory is released.
    pub(crate) fn poll(&mut self) -> Option<DecoderResult<DecodedPicture>> {
        let (result, _reservation) = self.finished.remove(&self.polled)?;
        self.polled += 1;
        Some(result)
    }

    /// Discards the pictures not polled yet; copies still running are discarded when they settle.
    pub(crate) fn reset(&mut self) {
        self.generation += 1;
        self.polled = self.submitted;
        self.finished.clear();
    }
}

/// Whether a picture's samples are expanded from limited to full range before the MS-RDPEGFX
/// conversion: its stream declares full range, and the browser reports changing only the range.
///
/// A frame whose color space names another matrix than the BT.709 one the stream is tagged with
/// was converted in a way range expansion cannot undo, so its samples are taken as copied.
pub(crate) fn expands_range(
    stream_full_range: bool,
    frame_full_range: Option<bool>,
    frame_matrix: Option<&str>,
) -> bool {
    stream_full_range && frame_full_range == Some(false) && frame_matrix.is_none_or(|matrix| matrix == "bt709")
}

/// The planes a `VideoFrame` copy wrote to `buffer`, for a `width` x `height` picture in `format`,
/// with each plane's offset and stride from `layout`.
///
/// I420 and NV12 are read, and I420A without its alpha plane; any other format is an error, since
/// its samples need another conversion than the one MS-RDPEGFX specifies.
/// With `expand_range`, limited-range samples are expanded back to full range, undoing a browser
/// that converted a full-range stream to limited range.
pub(crate) fn yuv420_planes(
    buffer: &[u8],
    format: &str,
    layout: &[(usize, usize)],
    width: u16,
    height: u16,
    expand_range: bool,
) -> DecoderResult<Yuv420Frame> {
    let picture = yuv420_planes_as_copied(buffer, format, layout, width, height)?;
    if !expand_range {
        return Ok(picture);
    }
    let (luma, chroma) = (expansion_table(16, 219), expansion_table(128 - 112, 224));
    let rows = |row: fn(&Yuv420Frame, usize) -> Option<&[u8]>, table: &[u8; 256]| {
        let mut plane = Vec::new();
        let mut index = 0;
        while let Some(samples) = row(&picture, index) {
            plane.extend(samples.iter().map(|&sample| table[usize::from(sample)]));
            index += 1;
        }
        plane
    };
    Yuv420Frame::new(
        width,
        height,
        rows(Yuv420Frame::luma_row, &luma),
        usize::from(width),
        rows(Yuv420Frame::u_row, &chroma),
        rows(Yuv420Frame::v_row, &chroma),
        usize::from(width.div_ceil(2)),
    )
}

/// Maps a limited-range sample, `span` levels from `floor` (ITU-R BT.709 §6.10: luma 16 to 235,
/// chroma 16 to 240), to the full range 0 to 255, rounding and clamping.
fn expansion_table(floor: u8, span: u16) -> [u8; 256] {
    let mut table = [0u8; 256];
    for (sample, expanded) in (0u16..).zip(table.iter_mut()) {
        let above = i32::from(sample) - i32::from(floor);
        let scaled = (above * 255 + i32::from(span) / 2).div_euclid(i32::from(span));
        *expanded = u8::try_from(scaled.clamp(0, 255)).unwrap_or(u8::MAX);
    }
    table
}

fn yuv420_planes_as_copied(
    buffer: &[u8],
    format: &str,
    layout: &[(usize, usize)],
    width: u16,
    height: u16,
) -> DecoderResult<Yuv420Frame> {
    let rows = usize::from(height);
    let chroma_rows = usize::from(height.div_ceil(2));
    let chroma_width = usize::from(width.div_ceil(2));
    let plane = |(offset, stride): (usize, usize), rows: usize, row_bytes: usize| {
        if stride < row_bytes {
            return Err(DecoderError::msg("a plane stride narrower than its rows"));
        }
        let end = stride
            .checked_mul(rows.saturating_sub(1))
            .and_then(|size| size.checked_add(row_bytes))
            .and_then(|size| size.checked_add(offset));
        end.and_then(|end| buffer.get(offset..end))
            .ok_or_else(|| DecoderError::msg("a plane outside the copied buffer"))
    };
    // Each plane is repacked with its stride, so its last row needs no padding after it.
    let packed = |plane: &[u8], stride: usize, rows: usize, row_bytes: usize| {
        let mut packed = Vec::with_capacity(row_bytes * rows);
        for row in 0..rows {
            packed.extend_from_slice(&plane[row * stride..row * stride + row_bytes]);
        }
        packed
    };
    let luma_width = usize::from(width);
    match (format, layout) {
        ("I420", &[y, u, v]) | ("I420A", &[y, u, v, _]) => Yuv420Frame::new(
            width,
            height,
            packed(plane(y, rows, luma_width)?, y.1, rows, luma_width),
            luma_width,
            packed(plane(u, chroma_rows, chroma_width)?, u.1, chroma_rows, chroma_width),
            packed(plane(v, chroma_rows, chroma_width)?, v.1, chroma_rows, chroma_width),
            chroma_width,
        ),
        ("NV12", &[y, uv]) => {
            let interleaved = plane(uv, chroma_rows, chroma_width * 2)?;
            let mut u = Vec::with_capacity(chroma_width * chroma_rows);
            let mut v = Vec::with_capacity(chroma_width * chroma_rows);
            for row in 0..chroma_rows {
                let start = row * uv.1;
                for pair in interleaved[start..start + chroma_width * 2].chunks_exact(2) {
                    u.push(pair[0]);
                    v.push(pair[1]);
                }
            }
            Yuv420Frame::new(
                width,
                height,
                packed(plane(y, rows, luma_width)?, y.1, rows, luma_width),
                luma_width,
                u,
                v,
                chroma_width,
            )
        }
        ("I420" | "I420A" | "NV12", _) => Err(DecoderError::msg(format!("a {format} copy with a wrong plane count"))),
        _ => Err(DecoderError::msg(format!(
            "unsupported VideoFrame format {format}: MS-RDPEGFX conversion needs 8-bit 4:2:0 planes"
        ))),
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    /// The capture's sequence parameter set: Constrained Baseline 1920x1080, level 4.
    const SPS: [u8; 20] = [
        0x27, 0x42, 0xC0, 0x28, 0x89, 0x8A, 0x28, 0x0F, 0x00, 0x44, 0xFC, 0xB3, 0x50, 0x20, 0x20, 0x20, 0x78, 0x40,
        0x21, 0x2C,
    ];
    const PPS: [u8; 4] = [0x28, 0xCE, 0x3C, 0x80];
    const IDR: [u8; 12] = [0x25, 0xB8, 0x00, 0x40, 0x00, 0x8D, 0xFF, 0xFF, 0xF8, 0x7A, 0x28, 0x01];
    const NON_IDR: [u8; 8] = [0x21, 0xE0, 0x02, 0x00, 0x12, 0x56, 0x75, 0x0A];

    fn annex_b(units: &[&[u8]]) -> Vec<u8> {
        let mut data = Vec::new();
        for unit in units {
            data.extend_from_slice(&[0, 0, 0, 1]);
            data.extend_from_slice(unit);
        }
        data
    }

    fn key() -> Vec<u8> {
        annex_b(&[&SPS, &PPS, &IDR])
    }

    fn delta() -> Vec<u8> {
        annex_b(&[&NON_IDR])
    }

    /// An IDR access unit whose last byte is `tag`.
    fn key_tagged(tag: u8) -> Vec<u8> {
        let mut idr = IDR.to_vec();
        idr.push(tag);
        annex_b(&[&SPS, &PPS, &idr])
    }

    /// A delta access unit whose last byte is `tag`.
    fn delta_tagged(tag: u8) -> Vec<u8> {
        let mut unit = NON_IDR.to_vec();
        unit.push(tag);
        annex_b(&[&unit])
    }

    /// An IDR access unit whose SPS declares full range, with the same profile and size as [`key`].
    fn full_range_key() -> Vec<u8> {
        let mut sps = SPS;
        sps[12..16].copy_from_slice(&[0x70, 0x10, 0x10, 0x10]);
        annex_b(&[&sps, &PPS, &IDR])
    }

    /// Records what the stream asks of the `VideoDecoder`, and can refuse configurations.
    #[derive(Default)]
    struct Recorder {
        requests: Vec<String>,
        /// The chunks given, each with the last byte of its data.
        chunks: Vec<String>,
        unsupported: Vec<String>,
    }

    impl Sink for Recorder {
        fn open(&mut self, config: &Config) -> Result<(), Failure> {
            self.requests.push(format!("open {} {:?}", config.codec, config.size));
            if self.unsupported.contains(&config.codec) {
                return Err(Failure::Unsupported);
            }
            Ok(())
        }

        fn decode(&mut self, chunk: &[u8], key: bool, timestamp: f64) -> Result<(), Failure> {
            let kind = if key { "key" } else { "delta" };
            self.requests.push(format!("{kind} {timestamp}"));
            let tag = chunk.last().copied().unwrap_or_default();
            self.chunks.push(format!("{kind} {timestamp} #{tag}"));
            Ok(())
        }

        fn flush(&mut self) -> Result<(), Failure> {
            self.requests.push("flush".to_owned());
            Ok(())
        }

        fn close(&mut self) {
            self.requests.push("close".to_owned());
        }
    }

    /// Pumps until the queue is empty, the `VideoDecoder` returning a picture for every chunk
    /// before each pump, and returns how many chunks each pump gave.
    fn drain(stream: &mut Stream, sink: &mut Recorder) -> Vec<usize> {
        let mut per_pump = Vec::new();
        loop {
            let before = sink.requests.len();
            stream.pump(sink, at(0));
            let given = sink.requests.len() - before;
            if given == 0 {
                return per_pump;
            }
            per_pump.push(given);
            for _ in 0..given {
                stream.on_output();
            }
        }
    }

    /// Submits and gives one delta to the `VideoDecoder`, which holds its picture.
    fn give_held(stream: &mut Stream, sink: &mut Recorder, number: u64) {
        stream.submit(sink, &delta(), number).expect("delta");
        stream.pump(sink, at(0));
    }

    // oracle: spec WebCodecs §"flush()" (after a flush the next chunk must be a key chunk), so the
    // chunks since the IDR go again first, never more than the bounds allow at once
    #[wasm_bindgen_test]
    fn a_flush_replays_the_stream_since_the_idr_in_bounded_steps() {
        let mut sink = Recorder::default();
        let mut stream = Stream::new("avc1.42E01F");
        stream.submit(&mut sink, &key(), 0).expect("IDR");
        for number in 1..=9 {
            stream.submit(&mut sink, &delta(), number).expect("delta");
        }
        drain(&mut stream, &mut sink);
        give_held(&mut stream, &mut sink, 10);
        sink.requests.clear();

        stream.flush(&mut sink);
        stream.submit(&mut sink, &delta(), 11).expect("delta after the flush");
        let first = drain(&mut stream, &mut sink);
        let first_requests = core::mem::take(&mut sink.requests);
        give_held(&mut stream, &mut sink, 12);
        sink.requests.clear();
        stream.flush(&mut sink);
        stream
            .submit(&mut sink, &delta(), 13)
            .expect("delta after another flush");
        drain(&mut stream, &mut sink);
        let second_requests = core::mem::take(&mut sink.requests);

        let mut expected = vec!["flush".to_owned(), "key -1".to_owned()];
        expected.extend(core::iter::repeat_n("delta -1".to_owned(), 10));
        expected.push("delta 11".to_owned());
        assert_eq!(first_requests, expected);
        assert!(first.iter().all(|&given| given <= 4), "{first:?}");
        let mut expected = vec!["flush".to_owned(), "key -1".to_owned()];
        expected.extend(core::iter::repeat_n("delta -1".to_owned(), 12));
        expected.push("delta 13".to_owned());
        assert_eq!(second_requests, expected);
    }

    // oracle: spec ITU-T H.264 §8.2.5.1 (an IDR picture marks every reference picture unused), so a
    // delta queued before an IDR picture is never replayed after it
    #[wasm_bindgen_test]
    fn a_delta_queued_before_an_idr_stays_out_of_its_replay() {
        let mut sink = Recorder::default();
        let mut stream = Stream::new("avc1.42E01F");
        stream.submit(&mut sink, &key_tagged(0), 0).expect("IDR");
        for tag in 1..8u8 {
            stream
                .submit(&mut sink, &delta_tagged(tag), u64::from(tag))
                .expect("delta");
        }
        stream.pump(&mut sink, at(0));
        stream.pump(&mut sink, at(0));
        stream
            .submit(&mut sink, &delta_tagged(8), 8)
            .expect("delta behind a full decoder");
        stream.submit(&mut sink, &key_tagged(9), 9).expect("IDR behind it");
        for _ in 0..8 {
            stream.on_output();
            stream.pump(&mut sink, at(0));
        }
        give_held(&mut stream, &mut sink, 10);
        stream.on_output();
        stream.on_output();
        sink.chunks.clear();

        stream.flush(&mut sink);
        stream.on_output();
        stream
            .submit(&mut sink, &delta_tagged(11), 11)
            .expect("delta after the flush");
        drain(&mut stream, &mut sink);

        assert_eq!(sink.chunks, ["key -1 #9", "delta -1 #10", "delta 11 #11"]);
    }

    fn at(milliseconds: u64) -> MonotonicInstant {
        MonotonicInstant::from_millis(milliseconds)
    }

    /// Gives IDR picture `first` and the nine deltas after it, the last two behind a full decoder;
    /// then flushes, the decoder outputting the eight it holds, and replays the eight given.
    fn replay_behind_a_full_decoder(stream: &mut Stream, sink: &mut Recorder, first: u64, now: MonotonicInstant) {
        stream.submit(sink, &key(), first).expect("IDR");
        for number in first + 1..=first + 9 {
            stream.submit(sink, &delta(), number).expect("delta");
        }
        for _ in 0..3 {
            stream.pump(sink, now);
        }
        stream.flush(sink);
        for _ in 0..8 {
            stream.on_output();
        }
        for _ in 0..3 {
            stream.pump(sink, now);
        }
    }

    // oracle: spec WebCodecs §"flush()" (pending outputs are emitted on flush) and mutant a flush asked
    // for during a replay dropped, so a decoder holding the live picture never outputs it
    #[wasm_bindgen_test]
    fn a_flush_during_a_replay_is_made_once_the_live_chunk_is_given() {
        let mut sink = Recorder::default();
        let mut stream = Stream::new("avc1.42E01F");
        replay_behind_a_full_decoder(&mut stream, &mut sink, 0, at(0));
        sink.requests.clear();

        let lost = stream.flush(&mut sink);
        let during = sink.requests.clone();
        for _ in 0..8 {
            stream.on_output();
        }
        stream.pump(&mut sink, at(150));

        assert!(lost.is_empty());
        assert!(during.is_empty(), "{during:?}");
        assert_eq!(sink.requests, ["delta 8", "flush"]);
    }

    // oracle: mutant a slow decoder closed at the graphics pipeline's flush deadline because no replayed
    // picture came out yet
    #[wasm_bindgen_test]
    fn a_slow_replay_is_not_given_up_before_the_stall_bound() {
        let mut sink = Recorder::default();
        let mut stream = Stream::new("avc1.42E01F");
        replay_behind_a_full_decoder(&mut stream, &mut sink, 0, at(0));
        sink.requests.clear();

        stream.flush(&mut sink);
        let early = [150, 999].map(|milliseconds| stream.pump(&mut sink, at(milliseconds)));
        stream.on_output();
        stream.pump(&mut sink, at(1500));

        assert!(early.iter().all(Vec::is_empty));
        assert_eq!(sink.requests, ["delta 8", "flush"]);
    }

    // oracle: mutant a decoder that outputs only when flushed replaying forever, or its configuration
    // marked unsupported on a single stall
    #[wasm_bindgen_test]
    fn a_configuration_whose_replay_stalls_twice_falls_back() {
        let mut sink = Recorder::default();
        let mut stream = Stream::new("avc1.42E01F");
        replay_behind_a_full_decoder(&mut stream, &mut sink, 0, at(0));
        let early = stream.pump(&mut sink, at(999));
        let lost = stream.pump(&mut sink, at(1000));
        sink.requests.clear();
        replay_behind_a_full_decoder(&mut stream, &mut sink, 10, at(2000));
        let reopened = sink.requests.first().cloned();
        let lost_again = stream.pump(&mut sink, at(3000));
        sink.requests.clear();

        stream.submit(&mut sink, &key(), 30).expect("IDR on the fallback");
        stream.pump(&mut sink, at(3000));

        assert!(early.is_empty());
        assert_eq!(lost, [8, 9]);
        assert_eq!(reopened.as_deref(), Some("open avc1.42C028 Some((1920, 1080))"));
        assert_eq!(lost_again, [18, 19]);
        assert_eq!(sink.requests, ["open avc1.42E01F None", "key 30"]);
    }

    // oracle: spec ITU-T H.264 §7.4.1.2.1 (a sequence parameter set takes effect at an IDR picture), so
    // a picture output late keeps the range of the parameter set it was encoded with
    #[wasm_bindgen_test]
    fn a_late_picture_keeps_the_range_of_its_own_sequence_parameter_set() {
        let mut sink = Recorder::default();
        let mut stream = Stream::new("avc1.42E01F");
        stream.submit(&mut sink, &key(), 0).expect("IDR");
        stream.submit(&mut sink, &delta(), 1).expect("delta");
        stream.submit(&mut sink, &full_range_key(), 2).expect("full-range IDR");
        stream.submit(&mut sink, &delta(), 3).expect("delta");

        let ranges = [0, 1, 2, 3].map(|number| stream.declares_full_range(number));
        let opened = sink
            .requests
            .iter()
            .filter(|request| request.starts_with("open"))
            .count();

        assert_eq!(ranges, [false, false, true, true]);
        assert_eq!(opened, 1);
    }

    // oracle: mutant range entries kept for pictures that can no longer be in flight
    #[wasm_bindgen_test]
    fn ranges_are_kept_only_for_pictures_still_in_flight() {
        let mut sink = Recorder::default();
        let mut stream = Stream::new("avc1.42E01F");
        for number in 0..200 {
            let key = if number % 2 == 0 { key() } else { full_range_key() };
            stream.submit(&mut sink, &key, number).expect("IDR");
        }

        assert!(stream.full_range.len() <= 33, "{}", stream.full_range.len());
        assert!(stream.declares_full_range(199));
        assert!(!stream.declares_full_range(198));
    }

    // oracle: mutant chunks given past the outstanding bound while the decoder returns nothing
    #[wasm_bindgen_test]
    fn chunks_wait_while_the_decoder_holds_eight() {
        let mut sink = Recorder::default();
        let mut stream = Stream::new("avc1.42E01F");
        stream.submit(&mut sink, &key(), 0).expect("IDR");
        for number in 1..20 {
            stream.submit(&mut sink, &delta(), number).expect("delta");
        }

        for _ in 0..5 {
            stream.pump(&mut sink, at(0));
        }

        let given = sink
            .requests
            .iter()
            .filter(|request| !request.starts_with("open"))
            .count();
        assert_eq!(given, 8);
    }

    // oracle: mutant replay history kept past its bounds
    #[wasm_bindgen_test]
    fn a_stream_past_the_history_bound_waits_for_an_idr_after_a_flush() {
        let mut sink = Recorder::default();
        let mut stream = Stream::new("avc1.42E01F");
        stream.submit(&mut sink, &key(), 0).expect("IDR");
        drain(&mut stream, &mut sink);
        for number in 1..=300 {
            stream.submit(&mut sink, &delta(), number).expect("delta");
            drain(&mut stream, &mut sink);
        }
        give_held(&mut stream, &mut sink, 301);
        sink.requests.clear();

        stream.flush(&mut sink);
        stream.submit(&mut sink, &delta(), 302).expect("queued");
        let lost = stream.pump(&mut sink, at(0));

        assert_eq!(lost, [302]);
        assert_eq!(sink.requests, ["flush", "close"]);
        assert!(stream.submit(&mut sink, &delta(), 303).is_err());
    }

    // oracle: mutant oversized access units accepted, the IDR one cloned first
    #[wasm_bindgen_test]
    fn an_access_unit_past_the_size_bound_is_rejected() {
        let mut sink = Recorder::default();
        let mut stream = Stream::new("avc1.42E01F");
        let mut huge = key();
        huge.resize(MAX_ACCESS_UNIT_BYTES + 1, 0x11);

        assert!(stream.submit(&mut sink, &huge, 0).is_err());
        assert!(sink.requests.is_empty());
    }

    // oracle: mutant a failed configuration remembered across streams or for another configuration
    #[wasm_bindgen_test]
    fn an_unsupported_configuration_falls_back_and_is_forgotten_on_reset() {
        let mut sink = Recorder {
            unsupported: vec!["avc1.42C028".to_owned()],
            ..Recorder::default()
        };
        let mut stream = Stream::new("avc1.42E01F");
        stream.submit(&mut sink, &key(), 0).expect("IDR on the fallback");
        let first = core::mem::take(&mut sink.requests);
        stream.reset(&mut sink);
        sink.unsupported.clear();
        sink.requests.clear();
        stream.submit(&mut sink, &key(), 1).expect("IDR after the reset");

        assert_eq!(first, ["open avc1.42C028 Some((1920, 1080))", "open avc1.42E01F None"]);
        assert_eq!(sink.requests, ["open avc1.42C028 Some((1920, 1080))"]);
    }

    // oracle: mutant corrupt data taken for an unsupported configuration
    #[wasm_bindgen_test]
    fn corrupt_data_does_not_mark_the_configuration_unsupported() {
        let mut sink = Recorder::default();
        let mut stream = Stream::new("avc1.42E01F");
        stream.submit(&mut sink, &key(), 0).expect("IDR");
        stream.fail(&mut sink, Failure::Corrupt);
        sink.requests.clear();

        stream.submit(&mut sink, &key(), 1).expect("IDR after the failure");

        assert_eq!(sink.requests, ["open avc1.42C028 Some((1920, 1080))"]);
    }

    // oracle: mutant an unsupported failure forgotten, so the next IDR retries the same configuration
    #[wasm_bindgen_test]
    fn a_configuration_failing_as_unsupported_falls_back() {
        let mut sink = Recorder::default();
        let mut stream = Stream::new("avc1.42E01F");
        stream.submit(&mut sink, &key(), 0).expect("IDR");
        stream.fail(&mut sink, Failure::Unsupported);
        sink.requests.clear();

        stream.submit(&mut sink, &key(), 1).expect("IDR on the fallback");

        assert_eq!(sink.requests, ["open avc1.42E01F None"]);
    }

    fn picture() -> DecodedPicture {
        Yuv420Frame::new(2, 2, vec![0; 4], 2, vec![128], vec![128], 1)
            .expect("planes")
            .into()
    }

    // oracle: mutant a copy's memory released when its picture is reset rather than when the copy settles
    #[wasm_bindgen_test]
    fn a_copy_keeps_its_memory_across_a_reset_until_it_settles() {
        let budget = Budget::new(100);
        let mut pictures = Pictures::default();
        let number = pictures.next_number().expect("room");
        pictures.submitted = number + 1;
        let generation = pictures.generation;
        let copying = budget.reserve(60).expect("in budget");

        pictures.reset();
        let during = budget.used();
        let refused = budget.reserve(60).is_none();
        let filed = pictures.finish(generation, number, Ok(picture()), Some(copying));

        assert_eq!(during, 60);
        assert!(refused);
        assert!(!filed);
        assert_eq!(budget.used(), 0);
    }

    // oracle: mutant a failed picture's error releasing the memory of a copy still running for it
    #[wasm_bindgen_test]
    fn a_failure_while_copying_releases_the_memory_once_the_copy_settles() {
        let budget = Budget::new(100);
        let mut pictures = Pictures::default();
        let number = pictures.next_number().expect("room");
        pictures.submitted = number + 1;
        let generation = pictures.generation;
        let copying = budget.reserve(60).expect("in budget");

        pictures.fail_all("the decoder failed");
        let polled_error = pictures.poll().map(|result| result.is_err());
        let during = budget.used();
        let filed = pictures.finish(generation, number, Ok(picture()), Some(copying));

        assert_eq!(polled_error, Some(true));
        assert_eq!(during, 60);
        assert!(!filed);
        assert_eq!(budget.used(), 0);
    }

    // oracle: mutant a picture's memory released before the graphics pipeline polls it
    #[wasm_bindgen_test]
    fn a_returned_picture_holds_its_memory_until_polled() {
        let budget = Budget::new(100);
        let mut pictures = Pictures::default();
        let number = pictures.next_number().expect("room");
        pictures.submitted = number + 1;
        let reservation = budget.reserve(60).expect("in budget");

        pictures.finish(pictures.generation, number, Ok(picture()), Some(reservation));
        let before_poll = budget.used();
        let polled = pictures.poll().is_some();

        assert_eq!(before_poll, 60);
        assert!(polled);
        assert_eq!(budget.used(), 0);
    }

    // oracle: spec WebCodecs §"VideoFrame copyTo" (each plane at its layout offset, rows at its stride);
    // I420 3x3 has 2x2 chroma
    #[wasm_bindgen_test]
    fn odd_sized_i420_planes_with_padded_strides_are_repacked() {
        #[rustfmt::skip]
        let buffer = [
            1, 2, 3, 0xEE,
            4, 5, 6, 0xEE,
            7, 8, 9, 0xEE,
            10, 11, 0xEE,
            12, 13, 0xEE,
            20, 21, 0xEE,
            22, 23,
        ];
        let picture = yuv420_planes(&buffer, "I420", &[(0, 4), (12, 3), (18, 3)], 3, 3, false).expect("planes");

        assert_eq!(picture.luma_row(2), Some(&[7, 8, 9][..]));
        assert_eq!(picture.u_row(1), Some(&[12, 13][..]));
        assert_eq!(picture.v_row(1), Some(&[22, 23][..]));
    }

    // oracle: spec WebCodecs §"Pixel Format" (NV12: a Y plane, then one plane of interleaved U and V)
    #[wasm_bindgen_test]
    fn nv12_chroma_is_deinterleaved() {
        let buffer = [1, 2, 3, 4, 10, 20, 0xEE, 0xEE];
        let picture = yuv420_planes(&buffer, "NV12", &[(0, 2), (4, 4)], 2, 2, false).expect("planes");

        assert_eq!(picture.u_row(0), Some(&[10][..]));
        assert_eq!(picture.v_row(0), Some(&[20][..]));
    }

    // oracle: mutant layouts outside the buffer, narrower than their rows or missing planes accepted
    #[wasm_bindgen_test]
    fn invalid_layouts_are_rejected() {
        let buffer = [0u8; 6];

        assert!(yuv420_planes(&buffer, "I420", &[(0, 2), (4, 1), (5, 1)], 2, 2, false).is_ok());
        assert!(yuv420_planes(&buffer, "I420", &[(0, 2), (4, 1), (6, 1)], 2, 2, false).is_err());
        assert!(yuv420_planes(&buffer, "I420", &[(0, 1), (4, 1), (5, 1)], 2, 2, false).is_err());
        assert!(yuv420_planes(&buffer, "I420", &[(0, 2), (4, 1)], 2, 2, false).is_err());
        assert!(yuv420_planes(&buffer, "NV12", &[(0, 2), (4, 1)], 2, 2, false).is_err());
    }

    // oracle: spec MS-RDPEGFX §3.3.8.3.1 (the conversion is defined for 8-bit samples only)
    #[wasm_bindgen_test]
    fn formats_without_an_rdp_conversion_are_rejected() {
        let buffer = [0u8; 64];

        assert!(yuv420_planes(&buffer, "I420P10", &[(0, 4), (16, 2), (24, 2)], 2, 2, false).is_err());
        assert!(yuv420_planes(&buffer, "RGBA", &[(0, 8)], 2, 2, false).is_err());
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod range_tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    // oracle: spec ITU-R BT.709 §6.10 (8-bit luma black 16 and white 235, chroma 16 to 240 around 128)
    #[wasm_bindgen_test]
    fn limited_range_samples_expand_to_full_range() {
        let buffer = [16, 235, 126, 125, 16, 240];
        let picture = yuv420_planes(&buffer, "I420", &[(0, 2), (4, 1), (5, 1)], 2, 2, true).expect("planes");

        assert_eq!(picture.luma_row(0), Some(&[0, 255][..]));
        assert_eq!(picture.luma_row(1), Some(&[128, 127][..]));
        assert_eq!(picture.u_row(0), Some(&[0][..]));
        assert_eq!(picture.v_row(0), Some(&[255][..]));
    }

    // oracle: spec WebCodecs §"VideoColorSpace" (range is reported apart from the matrix), so only a
    // full-range stream reported as limited range, with the stream's BT.709 matrix or none, is expanded
    #[wasm_bindgen_test]
    fn only_a_range_only_conversion_is_undone() {
        assert!(expands_range(true, Some(false), Some("bt709")));
        assert!(expands_range(true, Some(false), None));
        assert!(!expands_range(true, Some(true), Some("bt709")));
        assert!(!expands_range(true, None, Some("bt709")));
        assert!(!expands_range(false, Some(false), Some("bt709")));
        assert!(!expands_range(true, Some(false), Some("smpte170m")));
    }
}
