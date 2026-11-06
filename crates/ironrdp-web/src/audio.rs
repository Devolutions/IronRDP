//! Audio playback (MS-RDPEA over the RDPSND static channel).
//!
//! The backend lives inside the SVC processor, which must be `Send`, so it only forwards what the server
//! sends to the session loop; the loop hands it to the JS callback registered with the `audio_playback`
//! extension, which plays it.

use std::borrow::Cow;

use futures_channel::mpsc;
use ironrdp::rdpsnd::client::RdpsndClientHandler;
use ironrdp::rdpsnd::pdu::{AudioFormat, PitchPdu, VolumePdu, WaveFormat};
use tracing::debug;
use wasm_bindgen::JsValue;

use crate::session::RdpInputEvent;

/// What the RDPSND backend forwards to the session loop.
#[derive(Debug)]
pub(crate) enum AudioMessage {
    /// A block of interleaved 16-bit little-endian PCM.
    Wave {
        sample_rate: u32,
        channels: u16,
        data: Vec<u8>,
    },
    /// Server volume, 0..=0xFFFF per channel.
    Volume { left: u16, right: u16 },
    /// The server stopped playback.
    Close,
}

#[derive(Debug)]
pub(crate) struct WasmAudioBackend {
    tx: mpsc::UnboundedSender<RdpInputEvent>,
    formats: Vec<AudioFormat>,
}

impl WasmAudioBackend {
    pub(crate) fn new(tx: mpsc::UnboundedSender<RdpInputEvent>) -> Self {
        // Uncompressed PCM only: Web Audio plays it directly, and it keeps the client simple.
        let pcm = |rate: u32, channels: u16| AudioFormat {
            format: WaveFormat::PCM,
            n_channels: channels,
            n_samples_per_sec: rate,
            n_avg_bytes_per_sec: rate * u32::from(channels) * 2,
            n_block_align: channels * 2,
            bits_per_sample: 16,
            data: None,
        };
        Self {
            tx,
            formats: vec![pcm(48_000, 2), pcm(44_100, 2), pcm(22_050, 2)],
        }
    }

    fn send(&self, message: AudioMessage) {
        if self.tx.unbounded_send(RdpInputEvent::Audio(message)).is_err() {
            debug!("Audio message dropped: the session loop is gone");
        }
    }
}

impl RdpsndClientHandler for WasmAudioBackend {
    fn get_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn wave(&mut self, format: &AudioFormat, _ts: u32, data: Cow<'_, [u8]>) {
        if format.format != WaveFormat::PCM || format.bits_per_sample != 16 {
            debug!(?format, "Unsupported audio format; block dropped");
            return;
        }
        self.send(AudioMessage::Wave {
            sample_rate: format.n_samples_per_sec,
            channels: format.n_channels,
            data: data.into_owned(),
        });
    }

    fn set_volume(&mut self, volume: VolumePdu) {
        self.send(AudioMessage::Volume {
            left: volume.volume_left,
            right: volume.volume_right,
        });
    }

    fn set_pitch(&mut self, _pitch: PitchPdu) {}

    fn close(&mut self) {
        self.send(AudioMessage::Close);
    }
}

/// The message as the object the JS callback receives:
/// `{ type: 'wave', sampleRate, channels, data: Uint8Array }`, `{ type: 'volume', left, right }` (0..1),
/// or `{ type: 'close' }`.
pub(crate) fn to_js(message: AudioMessage) -> JsValue {
    let object = js_sys::Object::new();
    let set = |key: &str, value: JsValue| {
        let _ = js_sys::Reflect::set(&object, &JsValue::from_str(key), &value);
    };
    match message {
        AudioMessage::Wave {
            sample_rate,
            channels,
            data,
        } => {
            set("type", JsValue::from_str("wave"));
            set("sampleRate", JsValue::from(sample_rate));
            set("channels", JsValue::from(channels));
            set("data", js_sys::Uint8Array::from(data.as_slice()).into());
        }
        AudioMessage::Volume { left, right } => {
            set("type", JsValue::from_str("volume"));
            set("left", JsValue::from(f64::from(left) / f64::from(u16::MAX)));
            set("right", JsValue::from(f64::from(right) / f64::from(u16::MAX)));
        }
        AudioMessage::Close => set("type", JsValue::from_str("close")),
    }
    object.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> (WasmAudioBackend, mpsc::UnboundedReceiver<RdpInputEvent>) {
        let (tx, rx) = mpsc::unbounded();
        (WasmAudioBackend::new(tx), rx)
    }

    fn next_audio(rx: &mut mpsc::UnboundedReceiver<RdpInputEvent>) -> Option<AudioMessage> {
        match rx.try_recv() {
            Ok(RdpInputEvent::Audio(message)) => Some(message),
            _ => None,
        }
    }

    #[test]
    fn advertises_16_bit_pcm_only() {
        let (backend, _rx) = backend();
        let formats = backend.get_formats();
        assert!(!formats.is_empty());
        for format in formats {
            assert_eq!(format.format, WaveFormat::PCM);
            assert_eq!(format.bits_per_sample, 16);
            assert_eq!(format.n_block_align, format.n_channels * 2);
            assert_eq!(
                format.n_avg_bytes_per_sec,
                format.n_samples_per_sec * u32::from(format.n_block_align)
            );
        }
    }

    #[test]
    fn forwards_pcm_blocks_with_their_format() {
        let (mut backend, mut rx) = backend();
        let format = backend.get_formats()[1].clone();
        backend.wave(&format, 0, Cow::Borrowed(&[1, 2, 3, 4]));
        match next_audio(&mut rx) {
            Some(AudioMessage::Wave {
                sample_rate,
                channels,
                data,
            }) => {
                assert_eq!(sample_rate, format.n_samples_per_sec);
                assert_eq!(channels, format.n_channels);
                assert_eq!(data, [1, 2, 3, 4]);
            }
            other => panic!("expected a wave block, got {other:?}"),
        }
    }

    #[test]
    fn drops_blocks_it_cannot_play() {
        let (mut backend, mut rx) = backend();
        let mut eight_bit = backend.get_formats()[0].clone();
        eight_bit.bits_per_sample = 8;
        backend.wave(&eight_bit, 0, Cow::Borrowed(&[0; 4]));
        let mut compressed = backend.get_formats()[0].clone();
        compressed.format = WaveFormat::ADPCM;
        backend.wave(&compressed, 0, Cow::Borrowed(&[0; 4]));
        assert!(next_audio(&mut rx).is_none());
    }

    #[test]
    fn forwards_volume_and_close() {
        let (mut backend, mut rx) = backend();
        backend.set_volume(VolumePdu {
            volume_left: 0x8000,
            volume_right: 0xFFFF,
        });
        backend.close();
        assert!(matches!(
            next_audio(&mut rx),
            Some(AudioMessage::Volume {
                left: 0x8000,
                right: 0xFFFF
            })
        ));
        assert!(matches!(next_audio(&mut rx), Some(AudioMessage::Close)));
    }
}
