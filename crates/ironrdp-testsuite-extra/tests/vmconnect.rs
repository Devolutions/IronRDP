use core::time::Duration;

use ironrdp::pdu::pcb::{PcbVersion, PreconnectionBlob};
use ironrdp_core::decode;
use ironrdp_tokio::TokioFramed;
use ironrdp_vmconnect::{
    Mode, PCB_TRANSMIT_DEADLINE, encode_preconnection_blob, encode_preconnection_blob_payload,
    preconnection_blob_payload, send_preconnection_blob,
};
use tokio::io::AsyncReadExt as _;

const VM_ID: &str = "efd1efab-c750-4262-b1bb-af0f7733bdd6";
const ENHANCED_PCB: &[u8] = &[
    0x7A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x00, 0x65,
    0x00, 0x66, 0x00, 0x64, 0x00, 0x31, 0x00, 0x65, 0x00, 0x66, 0x00, 0x61, 0x00, 0x62, 0x00, 0x2D, 0x00, 0x63, 0x00,
    0x37, 0x00, 0x35, 0x00, 0x30, 0x00, 0x2D, 0x00, 0x34, 0x00, 0x32, 0x00, 0x36, 0x00, 0x32, 0x00, 0x2D, 0x00, 0x62,
    0x00, 0x31, 0x00, 0x62, 0x00, 0x62, 0x00, 0x2D, 0x00, 0x61, 0x00, 0x66, 0x00, 0x30, 0x00, 0x66, 0x00, 0x37, 0x00,
    0x37, 0x00, 0x33, 0x00, 0x33, 0x00, 0x62, 0x00, 0x64, 0x00, 0x64, 0x00, 0x36, 0x00, 0x3B, 0x00, 0x45, 0x00, 0x6E,
    0x00, 0x68, 0x00, 0x61, 0x00, 0x6E, 0x00, 0x63, 0x00, 0x65, 0x00, 0x64, 0x00, 0x4D, 0x00, 0x6F, 0x00, 0x64, 0x00,
    0x65, 0x00, 0x3D, 0x00, 0x31, 0x00, 0x00, 0x00,
];

#[test]
fn pcb_v2_serialization_selects_console_mode() {
    for (mode, expected) in [
        (Mode::Enhanced, format!("{VM_ID};EnhancedMode=1")),
        (Mode::Basic, VM_ID.to_owned()),
    ] {
        let bytes = encode_preconnection_blob(VM_ID, mode).expect("encode");
        let pcb: PreconnectionBlob = decode(&bytes).expect("decode");

        assert_eq!(pcb.version, PcbVersion::V2);
        assert_eq!(pcb.v2_payload.as_deref(), Some(expected.as_str()));
    }
}

#[test]
fn enhanced_payload_and_wire_size_match_vmconnect() {
    let payload = preconnection_blob_payload(VM_ID, Mode::Enhanced).expect("payload");
    let bytes = encode_preconnection_blob(VM_ID, Mode::Enhanced).expect("encode");

    assert_eq!(payload, format!("{VM_ID};EnhancedMode=1"));
    assert_eq!(bytes, ENHANCED_PCB);
}

#[test]
fn pcb_v2_supports_long_unicode_payloads() {
    let payload = format!("{};名字=虚拟机💻", "a".repeat(64));
    let bytes = encode_preconnection_blob_payload(payload.clone()).expect("encode");
    let pcb: PreconnectionBlob = decode(&bytes).expect("decode");

    assert_eq!(pcb.v2_payload.as_deref(), Some(payload.as_str()));
    assert_eq!(
        usize::from(u16::from_le_bytes(bytes[16..18].try_into().unwrap())),
        payload.encode_utf16().count() + 1
    );
}

#[test]
fn vmconnect_rejects_empty_vm_id() {
    assert!(preconnection_blob_payload("", Mode::Enhanced).is_err());
    assert!(preconnection_blob_payload("   ", Mode::Basic).is_err());
    assert!(encode_preconnection_blob("", Mode::Enhanced).is_err());
}

#[tokio::test]
async fn send_preconnection_blob_writes_encoded_pcb() {
    let expected = encode_preconnection_blob(VM_ID, Mode::Enhanced).expect("encode");
    let (client, mut server) = tokio::io::duplex(expected.len());
    let mut framed = TokioFramed::new(client);

    let _pcb_sent = send_preconnection_blob(&mut framed, VM_ID, Mode::Enhanced)
        .await
        .expect("send PCB");

    let mut actual = vec![0; expected.len()];
    server.read_exact(&mut actual).await.expect("read PCB");
    assert_eq!(actual, expected);
}

#[test]
fn pcb_transmit_deadline_matches_ms_rdpeps() {
    assert_eq!(PCB_TRANSMIT_DEADLINE, Duration::from_secs(10));
}
