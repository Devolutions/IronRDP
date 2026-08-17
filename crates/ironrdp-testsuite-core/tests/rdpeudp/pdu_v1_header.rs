use ironrdp_core::{DecodeResult, Encode as _, decode, encode_vec};
use ironrdp_rdpeudp::pdu::*;
/// SYN datagram header per [MS-RDPEUDP] 3.1.5.1.1.
///
/// All multi-byte fields are in network byte order, which 2.2 requires for
/// every message this protocol puts on the wire. That is the opposite of
/// MS-RDPEUDP2, whose 2.2 mandates little-endian.
const SYN_HEADER_BYTES: [u8; 8] = [
    0xFF, 0xFF, 0xFF, 0xFF, // snSourceAck = 0xFFFFFFFF
    0x00, 0x40, // uReceiveWindowSize = 64
    0x10, 0x01, // uFlags = SYN(0x0001) | SYNEX(0x1000) = 0x1001
];

fn syn_header() -> FecHeader {
    FecHeader {
        sn_source_ack: 0xFFFF_FFFF,
        receive_window_size: 64,
        flags: V1Flags::SYN | V1Flags::SYNEX,
    }
}

#[test]
fn encode_syn_header() {
    let encoded = encode_vec(&syn_header()).expect("encode should succeed");
    assert_eq!(encoded.as_slice(), &SYN_HEADER_BYTES);
}

#[test]
fn decode_syn_header() {
    let decoded: FecHeader = decode(&SYN_HEADER_BYTES).expect("decode should succeed");
    assert_eq!(decoded, syn_header());
}

#[test]
fn roundtrip() {
    let original = FecHeader {
        sn_source_ack: 0x0000_1234,
        receive_window_size: 128,
        flags: V1Flags::ACK | V1Flags::CN | V1Flags::ACK_OF_ACKS,
    };
    let encoded = encode_vec(&original).expect("encode");
    let decoded: FecHeader = decode(&encoded).expect("decode");
    assert_eq!(original, decoded);
}

#[test]
fn size_matches_encoding() {
    let header = syn_header();
    let encoded = encode_vec(&header).expect("encode");
    assert_eq!(header.size(), encoded.len());
}

#[test]
fn decode_insufficient_bytes() {
    let short = [0xFF, 0xFF, 0xFF]; // only 3 bytes, need 8
    let result: DecodeResult<FecHeader> = decode(&short);
    assert!(result.is_err());
}

/// The SYN capture from [MS-RDPEUDP] 4.1.1, which is the authority for the
/// byte order.
#[test]
fn decode_the_spec_syn_header_capture() {
    // ff ff ff ff 04 00 0A 01, documented as uReceiveWindowSize 0x0400 = 1024
    // and uFlags 0x0A01 = CORRELATION_ID | SYNLOSSY | SYN.
    const CAPTURE: [u8; 8] = [0xFF, 0xFF, 0xFF, 0xFF, 0x04, 0x00, 0x0A, 0x01];

    let header: FecHeader = decode(&CAPTURE).expect("decode");
    assert_eq!(header.sn_source_ack, 0xFFFF_FFFF);
    assert_eq!(header.receive_window_size, 1024);
    assert_eq!(header.flags, V1Flags::SYN | V1Flags::SYNLOSSY | V1Flags::CORRELATION_ID);

    assert_eq!(encode_vec(&header).expect("encode"), CAPTURE);
}

/// The SYN and ACK capture from [MS-RDPEUDP] 4.1.2.
#[test]
fn decode_the_spec_syn_ack_header_capture() {
    // 00 00 00 42 04 00 00 05, documented as uFlags 0x0005 = SYN | ACK.
    const CAPTURE: [u8; 8] = [0x00, 0x00, 0x00, 0x42, 0x04, 0x00, 0x00, 0x05];

    let header: FecHeader = decode(&CAPTURE).expect("decode");
    assert_eq!(header.sn_source_ack, 0x42);
    assert_eq!(header.receive_window_size, 1024);
    assert_eq!(header.flags, V1Flags::SYN | V1Flags::ACK);

    assert_eq!(encode_vec(&header).expect("encode"), CAPTURE);
}
