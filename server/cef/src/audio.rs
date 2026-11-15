use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ironrdp_rdpeai::pdu::pcm_format;
use ironrdp_rdpeai::server::{RdpeaiServer, RdpeaiServerBackend, RdpeaiServerMessage};
use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};
use ironrdp_server::{ServerEvent, ServerEventSender};
use tokio::sync::mpsc::UnboundedSender;

use crate::rdp::ChannelGuard;
use crate::remote_devices::RemoteMedia;

#[derive(Clone)]
pub(crate) struct AudioInput {
    shared: Arc<Mutex<Shared>>,
    media: RemoteMedia,
}

#[derive(Default)]
struct Shared {
    events: Option<UnboundedSender<ServerEvent>>,
    connected: bool,
    connection_token: Arc<AtomicBool>,
    active: bool,
    generation: u64,
    channel_requested: bool,
    channel_id: Option<u32>,
    token: Option<ChannelGuard>,
    open_token: Option<Arc<AtomicBool>>,
    negotiated: bool,
    opening: bool,
    opened: bool,
    attempted: bool,
}

impl AudioInput {
    pub(crate) fn new(media: RemoteMedia) -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared::default())),
            media,
        }
    }

    pub(crate) fn connected(&self) {
        let mut shared = self.shared.lock().expect("audio-input state poisoned");
        if !shared.connected {
            shared.connection_token = Arc::new(AtomicBool::new(true));
            shared.connected = true;
        }
    }

    pub(crate) fn disconnected(&self) {
        let mut shared = self.shared.lock().expect("audio-input state poisoned");
        shared.connection_token.store(false, Ordering::Release);
        shared.connected = false;
        shared.active = false;
        shared.generation = shared.generation.wrapping_add(1);
        shared.channel_requested = false;
        shared.channel_id = None;
        shared.token.take().inspect(ChannelGuard::invalidate);
        if let Some(token) = shared.open_token.take() {
            token.store(false, Ordering::Release);
        }
        shared.negotiated = false;
        shared.opening = false;
        shared.opened = false;
        shared.attempted = false;
        self.media.reset_microphone();
    }

    pub(crate) fn tick(&self) {
        let capture_active = 0 < self.media.microphone_streams();
        let (events, create_generation, mut queued) = {
            let mut shared = self.shared.lock().expect("audio-input state poisoned");
            let Some(events) = shared.events.clone() else {
                return;
            };
            if !shared.connected {
                if !capture_active {
                    shared.active = false;
                }
                return;
            }

            let mut queued = Vec::new();
            let mut create_generation = None;
            if capture_active {
                if !shared.active {
                    shared.active = true;
                    shared.generation = shared.generation.wrapping_add(1);
                }
                if shared.channel_id.is_none() && !shared.channel_requested {
                    shared.channel_requested = true;
                    create_generation = Some(shared.generation);
                }
                if let Some(channel_id) = shared.channel_id
                    && shared.negotiated
                    && !shared.opening
                    && !shared.opened
                    && !shared.attempted
                {
                    shared.opening = true;
                    shared.attempted = true;
                    let open_token = Arc::clone(shared.open_token.as_ref().expect("channel has open token"));
                    queued.push(ServerEvent::RdpeaiWithValidity {
                        message: RdpeaiServerMessage::OpenOnChannel {
                            channel_id,
                            frames_per_packet: 480,
                            initial_format: 0,
                            capture_format: pcm_format(1, 48_000, 16),
                        },
                        is_current: Box::new(move || open_token.load(Ordering::Acquire)),
                    });
                }
            } else {
                if shared.active {
                    shared.active = false;
                    shared.generation = shared.generation.wrapping_add(1);
                }
                if let Some(token) = shared.open_token.take() {
                    token.store(false, Ordering::Release);
                }
                if let Some(channel_id) = shared.channel_id.take()
                    && let Some(token) = shared.token.take()
                {
                    queued.push(token.close(channel_id));
                }
                shared.channel_requested = false;
                shared.negotiated = false;
                shared.opening = false;
                shared.opened = false;
                shared.attempted = false;
            }
            (events, create_generation, queued)
        };
        if let Some(generation) = create_generation {
            queued.insert(
                0,
                create_channel_event(Arc::clone(&self.shared), self.media, generation),
            );
        }

        for event in queued {
            if let Err(error) = events.send(event) {
                tracing::debug!(%error, "RDP server stopped before an AUDIO_INPUT event was sent");
                self.disconnected();
                break;
            }
        }
    }
}

impl ServerEventSender for AudioInput {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        self.shared.lock().expect("audio-input state poisoned").events = Some(sender);
    }
}

struct Backend {
    shared: Arc<Mutex<Shared>>,
    media: RemoteMedia,
    channel_id: u32,
    generation: u64,
    token: ChannelGuard,
    open_token: Arc<AtomicBool>,
    formats: Vec<AudioFormat>,
}

impl RdpeaiServerBackend for Backend {
    fn supported_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn on_formats_negotiated(&mut self, negotiated: &[AudioFormat]) {
        let mut shared = self.shared.lock().expect("audio-input state poisoned");
        if shared.channel_id == Some(self.channel_id) && shared.generation == self.generation {
            shared.channel_requested = false;
            shared.negotiated = !negotiated.is_empty();
        }
    }

    fn on_open_reply(&mut self, result: i32) {
        let mut shared = self.shared.lock().expect("audio-input state poisoned");
        if shared.channel_id == Some(self.channel_id) && shared.generation == self.generation {
            shared.opening = false;
            shared.opened = result >= 0;
        }
    }

    fn on_audio_data(&mut self, format: &AudioFormat, data: &[u8]) {
        if format.format != WaveFormat::PCM
            || format.n_channels != 1
            || format.n_samples_per_sec != 48_000
            || format.bits_per_sample != 16
        {
            tracing::warn!(?format, "Dropping unsupported AUDIO_INPUT format");
            return;
        }
        let samples = match decode_pcm16(data) {
            Ok(samples) => samples,
            Err(error) => {
                tracing::warn!(%error, "Dropping malformed AUDIO_INPUT packet");
                return;
            }
        };
        let shared = self.shared.lock().expect("audio-input state poisoned");
        if !(shared.active && shared.generation == self.generation && shared.channel_id == Some(self.channel_id)) {
            return;
        }
        if let Err(error) = self.media.push_microphone(&samples, 1, 48_000) {
            tracing::warn!(%error, "CEF rejected an AUDIO_INPUT packet");
        }
    }

    fn on_close(&mut self) {
        self.token.invalidate();
        self.open_token.store(false, Ordering::Release);
        let mut shared = self.shared.lock().expect("audio-input state poisoned");
        if shared.channel_id == Some(self.channel_id) && shared.generation == self.generation {
            shared.generation = shared.generation.wrapping_add(1);
            shared.channel_requested = false;
            shared.channel_id = None;
            shared.token = None;
            shared.negotiated = false;
            shared.opening = false;
            shared.opened = false;
            shared.attempted = false;
        }
    }
}

struct UnstartedAudioChannel {
    shared: Arc<Mutex<Shared>>,
    generation: u64,
    armed: bool,
}

impl UnstartedAudioChannel {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for UnstartedAudioChannel {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut state = self.shared.lock().expect("audio-input state poisoned");
        if state.connected && state.active && state.generation == self.generation && state.channel_id.is_none() {
            state.channel_requested = false;
        }
    }
}

fn create_channel_event(shared: Arc<Mutex<Shared>>, media: RemoteMedia, generation: u64) -> ServerEvent {
    let mut unstarted = UnstartedAudioChannel {
        shared: Arc::clone(&shared),
        generation,
        armed: true,
    };
    ServerEvent::CreateDynamicChannel(Box::new(move |channel_id| {
        unstarted.disarm();
        let (token, open_token, current, events) = {
            let mut state = shared.lock().expect("audio-input state poisoned");
            let token = ChannelGuard::new(&state.connection_token);
            let current = state.connected
                && state.active
                && state.generation == generation
                && state.channel_requested
                && state.channel_id.is_none();
            if current {
                state.channel_id = Some(channel_id);
                state.token = Some(token.clone());
                let open_token = Arc::new(AtomicBool::new(true));
                state.open_token = Some(Arc::clone(&open_token));
                (token, open_token, current, state.events.clone())
            } else {
                (token, Arc::new(AtomicBool::new(false)), current, state.events.clone())
            }
        };
        if !current {
            if let Some(events) = events {
                let _ = events.send(token.close(channel_id));
            }
        }
        Box::new(RdpeaiServer::new(Box::new(Backend {
            shared,
            media,
            channel_id,
            generation,
            token,
            open_token,
            formats: vec![pcm_format(1, 48_000, 16)],
        })))
    }))
}

fn decode_pcm16(data: &[u8]) -> anyhow::Result<Vec<i16>> {
    anyhow::ensure!(data.len().is_multiple_of(2), "PCM packet has an incomplete sample");
    Ok(data
        .chunks_exact(2)
        .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use std::ffi::{c_float, c_int};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Condvar, Mutex, mpsc};
    use std::thread;
    use std::time::Duration;

    use ironrdp_core::encode_vec;
    use ironrdp_rdpeai::pdu::{DataPdu, FormatChangePdu, FormatsPdu, OpenReplyPdu, RdpeaiPdu, Version, VersionPdu};
    use tokio::sync::mpsc::unbounded_channel;

    use super::*;

    static CAPTURING: AtomicBool = AtomicBool::new(true);
    static AUDIO_RESET_COUNT: AtomicUsize = AtomicUsize::new(0);
    static AUDIO: Mutex<Vec<i16>> = Mutex::new(Vec::new());
    static TEST_LOCK: Mutex<()> = Mutex::new(());
    static PUSH_GATE: (Mutex<(bool, bool)>, Condvar) = (Mutex::new((false, false)), Condvar::new());

    unsafe extern "C" fn camera_format(_: *mut c_int, _: *mut c_int, _: *mut c_float) -> c_int {
        0
    }

    unsafe extern "C" fn push_frame(_: c_int, _: c_int, _: c_int, _: *const u8, _: usize, _: i64) -> c_int {
        0
    }

    unsafe extern "C" fn microphone_active() -> c_int {
        c_int::from(CAPTURING.load(Ordering::Relaxed))
    }

    unsafe extern "C" fn push_audio(samples: *const i16, frames: usize, channels: c_int, _: c_int) -> c_int {
        let count = frames * usize::try_from(channels).unwrap();
        // SAFETY: RemoteMedia supplies `frames * channels` readable samples.
        let samples = unsafe { std::slice::from_raw_parts(samples, count) };
        AUDIO.lock().unwrap().extend_from_slice(samples);
        0
    }

    unsafe extern "C" fn reset_audio() {
        AUDIO_RESET_COUNT.fetch_add(1, Ordering::Relaxed);
    }

    unsafe extern "C" fn blocking_push_audio(
        samples: *const i16,
        frames: usize,
        channels: c_int,
        sample_rate: c_int,
    ) -> c_int {
        let (lock, changed) = &PUSH_GATE;
        let mut gate = lock.lock().unwrap();
        gate.0 = true;
        changed.notify_all();
        while !gate.1 {
            gate = changed.wait(gate).unwrap();
        }
        drop(gate);
        // SAFETY: The caller keeps the samples valid for the duration of this callback.
        unsafe { push_audio(samples, frames, channels, sample_rate) }
    }

    unsafe extern "C" fn clear_audio() {
        AUDIO.lock().unwrap().clear();
    }

    #[test]
    fn decodes_little_endian_pcm_samples() {
        // oracle: mutant interpreting little-endian PCM samples with the wrong byte order.
        assert_eq!(decode_pcm16(&[0x34, 0x12, 0x00, 0x80]).unwrap(), [0x1234, i16::MIN]);
    }

    #[test]
    fn rejects_incomplete_pcm_samples() {
        // oracle: mutant accepting a trailing byte as part of a PCM sample.
        assert!(decode_pcm16(&[0x34]).is_err());
    }

    #[test]
    fn client_pcm_reaches_cef_only_during_its_capture() {
        // oracle: mutant dropping client PCM or delivering it after capture stopped.
        let _guard = TEST_LOCK.lock().unwrap();
        CAPTURING.store(true, Ordering::Relaxed);
        AUDIO.lock().unwrap().clear();
        let media = RemoteMedia::for_test(camera_format, push_frame, microphone_active, push_audio);
        let mut audio = AudioInput::new(media);
        let (sender, mut events) = unbounded_channel();
        audio.set_sender(sender);
        audio.connected();
        audio.tick();

        let ServerEvent::CreateDynamicChannel(build) = events.try_recv().unwrap() else {
            panic!("expected channel creation");
        };
        let mut channel = build(7);
        channel.start(7).unwrap();
        let process = |channel: &mut dyn ironrdp_dvc::DvcServerProcessor, pdu| {
            channel.process(7, &encode_vec(&pdu).unwrap()).unwrap();
        };
        process(&mut *channel, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
        let format = pcm_format(1, 48_000, 16);
        process(&mut *channel, RdpeaiPdu::Formats(FormatsPdu::client(vec![format])));
        audio.tick();
        let ServerEvent::RdpeaiWithValidity {
            message:
                RdpeaiServerMessage::OpenOnChannel {
                    channel_id,
                    frames_per_packet,
                    initial_format,
                    capture_format,
                },
            is_current,
        } = events.try_recv().unwrap()
        else {
            panic!("expected guarded AUDIO_INPUT open");
        };
        assert!(is_current());
        assert_eq!(channel_id, 7);
        let server = channel.as_any_mut().downcast_mut::<RdpeaiServer>().unwrap();
        server.open(frames_per_packet, initial_format, capture_format).unwrap();
        process(&mut *channel, RdpeaiPdu::FormatChange(FormatChangePdu::new(0)));
        process(
            &mut *channel,
            RdpeaiPdu::OpenReply(OpenReplyPdu {
                result: OpenReplyPdu::S_OK,
            }),
        );
        let packet = || RdpeaiPdu::Data(DataPdu::new(vec![0x34, 0x12, 0x00, 0x80]));
        process(&mut *channel, packet());
        assert_eq!(*AUDIO.lock().unwrap(), [0x1234, i16::MIN]);

        CAPTURING.store(false, Ordering::Relaxed);
        audio.tick();
        process(&mut *channel, packet());
        assert_eq!(*AUDIO.lock().unwrap(), [0x1234, i16::MIN]);
    }

    #[test]
    fn queued_audio_close_is_invalid_after_reconnect() {
        // oracle: mutant applying a queued close to a reused channel ID on another connection.
        let _guard = TEST_LOCK.lock().unwrap();
        CAPTURING.store(true, Ordering::Relaxed);
        let media = RemoteMedia::for_test(camera_format, push_frame, microphone_active, push_audio);
        let mut audio = AudioInput::new(media);
        let (sender, mut events) = unbounded_channel();
        audio.set_sender(sender);
        audio.connected();
        audio.tick();
        let ServerEvent::CreateDynamicChannel(build) = events.try_recv().unwrap() else {
            panic!("expected channel creation");
        };
        let _channel = build(7);
        CAPTURING.store(false, Ordering::Relaxed);
        audio.tick();
        audio.disconnected();
        audio.connected();
        let ServerEvent::CloseDynamicChannel { is_current, .. } = events.try_recv().unwrap() else {
            panic!("expected channel close");
        };
        assert!(!is_current(), "the old connection's close remained valid");
    }

    #[test]
    fn disconnect_resets_audio_before_another_client_connects() {
        // oracle: mutant RDP disconnect path omits the CEF microphone-buffer reset.
        let _guard = TEST_LOCK.lock().unwrap();
        CAPTURING.store(true, Ordering::Relaxed);
        AUDIO_RESET_COUNT.store(0, Ordering::Relaxed);
        let media =
            RemoteMedia::for_test_with_reset(camera_format, push_frame, microphone_active, push_audio, reset_audio);
        let audio = AudioInput::new(media);
        audio.connected();
        let resets_before_disconnect = AUDIO_RESET_COUNT.load(Ordering::Relaxed);

        audio.disconnected();

        assert_eq!(
            AUDIO_RESET_COUNT.load(Ordering::Relaxed),
            resets_before_disconnect + 1,
            "disconnect left the previous client's microphone buffer available"
        );
    }

    #[test]
    fn disconnect_clears_audio_from_an_in_flight_packet() {
        // oracle: mutant an in-flight packet refills the old client's buffer after disconnect.
        let guard = TEST_LOCK.lock().unwrap();
        AUDIO.lock().unwrap().clear();
        *PUSH_GATE.0.lock().unwrap() = (false, false);
        let media = RemoteMedia::for_test_with_reset(
            camera_format,
            push_frame,
            microphone_active,
            blocking_push_audio,
            clear_audio,
        );
        let audio = AudioInput::new(media);
        audio.connected();
        let (generation, token) = {
            let mut shared = audio.shared.lock().unwrap();
            shared.active = true;
            shared.channel_id = Some(7);
            (shared.generation, ChannelGuard::new(&shared.connection_token))
        };
        let mut backend = Backend {
            shared: Arc::clone(&audio.shared),
            media,
            channel_id: 7,
            generation,
            token,
            open_token: Arc::new(AtomicBool::new(true)),
            formats: Vec::new(),
        };
        let push = thread::spawn(move || {
            backend.on_audio_data(&pcm_format(1, 48_000, 16), &[0x34, 0x12]);
        });
        let (lock, changed) = &PUSH_GATE;
        let gate = lock.lock().unwrap();
        let (gate, _) = changed
            .wait_timeout_while(gate, Duration::from_secs(5), |gate| !gate.0)
            .unwrap();
        if !gate.0 {
            drop(gate);
            drop(guard);
            panic!("audio callback did not run");
        }
        drop(gate);

        let (started_tx, started_rx) = mpsc::sync_channel(0);
        let (finished_tx, finished_rx) = mpsc::sync_channel(0);
        let disconnect = thread::spawn(move || {
            started_tx.send(()).unwrap();
            audio.disconnected();
            finished_tx.send(()).unwrap();
        });
        started_rx.recv().unwrap();
        let completed_before_push = finished_rx.recv_timeout(Duration::from_millis(200)).is_ok();
        let mut gate = lock.lock().unwrap();
        gate.1 = true;
        changed.notify_all();
        drop(gate);
        push.join().unwrap();
        if !completed_before_push {
            finished_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        disconnect.join().unwrap();

        let audio_was_cleared = AUDIO.lock().unwrap().is_empty();
        drop(guard);
        assert!(audio_was_cleared, "old audio survived disconnect");
    }

    #[test]
    fn queued_audio_open_is_invalid_after_capture_stops() {
        // oracle: mutant sending a queued AUDIO_INPUT Open after capture has stopped.
        let _guard = TEST_LOCK.lock().unwrap();
        CAPTURING.store(true, Ordering::Relaxed);
        let media = RemoteMedia::for_test(camera_format, push_frame, microphone_active, push_audio);
        let mut audio = AudioInput::new(media);
        let (sender, mut events) = unbounded_channel();
        audio.set_sender(sender);
        audio.connected();
        audio.tick();

        let ServerEvent::CreateDynamicChannel(build) = events.try_recv().unwrap() else {
            panic!("expected channel creation");
        };
        let mut channel = build(7);
        channel.start(7).unwrap();
        let process = |channel: &mut dyn ironrdp_dvc::DvcServerProcessor, pdu| {
            channel.process(7, &encode_vec(&pdu).unwrap()).unwrap();
        };
        process(&mut *channel, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
        let format = pcm_format(1, 48_000, 16);
        process(&mut *channel, RdpeaiPdu::Formats(FormatsPdu::client(vec![format])));
        audio.tick();
        let ServerEvent::RdpeaiWithValidity { is_current, .. } = events.try_recv().unwrap() else {
            panic!("expected guarded AUDIO_INPUT open");
        };
        assert!(is_current(), "the active capture request was not current");

        CAPTURING.store(false, Ordering::Relaxed);
        audio.tick();
        assert!(!is_current(), "the stopped capture request remained current");
    }

    #[test]
    fn stale_audio_open_is_invalid_after_channel_id_reuse() {
        // oracle: mutant stale open remains valid after its channel ID is reused.
        let _guard = TEST_LOCK.lock().unwrap();
        CAPTURING.store(true, Ordering::Relaxed);
        let media = RemoteMedia::for_test(camera_format, push_frame, microphone_active, push_audio);
        let mut audio = AudioInput::new(media);
        let (sender, mut events) = unbounded_channel();
        audio.set_sender(sender);
        audio.connected();
        audio.tick();

        let ServerEvent::CreateDynamicChannel(build) = events.try_recv().unwrap() else {
            panic!("expected channel creation");
        };
        let mut channel = build(7);
        channel.start(7).unwrap();
        let process = |channel: &mut dyn ironrdp_dvc::DvcServerProcessor, pdu| {
            channel.process(7, &encode_vec(&pdu).unwrap()).unwrap();
        };
        process(&mut *channel, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
        let format = pcm_format(1, 48_000, 16);
        process(&mut *channel, RdpeaiPdu::Formats(FormatsPdu::client(vec![format])));
        audio.tick();
        let ServerEvent::RdpeaiWithValidity { is_current, .. } = events.try_recv().unwrap() else {
            panic!("expected guarded AUDIO_INPUT open");
        };

        channel.close(7);
        audio.tick();
        let ServerEvent::CreateDynamicChannel(replacement) = events.try_recv().unwrap() else {
            panic!("expected replacement channel");
        };
        let _replacement = replacement(7);
        assert!(!is_current(), "the old channel's open remained valid after ID reuse");
    }

    #[test]
    fn stale_audio_factory_does_not_duplicate_a_new_request() {
        // oracle: mutant clearing a newer pending request when an old factory runs.
        let _guard = TEST_LOCK.lock().unwrap();
        CAPTURING.store(true, Ordering::Relaxed);
        let media = RemoteMedia::for_test(camera_format, push_frame, microphone_active, push_audio);
        let mut audio = AudioInput::new(media);
        let (sender, mut events) = unbounded_channel();
        audio.set_sender(sender);
        audio.connected();
        audio.tick();
        let ServerEvent::CreateDynamicChannel(old) = events.try_recv().unwrap() else {
            panic!("expected channel creation");
        };
        CAPTURING.store(false, Ordering::Relaxed);
        audio.tick();
        CAPTURING.store(true, Ordering::Relaxed);
        audio.tick();
        let ServerEvent::CreateDynamicChannel(_new) = events.try_recv().unwrap() else {
            panic!("expected replacement channel");
        };
        let _stale = old(7);
        assert!(matches!(events.try_recv(), Ok(ServerEvent::CloseDynamicChannel { .. })));
        audio.tick();
        assert!(events.try_recv().is_err(), "a duplicate creation was queued");
    }

    #[test]
    fn unready_drdynvc_drop_retries_audio_channel_request() {
        // oracle: mutant permanently wedging microphone capture when DRDYNVC drops an early CreateDynamicChannel.
        let _guard = TEST_LOCK.lock().unwrap();
        CAPTURING.store(true, Ordering::Relaxed);
        let media = RemoteMedia::for_test(camera_format, push_frame, microphone_active, push_audio);
        let mut audio = AudioInput::new(media);
        let (sender, mut events) = unbounded_channel();
        audio.set_sender(sender);
        audio.connected();
        audio.tick();
        let unready = events.try_recv().expect("initial channel request");
        drop(unready);

        audio.tick();
        assert!(matches!(events.try_recv(), Ok(ServerEvent::CreateDynamicChannel(_))));
    }
}
