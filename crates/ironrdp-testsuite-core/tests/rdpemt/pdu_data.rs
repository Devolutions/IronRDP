use ironrdp_core::{Encode as _, encode_vec};
use ironrdp_rdpemt::pdu::*;
#[test]
fn round_trip_simple_data() {
    let original = TunnelData {
        sub_headers: Vec::new(),
        higher_layer_data: vec![0x48, 0x65, 0x6C, 0x6C, 0x6F], // "Hello"
    };

    let encoded = encode_vec(&original).expect("encode");
    // Header: Action=2, PayloadLen=5, HeaderLen=4, then 5 data bytes
    assert_eq!(encoded, [0x02, 0x05, 0x00, 0x04, 0x48, 0x65, 0x6C, 0x6C, 0x6F]);

    let decoded: TunnelData = ironrdp_core::decode(&encoded).expect("decode");
    assert_eq!(decoded, original);
}

#[test]
fn round_trip_data_with_subheader() {
    let original = TunnelData {
        sub_headers: vec![TunnelSubHeader {
            sub_header_type: SubHeaderType::AutoDetectRequest,
            data: vec![0xFF],
        }],
        higher_layer_data: vec![0x01, 0x02],
    };

    let encoded = encode_vec(&original).expect("encode");
    // Header: Action=2, PayloadLen=2, HeaderLen=7 (4 + subheader(3))
    // SubHeader: len=3, type=0x00, data=0xFF
    // Payload: 0x01, 0x02
    assert_eq!(encoded, [0x02, 0x02, 0x00, 0x07, 0x03, 0x00, 0xFF, 0x01, 0x02]);

    let decoded: TunnelData = ironrdp_core::decode(&encoded).expect("decode");
    assert_eq!(decoded, original);
}

#[test]
fn round_trip_empty_data() {
    let original = TunnelData {
        sub_headers: Vec::new(),
        higher_layer_data: Vec::new(),
    };

    let encoded = encode_vec(&original).expect("encode");
    assert_eq!(encoded, [0x02, 0x00, 0x00, 0x04]);

    let decoded: TunnelData = ironrdp_core::decode(&encoded).expect("decode");
    assert_eq!(decoded, original);
}

#[test]
fn size_calculation() {
    let pdu = TunnelData {
        sub_headers: Vec::new(),
        higher_layer_data: vec![0; 100],
    };
    // 4 (header) + 100 (data) = 104
    assert_eq!(pdu.size(), 104);
}

/// Both length fields in a Data PDU are narrower than the values they describe,
/// so an oversized one must be refused rather than truncated.
///
/// A wrapped `HeaderLength` or `PayloadLength` produces a header that disagrees
/// with the bytes behind it, which a peer then reads as a differently-framed
/// PDU. Found by the spec-emt review pass.
#[test]
fn tunnel_data_refuses_lengths_that_do_not_fit_their_fields() {
    let oversized_payload = TunnelData {
        sub_headers: Vec::new(),
        higher_layer_data: vec![0; usize::from(u16::MAX) + 1],
    };
    encode_vec(&oversized_payload).expect_err("a payload above 65535 bytes has no representable length");

    // Each sub-header is at least two bytes, so 128 of them pass the one-byte
    // HeaderLength field.
    let oversized_header = TunnelData {
        sub_headers: core::iter::repeat_with(|| TunnelSubHeader {
            sub_header_type: SubHeaderType::AutoDetectRequest,
            data: vec![0; 6],
        })
        .take(128)
        .collect(),
        higher_layer_data: Vec::new(),
    };
    encode_vec(&oversized_header).expect_err("sub-headers beyond one byte of length are not representable");
}
