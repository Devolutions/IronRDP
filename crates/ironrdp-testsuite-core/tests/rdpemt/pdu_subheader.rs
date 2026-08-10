use ironrdp_core::DecodeResult;
use ironrdp_rdpemt::pdu::*;
#[test]
fn round_trip_empty_subheader() {
    let sub = TunnelSubHeader {
        sub_header_type: SubHeaderType::AutoDetectRequest,
        data: Vec::new(),
    };

    let encoded = ironrdp_core::encode_vec(&sub).expect("encode");
    assert_eq!(encoded, [0x02, 0x00]); // length=2, type=0x00

    let decoded: TunnelSubHeader = ironrdp_core::decode(&encoded).expect("decode");
    assert_eq!(decoded, sub);
}

#[test]
fn round_trip_subheader_with_data() {
    let sub = TunnelSubHeader {
        sub_header_type: SubHeaderType::AutoDetectResponse,
        data: vec![0x01, 0x02, 0x03],
    };

    let encoded = ironrdp_core::encode_vec(&sub).expect("encode");
    // length = 2 + 3 = 5, type = 0x01
    assert_eq!(encoded, [0x05, 0x01, 0x01, 0x02, 0x03]);

    let decoded: TunnelSubHeader = ironrdp_core::decode(&encoded).expect("decode");
    assert_eq!(decoded, sub);
}

#[test]
fn decode_rejects_too_small_length() {
    let wire = [0x01, 0x00]; // length=1, but minimum is 2
    let result: DecodeResult<TunnelSubHeader> = ironrdp_core::decode(&wire);
    assert!(result.is_err());
}

#[test]
fn decode_rejects_unknown_type() {
    let wire = [0x02, 0xFF]; // unknown type
    let result: DecodeResult<TunnelSubHeader> = ironrdp_core::decode(&wire);
    assert!(result.is_err());
}
