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

/// MS-RDPEMT 3.1.5.5 / 3.3.5.1 require a successful HRESULT, not
/// specifically S_OK. S_FALSE (0x00000001) has the severity bit clear, so
/// it is a success code even though it isn't S_OK.
#[test]
fn is_success_matches_severity_bit_not_exact_s_ok() {
    let s_false = TunnelCreateResponse {
        hr_response: 0x0000_0001,
    };
    assert!(
        s_false.is_success(),
        "S_FALSE has the severity bit clear and is a success HRESULT"
    );

    let failure_with_clear_low_bits = TunnelCreateResponse {
        hr_response: 0x8000_0000,
    };
    assert!(!failure_with_clear_low_bits.is_success());
}

/// PayloadLength is fixed at 4 for this PDU (MS-RDPEMT 2.2.2.2). A wire
/// claiming a different value, followed by the real 4-byte body, must be
/// rejected rather than silently accepted: ironrdp_core::decode does not
/// require the cursor to be fully consumed, so a wrong length here would
/// otherwise never surface.
#[test]
fn decode_rejects_payload_length_that_disagrees_with_the_body() {
    let mut wire = SPEC_EXAMPLE.to_vec();
    wire[1] = 0xFF; // PayloadLength low byte: 4 -> 255
    let result: ironrdp_core::DecodeResult<TunnelCreateResponse> = ironrdp_core::decode(&wire);
    assert!(result.is_err());
}

#[test]
fn wire_size_is_8() {
    let pdu = TunnelCreateResponse { hr_response: 0 };
    assert_eq!(pdu.size(), 8);
}
