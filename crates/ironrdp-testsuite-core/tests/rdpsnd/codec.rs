//! Wave codec tests against reference vectors.
//!
//! The `test_data/audio` vectors were produced with FFmpeg from the `mono.s16` and `stereo.s16`
//! signals: each `<name>.wav` holds FFmpeg's encoding, and `<name>.decoded.s16` holds FFmpeg's
//! decoding of it. FFmpeg's ADPCM and G.711 decoders are bit-exact with the Windows ACM codecs.
//! Its G.711 encoder rounds to the nearest code where the ITU reference truncates, so encoders are
//! checked by round trip instead.

use ironrdp_core::decode;
use ironrdp_rdpsnd::codec::{
    self, AudioDecoder, AudioEncoder, CodecError, alaw_to_linear, linear_to_alaw, linear_to_ulaw, ulaw_to_linear,
};
use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};
use rstest::rstest;

const MONO: &[u8] = include_bytes!("../../test_data/audio/mono.s16");
const STEREO: &[u8] = include_bytes!("../../test_data/audio/stereo.s16");

fn samples(bytes: &[u8]) -> Vec<i16> {
    bytes
        .chunks_exact(2)
        .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
        .collect()
}

/// Returns the `WAVEFORMATEX` and the sample data of a RIFF WAVE file.
fn parse_wav(file: &[u8]) -> (AudioFormat, &[u8]) {
    assert_eq!(&file[..4], b"RIFF");
    assert_eq!(&file[8..12], b"WAVE");
    let mut format = None;
    let mut data = None;
    let mut rest = &file[12..];
    while rest.len() >= 8 {
        let size = usize::try_from(u32::from_le_bytes(rest[4..8].try_into().unwrap())).unwrap();
        let body = &rest[8..8 + size];
        match &rest[..4] {
            b"fmt " => format = Some(decode::<AudioFormat>(body).unwrap()),
            b"data" => data = Some(body),
            _ => {}
        }
        rest = &rest[(8 + size + size % 2).min(rest.len())..];
    }
    (format.unwrap(), data.unwrap())
}

fn signal_to_noise_db(reference: &[i16], decoded: &[i16]) -> f64 {
    let (signal, noise) =
        reference
            .iter()
            .zip(decoded)
            .fold((0.0f64, 0.0f64), |(signal, noise), (reference, decoded)| {
                let error = f64::from(*reference) - f64::from(*decoded);
                (signal + f64::from(*reference).powi(2), noise + error * error)
            });
    10.0 * (signal / noise.max(1.0)).log10()
}

#[rstest]
#[case::ms_adpcm_mono(include_bytes!("../../test_data/audio/ms_adpcm_mono.wav"), include_bytes!("../../test_data/audio/ms_adpcm_mono.decoded.s16"))]
#[case::ms_adpcm_stereo(include_bytes!("../../test_data/audio/ms_adpcm_stereo.wav"), include_bytes!("../../test_data/audio/ms_adpcm_stereo.decoded.s16"))]
#[case::ima_adpcm_mono(include_bytes!("../../test_data/audio/ima_adpcm_mono.wav"), include_bytes!("../../test_data/audio/ima_adpcm_mono.decoded.s16"))]
#[case::ima_adpcm_stereo(include_bytes!("../../test_data/audio/ima_adpcm_stereo.wav"), include_bytes!("../../test_data/audio/ima_adpcm_stereo.decoded.s16"))]
#[case::alaw(include_bytes!("../../test_data/audio/alaw_mono.wav"), include_bytes!("../../test_data/audio/alaw_mono.decoded.s16"))]
#[case::mulaw(include_bytes!("../../test_data/audio/mulaw_mono.wav"), include_bytes!("../../test_data/audio/mulaw_mono.decoded.s16"))]
fn decoder_matches_reference(#[case] wav: &[u8], #[case] reference: &[u8]) {
    let (format, data) = parse_wav(wav);
    let mut decoder = AudioDecoder::new(&format).unwrap();
    assert_eq!(decoder.channels(), format.n_channels);
    assert_eq!(decoder.sample_rate(), format.n_samples_per_sec);

    let mut decoded = Vec::new();
    decoder.decode(data, &mut decoded).unwrap();

    assert_eq!(decoded, samples(reference));
}

#[test]
fn decoder_keeps_state_free_across_wave_blocks() {
    // Each ADPCM block restarts from its header, so decoding blocks one call at a time matches
    // decoding the whole payload at once.
    let (format, data) = parse_wav(include_bytes!("../../test_data/audio/ms_adpcm_stereo.wav"));
    let mut whole = Vec::new();
    AudioDecoder::new(&format).unwrap().decode(data, &mut whole).unwrap();

    let mut decoder = AudioDecoder::new(&format).unwrap();
    let mut split = Vec::new();
    for block in data.chunks(usize::from(format.n_block_align)) {
        decoder.decode(block, &mut split).unwrap();
    }

    assert_eq!(split, whole);
}

#[rstest]
#[case::alaw_positive_minimum(alaw_to_linear(0xD5), 8)]
#[case::alaw_negative_minimum(alaw_to_linear(0x55), -8)]
#[case::alaw_positive_maximum(alaw_to_linear(0xAA), 32256)]
#[case::alaw_negative_maximum(alaw_to_linear(0x2A), -32256)]
#[case::ulaw_positive_zero(ulaw_to_linear(0xFF), 0)]
#[case::ulaw_negative_zero(ulaw_to_linear(0x7F), 0)]
#[case::ulaw_positive_maximum(ulaw_to_linear(0x80), 32124)]
#[case::ulaw_negative_maximum(ulaw_to_linear(0x00), -32124)]
fn g711_expansion_known_values(#[case] decoded: i16, #[case] expected: i16) {
    assert_eq!(decoded, expected);
}

#[test]
fn g711_round_trip_is_idempotent() {
    for code in 0..=u8::MAX {
        assert_eq!(linear_to_alaw(alaw_to_linear(code)), code, "A-law {code:#04x}");
        // µ-law has two zero codes; 0x7F (negative zero) compresses back to 0xFF.
        if code != 0x7F {
            assert_eq!(linear_to_ulaw(ulaw_to_linear(code)), code, "µ-law {code:#04x}");
        }
    }
}

// The signals carry broadband noise and a tone near the Nyquist frequency, which first-order IMA
// prediction tracks poorly: FFmpeg's own IMA encoder scores 11.6 dB (mono) and 12.7 dB (stereo).
#[rstest]
#[case::ima_mono(codec::ima_adpcm_format(1, 8000, 256), MONO, 11.0)]
#[case::ima_stereo(codec::ima_adpcm_format(2, 22050, 512), STEREO, 12.0)]
#[case::ms_mono(codec::ms_adpcm_format(1, 8000, 256), MONO, 20.0)]
#[case::ms_stereo(codec::ms_adpcm_format(2, 22050, 512), STEREO, 20.0)]
#[case::alaw(codec::alaw_format(1, 8000), MONO, 30.0)]
#[case::mulaw(codec::mulaw_format(1, 8000), MONO, 30.0)]
fn encoder_round_trip(#[case] format: AudioFormat, #[case] signal: &[u8], #[case] minimum_snr_db: f64) {
    let pcm = samples(signal);
    let mut encoder = AudioEncoder::new(&format).unwrap();
    let block_frames = encoder.frames_per_block();
    let channels = usize::from(format.n_channels);
    // Pad to whole blocks so the encoder flushes everything it was given.
    let mut padded = pcm.clone();
    padded.resize(pcm.len().div_ceil(block_frames * channels) * block_frames * channels, 0);

    let mut encoded = Vec::new();
    encoder.encode(&padded, &mut encoded);
    let mut decoded = Vec::new();
    AudioDecoder::new(&format)
        .unwrap()
        .decode(&encoded, &mut decoded)
        .unwrap();

    assert_eq!(encoded.len() % usize::from(format.n_block_align), 0);
    assert_eq!(decoded.len(), padded.len());
    let snr = signal_to_noise_db(&pcm, &decoded[..pcm.len()]);
    assert!(minimum_snr_db < snr, "SNR {snr:.1} dB is below {minimum_snr_db} dB");
}

#[rstest]
#[case::ima(codec::ima_adpcm_format(2, 22050, 128))]
#[case::ms(codec::ms_adpcm_format(2, 22050, 128))]
fn encoder_buffers_partial_blocks(#[case] format: AudioFormat) {
    let pcm = samples(STEREO);
    let mut whole = Vec::new();
    AudioEncoder::new(&format).unwrap().encode(&pcm, &mut whole);

    let mut encoder = AudioEncoder::new(&format).unwrap();
    let mut split = Vec::new();
    for chunk in pcm.chunks(14) {
        encoder.encode(chunk, &mut split);
    }

    assert!(!whole.is_empty());
    assert_eq!(split, whole);
}

#[test]
fn ms_adpcm_blocks_with_spare_room_are_padded_to_block_align() {
    // Four samples per block in a 16-byte block: the header and one code byte, then padding.
    let mut format = codec::ms_adpcm_format(1, 8000, 16);
    format.data.as_mut().unwrap()[..2].copy_from_slice(&4u16.to_le_bytes());
    let pcm = [100, -200, 300, -400, 500, -600, 700, -800];

    let mut encoded = Vec::new();
    AudioEncoder::new(&format).unwrap().encode(&pcm, &mut encoded);
    let mut decoded = Vec::new();
    AudioDecoder::new(&format)
        .unwrap()
        .decode(&encoded, &mut decoded)
        .unwrap();

    assert_eq!(encoded.len(), 32);
    assert_eq!(decoded.len(), pcm.len());
    // The two header samples of each block are exact.
    assert_eq!([decoded[0], decoded[1], decoded[4], decoded[5]], [100, -200, 500, -600]);
}

#[test]
fn ms_adpcm_rejects_unknown_predictor() {
    let format = codec::ms_adpcm_format(1, 8000, 32);
    let mut block = vec![0u8; 32];
    block[0] = 7; // The standard table has predictors 0 to 6.

    let result = AudioDecoder::new(&format).unwrap().decode(&block, &mut Vec::new());

    assert!(matches!(result, Err(CodecError::InvalidData(_))));
}

#[test]
fn ima_adpcm_rejects_block_shorter_than_header() {
    let format = codec::ima_adpcm_format(2, 8000, 64);

    let result = AudioDecoder::new(&format).unwrap().decode(&[0u8; 6], &mut Vec::new());

    assert!(matches!(result, Err(CodecError::InvalidData(_))));
}

#[test]
fn truncated_final_block_decodes_complete_frames() {
    let (format, data) = parse_wav(include_bytes!("../../test_data/audio/ms_adpcm_stereo.wav"));
    let block = usize::from(format.n_block_align);
    let mut full = Vec::new();
    AudioDecoder::new(&format)
        .unwrap()
        .decode(&data[..block], &mut full)
        .unwrap();

    // Header (14 bytes) and five code bytes: two header frames plus five coded frames.
    let mut truncated = Vec::new();
    AudioDecoder::new(&format)
        .unwrap()
        .decode(&data[..14 + 5], &mut truncated)
        .unwrap();

    assert_eq!(truncated, full[..2 * (2 + 5)]);
}

#[rstest]
#[case::unknown_codec(AudioFormat { format: WaveFormat::GSM610, ..codec::alaw_format(1, 8000) })]
#[case::pcm_24_bit(AudioFormat { format: WaveFormat::PCM, bits_per_sample: 24, ..codec::alaw_format(1, 8000) })]
#[case::zero_channels(AudioFormat { n_channels: 0, ..codec::alaw_format(1, 8000) })]
#[case::ima_three_channels(AudioFormat { n_channels: 3, ..codec::ima_adpcm_format(1, 8000, 256) })]
#[case::ima_ragged_block(AudioFormat { n_block_align: 255, ..codec::ima_adpcm_format(1, 8000, 256) })]
#[case::ms_block_smaller_than_header(AudioFormat { n_block_align: 7, ..codec::ms_adpcm_format(1, 8000, 256) })]
#[case::ms_truncated_coefficients(AudioFormat { data: Some(vec![0xF4, 0x01, 0x07, 0x00, 0x00, 0x01]), ..codec::ms_adpcm_format(1, 8000, 256) })]
fn unsupported_formats_are_rejected(#[case] format: AudioFormat) {
    assert!(!AudioDecoder::supports(&format));
    assert!(!AudioEncoder::supports(&format));
}
