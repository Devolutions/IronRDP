use ironrdp_core::Encode as _;
use ironrdp_rdpemt::pdu::*;
/// S_OK response for request ID 42.
const WIRE_SUCCESS: &[u8] = &[
    0x2A, 0x00, 0x00, 0x00, // RequestID = 42
    0x00, 0x00, 0x00, 0x00, // HrResponse = S_OK
];

/// E_ABORT response for request ID 7.
const WIRE_ABORT: &[u8] = &[
    0x07, 0x00, 0x00, 0x00, // RequestID = 7
    0x04, 0x40, 0x00, 0x80, // HrResponse = E_ABORT (0x80004004 LE)
];

#[test]
fn decode_success() {
    let pdu: MultitransportResponse = ironrdp_core::decode(WIRE_SUCCESS).expect("decode");
    assert_eq!(pdu.request_id, 42);
    assert_eq!(pdu.hr_response, MultitransportResponse::S_OK);
    assert!(pdu.is_success());
}

#[test]
fn decode_abort() {
    let pdu: MultitransportResponse = ironrdp_core::decode(WIRE_ABORT).expect("decode");
    assert_eq!(pdu.request_id, 7);
    assert_eq!(pdu.hr_response, MultitransportResponse::E_ABORT);
    assert!(!pdu.is_success());
}

#[test]
fn encode_success_matches_wire() {
    let pdu = MultitransportResponse::success(42);
    let encoded = ironrdp_core::encode_vec(&pdu).expect("encode");
    assert_eq!(encoded.as_slice(), WIRE_SUCCESS);
}

#[test]
fn encode_abort_matches_wire() {
    let pdu = MultitransportResponse::abort(7);
    let encoded = ironrdp_core::encode_vec(&pdu).expect("encode");
    assert_eq!(encoded.as_slice(), WIRE_ABORT);
}

#[test]
fn round_trip_success() {
    let original = MultitransportResponse::success(0xDEAD_BEEF);
    let encoded = ironrdp_core::encode_vec(&original).expect("encode");
    let decoded: MultitransportResponse = ironrdp_core::decode(&encoded).expect("decode");
    assert_eq!(decoded, original);
}

#[test]
fn round_trip_abort() {
    let original = MultitransportResponse::abort(123);
    let encoded = ironrdp_core::encode_vec(&original).expect("encode");
    let decoded: MultitransportResponse = ironrdp_core::decode(&encoded).expect("decode");
    assert_eq!(decoded, original);
}

#[test]
fn wire_size_is_8() {
    let pdu = MultitransportResponse::success(0);
    assert_eq!(pdu.size(), 8);
}

#[test]
fn constructor_convenience() {
    let s = MultitransportResponse::success(10);
    assert_eq!(s.request_id, 10);
    assert!(s.is_success());

    let a = MultitransportResponse::abort(20);
    assert_eq!(a.request_id, 20);
    assert!(!a.is_success());
}

#[test]
fn unknown_hr_response_round_trips() {
    let pdu = MultitransportResponse {
        request_id: 1,
        hr_response: 0x1234_5678,
    };
    let encoded = ironrdp_core::encode_vec(&pdu).expect("encode");
    let decoded: MultitransportResponse = ironrdp_core::decode(&encoded).expect("decode");
    assert_eq!(decoded, pdu);
    assert!(!decoded.is_success());
}
