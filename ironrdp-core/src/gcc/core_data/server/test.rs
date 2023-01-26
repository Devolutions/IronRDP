use lazy_static::lazy_static;

use crate::connection_initiation;
use crate::gcc::core_data::server::*;
use crate::gcc::core_data::*;

const SERVER_CORE_DATA_BUFFER: [u8; 4] = [
    0x04, 0x00, 0x08, 0x00, // version
];
const REQUESTED_PROTOCOL_BUFFER: [u8; 4] = [
    0x00, 0x00, 0x00, 0x00, // client requested protocols
];
const FLAGS_BUFFER: [u8; 4] = [
    0x01, 0x00, 0x00, 0x00, // early capability flags
];

lazy_static! {
    pub static ref SERVER_CORE_DATA: ServerCoreData = ServerCoreData {
        version: RdpVersion::V5_PLUS,
        optional_data: ServerCoreOptionalData {
            client_requested_protocols: None,
            early_capability_flags: None,
        },
    };
    pub static ref SERVER_CORE_DATA_TO_FLAGS: ServerCoreData = ServerCoreData {
        version: RdpVersion::V5_PLUS,
        optional_data: ServerCoreOptionalData {
            client_requested_protocols: Some(connection_initiation::SecurityProtocol::RDP),
            early_capability_flags: None,
        },
    };
    pub static ref SERVER_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS: ServerCoreData = ServerCoreData {
        version: RdpVersion::V5_PLUS,
        optional_data: ServerCoreOptionalData {
            client_requested_protocols: Some(connection_initiation::SecurityProtocol::RDP),
            early_capability_flags: Some(ServerEarlyCapabilityFlags::EDGE_ACTIONS_SUPPORTED_V1),
        },
    };
    pub static ref SERVER_CORE_DATA_TO_REQUESTED_PROTOCOL_BUFFER: Vec<u8> = {
        let mut buffer = SERVER_CORE_DATA_BUFFER.to_vec();
        buffer.extend(REQUESTED_PROTOCOL_BUFFER.as_ref());

        buffer
    };
    static ref SERVER_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_BUFFER: Vec<u8> = {
        let mut buffer = SERVER_CORE_DATA_TO_REQUESTED_PROTOCOL_BUFFER.to_vec();
        buffer.extend(FLAGS_BUFFER.as_ref());

        buffer
    };
}

#[test]
fn from_buffer_correctly_parses_server_core_data_without_optional_fields() {
    let buffer = SERVER_CORE_DATA_BUFFER.as_ref();

    assert_eq!(*SERVER_CORE_DATA, ServerCoreData::from_buffer(buffer).unwrap());
}

#[test]
fn from_buffer_correctly_parses_server_core_data_without_few_optional_fields() {
    let buffer = SERVER_CORE_DATA_TO_REQUESTED_PROTOCOL_BUFFER.clone();

    assert_eq!(
        *SERVER_CORE_DATA_TO_FLAGS,
        ServerCoreData::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_server_core_data_with_all_optional_fields() {
    let buffer = SERVER_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_BUFFER.clone();

    assert_eq!(
        *SERVER_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS,
        ServerCoreData::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_server_core_data_without_optional_fields() {
    let data = SERVER_CORE_DATA.clone();
    let expected_buffer = SERVER_CORE_DATA_BUFFER;

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_ref(), buff.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_server_core_data_without_few_optional_fields() {
    let data = SERVER_CORE_DATA_TO_FLAGS.clone();
    let expected_buffer = SERVER_CORE_DATA_TO_REQUESTED_PROTOCOL_BUFFER.clone();

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_slice(), buff.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_server_core_data_with_all_optional_fields() {
    let core_data = SERVER_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS.clone();
    let expected_buffer = SERVER_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_BUFFER.clone();

    let mut buff = Vec::new();
    core_data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_slice(), buff.as_slice());
}

#[test]
fn buffer_length_is_correct_for_server_core_data_without_optional_fields() {
    let data = SERVER_CORE_DATA.clone();
    let expected_buffer_len = SERVER_CORE_DATA_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_server_core_data_without_few_optional_fields() {
    let data = SERVER_CORE_DATA_TO_FLAGS.clone();
    let expected_buffer_len = SERVER_CORE_DATA_TO_REQUESTED_PROTOCOL_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_server_core_data_with_all_optional_fields() {
    let data = SERVER_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = SERVER_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}
