use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use ironrdp_core::{decode, encode_vec};
use ironrdp_rdpsnd::client::{NoopRdpsndBackend, Rdpsnd, RdpsndClientHandler};
use ironrdp_rdpsnd::pdu::{self, AudioFormat, PitchPdu, VolumePdu, WaveFormat};
use ironrdp_svc::SvcProcessor as _;
use rstest::rstest;

// ============================================================================
// Encoding helpers
// ============================================================================

fn pcm(rate: u32, channels: u16) -> AudioFormat {
    let block = channels * 2;
    AudioFormat {
        format: WaveFormat::PCM,
        n_channels: channels,
        n_samples_per_sec: rate,
        n_avg_bytes_per_sec: rate * u32::from(block),
        n_block_align: block,
        bits_per_sample: 16,
        data: None,
    }
}

fn encoded_server_formats(version: pdu::Version) -> Vec<u8> {
    encoded_server_formats_with(version, vec![pcm(44100, 2)])
}

fn encoded_server_formats_with(version: pdu::Version, formats: Vec<AudioFormat>) -> Vec<u8> {
    encode_vec(&pdu::ServerAudioOutputPdu::AudioFormat(pdu::ServerAudioFormatPdu {
        version,
        formats,
    }))
    .unwrap()
}

fn encoded_training() -> Vec<u8> {
    encode_vec(&pdu::ServerAudioOutputPdu::Training(pdu::TrainingPdu {
        timestamp: 0x1234,
        data: vec![],
    }))
    .unwrap()
}

fn encoded_wave2(block_no: u8) -> Vec<u8> {
    encoded_wave2_with(block_no, 0)
}

fn encoded_wave2_with(block_no: u8, format_no: u16) -> Vec<u8> {
    encode_vec(&pdu::ServerAudioOutputPdu::Wave2(pdu::Wave2Pdu {
        timestamp: 0xA116,
        format_no,
        block_no,
        audio_timestamp: 0xDACB8C2,
        data: Cow::Borrowed(&[0x01, 0x02, 0x03, 0x04]),
    }))
    .unwrap()
}

fn encoded_volume() -> Vec<u8> {
    encode_vec(&pdu::ServerAudioOutputPdu::Volume(VolumePdu {
        volume_left: 0x8000,
        volume_right: 0x8000,
    }))
    .unwrap()
}

fn encoded_pitch() -> Vec<u8> {
    encode_vec(&pdu::ServerAudioOutputPdu::Pitch(PitchPdu { pitch: 0x00010000 })).unwrap()
}

fn encoded_close() -> Vec<u8> {
    encode_vec(&pdu::ServerAudioOutputPdu::Close).unwrap()
}

fn encoded_crypt_key() -> Vec<u8> {
    encode_vec(&pdu::ServerAudioOutputPdu::CryptKey(pdu::CryptKeyPdu {
        seed: [0xAB; 32],
    }))
    .unwrap()
}

fn encoded_wave_encrypt() -> Vec<u8> {
    encode_vec(&pdu::ServerAudioOutputPdu::WaveEncrypt(pdu::WaveEncryptPdu {
        timestamp: 0x1234,
        format_no: 0,
        block_no: 1,
        signature: Some([0xCC; 8]),
        data: vec![0x01, 0x02],
    }))
    .unwrap()
}

fn encoded_wave_info(audio_length: u16) -> Vec<u8> {
    encode_vec(&pdu::ServerAudioOutputPdu::Wave(pdu::WavePdu {
        timestamp: 0xADD7,
        format_no: 0,
        block_no: 1,
        data_prefix: [0x01, 0x02, 0x03, 0x04],
        audio_length,
    }))
    .unwrap()
}

/// Bare Wave payload after WaveInfo: bPad[4] + remaining audio after the prefix.
fn encoded_wave_data(remaining: &[u8]) -> Vec<u8> {
    encode_vec(&pdu::WaveDataPdu {
        data: remaining.to_vec(),
    })
    .unwrap()
}

fn encoded_wave() -> Vec<u8> {
    // WaveInfo for a 4-byte sample (prefix only; remaining audio empty).
    encoded_wave_info(4)
}

// ============================================================================
// State constructors
// ============================================================================

// Drive the client state machine from Start through to Ready.
fn client_in_ready(version: pdu::Version) -> Rdpsnd {
    let mut client = Rdpsnd::new(Box::new(NoopRdpsndBackend));
    client.process(&encoded_server_formats(version)).unwrap();
    client.process(&encoded_training()).unwrap();
    client
}

fn client_in_start() -> Rdpsnd {
    Rdpsnd::new(Box::new(NoopRdpsndBackend))
}

fn client_in_waiting() -> Rdpsnd {
    let mut client = Rdpsnd::new(Box::new(NoopRdpsndBackend));
    client.process(&encoded_server_formats(pdu::Version::V8)).unwrap();
    client
}

fn client_in_stop() -> Rdpsnd {
    let mut client = Rdpsnd::new(Box::new(NoopRdpsndBackend));
    // Training is invalid in Start state, transitions to Stop.
    client.process(&encoded_training()).unwrap();
    client
}

// ============================================================================
// Verification helpers
// ============================================================================

// Verify the client is in the Stop state by confirming that a valid PDU
// is silently ignored (empty response, no error).
fn assert_in_stop_state(client: &mut Rdpsnd) {
    let responses = client.process(&encoded_server_formats(pdu::Version::V8)).unwrap();
    assert!(responses.is_empty(), "Stop state should produce no responses");
}

#[test]
fn malformed_encrypted_wave_is_ignored() {
    let mut client = client_in_ready(pdu::Version::V8);

    // SNDWAVECRYPT carries its eight-byte fixed part but omits the mandatory v5 signature.
    let malformed_wave_encrypt = [
        0x09, 0x00, 0x08, 0x00, // SNDWAVECRYPT, body size 8
        0x00, 0x00, // wTimeStamp
        0x00, 0x00, // wFormatNo
        0x00, // cBlockNo
        0x00, 0x00, 0x00, // bPad
    ];

    assert!(
        client
            .process(&malformed_wave_encrypt)
            .expect("a malformed RDPSND PDU must be ignored")
            .is_empty()
    );

    let confirm = decode_single_response(&client.process(&encoded_wave2(1)).unwrap());
    assert!(matches!(confirm, pdu::ClientAudioOutputPdu::WaveConfirm(_)));
}

fn decode_single_response(responses: &[ironrdp_svc::SvcMessage]) -> pdu::ClientAudioOutputPdu {
    assert_eq!(responses.len(), 1);
    let encoded = responses[0].encode_unframed_pdu().unwrap();
    decode(&encoded).unwrap()
}

// ============================================================================
// Error-path tests: invalid PDU in a given state transitions to Stop
// ============================================================================

#[rstest]
#[case::start_training(client_in_start(), encoded_training())]
#[case::start_close(client_in_start(), encoded_close())]
#[case::start_volume(client_in_start(), encoded_volume())]
#[case::start_pitch(client_in_start(), encoded_pitch())]
#[case::start_wave(client_in_start(), encoded_wave())]
#[case::start_wave2(client_in_start(), encoded_wave2(0))]
#[case::start_crypt_key(client_in_start(), encoded_crypt_key())]
#[case::start_wave_encrypt(client_in_start(), encoded_wave_encrypt())]
#[case::waiting_volume(client_in_waiting(), encoded_volume())]
#[case::waiting_pitch(client_in_waiting(), encoded_pitch())]
#[case::waiting_close(client_in_waiting(), encoded_close())]
#[case::waiting_wave(client_in_waiting(), encoded_wave())]
#[case::waiting_wave2(client_in_waiting(), encoded_wave2(0))]
#[case::waiting_audio_format(client_in_waiting(), encoded_server_formats(pdu::Version::V8))]
#[case::waiting_crypt_key(client_in_waiting(), encoded_crypt_key())]
#[case::waiting_wave_encrypt(client_in_waiting(), encoded_wave_encrypt())]
fn transitions_to_stop_on_invalid_pdu(#[case] mut client: Rdpsnd, #[case] payload: Vec<u8>) {
    let responses = client.process(&payload).unwrap();
    assert!(responses.is_empty(), "invalid PDU should produce no responses");
    assert_in_stop_state(&mut client);
}

// ============================================================================
// Happy-path tests: Ready state
// ============================================================================

type RecordedWaves = Arc<Mutex<Vec<(AudioFormat, u32, Vec<u8>)>>>;

#[derive(Debug, Default)]
struct RecordingBackend {
    formats: Vec<AudioFormat>,
    waves: RecordedWaves,
}

impl RecordingBackend {
    fn with_formats(formats: Vec<AudioFormat>) -> Self {
        Self {
            formats,
            waves: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl RdpsndClientHandler for RecordingBackend {
    fn get_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn wave(&mut self, format: &AudioFormat, ts: u32, data: Cow<'_, [u8]>) {
        self.waves.lock().unwrap().push((format.clone(), ts, data.into_owned()));
    }

    fn set_volume(&mut self, _volume: VolumePdu) {}

    fn set_pitch(&mut self, _pitch: PitchPdu) {}

    fn close(&mut self) {}
}

#[rstest]
#[case::volume(encoded_volume())]
#[case::pitch(encoded_pitch())]
#[case::close(encoded_close())]
fn ready_silent_pdus_keep_state(#[case] payload: Vec<u8>) {
    let mut client = client_in_ready(pdu::Version::V8);

    let responses = client.process(&payload).unwrap();
    assert!(responses.is_empty(), "silent PDU should produce no responses");

    // Verify the client remains in Ready by processing a Wave2.
    let responses = client.process(&encoded_wave2(1)).unwrap();
    assert_eq!(responses.len(), 1, "wave2 should still produce WaveConfirm");
}

#[test]
fn ready_wave_two_step_sends_confirm_and_keeps_state() {
    let waves = Arc::new(Mutex::new(Vec::new()));
    let backend = RecordingBackend {
        formats: vec![pcm(44100, 2)],
        waves: Arc::clone(&waves),
    };
    let mut client = Rdpsnd::new(Box::new(backend));
    client.process(&encoded_server_formats(pdu::Version::V5)).unwrap();
    client.process(&encoded_training()).unwrap();

    // WaveInfo alone does not confirm yet — waiting for bare Wave payload.
    assert!(client.process(&encoded_wave_info(8)).unwrap().is_empty());

    let confirm = decode_single_response(&client.process(&encoded_wave_data(&[0x05, 0x06, 0x07, 0x08])).unwrap());
    assert!(matches!(confirm, pdu::ClientAudioOutputPdu::WaveConfirm(ref pdu) if pdu.block_no == 1));

    let recorded = waves.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].1, 0xADD7);
    assert_eq!(recorded[0].2, vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
    drop(recorded);

    // Channel stays Ready for a subsequent Wave2.
    let confirm = decode_single_response(&client.process(&encoded_wave2(2)).unwrap());
    assert!(matches!(confirm, pdu::ClientAudioOutputPdu::WaveConfirm(_)));
}

#[test]
fn ready_wave_concatenated_payload_finishes_immediately() {
    let mut client = client_in_ready(pdu::Version::V5);

    let mut payload = encoded_wave_info(8);
    payload.extend_from_slice(&encoded_wave_data(&[0x05, 0x06, 0x07, 0x08]));

    let confirm = decode_single_response(&client.process(&payload).unwrap());
    assert!(matches!(confirm, pdu::ClientAudioOutputPdu::WaveConfirm(ref pdu) if pdu.block_no == 1));
}

#[test]
fn ready_unsupported_optional_pdus_do_not_stop_channel() {
    let mut client = client_in_ready(pdu::Version::V8);

    assert!(client.process(&encoded_crypt_key()).unwrap().is_empty());
    assert!(client.process(&encoded_wave_encrypt()).unwrap().is_empty());

    let confirm = decode_single_response(&client.process(&encoded_wave2(1)).unwrap());
    assert!(matches!(confirm, pdu::ClientAudioOutputPdu::WaveConfirm(_)));
}

#[test]
fn client_format_list_preserves_handler_order_and_wave_index() {
    // Handler offers Opus (unsupported by server), then 48 kHz PCM, then 44.1 kHz PCM.
    // Server offers only the two PCM rates (44.1 first in server list — irrelevant to client index).
    let waves = Arc::new(Mutex::new(Vec::new()));
    let backend = RecordingBackend {
        formats: vec![
            AudioFormat {
                format: WaveFormat::OPUS,
                n_channels: 2,
                n_samples_per_sec: 48000,
                n_avg_bytes_per_sec: 192000,
                n_block_align: 4,
                bits_per_sample: 16,
                data: None,
            },
            pcm(48000, 2),
            pcm(44100, 2),
        ],
        waves: Arc::clone(&waves),
    };

    let mut client = Rdpsnd::new(Box::new(backend));
    let responses = client
        .process(&encoded_server_formats_with(
            pdu::Version::V8,
            vec![pcm(44100, 2), pcm(48000, 2)],
        ))
        .unwrap();

    let encoded = responses[0].encode_unframed_pdu().unwrap();
    let pdu::ClientAudioOutputPdu::AudioFormat(client_fmt) = decode(&encoded).unwrap() else {
        panic!("expected ClientAudioFormat");
    };
    // Client list must keep handler order among matches: 48 kHz then 44.1 kHz.
    assert_eq!(client_fmt.formats.len(), 2);
    assert_eq!(client_fmt.formats[0].n_samples_per_sec, 48000);
    assert_eq!(client_fmt.formats[1].n_samples_per_sec, 44100);

    client.process(&encoded_training()).unwrap();

    // wFormatNo 1 is the second client format (44.1 kHz), not server index 1 (48 kHz).
    let confirm = decode_single_response(&client.process(&encoded_wave2_with(7, 1)).unwrap());
    assert!(matches!(confirm, pdu::ClientAudioOutputPdu::WaveConfirm(_)));

    let recorded = waves.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0.n_samples_per_sec, 44100);
    assert_eq!(recorded[0].1, 0xDACB8C2);
    assert_eq!(recorded[0].2, vec![0x01, 0x02, 0x03, 0x04]);
}

#[test]
fn client_format_negotiation_ignores_derived_fields() {
    let mut server = pcm(44100, 2);
    server.n_avg_bytes_per_sec = 0;
    server.n_block_align = 99;

    let backend = RecordingBackend::with_formats(vec![pcm(44100, 2)]);
    let mut client = Rdpsnd::new(Box::new(backend));
    let responses = client
        .process(&encoded_server_formats_with(pdu::Version::V8, vec![server]))
        .unwrap();

    let encoded = responses[0].encode_unframed_pdu().unwrap();
    let pdu::ClientAudioOutputPdu::AudioFormat(client_fmt) = decode(&encoded).unwrap() else {
        panic!("expected ClientAudioFormat");
    };
    assert_eq!(client_fmt.formats.len(), 1);
    assert_eq!(client_fmt.formats[0].n_samples_per_sec, 44100);
}

#[test]
fn ready_training_sends_confirm() {
    let mut client = client_in_ready(pdu::Version::V8);

    let confirm = decode_single_response(&client.process(&encoded_training()).unwrap());
    assert!(matches!(confirm, pdu::ClientAudioOutputPdu::TrainingConfirm(_)));

    // Verify the client remains in Ready.
    let responses = client.process(&encoded_wave2(1)).unwrap();
    assert_eq!(responses.len(), 1);
}

// Ready -> AudioFormat -> QualityMode -> Training -> Wave2
//
// Verifies that receiving a new AudioFormat PDU in Ready state restarts
// the negotiation sequence and that audio resumes normally afterward.
#[test]
fn ready_audio_format_v6_restarts_negotiation() {
    let mut client = client_in_ready(pdu::Version::V6);

    let responses = client.process(&encoded_server_formats(pdu::Version::V6)).unwrap();

    // V6 >= V6: client should reply with AudioFormat + QualityMode.
    assert_eq!(responses.len(), 2);
    let encoded = responses[0].encode_unframed_pdu().unwrap();
    assert!(matches!(
        decode::<pdu::ClientAudioOutputPdu>(&encoded).unwrap(),
        pdu::ClientAudioOutputPdu::AudioFormat(_)
    ));
    let encoded = responses[1].encode_unframed_pdu().unwrap();
    assert!(matches!(
        decode::<pdu::ClientAudioOutputPdu>(&encoded).unwrap(),
        pdu::ClientAudioOutputPdu::QualityMode(_)
    ));

    let confirm = decode_single_response(&client.process(&encoded_training()).unwrap());
    assert!(matches!(confirm, pdu::ClientAudioOutputPdu::TrainingConfirm(_)));

    let confirm = decode_single_response(&client.process(&encoded_wave2(1)).unwrap());
    assert!(matches!(confirm, pdu::ClientAudioOutputPdu::WaveConfirm(_)));
}

// Renegotiation with version < V6 should not send QualityMode.
#[test]
fn ready_audio_format_v5_skips_quality_mode() {
    let mut client = client_in_ready(pdu::Version::V5);

    let confirm = decode_single_response(&client.process(&encoded_server_formats(pdu::Version::V5)).unwrap());
    assert!(matches!(confirm, pdu::ClientAudioOutputPdu::AudioFormat(_)));

    let confirm = decode_single_response(&client.process(&encoded_training()).unwrap());
    assert!(matches!(confirm, pdu::ClientAudioOutputPdu::TrainingConfirm(_)));

    let confirm = decode_single_response(&client.process(&encoded_wave2(1)).unwrap());
    assert!(matches!(confirm, pdu::ClientAudioOutputPdu::WaveConfirm(_)));
}

// Repeated renegotiation: Ready -> AudioFormat -> Training -> Ready -> AudioFormat -> ...
//
// Ensures that multiple consecutive renegotiation cycles do not corrupt
// internal state.
#[test]
fn ready_repeated_renegotiation_is_stable() {
    let mut client = client_in_ready(pdu::Version::V6);

    let server_formats = encoded_server_formats(pdu::Version::V6);
    let training = encoded_training();

    for cycle in 0u8..3 {
        let responses = client.process(&server_formats).unwrap();
        assert_eq!(responses.len(), 2, "cycle {cycle}: expected AudioFormat + QualityMode");

        let responses = client.process(&training).unwrap();
        assert_eq!(responses.len(), 1, "cycle {cycle}: expected TrainingConfirm");

        let confirm = decode_single_response(&client.process(&encoded_wave2(cycle)).unwrap());
        assert!(matches!(confirm, pdu::ClientAudioOutputPdu::WaveConfirm(_)));
    }
}

// ============================================================================
// Terminal state: Stop ignores every PDU type
// ============================================================================

#[rstest]
#[case::audio_format(encoded_server_formats(pdu::Version::V8))]
#[case::training(encoded_training())]
#[case::wave(encoded_wave())]
#[case::wave2(encoded_wave2(0))]
#[case::volume(encoded_volume())]
#[case::pitch(encoded_pitch())]
#[case::close(encoded_close())]
#[case::crypt_key(encoded_crypt_key())]
#[case::wave_encrypt(encoded_wave_encrypt())]
fn stop_ignores_all_pdus(#[case] payload: Vec<u8>) {
    let mut client = client_in_stop();

    let responses = client.process(&payload).unwrap();
    assert!(responses.is_empty(), "Stop state should ignore all PDUs");
}
