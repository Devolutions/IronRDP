use lazy_static::lazy_static;

use super::*;
use crate::gcc::conference_create;

const CONNECT_INITIAL_PREFIX_BUFFER_LEN: usize = 107;
const CONNECT_INITIAL_PREFIX_BUFFER: [u8; CONNECT_INITIAL_PREFIX_BUFFER_LEN] = [
    0x7f, 0x65, 0x82, 0x01, 0x99, 0x04, 0x01, 0x01, 0x04, 0x01, 0x01, 0x01, 0x01, 0xff, 0x30, 0x1a,
    0x02, 0x01, 0x22, 0x02, 0x01, 0x02, 0x02, 0x01, 0x00, 0x02, 0x01, 0x01, 0x02, 0x01, 0x00, 0x02,
    0x01, 0x01, 0x02, 0x03, 0x00, 0xff, 0xff, 0x02, 0x01, 0x02, 0x30, 0x19, 0x02, 0x01, 0x01, 0x02,
    0x01, 0x01, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x01, 0x02, 0x02,
    0x04, 0x20, 0x02, 0x01, 0x02, 0x30, 0x20, 0x02, 0x03, 0x00, 0xff, 0xff, 0x02, 0x03, 0x00, 0xfc,
    0x17, 0x02, 0x03, 0x00, 0xff, 0xff, 0x02, 0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x01, 0x02,
    0x03, 0x00, 0xff, 0xff, 0x02, 0x01, 0x02, 0x04, 0x82, 0x01, 0x33,
];

const CONNECT_RESPONSE_PREFIX_BUFFER_LEN: usize = 43;
const CONNECT_RESPONSE_PREFIX_BUFFER: [u8; CONNECT_RESPONSE_PREFIX_BUFFER_LEN] = [
    0x7f, 0x66, 0x82, 0x01, 0x46, 0x0a, 0x01, 0x00, 0x02, 0x01, 0x00, 0x30, 0x1a, 0x02, 0x01, 0x22,
    0x02, 0x01, 0x03, 0x02, 0x01, 0x00, 0x02, 0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x01, 0x02,
    0x03, 0x00, 0xff, 0xf8, 0x02, 0x01, 0x02, 0x04, 0x82, 0x01, 0x20,
];

lazy_static! {
    static ref CONNECT_INITIAL_BUFFER: Vec<u8> = {
        let mut buffer = CONNECT_INITIAL_PREFIX_BUFFER.to_vec();
        buffer.extend(conference_create::test::CONFERENCE_CREATE_REQUEST_BUFFER.as_slice());

        buffer
    };
    static ref CONNECT_INITIAL: ConnectInitial = ConnectInitial {
        calling_domain_selector: vec![0x01],
        called_domain_selector: vec![0x01],
        upward_flag: true,
        target_parameters: DomainParameters {
            max_channel_ids: 34,
            max_user_ids: 2,
            max_token_ids: 0,
            num_priorities: 1,
            min_throughput: 0,
            max_height: 1,
            max_mcs_pdu_size: 65535,
            protocol_version: 2,
        },
        min_parameters: DomainParameters {
            max_channel_ids: 1,
            max_user_ids: 1,
            max_token_ids: 1,
            num_priorities: 1,
            min_throughput: 0,
            max_height: 1,
            max_mcs_pdu_size: 1056,
            protocol_version: 2,
        },
        max_parameters: DomainParameters {
            max_channel_ids: 65535,
            max_user_ids: 64535,
            max_token_ids: 65535,
            num_priorities: 1,
            min_throughput: 0,
            max_height: 1,
            max_mcs_pdu_size: 65535,
            protocol_version: 2,
        },
        conference_create_request: conference_create::test::CONFERENCE_CREATE_REQUEST.clone(),
    };
    static ref CONNECT_RESPONSE_BUFFER: Vec<u8> = {
        let mut buffer = CONNECT_RESPONSE_PREFIX_BUFFER.to_vec();
        buffer.extend(conference_create::test::CONFERENCE_CREATE_RESPONSE_BUFFER.as_slice());

        buffer
    };
    static ref CONNECT_RESPONSE: ConnectResponse = ConnectResponse {
        called_connect_id: 0,
        domain_parameters: DomainParameters {
            max_channel_ids: 34,
            max_user_ids: 3,
            max_token_ids: 0,
            num_priorities: 1,
            min_throughput: 0,
            max_height: 1,
            max_mcs_pdu_size: 65528,
            protocol_version: 2,
        },
        conference_create_response: conference_create::test::CONFERENCE_CREATE_RESPONSE.clone(),
    };
}

#[test]
fn from_buffer_correct_parses_connect_initial() {
    let buffer = CONNECT_INITIAL_BUFFER.clone();
    let expected_blocks = CONNECT_INITIAL.clone();

    let blocks = ConnectInitial::from_buffer(buffer.as_slice()).unwrap();

    assert_eq!(expected_blocks, blocks);
}

#[test]
fn to_buffer_correct_serializes_connect_initial() {
    let blocks = CONNECT_INITIAL.clone();
    let expected_buffer = CONNECT_INITIAL_BUFFER.clone();

    let mut buf = Vec::new();
    blocks.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer, buf);
}

#[test]
fn buffer_length_is_correct_for_connect_initial() {
    let blocks = CONNECT_INITIAL.clone();
    let expected_buffer_len = CONNECT_INITIAL_BUFFER.len();

    let len = blocks.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correct_parses_connect_response() {
    let buffer = CONNECT_RESPONSE_BUFFER.clone();
    let expected_blocks = CONNECT_RESPONSE.clone();

    let blocks = ConnectResponse::from_buffer(buffer.as_slice()).unwrap();

    assert_eq!(expected_blocks, blocks);
}

#[test]
fn to_buffer_correct_serializes_connect_response() {
    let blocks = CONNECT_RESPONSE.clone();
    let expected_buffer = CONNECT_RESPONSE_BUFFER.clone();

    let mut buf = Vec::new();
    blocks.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer, buf);
}

#[test]
fn buffer_length_is_correct_for_connect_response() {
    let blocks = CONNECT_RESPONSE.clone();
    let expected_buffer_len = CONNECT_RESPONSE_BUFFER.len();

    let len = blocks.buffer_length();

    assert_eq!(expected_buffer_len, len);
}
