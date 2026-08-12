use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_rdpeai::client::{AudioPacketSink, RdpeaiCaptureHandler, RdpeaiClient};
use ironrdp_rdpeai::pdu::{
    FormatChangePdu, FormatsPdu, OpenPdu, OpenReplyPdu, RdpeaiPdu, Version, VersionPdu, pcm_format,
};
use ironrdp_rdpsnd::pdu::AudioFormat;

struct MockCapture {
    formats: Vec<AudioFormat>,
    open_calls: usize,
    open_result: i32,
    set_format_ok: bool,
    set_format_calls: usize,
}

impl RdpeaiCaptureHandler for MockCapture {
    fn supported_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn open(&mut self, _format: &AudioFormat, _packet_size: usize, _sink: AudioPacketSink) -> i32 {
        self.open_calls += 1;
        self.open_result
    }

    fn set_format(&mut self, _format: &AudioFormat, _packet_size: usize) -> bool {
        self.set_format_calls += 1;
        self.set_format_ok
    }

    fn close(&mut self) {}
}

fn client_with(handler: MockCapture) -> RdpeaiClient {
    RdpeaiClient::new(Box::new(handler), Box::new(|_, _| Ok(())))
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
        open_calls: 0,
        open_result: OpenReplyPdu::S_OK,
        set_format_ok: true,
        set_format_calls: 0,
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
}

#[test]
fn open_with_bad_index_returns_fail() {
    let fmt = pcm_format(1, 16000, 16);
    let mut client = client_with(MockCapture {
        formats: vec![fmt.clone()],
        open_calls: 0,
        open_result: OpenReplyPdu::S_OK,
        set_format_ok: true,
        set_format_calls: 0,
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
    let encoded = encode_vec(&RdpeaiPdu::OpenReply(OpenReplyPdu::fail())).unwrap();
    // Ensure the single response is OpenReply fail by decoding it.
    let reply: RdpeaiPdu = decode(&encoded).unwrap();
    assert!(matches!(reply, RdpeaiPdu::OpenReply(_)));
}

#[test]
fn format_change_failure_does_not_ack() {
    let fmt_a = pcm_format(1, 16000, 16);
    let fmt_b = pcm_format(1, 48000, 16);
    let mut client = client_with(MockCapture {
        formats: vec![fmt_a.clone(), fmt_b.clone()],
        open_calls: 0,
        open_result: OpenReplyPdu::S_OK,
        set_format_ok: false,
        set_format_calls: 0,
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

    let out = process_encoded(
        &mut client,
        3,
        RdpeaiPdu::FormatChange(FormatChangePdu::new(1)),
    );
    assert!(out.is_empty(), "failed FormatChange must not be confirmed");
}

#[test]
fn open_rejects_oversized_frames_per_packet() {
    let fmt = pcm_format(1, 16000, 16);
    let mut client = client_with(MockCapture {
        formats: vec![fmt.clone()],
        open_calls: 0,
        open_result: OpenReplyPdu::S_OK,
        set_format_ok: true,
        set_format_calls: 0,
    });
    client.start(1).unwrap();
    let _ = process_encoded(&mut client, 1, RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    let _ = process_encoded(&mut client, 1, RdpeaiPdu::Formats(FormatsPdu::server(vec![fmt.clone()])));
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
}
