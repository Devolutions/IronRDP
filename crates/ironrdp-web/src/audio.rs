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
        // Rdpsnd supplies a negotiated entry from get_formats(), which contains only 16-bit PCM.
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

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn advertises_stereo_16_bit_pcm_at_supported_rates() {
        let (backend, _rx) = backend();
        // These are the extension's advertised formats, in preference order. PCM byte rates
        // and frame alignment follow WAVEFORMATEX:
        // https://learn.microsoft.com/en-us/windows/win32/api/mmreg/ns-mmreg-waveformatex
        let formats: Vec<_> = backend
            .get_formats()
            .iter()
            .map(|format| {
                (
                    format.format,
                    format.n_channels,
                    format.n_samples_per_sec,
                    format.n_avg_bytes_per_sec,
                    format.n_block_align,
                    format.bits_per_sample,
                    format.data.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            formats,
            [
                (WaveFormat::PCM, 2, 48_000, 192_000, 4, 16, None),
                (WaveFormat::PCM, 2, 44_100, 176_400, 4, 16, None),
                (WaveFormat::PCM, 2, 22_050, 88_200, 4, 16, None),
            ]
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
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

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn forwards_volume() {
        let (mut backend, mut rx) = backend();
        backend.set_volume(VolumePdu {
            volume_left: 0x8000,
            volume_right: 0xFFFF,
        });
        match next_audio(&mut rx) {
            Some(AudioMessage::Volume { left, right }) => {
                assert_eq!(left, 0x8000);
                assert_eq!(right, 0xFFFF);
            }
            other => panic!("expected volume, got {other:?}"),
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn forwards_close() {
        let (mut backend, mut rx) = backend();
        backend.close();
        match next_audio(&mut rx) {
            Some(AudioMessage::Close) => {}
            other => panic!("expected close, got {other:?}"),
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn rdpsnd_dispatches_only_negotiated_pcm_formats() {
        use ironrdp::rdpsnd::client::Rdpsnd;
        use ironrdp::rdpsnd::pdu::{self, ServerAudioOutputPdu};
        use ironrdp_core::{decode, encode_vec};
        use ironrdp_svc::SvcProcessor as _;

        let (backend, mut rx) = backend();
        let formats = backend.get_formats().to_vec();
        let mut compressed = formats[0].clone();
        compressed.format = WaveFormat::ADPCM;
        let mut eight_bit = formats[0].clone();
        eight_bit.bits_per_sample = 8;
        eight_bit.n_block_align = 2;
        eight_bit.n_avg_bytes_per_sec = 96_000;
        let mut rdpsnd = Rdpsnd::new(Box::new(backend));
        let response = rdpsnd
            .process(
                &encode_vec(&ServerAudioOutputPdu::AudioFormat(pdu::ServerAudioFormatPdu {
                    version: pdu::Version::V8,
                    formats: vec![compressed, eight_bit, formats[1].clone(), formats[0].clone()],
                }))
                .expect("encode server formats"),
            )
            .expect("negotiate audio formats");
        let encoded = response[0].encode_unframed_pdu().expect("encode client formats");
        let pdu::ClientAudioOutputPdu::AudioFormat(client_formats) = decode(&encoded).expect("decode client formats")
        else {
            panic!("expected client audio formats");
        };
        assert_eq!(client_formats.formats, formats[..2]);
        match next_audio(&mut rx) {
            Some(AudioMessage::Close) => {}
            other => panic!("expected close at negotiation, got {other:?}"),
        }
        rdpsnd
            .process(
                &encode_vec(&ServerAudioOutputPdu::Training(pdu::TrainingPdu {
                    timestamp: 0,
                    data: Vec::new(),
                }))
                .expect("encode training"),
            )
            .expect("complete audio training");

        // Wave indices refer to the client reply, not the server's compressed/8-bit entries.
        // Index 2 is outside the two negotiated formats and must not reach the backend.
        for (format_no, expected_rate) in [(0, Some(48_000)), (1, Some(44_100)), (2, None)] {
            rdpsnd
                .process(
                    &encode_vec(&ServerAudioOutputPdu::Wave2(pdu::Wave2Pdu {
                        timestamp: 0,
                        format_no,
                        block_no: 0,
                        audio_timestamp: 0,
                        data: Cow::Borrowed(&[0, 128, 255, 127]),
                    }))
                    .expect("encode wave"),
                )
                .expect("process wave");
            match (expected_rate, next_audio(&mut rx)) {
                (
                    Some(expected_rate),
                    Some(AudioMessage::Wave {
                        sample_rate,
                        channels,
                        data,
                    }),
                ) => {
                    assert_eq!(sample_rate, expected_rate);
                    assert_eq!(channels, 2);
                    assert_eq!(data, [0, 128, 255, 127]);
                }
                (None, None) => {}
                other => panic!("unexpected dispatch for format index {format_no}: {other:?}"),
            }
        }
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    fn field(message: &JsValue, name: &str) -> JsValue {
        js_sys::Reflect::get(message, &JsValue::from_str(name)).expect("read audio message field")
    }

    fn keys(message: &JsValue) -> Vec<String> {
        let mut keys: Vec<_> = js_sys::Object::keys(message.unchecked_ref::<js_sys::Object>())
            .iter()
            .map(|key| key.as_string().expect("string key"))
            .collect();
        keys.sort();
        keys
    }

    #[wasm_bindgen_test]
    fn wave_callback_contract() {
        let message = to_js(AudioMessage::Wave {
            sample_rate: 44_100,
            channels: 2,
            data: vec![0, 128, 255, 127],
        });
        assert_eq!(keys(&message), ["channels", "data", "sampleRate", "type"]);
        assert_eq!(field(&message, "type"), "wave");
        assert_eq!(field(&message, "sampleRate"), 44_100);
        assert_eq!(field(&message, "channels"), 2);
        let data = field(&message, "data")
            .dyn_into::<js_sys::Uint8Array>()
            .expect("wave data is a Uint8Array");
        assert_eq!(data.to_vec(), [0, 128, 255, 127]);
    }

    #[wasm_bindgen_test]
    fn volume_callback_contract() {
        for (left, right, expected_left, expected_right) in [
            (0, 0xFFFF, 0.0, 1.0),
            (0xFFFF, 0, 1.0, 0.0),
            (0x8000, 0x4000, 0.5000076295109483, 0.2500038147554742),
        ] {
            let message = to_js(AudioMessage::Volume { left, right });
            assert_eq!(keys(&message), ["left", "right", "type"]);
            assert_eq!(field(&message, "type"), "volume");
            let actual_left = field(&message, "left").as_f64().expect("numeric left volume");
            let actual_right = field(&message, "right").as_f64().expect("numeric right volume");
            assert!(
                (actual_left - expected_left).abs() < f64::EPSILON,
                "left: {actual_left}"
            );
            assert!(
                (actual_right - expected_right).abs() < f64::EPSILON,
                "right: {actual_right}"
            );
        }
    }

    #[wasm_bindgen_test]
    fn close_callback_contract() {
        let message = to_js(AudioMessage::Close);
        assert_eq!(keys(&message), ["type"]);
        assert_eq!(field(&message, "type"), "close");
    }
}
