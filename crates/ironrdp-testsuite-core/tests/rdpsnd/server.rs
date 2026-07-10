//! Server-side tests for `ironrdp-rdpsnd`.
//!
//! Two layers:
//! - the crate-private `negotiate_formats` / `audio_format_eq` helpers, exposed
//!   to this testsuite via the rdpsnd crate's private `__test` feature (the lib
//!   itself has no inline test harness — `test = false`);
//! - the `SvcProcessor` negotiation wiring, driven black-box through the public
//!   surface (no `__test` shim needed).

use std::sync::{Arc, Mutex};

use ironrdp_core::encode_vec;
use ironrdp_rdpsnd::pdu::{
    AudioFormat, AudioFormatFlags, ClientAudioFormatPdu, ClientAudioOutputPdu, TrainingConfirmPdu, Version, WaveFormat,
};
use ironrdp_rdpsnd::server::{
    NegotiatedFormat, RdpsndError, RdpsndServer, RdpsndServerHandler, audio_format_eq, negotiate_formats,
};
use ironrdp_svc::SvcProcessor as _;

fn fmt(format: WaveFormat, rate: u32) -> AudioFormat {
    AudioFormat {
        format,
        n_channels: 2,
        n_samples_per_sec: rate,
        n_avg_bytes_per_sec: rate * 4,
        n_block_align: 4,
        bits_per_sample: 16,
        data: None,
    }
}

// ============================================================================
// `negotiate_formats` / `audio_format_eq` helpers (via the `__test` feature)
// ============================================================================

#[test]
fn wformat_no_addresses_the_client_list_not_the_server_list() {
    // Server prefers AAC over PCM; the client lists them in the opposite
    // order. wFormatNo must follow the CLIENT's indices.
    let server = [fmt(WaveFormat::AAC_MS, 44100), fmt(WaveFormat::PCM, 44100)];
    let client = [fmt(WaveFormat::PCM, 44100), fmt(WaveFormat::AAC_MS, 44100)];

    let common = negotiate_formats(&server, &client);

    // Ordering follows the server's preference (AAC first)...
    assert_eq!(common.len(), 2);
    assert_eq!(common[0].format().format, WaveFormat::AAC_MS);
    assert_eq!(common[1].format().format, WaveFormat::PCM);
    // ...but each wFormatNo is the position in the CLIENT list.
    assert_eq!(common[0].wformat_no(), 1); // AAC is client index 1
    assert_eq!(common[1].wformat_no(), 0); // PCM is client index 0
}

#[test]
fn pcm_only_client_gets_a_valid_client_index() {
    // Regression for the --enable-aac trap: server advertises [AAC, PCM]
    // but a PCM-only client must get wFormatNo 0 (its sole index), not
    // PCM's server-list index of 1 (which the client would reject).
    let server = [fmt(WaveFormat::AAC_MS, 44100), fmt(WaveFormat::PCM, 44100)];
    let client = [fmt(WaveFormat::PCM, 44100)];

    let common = negotiate_formats(&server, &client);

    assert_eq!(common.len(), 1);
    assert_eq!(common[0].format().format, WaveFormat::PCM);
    assert_eq!(common[0].wformat_no(), 0);
}

#[test]
fn no_shared_format_yields_empty() {
    let server = [fmt(WaveFormat::OPUS, 48000)];
    let client = [fmt(WaveFormat::PCM, 44100)];
    assert!(negotiate_formats(&server, &client).is_empty());
}

#[test]
fn equality_ignores_derived_fields_but_not_extra_data() {
    let mut a = fmt(WaveFormat::PCM, 44100);
    let mut b = fmt(WaveFormat::PCM, 44100);

    // The two derived fields are computable and a client need not echo them —
    // differing there is still the same format.
    b.n_avg_bytes_per_sec = 0;
    b.n_block_align = 99;
    assert!(audio_format_eq(&a, &b));

    // The codec extra-data blob IS significant (e.g. AAC config): a differing
    // `data` is a different format, even with identical WAVEFORMATEX fields.
    a.data = Some(vec![1, 2, 3]);
    b.data = None;
    assert!(!audio_format_eq(&a, &b));

    // A differing identity field (sample rate) is a different format.
    let c = fmt(WaveFormat::PCM, 48000);
    assert!(!audio_format_eq(&a, &c));
}

#[test]
fn extra_data_must_match_for_otherwise_identical_formats() {
    // Two AAC formats identical in every WAVEFORMATEX field but carrying
    // different HEAACWAVEINFO extra data are genuinely incompatible and must
    // not be treated as a match (the MS-RDPEA 2.2.2.1.1 `data` case).
    let mut server = fmt(WaveFormat::AAC_MS, 44100);
    server.data = Some(vec![0x11, 0x90]);
    let mut client = fmt(WaveFormat::AAC_MS, 44100);
    client.data = Some(vec![0x12, 0x08]);

    assert!(negotiate_formats(&[server], &[client]).is_empty());
}

// ============================================================================
// `SvcProcessor` negotiation wiring (black-box, public surface only)
// ============================================================================

#[derive(Debug, Default)]
struct Recording {
    choose_format_calls: usize,
    start_calls: usize,
    chosen_wformat: Option<u16>,
}

#[derive(Debug)]
struct FakeHandler {
    formats: Vec<AudioFormat>,
    rec: Arc<Mutex<Recording>>,
    start_ok: bool,
}

impl RdpsndServerHandler for FakeHandler {
    fn get_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn choose_format<'a>(&mut self, common: &'a [NegotiatedFormat]) -> Option<&'a NegotiatedFormat> {
        let mut rec = self.rec.lock().expect("poisoned");
        rec.choose_format_calls += 1;
        let chosen = common.first();
        rec.chosen_wformat = chosen.map(NegotiatedFormat::wformat_no);
        chosen
    }

    fn start(&mut self, _format: &NegotiatedFormat) -> Result<(), Box<dyn RdpsndError>> {
        self.rec.lock().expect("poisoned").start_calls += 1;
        if self.start_ok {
            Ok(())
        } else {
            Err(Box::new(std::io::Error::other("simulated init failure")))
        }
    }

    fn stop(&mut self) {}
}

/// Drive a fresh server through the handshake (server announce → client formats
/// → training confirm) so the negotiation (`choose_format` + `start`) runs.
/// Client version is V5 (< V6) to skip the optional Quality Mode step.
fn drive_to_ready(server: &mut RdpsndServer, client_formats: Vec<AudioFormat>) {
    server.start().expect("server announce");

    let client_af = ClientAudioOutputPdu::AudioFormat(ClientAudioFormatPdu {
        version: Version::V5,
        flags: AudioFormatFlags::empty(),
        formats: client_formats,
        volume_left: 0,
        volume_right: 0,
        pitch: 0,
        dgram_port: 0,
    });
    server
        .process(&encode_vec(&client_af).expect("encode client formats"))
        .expect("process client formats");

    let confirm = ClientAudioOutputPdu::TrainingConfirm(TrainingConfirmPdu {
        timestamp: 0,
        pack_size: 0,
    });
    server
        .process(&encode_vec(&confirm).expect("encode training confirm"))
        .expect("process training confirm");
}

#[test]
fn processor_skips_choose_format_when_nothing_in_common() {
    let rec = Arc::new(Mutex::new(Recording::default()));
    let mut server = RdpsndServer::new(Box::new(FakeHandler {
        formats: vec![fmt(WaveFormat::PCM, 44100)],
        rec: Arc::clone(&rec),
        start_ok: true,
    }));

    // Server offers only PCM; client offers only AAC → no common format.
    drive_to_ready(&mut server, vec![fmt(WaveFormat::AAC_MS, 44100)]);

    {
        let rec = rec.lock().expect("poisoned");
        assert_eq!(
            rec.choose_format_calls, 0,
            "choose_format must be skipped when common is empty"
        );
        assert_eq!(rec.start_calls, 0);
    }
    // Nothing negotiated → no format committed.
    assert!(server.wave(vec![0; 4], 0).is_err());
}

#[test]
fn processor_calls_start_once_and_streams_on_success() {
    let rec = Arc::new(Mutex::new(Recording::default()));
    let mut server = RdpsndServer::new(Box::new(FakeHandler {
        formats: vec![fmt(WaveFormat::PCM, 44100)],
        rec: Arc::clone(&rec),
        start_ok: true,
    }));

    drive_to_ready(&mut server, vec![fmt(WaveFormat::PCM, 44100)]);

    {
        let rec = rec.lock().expect("poisoned");
        assert_eq!(rec.choose_format_calls, 1);
        assert_eq!(rec.start_calls, 1, "start must be called exactly once");
        assert_eq!(rec.chosen_wformat, Some(0)); // PCM is the client's only entry
    }
    // Format committed → waves stream.
    assert!(server.wave(vec![0; 4], 0).is_ok());
}

#[test]
fn processor_declines_when_start_fails() {
    let rec = Arc::new(Mutex::new(Recording::default()));
    let mut server = RdpsndServer::new(Box::new(FakeHandler {
        formats: vec![fmt(WaveFormat::PCM, 44100)],
        rec: Arc::clone(&rec),
        start_ok: false, // simulate an encoder/init failure
    }));

    drive_to_ready(&mut server, vec![fmt(WaveFormat::PCM, 44100)]);

    assert_eq!(rec.lock().expect("poisoned").start_calls, 1);
    // `start` returned Err → the crate rolls `format_no` back to None and
    // declines, so no audio is streamed — rather than a silent
    // "negotiated, no audio" state with a committed format and no producer.
    assert!(server.wave(vec![0; 4], 0).is_err());
}
