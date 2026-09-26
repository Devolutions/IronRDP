use std::sync::{Arc, Mutex};

use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_rdpeai::pdu::{
    DataPdu, FormatChangePdu, FormatsPdu, OpenPdu, OpenReplyPdu, RdpeaiPdu, Version, VersionPdu, pcm_format,
};
use ironrdp_rdpeai::server::{NoopRdpeaiServerBackend, RdpeaiServer, RdpeaiServerBackend};
use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};

#[derive(Default)]
struct MockBackendState {
    negotiated: Vec<AudioFormat>,
    open_replies: Vec<i32>,
    audio: Vec<(AudioFormat, Vec<u8>)>,
    format_changes: Vec<AudioFormat>,
    closed: bool,
}

#[derive(Clone, Default)]
struct MockBackend {
    formats: Vec<AudioFormat>,
    state: Arc<Mutex<MockBackendState>>,
}

impl MockBackend {
    fn new(formats: Vec<AudioFormat>) -> Self {
        Self {
            formats,
            state: Arc::new(Mutex::new(MockBackendState::default())),
        }
    }
}

impl RdpeaiServerBackend for MockBackend {
    fn supported_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn on_formats_negotiated(&mut self, negotiated: &[AudioFormat]) {
        self.state.lock().unwrap().negotiated = negotiated.to_vec();
    }

    fn on_open_reply(&mut self, result: i32) {
        self.state.lock().unwrap().open_replies.push(result);
    }

    fn on_audio_data(&mut self, format: &AudioFormat, data: &[u8]) {
        self.state.lock().unwrap().audio.push((format.clone(), data.to_vec()));
    }

    fn on_format_change_confirmed(&mut self, format: &AudioFormat) {
        self.state.lock().unwrap().format_changes.push(format.clone());
    }

    fn on_close(&mut self) {
        self.state.lock().unwrap().closed = true;
    }
}

fn decode_dvc(msg: &ironrdp_dvc::DvcMessage) -> RdpeaiPdu {
    let bytes = encode_vec(msg.as_ref()).expect("encode dvc message");
    decode(&bytes).expect("decode dvc message")
}

fn process_encoded(server: &mut RdpeaiServer, channel_id: u32, pdu: RdpeaiPdu) -> Vec<ironrdp_dvc::DvcMessage> {
    server
        .process(channel_id, &encode_vec(&pdu).expect("encode"))
        .expect("process")
}

/// Drive the server through Version -> Formats -> Open -> FormatChange confirm -> OpenReply.
/// Every call site uses the same channel id, frames-per-packet, and initial format index, so
/// those are fixed here rather than threaded through as parameters.
fn negotiate_and_open(
    server: &mut RdpeaiServer,
    client_formats: Vec<AudioFormat>,
    capture_format: AudioFormat,
    open_result: i32,
) {
    let start_out = server.start(1).expect("start");
    assert_eq!(start_out.len(), 1);
    assert!(matches!(decode_dvc(&start_out[0]), RdpeaiPdu::Version(_)));

    let version_out = process_encoded(server, 1, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    assert_eq!(version_out.len(), 1);
    assert!(matches!(decode_dvc(&version_out[0]), RdpeaiPdu::Formats(_)));

    let formats_out = process_encoded(server, 1, RdpeaiPdu::Formats(FormatsPdu::client(client_formats)));
    assert!(formats_out.is_empty());

    let open_out = server.open(320, 0, capture_format).expect("open");
    assert_eq!(open_out.len(), 1);
    assert!(matches!(decode_dvc(&open_out[0]), RdpeaiPdu::Open(_)));

    let confirm_out = process_encoded(server, 1, RdpeaiPdu::FormatChange(FormatChangePdu::new(0)));
    assert!(confirm_out.is_empty());

    let reply_out = process_encoded(server, 1, RdpeaiPdu::OpenReply(OpenReplyPdu { result: open_result }));
    assert!(reply_out.is_empty());
}

#[test]
fn version_and_formats_negotiate_correctly() {
    let fmt_a = pcm_format(1, 16000, 16);
    let fmt_b = pcm_format(2, 48000, 16);
    let backend = MockBackend::new(vec![fmt_a.clone(), fmt_b.clone()]);
    let mut server = RdpeaiServer::new(Box::new(backend.clone()));

    let start_out = server.start(1).expect("start");
    match decode_dvc(&start_out[0]) {
        RdpeaiPdu::Version(v) => assert_eq!(v.version, Version::V2),
        other => panic!("expected server Version PDU, got {other:?}"),
    }

    let version_out = process_encoded(&mut server, 1, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    match decode_dvc(&version_out[0]) {
        RdpeaiPdu::Formats(f) => assert_eq!(f.formats, vec![fmt_a.clone(), fmt_b]),
        other => panic!("expected server Formats PDU, got {other:?}"),
    }

    // Client accepts only fmt_a.
    let _ = process_encoded(
        &mut server,
        1,
        RdpeaiPdu::Formats(FormatsPdu::client(vec![fmt_a.clone()])),
    );
    assert_eq!(server.negotiated_formats(), &[fmt_a]);
    assert_eq!(backend.state.lock().unwrap().negotiated, server.negotiated_formats());
    assert_eq!(server.client_version(), Some(Version::V1));
}

#[test]
fn formats_reply_is_filtered_to_the_servers_offered_set() {
    // MS-RDPEAI 3.2.5.1.5: the client's list MUST be a subset of the server's; a
    // non-conformant client claiming a format the server never offered must not poison
    // the negotiated list.
    let fmt_a = pcm_format(1, 16000, 16);
    let unoffered = pcm_format(2, 96000, 24);
    let backend = MockBackend::new(vec![fmt_a.clone()]);
    let mut server = RdpeaiServer::new(Box::new(backend));

    server.start(1).unwrap();
    let _ = process_encoded(&mut server, 1, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    let _ = process_encoded(
        &mut server,
        1,
        RdpeaiPdu::Formats(FormatsPdu::client(vec![fmt_a.clone(), unoffered])),
    );

    assert_eq!(server.negotiated_formats(), &[fmt_a]);
}

#[test]
fn open_through_data_delivers_audio_to_the_backend() {
    let fmt = pcm_format(1, 16000, 16);
    let backend = MockBackend::new(vec![fmt.clone()]);
    let mut server = RdpeaiServer::new(Box::new(backend.clone()));

    negotiate_and_open(&mut server, vec![fmt.clone()], fmt.clone(), OpenReplyPdu::S_OK);

    assert_eq!(backend.state.lock().unwrap().open_replies, vec![OpenReplyPdu::S_OK]);
    assert_eq!(server.current_format(), Some(&fmt));

    let data_out = process_encoded(&mut server, 1, RdpeaiPdu::Data(DataPdu::new(vec![1, 2, 3, 4])));
    assert!(data_out.is_empty());

    let audio = backend.state.lock().unwrap().audio.clone();
    assert_eq!(audio, vec![(fmt, vec![1, 2, 3, 4])]);
}

#[test]
fn data_before_open_is_ignored() {
    let fmt = pcm_format(1, 16000, 16);
    let backend = MockBackend::new(vec![fmt.clone()]);
    let mut server = RdpeaiServer::new(Box::new(backend.clone()));

    server.start(1).unwrap();
    let _ = process_encoded(&mut server, 1, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    let _ = process_encoded(&mut server, 1, RdpeaiPdu::Formats(FormatsPdu::client(vec![fmt])));

    // Still Ready: no Open in flight yet.
    let _ = process_encoded(&mut server, 1, RdpeaiPdu::Data(DataPdu::new(vec![9, 9, 9])));
    assert!(backend.state.lock().unwrap().audio.is_empty());
}

#[test]
fn open_rejects_out_of_range_initial_format() {
    let fmt = pcm_format(1, 16000, 16);
    let backend = MockBackend::new(vec![fmt.clone()]);
    let mut server = RdpeaiServer::new(Box::new(backend));

    server.start(1).unwrap();
    let _ = process_encoded(&mut server, 1, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    let _ = process_encoded(
        &mut server,
        1,
        RdpeaiPdu::Formats(FormatsPdu::client(vec![fmt.clone()])),
    );

    assert!(server.open(320, 7, fmt).is_err());
}

#[test]
fn open_rejects_when_not_yet_negotiated() {
    let backend = NoopRdpeaiServerBackend;
    let mut server = RdpeaiServer::new(Box::new(backend));
    server.start(1).unwrap();
    // Version/Formats round trip has not happened: state is not Ready.
    assert!(server.open(320, 0, pcm_format(1, 16000, 16)).is_err());
}

#[test]
fn open_reply_failure_returns_to_ready_and_allows_retry() {
    let fmt = pcm_format(1, 16000, 16);
    let backend = MockBackend::new(vec![fmt.clone()]);
    let mut server = RdpeaiServer::new(Box::new(backend.clone()));

    negotiate_and_open(&mut server, vec![fmt.clone()], fmt.clone(), OpenReplyPdu::E_FAIL);

    assert_eq!(backend.state.lock().unwrap().open_replies, vec![OpenReplyPdu::E_FAIL]);
    assert_eq!(server.current_format(), None);

    // MS-RDPEAI 3.3.5.1.8: on failure the server MAY retry Open; the state machine must
    // allow it rather than being stuck.
    assert!(server.open(320, 0, fmt).is_ok());
}

#[test]
fn open_reply_failure_before_format_change_confirm_returns_to_ready() {
    // The client rejects Open (e.g. initialFormat out of range, or FramesPerPacket rejected)
    // by replying with OpenReply failure directly, without first confirming the initial
    // format via FormatChange (ironrdp-rdpeai's own client does exactly this in
    // handle_open). The server must accept that reply from AwaitingFormatConfirm, not just
    // AwaitingOpenReply, or the channel is stuck forever with no path back to Ready.
    let fmt = pcm_format(1, 16000, 16);
    let backend = MockBackend::new(vec![fmt.clone()]);
    let mut server = RdpeaiServer::new(Box::new(backend.clone()));

    server.start(1).expect("start");
    process_encoded(&mut server, 1, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    process_encoded(
        &mut server,
        1,
        RdpeaiPdu::Formats(FormatsPdu::client(vec![fmt.clone()])),
    );
    server.open(320, 0, fmt.clone()).expect("open");

    // Skip the FormatChange confirm entirely; reply as if Open was rejected.
    let reply_out = process_encoded(
        &mut server,
        1,
        RdpeaiPdu::OpenReply(OpenReplyPdu {
            result: OpenReplyPdu::E_FAIL,
        }),
    );
    assert!(reply_out.is_empty());

    assert_eq!(backend.state.lock().unwrap().open_replies, vec![OpenReplyPdu::E_FAIL]);
    assert_eq!(server.current_format(), None);
    assert!(server.open(320, 0, fmt).is_ok());
}

#[test]
fn open_reply_with_a_non_s_ok_non_negative_result_is_treated_as_success() {
    // MS-RDPEAI 3.3.5.1.8: an HRESULT is an error only when its sign bit is set, so any
    // non-negative result is success, not just S_OK (0). S_FALSE (1) is one such code.
    const S_FALSE: i32 = 1;
    let fmt = pcm_format(1, 16000, 16);
    let backend = MockBackend::new(vec![fmt.clone()]);
    let mut server = RdpeaiServer::new(Box::new(backend.clone()));

    negotiate_and_open(&mut server, vec![fmt.clone()], fmt.clone(), S_FALSE);

    assert_eq!(backend.state.lock().unwrap().open_replies, vec![S_FALSE]);
    assert_eq!(server.current_format(), Some(&fmt));

    let data_out = process_encoded(&mut server, 1, RdpeaiPdu::Data(DataPdu::new(vec![5, 6, 7])));
    assert!(data_out.is_empty());
    assert_eq!(backend.state.lock().unwrap().audio, vec![(fmt, vec![5, 6, 7])]);
}

#[test]
fn change_format_round_trips_and_notifies_backend() {
    let fmt_a = pcm_format(1, 16000, 16);
    let fmt_b = pcm_format(1, 48000, 16);
    let backend = MockBackend::new(vec![fmt_a.clone(), fmt_b.clone()]);
    let mut server = RdpeaiServer::new(Box::new(backend.clone()));

    negotiate_and_open(
        &mut server,
        vec![fmt_a.clone(), fmt_b.clone()],
        fmt_a,
        OpenReplyPdu::S_OK,
    );

    let change_out = server.change_format(1).expect("change_format");
    assert_eq!(change_out.len(), 1);
    assert!(matches!(decode_dvc(&change_out[0]), RdpeaiPdu::FormatChange(_)));

    let confirm_out = process_encoded(&mut server, 1, RdpeaiPdu::FormatChange(FormatChangePdu::new(1)));
    assert!(confirm_out.is_empty());

    assert_eq!(server.current_format(), Some(&fmt_b));
    assert_eq!(backend.state.lock().unwrap().format_changes, vec![fmt_b]);
}

#[test]
fn change_format_confirm_ignores_divergent_echo() {
    // MS-RDPEAI 3.1.5: a confirm that echoes a different index than the server requested is
    // non-conformant and MUST be ignored, not trusted, keeping the server-requested format.
    let fmt_a = pcm_format(1, 16000, 16);
    let fmt_b = pcm_format(1, 48000, 16);
    let backend = MockBackend::new(vec![fmt_a.clone(), fmt_b.clone()]);
    let mut server = RdpeaiServer::new(Box::new(backend.clone()));

    negotiate_and_open(
        &mut server,
        vec![fmt_a.clone(), fmt_b.clone()],
        fmt_a,
        OpenReplyPdu::S_OK,
    );

    let change_out = server.change_format(1).expect("change_format");
    assert_eq!(change_out.len(), 1);

    // Client echoes index 0 instead of the requested index 1.
    let confirm_out = process_encoded(&mut server, 1, RdpeaiPdu::FormatChange(FormatChangePdu::new(0)));
    assert!(confirm_out.is_empty());

    assert_eq!(server.current_format(), Some(&fmt_b));
    assert_eq!(backend.state.lock().unwrap().format_changes, vec![fmt_b]);
}

#[test]
fn change_format_rejects_out_of_range() {
    let fmt = pcm_format(1, 16000, 16);
    let backend = MockBackend::new(vec![fmt.clone()]);
    let mut server = RdpeaiServer::new(Box::new(backend));

    negotiate_and_open(&mut server, vec![fmt.clone()], fmt, OpenReplyPdu::S_OK);

    assert!(server.change_format(5).is_err());
}

#[test]
fn change_format_rejects_before_open() {
    let fmt = pcm_format(1, 16000, 16);
    let backend = MockBackend::new(vec![fmt.clone()]);
    let mut server = RdpeaiServer::new(Box::new(backend));

    server.start(1).unwrap();
    let _ = process_encoded(&mut server, 1, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    let _ = process_encoded(&mut server, 1, RdpeaiPdu::Formats(FormatsPdu::client(vec![fmt])));

    assert!(server.change_format(0).is_err());
}

#[test]
fn change_format_skips_when_active_format_is_aac_and_client_is_version_one() {
    // MS-RDPEAI 3.3.5.3.1: if the client advertised only version 1 and the active format is
    // AAC, the server SHOULD NOT send FormatChange.
    let aac = AudioFormat {
        format: WaveFormat::AAC_MS,
        n_channels: 2,
        n_samples_per_sec: 48000,
        n_avg_bytes_per_sec: 12000,
        n_block_align: 1,
        bits_per_sample: 0,
        data: None,
    };
    let pcm = pcm_format(2, 48000, 16);
    let backend = MockBackend::new(vec![aac.clone(), pcm.clone()]);
    let mut server = RdpeaiServer::new(Box::new(backend.clone()));

    // aac is index 0
    negotiate_and_open(&mut server, vec![aac.clone(), pcm], aac, OpenReplyPdu::S_OK);
    assert_eq!(server.client_version(), Some(Version::V1));

    let out = server
        .change_format(1)
        .expect("change_format should not error, only skip");
    assert!(
        out.is_empty(),
        "FormatChange must not be sent per 3.3.5.3.1's SHOULD NOT"
    );
    assert!(backend.state.lock().unwrap().format_changes.is_empty());
}

#[test]
fn out_of_sequence_open_reply_before_open_is_ignored() {
    let fmt = pcm_format(1, 16000, 16);
    let backend = MockBackend::new(vec![fmt.clone()]);
    let mut server = RdpeaiServer::new(Box::new(backend.clone()));

    server.start(1).unwrap();
    let _ = process_encoded(&mut server, 1, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    let _ = process_encoded(&mut server, 1, RdpeaiPdu::Formats(FormatsPdu::client(vec![fmt])));

    let out = process_encoded(&mut server, 1, RdpeaiPdu::OpenReply(OpenReplyPdu::ok()));
    assert!(out.is_empty());
    assert!(backend.state.lock().unwrap().open_replies.is_empty());
}

#[test]
fn client_originated_open_pdu_is_ignored() {
    // Open is server-to-client only; a client sending one is non-conformant and must not
    // be treated as a valid transition (MS-RDPEAI 3.1.5: unrecognized/out-of-sequence
    // packets MUST be ignored).
    let fmt = pcm_format(1, 16000, 16);
    let backend = MockBackend::new(vec![fmt.clone()]);
    let mut server = RdpeaiServer::new(Box::new(backend));

    server.start(1).unwrap();
    let out = process_encoded(
        &mut server,
        1,
        RdpeaiPdu::Open(OpenPdu {
            frames_per_packet: 320,
            initial_format: 0,
            capture_format: fmt,
        }),
    );
    assert!(out.is_empty());
}

#[test]
fn malformed_pdu_is_ignored_not_errored() {
    // MS-RDPEAI 3.1.5: malformed or unrecognized PDUs MUST be ignored. A decode failure
    // must not surface as an Err, since process() is reached through the shared DRDYNVC
    // SVC processor and an Err there would fail the whole dynamic-channel message loop.
    let fmt = pcm_format(1, 16000, 16);
    let backend = MockBackend::new(vec![fmt]);
    let mut server = RdpeaiServer::new(Box::new(backend));

    server.start(1).unwrap();
    let out = server
        .process(1, &[0xFF])
        .expect("malformed PDU must be ignored, not errored");
    assert!(out.is_empty());
}

#[test]
fn close_resets_state_and_notifies_backend() {
    let fmt = pcm_format(1, 16000, 16);
    let backend = MockBackend::new(vec![fmt.clone()]);
    let mut server = RdpeaiServer::new(Box::new(backend.clone()));

    negotiate_and_open(&mut server, vec![fmt.clone()], fmt, OpenReplyPdu::S_OK);
    assert!(server.current_format().is_some());

    server.close(1);

    assert!(backend.state.lock().unwrap().closed);
    assert!(server.current_format().is_none());
    assert!(server.negotiated_formats().is_empty());
    // Opened state was reset to Start, so Open must be rejected again until re-negotiated.
    assert!(server.open(320, 0, pcm_format(1, 16000, 16)).is_err());
}
