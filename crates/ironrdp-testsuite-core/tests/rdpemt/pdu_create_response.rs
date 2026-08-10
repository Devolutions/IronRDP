use ironrdp_core::Encode as _;
use ironrdp_rdpemt::pdu::*;
/// Protocol example from MS-RDPEMT Section 4.2.
const SPEC_EXAMPLE: &[u8] = &[
    0x01, 0x04, 0x00, 0x04, // Header: Action=1, PayloadLen=4, HeaderLen=4
    0x00, 0x00, 0x00, 0x00, // HrResponse = S_OK
];

#[test]
fn decode_spec_example() {
    let pdu: TunnelCreateResponse = ironrdp_core::decode(SPEC_EXAMPLE).expect("decode");
    assert_eq!(pdu.hr_response, TunnelCreateResponse::S_OK);
    assert!(pdu.is_success());
}

#[test]
fn encode_matches_spec_example() {
    let pdu = TunnelCreateResponse {
        hr_response: TunnelCreateResponse::S_OK,
    };

    let encoded = ironrdp_core::encode_vec(&pdu).expect("encode");
    assert_eq!(encoded.as_slice(), SPEC_EXAMPLE);
}

#[test]
fn round_trip_success() {
    let original = TunnelCreateResponse { hr_response: 0 };
    let encoded = ironrdp_core::encode_vec(&original).expect("encode");
    let decoded: TunnelCreateResponse = ironrdp_core::decode(&encoded).expect("decode");
    assert_eq!(decoded, original);
    assert!(decoded.is_success());
}

#[test]
fn round_trip_failure() {
    let original = TunnelCreateResponse {
        hr_response: 0x8000_0001,
    };
    let encoded = ironrdp_core::encode_vec(&original).expect("encode");
    let decoded: TunnelCreateResponse = ironrdp_core::decode(&encoded).expect("decode");
    assert_eq!(decoded, original);
    assert!(!decoded.is_success());
}

#[test]
fn wire_size_is_8() {
    let pdu = TunnelCreateResponse { hr_response: 0 };
    assert_eq!(pdu.size(), 8);
}
