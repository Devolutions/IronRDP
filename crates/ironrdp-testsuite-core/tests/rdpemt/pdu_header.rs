use ironrdp_core::DecodeResult;
use ironrdp_rdpemt::pdu::*;
#[test]
fn round_trip_minimal_header() {
    let header = TunnelHeader {
        action: TunnelAction::CreateRequest,
        payload_length: 24,
        header_length: 4,
        sub_headers: Vec::new(),
    };

    let encoded = ironrdp_core::encode_vec(&header).expect("encode");
    assert_eq!(encoded, [0x00, 0x18, 0x00, 0x04]);

    let decoded: TunnelHeader = ironrdp_core::decode(&encoded).expect("decode");
    assert_eq!(decoded, header);
}

#[test]
fn round_trip_create_response_header() {
    let header = TunnelHeader {
        action: TunnelAction::CreateResponse,
        payload_length: 4,
        header_length: 4,
        sub_headers: Vec::new(),
    };

    let encoded = ironrdp_core::encode_vec(&header).expect("encode");
    assert_eq!(encoded, [0x01, 0x04, 0x00, 0x04]);

    let decoded: TunnelHeader = ironrdp_core::decode(&encoded).expect("decode");
    assert_eq!(decoded, header);
}

#[test]
fn round_trip_data_header() {
    let header = TunnelHeader {
        action: TunnelAction::Data,
        payload_length: 100,
        header_length: 4,
        sub_headers: Vec::new(),
    };

    let encoded = ironrdp_core::encode_vec(&header).expect("encode");
    assert_eq!(encoded, [0x02, 0x64, 0x00, 0x04]);

    let decoded: TunnelHeader = ironrdp_core::decode(&encoded).expect("decode");
    assert_eq!(decoded, header);
}

#[test]
fn decode_rejects_nonzero_flags() {
    // Byte 0 = 0x10 means Action=0, Flags=1
    let wire = [0x10, 0x18, 0x00, 0x04];
    let result: DecodeResult<TunnelHeader> = ironrdp_core::decode(&wire);
    assert!(result.is_err());
}

#[test]
fn decode_rejects_unknown_action() {
    // Action = 0x0F
    let wire = [0x0F, 0x04, 0x00, 0x04];
    let result: DecodeResult<TunnelHeader> = ironrdp_core::decode(&wire);
    assert!(result.is_err());
}

#[test]
fn decode_rejects_header_too_small() {
    // HeaderLength = 3 (below minimum of 4)
    let wire = [0x00, 0x18, 0x00, 0x03];
    let result: DecodeResult<TunnelHeader> = ironrdp_core::decode(&wire);
    assert!(result.is_err());
}

#[test]
fn decode_rejects_truncated_input() {
    let wire = [0x00, 0x18]; // only 2 bytes
    let result: DecodeResult<TunnelHeader> = ironrdp_core::decode(&wire);
    assert!(result.is_err());
}

#[test]
fn round_trip_header_with_subheader() {
    let header = TunnelHeader {
        action: TunnelAction::Data,
        payload_length: 50,
        // 4 (base) + 4 (one subheader: len=4, type=1, data=[0xAA, 0xBB])
        header_length: 8,
        sub_headers: vec![TunnelSubHeader {
            sub_header_type: SubHeaderType::AutoDetectResponse,
            data: vec![0xAA, 0xBB],
        }],
    };

    let encoded = ironrdp_core::encode_vec(&header).expect("encode");
    // byte 0: 0x02 (Data), payload_length: 50=0x0032, header_length: 8
    // subheader: length=4, type=0x01, data=0xAA 0xBB
    assert_eq!(encoded, [0x02, 0x32, 0x00, 0x08, 0x04, 0x01, 0xAA, 0xBB]);

    let decoded: TunnelHeader = ironrdp_core::decode(&encoded).expect("decode");
    assert_eq!(decoded, header);
}
