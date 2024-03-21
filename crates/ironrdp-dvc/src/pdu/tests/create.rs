use super::*;
use alloc::vec;
use lazy_static::lazy_static;

const CHANNEL_ID: u32 = 0x0000_0003;
const REQ_ENCODED: [u8; 10] = [0x10, 0x03, 0x74, 0x65, 0x73, 0x74, 0x64, 0x76, 0x63, 0x00];
const RESP_ENCODED: [u8; 6] = [0x10, 0x03, 0x00, 0x00, 0x00, 0x00];

lazy_static! {
    static ref REQ_DECODED: CreateRequestPdu = CreateRequestPdu::new(CHANNEL_ID, String::from("testdvc"));
    static ref RESP_DECODED: CreateResponsePdu = CreateResponsePdu::new(CHANNEL_ID, CreationStatus::OK);
}

#[test]
fn decodes_create_request() {
    let mut src = ReadCursor::new(&REQ_ENCODED);
    match DrdynvcServerPdu::decode(&mut src).unwrap() {
        DrdynvcServerPdu::Create(pdu) => assert_eq!(*REQ_DECODED, pdu),
        _ => panic!("Expected Create"),
    }
}

#[test]
fn encodes_create_request() {
    let data = &*REQ_DECODED;
    let mut buffer = vec![0x00; data.size()];
    let mut cursor = WriteCursor::new(&mut buffer);
    data.encode(&mut cursor).unwrap();
    assert_eq!(REQ_ENCODED.as_slice(), buffer.as_slice());
}

#[test]
fn decodes_create_response() {
    let mut src = ReadCursor::new(&RESP_ENCODED);
    match DrdynvcClientPdu::decode(&mut src).unwrap() {
        DrdynvcClientPdu::Create(pdu) => assert_eq!(*RESP_DECODED, pdu),
        _ => panic!("Expected Create"),
    }
}

#[test]
fn encodes_create_response() {
    let data = &*RESP_DECODED;
    let mut buffer = vec![0x00; data.size()];
    let mut cursor = WriteCursor::new(&mut buffer);
    data.encode(&mut cursor).unwrap();
    assert_eq!(RESP_ENCODED.as_slice(), buffer.as_slice());
}

// #[test]
// fn to_buffer_correct_serializes_dvc_create_response_pdu() {
//     let create_response = DVC_CREATE_RESPONSE.clone();

//     let mut buffer = Vec::new();
//     create_response.to_buffer(&mut buffer).unwrap();

//     assert_eq!(RESP_ENCODED.as_ref(), buffer.as_slice());
// }

// #[test]
// fn buffer_length_is_correct_for_dvc_create_response_pdu() {
//     let create_response = DVC_CREATE_RESPONSE.clone();
//     let expected_buf_len = RESP_ENCODED.len();

//     let len = create_response.buffer_length();

//     assert_eq!(expected_buf_len, len);
// }
