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

/// `total` (2 + data.len()) above 255 must not silently wrap into the
/// one-byte SubHeaderLength field: the length byte would then disagree
/// with the data bytes that actually follow it on the wire.
#[test]
fn encode_rejects_data_that_overflows_sub_header_length() {
    let sub = TunnelSubHeader {
        sub_header_type: SubHeaderType::AutoDetectRequest,
        data: vec![0u8; 254], // 2 + 254 = 256, one past u8::MAX
    };

    let result = ironrdp_core::encode_vec(&sub);
    assert!(result.is_err());
}

#[test]
fn encode_accepts_data_at_the_sub_header_length_boundary() {
    let sub = TunnelSubHeader {
        sub_header_type: SubHeaderType::AutoDetectRequest,
        data: vec![0u8; 253], // 2 + 253 = 255, the u8::MAX boundary
    };

    let encoded = ironrdp_core::encode_vec(&sub).expect("255 is the boundary, not an overflow");
    assert_eq!(encoded[0], 255);
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

/// An auto-detect sub-header is the auto-detect structure itself: Its
/// SubHeaderLength and SubHeaderType are the structure's headerLength and
/// headerTypeId, so SubHeaderData starts at the sequence number
/// (MS-RDPEMT 2.2.1.1.1, MS-RDPBCGR 2.2.14.1.2).
#[test]
fn autodetect_subheader_is_the_autodetect_structure() {
    use ironrdp_pdu::rdp::autodetect::AutoDetectRequest;

    // RDP_BW_START: headerLength 6, TYPE_ID_AUTODETECT_REQUEST,
    // sequenceNumber 7, requestType 0x0014.
    let wire = [0x06, 0x00, 0x07, 0x00, 0x14, 0x00];

    let sub: TunnelSubHeader = ironrdp_core::decode(&wire).expect("decode as a sub-header");
    assert_eq!(sub.sub_header_type, SubHeaderType::AutoDetectRequest);
    assert_eq!(sub.data, [0x07, 0x00, 0x14, 0x00]);

    let request: AutoDetectRequest = ironrdp_core::decode(&wire).expect("decode as an auto-detect request");
    assert_eq!(request, AutoDetectRequest::bw_start_continuous(7));
    assert_eq!(ironrdp_core::encode_vec(&request).expect("encode"), wire);
}
