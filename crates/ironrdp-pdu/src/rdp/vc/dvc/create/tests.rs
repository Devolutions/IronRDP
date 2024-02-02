use lazy_static::lazy_static;

use super::*;
use crate::encode_vec;

const TEST_CHANNEL_ID: u32 = 0x0000_0003;

const DVC_CREATE_REQUEST_BUFFER_SIZE: usize = 10;
const DVC_CREATE_REQUEST_BUFFER: [u8; DVC_CREATE_REQUEST_BUFFER_SIZE] =
    [0x10, 0x03, 0x74, 0x65, 0x73, 0x74, 0x64, 0x76, 0x63, 0x00];

const DVC_CREATE_RESPONSE_BUFFER_SIZE: usize = 6;
const DVC_CREATE_RESPONSE_BUFFER: [u8; DVC_CREATE_RESPONSE_BUFFER_SIZE] = [0x10, 0x03, 0x00, 0x00, 0x00, 0x00];

const DVC_TEST_HEADER_SIZE: usize = 0x01;

lazy_static! {
    static ref DVC_CREATE_REQUEST: CreateRequestPdu = CreateRequestPdu {
        channel_id_type: FieldType::U8,
        channel_id: TEST_CHANNEL_ID,
        channel_name: String::from("testdvc")
    };
    static ref DVC_CREATE_RESPONSE: CreateResponsePdu = CreateResponsePdu {
        channel_id_type: FieldType::U8,
        channel_id: TEST_CHANNEL_ID,
        creation_status: DVC_CREATION_STATUS_OK
    };
}

#[test]
fn from_buffer_correct_parses_dvc_create_request_pdu() {
    let mut cur = ReadCursor::new(&DVC_CREATE_REQUEST_BUFFER[1..]);
    assert_eq!(
        DVC_CREATE_REQUEST.clone(),
        CreateRequestPdu::decode(
            &mut cur,
            FieldType::U8,
            DVC_CREATE_REQUEST_BUFFER_SIZE - DVC_TEST_HEADER_SIZE
        )
        .unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_dvc_create_request_pdu() {
    let create_request = DVC_CREATE_REQUEST.clone();

    let buffer = encode_vec(&create_request).unwrap();

    assert_eq!(DVC_CREATE_REQUEST_BUFFER.as_ref(), buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_dvc_create_request_pdu() {
    let create_request = DVC_CREATE_REQUEST.clone();
    let expected_buf_len = DVC_CREATE_REQUEST_BUFFER.len();

    let len = create_request.size();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn from_buffer_correct_parses_dvc_create_response_pdu() {
    let mut cur = ReadCursor::new(&DVC_CREATE_RESPONSE_BUFFER[1..]);
    assert_eq!(
        DVC_CREATE_RESPONSE.clone(),
        CreateResponsePdu::decode(&mut cur, FieldType::U8).unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_dvc_create_response_pdu() {
    let create_response = DVC_CREATE_RESPONSE.clone();

    let buffer = encode_vec(&create_response).unwrap();

    assert_eq!(DVC_CREATE_RESPONSE_BUFFER.as_ref(), buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_dvc_create_response_pdu() {
    let create_response = DVC_CREATE_RESPONSE.clone();
    let expected_buf_len = DVC_CREATE_RESPONSE_BUFFER.len();

    let len = create_response.size();

    assert_eq!(expected_buf_len, len);
}
