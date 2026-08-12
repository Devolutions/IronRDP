use ironrdp_core::{Encode as _, decode, encode_vec};
use ironrdp_rdpeai::pdu::{
    DataPdu, FormatChangePdu, FormatsPdu, MAX_DATA_PACKET_SIZE, OpenPdu, OpenReplyPdu, RdpeaiPdu, Version, VersionPdu,
    pcm_format,
};

fn roundtrip(pdu: RdpeaiPdu) {
    let bytes = encode_vec(&pdu).expect("encode");
    let decoded: RdpeaiPdu = decode(&bytes).expect("decode");
    assert_eq!(decoded, pdu);
}

#[test]
fn version_roundtrip() {
    roundtrip(RdpeaiPdu::Version(VersionPdu::new(Version::V1)));
    roundtrip(RdpeaiPdu::Version(VersionPdu::new(Version::V2)));
}

#[test]
fn formats_client_sets_cb_size() {
    let fmt = pcm_format(1, 16000, 16);
    let pdu = FormatsPdu::client(vec![fmt]);
    assert_eq!(
        usize::try_from(pdu.cb_size_formats_packet).expect("size fits usize"),
        pdu.size()
    );
    roundtrip(RdpeaiPdu::Formats(pdu));
}

#[test]
fn open_packet_size_uses_ms_rdpeai_formula() {
    // nChannels * 2 * FramesPerPacket (mono, 480 frames) => 960
    let open = OpenPdu {
        frames_per_packet: 480,
        initial_format: 0,
        capture_format: pcm_format(1, 48000, 16),
    };
    assert_eq!(open.data_packet_size(), Some(960));

    // Stereo capture: channels still drive the Data PDU size.
    let stereo = OpenPdu {
        frames_per_packet: 480,
        initial_format: 0,
        capture_format: pcm_format(2, 48000, 16),
    };
    assert_eq!(stereo.data_packet_size(), Some(1920));
    roundtrip(RdpeaiPdu::Open(open));
}

#[test]
fn open_packet_size_rejects_oversized() {
    let open = OpenPdu {
        frames_per_packet: 1_000_000,
        initial_format: 0,
        capture_format: pcm_format(2, 48000, 16),
    };
    assert!(open.data_packet_size().is_none());
    const { assert!(MAX_DATA_PACKET_SIZE >= 192_000) };
}

#[test]
fn open_reply_data_formatchange_roundtrip() {
    roundtrip(RdpeaiPdu::OpenReply(OpenReplyPdu::ok()));
    roundtrip(RdpeaiPdu::OpenReply(OpenReplyPdu::fail()));
    roundtrip(RdpeaiPdu::DataIncoming);
    roundtrip(RdpeaiPdu::Data(DataPdu::new(vec![1, 2, 3, 4])));
    roundtrip(RdpeaiPdu::FormatChange(FormatChangePdu::new(2)));
}
