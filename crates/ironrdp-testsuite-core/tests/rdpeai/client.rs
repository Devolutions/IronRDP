use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_rdpeai::client::{AudioPacketSink, RdpeaiCaptureHandler, RdpeaiClient};
use ironrdp_rdpeai::pdu::{
    FormatChangePdu, FormatsPdu, OpenPdu, OpenReplyPdu, RdpeaiPdu, Version, VersionPdu, pcm_format,
};
use ironrdp_rdpsnd::pdu::AudioFormat;

struct MockCapture {
    formats: Vec<AudioFormat>,
    open_calls: Arc<AtomicUsize>,
    open_result: i32,
    set_format_ok: bool,
    set_format_calls: Arc<AtomicUsize>,
}

impl RdpeaiCaptureHandler for MockCapture {
    fn supported_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn open(
        &mut self,
        _capture_format: &AudioFormat,
        _encode_format: &AudioFormat,
        _packet_size: usize,
        _sink: AudioPacketSink,
    ) -> i32 {
        self.open_calls.fetch_add(1, Ordering::Relaxed);
        self.open_result
    }

    fn set_format(&mut self, _encode_format: &AudioFormat, _packet_size: usize) -> bool {
        self.set_format_calls.fetch_add(1, Ordering::Relaxed);
        self.set_format_ok
    }

    fn close(&mut self) {}
}

fn decode_dvc(msg: &ironrdp_dvc::DvcMessage) -> RdpeaiPdu {
    let bytes = encode_vec(msg.as_ref()).expect("encode dvc message");
    decode(&bytes).expect("decode dvc message")
}

fn client_with(handler: MockCapture) -> RdpeaiClient {
    RdpeaiClient::new(Box::new(handler), Box::new(|_, _| Ok(())))
}

type Uplinked = Arc<Mutex<Vec<ironrdp_svc::SvcMessage>>>;

/// Builds a client whose uplink records the messages it carries.
fn client_with_uplink(handler: impl RdpeaiCaptureHandler + 'static) -> (RdpeaiClient, Uplinked) {
    let uplinked = Uplinked::default();
    let recorder = Arc::clone(&uplinked);
    let client = RdpeaiClient::new(
        Box::new(handler),
        Box::new(move |_, messages| {
            recorder.lock().unwrap().extend(messages);
            Ok(())
        }),
    );
    (client, uplinked)
}

/// Decodes the AUDIO_INPUT PDU carried by an uplinked DRDYNVC data message.
fn decode_uplinked(message: &ironrdp_svc::SvcMessage) -> RdpeaiPdu {
    use ironrdp_core::{Decode as _, ReadCursor};
    use ironrdp_dvc::pdu::{DrdynvcClientPdu, DrdynvcDataPdu};

    let encoded = message.encode_unframed_pdu().expect("encode uplinked message");
    let DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(data)) =
        DrdynvcClientPdu::decode(&mut ReadCursor::new(&encoded)).expect("decode DRDYNVC")
    else {
        panic!("expected DVC data");
    };
    decode(data.data()).expect("decode AUDIO_INPUT PDU")
}

fn process_encoded(client: &mut RdpeaiClient, channel_id: u32, pdu: RdpeaiPdu) -> Vec<ironrdp_dvc::DvcMessage> {
    client
        .process(channel_id, &encode_vec(&pdu).expect("encode"))
        .expect("process")
}

#[test]
fn version_formats_open_happy_path() {
    let fmt = pcm_format(1, 16000, 16);
    let mut client = client_with(MockCapture {
        formats: vec![fmt.clone(), pcm_format(2, 48000, 16)],
        open_calls: Arc::new(AtomicUsize::new(0)),
        open_result: OpenReplyPdu::S_OK,
        set_format_ok: true,
        set_format_calls: Arc::new(AtomicUsize::new(0)),
    });

    client.start(7).expect("start");
    let version_out = process_encoded(&mut client, 7, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    assert_eq!(version_out.len(), 1);

    let formats_out = process_encoded(
        &mut client,
        7,
        RdpeaiPdu::Formats(FormatsPdu::server(vec![fmt.clone()])),
    );
    // Incoming + client Formats
    assert_eq!(formats_out.len(), 2);

    let open_out = process_encoded(
        &mut client,
        7,
        RdpeaiPdu::Open(OpenPdu {
            frames_per_packet: 320,
            initial_format: 0,
            capture_format: fmt,
        }),
    );
    // FormatChange + OpenReply
    assert_eq!(open_out.len(), 2);
    assert!(matches!(decode_dvc(&open_out[0]), RdpeaiPdu::FormatChange(_)));
    match decode_dvc(&open_out[1]) {
        RdpeaiPdu::OpenReply(reply) => assert_eq!(reply.result, OpenReplyPdu::S_OK),
        other => panic!("expected OpenReply ok, got {other:?}"),
    }
}

#[test]
fn open_with_bad_index_returns_fail() {
    let fmt = pcm_format(1, 16000, 16);
    let mut client = client_with(MockCapture {
        formats: vec![fmt.clone()],
        open_calls: Arc::new(AtomicUsize::new(0)),
        open_result: OpenReplyPdu::S_OK,
        set_format_ok: true,
        set_format_calls: Arc::new(AtomicUsize::new(0)),
    });
    client.start(1).unwrap();
    let _ = process_encoded(&mut client, 1, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    let _ = process_encoded(&mut client, 1, RdpeaiPdu::Formats(FormatsPdu::server(vec![fmt])));
    let out = process_encoded(
        &mut client,
        1,
        RdpeaiPdu::Open(OpenPdu {
            frames_per_packet: 100,
            initial_format: 5,
            capture_format: pcm_format(1, 16000, 16),
        }),
    );
    assert_eq!(out.len(), 1);
    match decode_dvc(&out[0]) {
        RdpeaiPdu::OpenReply(reply) => assert_eq!(reply.result, OpenReplyPdu::E_FAIL),
        other => panic!("expected OpenReply fail, got {other:?}"),
    }
}

// oracle: spec MS-RDPEAI §3.2.5.3.1
#[test]
fn format_change_to_other_index_is_confirmed() {
    let fmt_a = pcm_format(1, 16000, 16);
    let fmt_b = pcm_format(1, 48000, 16);
    let set_format_calls = Arc::new(AtomicUsize::new(0));
    let (mut client, uplinked) = client_with_uplink(MockCapture {
        formats: vec![fmt_a.clone(), fmt_b.clone()],
        open_calls: Arc::new(AtomicUsize::new(0)),
        open_result: OpenReplyPdu::S_OK,
        set_format_ok: true,
        set_format_calls: Arc::clone(&set_format_calls),
    });

    client.start(3).unwrap();
    let _ = process_encoded(&mut client, 3, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    let _ = process_encoded(
        &mut client,
        3,
        RdpeaiPdu::Formats(FormatsPdu::server(vec![fmt_a.clone(), fmt_b])),
    );
    let _ = process_encoded(
        &mut client,
        3,
        RdpeaiPdu::Open(OpenPdu {
            frames_per_packet: 320,
            initial_format: 0,
            capture_format: fmt_a,
        }),
    );

    let out = process_encoded(&mut client, 3, RdpeaiPdu::FormatChange(FormatChangePdu::new(1)));

    // The confirmation goes through the uplink, behind the capture packets already queued there.
    assert!(out.is_empty());
    assert_eq!(set_format_calls.load(Ordering::Relaxed), 1);
    let uplinked = uplinked.lock().unwrap();
    let [confirmation] = uplinked.as_slice() else {
        panic!("expected one uplinked message, got {}", uplinked.len());
    };
    match decode_uplinked(confirmation) {
        RdpeaiPdu::FormatChange(change) => assert_eq!(change.new_format, 1),
        other => panic!("expected FormatChange confirm, got {other:?}"),
    }
}

#[test]
fn format_change_failure_still_acks() {
    let fmt_a = pcm_format(1, 16000, 16);
    let fmt_b = pcm_format(1, 48000, 16);
    let mut client = client_with(MockCapture {
        formats: vec![fmt_a.clone(), fmt_b.clone()],
        open_calls: Arc::new(AtomicUsize::new(0)),
        open_result: OpenReplyPdu::S_OK,
        set_format_ok: false,
        set_format_calls: Arc::new(AtomicUsize::new(0)),
    });

    client.start(3).unwrap();
    let _ = process_encoded(&mut client, 3, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    let _ = process_encoded(
        &mut client,
        3,
        RdpeaiPdu::Formats(FormatsPdu::server(vec![fmt_a.clone(), fmt_b])),
    );
    let _ = process_encoded(
        &mut client,
        3,
        RdpeaiPdu::Open(OpenPdu {
            frames_per_packet: 320,
            initial_format: 0,
            capture_format: fmt_a,
        }),
    );

    // MS-RDPEAI always confirms a valid FormatChange so the server never hangs.
    let out = process_encoded(&mut client, 3, RdpeaiPdu::FormatChange(FormatChangePdu::new(1)));
    assert_eq!(out.len(), 1);
    match decode_dvc(&out[0]) {
        RdpeaiPdu::FormatChange(change) => assert_eq!(change.new_format, 1),
        other => panic!("expected FormatChange confirm, got {other:?}"),
    }
}

#[test]
fn open_rejects_oversized_frames_per_packet() {
    let fmt = pcm_format(1, 16000, 16);
    let mut client = client_with(MockCapture {
        formats: vec![fmt.clone()],
        open_calls: Arc::new(AtomicUsize::new(0)),
        open_result: OpenReplyPdu::S_OK,
        set_format_ok: true,
        set_format_calls: Arc::new(AtomicUsize::new(0)),
    });
    client.start(1).unwrap();
    let _ = process_encoded(&mut client, 1, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    let _ = process_encoded(
        &mut client,
        1,
        RdpeaiPdu::Formats(FormatsPdu::server(vec![fmt.clone()])),
    );
    let out = process_encoded(
        &mut client,
        1,
        RdpeaiPdu::Open(OpenPdu {
            frames_per_packet: u32::MAX,
            initial_format: 0,
            capture_format: fmt,
        }),
    );
    assert_eq!(out.len(), 1);
    match decode_dvc(&out[0]) {
        RdpeaiPdu::OpenReply(reply) => assert_eq!(reply.result, OpenReplyPdu::E_FAIL),
        other => panic!("expected OpenReply fail, got {other:?}"),
    }
}

// oracle: spec MS-RDPEAI §3.2.5.3.1
#[test]
fn out_of_sequence_version_is_ignored() {
    let fmt = pcm_format(1, 16000, 16);
    let open_calls = Arc::new(AtomicUsize::new(0));
    let set_format_calls = Arc::new(AtomicUsize::new(0));
    let (mut client, uplinked) = client_with_uplink(MockCapture {
        formats: vec![fmt.clone()],
        open_calls: Arc::clone(&open_calls),
        open_result: OpenReplyPdu::S_OK,
        set_format_ok: true,
        set_format_calls: Arc::clone(&set_format_calls),
    });
    client.start(1).unwrap();
    let _ = process_encoded(&mut client, 1, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    let _ = process_encoded(
        &mut client,
        1,
        RdpeaiPdu::Formats(FormatsPdu::server(vec![fmt.clone()])),
    );
    let _ = process_encoded(
        &mut client,
        1,
        RdpeaiPdu::Open(OpenPdu {
            frames_per_packet: 320,
            initial_format: 0,
            capture_format: fmt,
        }),
    );
    assert_eq!(open_calls.load(Ordering::Relaxed), 1);

    let out = process_encoded(&mut client, 1, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    assert!(out.is_empty(), "opened client must ignore duplicate Version");

    // Still opened: FormatChange should reach the backend rather than idle-ack only.
    let change_out = process_encoded(&mut client, 1, RdpeaiPdu::FormatChange(FormatChangePdu::new(0)));
    assert!(change_out.is_empty());
    assert_eq!(set_format_calls.load(Ordering::Relaxed), 1);
    let uplinked = uplinked.lock().unwrap();
    assert!(matches!(decode_uplinked(&uplinked[0]), RdpeaiPdu::FormatChange(_)));
}

/// Capture backend that keeps the packet sink so a test can queue capture data.
struct SinkKeepingCapture {
    formats: Vec<AudioFormat>,
    sink: Arc<Mutex<Option<AudioPacketSink>>>,
}

impl RdpeaiCaptureHandler for SinkKeepingCapture {
    fn supported_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn open(
        &mut self,
        _capture_format: &AudioFormat,
        _encode_format: &AudioFormat,
        _packet_size: usize,
        sink: AudioPacketSink,
    ) -> i32 {
        *self.sink.lock().unwrap() = Some(sink);
        OpenReplyPdu::S_OK
    }

    fn set_format(&mut self, _encode_format: &AudioFormat, _packet_size: usize) -> bool {
        true
    }

    fn close(&mut self) {}
}

// oracle: spec MS-RDPEAI §3.2.5.3.1 (all audio data after the confirmation uses the new format)
#[test]
fn format_change_confirmation_follows_already_queued_data() {
    let fmt_a = pcm_format(1, 16000, 16);
    let fmt_b = pcm_format(1, 48000, 16);
    let sink = Arc::new(Mutex::new(None));
    let (mut client, uplinked) = client_with_uplink(SinkKeepingCapture {
        formats: vec![fmt_a.clone(), fmt_b.clone()],
        sink: Arc::clone(&sink),
    });
    client.start(3).unwrap();
    let _ = process_encoded(&mut client, 3, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    let _ = process_encoded(
        &mut client,
        3,
        RdpeaiPdu::Formats(FormatsPdu::server(vec![fmt_a.clone(), fmt_b])),
    );
    let _ = process_encoded(
        &mut client,
        3,
        RdpeaiPdu::Open(OpenPdu {
            frames_per_packet: 2,
            initial_format: 0,
            capture_format: fmt_a,
        }),
    );

    // A packet captured in the old format is queued before the server asks for the new one.
    (sink.lock().unwrap().as_mut().unwrap())(vec![1, 2, 3, 4]);
    let replies = process_encoded(&mut client, 3, RdpeaiPdu::FormatChange(FormatChangePdu::new(1)));

    assert!(replies.is_empty());
    let uplinked: Vec<RdpeaiPdu> = uplinked.lock().unwrap().iter().map(decode_uplinked).collect();
    assert!(
        matches!(
            uplinked.as_slice(),
            [RdpeaiPdu::DataIncoming, RdpeaiPdu::Data(_), RdpeaiPdu::FormatChange(change)] if change.new_format == 1
        ),
        "the confirmation must follow the queued data, got {uplinked:?}"
    );
}

// oracle: spec MS-RDPEAI §3.2.5.1.5 (the client replies with formats it supports among those the server sent)
#[test]
fn negotiated_formats_keep_client_order_and_only_offered_formats() {
    let mono_16k = pcm_format(1, 16000, 16);
    let mono_48k = pcm_format(1, 48000, 16);
    let stereo_44k = pcm_format(2, 44100, 16);
    let mut client = client_with(MockCapture {
        formats: vec![mono_48k.clone(), stereo_44k, mono_16k.clone()],
        open_calls: Arc::new(AtomicUsize::new(0)),
        open_result: OpenReplyPdu::S_OK,
        set_format_ok: true,
        set_format_calls: Arc::new(AtomicUsize::new(0)),
    });
    client.start(3).unwrap();
    let _ = process_encoded(&mut client, 3, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));

    let replies = process_encoded(
        &mut client,
        3,
        RdpeaiPdu::Formats(FormatsPdu::server(vec![
            mono_16k.clone(),
            pcm_format(2, 8000, 16),
            mono_48k.clone(),
        ])),
    );

    let RdpeaiPdu::Formats(reply) = decode_dvc(&replies[1]) else {
        panic!("expected the client Sound Formats PDU");
    };
    // Client preference order, limited to what the server offered.
    assert_eq!(reply.formats, vec![mono_48k, mono_16k]);
}
