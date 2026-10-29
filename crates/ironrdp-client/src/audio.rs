//! Audio output ([MS-RDPEA]) and audio input ([MS-RDPEAI]) wiring.
//!
//! One playback backend serves the static `RDPSND` channel and the `AUDIO_PLAYBACK_DVC` and
//! `AUDIO_PLAYBACK_LOSSY_DVC` dynamic channels. Windows 8 and later servers use the dynamic channel,
//! which rides the UDP transport when one is up, and keep the static channel as the fallback.
//!
//! [MS-RDPEA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/bea2d5cf-e3b9-4419-92e5-0e074ff9bc5b
//! [MS-RDPEAI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeai/2eb8be0c-4f17-418b-9911-edb8d2ffcde5

use core::time::Duration;
use std::borrow::Cow;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use ironrdp_dvc::{DrdynvcClient, DvcChannelListener, DvcClientProcessor};
use ironrdp_pdu::pdu_other_err;
use ironrdp_rdpsnd::client::{Clock, DVC_CHANNEL_NAME, LOSSY_DVC_CHANNEL_NAME, RdpsndClientHandler, RdpsndDvcState};
use ironrdp_rdpsnd::pdu::{AudioFormat, AudioFormatFlags, PitchPdu, QualityMode, VolumePdu};
use ironrdp_rdpsnd_native::DeviceSelector;
use ironrdp_rdpsnd_native::RdpeaiCaptureBackend;
use ironrdp_rdpsnd_native::capture::CaptureConfig;
use ironrdp_rdpsnd_native::cpal::{PlaybackConfig, RdpsndBackend};

use crate::rdp::{RdpInputEvent, RdpInputSender};

pub use ironrdp_rdpsnd_native::{AudioDeviceInfo, Direction, list_devices};

/// `.rdp` property naming the audio output device (IronRDP extension).
pub const OUTPUT_DEVICE_PROPERTY: &str = "ironrdp_audiooutputdevice";

/// `.rdp` property naming the audio input device (IronRDP extension).
pub const INPUT_DEVICE_PROPERTY: &str = "ironrdp_audioinputdevice";

/// `.rdp` property holding the playback latency target in milliseconds (IronRDP extension).
pub const LATENCY_PROPERTY: &str = "ironrdp_audiolatency";

/// Audio device and buffering settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioConfig {
    /// Output device: a device ID or (part of) a device name. `None` follows the system default.
    pub output_device: Option<String>,
    /// Input device: a device ID or (part of) a device name. `None` follows the system default.
    pub input_device: Option<String>,
    /// Audio kept buffered ahead of the output device. `None` uses the backend default (100 ms).
    pub latency: Option<Duration>,
}

impl AudioConfig {
    fn playback(&self) -> PlaybackConfig {
        let mut config = PlaybackConfig::default();
        if let Some(device) = &self.output_device {
            config.device = DeviceSelector::parse(device);
        }
        if let Some(latency) = self.latency {
            config.latency = latency;
        }
        config
    }

    fn capture(&self) -> CaptureConfig {
        CaptureConfig {
            device: self
                .input_device
                .as_deref()
                .map_or(DeviceSelector::Default, DeviceSelector::parse),
        }
    }
}

/// Monotonic clock for Wave Confirm timing.
pub(crate) fn clock() -> Clock {
    let origin = Instant::now();
    Box::new(move || origin.elapsed())
}

/// Playback backend shared by the static and dynamic audio output channels.
#[derive(Debug, Clone)]
pub(crate) struct SharedPlayback {
    backend: Arc<Mutex<RdpsndBackend>>,
    /// Snapshot of the backend's fallback format list, which never changes.
    formats: Arc<[AudioFormat]>,
}

impl SharedPlayback {
    pub(crate) fn new(config: &AudioConfig) -> Self {
        let backend = RdpsndBackend::with_config(config.playback());
        let formats = backend.get_formats().into();
        Self {
            backend: Arc::new(Mutex::new(backend)),
            formats,
        }
    }

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
        self.with_backend(|backend| backend.wave(format, ts, data));
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

/// Registers the audio output dynamic channels, backed by `playback`.
///
/// Both channels share one negotiation state; see [`RdpsndDvcState`].
pub(crate) fn attach_playback(
    drdynvc: DrdynvcClient,
    playback: &SharedPlayback,
    quality_mode: QualityMode,
) -> DrdynvcClient {
    let state = RdpsndDvcState::new(Box::new(playback.clone()))
        .with_quality_mode(quality_mode)
        .with_clock(clock());
    let lossy = state.clone();
    drdynvc
        .with_dynamic_channel_factory(DVC_CHANNEL_NAME, move || state.channel())
        .with_dynamic_channel_factory(LOSSY_DVC_CHANNEL_NAME, move || lossy.lossy_channel())
}

/// Listener that hands out a fresh [`ironrdp_rdpeai::client::RdpeaiClient`] on every
/// `DVC Create Request` for the AUDIO_INPUT channel.
///
/// Windows opens and closes the AUDIO_INPUT (MS-RDPEAI) dynamic virtual channel repeatedly
/// over the lifetime of a session, as applications on the remote side grab and release the
/// microphone. Registering this channel via `with_dynamic_channel` (the `OnceListener` path)
/// hands out its `RdpeaiClient` exactly once: after the first Close, `OnceListener::create`
/// keeps returning `None`, so the client answers every later Create Request with
/// `NO_LISTENER` and the microphone silently stops working for the rest of the session.
/// `with_listener` + this factory recreates the processor (and the underlying capture
/// backend/cpal stream) on every open, matching how the RDPEWA listener is wired.
pub(crate) struct AudioInputListener {
    pub(crate) sender: RdpInputSender,
    pub(crate) config: AudioConfig,
}

impl DvcChannelListener for AudioInputListener {
    fn channel_name(&self) -> &str {
        ironrdp_rdpeai::CHANNEL_NAME
    }

    fn create(&mut self, _channel_id: ironrdp_dvc::DynamicChannelId) -> Option<Box<dyn DvcClientProcessor>> {
        let sender = self.sender.clone();
        Some(Box::new(ironrdp_rdpeai::client::RdpeaiClient::new(
            Box::new(RdpeaiCaptureBackend::with_config(self.config.capture())),
            Box::new(move |channel_id, messages| {
                sender
                    .try_send(RdpInputEvent::SendDvcMessages { channel_id, messages })
                    .map_err(|_| pdu_other_err!("send AUDIO_INPUT messages to the event loop"))?;
                Ok(())
            }),
        )))
    }

    fn is_available(&self) -> bool {
        true
    }
}
