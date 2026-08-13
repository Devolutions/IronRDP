use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};
use ironrdp_rdpsnd_native::{is_pcm_capture_format, take_capture_packets};

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
