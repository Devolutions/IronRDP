use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::time::Duration;
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{SampleFormat, Stream, StreamConfig, SupportedStreamConfigRange};
use ironrdp_error::bail;
use ironrdp_rdpsnd::client::RdpsndClientHandler;
use ironrdp_rdpsnd::pdu::{AudioFormat, AudioFormatFlags, PitchPdu, VolumePdu, WaveFormat};
use tracing::{debug, error, trace, warn};

use crate::error::{RdpsndNativeError, RdpsndNativeErrorKind, RdpsndNativeResult};

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

/// Scale interleaved PCM samples by left/right volume (0..=0xFFFF).
///
/// Uses a simple amplitude scale per sample. MS-RDPEA §2.2.2.2 describes
/// `dwVolume` as logarithmic for perceived loudness; this path is a fidelity
/// approximation that preserves the endpoints (silence / full scale).
///
/// `sample_phase` is the channel index of the next sample and is advanced so
/// L/R assignment stays correct across wave blocks and multi-channel layouts.
#[doc(hidden)]
pub fn apply_volume(
    data: &mut [u8],
    bits_per_sample: u16,
    channels: u16,
    left: u16,
    right: u16,
    sample_phase: &mut usize,
) {
    if left == 0xFFFF && right == 0xFFFF {
        // Still advance phase so a later non-full volume stays aligned.
        let sample_bytes = match bits_per_sample {
            8 => 1usize,
            16 => 2usize,
            _ => return,
        };
        *sample_phase = sample_phase.wrapping_add(data.len() / sample_bytes);
        return;
    }

    let channels = usize::from(channels.max(1));
    let lane_volume = |phase: usize| -> u16 {
        if channels == 1 {
            left
        } else {
            match phase % channels {
                1 => right,
                _ => left,
            }
        }
    };

    match bits_per_sample {
        8 => {
            for sample in data.iter_mut() {
                let vol = lane_volume(*sample_phase);
                // U8 PCM is unsigned with midpoint 128.
                let centered = i16::from(*sample) - 128;
                let scaled = (i32::from(centered) * i32::from(vol)) / 0xFFFF;
                let out = (scaled + 128).clamp(0, 255);
                *sample = u8::try_from(out).unwrap_or(0);
                *sample_phase = sample_phase.wrapping_add(1);
            }
        }
        16 => {
            for chunk in data.chunks_exact_mut(2) {
                let vol = lane_volume(*sample_phase);
                let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                let scaled = (i32::from(sample) * i32::from(vol)) / 0xFFFF;
                let out = scaled.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
                let bytes = i16::try_from(out).unwrap_or(0).to_le_bytes();
                chunk[0] = bytes[0];
                chunk[1] = bytes[1];
                *sample_phase = sample_phase.wrapping_add(1);
            }
        }
        _ => {}
    }
}

#[derive(Debug)]
pub struct RdpsndBackend {
    // Unfortunately, Stream is not `Send`, so we move it to a separate thread.
    stream_handle: Option<JoinHandle<()>>,
    stream_ended: Arc<AtomicBool>,
    tx: Option<Sender<Vec<u8>>>,
    active_format: Option<AudioFormat>,
    volume: Arc<AtomicU32>,
    /// Formats advertised during RDPSND negotiation (device-filtered PCM + optional Opus).
    formats: Vec<AudioFormat>,
}

impl Default for RdpsndBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl RdpsndBackend {
    pub fn new() -> Self {
        Self {
            tx: None,
            active_format: None,
            stream_handle: None,
            stream_ended: Arc::new(AtomicBool::new(false)),
            volume: Arc::new(AtomicU32::new(pack_volume(0xFFFF, 0xFFFF))),
            formats: build_output_formats(),
        }
    }
}

fn candidate_output_formats() -> Vec<AudioFormat> {
    vec![
        #[cfg(feature = "opus")]
        AudioFormat {
            format: WaveFormat::OPUS,
            n_channels: 2,
            n_samples_per_sec: 48000,
            n_avg_bytes_per_sec: 192000,
            n_block_align: 4,
            bits_per_sample: 16,
            data: None,
        },
        AudioFormat {
            format: WaveFormat::PCM,
            n_channels: 2,
            n_samples_per_sec: 48000,
            n_avg_bytes_per_sec: 192000,
            n_block_align: 4,
            bits_per_sample: 16,
            data: None,
        },
        AudioFormat {
            format: WaveFormat::PCM,
            n_channels: 2,
            n_samples_per_sec: 44100,
            n_avg_bytes_per_sec: 176400,
            n_block_align: 4,
            bits_per_sample: 16,
            data: None,
        },
        AudioFormat {
            format: WaveFormat::PCM,
            n_channels: 2,
            n_samples_per_sec: 22050,
            n_avg_bytes_per_sec: 88200,
            n_block_align: 4,
            bits_per_sample: 16,
            data: None,
        },
        AudioFormat {
            format: WaveFormat::PCM,
            n_channels: 1,
            n_samples_per_sec: 44100,
            n_avg_bytes_per_sec: 88200,
            n_block_align: 2,
            bits_per_sample: 16,
            data: None,
        },
        AudioFormat {
            format: WaveFormat::PCM,
            n_channels: 1,
            n_samples_per_sec: 22050,
            n_avg_bytes_per_sec: 44100,
            n_block_align: 2,
            bits_per_sample: 16,
            data: None,
        },
        AudioFormat {
            format: WaveFormat::PCM,
            n_channels: 2,
            n_samples_per_sec: 16000,
            n_avg_bytes_per_sec: 64000,
            n_block_align: 4,
            bits_per_sample: 16,
            data: None,
        },
        AudioFormat {
            format: WaveFormat::PCM,
            n_channels: 1,
            n_samples_per_sec: 16000,
            n_avg_bytes_per_sec: 32000,
            n_block_align: 2,
            bits_per_sample: 16,
            data: None,
        },
        AudioFormat {
            format: WaveFormat::PCM,
            n_channels: 1,
            n_samples_per_sec: 8000,
            n_avg_bytes_per_sec: 16000,
            n_block_align: 2,
            bits_per_sample: 16,
            data: None,
        },
    ]
}

fn pcm_sample_format(bits_per_sample: u16) -> Option<SampleFormat> {
    match bits_per_sample {
        8 => Some(SampleFormat::U8),
        16 => Some(SampleFormat::I16),
        _ => None,
    }
}

fn config_supports_pcm(range: &SupportedStreamConfigRange, format: &AudioFormat) -> bool {
    let Some(sample_format) = pcm_sample_format(format.bits_per_sample) else {
        return false;
    };
    if range.sample_format() != sample_format {
        return false;
    }
    if range.channels() < format.n_channels {
        return false;
    }
    let rate = format.n_samples_per_sec;
    rate >= range.min_sample_rate() && rate <= range.max_sample_rate()
}

fn build_output_formats() -> Vec<AudioFormat> {
    let candidates = candidate_output_formats();
    let host = cpal::default_host();
    let Some(device) = host.default_output_device() else {
        warn!("No default output device; advertising unfiltered PCM candidates");
        return candidates;
    };
    let configs = match device.supported_output_configs() {
        Ok(configs) => configs.collect::<Vec<_>>(),
        Err(error) => {
            warn!(%error, "Failed to query output configs; advertising unfiltered PCM candidates");
            return candidates;
        }
    };

    candidates
        .into_iter()
        .filter(|format| {
            if format.format != WaveFormat::PCM {
                // Codec formats (e.g. Opus) are decoded to PCM before the device.
                return true;
            }
            configs.iter().any(|range| config_supports_pcm(range, format))
        })
        .collect()
}

impl Drop for RdpsndBackend {
    fn drop(&mut self) {
        self.close();
    }
}

impl RdpsndClientHandler for RdpsndBackend {
    fn get_flags(&self) -> AudioFormatFlags {
        // VOLUME is implemented via sample scaling in the playback path.
        // PITCH is not implemented — do not advertise it.
        AudioFormatFlags::VOLUME
    }

    fn get_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn wave(&mut self, format: &AudioFormat, _ts: u32, data: Cow<'_, [u8]>) {
        // Soft-fail reopen: if the stream thread exited after a device/open error,
        // reap it so this (or a later) wave block can try again.
        if self.stream_handle.as_ref().is_some_and(|handle| handle.is_finished()) {
            self.close();
        }

        let format_changed = self
            .active_format
            .as_ref()
            .is_none_or(|active| !active.matches_for_negotiation(format));

        if format_changed {
            debug!(?format, "New audio format");
            self.close();
        }

        if self.stream_handle.is_none() {
            let (tx, rx) = mpsc::channel();
            self.tx = Some(tx);

            self.active_format = Some(format.clone());
            let format = format.clone();
            self.stream_ended.store(false, Ordering::Relaxed);
            let stream_ended = Arc::clone(&self.stream_ended);
            let volume = Arc::clone(&self.volume);
            self.stream_handle = Some(thread::spawn(move || {
                let stream = match DecodeStream::new(&format, rx, volume) {
                    Ok(stream) => stream,
                    Err(e) => {
                        // Soft-fail: log and exit the stream thread. Further wave
                        // blocks will retry opening a stream.
                        error!(error = %e.report().with_locations());
                        return;
                    }
                };
                debug!("Stream thread parking loop");
                while !stream_ended.load(Ordering::Relaxed) {
                    thread::park();
                }
                debug!("Stream thread unparked");
                drop(stream);
            }));
        }

        if let Some(ref tx) = self.tx {
            if let Err(error) = tx.send(data.to_vec()) {
                error!(%error);
            }
        };
    }

    fn set_volume(&mut self, volume: VolumePdu) {
        debug!(?volume);
        self.volume
            .store(pack_volume(volume.volume_left, volume.volume_right), Ordering::Relaxed);
    }

    fn set_pitch(&mut self, pitch: PitchPdu) {
        // Not advertised via get_flags; ignore if a server still sends it.
        debug!(?pitch, "Ignoring pitch (not implemented)");
    }

    fn close(&mut self) {
        self.tx = None;
        self.active_format = None;
        if let Some(stream) = self.stream_handle.take() {
            self.stream_ended.store(true, Ordering::Relaxed);
            stream.thread().unpark();
            if let Err(err) = stream.join() {
                error!(?err, "Failed to join a stream thread");
            }
        }
    }
}

#[doc(hidden)]
pub struct DecodeStream {
    _dec_thread: Option<JoinHandle<()>>,
    stream: Stream,
}

impl DecodeStream {
    pub fn new(rx_format: &AudioFormat, rx: Receiver<Vec<u8>>, volume: Arc<AtomicU32>) -> RdpsndNativeResult<Self> {
        let mut dec_thread = None;
        let frame_source = match rx_format.format {
            #[cfg(feature = "opus")]
            WaveFormat::OPUS => {
                let chan = match rx_format.n_channels {
                    1 => opus2::Channels::Mono,
                    2 => opus2::Channels::Stereo,
                    _ => bail!(
                        "unsupported channel count for Opus",
                        RdpsndNativeErrorKind::UnsupportedFormat,
                    ),
                };
                let (dec_tx, dec_rx) = mpsc::channel::<Vec<i16>>();
                let mut dec = opus2::Decoder::new(rx_format.n_samples_per_sec, chan).map_err(|e| {
                    RdpsndNativeError::new("creating Opus decoder", RdpsndNativeErrorKind::OpusInit).with_source(e)
                })?;
                dec_thread = Some(thread::spawn(move || {
                    while let Ok(pkt) = rx.recv() {
                        let nb_samples = match dec.get_nb_samples(&pkt) {
                            Ok(nb_samples) => nb_samples,
                            Err(error) => {
                                error!(?error, "Failed to get the number of samples of an Opus packet");
                                continue;
                            }
                        };

                        #[expect(
                            clippy::as_conversions,
                            reason = "opus::Channels has no conversions to usize implemented"
                        )]
                        let mut pcm_i16 = vec![0i16; nb_samples * chan as usize];
                        if let Err(error) = dec.decode(&pkt, &mut pcm_i16, false) {
                            error!(?error, "Failed to decode an Opus packet");
                            continue;
                        }
                        // Send the decoded samples as-is. RxBuffer reads them through a
                        // byte view (bytemuck::cast_slice) only at the point it copies
                        // into the cpal output buffer, so this avoids the extra
                        // allocation and memcpy a Vec<i16> -> Vec<u8> conversion here
                        // would otherwise cost on every decoded packet. Allocating as
                        // Vec<i16> in the first place (rather than decoding into a u8
                        // buffer) also keeps clear of the alignment hazard that caused
                        // a hard crash in #1202: a u8 buffer is not guaranteed to be
                        // 2-byte aligned, so reinterpreting it as i16 can fail, while
                        // the reverse (i16 buffer viewed as bytes, used below and in
                        // RxBuffer) narrows alignment and cannot.
                        if dec_tx.send(pcm_i16).is_err() {
                            error!("Failed to send the decoded Opus packet over the channel");
                            // If send has failed, it means that the receiver has been dropped.
                            // There is no point in continuing the loop in this case.
                            break;
                        }
                    }
                }));
                FrameSource::I16(dec_rx)
            }
            WaveFormat::PCM => FrameSource::Pcm(rx),
            _ => bail!(
                "matching server-requested wave format",
                RdpsndNativeErrorKind::UnsupportedFormat,
            ),
        };

        let sample_format = match rx_format.bits_per_sample {
            8 => SampleFormat::U8,
            16 => SampleFormat::I16,
            _ => bail!(
                "only PCM 8/16 bit formats supported",
                RdpsndNativeErrorKind::UnsupportedFormat,
            ),
        };

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| RdpsndNativeError::new("no default output device", RdpsndNativeErrorKind::AudioDevice))?;
        let _supported_configs_range = device.supported_output_configs().map_err(|e| {
            RdpsndNativeError::new("no supported output configs", RdpsndNativeErrorKind::AudioDevice).with_source(e)
        })?;
        let default_config = device.default_output_config().map_err(|e| {
            RdpsndNativeError::new("default output config", RdpsndNativeErrorKind::AudioDevice).with_source(e)
        })?;
        debug!(?default_config);

        let bits_per_sample = rx_format.bits_per_sample;
        let channels = rx_format.n_channels;
        let mut rx = RxBuffer::new(frame_source, volume, bits_per_sample, channels);
        // cpal 0.17: SampleRate is a u32 type alias (not a newtype).
        let config = StreamConfig {
            channels: rx_format.n_channels,
            sample_rate: rx_format.n_samples_per_sec,
            buffer_size: cpal::BufferSize::Default,
        };
        debug!(?config);

        let stream = device
            .build_output_stream_raw(
                &config,
                sample_format,
                move |data, _info: &cpal::OutputCallbackInfo| {
                    let data = data.bytes_mut();
                    rx.fill(data);
                },
                |error| error!(%error),
                None,
            )
            .map_err(|e| {
                RdpsndNativeError::new("building cpal output stream", RdpsndNativeErrorKind::StreamBuild).with_source(e)
            })?;

        // cpal streams start paused; without play() the callback never runs.
        stream.play().map_err(|e| {
            RdpsndNativeError::new("starting cpal output stream", RdpsndNativeErrorKind::StreamBuild).with_source(e)
        })?;

        Ok(Self {
            _dec_thread: dec_thread,
            stream,
        })
    }

    pub fn stream(&self) -> &Stream {
        &self.stream
    }
}

/// Which channel `RxBuffer` is reading from: raw PCM bytes straight off the
/// wire, or samples already decoded by the Opus decode thread. Kept as two
/// natively-typed receivers rather than one channel of a shared element type
/// so the Opus path can send `Vec<i16>` directly with no conversion.
enum FrameSource {
    Pcm(Receiver<Vec<u8>>),
    #[cfg(feature = "opus")]
    I16(Receiver<Vec<i16>>),
}

/// One block of audio pulled off `FrameSource`, still owning its native
/// element type. `as_bytes` and `apply_volume_in_place` view or mutate it
/// through a `bytemuck::cast_slice[_mut]` byte reinterpretation rather than
/// copying into a `Vec<u8>`; that cast is infallible for `i16 -> u8` since it
/// narrows alignment, unlike the reverse.
enum DecodedFrame {
    Pcm(Vec<u8>),
    #[cfg(feature = "opus")]
    I16(Vec<i16>),
}

impl DecodedFrame {
    fn as_bytes(&self) -> &[u8] {
        match self {
            DecodedFrame::Pcm(bytes) => bytes,
            #[cfg(feature = "opus")]
            DecodedFrame::I16(samples) => bytemuck::cast_slice(samples),
        }
    }

    fn apply_volume_in_place(&mut self, bits_per_sample: u16, channels: u16, left: u16, right: u16, phase: &mut usize) {
        match self {
            DecodedFrame::Pcm(bytes) => apply_volume(bytes, bits_per_sample, channels, left, right, phase),
            #[cfg(feature = "opus")]
            DecodedFrame::I16(samples) => {
                apply_volume(
                    bytemuck::cast_slice_mut(samples),
                    bits_per_sample,
                    channels,
                    left,
                    right,
                    phase,
                );
            }
        }
    }
}

struct RxBuffer {
    receiver: FrameSource,
    last: Option<DecodedFrame>,
    idx: usize,
    volume: Arc<AtomicU32>,
    bits_per_sample: u16,
    channels: u16,
    /// Next interleaved sample's channel index for volume L/R assignment.
    volume_sample_phase: usize,
}

impl RxBuffer {
    fn new(receiver: FrameSource, volume: Arc<AtomicU32>, bits_per_sample: u16, channels: u16) -> Self {
        Self {
            receiver,
            last: None,
            idx: 0,
            volume,
            bits_per_sample,
            channels,
            volume_sample_phase: 0,
        }
    }

    fn fill(&mut self, data: &mut [u8]) {
        let mut filled = 0;

        while filled < data.len() {
            if self.last.is_none() {
                let received = match &mut self.receiver {
                    FrameSource::Pcm(rx) => rx.recv_timeout(Duration::from_millis(4000)).map(DecodedFrame::Pcm),
                    #[cfg(feature = "opus")]
                    FrameSource::I16(rx) => rx.recv_timeout(Duration::from_millis(4000)).map(DecodedFrame::I16),
                };
                match received {
                    Ok(mut frame) => {
                        debug!(rx.len = frame.as_bytes().len());
                        let (left, right) = unpack_volume(self.volume.load(Ordering::Relaxed));
                        frame.apply_volume_in_place(
                            self.bits_per_sample,
                            self.channels,
                            left,
                            right,
                            &mut self.volume_sample_phase,
                        );
                        self.last = Some(frame);
                    }
                    Err(error) => {
                        warn!(%error);
                    }
                }
            }

            let Some(ref last) = self.last else {
                trace!("Playback rx underrun");
                return;
            };
            let last = last.as_bytes();

            #[expect(clippy::arithmetic_side_effects)]
            while self.idx < last.len() && filled < data.len() {
                data[filled] = last[self.idx];
                assert!(filled < usize::MAX);
                assert!(self.idx < usize::MAX);
                filled += 1;
                self.idx += 1;
            }

            // If all elements from last have been consumed, clear `self.last`
            if self.idx >= last.len() {
                self.last = None;
                self.idx = 0;
            }
        }
    }
}
