//! Audio output over the dynamic channels and UDP, and Wave Confirm timing.

use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;
use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use ironrdp_core::{Decode as _, ReadCursor, decode, encode_vec};
use ironrdp_dvc::pdu::{
    CapabilitiesRequestPdu, CapsVersion, CreateRequestPdu, CreationStatus, DataPdu, DrdynvcClientPdu, DrdynvcDataPdu,
    DrdynvcServerPdu, SoftSyncChannelList, SoftSyncRequestPdu, SoftSyncTunnelType,
};
use ironrdp_dvc::{DrdynvcClient, DvcMessage, DvcProcessor as _};
use ironrdp_rdpsnd::client::{
    DVC_CHANNEL_NAME, LOSSY_DVC_CHANNEL_NAME, Rdpsnd, RdpsndClientHandler, RdpsndDvc, RdpsndDvcState,
};
use ironrdp_rdpsnd::codec;
use ironrdp_rdpsnd::pdu::{self, AudioFormat, PitchPdu, VolumePdu, WaveFormat};
use ironrdp_svc::{SvcMessage, SvcProcessor as _};
use rstest::rstest;

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

fn server_formats(formats: Vec<AudioFormat>) -> Vec<u8> {
    encode_vec(&pdu::ServerAudioOutputPdu::AudioFormat(pdu::ServerAudioFormatPdu {
        version: pdu::Version::V8,
        formats,
    }))
    .unwrap()
}

fn training() -> Vec<u8> {
    encode_vec(&pdu::ServerAudioOutputPdu::Training(pdu::TrainingPdu {
        timestamp: 0x1234,
        data: vec![],
    }))
    .unwrap()
}

fn wave2(timestamp: u16, block_no: u8, format_no: u16, data: &[u8]) -> Vec<u8> {
    encode_vec(&pdu::ServerAudioOutputPdu::Wave2(pdu::Wave2Pdu {
        timestamp,
        format_no,
        block_no,
        audio_timestamp: 0x0102_0304,
        data: Cow::Borrowed(data),
    }))
    .unwrap()
}

fn decode_svc(message: &SvcMessage) -> pdu::ClientAudioOutputPdu {
    decode(&message.encode_unframed_pdu().unwrap()).unwrap()
}

fn decode_dvc(message: &DvcMessage) -> pdu::ClientAudioOutputPdu {
    decode(&encode_vec(message.as_ref()).unwrap()).unwrap()
}

fn wave_confirm(message: pdu::ClientAudioOutputPdu) -> pdu::WaveConfirmPdu {
    let pdu::ClientAudioOutputPdu::WaveConfirm(confirm) = message else {
        panic!("expected a Wave Confirm PDU, got {message:?}");
    };
    confirm
}

type Waves = Arc<Mutex<Vec<(AudioFormat, Vec<u8>)>>>;

/// Handler that records waves, reports a fixed latency and advances a shared clock on each wave.
#[derive(Debug)]
struct TimedBackend {
    formats: Vec<AudioFormat>,
    waves: Waves,
    latency: Duration,
    clock_ms: Arc<AtomicU64>,
    processing_ms: u64,
}

impl TimedBackend {
    fn new(formats: Vec<AudioFormat>) -> Self {
        Self {
            formats,
            waves: Waves::default(),
            latency: Duration::ZERO,
            clock_ms: Arc::new(AtomicU64::new(1_000)),
            processing_ms: 0,
        }
    }

    fn clock(&self) -> ironrdp_rdpsnd::client::Clock {
        let clock_ms = Arc::clone(&self.clock_ms);
        Box::new(move || Duration::from_millis(clock_ms.load(Ordering::SeqCst)))
    }
}

impl RdpsndClientHandler for TimedBackend {
    fn get_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn wave(&mut self, format: &AudioFormat, _ts: u32, data: Cow<'_, [u8]>) {
        self.clock_ms.fetch_add(self.processing_ms, Ordering::SeqCst);
        self.waves.lock().unwrap().push((format.clone(), data.into_owned()));
    }

    fn playback_latency(&self) -> Duration {
        self.latency
    }

    fn set_volume(&mut self, _volume: VolumePdu) {}

    fn set_pitch(&mut self, _pitch: PitchPdu) {}

    fn close(&mut self) {}
}

// ============================================================================
// Wave Confirm timing
// ============================================================================

// oracle: spec MS-RDPEA §3.2.5.2.1.6 (timestamp plus the time until the sample is emitted)
#[rstest]
#[case::no_delay(0xA116, 0, 0, 0xA116)]
#[case::playback_latency_only(0xA116, 0, 120, 0xA116 + 120)]
#[case::held_and_playback_latency(0xA116, 7, 120, 0xA116 + 127)]
#[case::wraps_around(0xFFF0, 5, 30, 0x0013)]
fn wave_confirm_reports_hold_time_and_playback_latency(
    #[case] server_timestamp: u16,
    #[case] processing_ms: u64,
    #[case] latency_ms: u64,
    #[case] expected: u16,
) {
    let mut backend = TimedBackend::new(vec![pcm(44100, 2)]);
    backend.processing_ms = processing_ms;
    backend.latency = Duration::from_millis(latency_ms);
    let clock = backend.clock();
    let mut client = Rdpsnd::new(Box::new(backend)).with_clock(clock);
    client.process(&server_formats(vec![pcm(44100, 2)])).unwrap();
    client.process(&training()).unwrap();

    let responses = client.process(&wave2(server_timestamp, 9, 0, &[0; 8])).unwrap();

    let [response] = responses.as_slice() else {
        panic!("expected one response");
    };
    let confirm = wave_confirm(decode_svc(response));
    assert_eq!(confirm.timestamp, expected);
    assert_eq!(confirm.block_no, 9);
}

// oracle: spec MS-RDPEA §3.2.5.2.1.6
#[test]
fn wave_confirm_without_clock_reports_playback_latency() {
    let mut backend = TimedBackend::new(vec![pcm(44100, 2)]);
    backend.processing_ms = 50; // Not observable without a clock.
    backend.latency = Duration::from_millis(80);
    let mut client = Rdpsnd::new(Box::new(backend));
    client.process(&server_formats(vec![pcm(44100, 2)])).unwrap();
    client.process(&training()).unwrap();

    let responses = client.process(&wave2(100, 1, 0, &[0; 4])).unwrap();

    assert_eq!(wave_confirm(decode_svc(&responses[0])).timestamp, 180);
}

// ============================================================================
// Format choice
// ============================================================================

#[derive(Debug)]
struct ChoosingBackend {
    chosen: Vec<AudioFormat>,
}

impl RdpsndClientHandler for ChoosingBackend {
    fn get_formats(&self) -> &[AudioFormat] {
        &[]
    }

    fn choose_formats(&self, _server_formats: &[AudioFormat]) -> Vec<AudioFormat> {
        self.chosen.clone()
    }

    fn wave(&mut self, _format: &AudioFormat, _ts: u32, _data: Cow<'_, [u8]>) {}

    fn set_volume(&mut self, _volume: VolumePdu) {}

    fn set_pitch(&mut self, _pitch: PitchPdu) {}

    fn close(&mut self) {}
}

// oracle: spec MS-RDPEA §2.2.2.2 (sndFormats must appear in the server's list)
#[test]
fn chosen_formats_are_advertised_in_handler_order_and_limited_to_offered_ones() {
    let adpcm = codec::ms_adpcm_format(2, 22050, 2048);
    let alaw = codec::alaw_format(1, 8000);
    let not_offered = codec::ima_adpcm_format(1, 8000, 256);
    let mut client = Rdpsnd::new(Box::new(ChoosingBackend {
        chosen: vec![alaw.clone(), not_offered, adpcm.clone()],
    }));

    let responses = client
        .process(&server_formats(vec![pcm(44100, 2), adpcm.clone(), alaw.clone()]))
        .unwrap();

    let pdu::ClientAudioOutputPdu::AudioFormat(advertised) = decode_svc(&responses[0]) else {
        panic!("expected the Client Audio Formats PDU");
    };
    assert_eq!(advertised.formats, vec![alaw, adpcm]);
}

// ============================================================================
// Dynamic channels
// ============================================================================

// oracle: spec MS-RDPEA §2.1 and §3.2.5.1
#[test]
fn dvc_client_negotiates_and_confirms_on_its_channel() {
    let backend = TimedBackend::new(vec![pcm(48000, 2)]);
    let waves = Arc::clone(&backend.waves);
    let mut client = RdpsndDvcState::new(Box::new(backend))
        .with_quality_mode(pdu::QualityMode::Dynamic)
        .channel();
    assert_eq!(client.channel_name(), DVC_CHANNEL_NAME);

    assert!(client.start(3).unwrap().is_empty());
    let replies = client.process(3, &server_formats(vec![pcm(48000, 2)])).unwrap();
    let replies: Vec<_> = replies.iter().map(decode_dvc).collect();
    let [
        pdu::ClientAudioOutputPdu::AudioFormat(formats),
        pdu::ClientAudioOutputPdu::QualityMode(quality),
    ] = replies.as_slice()
    else {
        panic!("expected the formats and quality mode replies, got {replies:?}");
    };
    assert_eq!(formats.formats, vec![pcm(48000, 2)]);
    assert_eq!(quality.quality_mode, pdu::QualityMode::Dynamic);

    let replies = client.process(3, &training()).unwrap();
    assert!(matches!(
        decode_dvc(&replies[0]),
        pdu::ClientAudioOutputPdu::TrainingConfirm(_)
    ));

    let replies = client.process(3, &wave2(0x0400, 4, 0, &[1, 2, 3, 4])).unwrap();
    assert_eq!(wave_confirm(decode_dvc(&replies[0])).block_no, 4);
    assert_eq!(waves.lock().unwrap().as_slice(), [(pcm(48000, 2), vec![1, 2, 3, 4])]);
}

// oracle: mutant formats of a closed channel reused by its successor
#[test]
fn reopened_dvc_client_waits_for_a_new_negotiation() {
    let backend = TimedBackend::new(vec![pcm(48000, 2)]);
    let waves = Arc::clone(&backend.waves);
    let mut client = RdpsndDvc::new(Box::new(backend));
    client.start(3).unwrap();
    client.process(3, &server_formats(vec![pcm(48000, 2)])).unwrap();
    client.process(3, &training()).unwrap();
    client.close(3);

    // The negotiated formats belong to the closed channel, so a Wave PDU is out of sequence.
    client.start(4).unwrap();
    assert!(client.process(4, &wave2(1, 1, 0, &[0; 4])).unwrap().is_empty());
    assert!(waves.lock().unwrap().is_empty());
}

// oracle: reference Windows Server 2025 (26100) live capture: AAC Wave2 PDUs on AUDIO_PLAYBACK_LOSSY_DVC index the AUDIO_PLAYBACK_DVC formats
#[test]
fn lossy_channel_plays_formats_negotiated_on_the_reliable_channel() {
    let backend = TimedBackend::new(vec![pcm(48000, 2), pcm(44100, 2)]);
    let waves = Arc::clone(&backend.waves);
    let state = RdpsndDvcState::new(Box::new(backend));
    let mut reliable = state.channel();
    let mut lossy = state.lossy_channel();
    assert_eq!(reliable.channel_name(), DVC_CHANNEL_NAME);
    assert_eq!(lossy.channel_name(), LOSSY_DVC_CHANNEL_NAME);
    reliable.start(5).unwrap();
    lossy.start(6).unwrap();
    reliable
        .process(5, &server_formats(vec![pcm(48000, 2), pcm(44100, 2)]))
        .unwrap();
    reliable.process(5, &training()).unwrap();

    // Windows sends AAC wave data on the lossy channel, indexing the reliable channel's formats.
    let replies = lossy.process(6, &wave2(1, 1, 1, &[9; 4])).unwrap();

    assert!(replies.is_empty(), "waves on the lossy channel are not confirmed");
    assert_eq!(waves.lock().unwrap().as_slice(), [(pcm(44100, 2), vec![9; 4])]);
}

// oracle: reference Windows Server 2025 (26100) live capture: the lossy channel reopens without renegotiation
#[test]
fn reopening_the_lossy_channel_keeps_the_negotiation() {
    let backend = TimedBackend::new(vec![pcm(48000, 2)]);
    let waves = Arc::clone(&backend.waves);
    let state = RdpsndDvcState::new(Box::new(backend));
    let mut reliable = state.channel();
    reliable.start(5).unwrap();
    reliable.process(5, &server_formats(vec![pcm(48000, 2)])).unwrap();
    reliable.process(5, &training()).unwrap();

    let mut lossy = state.lossy_channel();
    lossy.start(6).unwrap();
    lossy.close(6);
    let mut lossy = state.lossy_channel();
    lossy.start(7).unwrap();
    lossy.process(7, &wave2(1, 1, 0, &[1; 4])).unwrap();

    assert_eq!(waves.lock().unwrap().len(), 1);
    // The reliable channel still confirms.
    let replies = reliable.process(5, &wave2(2, 2, 0, &[2; 4])).unwrap();
    assert_eq!(wave_confirm(decode_dvc(&replies[0])).block_no, 2);
}

// oracle: reference Windows Server 2025 (26100) live capture: formats are negotiated on AUDIO_PLAYBACK_DVC only
#[test]
fn lossy_channel_cannot_renegotiate_or_play_before_negotiation() {
    let backend = TimedBackend::new(vec![pcm(48000, 2), pcm(44100, 2)]);
    let waves = Arc::clone(&backend.waves);
    let state = RdpsndDvcState::new(Box::new(backend));
    let mut reliable = state.channel();
    let mut lossy = state.lossy_channel();
    reliable.start(5).unwrap();
    lossy.start(6).unwrap();

    // Before the negotiation, wave data on the lossy channel is dropped without changing state.
    assert!(lossy.process(6, &wave2(1, 1, 0, &[1; 4])).unwrap().is_empty());
    reliable
        .process(5, &server_formats(vec![pcm(48000, 2), pcm(44100, 2)]))
        .unwrap();
    reliable.process(5, &training()).unwrap();
    // A format list on the lossy channel does not replace the negotiated one.
    assert!(
        lossy
            .process(6, &server_formats(vec![pcm(44100, 2)]))
            .unwrap()
            .is_empty()
    );
    lossy.process(6, &wave2(2, 2, 1, &[2; 4])).unwrap();

    assert_eq!(waves.lock().unwrap().as_slice(), [(pcm(44100, 2), vec![2; 4])]);
}

fn decode_drdynvc(message: &SvcMessage) -> DrdynvcClientPdu {
    let encoded = message.encode_unframed_pdu().unwrap();
    DrdynvcClientPdu::decode(&mut ReadCursor::new(&encoded)).unwrap()
}

// oracle: mutant audio channel declined or answered off the tunnel it was created on
#[test]
fn audio_playback_dvc_opens_on_the_udp_tunnel() {
    let mut drdynvc =
        DrdynvcClient::new().with_dynamic_channel(RdpsndDvc::new(Box::new(TimedBackend::new(vec![pcm(44100, 2)]))));
    let caps = encode_vec(&DrdynvcServerPdu::Capabilities(CapabilitiesRequestPdu::new(
        CapsVersion::V3,
        None,
    )))
    .unwrap();
    drdynvc.process(&caps).unwrap();
    drdynvc.enable_soft_sync_tunnel(SoftSyncTunnelType::RELIABLE_UDP);
    let soft_sync = encode_vec(&DrdynvcServerPdu::SoftSyncRequest(SoftSyncRequestPdu::new(vec![
        SoftSyncChannelList::new(SoftSyncTunnelType::RELIABLE_UDP, vec![]),
    ])))
    .unwrap();
    drdynvc.process(&soft_sync).unwrap();

    // Windows creates AUDIO_PLAYBACK_DVC directly on the tunnel once Soft-Sync switched to it.
    let create = encode_vec(&DrdynvcServerPdu::Create(CreateRequestPdu::new(
        21,
        DVC_CHANNEL_NAME.to_owned(),
    )))
    .unwrap();
    let batch = drdynvc
        .process_tunnel(SoftSyncTunnelType::RELIABLE_UDP, &create)
        .unwrap();
    let DrdynvcClientPdu::Create(response) = decode_drdynvc(&batch.messages()[0]) else {
        panic!("expected a Create Response");
    };
    assert_eq!(response.creation_status(), CreationStatus::OK);
    assert_eq!(drdynvc.tunnel_for_channel(21), Some(SoftSyncTunnelType::RELIABLE_UDP));

    let formats = encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
        21,
        server_formats(vec![pcm(44100, 2)]),
    ))))
    .unwrap();
    let batch = drdynvc
        .process_tunnel(SoftSyncTunnelType::RELIABLE_UDP, &formats)
        .unwrap();

    // The replies go back on the tunnel, addressed to the audio channel.
    assert_eq!(batch.channel_id(), 21);
    let DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(reply)) = decode_drdynvc(&batch.messages()[0]) else {
        panic!("expected DVC data");
    };
    assert!(matches!(
        decode::<pdu::ClientAudioOutputPdu>(reply.data()).unwrap(),
        pdu::ClientAudioOutputPdu::AudioFormat(_)
    ));
}

// ============================================================================
// UDP
// ============================================================================

// oracle: spec MS-RDPEA §2.2.3.6
#[rstest]
#[case::short_fragment_number(0x7F, &[0x0A, 0x05, 0x7F, 0xAA])]
#[case::long_fragment_number(0x0180, &[0x0A, 0x05, 0x81, 0x80, 0xAA])]
#[case::largest_fragment_number(0x7FFF, &[0x0A, 0x05, 0xFF, 0xFF, 0xAA])]
fn udp_wave_fragment_number_encoding(#[case] frag_no: u16, #[case] wire: &[u8]) {
    let pdu = pdu::UdpWavePdu {
        block_no: 5,
        frag_no,
        data: Cow::Borrowed(&[0xAA]),
    };

    assert_eq!(encode_vec(&pdu).unwrap(), wire);
    assert_eq!(decode::<pdu::UdpWavePdu<'_>>(wire).unwrap(), pdu);
}

// oracle: spec MS-RDPEA §2.2.3.6
#[test]
fn udp_wave_rejects_fragment_number_beyond_15_bits() {
    let pdu = pdu::UdpWavePdu {
        block_no: 0,
        frag_no: 0x8000,
        data: Cow::Borrowed(&[]),
    };

    assert!(encode_vec(&pdu).is_err());
}

// oracle: spec MS-RDPEA §2.2.3.7
#[test]
fn udp_wave_last_round_trip() {
    let wire = [
        0x0B, // SNDC_UDPWAVELAST
        0x0C, 0x00, // wTotalSize
        0x34, 0x12, // wTimeStamp
        0x01, 0x00, // wFormatNo
        0x07, // cBlockNo
        0x00, 0x00, 0x00, // bPad
        0xDE, 0xAD,
    ];
    let pdu = pdu::UdpWaveLastPdu {
        total_size: 12,
        timestamp: 0x1234,
        format_no: 1,
        block_no: 7,
        data: Cow::Borrowed(&[0xDE, 0xAD]),
    };

    assert_eq!(decode::<pdu::UdpWaveLastPdu<'_>>(&wire).unwrap(), pdu);
    assert_eq!(encode_vec(&pdu).unwrap(), wire);
}

/// `AUDIO_FRAGDATA` signatures computed independently with Python's `hashlib`: the first eight bytes
/// of SHA-1 over the Crypt Key seed, the block number, three zero bytes and the audio data.
const SIGNATURE_BLOCK_3: [u8; 8] = [0x9F, 0x90, 0xCE, 0x73, 0x06, 0x20, 0x4E, 0x96]; // zero seed, audio 0..40
const SIGNATURE_BLOCK_1: [u8; 8] = [0xC8, 0x57, 0x42, 0x3D, 0x83, 0x0E, 0xEE, 0x05]; // zero seed, [1; 20]
const SIGNATURE_BLOCK_2: [u8; 8] = [0x6B, 0x12, 0xA2, 0x94, 0xA4, 0xC6, 0x45, 0x27]; // zero seed, [2; 20]
const SIGNATURE_BLOCK_200: [u8; 8] = [0xAB, 0x03, 0xC6, 0x77, 0x27, 0x43, 0xEC, 0x5D]; // zero seed, [3; 20]
const SIGNATURE_SEEDED: [u8; 8] = [0x5E, 0xBB, 0x1C, 0x46, 0x4B, 0x5E, 0x6D, 0x5D]; // seed [0xAB; 32], block 4, [7; 16]

/// Splits a signed sample into the UDP Wave fragments and the UDP Wave Last PDU of block `block_no`.
fn fragment(
    signature: [u8; 8],
    audio: &[u8],
    fragment_size: usize,
    block_no: u8,
    timestamp: u16,
) -> (Vec<Vec<u8>>, Vec<u8>) {
    let mut fragdata = signature.to_vec();
    fragdata.extend_from_slice(audio);
    let mut chunks: Vec<&[u8]> = fragdata.chunks(fragment_size).collect();
    let last = chunks.pop().unwrap();
    let fragments = chunks
        .iter()
        .enumerate()
        .map(|(frag_no, data)| udp_wave(block_no, u16::try_from(frag_no).unwrap(), data))
        .collect();
    let last = encode_vec(&pdu::UdpWaveLastPdu {
        total_size: u16::try_from(fragdata.len()).unwrap(),
        timestamp,
        format_no: 0,
        block_no,
        data: Cow::Borrowed(last),
    })
    .unwrap();
    (fragments, last)
}

fn udp_wave(block_no: u8, frag_no: u16, data: &[u8]) -> Vec<u8> {
    encode_vec(&pdu::UdpWavePdu {
        block_no,
        frag_no,
        data: Cow::Borrowed(data),
    })
    .unwrap()
}

fn udp_client(waves: &Waves) -> Rdpsnd {
    let backend = TimedBackend {
        waves: Arc::clone(waves),
        ..TimedBackend::new(vec![pcm(22050, 1)])
    };
    let mut client = Rdpsnd::new(Box::new(backend)).with_datagram_port(4321);
    let responses = client.process(&server_formats(vec![pcm(22050, 1)])).unwrap();
    let pdu::ClientAudioOutputPdu::AudioFormat(advertised) = decode_svc(&responses[0]) else {
        panic!("expected the Client Audio Formats PDU");
    };
    assert_eq!(advertised.dgram_port, 4321);
    client.process(&training()).unwrap();
    client
}

fn deliver(client: &mut Rdpsnd, (fragments, last): &(Vec<Vec<u8>>, Vec<u8>)) -> Vec<pdu::ClientAudioOutputPdu> {
    let mut replies = Vec::new();
    for datagram in fragments.iter().chain([last]) {
        replies.extend(client.process_datagram(datagram).unwrap());
    }
    replies
}

// oracle: spec MS-RDPEA §3.2.5.1.1.5
#[test]
fn udp_training_is_confirmed() {
    let mut client = udp_client(&Waves::default());

    let replies = client.process_datagram(&training()).unwrap();

    assert!(matches!(
        replies.as_slice(),
        [pdu::ClientAudioOutputPdu::TrainingConfirm(pdu::TrainingConfirmPdu {
            timestamp: 0x1234,
            ..
        })]
    ));
}

// oracle: spec MS-RDPEA §3.2.5.2.1.5
#[test]
fn udp_wave_fragments_reassemble_in_any_order() {
    let waves = Waves::default();
    let mut client = udp_client(&waves);
    let audio: Vec<u8> = (0..40).collect();
    let (fragments, last) = fragment(SIGNATURE_BLOCK_3, &audio, 12, 3, 0x2000);
    assert_eq!(fragments.len(), 3);

    // The last PDU and the fragments arrive out of order; nothing plays until all are in.
    assert!(client.process_datagram(&fragments[2]).unwrap().is_empty());
    assert!(client.process_datagram(&last).unwrap().is_empty());
    assert!(client.process_datagram(&fragments[0]).unwrap().is_empty());
    let replies = client.process_datagram(&fragments[1]).unwrap();

    let [pdu::ClientAudioOutputPdu::WaveConfirm(confirm)] = replies.as_slice() else {
        panic!("expected a Wave Confirm PDU, got {replies:?}");
    };
    assert_eq!(confirm.block_no, 3);
    assert_eq!(confirm.timestamp, 0x2000);
    assert_eq!(waves.lock().unwrap().as_slice(), [(pcm(22050, 1), audio)]);
}

// oracle: spec MS-RDPEA §3.3.5.2.1.3 (signature computed with Python hashlib)
#[test]
fn udp_wave_signature_uses_the_crypt_key_seed() {
    let waves = Waves::default();
    let mut client = udp_client(&waves);
    let crypt_key = encode_vec(&pdu::ServerAudioOutputPdu::CryptKey(pdu::CryptKeyPdu {
        seed: [0xAB; 32],
    }))
    .unwrap();
    client.process(&crypt_key).unwrap();

    let replies = deliver(&mut client, &fragment(SIGNATURE_SEEDED, &[7; 16], 10, 4, 0));

    assert_eq!(replies.len(), 1);
    assert_eq!(waves.lock().unwrap().as_slice(), [(pcm(22050, 1), vec![7; 16])]);
}

// oracle: spec MS-RDPEA §3.3.5.2.1.3
#[test]
fn udp_wave_with_a_wrong_signature_is_dropped() {
    let waves = Waves::default();
    let mut client = udp_client(&waves);

    // The signature of a different block does not authenticate this one.
    let replies = deliver(&mut client, &fragment(SIGNATURE_BLOCK_1, &[2; 20], 10, 2, 0));

    assert!(
        replies.is_empty(),
        "an unauthenticated sample is neither played nor confirmed"
    );
    assert!(waves.lock().unwrap().is_empty());
}

// oracle: spec MS-RDPEA §3.2.5.2.1.1
#[test]
fn udp_wave_for_a_later_block_abandons_the_incomplete_one() {
    let waves = Waves::default();
    let mut client = udp_client(&waves);
    let (first_fragments, _) = fragment(SIGNATURE_BLOCK_1, &[1; 20], 10, 1, 0);
    let second = fragment(SIGNATURE_BLOCK_2, &[2; 20], 10, 2, 0);

    client.process_datagram(&first_fragments[0]).unwrap();
    deliver(&mut client, &second);
    // A straggler from the abandoned block is ignored.
    assert!(client.process_datagram(&first_fragments[1]).unwrap().is_empty());

    assert_eq!(waves.lock().unwrap().as_slice(), [(pcm(22050, 1), vec![2; 20])]);
}

// oracle: spec MS-RDPEA §3.2.5.2.1.1
#[test]
fn stale_udp_fragment_does_not_disturb_the_current_block() {
    let waves = Waves::default();
    let mut client = udp_client(&waves);
    let (first_fragments, _) = fragment(SIGNATURE_BLOCK_1, &[1; 20], 10, 1, 0);
    let (second_fragments, second_last) = fragment(SIGNATURE_BLOCK_2, &[2; 20], 10, 2, 0);

    client.process_datagram(&first_fragments[0]).unwrap();
    client.process_datagram(&second_fragments[0]).unwrap();
    // A delayed fragment of block 1 arrives in the middle of block 2.
    client.process_datagram(&first_fragments[1]).unwrap();
    client.process_datagram(&second_fragments[1]).unwrap();
    client.process_datagram(&second_last).unwrap();

    assert_eq!(waves.lock().unwrap().as_slice(), [(pcm(22050, 1), vec![2; 20])]);
}

// oracle: spec MS-RDPEA §3.2.5.2.1.1 (cBlockNo is an 8-bit counter)
#[test]
fn udp_block_numbers_wrap_around() {
    let waves = Waves::default();
    let mut client = udp_client(&waves);
    let (old_fragments, _) = fragment(SIGNATURE_BLOCK_200, &[3; 20], 10, 200, 0);
    let wrapped = fragment(SIGNATURE_BLOCK_2, &[2; 20], 10, 2, 0);

    client.process_datagram(&old_fragments[0]).unwrap();
    // Block 2 follows block 200 across the 8-bit wraparound.
    assert_eq!(deliver(&mut client, &wrapped).len(), 1);

    assert_eq!(waves.lock().unwrap().as_slice(), [(pcm(22050, 1), vec![2; 20])]);
}

// oracle: spec MS-RDPEA §2.2.3.7 (wTotalSize is 16 bits)
#[test]
fn oversized_udp_wave_is_dropped_without_completing() {
    let waves = Waves::default();
    let mut client = udp_client(&waves);
    let chunk = vec![0u8; 1400];

    // Forty-eight 1400-byte fragments exceed the 65,535 bytes wTotalSize can describe.
    for frag_no in 0..48 {
        client.process_datagram(&udp_wave(9, frag_no, &chunk)).unwrap();
    }
    // The sample was abandoned, so a final fragment cannot complete it.
    let last = encode_vec(&pdu::UdpWaveLastPdu {
        total_size: 0xFFFF,
        timestamp: 0,
        format_no: 0,
        block_no: 9,
        data: Cow::Borrowed(&chunk[..1]),
    })
    .unwrap();

    assert!(client.process_datagram(&last).unwrap().is_empty());
    assert!(waves.lock().unwrap().is_empty());
}

// oracle: spec MS-RDPEA §2.2.3.6 (UDP Wave PDUs only when both versions are at least 5)
#[test]
fn udp_wave_before_version_5_is_ignored() {
    let waves = Waves::default();
    let backend = TimedBackend {
        waves: Arc::clone(&waves),
        ..TimedBackend::new(vec![pcm(22050, 1)])
    };
    let mut client = Rdpsnd::new(Box::new(backend)).with_datagram_port(4321);
    let formats = encode_vec(&pdu::ServerAudioOutputPdu::AudioFormat(pdu::ServerAudioFormatPdu {
        version: pdu::Version::V2,
        formats: vec![pcm(22050, 1)],
    }))
    .unwrap();
    client.process(&formats).unwrap();
    client.process(&training()).unwrap();

    let replies = deliver(
        &mut client,
        &fragment(SIGNATURE_BLOCK_3, &(0..40).collect::<Vec<u8>>(), 12, 3, 0),
    );

    assert!(replies.is_empty());
    assert!(waves.lock().unwrap().is_empty());
}

// oracle: spec MS-RDPEA §3.1.5 (malformed packets are ignored)
#[test]
fn malformed_datagram_is_ignored() {
    let mut client = udp_client(&Waves::default());

    assert!(client.process_datagram(&[0x0B, 0x00]).unwrap().is_empty());
}
