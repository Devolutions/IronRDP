use lazy_static::lazy_static;

use super::*;
use crate::gcc;

const CONFERENCE_CREATE_REQUEST_PREFIX_BUFFER: [u8; 23] = [
    0x00, 0x05, 0x00, 0x14, 0x7c, 0x00, 0x01, 0x81, 0x28, 0x00, 0x08, 0x00, 0x10, 0x00, 0x01, 0xc0,
    0x00, 0x44, 0x75, 0x63, 0x61, 0x81, 0x1c,
];
const CONFERENCE_CREATE_RESPONSE_PREFIX_BUFFER: [u8; 24] = [
    0x00, 0x05, 0x00, 0x14, 0x7c, 0x00, 0x01, 0x81, 0x15, 0x14, 0x76, 0x0a, 0x01, 0x01, 0x00, 0x01,
    0xc0, 0x00, 0x4d, 0x63, 0x44, 0x6e, 0x81, 0x08,
];

lazy_static! {
    pub static ref CONFERENCE_CREATE_REQUEST: ConferenceCreateRequest = ConferenceCreateRequest {
        gcc_blocks: gcc::test::CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD.clone(),
    };
    pub static ref CONFERENCE_CREATE_RESPONSE: ConferenceCreateResponse =
        ConferenceCreateResponse {
            user_id: 0x79f3,
            gcc_blocks: gcc::test::SERVER_GCC_WITHOUT_OPTIONAL_FIELDS.clone(),
        };
    pub static ref CONFERENCE_CREATE_REQUEST_BUFFER: Vec<u8> = {
        let mut buffer = CONFERENCE_CREATE_REQUEST_PREFIX_BUFFER.to_vec();
        buffer.extend(gcc::test::CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD_BUFFER.as_slice());

        buffer
    };
    pub static ref CONFERENCE_CREATE_RESPONSE_BUFFER: Vec<u8> = {
        let mut buffer = CONFERENCE_CREATE_RESPONSE_PREFIX_BUFFER.to_vec();
        buffer.extend(gcc::test::SERVER_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER.as_slice());

        buffer
    };
}

#[test]
fn from_buffer_correctly_parses_conference_create_request() {
    let buffer = CONFERENCE_CREATE_REQUEST_BUFFER.clone();

    assert_eq!(
        *CONFERENCE_CREATE_REQUEST,
        ConferenceCreateRequest::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_conference_create_request() {
    let data = CONFERENCE_CREATE_REQUEST.clone();
    let expected_buffer = CONFERENCE_CREATE_REQUEST_BUFFER.clone();

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_slice(), buff.as_slice());
}

#[test]
fn buffer_length_is_correct_for_conference_create_request() {
    let data = CONFERENCE_CREATE_REQUEST.clone();
    let expected_buffer_len = CONFERENCE_CREATE_REQUEST_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_parses_conference_create_response() {
    let buffer = CONFERENCE_CREATE_RESPONSE_BUFFER.clone();

    assert_eq!(
        *CONFERENCE_CREATE_RESPONSE,
        ConferenceCreateResponse::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_conference_create_response() {
    let data = CONFERENCE_CREATE_RESPONSE.clone();
    let expected_buffer = CONFERENCE_CREATE_RESPONSE_BUFFER.clone();

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_slice(), buff.as_slice());
}

#[test]
fn buffer_length_is_correct_for_conference_create_response() {
    let data = CONFERENCE_CREATE_RESPONSE.clone();
    let expected_buffer_len = CONFERENCE_CREATE_RESPONSE_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}
