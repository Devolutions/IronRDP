use ironrdp_core::Encode as _;
use ironrdp_rdpemt::pdu::*;
/// Protocol example from MS-RDPEMT Section 4.1.
const SPEC_EXAMPLE: &[u8] = &[
    0x00, 0x18, 0x00, 0x04, // Header: Action=0, PayloadLen=24, HeaderLen=4
    0x07, 0x00, 0x00, 0x00, // RequestID = 7
    0x00, 0x00, 0x00, 0x00, // Reserved = 0
    0xe2, 0xf0, 0xd1, 0x08, 0x56, 0x7f, 0xb4, 0x3a, // SecurityCookie (16 bytes)
    0xdc, 0xf4, 0xb3, 0xdc, 0x16, 0x92, 0x1e, 0x3a,
];

#[test]
fn decode_spec_example() {
    let pdu: TunnelCreateRequest = ironrdp_core::decode(SPEC_EXAMPLE).expect("decode");
    assert_eq!(pdu.request_id, 7);
    assert_eq!(
        pdu.security_cookie,
        [
            0xe2, 0xf0, 0xd1, 0x08, 0x56, 0x7f, 0xb4, 0x3a, 0xdc, 0xf4, 0xb3, 0xdc, 0x16, 0x92, 0x1e, 0x3a
        ]
    );
}

#[test]
fn encode_matches_spec_example() {
    let pdu = TunnelCreateRequest {
        request_id: 7,
        security_cookie: [
            0xe2, 0xf0, 0xd1, 0x08, 0x56, 0x7f, 0xb4, 0x3a, 0xdc, 0xf4, 0xb3, 0xdc, 0x16, 0x92, 0x1e, 0x3a,
        ],
    };

    let encoded = ironrdp_core::encode_vec(&pdu).expect("encode");
    assert_eq!(encoded.as_slice(), SPEC_EXAMPLE);
}

#[test]
fn round_trip() {
    let original = TunnelCreateRequest {
        request_id: 0xDEAD_BEEF,
        security_cookie: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
    };

    let encoded = ironrdp_core::encode_vec(&original).expect("encode");
    let decoded: TunnelCreateRequest = ironrdp_core::decode(&encoded).expect("decode");
    assert_eq!(decoded, original);
}

#[test]
fn wire_size_is_28() {
    let pdu = TunnelCreateRequest {
        request_id: 0,
        security_cookie: [0; 16],
    };
    assert_eq!(pdu.size(), 28);
}
