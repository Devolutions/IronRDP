use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};
use ironrdp_rdpsnd_native::{is_pcm_capture_format, take_capture_packets};

// oracle: regression #2469
#[test]
fn consecutive_capture_uplink_failures_warn_once_until_recovery() {
    use core::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use ironrdp_core::encode_vec;
    use ironrdp_dvc::DvcProcessor as _;
    use ironrdp_rdpeai::client::{AudioPacketSink, RdpeaiCaptureHandler, RdpeaiClient};
    use ironrdp_rdpeai::pdu::{FormatsPdu, OpenPdu, OpenReplyPdu, RdpeaiPdu, Version, VersionPdu, pcm_format};
    use tracing_subscriber::layer::SubscriberExt as _;

    struct Capture {
        formats: Vec<AudioFormat>,
        sink: Arc<Mutex<Option<AudioPacketSink>>>,
    }

    impl RdpeaiCaptureHandler for Capture {
        fn supported_formats(&self) -> &[AudioFormat] {
            &self.formats
        }

        fn open(&mut self, _: &AudioFormat, _: &AudioFormat, _: usize, sink: AudioPacketSink) -> i32 {
            *self.sink.lock().unwrap() = Some(sink);
            OpenReplyPdu::S_OK
        }

        fn set_format(&mut self, _: &AudioFormat, _: usize) -> bool {
            true
        }

        fn close(&mut self) {
            *self.sink.lock().unwrap() = None;
        }
    }

    #[derive(Clone, Default)]
    struct Warnings(Arc<Mutex<Vec<String>>>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Warnings {
        fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
            struct Message(String);
            impl tracing::field::Visit for Message {
                fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn core::fmt::Debug) {
                    if field.name() == "message" {
                        self.0 = format!("{value:?}");
                    }
                }
            }
            if event.metadata().target() == "ironrdp_rdpeai::client"
                && *event.metadata().level() == tracing::Level::WARN
            {
                let mut message = Message(String::new());
                event.record(&mut message);
                self.0.lock().unwrap().push(message.0);
            }
        }
    }

    let warnings = Warnings::default();
    let _guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(warnings.clone()));
    let format = pcm_format(1, 16_000, 16);
    let sink = Arc::new(Mutex::new(None));
    let failing = Arc::new(AtomicBool::new(true));
    let uplink_failing = Arc::clone(&failing);
    let mut client = RdpeaiClient::new(
        Box::new(Capture {
            formats: vec![format.clone()],
            sink: Arc::clone(&sink),
        }),
        Box::new(move |_, _| {
            if uplink_failing.load(Ordering::Acquire) {
                Err(ironrdp_pdu::pdu_other_err!("capture queue full"))
            } else {
                Ok(())
            }
        }),
    );
    client.start(3).unwrap();
    for pdu in [
        RdpeaiPdu::Version(VersionPdu::new(Version::V1)),
        RdpeaiPdu::Formats(FormatsPdu::server(vec![format.clone()])),
    ] {
        client.process(3, &encode_vec(&pdu).unwrap()).unwrap();
    }

    for expected in [2, 4] {
        failing.store(true, Ordering::Release);
        let open = RdpeaiPdu::Open(OpenPdu {
            frames_per_packet: 2,
            initial_format: 0,
            capture_format: format.clone(),
        });
        client.process(3, &encode_vec(&open).unwrap()).unwrap();
        for _ in 0..3 {
            sink.lock().unwrap().as_mut().unwrap()(vec![0; 4]);
        }
        failing.store(false, Ordering::Release);
        sink.lock().unwrap().as_mut().unwrap()(vec![0; 4]);
        failing.store(true, Ordering::Release);
        sink.lock().unwrap().as_mut().unwrap()(vec![0; 4]);
        assert_eq!(
            warnings
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|message| message.as_str() == "Failed to uplink AUDIO_INPUT data")
                .count(),
            expected
        );
    }
}

#[test]
fn take_capture_packets_splits_exact_frames() {
    let mut buffer = vec![0u8; 10];
    for (i, b) in buffer.iter_mut().enumerate() {
        *b = u8::try_from(i).expect("small index");
    }
    let packets = take_capture_packets(&mut buffer, 4);
    assert_eq!(packets.len(), 2);
    assert_eq!(packets[0], vec![0, 1, 2, 3]);
    assert_eq!(packets[1], vec![4, 5, 6, 7]);
    assert_eq!(buffer, vec![8, 9]);
}

#[test]
fn take_capture_packets_zero_size_is_noop() {
    let mut buffer = vec![1, 2, 3];
    let packets = take_capture_packets(&mut buffer, 0);
    assert!(packets.is_empty());
    assert_eq!(buffer, vec![1, 2, 3]);
}

#[test]
fn plain_pcm_is_capture_format() {
    let fmt = AudioFormat {
        format: WaveFormat::PCM,
        n_channels: 1,
        n_samples_per_sec: 16_000,
        n_avg_bytes_per_sec: 32_000,
        n_block_align: 2,
        bits_per_sample: 16,
        data: None,
    };
    assert!(is_pcm_capture_format(&fmt));
}

#[test]
fn extensible_pcm_subtype_is_capture_format() {
    // wValidBitsPerSample=16, dwChannelMask=SPEAKER_FRONT_CENTER, SubFormat=PCM
    let mut data = vec![16, 0, 0x04, 0x00, 0x00, 0x00];
    data.extend_from_slice(&[
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
    ]);
    let fmt = AudioFormat {
        format: WaveFormat::EXTENSIBLE,
        n_channels: 1,
        n_samples_per_sec: 16_000,
        n_avg_bytes_per_sec: 32_000,
        n_block_align: 2,
        bits_per_sample: 16,
        data: Some(data),
    };
    assert!(is_pcm_capture_format(&fmt));
}

#[test]
fn extensible_non_pcm_subtype_is_rejected() {
    let mut data = vec![16, 0, 0x04, 0x00, 0x00, 0x00];
    // IEEE_FLOAT subtype
    data.extend_from_slice(&[
        0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
    ]);
    let fmt = AudioFormat {
        format: WaveFormat::EXTENSIBLE,
        n_channels: 1,
        n_samples_per_sec: 16_000,
        n_avg_bytes_per_sec: 32_000,
        n_block_align: 2,
        bits_per_sample: 16,
        data: Some(data),
    };
    assert!(!is_pcm_capture_format(&fmt));
}

fn gsm() -> AudioFormat {
    AudioFormat {
        format: WaveFormat::GSM610,
        n_channels: 1,
        n_samples_per_sec: 8_000,
        n_avg_bytes_per_sec: 1_625,
        n_block_align: 65,
        bits_per_sample: 0,
        data: Some(vec![0x40, 0x01]),
    }
}

// oracle: mutant unencodable formats offered, or PCM offered before compressed formats
#[test]
fn capture_offers_encodable_formats_compressed_first() {
    use ironrdp_rdpeai::client::RdpeaiCaptureHandler as _;
    use ironrdp_rdpeai::pdu::pcm_format;
    use ironrdp_rdpsnd::codec;
    use ironrdp_rdpsnd_native::RdpeaiCaptureBackend;

    let server = vec![
        pcm_format(1, 16_000, 16),
        gsm(),
        codec::alaw_format(1, 8_000),
        codec::ms_adpcm_format(1, 22_050, 512),
        pcm_format(2, 44_100, 8),
    ];

    let chosen = RdpeaiCaptureBackend::new().choose_formats(&server);

    // GSM 6.10 has no encoder; every other format is offered, compressed ones before PCM.
    assert_eq!(chosen.len(), 4);
    assert!(!chosen.contains(&gsm()));
    let first_pcm = chosen.iter().position(is_pcm_capture_format).unwrap();
    assert!(chosen[..first_pcm].iter().all(|format| !is_pcm_capture_format(format)));
    assert!(chosen[first_pcm..].iter().all(is_pcm_capture_format));
    assert_eq!(first_pcm, 2);
}

// oracle: mutant capture opened with an encoder for another rate
#[test]
fn capture_refuses_an_encode_format_with_other_rate_or_channels() {
    use ironrdp_rdpeai::client::RdpeaiCaptureHandler as _;
    use ironrdp_rdpeai::pdu::{OpenReplyPdu, pcm_format};
    use ironrdp_rdpsnd::codec;
    use ironrdp_rdpsnd_native::RdpeaiCaptureBackend;

    let mut backend = RdpeaiCaptureBackend::new();

    // Refused before any device is opened.
    let result = backend.open(
        &pcm_format(1, 16_000, 16),
        &codec::alaw_format(1, 8_000),
        640,
        Box::new(|_| {}),
    );

    assert_eq!(result, OpenReplyPdu::E_FAIL);
}

// oracle: mutant undecodable formats advertised or the server's order lost
// WASAPI opens every layout, so the device does not filter the formats on Windows.
#[cfg(windows)]
#[test]
fn playback_advertises_decodable_formats_in_server_order() {
    use ironrdp_rdpsnd::client::RdpsndClientHandler as _;
    use ironrdp_rdpsnd::codec;
    use ironrdp_rdpsnd_native::cpal::RdpsndBackend;

    let pcm = AudioFormat {
        format: WaveFormat::PCM,
        n_channels: 2,
        n_samples_per_sec: 44_100,
        n_avg_bytes_per_sec: 176_400,
        n_block_align: 4,
        bits_per_sample: 16,
        data: None,
    };
    let server = vec![
        codec::ms_adpcm_format(2, 44_100, 2048),
        gsm(),
        pcm.clone(),
        codec::ima_adpcm_format(1, 22_050, 512),
        codec::mulaw_format(2, 22_050),
    ];

    let chosen = RdpsndBackend::new().choose_formats(&server);

    assert_eq!(
        chosen,
        vec![
            codec::ms_adpcm_format(2, 44_100, 2048),
            pcm,
            codec::ima_adpcm_format(1, 22_050, 512),
            codec::mulaw_format(2, 22_050),
        ]
    );
}

// oracle: mutant server-chosen layouts sizing unbounded buffers
#[test]
fn playback_refuses_extreme_channel_counts_and_rates() {
    use ironrdp_rdpsnd::client::RdpsndClientHandler as _;
    use ironrdp_rdpsnd_native::cpal::RdpsndBackend;

    let pcm = |channels: u16, rate: u32| AudioFormat {
        format: WaveFormat::PCM,
        n_channels: channels,
        n_samples_per_sec: rate,
        n_avg_bytes_per_sec: rate.saturating_mul(u32::from(channels) * 2),
        n_block_align: channels.saturating_mul(2),
        bits_per_sample: 16,
        data: None,
    };
    let server = vec![pcm(9, 48_000), pcm(2, 3_999), pcm(2, 192_001), pcm(u16::MAX, u32::MAX)];

    // Each of these would size the playback buffer from a server-chosen value.
    assert!(RdpsndBackend::new().choose_formats(&server).is_empty());
}

// oracle: mutant the Media Foundation AAC path panics, hangs or wedges the backend on hostile frames
#[cfg(windows)]
#[test]
fn malformed_aac_frames_leave_the_backend_usable() {
    use std::borrow::Cow;

    use ironrdp_rdpsnd::client::RdpsndClientHandler as _;
    use ironrdp_rdpsnd_native::cpal::RdpsndBackend;

    // The format Windows Server 2025 offers for AAC: no HEAACWAVEINFO bytes.
    let aac = AudioFormat {
        format: WaveFormat::AAC_MS,
        n_channels: 2,
        n_samples_per_sec: 44_100,
        n_avg_bytes_per_sec: 12_000,
        n_block_align: 4,
        bits_per_sample: 16,
        data: None,
    };
    let mut state: u32 = 0x1234_5678;
    let mut noise = |len: usize| -> Vec<u8> {
        core::iter::repeat_with(|| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state.to_le_bytes()[0]
        })
        .take(len)
        .collect()
    };
    let mut frames = vec![
        Vec::new(),
        vec![0x21],
        vec![0xFF, 0xF1, 0x50, 0x80, 0xFF, 0xFF, 0xFC], // ADTS header with an impossible length
        vec![0xFF; 4096],
    ];
    frames.extend((0..32).map(|index| noise(1 + index * 37)));

    let mut backend = RdpsndBackend::new();
    for frame in &frames {
        backend.wave(&aac, 0, Cow::Borrowed(frame));
    }
    // The backend still answers after the hostile frames, and closes cleanly.
    let _ = backend.playback_latency();
    backend.close();
}
