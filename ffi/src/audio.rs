#[cfg(not(target_os = "android"))]
mod native {
    use core::sync::atomic::{AtomicBool, Ordering};
    use core::time::Duration;
    use std::borrow::Cow;
    use std::sync::{Arc, Mutex, PoisonError, Weak};
    use std::time::Instant;

    use ironrdp::dvc::{DvcChannelListener, DvcClientProcessor};
    use ironrdp::rdpsnd::client::{Clock, RdpsndClientHandler};
    use ironrdp::rdpsnd::pdu::{AudioFormat, AudioFormatFlags, PitchPdu, VolumePdu};
    use ironrdp_rdpsnd_native::DeviceSelector;
    use ironrdp_rdpsnd_native::cpal::{PlaybackConfig, RdpsndBackend};

    use super::PLAYBACK_CHANNEL_NAMES;

    pub(crate) struct AudioShared {
        pub(super) output_device: DeviceSelector,
        pub(super) latency: Duration,
        pub(super) attached: AtomicBool,
        pub(super) playback_enabled: AtomicBool,
        /// This slot lets StopAudio close playback without owning the backend.
        pub(super) playback_backend: Mutex<Option<Weak<Mutex<RdpsndBackend>>>>,
    }

    impl AudioShared {
        pub(crate) fn attach(
            self: &Arc<Self>,
            connector: &mut ironrdp::connector::ClientConnector,
        ) -> Result<(), &'static str> {
            let channels = &connector.static_channels;
            let drdynvc_type = channels.get_by_type::<ironrdp::dvc::DrdynvcClient>().is_some();
            let rdpdr_type = channels.get_by_type::<ironrdp::rdpdr::Rdpdr>().is_some();
            if channels
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
            if !rdpdr_type && channels.get_by_channel_name_key(&ironrdp::rdpdr::Rdpdr::NAME).is_some() {
                return Err("RDPDR channel name is already registered");
            }
            let needed = 1 + usize::from(!drdynvc_type) + usize::from(!rdpdr_type);
            if ironrdp::svc::MAX_STATIC_CHANNELS < channels.len() + needed {
                return Err("no space for native audio channels");
            }
            if let Some(drdynvc) = connector.get_static_channel_processor_mut::<ironrdp::dvc::DrdynvcClient>()
                && PLAYBACK_CHANNEL_NAMES.iter().any(|name| drdynvc.has_channel_name(name))
            {
                return Err("native audio dynamic channel is already registered");
            }
            if self.attached.swap(true, Ordering::AcqRel) {
                return Err("native audio is already attached");
            }

            connector.config.enable_audio_playback = true;
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
            drdynvc.attach_listener(PlaybackListener {
                state: state.clone(),
                lossy: false,
            });
            drdynvc.attach_listener(PlaybackListener { state, lossy: true });
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
}

#[cfg(not(target_os = "android"))]
pub(crate) use native::AudioShared;

#[cfg(target_os = "android")]
pub(crate) struct AudioShared;

#[cfg(target_os = "android")]
impl AudioShared {
    pub(crate) fn attach(
        self: &std::sync::Arc<Self>,
        _connector: &mut ironrdp::connector::ClientConnector,
    ) -> Result<(), &'static str> {
        Err("native audio is not supported on Android")
    }

    pub(crate) fn stop_playback(&self) {}
}

pub(crate) const PLAYBACK_CHANNEL_NAMES: [&str; 2] = [
    ironrdp::rdpsnd::client::DVC_CHANNEL_NAME,
    ironrdp::rdpsnd::client::LOSSY_DVC_CHANNEL_NAME,
];

#[diplomat::bridge]
pub mod ffi {
    use std::sync::Arc;

    use super::AudioShared;
    use crate::error::ffi::IronRdpError;

    #[diplomat::opaque]
    pub struct NativeAudio(pub(crate) Arc<AudioShared>);

    impl NativeAudio {
        /// Creates native audio playback settings for one connection.
        ///
        /// An empty device selector follows the system default.
        /// A latency of zero selects the native default.
        pub fn new(output_device: &str, latency_ms: u32) -> Result<Box<Self>, Box<IronRdpError>> {
            #[cfg(target_os = "android")]
            {
                let _ = (output_device, latency_ms);
                Err("native audio is not supported on Android".into())
            }
            #[cfg(not(target_os = "android"))]
            {
                let latency = if latency_ms == 0 {
                    ironrdp_rdpsnd_native::cpal::DEFAULT_PLAYBACK_LATENCY
                } else {
                    core::time::Duration::from_millis(u64::from(latency_ms))
                };
                Ok(Box::new(Self(Arc::new(AudioShared {
                    output_device: ironrdp_rdpsnd_native::DeviceSelector::parse(output_device),
                    latency,
                    attached: false.into(),
                    playback_enabled: true.into(),
                    playback_backend: None.into(),
                }))))
            }
        }
    }
}
