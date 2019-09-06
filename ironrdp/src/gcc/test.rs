use lazy_static::lazy_static;
use num_traits::{FromPrimitive, ToPrimitive};

use super::*;
use crate::gcc::{
    cluster_data, core_data, message_channel_data, monitor_data, monitor_extended_data,
    multi_transport_channel_data, network_data, security_data,
};

const USER_HEADER_LEN: usize = 4;

lazy_static! {
    pub static ref CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER: Vec<u8> = {
        let mut buffer = CLIENT_GCC_CORE_BLOCK_BUFFER.to_vec();
        buffer.extend(CLIENT_GCC_SECURITY_BLOCK_BUFFER.as_slice());
        buffer.extend(CLIENT_GCC_NETWORK_BLOCK_BUFFER.as_slice());

        buffer
    };
    pub static ref CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD_BUFFER: Vec<u8> = {
        let mut buffer = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER.clone();
        buffer.extend(CLIENT_GCC_CLUSTER_BLOCK_BUFFER.as_slice());

        buffer
    };
    pub static ref CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS_BUFFER: Vec<u8> = {
        let mut buffer = CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD_BUFFER.clone();
        buffer.extend(CLIENT_GCC_MONITOR_BLOCK_BUFFER.as_slice());
        buffer.extend(CLIENT_GCC_MONITOR_EXTENDED_BLOCK_BUFFER.as_slice());

        buffer
    };
    pub static ref CLIENT_GCC_WITH_OPTIONAL_FIELDS_IN_DIFFERENT_ORDER_BUFFER: Vec<u8> = {
        let mut buffer = CLIENT_GCC_CORE_BLOCK_BUFFER.to_vec();
        buffer.extend(CLIENT_GCC_CLUSTER_BLOCK_BUFFER.as_slice());
        buffer.extend(CLIENT_GCC_SECURITY_BLOCK_BUFFER.as_slice());
        buffer.extend(CLIENT_GCC_MONITOR_BLOCK_BUFFER.as_slice());
        buffer.extend(CLIENT_GCC_NETWORK_BLOCK_BUFFER.as_slice());
        buffer.extend(CLIENT_GCC_MONITOR_EXTENDED_BLOCK_BUFFER.as_slice());

        buffer
    };
    pub static ref SERVER_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER: Vec<u8> = {
        let mut buffer = SERVER_GCC_CORE_BLOCK_BUFFER.to_vec();
        buffer.extend(SERVER_GCC_NETWORK_BLOCK_BUFFER.as_slice());
        buffer.extend(SERVER_GCC_SECURITY_BLOCK_BUFFER.as_slice());

        buffer
    };
    pub static ref SERVER_GCC_WITH_OPTIONAL_FIELDS_BUFFER: Vec<u8> = {
        let mut buffer = SERVER_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER.clone();
        buffer.extend(SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER.as_slice());
        buffer.extend(SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER.as_slice());

        buffer
    };
    pub static ref SERVER_GCC_WITH_OPTIONAL_FIELDS_IN_DIFFERENT_ORDER_BUFFER: Vec<u8> = {
        let mut buffer = SERVER_GCC_CORE_BLOCK_BUFFER.to_vec();
        buffer.extend(SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER.as_slice());
        buffer.extend(SERVER_GCC_NETWORK_BLOCK_BUFFER.as_slice());
        buffer.extend(SERVER_GCC_SECURITY_BLOCK_BUFFER.as_slice());
        buffer.extend(SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER.as_slice());

        buffer
    };
    pub static ref CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS: ClientGccBlocks = ClientGccBlocks {
        core: core_data::client::test::CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL
            .clone(),
        security: security_data::test::CLIENT_SECURITY_DATA.clone(),
        network: Some(network_data::test::CLIENT_NETOWORK_DATA_WITH_CHANNELS.clone()),
        cluster: None,
        monitor: None,
        message_channel: None,
        multi_transport_channel: None,
        monitor_extended: None,
    };
    pub static ref CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD: ClientGccBlocks = {
        let mut data = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
        data.cluster = Some(cluster_data::test::CLUSTER_DATA.clone());

        data
    };
    pub static ref CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS: ClientGccBlocks = {
        let mut data = CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD.clone();
        data.monitor = Some(monitor_data::test::MONITOR_DATA_WITH_MONITORS.clone());
        data.monitor_extended =
            Some(monitor_extended_data::test::MONITOR_DATA_WITH_MONITORS.clone());

        data
    };
    pub static ref SERVER_GCC_WITHOUT_OPTIONAL_FIELDS: ServerGccBlocks = ServerGccBlocks {
        core: core_data::server::test::SERVER_CORE_DATA_TO_FLAGS.clone(),
        network: network_data::test::SERVER_NETOWORK_DATA_WITH_CHANNELS_ID.clone(),
        security: security_data::test::SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS.clone(),
        message_channel: None,
        multi_transport_channel: None,
    };
    pub static ref SERVER_GCC_WITH_OPTIONAL_FIELDS: ServerGccBlocks = {
        let mut data = SERVER_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
        data.message_channel = Some(message_channel_data::test::SERVER_GCC_MESSAGE_CHANNEL_BLOCK);
        data.multi_transport_channel = Some(
            multi_transport_channel_data::test::SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK.clone(),
        );

        data
    };
    pub static ref CLIENT_GCC_CORE_BLOCK_BUFFER: Vec<u8> = make_gcc_block_buffer(
        ClientGccType::CoreData,
        core_data::client::test::CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL_BUFFER
            .as_ref(),
    );
    pub static ref CLIENT_GCC_SECURITY_BLOCK_BUFFER: Vec<u8> = make_gcc_block_buffer(
        ClientGccType::SecurityData,
        security_data::test::CLIENT_SECURITY_DATA_BUFFER.as_ref(),
    );
    pub static ref CLIENT_GCC_NETWORK_BLOCK_BUFFER: Vec<u8> = make_gcc_block_buffer(
        ClientGccType::NetworkData,
        network_data::test::CLIENT_NETWORK_DATA_WITH_CHANNELS_BUFFER.as_ref(),
    );
    pub static ref CLIENT_GCC_CLUSTER_BLOCK_BUFFER: Vec<u8> = make_gcc_block_buffer(
        ClientGccType::ClusterData,
        cluster_data::test::CLUSTER_DATA_BUFFER.as_ref(),
    );
    pub static ref CLIENT_GCC_MONITOR_BLOCK_BUFFER: Vec<u8> = make_gcc_block_buffer(
        ClientGccType::MonitorData,
        monitor_data::test::MONITOR_DATA_WITH_MONITORS_BUFFER.as_ref(),
    );
    pub static ref CLIENT_GCC_MONITOR_EXTENDED_BLOCK_BUFFER: Vec<u8> = make_gcc_block_buffer(
        ClientGccType::MonitorExtendedData,
        monitor_extended_data::test::MONITOR_DATA_WITH_MONITORS_BUFFER.as_ref(),
    );
    pub static ref SERVER_GCC_CORE_BLOCK_BUFFER: Vec<u8> = make_gcc_block_buffer(
        ServerGccType::CoreData,
        core_data::server::test::SERVER_CORE_DATA_TO_REQUESTED_PROTOCOL_BUFFER.as_ref(),
    );
    pub static ref SERVER_GCC_NETWORK_BLOCK_BUFFER: Vec<u8> = make_gcc_block_buffer(
        ServerGccType::NetworkData,
        network_data::test::SERVER_NETWORK_DATA_WITH_CHANNELS_ID_BUFFER.as_ref(),
    );
    pub static ref SERVER_GCC_SECURITY_BLOCK_BUFFER: Vec<u8> = make_gcc_block_buffer(
        ServerGccType::SecurityData,
        security_data::test::SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS_BUFFER.as_ref(),
    );
    pub static ref SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER: Vec<u8> = make_gcc_block_buffer(
        ServerGccType::MessageChannelData,
        message_channel_data::test::SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER.as_ref(),
    );
    pub static ref SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER: Vec<u8> = make_gcc_block_buffer(
        ServerGccType::MultiTransportChannelData,
        multi_transport_channel_data::test::SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER
            .as_ref(),
    );
}

fn make_gcc_block_buffer<T: FromPrimitive + ToPrimitive>(data_type: T, buffer: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    result.extend(data_type.to_u16().unwrap().to_le_bytes().as_ref());
    result.extend(
        ((buffer.len() + USER_HEADER_LEN) as u16)
            .to_le_bytes()
            .as_ref(),
    );
    result.extend(buffer.as_ref());

    result
}

#[test]
fn from_buffer_correctly_parses_client_gcc_blocks_without_optional_data_blocks() {
    let buffer = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER.clone();

    assert_eq!(
        *CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS,
        ClientGccBlocks::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_client_gcc_blocks_with_one_optional_data_blocks() {
    let buffer = CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD_BUFFER.clone();

    assert_eq!(
        *CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD,
        ClientGccBlocks::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_client_gcc_blocks_with_all_optional_data_blocks() {
    let buffer = CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS_BUFFER.clone();

    assert_eq!(
        *CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS,
        ClientGccBlocks::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_client_gcc_blocks_with_optional_data_blocks_in_different_order() {
    let buffer = CLIENT_GCC_WITH_OPTIONAL_FIELDS_IN_DIFFERENT_ORDER_BUFFER.clone();

    assert_eq!(
        *CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS,
        ClientGccBlocks::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_fails_on_invalid_gcc_type_for_client_gcc_blocks() {
    let mut buffer = CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS_BUFFER.clone();
    buffer[CLIENT_GCC_CORE_BLOCK_BUFFER.len()] = 0x00;

    assert!(ClientGccBlocks::from_buffer(buffer.as_slice()).is_err());
}

#[test]
fn to_buffer_correctly_serializes_client_gcc_blocks_without_optional_data_blocks() {
    let data = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    let expected_buffer = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER.clone();

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_slice(), buff.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_client_gcc_blocks_with_one_optional_data_blocks() {
    let data = CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD.clone();
    let expected_buffer = CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD_BUFFER.clone();

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_slice(), buff.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_client_gcc_blocks_with_all_optional_data_blocks() {
    let data = CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS.clone();
    let expected_buffer = CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS_BUFFER.clone();

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_slice(), buff.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_gcc_blocks_without_optional_data_blocks() {
    let data = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_client_gcc_blocks_with_one_optional_data_blocks() {
    let data = CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD.clone();
    let expected_buffer_len = CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_client_gcc_blocks_with_all_optional_data_blocks() {
    let data = CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_parses_server_gcc_blocks_without_optional_data_blocks() {
    let buffer = SERVER_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER.clone();

    assert_eq!(
        *SERVER_GCC_WITHOUT_OPTIONAL_FIELDS,
        ServerGccBlocks::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_server_gcc_blocks_with_optional_data_blocks() {
    let buffer = SERVER_GCC_WITH_OPTIONAL_FIELDS_BUFFER.clone();

    assert_eq!(
        *SERVER_GCC_WITH_OPTIONAL_FIELDS,
        ServerGccBlocks::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_server_gcc_blocks_with_optional_data_blocks_in_different_order() {
    let buffer = SERVER_GCC_WITH_OPTIONAL_FIELDS_IN_DIFFERENT_ORDER_BUFFER.clone();

    assert_eq!(
        *SERVER_GCC_WITH_OPTIONAL_FIELDS,
        ServerGccBlocks::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_fails_on_invalid_gcc_type_for_server_gcc_blocks() {
    let mut buffer = SERVER_GCC_WITH_OPTIONAL_FIELDS_BUFFER.clone();
    buffer[SERVER_GCC_CORE_BLOCK_BUFFER.len()] = 0x00;

    assert!(ServerGccBlocks::from_buffer(buffer.as_slice()).is_err());
}

#[test]
fn to_buffer_correctly_serializes_server_gcc_blocks_without_optional_data_blocks() {
    let data = SERVER_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    let expected_buffer = SERVER_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER.clone();

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_slice(), buff.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_server_gcc_blocks_with_optional_data_blocks() {
    let data = SERVER_GCC_WITH_OPTIONAL_FIELDS.clone();
    let expected_buffer = SERVER_GCC_WITH_OPTIONAL_FIELDS_BUFFER.clone();

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_slice(), buff.as_slice());
}

#[test]
fn buffer_length_is_correct_for_server_gcc_blocks_without_optional_data_blocks() {
    let data = SERVER_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = SERVER_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_server_gcc_blocks_with_optional_data_blocks() {
    let data = SERVER_GCC_WITH_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = SERVER_GCC_WITH_OPTIONAL_FIELDS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_handles_invalid_lengths_in_user_data_header() {
    let buffer: [u8; 4] = [0x01, 0xc0, 0x00, 0x00];

    assert!(UserDataHeader::<ClientGccType>::from_buffer(buffer.as_ref()).is_err());
}
