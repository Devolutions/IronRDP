use lazy_static::lazy_static;

use super::*;

pub const CLUSTER_DATA_BUFFER: [u8; 8] = [0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

lazy_static! {
    pub static ref CLUSTER_DATA: ClientClusterData = ClientClusterData {
        flags: RedirectionFlags::REDIRECTION_SUPPORTED,
        redirection_version: RedirectionVersion::V4,
        redirected_session_id: 0,
    };
}

#[test]
fn from_buffer_correctly_parses_client_cluster_data() {
    let buffer = CLUSTER_DATA_BUFFER.as_ref();

    assert_eq!(*CLUSTER_DATA, ClientClusterData::from_buffer(buffer).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_client_cluster_data() {
    let data = CLUSTER_DATA.clone();
    let expected_buffer = CLUSTER_DATA_BUFFER;

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_ref(), buff.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_cluster_data() {
    let data = CLUSTER_DATA.clone();
    let expected_buffer_len = CLUSTER_DATA_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}
