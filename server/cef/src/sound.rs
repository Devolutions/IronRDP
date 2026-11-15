use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};
use ironrdp_rdpsnd::server::{NegotiatedFormat, RdpsndError, RdpsndServerHandler, RdpsndServerMessage};
use ironrdp_server::{ServerEvent, ServerEventSender, SoundServerFactory};
use tokio::sync::Semaphore;
use tokio::sync::mpsc::UnboundedSender;

pub const CAPTURE_SAMPLE_RATE: u32 = 48_000;
pub const CAPTURE_FRAMES_PER_BUFFER: i32 = 960;
const MAX_PACKET_FRAMES: usize = 4_800;
const MAX_IN_FLIGHT_WAVES: usize = 32;
const MAX_QUEUED_WAVES: usize = 4;
const CONFIRM_STALL_RECOVERY: Duration = Duration::from_secs(5);

fn pcm_output_format(channels: u16, sample_rate: u32) -> AudioFormat {
    let block_align = channels * 2;
    AudioFormat {
        format: WaveFormat::PCM,
        n_channels: channels,
        n_samples_per_sec: sample_rate,
        n_avg_bytes_per_sec: sample_rate * u32::from(block_align),
        n_block_align: block_align,
        bits_per_sample: 16,
        data: None,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AudioOutput {
    enabled: bool,
    shared: Arc<Mutex<Shared>>,
}

#[derive(Debug, Default)]
struct Shared {
    events: Option<UnboundedSender<ServerEvent>>,
    active_tab: i32,
    generation: u64,
    stream: Option<ActiveStream>,
}

#[derive(Debug)]
struct ActiveStream {
    generation: u64,
    format: AudioFormat,
    token: Arc<AtomicBool>,
    queued_permits: Arc<Semaphore>,
    unconfirmed: usize,
    blocked_since: Option<Instant>,
}

impl AudioOutput {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            shared: Arc::new(Mutex::new(Shared::default())),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn set_active_tab(&self, tab: i32) {
        if self.enabled {
            self.shared.lock().expect("audio-output state poisoned").active_tab = tab;
        }
    }

    pub(crate) fn disconnected(&self) {
        if !self.enabled {
            return;
        }
        let mut shared = self.shared.lock().expect("audio-output state poisoned");
        shared.generation = shared.generation.wrapping_add(1);
        if let Some(stream) = shared.stream.take() {
            stream.token.store(false, Ordering::Release);
        }
    }

    pub(crate) fn factory(&self) -> Option<Box<dyn SoundServerFactory>> {
        self.enabled
            .then(|| Box::new(self.clone()) as Box<dyn SoundServerFactory>)
    }

    pub(crate) fn push_planar_f32(&self, tab: i32, channels: &[&[f32]], frames: usize, pts_ms: i64) {
        if !self.enabled || channels.is_empty() || frames == 0 {
            return;
        }
        let bounded_frames = frames.min(MAX_PACKET_FRAMES);
        if channels.iter().any(|plane| plane.len() < bounded_frames) {
            return;
        }
        let dispatch = {
            let mut shared = self.shared.lock().expect("audio-output state poisoned");
            if shared.active_tab != tab {
                return;
            }
            let Some(events) = shared.events.clone() else {
                return;
            };
            let Some(stream) = shared.stream.as_mut() else {
                return;
            };
            let queued = MAX_QUEUED_WAVES.saturating_sub(stream.queued_permits.available_permits());
            if MAX_IN_FLIGHT_WAVES <= stream.unconfirmed.saturating_add(queued) {
                let now = Instant::now();
                let since = *stream.blocked_since.get_or_insert(now);
                if now.duration_since(since) < CONFIRM_STALL_RECOVERY {
                    tracing::trace!("Dropping browser audio packet while RDPSND blocks are unconfirmed");
                    return;
                }
                // Recover if client confirmations stalled past the 16-bit RDPSND timestamp horizon.
                stream.unconfirmed = 0;
                stream.blocked_since = None;
            } else {
                stream.blocked_since = None;
            }
            let Ok(permit) = Arc::clone(&stream.queued_permits).try_acquire_owned() else {
                tracing::trace!("Dropping browser audio packet while RDPSND event queue is full");
                return;
            };
            let pcm = encode_planar_f32_to_pcm16(channels, bounded_frames, &stream.format);
            if pcm.is_empty() {
                return;
            }
            let timestamp = u32::try_from(pts_ms.max(0) & i64::from(u32::MAX)).unwrap_or(0);
            (events, pcm, timestamp, permit, Arc::clone(&stream.token))
        };
        let (events, pcm, timestamp, permit, token) = dispatch;
        if events
            .send(ServerEvent::RdpsndWithValidity {
                message: RdpsndServerMessage::Wave(pcm, timestamp),
                permit: Some(permit),
                is_current: Box::new(move || token.load(Ordering::Acquire)),
            })
            .is_err()
        {
            self.disconnected();
        }
    }
}

impl ServerEventSender for AudioOutput {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        self.shared.lock().expect("audio-output state poisoned").events = Some(sender);
    }
}

impl SoundServerFactory for AudioOutput {
    fn build_backend(&self) -> Box<dyn RdpsndServerHandler> {
        let mut shared = self.shared.lock().expect("audio-output state poisoned");
        shared.generation = shared.generation.wrapping_add(1);
        if let Some(stream) = shared.stream.take() {
            stream.token.store(false, Ordering::Release);
        }
        let generation = shared.generation;
        drop(shared);
        Box::new(Backend {
            shared: Arc::clone(&self.shared),
            generation,
            formats: vec![
                pcm_output_format(2, CAPTURE_SAMPLE_RATE),
                pcm_output_format(1, CAPTURE_SAMPLE_RATE),
                pcm_output_format(2, 44_100),
            ],
        })
    }
}

#[derive(Debug)]
struct Backend {
    shared: Arc<Mutex<Shared>>,
    generation: u64,
    formats: Vec<AudioFormat>,
}

impl RdpsndServerHandler for Backend {
    fn get_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn choose_format<'a>(&mut self, common: &'a [NegotiatedFormat]) -> Option<&'a NegotiatedFormat> {
        common.first()
    }

    fn start(&mut self, format: &NegotiatedFormat) -> Result<(), Box<dyn RdpsndError>> {
        let mut shared = self.shared.lock().expect("audio-output state poisoned");
        if shared.generation == self.generation {
            if let Some(previous) = shared.stream.take() {
                previous.token.store(false, Ordering::Release);
            }
            shared.stream = Some(ActiveStream {
                generation: self.generation,
                format: format.format().clone(),
                token: Arc::new(AtomicBool::new(true)),
                queued_permits: Arc::new(Semaphore::new(MAX_QUEUED_WAVES)),
                unconfirmed: 0,
                blocked_since: None,
            });
        }
        Ok(())
    }

    fn unconfirmed_waves_changed(&mut self, unconfirmed: usize) {
        let mut shared = self.shared.lock().expect("audio-output state poisoned");
        if let Some(stream) = shared.stream.as_mut()
            && stream.generation == self.generation
        {
            if unconfirmed < stream.unconfirmed {
                stream.blocked_since = None;
            }
            stream.unconfirmed = unconfirmed;
        }
    }

    fn stop(&mut self) {
        let mut shared = self.shared.lock().expect("audio-output state poisoned");
        if shared.stream.as_ref().is_some_and(|s| s.generation == self.generation)
            && let Some(stream) = shared.stream.take()
        {
            stream.token.store(false, Ordering::Release);
        }
    }
}

fn sample_to_i16(sample: f32) -> i16 {
    if !sample.is_finite() {
        return 0;
    }
    let scaled = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round();
    scaled as i16
}

fn encode_planar_f32_to_pcm16(planes: &[&[f32]], in_frames: usize, target: &AudioFormat) -> Vec<u8> {
    let out_channels = usize::from(target.n_channels);
    if !(out_channels == 1 || out_channels == 2) || target.bits_per_sample != 16 || target.format != WaveFormat::PCM {
        return Vec::new();
    }
    let out_rate = target.n_samples_per_sec.max(1);
    let out_frames = if out_rate == CAPTURE_SAMPLE_RATE {
        in_frames
    } else {
        usize::try_from((u64::try_from(in_frames).unwrap_or(0) * u64::from(out_rate)) / u64::from(CAPTURE_SAMPLE_RATE))
            .unwrap_or(0)
    };
    if out_frames == 0 {
        return Vec::new();
    }
    let mut pcm = Vec::with_capacity(out_frames * out_channels * 2);
    for out_idx in 0..out_frames {
        let src_idx = if out_rate == CAPTURE_SAMPLE_RATE {
            out_idx
        } else {
            ((out_idx * in_frames) / out_frames).min(in_frames - 1)
        };
        let left = planes[0][src_idx];
        let right = planes.get(1).map_or(left, |plane| plane[src_idx]);
        if out_channels == 2 {
            pcm.extend_from_slice(&sample_to_i16(left).to_le_bytes());
            pcm.extend_from_slice(&sample_to_i16(right).to_le_bytes());
        } else {
            let mono = 0.5 * (left + right);
            pcm.extend_from_slice(&sample_to_i16(mono).to_le_bytes());
        }
    }
    pcm
}

#[cfg(test)]
mod tests {
    use ironrdp_core::encode_vec;
    use ironrdp_rdpsnd::pdu::{ClientAudioFormatPdu, ClientAudioOutputPdu, QualityMode, TrainingConfirmPdu, Version};
    use ironrdp_rdpsnd::server::RdpsndServer;
    use ironrdp_svc::SvcProcessor as _;
    use tokio::sync::mpsc::unbounded_channel;

    use super::*;

    fn negotiate(output: &mut AudioOutput, channels: u16) -> RdpsndServer {
        let backend = output.build_backend();
        let mut server = RdpsndServer::new(backend);
        server.start().unwrap();
        let client_formats = ClientAudioOutputPdu::AudioFormat(ClientAudioFormatPdu {
            version: Version::V8,
            flags: ironrdp_rdpsnd::pdu::AudioFormatFlags::ALIVE,
            formats: vec![pcm_output_format(channels, CAPTURE_SAMPLE_RATE)],
            volume_left: 0xFFFF,
            volume_right: 0xFFFF,
            pitch: 0,
            dgram_port: 0,
        });
        server.process(&encode_vec(&client_formats).unwrap()).unwrap();
        let quality = ClientAudioOutputPdu::QualityMode(ironrdp_rdpsnd::pdu::QualityModePdu {
            quality_mode: QualityMode::Dynamic,
        });
        server.process(&encode_vec(&quality).unwrap()).unwrap();
        let training = ClientAudioOutputPdu::TrainingConfirm(TrainingConfirmPdu {
            timestamp: 1,
            pack_size: 8,
        });
        server.process(&encode_vec(&training).unwrap()).unwrap();
        server
    }

    // oracle: mutant swapping stereo channels, mis-scaling normalized floats, or propagating NaN.
    #[test]
    fn planar_float_packets_encode_interleaved_little_endian_pcm16_for_active_tab() {
        let mut output = AudioOutput::new(true);
        let (sender, mut events) = unbounded_channel();
        output.set_sender(sender);
        output.set_active_tab(1);
        let _server = negotiate(&mut output, 2);

        output.push_planar_f32(2, &[&[1.0], &[1.0]], 1, 100);
        assert!(events.try_recv().is_err(), "inactive tab audio was forwarded");

        output.push_planar_f32(1, &[&[1.0, f32::NAN], &[-1.0, 0.0]], 2, 250);
        let ServerEvent::RdpsndWithValidity {
            message: RdpsndServerMessage::Wave(bytes, ts),
            is_current,
            ..
        } = events.try_recv().expect("RDPSND wave")
        else {
            panic!("expected RDPSND wave event");
        };
        assert!(is_current());
        output.disconnected();
        assert!(!is_current(), "disconnect must invalidate queued RDPSND waves");
        assert_eq!(ts, 250);
        let samples: Vec<i16> = bytes
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        assert_eq!(samples, [i16::MAX, -i16::MAX, 0, 0]);
    }

    // oracle: mutant queueing unbounded RDPSND waves without client WaveConfirm or double-counting duplicate/forged confirms.
    #[test]
    fn unconfirmed_waves_cap_at_limit_and_resume_after_valid_wave_confirm() {
        let mut output = AudioOutput::new(true);
        let (sender, mut events) = unbounded_channel();
        output.set_sender(sender);
        output.set_active_tab(1);
        let mut server = negotiate(&mut output, 1);

        for i in 0..MAX_IN_FLIGHT_WAVES {
            output.push_planar_f32(1, &[&[0.5], &[0.5]], 1, (100 + i) as i64);
            let ServerEvent::RdpsndWithValidity {
                message: RdpsndServerMessage::Wave(data, ts),
                ..
            } = events.try_recv().expect("queued wave")
            else {
                panic!("expected wave");
            };
            server.wave(data, ts).unwrap();
        }
        output.push_planar_f32(1, &[&[0.5], &[0.5]], 1, 500);
        assert!(events.try_recv().is_err(), "exceeded MAX_IN_FLIGHT_WAVES");

        // Stale confirm (timestamp before send timestamp 100) must not release a slot.
        let stale = ClientAudioOutputPdu::WaveConfirm(ironrdp_rdpsnd::pdu::WaveConfirmPdu {
            timestamp: 50,
            block_no: 0,
        });
        server.process(&encode_vec(&stale).unwrap()).unwrap();
        output.push_planar_f32(1, &[&[0.5], &[0.5]], 1, 501);
        assert!(events.try_recv().is_err(), "stale confirm released a wave slot");

        let confirm = ClientAudioOutputPdu::WaveConfirm(ironrdp_rdpsnd::pdu::WaveConfirmPdu {
            timestamp: 110,
            block_no: 0,
        });
        // Windows and FreeRDP send two WaveConfirms per block; the second must not free an extra slot.
        server.process(&encode_vec(&confirm).unwrap()).unwrap();
        server.process(&encode_vec(&confirm).unwrap()).unwrap();

        output.push_planar_f32(1, &[&[0.5], &[0.5]], 1, 999);
        let ServerEvent::RdpsndWithValidity {
            message: RdpsndServerMessage::Wave(data, ts),
            ..
        } = events.try_recv().expect("resumed wave")
        else {
            panic!("expected resumed wave");
        };
        server.wave(data, ts).unwrap();
        output.push_planar_f32(1, &[&[0.5], &[0.5]], 1, 1000);
        assert!(events.try_recv().is_err(), "duplicate confirm freed an extra wave slot");
    }
}
