use std::borrow::Cow;

use ironrdp_core::{decode, encode_vec};
use ironrdp_rdpsnd::client::{NoopRdpsndBackend, Rdpsnd};
use ironrdp_rdpsnd::pdu;
use ironrdp_svc::SvcProcessor as _;
use rstest::rstest;

// ============================================================================
// Encoding helpers
// ============================================================================

fn encoded_server_formats(version: pdu::Version) -> Vec<u8> {
    encode_vec(&pdu::ServerAudioOutputPdu::AudioFormat(pdu::ServerAudioFormatPdu {
        version,
        formats: vec![pdu::AudioFormat {
            format: pdu::WaveFormat::PCM,
            n_channels: 2,
            n_samples_per_sec: 44100,
            n_avg_bytes_per_sec: 176400,
            n_block_align: 4,
            bits_per_sample: 16,
            data: None,
        }],
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
    encode_vec(&pdu::ServerAudioOutputPdu::Wave2(pdu::Wave2Pdu {
        timestamp: 0xA116,
        format_no: 0,
        block_no,
        audio_timestamp: 0xDACB8C2,
        data: Cow::Borrowed(&[0x01, 0x02, 0x03, 0x04]),
    }))
    .unwrap()
}

fn encoded_volume() -> Vec<u8> {
    encode_vec(&pdu::ServerAudioOutputPdu::Volume(pdu::VolumePdu {
        volume_left: 0x8000,
        volume_right: 0x8000,
    }))
    .unwrap()
}

fn encoded_pitch() -> Vec<u8> {
    encode_vec(&pdu::ServerAudioOutputPdu::Pitch(pdu::PitchPdu { pitch: 0x00010000 })).unwrap()
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

fn encoded_wave() -> Vec<u8> {
    encode_vec(&pdu::ServerAudioOutputPdu::Wave(pdu::WavePdu {
        timestamp: 0xADD7,
        format_no: 0,
        block_no: 1,
        data: Cow::Borrowed(&[0x01, 0x02, 0x03, 0x04]),
    }))
    .unwrap()
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
