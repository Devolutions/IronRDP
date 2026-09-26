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

#[cfg(windows)]
#[test]
fn native_credssp_binding_hash_matches_v5_v6_vector() {
    const CLIENT_SERVER_HASH_MAGIC: &[u8] = b"CredSSP Client-To-Server Binding Hash\0";
    const SERVER_CLIENT_HASH_MAGIC: &[u8] = b"CredSSP Server-To-Client Binding Hash\0";

    // MS-CSSP 3.1.5: SHA256(magic || nonce || SubjectPublicKey), including the magic string's NUL.
    let nonce = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11,
        0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F,
    ];
    let public_key = [1, 2, 3, 4];
    let expected = [
        0xB4, 0x4E, 0x3A, 0x42, 0x23, 0x29, 0x60, 0xBC, 0x17, 0x4B, 0x37, 0x7F, 0x77, 0xAE, 0xBF, 0xE4, 0xD6, 0x4B,
        0xF7, 0x25, 0x41, 0x82, 0x62, 0xD8, 0x32, 0x84, 0x6B, 0xE4, 0x83, 0x29, 0x1F, 0x7B,
    ];
    assert_eq!(
        ironrdp_vmconnect::__test_binding_hash(CLIENT_SERVER_HASH_MAGIC, &nonce, &public_key).as_slice(),
        expected
    );

    let mut changed_nonce = nonce;
    changed_nonce[0] ^= 0x80;
    assert_ne!(
        ironrdp_vmconnect::__test_binding_hash(CLIENT_SERVER_HASH_MAGIC, &changed_nonce, &public_key).as_slice(),
        expected
    );

    let mut changed_public_key = public_key;
    changed_public_key[0] ^= 0x80;
    assert_ne!(
        ironrdp_vmconnect::__test_binding_hash(CLIENT_SERVER_HASH_MAGIC, &nonce, &changed_public_key).as_slice(),
        expected
    );

    assert_ne!(
        ironrdp_vmconnect::__test_binding_hash(SERVER_CLIENT_HASH_MAGIC, &nonce, &public_key).as_slice(),
        expected
    );
}
