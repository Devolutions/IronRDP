use lazy_static::lazy_static;

use super::*;

const DVC_TEST_CHANNEL_ID: u32 = 0x0000_0003;
const DVC_CREATION_STATUS: u32 = 0x0000_0000;

const DVC_CREATE_REQUEST_BUFFER_SIZE: usize = 10;
const DVC_CREATE_REQUEST_BUFFER: [u8; DVC_CREATE_REQUEST_BUFFER_SIZE] =
    [0x10, 0x03, 0x74, 0x65, 0x73, 0x74, 0x64, 0x76, 0x63, 0x00];

const DVC_CREATE_RESPONSE_BUFFER_SIZE: usize = 6;
const DVC_CREATE_RESPONSE_BUFFER: [u8; DVC_CREATE_RESPONSE_BUFFER_SIZE] =
    [0x10, 0x03, 0x00, 0x00, 0x00, 0x00];

lazy_static! {
    static ref DVC_CREATE_REQUEST: CreateRequestPdu = CreateRequestPdu {
        channel_id_type: FieldType::U8,
        channel_id: DVC_TEST_CHANNEL_ID,
        channel_name: String::from("testdvc")
    };
    static ref DVC_CREATE_RESPONSE: CreateResponsePdu = CreateResponsePdu {
        channel_id_type: FieldType::U8,
        channel_id: DVC_TEST_CHANNEL_ID,
        creation_status: DVC_CREATION_STATUS
    };
}

#[test]
fn from_buffer_correct_parses_dvc_create_request_pdu() {
    assert_eq!(
        DVC_CREATE_REQUEST.clone(),
        CreateRequestPdu::from_buffer(&DVC_CREATE_REQUEST_BUFFER[1..], FieldType::U8).unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_dvc_create_request_pdu() {
    let create_request = DVC_CREATE_REQUEST.clone();

    let mut buffer = Vec::new();
    create_request.to_buffer(&mut buffer).unwrap();

    assert_eq!(DVC_CREATE_REQUEST_BUFFER.as_ref(), buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_dvc_create_request_pdu() {
    let create_request = DVC_CREATE_REQUEST.clone();
    let expected_buf_len = DVC_CREATE_REQUEST_BUFFER.len();

    let len = create_request.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn from_buffer_correct_parses_dvc_create_response_pdu() {
    assert_eq!(
        DVC_CREATE_RESPONSE.clone(),
        CreateResponsePdu::from_buffer(&DVC_CREATE_RESPONSE_BUFFER[1..], FieldType::U8).unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_dvc_create_response_pdu() {
    let create_response = DVC_CREATE_RESPONSE.clone();

    let mut buffer = Vec::new();
    create_response.to_buffer(&mut buffer).unwrap();

    assert_eq!(DVC_CREATE_RESPONSE_BUFFER.as_ref(), buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_dvc_create_response_pdu() {
    let create_response = DVC_CREATE_RESPONSE.clone();
    let expected_buf_len = DVC_CREATE_RESPONSE_BUFFER.len();

    let len = create_response.buffer_length();

    assert_eq!(expected_buf_len, len);
}
