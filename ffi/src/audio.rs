#[cfg(not(target_os = "android"))]
mod native {
    use super::{AudioChannelKind, audio_channel_kind};
    use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
    use core::time::Duration;
    use std::borrow::Cow;
    use std::sync::{Arc, Mutex, PoisonError, Weak, mpsc};
    use std::time::Instant;

    use ironrdp::dvc::{DvcChannelListener, DvcClientProcessor, DvcMessage, DvcProcessor};
    use ironrdp::pdu::PduResult;
    use ironrdp::rdpsnd::client::{Clock, RdpsndClientHandler};
    use ironrdp::rdpsnd::pdu::{AudioFormat, AudioFormatFlags, PitchPdu, VolumePdu};
    use ironrdp::svc::SvcMessage;
    use ironrdp_rdpeai::client::{AudioPacketSink, RdpeaiCaptureHandler, RdpeaiClient};
    use ironrdp_rdpsnd_native::DeviceSelector;
    use ironrdp_rdpsnd_native::capture::{CaptureConfig, CaptureStopSignal, RdpeaiCaptureBackend};
    use ironrdp_rdpsnd_native::cpal::{PlaybackConfig, RdpsndBackend};
    use tracing::warn;

    // Keep this FFI ceiling independent; the capture queue is sized for this rate.
    const MAX_FFI_CAPTURE_SAMPLE_RATE: u32 = 48_000;
    const MAX_CAPTURE_CHANNELS: u16 = 2;
    // A mono Open with one frame per packet can become stereo on FormatChange.
    pub(super) const CAPTURE_QUEUE_CAPACITY: usize = 12_000;
    const CAPTURE_QUEUE_BYTE_LIMIT: usize = 8 * 1024 * 1024;
    const CAPTURE_OUTPUT_LIMIT: usize = 256 * 1024;
    // Account for each message's allocation as well as its encoded bytes.
    const MESSAGE_ALLOCATION_OVERHEAD: usize = 64;

    pub(super) struct CaptureMessages {
        token: u64,
        messages: Vec<SvcMessage>,
        charge: usize,
    }

    type PlaybackBackend = Mutex<Box<dyn RdpsndClientHandler>>;

    pub(crate) struct AudioShared {
        pub(super) playback: bool,
        pub(super) capture: bool,
        pub(super) output_device: DeviceSelector,
        pub(super) input_device: DeviceSelector,
        pub(super) latency: Duration,
        pub(super) attached: AtomicBool,
        pub(super) playback_enabled: AtomicBool,
        pub(super) capture_enabled: AtomicBool,
        /// This slot lets StopAudio close playback without owning the backend.
        pub(super) playback_backend: Mutex<Option<Weak<PlaybackBackend>>>,
        pub(super) next_generation: AtomicU32,
        pub(super) active_capture: AtomicU64,
        pub(super) failed_capture: AtomicU64,
        pub(super) queued_bytes: AtomicUsize,
        pub(super) capture_tx: mpsc::SyncSender<CaptureMessages>,
        /// INVARIANT: Polling is the sole receiver; the mutex lets AudioShared cross callback threads.
        pub(super) capture_rx: Mutex<mpsc::Receiver<CaptureMessages>>,
    }

    impl AudioShared {
        pub(crate) fn has_playback(&self) -> bool {
            self.playback
        }

        pub(crate) fn reserves_dynamic_channel(&self, channel_name: &str) -> bool {
            match audio_channel_kind(channel_name) {
                Some(AudioChannelKind::Playback) => self.playback,
                Some(AudioChannelKind::Capture) => self.capture,
                None => false,
            }
        }

        fn enqueue_capture(&self, token: u64, messages: Vec<SvcMessage>) -> PduResult<()> {
            // UNTESTED: audio callbacks must not wait for the session loop or shutdown; keep this
            // enqueue nonblocking so a stalled transport cannot deadlock capture shutdown.
            if self.failed_capture.load(Ordering::Acquire) == token {
                return Err(ironrdp::pdu::pdu_other_err!("audio capture uplink is stopped"));
            }
            let charge = messages
                .iter()
                .try_fold(size_of::<CaptureMessages>(), |total, message| {
                    total
                        .checked_add(size_of::<SvcMessage>() + MESSAGE_ALLOCATION_OVERHEAD)?
                        .checked_add(message.unframed_pdu_size())
                });
            let reserved = charge.is_some_and(|charge| {
                self.queued_bytes
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                        current
                            .checked_add(charge)
                            .filter(|next| *next <= CAPTURE_QUEUE_BYTE_LIMIT)
                    })
                    .is_ok()
            });
            if !reserved {
                return self.fail_capture_uplink(token);
            }
            let charge = charge.expect("reservation requires a valid charge");
            if self
                .capture_tx
                .try_send(CaptureMessages {
                    token,
                    messages,
                    charge,
                })
                .is_err()
            {
                self.queued_bytes.fetch_sub(charge, Ordering::AcqRel);
                return self.fail_capture_uplink(token);
            }
            Ok(())
        }

        pub(crate) fn stop_playback(&self) {
            self.playback_enabled.store(false, Ordering::Release);
            let backend = self
                .playback_backend
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
                .and_then(Weak::upgrade);
            if let Some(backend) = backend {
                backend.lock().unwrap_or_else(PoisonError::into_inner).close();
            }
        }

        fn fail_capture_uplink(&self, token: u64) -> PduResult<()> {
            self.failed_capture.store(token, Ordering::Release);
            warn!("Audio capture uplink overflow; stopping capture");
            Err(ironrdp::pdu::pdu_other_err!("audio capture uplink overflow"))
        }

        fn try_recv_capture(&self) -> Result<CaptureMessages, mpsc::TryRecvError> {
            let message = self
                .capture_rx
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .try_recv()?;
            self.queued_bytes.fetch_sub(message.charge, Ordering::AcqRel);
            Ok(message)
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
                    && AudioChannelKind::Playback
                        .channel_names()
                        .iter()
                        .any(|name| drdynvc.has_channel_name(name));
                let capture_in_use = self.capture
                    && AudioChannelKind::Capture
                        .channel_names()
                        .iter()
                        .any(|name| drdynvc.has_channel_name(name));
                if playback_in_use || capture_in_use {
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
                    // UNTESTED: Windows servers require RDPDR to start playback.
                    // Verify audible playback with no devices redirected when changing this registration.
                    connector.attach_static_channel(ironrdp::rdpdr::Rdpdr::new(
                        Box::new(ironrdp::rdpdr::NoopRdpdrBackend),
                        connector.config.client_name.clone(),
                    ));
                }
                let config = PlaybackConfig {
                    device: self.output_device.clone(),
                    latency: self.latency,
                };
                let playback = SharedPlayback::new(self, Box::new(RdpsndBackend::with_config(config)));
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
        /// INVARIANT: Session processing, device callbacks, and shutdown serialize access to the backend.
        backend: Arc<PlaybackBackend>,
        formats: Arc<[AudioFormat]>,
        shared: Arc<AudioShared>,
    }

    impl core::fmt::Debug for SharedPlayback {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("SharedPlayback").finish_non_exhaustive()
        }
    }

    impl SharedPlayback {
        fn new(shared: &Arc<AudioShared>, backend: Box<dyn RdpsndClientHandler>) -> Self {
            let formats = backend.get_formats().to_vec().into();
            let backend = Arc::new(Mutex::new(backend));
            *shared.playback_backend.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::downgrade(&backend));
            Self {
                backend,
                formats,
                shared: Arc::clone(shared),
            }
        }

        fn with_backend<T>(&self, f: impl FnOnce(&mut dyn RdpsndClientHandler) -> T) -> T {
            f(self.backend.lock().unwrap_or_else(PoisonError::into_inner).as_mut())
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
            self.with_backend(|backend| backend.close());
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

    struct BoundedCapture(RdpeaiCaptureBackend);

    impl RdpeaiCaptureHandler for BoundedCapture {
        fn supported_formats(&self) -> &[AudioFormat] {
            self.0.supported_formats()
        }

        fn choose_formats(&self, server_formats: &[AudioFormat]) -> Vec<AudioFormat> {
            self.0
                .choose_formats(server_formats)
                .into_iter()
                .filter(capture_format_fits_queue)
                .collect()
        }

        fn open(
            &mut self,
            capture_format: &AudioFormat,
            encode_format: &AudioFormat,
            packet_size: usize,
            sink: AudioPacketSink,
        ) -> i32 {
            if !capture_format_fits_queue(capture_format) {
                warn!("Refusing capture format beyond the audio queue limit");
                return ironrdp_rdpeai::pdu::OpenReplyPdu::fail().result;
            }
            self.0.open(capture_format, encode_format, packet_size, sink)
        }

        fn set_format(&mut self, encode_format: &AudioFormat, packet_size: usize) -> bool {
            self.0.set_format(encode_format, packet_size)
        }

        fn set_format_ordered(
            &mut self,
            encode_format: &AudioFormat,
            packet_size: usize,
            confirm: &mut dyn FnMut() -> PduResult<()>,
        ) -> PduResult<bool> {
            self.0.set_format_ordered(encode_format, packet_size, confirm)
        }

        fn close(&mut self) {
            self.0.close();
        }
    }

    fn capture_format_fits_queue(format: &AudioFormat) -> bool {
        format.n_channels <= MAX_CAPTURE_CHANNELS && format.n_samples_per_sec <= MAX_FFI_CAPTURE_SAMPLE_RATE
    }

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
            let stop_signal = backend.stop_signal();
            let sender = Arc::clone(shared);
            let uplink_stop_signal = stop_signal.clone();
            let client = RdpeaiClient::new(
                Box::new(BoundedCapture(backend)),
                Box::new(move |_, messages| enqueue_capture_for_channel(&sender, token, messages, &uplink_stop_signal)),
            )
            .with_ordered_replies();
            Some(CaptureChannel {
                client,
                token,
                shared: Arc::clone(shared),
                stop_signal,
            })
        }
    }

    fn enqueue_capture_for_channel(
        shared: &AudioShared,
        token: u64,
        messages: Vec<SvcMessage>,
        stop_signal: &CaptureStopSignal,
    ) -> PduResult<()> {
        let result = shared.enqueue_capture(token, messages);
        if result.is_err() {
            stop_signal.request_stop();
        }
        result
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
        stop_signal: CaptureStopSignal,
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
                    self.stop_signal.request_stop();
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
        let mut pending = Vec::new();
        // A failure can follow a queued confirmation, which must be sent before the channel closes.
        for _ in 0..=CAPTURE_QUEUE_CAPACITY {
            let next = shared.try_recv_capture();
            let Ok(next) = next else {
                let failed = shared.failed_capture.load(Ordering::Acquire);
                let failure = match failed {
                    0 => None,
                    failed if shared.active_capture.load(Ordering::Acquire) == failed => Some(failed),
                    failed => {
                        let _ = shared
                            .failed_capture
                            .compare_exchange(failed, 0, Ordering::AcqRel, Ordering::Acquire);
                        None
                    }
                };
                if let Some(failure) = failure {
                    shared.capture_enabled.store(false, Ordering::Release);
                    flush_capture_output(&mut outputs, &mut pending);
                    append_capture_close(&mut outputs, shared, failure, close_capture(stage, shared, failure))?;
                }
                break;
            };
            if shared.active_capture.load(Ordering::Acquire) != next.token {
                continue;
            }
            let result = stage
                .encode_dvc_messages(next.messages)
                .map_err(|error| error.report().to_string())
                .and_then(|frame| append_capture_output(&mut outputs, &mut pending, &frame).map_err(str::to_owned));
            if let Err(error) = result {
                shared.capture_enabled.store(false, Ordering::Release);
                shared.failed_capture.store(next.token, Ordering::Release);
                warn!(error, "Failed to process audio capture frame; stopping capture");
                flush_capture_output(&mut outputs, &mut pending);
                append_capture_close(
                    &mut outputs,
                    shared,
                    next.token,
                    close_capture(stage, shared, next.token),
                )?;
                return Ok(outputs);
            }
        }
        flush_capture_output(&mut outputs, &mut pending);
        Ok(outputs)
    }

    fn append_capture_close(
        outputs: &mut Vec<ironrdp::session::ActiveStageOutput>,
        shared: &AudioShared,
        token: u64,
        close_result: Result<Vec<ironrdp::session::ActiveStageOutput>, Box<crate::error::ffi::IronRdpError>>,
    ) -> Result<(), Box<crate::error::ffi::IronRdpError>> {
        match close_result {
            Ok(close_outputs) => {
                outputs.extend(close_outputs);
                let _ = shared
                    .failed_capture
                    .compare_exchange(token, 0, Ordering::AcqRel, Ordering::Acquire);
                Ok(())
            }
            Err(_) if !outputs.is_empty() => {
                warn!("Failed to close audio capture channel; retrying after queued output");
                // INVARIANT: Keep the failure token set so the next poll retries the channel close.
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn append_capture_output(
        outputs: &mut Vec<ironrdp::session::ActiveStageOutput>,
        pending: &mut Vec<u8>,
        mut frame: &[u8],
    ) -> Result<(), &'static str> {
        while !frame.is_empty() {
            let header = ironrdp::pdu::tpkt::TpktHeader::read(&mut ironrdp_core::ReadCursor::new(frame))
                .map_err(|_| "invalid TPKT header")?;
            let length = header.packet_length();
            if frame.len() < length {
                return Err("truncated TPKT frame");
            }
            if CAPTURE_OUTPUT_LIMIT.saturating_sub(pending.len()) < length {
                flush_capture_output(outputs, pending);
            }
            pending.extend_from_slice(&frame[..length]);
            frame = &frame[length..];
        }
        Ok(())
    }

    fn flush_capture_output(outputs: &mut Vec<ironrdp::session::ActiveStageOutput>, pending: &mut Vec<u8>) {
        if !pending.is_empty() {
            outputs.push(ironrdp::session::ActiveStageOutput::ResponseFrame(core::mem::take(
                pending,
            )));
        }
    }

    pub(crate) fn stop_audio(
        stage: &mut ironrdp::session::ActiveStage,
        shared: &Arc<AudioShared>,
    ) -> Result<Vec<ironrdp::session::ActiveStageOutput>, Box<crate::error::ffi::IronRdpError>> {
        shared.capture_enabled.store(false, Ordering::Release);
        shared.playback_enabled.store(false, Ordering::Release);
        let token = shared.active_capture.load(Ordering::Acquire);
        if token != 0 {
            let channel_id = u32::try_from(token & u64::from(u32::MAX)).expect("channel id fits in u32");
            if let Some(drdynvc) = stage.get_svc_processor_mut::<ironrdp::dvc::DrdynvcClient>()
                && let Some(mut channel) = drdynvc.get_dvc_by_channel_id_mut::<CaptureChannel>(channel_id)
                && channel.processor().token == token
                && Arc::ptr_eq(&channel.processor().shared, shared)
            {
                channel.processor_mut().close(channel_id);
            }
        }
        let capture_result = (|| {
            let mut outputs = poll_audio(stage, shared)?;
            let token = shared.active_capture.load(Ordering::Acquire);
            if token != 0 {
                shared.failed_capture.store(token, Ordering::Release);
                append_capture_close(&mut outputs, shared, token, close_capture(stage, shared, token))?;
            }
            Ok(outputs)
        })();
        shared.stop_playback();
        capture_result
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
        // Encode the same PDU as close_channel before removing the channel so a failure can be retried.
        let close = SvcMessage::from(ironrdp::dvc::pdu::DrdynvcClientPdu::Close(
            ironrdp::dvc::pdu::ClosePdu::new(channel_id),
        ));
        let frame = stage.encode_dvc_messages(vec![close])?;
        let drdynvc = stage
            .get_svc_processor_mut::<ironrdp::dvc::DrdynvcClient>()
            .ok_or("DRDYNVC channel is not available")?;
        drdynvc
            .close_channel(channel_id)
            .ok_or("audio capture channel is not available")?;
        Ok(vec![ironrdp::session::ActiveStageOutput::ResponseFrame(frame)])
    }

    #[cfg(test)]
    mod playback_tests {
        use super::*;
        use crate::audio::ffi::NativeAudio;
        use crate::connector::config::ffi::{ConfigBuilder, PerformanceFlags};
        use crate::session::ffi::ActiveStage;

        #[derive(Clone, Copy, Debug, Default)]
        struct PlaybackState {
            active: bool,
            waves: usize,
            closed_active: usize,
        }

        #[derive(Debug)]
        struct RecordingPlayback(Arc<Mutex<PlaybackState>>);

        impl RdpsndClientHandler for RecordingPlayback {
            fn get_formats(&self) -> &[AudioFormat] {
                &[]
            }

            fn wave(&mut self, _: &AudioFormat, _: u32, _: Cow<'_, [u8]>) {
                let mut state = self.0.lock().unwrap();
                state.active = true;
                state.waves += 1;
            }

            fn set_volume(&mut self, _: VolumePdu) {}

            fn set_pitch(&mut self, _: PitchPdu) {}

            fn close(&mut self) {
                let mut state = self.0.lock().unwrap();
                if state.active {
                    state.active = false;
                    state.closed_active += 1;
                }
            }
        }

        fn playback_stage() -> (ActiveStage, Box<NativeAudio>, SharedPlayback, Arc<Mutex<PlaybackState>>) {
            let audio = NativeAudio::new(true, false, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let observed = Arc::new(Mutex::new(PlaybackState::default()));
            let playback = SharedPlayback::new(&audio.0, Box::new(RecordingPlayback(Arc::clone(&observed))));
            let mut channels = ironrdp::svc::StaticChannelSet::new();
            channels.insert(rdpsnd_with_clock(Box::new(playback.clone())));
            let stage = ironrdp::session::ActiveStageBuilder {
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
            let mut builder = ConfigBuilder::new();
            builder.with_username_and_password("user", "password");
            builder.set_desktop_size(768, 1024);
            builder.set_client_name("client");
            builder.set_client_dir("C:\\");
            builder.set_performance_flags(&PerformanceFlags::new_default());
            let config = builder.build().unwrap_or_else(|_| panic!("connector config"));
            let factory = ironrdp::connector::connection_activation::ConnectionActivationFactory::new(
                config.connector.clone(),
                1003,
                1002,
            );
            (
                ActiveStage(stage, factory, Some(Arc::clone(&audio.0))),
                audio,
                playback,
                observed,
            )
        }

        fn play_wave(playback: &mut SharedPlayback) {
            let format = AudioFormat {
                format: ironrdp::rdpsnd::pdu::WaveFormat::PCM,
                n_channels: 1,
                n_samples_per_sec: 8_000,
                n_avg_bytes_per_sec: 16_000,
                n_block_align: 2,
                bits_per_sample: 16,
                data: None,
            };
            playback.wave(&format, 0, Cow::Borrowed(&[0, 0]));
        }

        // oracle: regression Devolutions/IronRDP#2480 review 5980962351
        #[test]
        fn stopping_audio_closes_active_playback() {
            let (mut stage, audio, mut playback, observed) = playback_stage();
            play_wave(&mut playback);
            assert!(observed.lock().unwrap().active);

            stage.stop_audio(&audio).unwrap_or_else(|_| panic!("stop audio"));

            let observed = *observed.lock().unwrap();
            assert!(!observed.active);
            assert_eq!(observed.closed_active, 1);
        }

        // oracle: regression Devolutions/IronRDP#2480 review 5980962351
        #[test]
        fn stopping_audio_permanently_suppresses_later_waves() {
            let (mut stage, audio, mut playback, observed) = playback_stage();
            play_wave(&mut playback);
            stage.stop_audio(&audio).unwrap_or_else(|_| panic!("stop audio"));

            play_wave(&mut playback);
            play_wave(&mut playback);

            let observed = *observed.lock().unwrap();
            assert!(!observed.active);
            assert_eq!(observed.waves, 1);
        }

        // oracle: regression Devolutions/IronRDP#2480 review 5980962351
        #[test]
        fn dropping_stage_closes_playback_and_suppresses_retained_handlers() {
            let (stage, _audio, mut playback, observed) = playback_stage();
            play_wave(&mut playback);
            assert!(observed.lock().unwrap().active);

            drop(stage);

            {
                let observed = *observed.lock().unwrap();
                assert!(!observed.active);
                assert_eq!(observed.closed_active, 1);
            }
            play_wave(&mut playback);
            let observed = *observed.lock().unwrap();
            assert!(!observed.active);
            assert_eq!(observed.waves, 1);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::audio::ffi::NativeAudio;

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

        fn stage_with_capture_channel(channel: CaptureChannel) -> ironrdp::session::ActiveStage {
            stage_with_capture_channel_id(channel, Some(1004))
        }

        fn stage_with_capture_channel_id(
            channel: CaptureChannel,
            channel_id: Option<u16>,
        ) -> ironrdp::session::ActiveStage {
            let mut drdynvc = ironrdp::dvc::DrdynvcClient::new();
            drdynvc
                .attach_established_dynamic_channel(7, channel)
                .expect("attach capture channel");
            let mut channels = ironrdp::svc::StaticChannelSet::new();
            channels.insert(drdynvc);
            if let Some(channel_id) = channel_id {
                channels.attach_channel_id(core::any::TypeId::of::<ironrdp::dvc::DrdynvcClient>(), channel_id);
            }
            ironrdp::session::ActiveStageBuilder {
                static_channels: channels,
                user_channel_id: 1002,
                io_channel_id: 1003,
                message_channel_id: None,
                share_id: 1,
                compression_type: None,
                enable_server_pointer: false,
                pointer_software_rendering: false,
            }
            .build()
        }

        // oracle: user decision to retry closing after queued output is returned
        #[test]
        fn failed_capture_close_encoding_keeps_channel_for_retry() {
            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let listener = CaptureListener(Arc::clone(&audio.0));
            let channel = listener.create_channel(7).expect("capture channel");
            let token = channel.token;
            let mut stage = stage_with_capture_channel_id(channel, None);

            assert!(close_capture(&mut stage, &audio.0, token).is_err());
            assert_eq!(audio.0.active_capture.load(Ordering::Acquire), token);
            assert!(stage.get_dvc_by_channel_id::<CaptureChannel>(7).is_some());
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
        fn capture_queue_accepts_100ms_after_mono_to_stereo_change() {
            use ironrdp::svc::ChannelFlags;
            use ironrdp_rdpeai::pdu::{DataPdu, RdpeaiPdu};

            // A mono Open with one frame per packet fixes the packet size at two bytes.
            // At 48 kHz stereo, each frame produces two such packets.
            let poll_batch_count = 48_000 * 2 / 10;
            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let listener = CaptureListener(Arc::clone(&audio.0));
            let channel = listener.create_channel(7).expect("capture channel");
            let token = channel.token;
            let mut stage = stage_with_capture_channel(channel);

            let mut expected = Vec::new();
            for index in 0..poll_batch_count {
                let packet = u16::try_from(index).expect("packet index").to_le_bytes();
                let make_messages = || {
                    ironrdp::dvc::encode_dvc_messages(
                        7,
                        vec![
                            Box::new(RdpeaiPdu::DataIncoming),
                            Box::new(RdpeaiPdu::Data(DataPdu::new(packet.to_vec()))),
                        ],
                        ChannelFlags::empty(),
                    )
                    .expect("encode capture batch")
                };
                expected.extend(
                    stage
                        .encode_dvc_messages(make_messages())
                        .expect("encode expected frame"),
                );
                audio
                    .0
                    .enqueue_capture(token, make_messages())
                    .expect("100 ms of maximum-rate capture fits");
            }

            let outputs = poll_audio(&mut stage, &audio.0).unwrap_or_else(|_| panic!("poll audio"));
            assert!(outputs.len() < 16);
            let mut actual = Vec::new();
            for output in outputs {
                let ironrdp::session::ActiveStageOutput::ResponseFrame(frame) = output else {
                    panic!("expected response frame");
                };
                assert!(frame.len() <= CAPTURE_OUTPUT_LIMIT);
                actual.extend(frame);
            }
            assert_eq!(actual, expected);
            assert_eq!(audio.0.queued_bytes.load(Ordering::Acquire), 0);
        }

        // oracle: regression #2469
        #[test]
        fn large_encoded_batch_splits_only_between_tpkts() {
            let mut input = Vec::new();
            for value in 0..5u8 {
                input.extend_from_slice(&[3, 0, 0xff, 0xff]);
                input.extend(core::iter::repeat_n(value, 65_531));
            }
            let mut outputs = Vec::new();
            let mut pending = Vec::new();
            append_capture_output(&mut outputs, &mut pending, &input).expect("append valid TPKTs");
            flush_capture_output(&mut outputs, &mut pending);

            assert_eq!(outputs.len(), 2);
            let mut actual = Vec::new();
            for output in outputs {
                let ironrdp::session::ActiveStageOutput::ResponseFrame(frame) = output else {
                    panic!("expected response frame");
                };
                assert!(frame.len() <= CAPTURE_OUTPUT_LIMIT);
                assert_eq!(frame.len() % 65_535, 0);
                actual.extend(frame);
            }
            assert_eq!(actual, input);
        }

        // oracle: regression #2469
        #[test]
        fn capture_queue_bounds_large_packets_and_releases_capacity() {
            use ironrdp::svc::ChannelFlags;
            use ironrdp_rdpeai::pdu::{DataPdu, MAX_DATA_PACKET_SIZE, RdpeaiPdu};

            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let backend = RdpeaiCaptureBackend::new();
            let stop_signal = backend.stop_signal();
            let token = (1u64 << 32) | 7;
            let packet = || {
                ironrdp::dvc::encode_dvc_messages(
                    7,
                    vec![
                        Box::new(RdpeaiPdu::DataIncoming),
                        Box::new(RdpeaiPdu::Data(DataPdu::new(vec![0; MAX_DATA_PACKET_SIZE]))),
                    ],
                    ChannelFlags::empty(),
                )
                .expect("encode capture batch")
            };
            let mut accepted = 0;
            while enqueue_capture_for_channel(&audio.0, token, packet(), &stop_signal).is_ok() {
                accepted += 1;
            }
            assert!(accepted > 0 && accepted < 100);
            assert!(stop_signal.is_requested());
            assert!(audio.0.queued_bytes.load(Ordering::Acquire) <= CAPTURE_QUEUE_BYTE_LIMIT);
            while audio.0.try_recv_capture().is_ok() {}
            assert_eq!(audio.0.queued_bytes.load(Ordering::Acquire), 0);
        }

        // oracle: regression #2469
        #[test]
        fn capture_message_queue_overflow_requests_worker_stop() {
            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let backend = RdpeaiCaptureBackend::new();
            let stop_signal = backend.stop_signal();
            let token = (1u64 << 32) | 7;
            for _ in 0..CAPTURE_QUEUE_CAPACITY {
                audio.0.enqueue_capture(token, Vec::new()).expect("queue has space");
            }

            assert!(!stop_signal.is_requested());
            assert!(enqueue_capture_for_channel(&audio.0, token, Vec::new(), &stop_signal).is_err());
            assert!(stop_signal.is_requested());
        }

        // oracle: regression #2469
        #[test]
        fn ffi_capture_limits_server_formats_and_open_capture_format() {
            use ironrdp_rdpeai::pdu::{OpenReplyPdu, pcm_format};

            let mut backend = BoundedCapture(RdpeaiCaptureBackend::new());
            let supported = pcm_format(2, 48_000, 16);
            let too_fast = pcm_format(1, 192_000, 16);
            let too_many_channels = pcm_format(3, 48_000, 16);
            assert_eq!(
                backend.choose_formats(&[supported.clone(), too_fast.clone(), too_many_channels]),
                core::slice::from_ref(&supported)
            );
            assert_eq!(
                backend.open(&too_fast, &supported, 2, Box::new(|_| {})),
                OpenReplyPdu::E_FAIL
            );
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

        struct FailedRestartCapture {
            formats: Vec<AudioFormat>,
        }

        impl RdpeaiCaptureHandler for FailedRestartCapture {
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
                Ok(false)
            }

            fn close(&mut self) {}
        }

        // oracle: regression #2469
        #[test]
        fn failed_restart_sends_confirmation_before_closing_capture() {
            assert_failed_restart_confirmation_precedes_close(false);
        }

        // oracle: regression #2469
        #[test]
        fn stopping_after_failed_restart_sends_confirmation_before_closing_capture() {
            assert_failed_restart_confirmation_precedes_close(true);
        }

        // oracle: regression #2469
        #[test]
        fn dropping_active_stage_closes_capture_before_waiting_for_playback() {
            struct StopSignal(mpsc::Sender<()>);

            impl RdpeaiCaptureHandler for StopSignal {
                fn supported_formats(&self) -> &[AudioFormat] {
                    &[]
                }

                fn open(&mut self, _: &AudioFormat, _: &AudioFormat, _: usize, _: AudioPacketSink) -> i32 {
                    ironrdp_rdpeai::pdu::OpenReplyPdu::E_FAIL
                }

                fn set_format(&mut self, _: &AudioFormat, _: usize) -> bool {
                    false
                }

                fn close(&mut self) {
                    let _ = self.0.send(());
                }
            }

            let audio = NativeAudio::new(true, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let token = (1u64 << 32) | 7;
            audio.0.active_capture.store(token, Ordering::Release);
            let (closed_tx, closed_rx) = mpsc::channel();
            let client = RdpeaiClient::new(Box::new(StopSignal(closed_tx)), Box::new(|_, _| Ok(())));
            let channel = CaptureChannel {
                client,
                token,
                shared: Arc::clone(&audio.0),
                stop_signal: RdpeaiCaptureBackend::new().stop_signal(),
            };
            let stage = stage_with_capture_channel(channel);
            let stage = crate::session::ffi::ActiveStage(
                stage,
                {
                    let mut builder = crate::connector::config::ffi::ConfigBuilder::default();
                    builder.with_username_and_password("user", "password");
                    builder.set_desktop_size(4, 8);
                    builder.set_client_name("test");
                    builder.set_client_dir("");
                    let performance_flags = crate::connector::config::ffi::PerformanceFlags::new_default();
                    builder.set_performance_flags(&performance_flags);
                    let config = builder.build().unwrap_or_else(|_| panic!("build test config"));
                    ironrdp::connector::connection_activation::ConnectionActivationFactory::new(config.connector, 0, 0)
                },
                Some(Arc::clone(&audio.0)),
            );
            let backend = Arc::new(Mutex::new(RdpsndBackend::with_config(PlaybackConfig::default())));
            *audio.0.playback_backend.lock().unwrap() = Some(Arc::downgrade(&backend));
            let playback_guard = backend.lock().unwrap();
            let worker = std::thread::spawn(move || drop(stage));
            let closed_before_unlock = closed_rx.recv_timeout(Duration::from_secs(2)).is_ok();
            drop(playback_guard);
            worker.join().expect("drop active stage");
            assert!(closed_before_unlock);
        }

        fn assert_failed_restart_confirmation_precedes_close(stop: bool) {
            use ironrdp::svc::ChannelFlags;
            use ironrdp_rdpeai::pdu::{
                FormatChangePdu, FormatsPdu, OpenPdu, RdpeaiPdu, Version, VersionPdu, pcm_format,
            };

            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let token = (1u64 << 32) | 7;
            audio.0.active_capture.store(token, Ordering::Release);
            let formats = vec![pcm_format(1, 16_000, 16), pcm_format(1, 48_000, 16)];
            let sender = Arc::clone(&audio.0);
            let client = RdpeaiClient::new(
                Box::new(FailedRestartCapture {
                    formats: formats.clone(),
                }),
                Box::new(move |_, messages| sender.enqueue_capture(token, messages)),
            )
            .with_ordered_replies();
            let mut channel = CaptureChannel {
                client,
                token,
                shared: Arc::clone(&audio.0),
                stop_signal: RdpeaiCaptureBackend::new().stop_signal(),
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
                channel
                    .process(7, &ironrdp_core::encode_vec(&pdu).expect("encode PDU"))
                    .expect("process PDU");
            }
            while audio.0.try_recv_capture().is_ok() {}
            let change = RdpeaiPdu::FormatChange(FormatChangePdu::new(1));
            channel
                .process(7, &ironrdp_core::encode_vec(&change).expect("encode PDU"))
                .expect("isolate failed restart");
            assert_eq!(audio.0.failed_capture.load(Ordering::Acquire), token);

            let mut stage = stage_with_capture_channel(channel);
            let confirmation = ironrdp::dvc::encode_dvc_messages(7, vec![Box::new(change)], ChannelFlags::empty())
                .expect("encode confirmation");
            let expected = stage.encode_dvc_messages(confirmation).expect("frame confirmation");

            let outputs = if stop {
                stop_audio(&mut stage, &audio.0)
            } else {
                poll_audio(&mut stage, &audio.0)
            }
            .unwrap_or_else(|_| panic!("flush audio"));
            let [
                ironrdp::session::ActiveStageOutput::ResponseFrame(first),
                ironrdp::session::ActiveStageOutput::ResponseFrame(_close),
            ] = outputs.as_slice()
            else {
                panic!("expected confirmation and close frames");
            };
            assert_eq!(first, &expected);
            assert!(stage.get_dvc_by_channel_id::<CaptureChannel>(7).is_none());
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
                stop_signal: RdpeaiCaptureBackend::new().stop_signal(),
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
            while audio.0.try_recv_capture().is_ok() {}

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

            let mut stage = stage_with_capture_channel(channel);

            let outputs = poll_audio(&mut stage, &audio.0).unwrap_or_else(|_| panic!("poll audio"));
            assert!(matches!(
                outputs.last(),
                Some(ironrdp::session::ActiveStageOutput::ResponseFrame(_))
            ));
            assert!(stage.get_dvc_by_channel_id::<CaptureChannel>(7).is_none());
            assert_eq!(audio.0.active_capture.load(Ordering::Acquire), 0);
        }

        // oracle: user decision to return queued output before retrying capture close
        #[test]
        fn capture_close_failure_preserves_queued_output_for_retry() {
            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let token = (1u64 << 32) | 7;
            audio.0.failed_capture.store(token, Ordering::Release);
            let frame = ironrdp::session::ActiveStageOutput::ResponseFrame(vec![0x01, 0x02]);
            let mut outputs = vec![frame];

            let result = append_capture_close(&mut outputs, &audio.0, token, Err("DVC close encoding failed".into()));
            match result {
                Ok(()) => {}
                Err(_) => panic!("queued output defers capture close failure"),
            };

            assert_eq!(audio.0.failed_capture.load(Ordering::Acquire), token);
            assert!(matches!(
                outputs.as_slice(),
                [ironrdp::session::ActiveStageOutput::ResponseFrame(frame)] if frame == &[0x01, 0x02]
            ));
        }

        // oracle: user decision to surface capture close errors when no output is ready
        #[test]
        fn capture_close_failure_is_returned_without_queued_output() {
            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let token = (1u64 << 32) | 7;
            audio.0.failed_capture.store(token, Ordering::Release);
            let mut outputs = Vec::new();

            assert!(
                append_capture_close(&mut outputs, &audio.0, token, Err("DVC close encoding failed".into()),).is_err()
            );
            assert_eq!(audio.0.failed_capture.load(Ordering::Acquire), token);
        }

        // oracle: regression #2469
        #[test]
        fn polling_an_overflow_closes_capture_without_closing_the_session() {
            let audio = NativeAudio::new(false, true, "", "", 0).unwrap_or_else(|_| panic!("audio settings"));
            let listener = CaptureListener(Arc::clone(&audio.0));
            let channel = listener.create_channel(7).expect("capture channel");
            let token = channel.token;
            let mut stage = stage_with_capture_channel(channel);
            for _ in 0..CAPTURE_QUEUE_CAPACITY {
                audio.0.enqueue_capture(token, Vec::new()).expect("queue has space");
            }
            assert!(audio.0.enqueue_capture(token, Vec::new()).is_err());
            let outputs = poll_audio(&mut stage, &audio.0).unwrap_or_else(|_| panic!("poll audio"));
            assert!(matches!(
                outputs.last(),
                Some(ironrdp::session::ActiveStageOutput::ResponseFrame(_))
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AudioChannelKind {
    Playback,
    Capture,
}

impl AudioChannelKind {
    pub(crate) fn channel_names(self) -> &'static [&'static str] {
        match self {
            Self::Playback => &[
                ironrdp::rdpsnd::client::DVC_CHANNEL_NAME,
                ironrdp::rdpsnd::client::LOSSY_DVC_CHANNEL_NAME,
            ],
            Self::Capture => &[ironrdp_rdpeai::CHANNEL_NAME],
        }
    }
}

pub(crate) fn audio_channel_kind(channel_name: &str) -> Option<AudioChannelKind> {
    [AudioChannelKind::Playback, AudioChannelKind::Capture]
        .into_iter()
        .find(|kind| kind.channel_names().contains(&channel_name))
}

#[diplomat::bridge]
pub mod ffi {
    use std::sync::Arc;

    use super::AudioShared;
    use crate::error::ffi::IronRdpError;

    /// This type owns a bounded capture queue when capture is enabled.
    /// The .NET garbage collector cannot see that allocation.
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
                    queued_bytes: 0.into(),
                    capture_tx,
                    capture_rx: capture_rx.into(),
                }))))
            }
        }
    }
}
