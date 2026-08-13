use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{SampleFormat, Stream, StreamConfig};
use ironrdp_rdpeai::client::{AudioPacketSink, RdpeaiCaptureHandler};
use ironrdp_rdpeai::pdu::{MAX_DATA_PACKET_SIZE, OpenReplyPdu, pcm_format};
use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};
use tracing::{debug, error, warn};

/// CPAL-backed MS-RDPEAI capture handler.
///
/// This backend captures PCM only and ships capture-sized PCM Data PDUs.
/// Encode formats that differ from the Open capture WAVEFORMATEX are rejected
/// until a real encoder is wired.
pub struct RdpeaiCaptureBackend {
    formats: Vec<AudioFormat>,
    stream_handle: Option<JoinHandle<()>>,
    stream_ended: Arc<AtomicBool>,
    /// Capture WAVEFORMATEX established by a successful Open.
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

        if format.format != WaveFormat::PCM {
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

        let sample_format = SampleFormat::I16;

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
            let stream = match build_input_stream(&device, &config, sample_format, Arc::clone(&sink_state)) {
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

        match ready_rx.recv_timeout(Duration::from_secs(5)) {
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
        // PCM-only backend: encode must match capture so Data PDUs stay decodable.
        if !encode_format.matches_for_negotiation(capture_format) {
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
        // FormatChange switches encoding only. Capture WAVEFORMATEX and Data PDU size
        // stay at the values established by Open; do not restart the input stream.
        let _ = packet_size;
        let Some(capture_format) = self.open_capture_format.as_ref() else {
            return false;
        };
        if !encode_format.matches_for_negotiation(capture_format) {
            warn!(
                ?capture_format,
                ?encode_format,
                "Rejecting FormatChange: encode format differs from open capture (PCM-only backend)"
            );
            return false;
        }
        self.sink_state.lock().map(|guard| guard.is_some()).unwrap_or(false)
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

fn build_input_stream(
    device: &cpal::Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    sink_state: Arc<Mutex<Option<SinkState>>>,
) -> Result<Stream, String> {
    let err_fn = |error| error!(%error, "CPAL input stream error");

    match sample_format {
        SampleFormat::I16 => device
            .build_input_stream(
                config,
                move |data: &[i16], _| {
                    let mut bytes = Vec::with_capacity(data.len().saturating_mul(2));
                    for sample in data {
                        bytes.extend_from_slice(&sample.to_le_bytes());
                    }
                    push_samples(&sink_state, &bytes);
                },
                err_fn,
                None,
            )
            .map_err(|e| e.to_string()),
        other => Err(format!("unsupported CPAL sample format: {other:?}")),
    }
}

fn push_samples(sink_state: &Arc<Mutex<Option<SinkState>>>, samples: &[u8]) {
    let Ok(mut guard) = sink_state.lock() else {
        return;
    };
    let Some(state) = guard.as_mut() else {
        return;
    };
    state.buffer.extend_from_slice(samples);
    while state.buffer.len() >= state.packet_size {
        let packet: Vec<u8> = state.buffer.drain(..state.packet_size).collect();
        (state.sink)(packet);
    }
}
