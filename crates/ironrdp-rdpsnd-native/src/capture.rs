use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{Stream, StreamConfig};
use ironrdp_rdpeai::client::{AudioPacketSink, RdpeaiCaptureHandler};
use ironrdp_rdpeai::pdu::{MAX_DATA_PACKET_SIZE, OpenReplyPdu, pcm_format};
use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};
use tracing::{debug, error, warn};

/// Upper bound for waiting on CPAL input stream startup from the session task.
///
/// Open must report a real HRESULT, but the client session loop is single-threaded.
/// Keep this short so a slow/hung device does not stall the whole RDP session.
const STREAM_START_TIMEOUT: Duration = Duration::from_millis(250);

/// `KSDATAFORMAT_SUBTYPE_PCM` (`{00000001-0000-0010-8000-00aa00389b71}`).
///
/// Layout is the on-wire GUID bytes used in `WAVEFORMATEXTENSIBLE.SubFormat`.
const KSDATAFORMAT_SUBTYPE_PCM: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, // Data1
    0x00, 0x00, // Data2
    0x10, 0x00, // Data3
    0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71, // Data4
];

/// CPAL-backed MS-RDPEAI capture handler.
///
/// This backend captures PCM only and ships capture-sized PCM Data PDUs.
/// Encode formats that differ from the Open capture WAVEFORMATEX cause a
/// capture-stream restart so FormatChange can always be confirmed.
pub struct RdpeaiCaptureBackend {
    formats: Vec<AudioFormat>,
    stream_handle: Option<JoinHandle<()>>,
    stream_ended: Arc<AtomicBool>,
    /// Capture WAVEFORMATEX established by a successful Open (or last restart).
    open_capture_format: Option<AudioFormat>,
    /// Shared sink + packet size for the capture callback thread.
    sink_state: Arc<Mutex<Option<SinkState>>>,
}

struct SinkState {
    sink: AudioPacketSink,
    packet_size: usize,
    buffer: Vec<u8>,
}

impl Default for RdpeaiCaptureBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl RdpeaiCaptureBackend {
    pub fn new() -> Self {
        Self {
            formats: default_capture_formats(),
            stream_handle: None,
            stream_ended: Arc::new(AtomicBool::new(false)),
            open_capture_format: None,
            sink_state: Arc::new(Mutex::new(None)),
        }
    }

    /// Stop the worker thread without clearing an already-installed sink.
    fn stop_worker(&mut self) {
        if let Some(handle) = self.stream_handle.take() {
            self.stream_ended.store(true, Ordering::Relaxed);
            handle.thread().unpark();
            if let Err(err) = handle.join() {
                error!(?err, "Failed to join capture stream thread");
            }
        }
    }

    fn clear_sink(&mut self) {
        if let Ok(mut guard) = self.sink_state.lock() {
            *guard = None;
        }
    }

    fn start_stream(&mut self, format: &AudioFormat) -> Result<(), String> {
        self.stop_worker();

        if !is_pcm_capture_format(format) {
            return Err(format!("unsupported capture wave format: {:?}", format.format));
        }
        // MS-RDPEAI Data PDU size is fixed at 16-bit samples (nChannels * 2 * FramesPerPacket).
        if format.bits_per_sample != 16 {
            return Err(format!(
                "unsupported capture bits_per_sample: {} (PCM capture requires 16-bit)",
                format.bits_per_sample
            ));
        }

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "no default input device".to_owned())?;

        let config = StreamConfig {
            channels: format.n_channels,
            sample_rate: format.n_samples_per_sec,
            buffer_size: cpal::BufferSize::Default,
        };

        let stream_ended = Arc::clone(&self.stream_ended);
        let sink_state = Arc::clone(&self.sink_state);
        self.stream_ended.store(false, Ordering::Relaxed);

        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);

        self.stream_handle = Some(thread::spawn(move || {
            let stream = match build_input_stream(&device, &config, Arc::clone(&sink_state)) {
                Ok(stream) => stream,
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
            };
            if let Err(error) = stream.play() {
                let _ = ready_tx.send(Err(error.to_string()));
                return;
            }
            let _ = ready_tx.send(Ok(()));
            debug!("AUDIO_INPUT capture stream running");
            while !stream_ended.load(Ordering::Relaxed) {
                thread::park();
            }
            drop(stream);
            debug!("AUDIO_INPUT capture stream stopped");
        }));

        match ready_rx.recv_timeout(STREAM_START_TIMEOUT) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.stop_worker();
                Err(error)
            }
            Err(error) => {
                self.stop_worker();
                Err(format!("capture stream startup timed out: {error}"))
            }
        }
    }
}

impl Drop for RdpeaiCaptureBackend {
    fn drop(&mut self) {
        self.close();
    }
}

impl RdpeaiCaptureHandler for RdpeaiCaptureBackend {
    fn supported_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn open(
        &mut self,
        capture_format: &AudioFormat,
        encode_format: &AudioFormat,
        packet_size: usize,
        sink: AudioPacketSink,
    ) -> i32 {
        // PCM-only backend: encode must match capture rate/channels so Data PDUs stay decodable.
        if !same_pcm_params(capture_format, encode_format) {
            warn!(
                ?capture_format,
                ?encode_format,
                "Refusing capture open: encode format differs from capture (PCM-only backend)"
            );
            return OpenReplyPdu::E_FAIL;
        }

        if packet_size == 0 || packet_size > MAX_DATA_PACKET_SIZE {
            warn!(packet_size, "Refusing capture open with invalid packet size");
            return OpenReplyPdu::E_FAIL;
        }

        let mut buffer = Vec::new();
        if buffer.try_reserve_exact(packet_size.saturating_mul(2)).is_err() {
            warn!(packet_size, "Refusing capture open: packet buffer allocation failed");
            return OpenReplyPdu::E_FAIL;
        }

        {
            let Ok(mut guard) = self.sink_state.lock() else {
                return OpenReplyPdu::E_FAIL;
            };
            *guard = Some(SinkState {
                sink,
                packet_size,
                buffer,
            });
        }

        match self.start_stream(capture_format) {
            Ok(()) => {
                self.open_capture_format = Some(capture_format.clone());
                OpenReplyPdu::S_OK
            }
            Err(error) => {
                warn!(%error, "AUDIO_INPUT capture open failed");
                self.clear_sink();
                self.open_capture_format = None;
                OpenReplyPdu::E_FAIL
            }
        }
    }

    fn set_format(&mut self, encode_format: &AudioFormat, packet_size: usize) -> bool {
        // FormatChange switches encoding. Capture packet size stays at the Open value.
        let _ = packet_size;
        if self.open_capture_format.is_none() {
            return false;
        }
        if !is_pcm_capture_format(encode_format) || encode_format.bits_per_sample != 16 {
            warn!(
                ?encode_format,
                "Rejecting FormatChange: encode format is not 16-bit PCM"
            );
            return false;
        }

        if self
            .open_capture_format
            .as_ref()
            .is_some_and(|capture| same_pcm_params(capture, encode_format))
        {
            return self.sink_state.lock().map(|guard| guard.is_some()).unwrap_or(false);
        }

        // Restart capture at the newly selected PCM params so confirmation is honest.
        match self.start_stream(encode_format) {
            Ok(()) => {
                self.open_capture_format = Some(encode_format.clone());
                true
            }
            Err(error) => {
                warn!(%error, "AUDIO_INPUT capture restart for FormatChange failed");
                false
            }
        }
    }

    fn close(&mut self) {
        self.stop_worker();
        self.clear_sink();
        self.open_capture_format = None;
    }
}

fn default_capture_formats() -> Vec<AudioFormat> {
    // Prefer common VoIP / Windows mic rates, mono first.
    vec![
        pcm_format(1, 48000, 16),
        pcm_format(1, 44100, 16),
        pcm_format(1, 22050, 16),
        pcm_format(1, 16000, 16),
        pcm_format(1, 8000, 16),
        pcm_format(2, 48000, 16),
        pcm_format(2, 44100, 16),
        pcm_format(2, 16000, 16),
    ]
}

/// True when `format` is ordinary 16-bit PCM or `WAVE_FORMAT_EXTENSIBLE` PCM subtype.
#[doc(hidden)]
pub fn is_pcm_capture_format(format: &AudioFormat) -> bool {
    match format.format {
        WaveFormat::PCM => true,
        WaveFormat::EXTENSIBLE => is_extensible_pcm_subtype(format),
        _ => false,
    }
}

fn is_extensible_pcm_subtype(format: &AudioFormat) -> bool {
    // WAVEFORMATEXTENSIBLE extras: wValidBitsPerSample (2) + dwChannelMask (4) + SubFormat (16).
    const SUBTYPE_OFFSET: usize = 2 /* valid bits */ + 4 /* channel mask */;
    const EXTENSIBLE_EXTRA: usize = SUBTYPE_OFFSET + 16;
    let Some(data) = format.data.as_deref() else {
        return false;
    };
    if data.len() < EXTENSIBLE_EXTRA {
        return false;
    }
    data[SUBTYPE_OFFSET..SUBTYPE_OFFSET + 16] == KSDATAFORMAT_SUBTYPE_PCM
}

/// Compare PCM capture parameters, treating plain PCM and extensible PCM subtype as equivalent.
fn same_pcm_params(a: &AudioFormat, b: &AudioFormat) -> bool {
    is_pcm_capture_format(a)
        && is_pcm_capture_format(b)
        && a.n_channels == b.n_channels
        && a.n_samples_per_sec == b.n_samples_per_sec
        && a.bits_per_sample == b.bits_per_sample
}

/// Split complete fixed-size capture packets out of `buffer`.
///
/// Used by the CPAL callback path and exposed for unit tests (`test = false` on this crate).
#[doc(hidden)]
pub fn take_capture_packets(buffer: &mut Vec<u8>, packet_size: usize) -> Vec<Vec<u8>> {
    if packet_size == 0 {
        return Vec::new();
    }
    let mut packets = Vec::new();
    while buffer.len() >= packet_size {
        let rest = buffer.split_off(packet_size);
        let packet = core::mem::replace(buffer, rest);
        packets.push(packet);
    }
    packets
}

fn append_i16_le(buffer: &mut Vec<u8>, samples: &[i16]) {
    let extra = samples.len().saturating_mul(2);
    buffer.reserve(extra);
    for sample in samples {
        buffer.extend_from_slice(&sample.to_le_bytes());
    }
}

fn f32_to_i16(sample: f32) -> i16 {
    let clamped = sample.clamp(-1.0, 1.0);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::as_conversions,
        reason = "f32 sample scaled into i16 PCM range after clamp"
    )]
    {
        (clamped * 32767.0) as i16
    }
}

fn build_input_stream(
    device: &cpal::Device,
    config: &StreamConfig,
    sink_state: Arc<Mutex<Option<SinkState>>>,
) -> Result<Stream, String> {
    let err_fn = |error| error!(%error, "CPAL input stream error");

    // Prefer native i16; fall back to f32→i16 so CoreAudio / float WASAPI hosts still work.
    let i16_result = device.build_input_stream(
        config,
        {
            let sink_state = Arc::clone(&sink_state);
            move |data: &[i16], _| {
                push_i16_samples(&sink_state, data);
            }
        },
        err_fn,
        None,
    );

    match i16_result {
        Ok(stream) => Ok(stream),
        Err(i16_error) => {
            debug!(%i16_error, "I16 input stream unsupported; trying F32");
            device
                .build_input_stream(
                    config,
                    move |data: &[f32], _| {
                        push_f32_samples(&sink_state, data);
                    },
                    err_fn,
                    None,
                )
                .map_err(|f32_error| format!("i16: {i16_error}; f32: {f32_error}"))
        }
    }
}

fn push_i16_samples(sink_state: &Arc<Mutex<Option<SinkState>>>, samples: &[i16]) {
    let (mut sink, packets) = {
        let Ok(mut guard) = sink_state.lock() else {
            return;
        };
        let Some(state) = guard.as_mut() else {
            return;
        };
        append_i16_le(&mut state.buffer, samples);
        let packets = take_capture_packets(&mut state.buffer, state.packet_size);
        // Move sink out so the uplink encode runs without holding sink_state.
        let sink = core::mem::replace(&mut state.sink, Box::new(|_| {}));
        (sink, packets)
    };

    for packet in packets {
        sink(packet);
    }

    if let Ok(mut guard) = sink_state.lock() {
        if let Some(state) = guard.as_mut() {
            state.sink = sink;
        }
    }
}

fn push_f32_samples(sink_state: &Arc<Mutex<Option<SinkState>>>, samples: &[f32]) {
    let (mut sink, packets) = {
        let Ok(mut guard) = sink_state.lock() else {
            return;
        };
        let Some(state) = guard.as_mut() else {
            return;
        };
        state.buffer.reserve(samples.len().saturating_mul(2));
        for sample in samples {
            state.buffer.extend_from_slice(&f32_to_i16(*sample).to_le_bytes());
        }
        let packets = take_capture_packets(&mut state.buffer, state.packet_size);
        let sink = core::mem::replace(&mut state.sink, Box::new(|_| {}));
        (sink, packets)
    };

    for packet in packets {
        sink(packet);
    }

    if let Ok(mut guard) = sink_state.lock() {
        if let Some(state) = guard.as_mut() {
            state.sink = sink;
        }
    }
}
