use ironrdp_core::{Decode as _, DecodeResult, Encode as _, ReadCursor, WriteCursor, decode, encode_vec};
use ironrdp_rdpeudp::pdu::*;
// ── DataHeader tests ──

#[test]
fn data_header_encode() {
    let header = DataHeader { data_seq_num: 0x1234 };
    let encoded = encode_vec(&header).expect("encode");
    assert_eq!(encoded.as_slice(), &[0x34, 0x12]); // little-endian
}

#[test]
fn data_header_decode() {
    let bytes = [0x34, 0x12];
    let decoded: DataHeader = decode(&bytes).expect("decode");
    assert_eq!(decoded.data_seq_num, 0x1234);
}

#[test]
fn data_header_roundtrip() {
    let original = DataHeader { data_seq_num: 0xFFFF };
    let encoded = encode_vec(&original).expect("encode");
    let decoded: DataHeader = decode(&encoded).expect("decode");
    assert_eq!(original, decoded);
}

#[test]
fn data_header_size() {
    let header = DataHeader { data_seq_num: 0 };
    assert_eq!(header.size(), 2);
}

#[test]
fn data_header_insufficient_bytes() {
    let bytes = [0x34]; // only 1 byte, need 2
    let result: DecodeResult<DataHeader> = decode(&bytes);
    assert!(result.is_err());
}

// ── DataBody tests ──

#[test]
fn data_body_with_payload() {
    let body = DataBody {
        channel_seq_num: 0x0042,
        data: vec![0x01, 0x02, 0x03, 0x04],
    };
    let encoded = encode_vec(&body).expect("encode");
    assert_eq!(
        encoded.as_slice(),
        &[
            0x42, 0x00, // channel_seq_num = 0x0042
            0x01, 0x02, 0x03, 0x04, // data
        ]
    );
    assert_eq!(body.size(), 6);
}

#[test]
fn data_body_empty_data() {
    let body = DataBody {
        channel_seq_num: 0,
        data: Vec::new(),
    };
    let encoded = encode_vec(&body).expect("encode");
    assert_eq!(encoded.as_slice(), &[0x00, 0x00]);
    assert_eq!(body.size(), 2);
}

#[test]
fn data_body_roundtrip() {
    let original = DataBody {
        channel_seq_num: 0xABCD,
        data: vec![0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE],
    };
    let encoded = encode_vec(&original).expect("encode");
    let decoded: DataBody = decode(&encoded).expect("decode");
    assert_eq!(original, decoded);
}

#[test]
fn data_body_large_payload() {
    // Simulate a near-MTU payload
    let original = DataBody {
        channel_seq_num: 1,
        data: vec![0xAA; 1200],
    };
    let encoded = encode_vec(&original).expect("encode");
    let decoded: DataBody = decode(&encoded).expect("decode");
    assert_eq!(original.data.len(), decoded.data.len());
    assert_eq!(original, decoded);
}

#[test]
fn data_body_insufficient_bytes() {
    let bytes = [0x42]; // only 1 byte, need 2 for ChannelSeqNum
    let result: DecodeResult<DataBody> = decode(&bytes);
    assert!(result.is_err());
}

// ── Combined DataHeader + DataBody test ──

#[test]
fn data_header_then_body_sequential() {
    // Simulate reading a DATA payload: DataHeader then DataBody
    let header = DataHeader { data_seq_num: 100 };
    let body = DataBody {
        channel_seq_num: 50,
        data: vec![0x11, 0x22, 0x33],
    };

    let mut buf = vec![0u8; header.size() + body.size()];
    let mut cursor = WriteCursor::new(&mut buf);
    header.encode(&mut cursor).expect("encode header");
    body.encode(&mut cursor).expect("encode body");

    let mut read_cursor = ReadCursor::new(&buf);
    let decoded_header = DataHeader::decode(&mut read_cursor).expect("decode header");
    let decoded_body = DataBody::decode(&mut read_cursor).expect("decode body");

    assert_eq!(decoded_header, header);
    assert_eq!(decoded_body, body);
}
