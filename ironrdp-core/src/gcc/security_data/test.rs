use lazy_static::lazy_static;

use super::*;

pub const CLIENT_SECURITY_DATA_BUFFER: [u8; 8] = [
    0x1b, 0x00, 0x00, 0x00, // encryption methods
    0x00, 0x00, 0x00, 0x00, // ext encryption methods
];

const SERVER_SECURITY_DATA_WITHOUT_OPTIONAL_FIELDS_BUFFER: [u8; 8] = [
    0x00, 0x00, 0x00, 0x00, // encryption method
    0x00, 0x00, 0x00, 0x00, // encryption level
];
const SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS_PREFIX_BUFFER: [u8; 8] = [
    0x02, 0x00, 0x00, 0x00, // encryption method
    0x02, 0x00, 0x00, 0x00, // encryption level
];
const SERVER_RANDOM_BUFFER: [u8; 32] = [
    0x10, 0x11, 0x77, 0x20, 0x30, 0x61, 0x0a, 0x12, 0xe4, 0x34, 0xa1, 0x1e, 0xf2, 0xc3, 0x9f, 0x31, 0x7d, 0xa4, 0x5f,
    0x01, 0x89, 0x34, 0x96, 0xe0, 0xff, 0x11, 0x08, 0x69, 0x7f, 0x1a, 0xc3, 0xd2,
];
const SERVER_CERT_BUFFER: [u8; 184] = [
    0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x00, 0x5c, 0x00, 0x52, 0x53, 0x41,
    0x31, 0x48, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x3f, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0xcb, 0x81,
    0xfe, 0xba, 0x6d, 0x61, 0xc3, 0x55, 0x05, 0xd5, 0x5f, 0x2e, 0x87, 0xf8, 0x71, 0x94, 0xd6, 0xf1, 0xa5, 0xcb, 0xf1,
    0x5f, 0x0c, 0x3d, 0xf8, 0x70, 0x02, 0x96, 0xc4, 0xfb, 0x9b, 0xc8, 0x3c, 0x2d, 0x55, 0xae, 0xe8, 0xff, 0x32, 0x75,
    0xea, 0x68, 0x79, 0xe5, 0xa2, 0x01, 0xfd, 0x31, 0xa0, 0xb1, 0x1f, 0x55, 0xa6, 0x1f, 0xc1, 0xf6, 0xd1, 0x83, 0x88,
    0x63, 0x26, 0x56, 0x12, 0xbc, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x48, 0x00, 0xe9, 0xe1,
    0xd6, 0x28, 0x46, 0x8b, 0x4e, 0xf5, 0x0a, 0xdf, 0xfd, 0xee, 0x21, 0x99, 0xac, 0xb4, 0xe1, 0x8f, 0x5f, 0x81, 0x57,
    0x82, 0xef, 0x9d, 0x96, 0x52, 0x63, 0x27, 0x18, 0x29, 0xdb, 0xb3, 0x4a, 0xfd, 0x9a, 0xda, 0x42, 0xad, 0xb5, 0x69,
    0x21, 0x89, 0x0e, 0x1d, 0xc0, 0x4c, 0x1a, 0xa8, 0xaa, 0x71, 0x3e, 0x0f, 0x54, 0xb9, 0x9a, 0xe4, 0x99, 0x68, 0x3f,
    0x6c, 0xd6, 0x76, 0x84, 0x61, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

lazy_static! {
    pub static ref CLIENT_SECURITY_DATA: ClientSecurityData = ClientSecurityData {
        encryption_methods: EncryptionMethod::BIT_40
            | EncryptionMethod::BIT_128
            | EncryptionMethod::BIT_56
            | EncryptionMethod::FIPS,
        ext_encryption_methods: 0,
    };
    pub static ref SERVER_SECURITY_DATA_WITHOUT_OPTIONAL_FIELDS: ServerSecurityData = ServerSecurityData {
        encryption_method: EncryptionMethod::empty(),
        encryption_level: EncryptionLevel::None,
        server_random: None,
        server_cert: Vec::new(),
    };
    pub static ref SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS: ServerSecurityData = ServerSecurityData {
        encryption_method: EncryptionMethod::BIT_128,
        encryption_level: EncryptionLevel::ClientCompatible,
        server_random: Some(SERVER_RANDOM_BUFFER),
        server_cert: SERVER_CERT_BUFFER.to_vec(),
    };
    pub static ref SERVER_SECURITY_DATA_WITH_MISMATCH_OF_REQUIRED_AND_OPTIONAL_FIELDS: ServerSecurityData =
        ServerSecurityData {
            encryption_method: EncryptionMethod::empty(),
            encryption_level: EncryptionLevel::None,
            server_random: Some(SERVER_RANDOM_BUFFER),
            server_cert: SERVER_CERT_BUFFER.to_vec(),
        };
    pub static ref SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS_BUFFER: Vec<u8> = {
        let server_random_len = SERVER_RANDOM_BUFFER.len() as u32;
        let server_cert_len = SERVER_CERT_BUFFER.len() as u32;
        let server_random_len_bytes = server_random_len.to_le_bytes();
        let server_cert_len_bytes = server_cert_len.to_le_bytes();

        let mut buffer = SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS_PREFIX_BUFFER.to_vec();
        buffer.extend(server_random_len_bytes.as_ref());
        buffer.extend(server_cert_len_bytes.as_ref());
        buffer.extend(SERVER_RANDOM_BUFFER.as_ref());
        buffer.extend(SERVER_CERT_BUFFER.as_ref());

        buffer
    };
    static ref SERVER_SECURITY_DATA_WITH_INVALID_SERVER_RANDOM_BUFFER: Vec<u8> = {
        let mut server_random = SERVER_RANDOM_BUFFER.to_vec();
        server_random.push(0);
        let server_random_len = server_random.len() as u32;
        let server_cert_len = SERVER_CERT_BUFFER.len() as u32;
        let server_random_len_bytes = server_random_len.to_le_bytes();
        let server_cert_len_bytes = server_cert_len.to_le_bytes();

        let mut buffer = SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS_PREFIX_BUFFER.to_vec();
        buffer.extend(server_random_len_bytes.as_ref());
        buffer.extend(server_cert_len_bytes.as_ref());
        buffer.extend(server_random.as_slice());
        buffer.extend(SERVER_CERT_BUFFER.as_ref());

        buffer
    };
}

#[test]
fn from_buffer_correctly_parses_client_security_data() {
    let buffer = CLIENT_SECURITY_DATA_BUFFER.as_ref();

    assert_eq!(*CLIENT_SECURITY_DATA, ClientSecurityData::from_buffer(buffer).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_client_security_data() {
    let security_data = CLIENT_SECURITY_DATA.clone();
    let expected_buffer = CLIENT_SECURITY_DATA_BUFFER;

    let mut buff = Vec::new();
    security_data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_ref(), buff.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_security_data() {
    let data = CLIENT_SECURITY_DATA.clone();
    let expected_buffer_len = CLIENT_SECURITY_DATA_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_parses_server_security_data_without_optional_fields() {
    let buffer = SERVER_SECURITY_DATA_WITHOUT_OPTIONAL_FIELDS_BUFFER.as_ref();

    assert_eq!(
        *SERVER_SECURITY_DATA_WITHOUT_OPTIONAL_FIELDS,
        ServerSecurityData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_server_security_data_with_all_fields() {
    let buffer = SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS_BUFFER.clone();

    assert_eq!(
        *SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS,
        ServerSecurityData::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_server_security_data_fails_with_invalid_server_random_length() {
    let buffer = SERVER_SECURITY_DATA_WITH_INVALID_SERVER_RANDOM_BUFFER.clone();

    match ServerSecurityData::from_buffer(buffer.as_slice()) {
        Err(SecurityDataError::InvalidServerRandomLen(_)) => (),
        res => panic!("Expected the invalid server random length error, got: {:?}", res),
    };
}

#[test]
fn to_buffer_correctly_serializes_server_security_data_without_optional_fields() {
    let security_data = SERVER_SECURITY_DATA_WITHOUT_OPTIONAL_FIELDS.clone();
    let expected_buffer = SERVER_SECURITY_DATA_WITHOUT_OPTIONAL_FIELDS_BUFFER;

    let mut buff = Vec::new();
    security_data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_ref(), buff.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_server_security_data_with_optional_fields() {
    let security_data = SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS.clone();
    let expected_buffer = SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS_BUFFER.clone();

    let mut buff = Vec::new();
    security_data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer, buff);
}

#[test]
fn to_buffer_server_security_data_fails_on_mismatch_of_required_and_optional_fields() {
    let security_data = SERVER_SECURITY_DATA_WITH_MISMATCH_OF_REQUIRED_AND_OPTIONAL_FIELDS.clone();

    let mut buff = Vec::new();
    match security_data.to_buffer(&mut buff) {
        Err(SecurityDataError::InvalidInput(_)) => (),
        res => panic!("Expected the invalid input error, got: {:?}", res),
    };
}

#[test]
fn buffer_length_is_correct_for_server_security_data_without_optional_fields() {
    let data = SERVER_SECURITY_DATA_WITHOUT_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = SERVER_SECURITY_DATA_WITHOUT_OPTIONAL_FIELDS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_server_security_data_with_optional_fields() {
    let data = SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}
