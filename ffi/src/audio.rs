#[cfg(not(target_os = "android"))]
mod native {
    use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    use core::time::Duration;
    use std::borrow::Cow;
    use std::sync::{Arc, Mutex, PoisonError, Weak, mpsc};
    use std::time::Instant;

    use ironrdp::dvc::{DvcChannelListener, DvcClientProcessor, DvcMessage, DvcProcessor};
    use ironrdp::pdu::PduResult;
    use ironrdp::rdpsnd::client::{Clock, RdpsndClientHandler};
    use ironrdp::rdpsnd::pdu::{AudioFormat, AudioFormatFlags, PitchPdu, VolumePdu};
    use ironrdp::svc::SvcMessage;
    use ironrdp_rdpeai::client::RdpeaiClient;
    use ironrdp_rdpsnd_native::DeviceSelector;
    use ironrdp_rdpsnd_native::capture::{CaptureConfig, RdpeaiCaptureBackend};
    use ironrdp_rdpsnd_native::cpal::{PlaybackConfig, RdpsndBackend};
    use tracing::warn;

    /// Buffers 100 ms at the native capture backend's highest sample rate with one frame per packet.
    pub(super) const CAPTURE_QUEUE_CAPACITY: usize = 4_800;

    pub(super) struct CaptureMessages {
        token: u64,
        messages: Vec<SvcMessage>,
    }

    pub(crate) struct AudioShared {
        pub(super) playback: bool,
        pub(super) capture: bool,
        pub(super) output_device: DeviceSelector,
        pub(super) input_device: DeviceSelector,
        pub(super) latency: Duration,
        pub(super) attached: AtomicBool,
        pub(super) playback_enabled: AtomicBool,
        pub(super) capture_enabled: AtomicBool,
        pub(super) playback_backend: Mutex<Option<Weak<Mutex<RdpsndBackend>>>>,
        pub(super) next_generation: AtomicU32,
        pub(super) active_capture: AtomicU64,
        pub(super) failed_capture: AtomicU64,
        pub(super) capture_tx: mpsc::SyncSender<CaptureMessages>,
        pub(super) capture_rx: Mutex<mpsc::Receiver<CaptureMessages>>,
    }

    impl AudioShared {
        pub(crate) fn has_playback(&self) -> bool {
            self.playback
        }

        pub(crate) fn reserves_dynamic_channel(&self, channel_name: &str) -> bool {
            (self.playback
                && matches!(
                    channel_name,
                    ironrdp::rdpsnd::client::DVC_CHANNEL_NAME | ironrdp::rdpsnd::client::LOSSY_DVC_CHANNEL_NAME
                ))
                || (self.capture && channel_name == ironrdp_rdpeai::CHANNEL_NAME)
        }

        fn enqueue_capture(&self, token: u64, messages: Vec<SvcMessage>) -> PduResult<()> {
            // UNTESTED: audio callbacks must not wait for the session loop or shutdown; keep this
            // enqueue nonblocking so a stalled transport cannot deadlock capture shutdown.
            if self.failed_capture.load(Ordering::Acquire) == token {
                return Err(ironrdp::pdu::pdu_other_err!("audio capture uplink is stopped"));
            }
            if self.capture_tx.try_send(CaptureMessages { token, messages }).is_err() {
                self.failed_capture.store(token, Ordering::Release);
                warn!("Audio capture uplink overflow; stopping capture");
                return Err(ironrdp::pdu::pdu_other_err!("audio capture uplink overflow"));
            }
            Ok(())
        }

        pub(crate) fn attach(
            self: &Arc<Self>,
            connector: &mut ironrdp::connector::ClientConnector,
        ) -> Result<(), &'static str> {
            let channels = &connector.static_channels;
            let playback_type = channels.get_by_type::<ironrdp::rdpsnd::client::Rdpsnd>().is_some();
            let drdynvc_type = channels.get_by_type::<ironrdp::dvc::DrdynvcClient>().is_some();
            let rdpdr_type = channels.get_by_type::<ironrdp::rdpdr::Rdpdr>().is_some();
            if self.playback
                && !playback_type
                && channels
                    .get_by_channel_name_key(&ironrdp::rdpsnd::client::Rdpsnd::NAME)
                    .is_some()
            {
                return Err("RDPSND channel name is already registered");
            }
            if !drdynvc_type
                && channels
                    .get_by_channel_name_key(&ironrdp::dvc::DrdynvcClient::NAME)
                    .is_some()
            {
                return Err("DRDYNVC channel name is already registered");
            }
            if self.playback && !rdpdr_type && channels.get_by_channel_name_key(&ironrdp::rdpdr::Rdpdr::NAME).is_some()
            {
                return Err("RDPDR channel name is already registered");
            }
            let needed = usize::from(self.playback && !playback_type)
                + usize::from(!drdynvc_type)
                + usize::from(self.playback && !rdpdr_type);
            if ironrdp::svc::MAX_STATIC_CHANNELS < channels.len() + needed {
                return Err("no space for native audio channels");
            }
            if let Some(drdynvc) = connector.get_static_channel_processor_mut::<ironrdp::dvc::DrdynvcClient>() {
                let playback_in_use = self.playback
                    && (drdynvc.has_channel_name(ironrdp::rdpsnd::client::DVC_CHANNEL_NAME)
                        || drdynvc.has_channel_name(ironrdp::rdpsnd::client::LOSSY_DVC_CHANNEL_NAME));
                if playback_in_use || (self.capture && drdynvc.has_channel_name(ironrdp_rdpeai::CHANNEL_NAME)) {
                    return Err("native audio dynamic channel is already registered");
                }
            }
            if self.attached.swap(true, Ordering::AcqRel) {
                return Err("native audio is already attached");
            }

            connector.config.enable_audio_capture = self.capture;
            connector.config.enable_audio_playback = self.playback || playback_type;
            if self.playback {
                if !rdpdr_type {
                    // Windows servers require RDPDR to start playback, even without redirected devices.
                    connector.attach_static_channel(ironrdp::rdpdr::Rdpdr::new(
                        Box::new(ironrdp::rdpdr::NoopRdpdrBackend),
                        connector.config.client_name.clone(),
                    ));
                }
                let config = PlaybackConfig {
                    device: self.output_device.clone(),
                    latency: self.latency,
                };
                let backend = RdpsndBackend::with_config(config);
                let formats = backend.get_formats().to_vec().into();
                let backend = Arc::new(Mutex::new(backend));
                *self.playback_backend.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::downgrade(&backend));
                let playback = SharedPlayback {
                    backend,
                    formats,
                    shared: Arc::clone(self),
                };
                connector.attach_static_channel(rdpsnd_with_clock(Box::new(playback.clone())));
                let state = rdpsnd_dvc_with_clock(Box::new(playback));
                let drdynvc = drdynvc(connector);
                let channel = state.clone();
                drdynvc.attach_listener(PlaybackListener {
                    state: channel,
                    lossy: false,
                });
                drdynvc.attach_listener(PlaybackListener { state, lossy: true });
            }
            if self.capture {
                drdynvc(connector).attach_listener(CaptureListener(Arc::clone(self)));
            }
            Ok(())
        }
    }

    fn drdynvc(connector: &mut ironrdp::connector::ClientConnector) -> &mut ironrdp::dvc::DrdynvcClient {
        if connector
            .get_static_channel_processor_mut::<ironrdp::dvc::DrdynvcClient>()
            .is_none()
        {
            connector.attach_static_channel(ironrdp::dvc::DrdynvcClient::new());
        }
        connector
            .get_static_channel_processor_mut()
            .expect("DRDYNVC was attached")
    }

    fn playback_clock() -> Clock {
        let origin = Instant::now();
        Box::new(move || origin.elapsed())
    }

    fn rdpsnd_with_clock(handler: Box<dyn RdpsndClientHandler>) -> ironrdp::rdpsnd::client::Rdpsnd {
        ironrdp::rdpsnd::client::Rdpsnd::new(handler).with_clock(playback_clock())
    }

    fn rdpsnd_dvc_with_clock(handler: Box<dyn RdpsndClientHandler>) -> ironrdp::rdpsnd::client::RdpsndDvcState {
        ironrdp::rdpsnd::client::RdpsndDvcState::new(handler).with_clock(playback_clock())
    }

    #[derive(Clone)]
    struct SharedPlayback {
        backend: Arc<Mutex<RdpsndBackend>>,
        formats: Arc<[AudioFormat]>,
        shared: Arc<AudioShared>,
    }

    impl core::fmt::Debug for SharedPlayback {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("SharedPlayback").finish_non_exhaustive()
        }
    }

    impl SharedPlayback {
        fn with_backend<T>(&self, f: impl FnOnce(&mut RdpsndBackend) -> T) -> T {
            f(&mut self.backend.lock().unwrap_or_else(PoisonError::into_inner))
        }
    }

    impl RdpsndClientHandler for SharedPlayback {
        fn get_flags(&self) -> AudioFormatFlags {
            self.with_backend(|backend| backend.get_flags())
        }
        fn get_formats(&self) -> &[AudioFormat] {
            &self.formats
        }
        fn choose_formats(&self, server_formats: &[AudioFormat]) -> Vec<AudioFormat> {
            self.with_backend(|backend| backend.choose_formats(server_formats))
        }
        fn wave(&mut self, format: &AudioFormat, ts: u32, data: Cow<'_, [u8]>) {
            if self.shared.playback_enabled.load(Ordering::Acquire) {
                self.with_backend(|backend| backend.wave(format, ts, data));
            }
        }
        fn playback_latency(&self) -> Duration {
            self.with_backend(|backend| backend.playback_latency())
        }
        fn set_volume(&mut self, volume: VolumePdu) {
            self.with_backend(|backend| backend.set_volume(volume));
        }
        fn set_pitch(&mut self, pitch: PitchPdu) {
            self.with_backend(|backend| backend.set_pitch(pitch));
        }
        fn close(&mut self) {
            self.with_backend(RdpsndBackend::close);
        }
    }

    struct PlaybackListener {
        state: ironrdp::rdpsnd::client::RdpsndDvcState,
        lossy: bool,
    }

    impl DvcChannelListener for PlaybackListener {
        fn channel_name(&self) -> &str {
            if self.lossy {
                ironrdp::rdpsnd::client::LOSSY_DVC_CHANNEL_NAME
            } else {
                ironrdp::rdpsnd::client::DVC_CHANNEL_NAME
            }
        }
        fn create(&mut self, _: u32) -> Option<Box<dyn DvcClientProcessor>> {
            Some(Box::new(if self.lossy {
                self.state.lossy_channel()
            } else {
                self.state.channel()
            }))
        }
    }

    struct CaptureListener(Arc<AudioShared>);

    impl CaptureListener {
        fn create_channel(&self, channel_id: u32) -> Option<CaptureChannel> {
            let shared = &self.0;
            if !shared.capture_enabled.load(Ordering::Acquire) {
                return None;
            }
            let generation = shared.next_generation.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
            let token = (u64::from(generation) << 32) | u64::from(channel_id);
            if token == 0
                || shared
                    .active_capture
                    .compare_exchange(0, token, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
            {
                return None;
            }
            let config = CaptureConfig {
                device: shared.input_device.clone(),
            };
            let backend = RdpeaiCaptureBackend::with_config(config);
            let sender = Arc::clone(shared);
            let client = RdpeaiClient::new(
                Box::new(backend),
                Box::new(move |_, messages| sender.enqueue_capture(token, messages)),
            )
            .with_ordered_replies();
            Some(CaptureChannel {
                client,
                token,
                shared: Arc::clone(shared),
            })
        }
    }

    impl DvcChannelListener for CaptureListener {
        fn channel_name(&self) -> &str {
            ironrdp_rdpeai::CHANNEL_NAME
        }
        fn create(&mut self, channel_id: u32) -> Option<Box<dyn DvcClientProcessor>> {
            self.create_channel(channel_id).map(|channel| {
                let processor: Box<dyn DvcClientProcessor> = Box::new(channel);
                processor
            })
        }
    }

    pub(crate) struct CaptureChannel {
        client: RdpeaiClient,
        token: u64,
        shared: Arc<AudioShared>,
    }

    ironrdp_core::impl_as_any!(CaptureChannel);

    impl DvcProcessor for CaptureChannel {
        fn channel_name(&self) -> &str {
            self.client.channel_name()
        }
        fn max_message_size(&self) -> Option<usize> {
            self.client.max_message_size()
        }
        fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
            self.client.start(channel_id)
        }
        fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
            match self.client.process(channel_id, payload) {
                Ok(messages) => Ok(messages),
                Err(error) => {
                    warn!(%error, "Audio capture channel failed; stopping capture");
                    self.shared.failed_capture.store(self.token, Ordering::Release);
                    Ok(Vec::new())
                }
            }
        }
        fn close(&mut self, id: u32) {
            self.client.close(id);
        }
    }

    impl DvcClientProcessor for CaptureChannel {}

    impl Drop for CaptureChannel {
        fn drop(&mut self) {
            let _ = self
                .shared
                .active_capture
                .compare_exchange(self.token, 0, Ordering::AcqRel, Ordering::Acquire);
        }
    }

    pub(crate) fn poll_audio(
        stage: &mut ironrdp::session::ActiveStage,
        shared: &Arc<AudioShared>,
    ) -> Result<Vec<ironrdp::session::ActiveStageOutput>, Box<crate::error::ffi::IronRdpError>> {
        let mut outputs = Vec::new();
        for _ in 0..CAPTURE_QUEUE_CAPACITY {
            if let Some(failure) = take_active_capture_failure(shared) {
                shared.capture_enabled.store(false, Ordering::Release);
                outputs.extend(close_capture(stage, shared, failure)?);
                return Ok(outputs);
            }
            let next = shared
                .capture_rx
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .try_recv();
            let Ok(next) = next else {
                break;
            };
            if shared.active_capture.load(Ordering::Acquire) != next.token {
                continue;
            }
            match stage.encode_dvc_messages(next.messages) {
                Ok(frame) => outputs.push(ironrdp::session::ActiveStageOutput::ResponseFrame(frame)),
                Err(error) => {
                    shared.capture_enabled.store(false, Ordering::Release);
                    warn!(error = %error.report(), "Failed to encode audio capture frame; stopping capture");
                    outputs.extend(close_capture(stage, shared, next.token)?);
                    return Ok(outputs);
                }
            }
        }
        if let Some(failure) = take_active_capture_failure(shared) {
            shared.capture_enabled.store(false, Ordering::Release);
            outputs.extend(close_capture(stage, shared, failure)?);
        }
        Ok(outputs)
    }

    fn take_active_capture_failure(shared: &AudioShared) -> Option<u64> {
        let failure = shared.failed_capture.swap(0, Ordering::AcqRel);
        (failure != 0 && shared.active_capture.load(Ordering::Acquire) == failure).then_some(failure)
    }

    pub(crate) fn stop_audio(
        stage: &mut ironrdp::session::ActiveStage,
        shared: &Arc<AudioShared>,
    ) -> Result<Vec<ironrdp::session::ActiveStageOutput>, Box<crate::error::ffi::IronRdpError>> {
        shared.capture_enabled.store(false, Ordering::Release);
        shared.playback_enabled.store(false, Ordering::Release);
        if let Some(backend) = shared
            .playback_backend
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(Weak::upgrade)
        {
            backend.lock().unwrap_or_else(PoisonError::into_inner).close();
        }
        let token = shared.active_capture.load(Ordering::Acquire);
        if token == 0 {
            return Ok(Vec::new());
        }
        close_capture(stage, shared, token)
    }

    fn close_capture(
        stage: &mut ironrdp::session::ActiveStage,
        shared: &Arc<AudioShared>,
        token: u64,
    ) -> Result<Vec<ironrdp::session::ActiveStageOutput>, Box<crate::error::ffi::IronRdpError>> {
        let channel_id = u32::try_from(token & u64::from(u32::MAX)).expect("channel id fits in u32");
        let matches = stage
            .get_dvc_by_channel_id::<CaptureChannel>(channel_id)
            .is_some_and(|channel| {
                channel.processor().token == token && Arc::ptr_eq(&channel.processor().shared, shared)
            });
        if !matches {
            return Ok(Vec::new());
        }
        let drdynvc = stage
            .get_svc_processor_mut::<ironrdp::dvc::DrdynvcClient>()
            .ok_or("DRDYNVC channel is not available")?;
        let message = drdynvc
            .close_channel(channel_id)
            .ok_or("audio capture channel is not available")?;
        let frame = stage.encode_dvc_messages(vec![message])?;
        Ok(vec![ironrdp::session::ActiveStageOutput::ResponseFrame(frame)])
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::audio::ffi::NativeAudio;
        use ironrdp_rdpeai::client::{AudioPacketSink, RdpeaiCaptureHandler};

        #[derive(Debug)]
        struct DelayedPlayback {
            formats: Vec<AudioFormat>,
        }

        impl DelayedPlayback {
            fn new() -> Self {
                Self {
                    formats: vec![AudioFormat {
                        format: ironrdp::rdpsnd::pdu::WaveFormat::PCM,
                        n_channels: 2,
                        n_samples_per_sec: 48_000,
                        n_avg_bytes_per_sec: 192_000,
                        n_block_align: 4,
                        bits_per_sample: 16,
                        data: None,
                    }],
                }
            }
        }

        impl RdpsndClientHandler for DelayedPlayback {
            fn get_formats(&self) -> &[AudioFormat] {
                &self.formats
            }

            fn wave(&mut self, _: &AudioFormat, _: u32, _: Cow<'_, [u8]>) {
                std::thread::sleep(Duration::from_millis(5));
            }

            fn set_volume(&mut self, _: VolumePdu) {}

            fn set_pitch(&mut self, _: PitchPdu) {}

            fn close(&mut self) {}
        }

        fn server_audio_payload(pdu: ironrdp::rdpsnd::pdu::ServerAudioOutputPdu<'_>) -> Vec<u8> {
            ironrdp_core::encode_vec(&pdu).expect("encode audio test PDU")
        }

        fn audio_negotiation_payloads() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
            use ironrdp::rdpsnd::pdu::{ServerAudioFormatPdu, ServerAudioOutputPdu, TrainingPdu, Version, Wave2Pdu};

            let format = DelayedPlayback::new().formats[0].clone();
            let formats = server_audio_payload(ServerAudioOutputPdu::AudioFormat(ServerAudioFormatPdu {
                version: Version::V8,
                formats: vec![format],
            }));
            let training = server_audio_payload(ServerAudioOutputPdu::Training(TrainingPdu {
                timestamp: 0,
                data: Vec::new(),
            }));
            let wave = server_audio_payload(ServerAudioOutputPdu::Wave2(Wave2Pdu {
                timestamp: 100,
                format_no: 0,
                block_no: 1,
                audio_timestamp: 0,
                data: Cow::Borrowed(&[0; 4]),
            }));

            (formats, training, wave)
        }

        fn connector() -> ironrdp::connector::ClientConnector {
            use crate::connector::config::ffi::{ConfigBuilder, PerformanceFlags};

            let mut builder = ConfigBuilder::new();
            builder.with_username_and_password("user", "password");
            builder.set_desktop_size(768, 1024);
            builder.set_client_name("client");
            builder.set_client_dir("C:\\");
            builder.set_performance_flags(&PerformanceFlags::new_default());
            let config = builder.build().unwrap_or_else(|_| panic!("connector config"));
            ironrdp::connector::ClientConnector::new(config.connector.clone(), "127.0.0.1:0".parse().unwrap())
        }

        // oracle: regression #2469
        // Windows servers need the device-redirection channel joined before starting playback.
        #[test]
        fn playback_attaches_rdpdr_when_missing() {
            let audio = NativeAudio::new(true, false, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let mut connector = connector();
            audio.0.attach(&mut connector).expect("attach native playback");

            assert!(
                connector
                    .get_static_channel_processor_mut::<ironrdp::rdpdr::Rdpdr>()
                    .is_some()
            );
        }

        // oracle: regression #2469
        #[test]
        fn playback_preserves_an_existing_rdpdr_processor() {
            let audio = NativeAudio::new(true, false, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let mut connector = connector();
            connector.attach_static_channel(
                ironrdp::rdpdr::Rdpdr::new(Box::new(ironrdp::rdpdr::NoopRdpdrBackend), "custom".to_owned())
                    .with_smartcard(42),
            );
            let original = core::ptr::from_ref(
                connector
                    .get_static_channel_processor_mut::<ironrdp::rdpdr::Rdpdr>()
                    .unwrap(),
            );

            audio.0.attach(&mut connector).expect("attach native playback");

            assert_eq!(
                original,
                core::ptr::from_ref(
                    connector
                        .get_static_channel_processor_mut::<ironrdp::rdpdr::Rdpdr>()
                        .unwrap()
                )
            );
        }

        // oracle: regression #2469
        #[test]
        fn capture_only_does_not_attach_rdpdr() {
            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let mut connector = connector();
            audio.0.attach(&mut connector).expect("attach native capture");

            assert!(
                connector
                    .get_static_channel_processor_mut::<ironrdp::rdpdr::Rdpdr>()
                    .is_none()
            );
        }

        // oracle: regression #2469
        #[test]
        fn full_capture_queue_rejects_later_messages() {
            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let token = (1u64 << 32) | 7;
            for _ in 0..CAPTURE_QUEUE_CAPACITY {
                audio.0.enqueue_capture(token, Vec::new()).expect("queue has space");
            }
            assert!(audio.0.enqueue_capture(token, Vec::new()).is_err());
            assert!(audio.0.enqueue_capture(token, Vec::new()).is_err());
            assert_eq!(audio.0.failed_capture.load(Ordering::Acquire), token);
        }

        // oracle: regression #2469
        #[test]
        fn capture_only_disables_playback_advertisement() {
            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let mut connector = connector();
            audio.0.attach(&mut connector).expect("attach native capture");

            assert!(!connector.config.enable_audio_playback);
        }

        // oracle: spec MS-RDPEA §3.2.5.2.1.6
        #[test]
        fn static_playback_wave_confirm_includes_processing_time() {
            use ironrdp::rdpsnd::pdu::ClientAudioOutputPdu;
            use ironrdp::svc::SvcProcessor as _;

            let (formats, training, wave) = audio_negotiation_payloads();
            let mut client = rdpsnd_with_clock(Box::new(DelayedPlayback::new()));
            client.process(&formats).expect("negotiate formats");
            client.process(&training).expect("finish training");
            let replies = client.process(&wave).expect("process wave");
            let [reply] = replies.as_slice() else {
                panic!("expected one Wave Confirm response");
            };
            let encoded = reply.encode_unframed_pdu().expect("encode response");
            let response: ClientAudioOutputPdu = ironrdp_core::decode(&encoded).expect("decode response");
            let ClientAudioOutputPdu::WaveConfirm(confirm) = response else {
                panic!("expected Wave Confirm");
            };

            assert!(confirm.timestamp > 100);
        }

        // oracle: spec MS-RDPEA §3.2.5.2.1.6
        #[test]
        fn dynamic_playback_wave_confirm_includes_processing_time() {
            use ironrdp::dvc::DvcProcessor as _;
            use ironrdp::rdpsnd::pdu::ClientAudioOutputPdu;

            let (formats, training, wave) = audio_negotiation_payloads();
            let mut client = rdpsnd_dvc_with_clock(Box::new(DelayedPlayback::new())).channel();
            client.start(7).expect("start playback channel");
            client.process(7, &formats).expect("negotiate formats");
            client.process(7, &training).expect("finish training");
            let replies = client.process(7, &wave).expect("process wave");
            let [reply] = replies.as_slice() else {
                panic!("expected one Wave Confirm response");
            };
            let encoded = ironrdp_core::encode_vec(reply.as_ref()).expect("encode response");
            let response: ClientAudioOutputPdu = ironrdp_core::decode(&encoded).expect("decode response");
            let ClientAudioOutputPdu::WaveConfirm(confirm) = response else {
                panic!("expected Wave Confirm");
            };

            assert!(confirm.timestamp > 100);
        }

        // oracle: regression #2469
        #[test]
        fn documented_poll_interval_accepts_maximum_50ms_capture_burst() {
            use ironrdp::svc::ChannelFlags;
            use ironrdp_rdpeai::pdu::RdpeaiPdu;

            let backend = RdpeaiCaptureBackend::new();
            let max_sample_rate = backend
                .supported_formats()
                .iter()
                .map(|format| format.n_samples_per_sec)
                .max()
                .expect("capture formats");
            let max_sample_rate = usize::try_from(max_sample_rate).expect("sample rate");
            let poll_batch_count = max_sample_rate / 20;
            assert!(CAPTURE_QUEUE_CAPACITY >= max_sample_rate / 10);
            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let listener = CaptureListener(Arc::clone(&audio.0));
            let channel = listener.create_channel(7).expect("capture channel");
            let token = channel.token;
            let mut drdynvc = ironrdp::dvc::DrdynvcClient::new();
            drdynvc
                .attach_established_dynamic_channel(7, channel)
                .expect("attach capture channel");
            let mut channels = ironrdp::svc::StaticChannelSet::new();
            channels.insert(drdynvc);
            channels.attach_channel_id(core::any::TypeId::of::<ironrdp::dvc::DrdynvcClient>(), 1004);
            let mut stage = ironrdp::session::ActiveStageBuilder {
                static_channels: channels,
                user_channel_id: 1002,
                io_channel_id: 1003,
                message_channel_id: None,
                share_id: 1,
                compression_type: None,
                enable_server_pointer: false,
                pointer_software_rendering: false,
            }
            .build();

            for _ in 0..poll_batch_count {
                let messages = ironrdp::dvc::encode_dvc_messages(
                    7,
                    vec![Box::new(RdpeaiPdu::DataIncoming)],
                    ChannelFlags::empty(),
                )
                .expect("encode capture batch");
                audio
                    .0
                    .enqueue_capture(token, messages)
                    .expect("50 ms of maximum-rate capture fits");
            }

            let outputs = poll_audio(&mut stage, &audio.0).unwrap_or_else(|_| panic!("poll audio"));
            assert_eq!(outputs.len(), poll_batch_count);
        }

        // oracle: regression #2469
        #[test]
        fn capture_listener_allows_reopen_only_after_previous_channel_is_dropped() {
            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let mut listener = CaptureListener(Arc::clone(&audio.0));
            let first = listener.create(7).expect("first channel");
            assert!(listener.create(8).is_none());
            drop(first);
            let second = listener.create(7).expect("reopened channel");
            assert_ne!(audio.0.active_capture.load(Ordering::Acquire), 0);
            drop(second);
            assert_eq!(audio.0.active_capture.load(Ordering::Acquire), 0);
        }

        // oracle: regression #2469
        #[test]
        fn malformed_capture_pdu_stops_capture_without_failing_the_session() {
            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let listener = CaptureListener(Arc::clone(&audio.0));
            let mut channel = listener.create_channel(7).expect("capture channel");
            channel.start(7).expect("start capture channel");

            assert!(channel.process(7, &[]).expect("capture failure is isolated").is_empty());
            assert_eq!(audio.0.failed_capture.load(Ordering::Acquire), channel.token);
        }

        // oracle: regression #2469
        #[test]
        fn full_queue_isolates_a_capture_control_reply_failure() {
            use ironrdp_rdpeai::pdu::{RdpeaiPdu, Version, VersionPdu};

            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let listener = CaptureListener(Arc::clone(&audio.0));
            let mut channel = listener.create_channel(7).expect("capture channel");
            channel.start(7).expect("start capture channel");
            for _ in 0..CAPTURE_QUEUE_CAPACITY {
                audio
                    .0
                    .enqueue_capture(channel.token, Vec::new())
                    .expect("queue has space");
            }
            let version =
                ironrdp_core::encode_vec(&RdpeaiPdu::Version(VersionPdu::new(Version::V1))).expect("encode version");

            assert!(
                channel
                    .process(7, &version)
                    .expect("capture failure is isolated")
                    .is_empty()
            );
            assert_eq!(audio.0.failed_capture.load(Ordering::Acquire), channel.token);
        }

        struct ConfirmingCapture {
            formats: Vec<AudioFormat>,
            closed: Arc<AtomicBool>,
        }

        impl RdpeaiCaptureHandler for ConfirmingCapture {
            fn supported_formats(&self) -> &[AudioFormat] {
                &self.formats
            }

            fn open(&mut self, _: &AudioFormat, _: &AudioFormat, _: usize, _: AudioPacketSink) -> i32 {
                ironrdp_rdpeai::pdu::OpenReplyPdu::S_OK
            }

            fn set_format(&mut self, _: &AudioFormat, _: usize) -> bool {
                false
            }

            fn set_format_ordered(
                &mut self,
                _: &AudioFormat,
                _: usize,
                confirm: &mut dyn FnMut() -> PduResult<()>,
            ) -> PduResult<bool> {
                confirm()?;
                Ok(true)
            }

            fn close(&mut self) {
                self.closed.store(true, Ordering::Release);
            }
        }

        // oracle: regression #2469
        #[test]
        fn failed_format_confirmation_closes_capture_when_polled() {
            use ironrdp_rdpeai::pdu::{
                FormatChangePdu, FormatsPdu, OpenPdu, RdpeaiPdu, Version, VersionPdu, pcm_format,
            };

            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let token = (1u64 << 32) | 7;
            audio.0.active_capture.store(token, Ordering::Release);
            let closed = Arc::new(AtomicBool::new(false));
            let formats = vec![pcm_format(1, 16_000, 16), pcm_format(1, 48_000, 16)];
            let sender = Arc::clone(&audio.0);
            let client = RdpeaiClient::new(
                Box::new(ConfirmingCapture {
                    formats: formats.clone(),
                    closed: Arc::clone(&closed),
                }),
                Box::new(move |_, messages| sender.enqueue_capture(token, messages)),
            )
            .with_ordered_replies();
            let mut channel = CaptureChannel {
                client,
                token,
                shared: Arc::clone(&audio.0),
            };
            channel.start(7).expect("start capture channel");

            for pdu in [
                RdpeaiPdu::Version(VersionPdu::new(Version::V1)),
                RdpeaiPdu::Formats(FormatsPdu::server(formats.clone())),
                RdpeaiPdu::Open(OpenPdu {
                    frames_per_packet: 2,
                    initial_format: 0,
                    capture_format: formats[0].clone(),
                }),
            ] {
                let payload = ironrdp_core::encode_vec(&pdu).expect("encode capture PDU");
                channel.process(7, &payload).expect("process capture PDU");
            }
            while audio
                .0
                .capture_rx
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .try_recv()
                .is_ok()
            {}

            for _ in 0..CAPTURE_QUEUE_CAPACITY {
                audio.0.enqueue_capture(token, Vec::new()).expect("queue has space");
            }
            let format_change =
                ironrdp_core::encode_vec(&RdpeaiPdu::FormatChange(FormatChangePdu::new(1))).expect("encode PDU");
            assert!(
                channel
                    .process(7, &format_change)
                    .expect("isolate capture failure")
                    .is_empty()
            );
            assert!(closed.load(Ordering::Acquire));

            let mut drdynvc = ironrdp::dvc::DrdynvcClient::new();
            drdynvc
                .attach_established_dynamic_channel(7, channel)
                .expect("attach capture channel");
            let mut channels = ironrdp::svc::StaticChannelSet::new();
            channels.insert(drdynvc);
            channels.attach_channel_id(core::any::TypeId::of::<ironrdp::dvc::DrdynvcClient>(), 1004);
            let mut stage = ironrdp::session::ActiveStageBuilder {
                static_channels: channels,
                user_channel_id: 1002,
                io_channel_id: 1003,
                message_channel_id: None,
                share_id: 1,
                compression_type: None,
                enable_server_pointer: false,
                pointer_software_rendering: false,
            }
            .build();

            let outputs = poll_audio(&mut stage, &audio.0).unwrap_or_else(|_| panic!("poll audio"));
            assert!(matches!(
                outputs.as_slice(),
                [ironrdp::session::ActiveStageOutput::ResponseFrame(_)]
            ));
            assert!(stage.get_dvc_by_channel_id::<CaptureChannel>(7).is_none());
            assert_eq!(audio.0.active_capture.load(Ordering::Acquire), 0);
        }

        // oracle: regression #2469
        #[test]
        fn polling_an_overflow_closes_capture_without_closing_the_session() {
            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let listener = CaptureListener(Arc::clone(&audio.0));
            let channel = listener.create_channel(7).expect("capture channel");
            let token = channel.token;
            let mut drdynvc = ironrdp::dvc::DrdynvcClient::new();
            drdynvc
                .attach_established_dynamic_channel(7, channel)
                .expect("attach capture channel");
            let mut channels = ironrdp::svc::StaticChannelSet::new();
            channels.insert(drdynvc);
            channels.attach_channel_id(core::any::TypeId::of::<ironrdp::dvc::DrdynvcClient>(), 1004);
            let mut stage = ironrdp::session::ActiveStageBuilder {
                static_channels: channels,
                user_channel_id: 1002,
                io_channel_id: 1003,
                message_channel_id: None,
                share_id: 1,
                compression_type: None,
                enable_server_pointer: false,
                pointer_software_rendering: false,
            }
            .build();
            for _ in 0..CAPTURE_QUEUE_CAPACITY {
                audio.0.enqueue_capture(token, Vec::new()).expect("queue has space");
            }
            assert!(audio.0.enqueue_capture(token, Vec::new()).is_err());
            let outputs = poll_audio(&mut stage, &audio.0).unwrap_or_else(|_| panic!("poll audio"));
            assert!(matches!(
                outputs.as_slice(),
                [ironrdp::session::ActiveStageOutput::ResponseFrame(_)]
            ));
            assert!(stage.get_dvc_by_channel_id::<CaptureChannel>(7).is_none());
            assert!(listener.create_channel(7).is_none());
        }
    }
}

#[cfg(not(target_os = "android"))]
pub(crate) use native::{AudioShared, poll_audio, stop_audio};

#[cfg(target_os = "android")]
mod android {
    use std::sync::Arc;

    pub(crate) struct AudioShared;

    impl AudioShared {
        pub(crate) fn has_playback(&self) -> bool {
            false
        }

        pub(crate) fn reserves_dynamic_channel(&self, _channel_name: &str) -> bool {
            false
        }

        pub(crate) fn attach(
            self: &Arc<Self>,
            _connector: &mut ironrdp::connector::ClientConnector,
        ) -> Result<(), &'static str> {
            Err("native audio is not supported on Android")
        }
    }

    pub(crate) fn poll_audio(
        _stage: &mut ironrdp::session::ActiveStage,
        _shared: &Arc<AudioShared>,
    ) -> Result<Vec<ironrdp::session::ActiveStageOutput>, Box<crate::error::ffi::IronRdpError>> {
        Err("native audio is not supported on Android".into())
    }

    pub(crate) fn stop_audio(
        _stage: &mut ironrdp::session::ActiveStage,
        _shared: &Arc<AudioShared>,
    ) -> Result<Vec<ironrdp::session::ActiveStageOutput>, Box<crate::error::ffi::IronRdpError>> {
        Err("native audio is not supported on Android".into())
    }
}

#[cfg(target_os = "android")]
pub(crate) use android::{AudioShared, poll_audio, stop_audio};

#[diplomat::bridge]
pub mod ffi {
    use std::sync::Arc;

    use super::AudioShared;
    use crate::error::ffi::IronRdpError;

    #[diplomat::attr(dotnet, manually_disposable)]
    #[diplomat::opaque]
    pub struct NativeAudio(pub(crate) Arc<AudioShared>);

    impl NativeAudio {
        /// Creates session-local audio settings.
        ///
        /// Empty device names follow the system default.
        /// Capture is opt-in and requires OS microphone permission.
        /// A latency of zero selects the native default.
        pub fn new(
            playback: bool,
            capture: bool,
            output_device: &str,
            input_device: &str,
            latency_ms: u32,
        ) -> Result<Box<Self>, Box<IronRdpError>> {
            #[cfg(target_os = "android")]
            {
                let _ = (playback, capture, output_device, input_device, latency_ms);
                Err("native audio is not supported on Android".into())
            }
            #[cfg(not(target_os = "android"))]
            {
                if !playback && !capture {
                    return Err("native audio needs playback or capture enabled".into());
                }
                let capture_queue_capacity = if capture {
                    super::native::CAPTURE_QUEUE_CAPACITY
                } else {
                    0
                };
                let (capture_tx, capture_rx) = std::sync::mpsc::sync_channel(capture_queue_capacity);
                let latency = if latency_ms == 0 {
                    ironrdp_rdpsnd_native::cpal::DEFAULT_PLAYBACK_LATENCY
                } else {
                    core::time::Duration::from_millis(u64::from(latency_ms))
                };
                Ok(Box::new(Self(Arc::new(AudioShared {
                    playback,
                    capture,
                    output_device: ironrdp_rdpsnd_native::DeviceSelector::parse(output_device),
                    input_device: ironrdp_rdpsnd_native::DeviceSelector::parse(input_device),
                    latency,
                    attached: false.into(),
                    playback_enabled: playback.into(),
                    capture_enabled: capture.into(),
                    playback_backend: None.into(),
                    next_generation: 0.into(),
                    active_capture: 0.into(),
                    failed_capture: 0.into(),
                    capture_tx,
                    capture_rx: capture_rx.into(),
                }))))
            }
        }
    }
}
