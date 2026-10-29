//! CPAL playback backend for RDPSND.
//!
//! Wave blocks are decoded on the thread that delivers them, so the latency reported for the Wave
//! Confirm PDU includes them, and queued as 16-bit PCM in a lock-free ring buffer. Decoding is
//! cheap: a 20 ms block of 44.1 kHz stereo ADPCM decodes in about 10 µs and G.711 in about 1.5 µs
//! (optimized build on a laptop CPU), and Opus and AAC decode one frame per call (AAC on a Media
//! Foundation worker thread). The device callback
//! drains it under a [`PlayoutController`], which keeps the configured latency buffered, rebuffers
//! after underruns, absorbs clock drift and discards backlogs. A dedicated thread owns the CPAL
//! stream, because CPAL streams are not `Send`, and reopens it when the device disappears or, when
//! following the system default, when the default device changes.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use core::time::Duration;
use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use cpal::traits::{DeviceTrait as _, StreamTrait as _};
use cpal::{SampleFormat, StreamConfig};
use ironrdp_rdpsnd::client::RdpsndClientHandler;
use ironrdp_rdpsnd::codec::AudioDecoder;
use ironrdp_rdpsnd::pdu::{AudioFormat, AudioFormatFlags, PitchPdu, VolumePdu, WaveFormat};
use ironrdp_rdpsnd::playout::{self, Period, PlayoutController, stretch};
use ringbuf::traits::{Consumer as _, Observer as _, Producer as _, Split as _};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use tracing::{debug, error, info, trace, warn};

use crate::device::{self, DeviceSelector, Direction};
use crate::error::{RdpsndNativeError, RdpsndNativeErrorKind, RdpsndNativeResult};

/// Default audio kept buffered ahead of the device.
pub const DEFAULT_PLAYBACK_LATENCY: Duration = Duration::from_millis(100);

/// Smallest latency target accepted; below this, ordinary scheduling jitter starves the device.
const MIN_PLAYBACK_LATENCY: Duration = Duration::from_millis(20);

/// Largest latency target accepted.
const MAX_PLAYBACK_LATENCY: Duration = Duration::from_secs(2);

/// Playback parameters accepted from the server, which bound every buffer sized from them.
const MAX_CHANNELS: u16 = 8;
const MIN_SAMPLE_RATE: u32 = 4_000;
const MAX_SAMPLE_RATE: u32 = 192_000;

/// Ring buffer capacity in multiples of the latency target, with a floor of [`MIN_RING_DURATION`].
const RING_LATENCY_MULTIPLE: u32 = 4;
const MIN_RING_DURATION: Duration = Duration::from_secs(2);

/// Playback settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackConfig {
    /// Output device.
    pub device: DeviceSelector,
    /// Audio kept buffered ahead of the device; higher values ride out more network jitter.
    pub latency: Duration,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            device: DeviceSelector::Default,
            latency: DEFAULT_PLAYBACK_LATENCY,
        }
    }
}

/// Pack left/right 16-bit volume levels into one atomic word.
///
/// Layout is internal only: left in the high half, right in the low half.
/// This is **not** the MS-RDPEA `VolumePdu` wire packing (`right << 16 | left`
/// written little-endian).
#[doc(hidden)]
pub fn pack_volume(left: u16, right: u16) -> u32 {
    (u32::from(left) << 16) | u32::from(right)
}

#[doc(hidden)]
pub fn unpack_volume(packed: u32) -> (u16, u16) {
    #[expect(clippy::as_conversions, reason = "mask isolates a u16 lane from the packed word")]
    let left = ((packed >> 16) & 0xFFFF) as u16;
    #[expect(clippy::as_conversions, reason = "mask isolates a u16 lane from the packed word")]
    let right = (packed & 0xFFFF) as u16;
    (left, right)
}

/// Scale interleaved samples by per-channel gains (0..=0xFFFF, where 0xFFFF is unity).
///
/// Channel 1 uses `right`; every other channel uses `left`.
#[doc(hidden)]
pub fn apply_gain(samples: &mut [i16], channels: usize, left: u16, right: u16) {
    if left == 0xFFFF && right == 0xFFFF {
        return;
    }
    let channels = channels.max(1);
    for frame in samples.chunks_mut(channels) {
        for (channel, sample) in frame.iter_mut().enumerate() {
            let gain = if channel == 1 { right } else { left };
            let scaled = (i32::from(*sample) * i32::from(gain)) / 0xFFFF;
            *sample = i16::try_from(scaled).expect("a gain of at most one keeps the sample in range");
        }
    }
}

/// Splits a stereo volume into a master level and per-channel balance gains.
///
/// The master is the louder channel; the balance gains scale the quieter channel relative to it.
#[doc(hidden)]
pub fn split_volume(left: u16, right: u16) -> (f32, u16, u16) {
    let master = left.max(right);
    if master == 0 {
        return (0.0, 0xFFFF, 0xFFFF);
    }
    let balance = |level: u16| {
        let gain = u32::from(level) * 0xFFFF / u32::from(master);
        u16::try_from(gain).expect("level is at most the master")
    };
    (f32::from(master) / f32::from(u16::MAX), balance(left), balance(right))
}

/// Decoder for the wave data of the active format.
enum Decoder {
    Codec(AudioDecoder),
    #[cfg(feature = "opus")]
    Opus(opus2::Decoder, usize),
    #[cfg(all(windows, feature = "aac"))]
    Aac(crate::aac::AacDecoder),
}

impl Decoder {
    fn new(format: &AudioFormat) -> RdpsndNativeResult<Self> {
        if !has_playable_layout(format) {
            return Err(RdpsndNativeError::new(
                "channel count or sample rate out of range",
                RdpsndNativeErrorKind::UnsupportedFormat,
            ));
        }
        match format.format {
            #[cfg(feature = "opus")]
            WaveFormat::OPUS => {
                let channels = match format.n_channels {
                    1 => opus2::Channels::Mono,
                    2 => opus2::Channels::Stereo,
                    _ => {
                        return Err(RdpsndNativeError::new(
                            "unsupported channel count for Opus",
                            RdpsndNativeErrorKind::UnsupportedFormat,
                        ));
                    }
                };
                let decoder = opus2::Decoder::new(format.n_samples_per_sec, channels).map_err(|e| {
                    RdpsndNativeError::new("creating Opus decoder", RdpsndNativeErrorKind::OpusInit).with_source(e)
                })?;
                Ok(Self::Opus(decoder, usize::from(format.n_channels)))
            }
            #[cfg(all(windows, feature = "aac"))]
            WaveFormat::AAC_MS => Ok(Self::Aac(crate::aac::AacDecoder::new(format)?)),
            _ => AudioDecoder::new(format).map(Self::Codec).map_err(|e| {
                RdpsndNativeError::new(
                    "no decoder for the audio format",
                    RdpsndNativeErrorKind::UnsupportedFormat,
                )
                .with_source(e)
            }),
        }
    }

    /// Returns whether `format` can be decoded.
    fn supports(format: &AudioFormat) -> bool {
        if !has_playable_layout(format) {
            return false;
        }
        match format.format {
            #[cfg(feature = "opus")]
            WaveFormat::OPUS => matches!(format.n_channels, 1 | 2),
            #[cfg(all(windows, feature = "aac"))]
            WaveFormat::AAC_MS => crate::aac::AacDecoder::supports(format),
            _ => AudioDecoder::supports(format),
        }
    }

    /// Decodes `data`, appending interleaved samples to `out`.
    fn decode(&mut self, data: &[u8], out: &mut Vec<i16>) {
        match self {
            Self::Codec(decoder) => {
                if let Err(error) = decoder.decode(data, out) {
                    warn!(%error, "Failed to decode a wave block");
                }
            }
            #[cfg(feature = "opus")]
            Self::Opus(decoder, channels) => {
                let frames = match decoder.get_nb_samples(data) {
                    Ok(frames) => frames,
                    Err(error) => {
                        warn!(?error, "Failed to get the number of samples of an Opus packet");
                        return;
                    }
                };
                let start = out.len();
                out.resize(start + frames * *channels, 0);
                match decoder.decode(data, &mut out[start..], false) {
                    Ok(frames_written) => out.truncate(start + frames_written * *channels),
                    Err(error) => {
                        warn!(?error, "Failed to decode an Opus packet");
                        out.truncate(start);
                    }
                }
            }
            #[cfg(all(windows, feature = "aac"))]
            Self::Aac(decoder) => {
                if let Err(error) = decoder.decode(data, out) {
                    warn!(error = %error.report(), "Failed to decode an AAC frame");
                }
            }
        }
    }
}

/// State shared between the backend, the stream thread and the device callback.
#[derive(Debug, Default)]
struct StreamShared {
    ended: AtomicBool,
    /// Set once the server has sent a volume; until then the session level is left alone.
    volume_received: AtomicBool,
    volume_changed: AtomicBool,
    device_lost: AtomicBool,
    /// Frames discarded to cut a backlog, and underruns, counted by the device callback.
    skipped_frames: AtomicU64,
    underruns: AtomicU64,
    /// Whether the stream's audio session applies the master volume, leaving only balance in software.
    session_volume: AtomicBool,
    /// Delay between the device callback and the moment its samples reach the speaker.
    device_latency_us: AtomicU64,
}

/// Device callback state, kept across stream rebuilds.
struct Playout {
    consumer: HeapCons<i16>,
    controller: PlayoutController,
    channels: usize,
    scratch: Vec<i16>,
    shared: Arc<StreamShared>,
}

impl Playout {
    /// Fills one device period.
    ///
    /// Runs on the real-time audio thread: it never blocks or logs, and allocates only if a period
    /// is longer than the scratch space reserved up front.
    // INVARIANT: this path does not block, log or allocate per period, or the device glitches.
    // UNTESTED: no test checks it; keep every call here lock-free (ring buffer, atomics).
    fn fill(&mut self, data: &mut [i16]) {
        let channels = self.channels;
        let requested = data.len() / channels;
        let buffered = self.consumer.occupied_len() / channels;

        match self.controller.next_period(buffered, requested) {
            Period::Silence => data.fill(0),
            Period::Play { skip, consume } => {
                if skip != 0 {
                    self.consumer.skip(skip * channels);
                    let skip = u64::try_from(skip).unwrap_or(u64::MAX);
                    self.shared.skipped_frames.fetch_add(skip, Ordering::Relaxed);
                }
                self.scratch.resize(consume * channels, 0);
                let popped = self.consumer.pop_slice(&mut self.scratch);
                self.scratch.truncate(popped);
                stretch(&self.scratch, channels, data);
            }
            Period::Underrun { consume } => {
                self.shared.underruns.fetch_add(1, Ordering::Relaxed);
                let (played, silence) = data.split_at_mut(consume * channels);
                let popped = self.consumer.pop_slice(played);
                played[popped..].fill(0);
                silence.fill(0);
            }
        }
    }
}

struct ActiveStream {
    format: AudioFormat,
    decoder: Decoder,
    producer: HeapProd<i16>,
    channels: usize,
    sample_rate: u32,
    shared: Arc<StreamShared>,
    thread: JoinHandle<()>,
    decoded: Vec<i16>,
}

impl core::fmt::Debug for ActiveStream {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ActiveStream")
            .field("format", &self.format)
            .field("channels", &self.channels)
            .field("sample_rate", &self.sample_rate)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct RdpsndBackend {
    config: PlaybackConfig,
    stream: Option<ActiveStream>,
    volume: Arc<AtomicU32>,
    /// Whether the server has sent a volume during this connection.
    volume_received: bool,
    /// Formats the [`RdpsndClientHandler::get_formats`] fallback advertises.
    formats: Vec<AudioFormat>,
}

impl Default for RdpsndBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl RdpsndBackend {
    pub fn new() -> Self {
        Self::with_config(PlaybackConfig::default())
    }

    pub fn with_config(config: PlaybackConfig) -> Self {
        let mut config = config;
        config.latency = config.latency.clamp(MIN_PLAYBACK_LATENCY, MAX_PLAYBACK_LATENCY);
        Self {
            config,
            stream: None,
            volume: Arc::new(AtomicU32::new(pack_volume(0xFFFF, 0xFFFF))),
            volume_received: false,
            formats: fallback_formats(),
        }
    }

    fn open(&mut self, format: &AudioFormat) -> RdpsndNativeResult<()> {
        let decoder = Decoder::new(format)?;
        let channels = usize::from(format.n_channels);
        let sample_rate = format.n_samples_per_sec;

        let ring_duration = (self.config.latency * RING_LATENCY_MULTIPLE).max(MIN_RING_DURATION);
        // INVARIANT: `Decoder::new` bounded the channel count and the sample rate, and the latency
        // is clamped, so the ring holds at most 8 s at 192 kHz on 8 channels.
        let ring_samples = frames_in(ring_duration, sample_rate)
            .checked_mul(channels)
            .ok_or_else(|| {
                RdpsndNativeError::new(
                    "playback buffer size overflow",
                    RdpsndNativeErrorKind::UnsupportedFormat,
                )
            })?;
        let (producer, consumer) = HeapRb::<i16>::new(ring_samples).split();
        let mut playout = Playout {
            consumer,
            controller: PlayoutController::new(sample_rate, self.config.latency),
            channels,
            // A quarter of a second covers any device period in practice.
            scratch: Vec::with_capacity(frames_in(Duration::from_millis(250), sample_rate) * channels),
            shared: Arc::default(),
        };

        let shared = Arc::new(StreamShared::default());
        shared.volume_received.store(self.volume_received, Ordering::Relaxed);
        playout.shared = Arc::clone(&shared);
        let thread = {
            let shared = Arc::clone(&shared);
            let volume = Arc::clone(&self.volume);
            let selector = self.config.device.clone();
            let config = StreamConfig {
                channels: format.n_channels,
                sample_rate,
                buffer_size: cpal::BufferSize::Default,
            };
            thread::Builder::new()
                .name("rdpsnd-playback".to_owned())
                .spawn(move || run_stream(&selector, &config, playout, &shared, &volume))
                .map_err(|e| {
                    RdpsndNativeError::new("spawning the playback thread", RdpsndNativeErrorKind::StreamBuild)
                        .with_source(e)
                })?
        };

        debug!(?format, latency = ?self.config.latency, "Opened playback stream");
        self.stream = Some(ActiveStream {
            format: format.clone(),
            decoder,
            producer,
            channels,
            sample_rate,
            shared,
            thread,
            decoded: Vec::new(),
        });
        Ok(())
    }
}

/// Formats advertised when the server's list is not consulted (see `choose_formats`).
fn fallback_formats() -> Vec<AudioFormat> {
    let pcm = |channels: u16, rate: u32| AudioFormat {
        format: WaveFormat::PCM,
        n_channels: channels,
        n_samples_per_sec: rate,
        n_avg_bytes_per_sec: rate * u32::from(channels) * 2,
        n_block_align: channels * 2,
        bits_per_sample: 16,
        data: None,
    };
    vec![
        #[cfg(feature = "opus")]
        AudioFormat {
            format: WaveFormat::OPUS,
            ..pcm(2, 48000)
        },
        pcm(2, 48000),
        pcm(2, 44100),
        pcm(2, 22050),
        pcm(1, 44100),
        pcm(1, 22050),
        pcm(2, 16000),
        pcm(1, 16000),
        pcm(1, 8000),
    ]
}

/// Returns whether `format` has a channel count and sample rate playback accepts.
fn has_playable_layout(format: &AudioFormat) -> bool {
    (1..=MAX_CHANNELS).contains(&format.n_channels)
        && (MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&format.n_samples_per_sec)
}

/// Returns whether the output device can open a 16-bit stream at the decoded layout of `format`.
///
/// WASAPI streams convert the rate and channel layout to the device's mix format, so every layout
/// opens there. Other hosts are asked for their supported configurations; when that fails, the
/// format is accepted and an open failure is reported later.
fn device_accepts(configs: Option<&[cpal::SupportedStreamConfigRange]>, format: &AudioFormat) -> bool {
    let Some(configs) = configs else {
        return true;
    };
    configs.iter().any(|range| {
        range.sample_format() == SampleFormat::I16
            && range.channels() == format.n_channels
            && range.min_sample_rate() <= format.n_samples_per_sec
            && format.n_samples_per_sec <= range.max_sample_rate()
    })
}

/// Supported configurations of the selected output device, or `None` when every layout opens.
fn device_configs(selector: &DeviceSelector) -> Option<Vec<cpal::SupportedStreamConfigRange>> {
    #[cfg(windows)]
    if cpal::default_host().id() == cpal::HostId::Wasapi {
        return None;
    }
    let device = device::open_device(selector, Direction::Output)?;
    device.supported_output_configs().ok().map(|configs| configs.collect())
}

fn frames_in(duration: Duration, sample_rate: u32) -> usize {
    playout::frames_in(duration, sample_rate).max(1)
}

impl Drop for RdpsndBackend {
    fn drop(&mut self) {
        self.close();
    }
}

impl RdpsndClientHandler for RdpsndBackend {
    fn get_flags(&self) -> AudioFormatFlags {
        // VOLUME is applied through the audio session (Windows) or sample scaling.
        // PITCH is not implemented — do not advertise it.
        AudioFormatFlags::VOLUME
    }

    fn get_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn choose_formats(&self, server_formats: &[AudioFormat]) -> Vec<AudioFormat> {
        // Windows lists its formats from most to least preferred and then picks among the client's
        // according to the quality mode and the measured bandwidth, so every format the device can
        // play is advertised, in the server's order.
        let configs = device_configs(&self.config.device);
        let decodable: Vec<AudioFormat> = server_formats
            .iter()
            .filter(|format| Decoder::supports(format))
            .cloned()
            .collect();
        let mut chosen: Vec<AudioFormat> = decodable
            .iter()
            .filter(|format| device_accepts(configs.as_deref(), format))
            .cloned()
            .collect();
        if chosen.is_empty() && !decodable.is_empty() {
            // A host that lists no matching 16-bit layout may still open one; advertising nothing
            // would end audio redirection outright.
            warn!("No offered format matches the output device's listed layouts; advertising all decodable ones");
            chosen = decodable;
        }
        debug!(
            offered = server_formats.len(),
            chosen = chosen.len(),
            "Chose decodable audio output formats"
        );
        chosen
    }

    fn wave(&mut self, format: &AudioFormat, _ts: u32, data: Cow<'_, [u8]>) {
        if self
            .stream
            .as_ref()
            .is_some_and(|stream| !stream.format.matches_for_negotiation(format))
        {
            debug!(?format, "New audio format");
            self.close();
        }

        if self.stream.is_none() {
            if let Err(e) = self.open(format) {
                // Soft-fail: the next wave block retries.
                error!(error = %e.report().with_locations(), "Failed to open the playback stream");
                return;
            }
        }
        let stream = self.stream.as_mut().expect("opened above");

        stream.decoded.clear();
        stream.decoder.decode(&data, &mut stream.decoded);
        let written = stream.producer.push_slice(&stream.decoded);
        if written < stream.decoded.len() {
            warn!(
                dropped_samples = stream.decoded.len() - written,
                "Playback buffer full; dropping audio"
            );
        }
    }

    fn playback_latency(&self) -> Duration {
        let Some(stream) = self.stream.as_ref() else {
            return Duration::ZERO;
        };
        let buffered_frames = stream.producer.occupied_len() / stream.channels;
        let buffered = Duration::from_micros(
            u64::try_from(buffered_frames).unwrap_or(u64::MAX) * 1_000_000 / u64::from(stream.sample_rate.max(1)),
        );
        let device = Duration::from_micros(stream.shared.device_latency_us.load(Ordering::Relaxed));
        buffered + device
    }

    fn set_volume(&mut self, volume: VolumePdu) {
        debug!(?volume);
        self.volume
            .store(pack_volume(volume.volume_left, volume.volume_right), Ordering::Relaxed);
        self.volume_received = true;
        if let Some(stream) = self.stream.as_ref() {
            stream.shared.volume_received.store(true, Ordering::Relaxed);
            stream.shared.volume_changed.store(true, Ordering::Relaxed);
            stream.thread.thread().unpark();
        }
    }

    fn set_pitch(&mut self, pitch: PitchPdu) {
        // Not advertised via get_flags; ignore if a server still sends it.
        debug!(?pitch, "Ignoring pitch (not implemented)");
    }

    fn close(&mut self) {
        if let Some(stream) = self.stream.take() {
            stream.shared.ended.store(true, Ordering::Relaxed);
            stream.thread.thread().unpark();
            if let Err(error) = stream.thread.join() {
                error!(?error, "Failed to join the playback thread");
            }
        }
    }
}

/// Body of the stream thread: keeps a CPAL stream open on the selected device until `ended`.
fn run_stream(
    selector: &DeviceSelector,
    config: &StreamConfig,
    playout: Playout,
    shared: &Arc<StreamShared>,
    volume: &Arc<AtomicU32>,
) {
    let playout = Arc::new(Mutex::new(playout));
    let mut reported_failure = false;

    while !shared.ended.load(Ordering::Relaxed) {
        let Some(device) = device::open_device(selector, Direction::Output) else {
            if !reported_failure {
                warn!(device = %selector, "Audio output device not found; retrying");
                reported_failure = true;
            }
            thread::park_timeout(device::DEVICE_RETRY_INTERVAL);
            continue;
        };
        let device_id = device::device_id(&device);

        let stream = match build_stream(&device, config, &playout, shared, volume) {
            Ok(stream) => stream,
            Err(e) => {
                if !reported_failure {
                    warn!(error = %e.report().with_locations(), "Failed to open the audio output device; retrying");
                    reported_failure = true;
                }
                thread::park_timeout(device::DEVICE_RETRY_INTERVAL);
                continue;
            }
        };
        reported_failure = false;
        info!(device = ?device_id, "Playing audio");

        #[cfg(windows)]
        let session_volume =
            device::endpoint_id(&device).and_then(|id| match crate::session_volume::SessionVolume::open(&id) {
                Ok(session_volume) => Some(session_volume),
                Err(error) => {
                    warn!(%error, "Audio session volume unavailable; scaling samples instead");
                    None
                }
            });
        #[cfg(windows)]
        let apply_session_volume = |session_volume: &Option<crate::session_volume::SessionVolume>| {
            if let Some(session_volume) = session_volume {
                let (left, right) = unpack_volume(volume.load(Ordering::Relaxed));
                let (master, _, _) = split_volume(left, right);
                if let Err(error) = session_volume.set(master) {
                    warn!(%error, "Failed to set the audio session volume");
                }
            }
        };
        #[cfg(windows)]
        {
            shared.session_volume.store(session_volume.is_some(), Ordering::Relaxed);
            // INVARIANT: the session level stays untouched until the server sends a volume, as Windows
            // keeps the user's per-application level across runs.
            // UNTESTED: no test checks it; keep `volume_received` gating every `SessionVolume::set` call.
            if shared.volume_received.load(Ordering::Relaxed) {
                apply_session_volume(&session_volume);
            }
        }

        let (mut skipped_frames, mut underruns) = (0, 0);
        device::watch_device(
            selector,
            Direction::Output,
            device_id.as_deref(),
            &shared.ended,
            &shared.device_lost,
            || {
                let (skipped_now, underruns_now) = (
                    shared.skipped_frames.load(Ordering::Relaxed),
                    shared.underruns.load(Ordering::Relaxed),
                );
                if skipped_now != skipped_frames || underruns_now != underruns {
                    debug!(
                        skipped_frames = skipped_now - skipped_frames,
                        underruns = underruns_now - underruns,
                        "Playback buffer corrections"
                    );
                    (skipped_frames, underruns) = (skipped_now, underruns_now);
                }
                if shared.volume_changed.swap(false, Ordering::Relaxed) {
                    #[cfg(windows)]
                    apply_session_volume(&session_volume);
                }
            },
        );

        drop(stream);
    }
    debug!("Playback thread exiting");
}

fn build_stream(
    device: &cpal::Device,
    config: &StreamConfig,
    playout: &Arc<Mutex<Playout>>,
    shared: &Arc<StreamShared>,
    volume: &Arc<AtomicU32>,
) -> RdpsndNativeResult<cpal::Stream> {
    let channels = usize::from(config.channels);
    let stream_thread = thread::current();

    let stream = {
        let playout = Arc::clone(playout);
        let shared = Arc::clone(shared);
        let volume = Arc::clone(volume);
        let error_shared = Arc::clone(&shared);
        device.build_output_stream_raw(
            config,
            SampleFormat::I16,
            move |data, info: &cpal::OutputCallbackInfo| {
                let Some(samples) = data.as_slice_mut::<i16>() else {
                    return;
                };
                // The stream thread only holds the lock between streams.
                let Ok(mut playout) = playout.try_lock() else {
                    samples.fill(0);
                    return;
                };
                playout.fill(samples);

                let (left, right) = unpack_volume(volume.load(Ordering::Relaxed));
                let (left, right) = if shared.session_volume.load(Ordering::Relaxed) {
                    let (_, left, right) = split_volume(left, right);
                    (left, right)
                } else {
                    (left, right)
                };
                apply_gain(samples, channels, left, right);

                let timestamp = info.timestamp();
                if let Some(latency) = timestamp.playback.duration_since(&timestamp.callback) {
                    let micros = u64::try_from(latency.as_micros()).unwrap_or(u64::MAX);
                    shared.device_latency_us.store(micros, Ordering::Relaxed);
                }
            },
            move |error| match error {
                cpal::StreamError::DeviceNotAvailable | cpal::StreamError::StreamInvalidated => {
                    error_shared.device_lost.store(true, Ordering::Relaxed);
                    stream_thread.unpark();
                }
                cpal::StreamError::BufferUnderrun => trace!("Audio device underrun"),
                error => error!(%error, "Audio output stream error"),
            },
            None,
        )
    }
    .map_err(|e| {
        RdpsndNativeError::new("building cpal output stream", RdpsndNativeErrorKind::StreamBuild).with_source(e)
    })?;

    // cpal streams start paused; without play() the callback never runs.
    stream.play().map_err(|e| {
        RdpsndNativeError::new("starting cpal output stream", RdpsndNativeErrorKind::StreamBuild).with_source(e)
    })?;

    Ok(stream)
}
