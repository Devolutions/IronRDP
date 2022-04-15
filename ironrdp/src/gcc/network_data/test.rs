use lazy_static::lazy_static;

use super::*;

pub const CLIENT_NETWORK_DATA_WITH_CHANNELS_BUFFER: [u8; 40] = [
    0x03, 0x00, 0x00, 0x00, // channels count
    0x72, 0x64, 0x70, 0x64, 0x72, 0x00, 0x00, 0x00, // channel 1::name
    0x00, 0x00, 0x80, 0x80, // channel 1::options
    0x63, 0x6c, 0x69, 0x70, 0x72, 0x64, 0x72, 0x00, // channel 2::name
    0x00, 0x00, 0xa0, 0xc0, // channel 2::options
    0x72, 0x64, 0x70, 0x73, 0x6e, 0x64, 0x00, 0x00, // channel 3::name
    0x00, 0x00, 0x00, 0xc0, // channel 3::options
];
pub const SERVER_NETWORK_DATA_WITH_CHANNELS_ID_BUFFER: [u8; 12] = [
    0xeb, 0x03, // io channel
    0x03, 0x00, // channels count
    0xec, 0x03, // channel 1::id
    0xed, 0x03, // channel 2::id
    0xee, 0x03, // channel 3::id
    0x00, 0x00, // padding
];
const CLIENT_NETWORK_DATA_WITHOUT_CHANNELS_BUFFER: [u8; 4] = [
    0x00, 0x00, 0x00, 0x00, // channels count
];
const SERVER_NETWORK_DATA_WITHOUT_CHANNELS_ID_BUFFER: [u8; 4] = [
    0xeb, 0x03, // io channel
    0x00, 0x00, // channels count
];

lazy_static! {
    pub static ref CLIENT_NETOWORK_DATA_WITH_CHANNELS: ClientNetworkData = ClientNetworkData {
        channels: vec![
            Channel {
                name: String::from("rdpdr"),
                options: ChannelOptions::INITIALIZED | ChannelOptions::COMPRESS_RDP,
            },
            Channel {
                name: String::from("cliprdr"),
                options: ChannelOptions::INITIALIZED
                    | ChannelOptions::COMPRESS_RDP
                    | ChannelOptions::ENCRYPT_RDP
                    | ChannelOptions::SHOW_PROTOCOL,
            },
            Channel {
                name: String::from("rdpsnd"),
                options: ChannelOptions::INITIALIZED | ChannelOptions::ENCRYPT_RDP,
            },
        ],
    };
    pub static ref SERVER_NETOWORK_DATA_WITH_CHANNELS_ID: ServerNetworkData = ServerNetworkData {
        io_channel: 1003,
        channel_ids: vec![1004, 1005, 1006],
    };
    static ref CLIENT_NETOWORK_DATA_WITHOUT_CHANNELS: ClientNetworkData = ClientNetworkData { channels: Vec::new() };
    static ref SERVER_NETOWORK_DATA_WITHOUT_CHANNELS_ID: ServerNetworkData = ServerNetworkData {
        io_channel: 1003,
        channel_ids: Vec::new(),
    };
}

#[test]
fn from_buffer_correctly_parses_client_network_data_without_channels() {
    let buffer = CLIENT_NETWORK_DATA_WITHOUT_CHANNELS_BUFFER.as_ref();

    assert_eq!(
        *CLIENT_NETOWORK_DATA_WITHOUT_CHANNELS,
        ClientNetworkData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_client_network_data_with_channels() {
    let buffer = CLIENT_NETWORK_DATA_WITH_CHANNELS_BUFFER.as_ref();

    assert_eq!(
        *CLIENT_NETOWORK_DATA_WITH_CHANNELS,
        ClientNetworkData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_client_network_data_without_channels() {
    let data = CLIENT_NETOWORK_DATA_WITHOUT_CHANNELS.clone();
    let expected_buffer = CLIENT_NETWORK_DATA_WITHOUT_CHANNELS_BUFFER;

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_ref(), buff.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_client_network_data_with_channels() {
    let data = CLIENT_NETOWORK_DATA_WITH_CHANNELS.clone();
    let expected_buffer = CLIENT_NETWORK_DATA_WITH_CHANNELS_BUFFER;

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_ref(), buff.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_network_data_without_channels() {
    let data = CLIENT_NETOWORK_DATA_WITHOUT_CHANNELS.clone();
    let expected_buffer_len = CLIENT_NETWORK_DATA_WITHOUT_CHANNELS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_client_network_data_with_channels() {
    let data = CLIENT_NETOWORK_DATA_WITH_CHANNELS.clone();
    let expected_buffer_len = CLIENT_NETWORK_DATA_WITH_CHANNELS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_parses_server_network_data_without_channels_id() {
    let buffer = SERVER_NETWORK_DATA_WITHOUT_CHANNELS_ID_BUFFER.as_ref();

    assert_eq!(
        *SERVER_NETOWORK_DATA_WITHOUT_CHANNELS_ID,
        ServerNetworkData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_server_network_data_with_channels_id() {
    let buffer = SERVER_NETWORK_DATA_WITH_CHANNELS_ID_BUFFER.as_ref();

    assert_eq!(
        *SERVER_NETOWORK_DATA_WITH_CHANNELS_ID,
        ServerNetworkData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_server_network_data_without_channels_id() {
    let data = SERVER_NETOWORK_DATA_WITHOUT_CHANNELS_ID.clone();
    let expected_buffer = SERVER_NETWORK_DATA_WITHOUT_CHANNELS_ID_BUFFER;

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_ref(), buff.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_server_network_data_with_channels_id() {
    let data = SERVER_NETOWORK_DATA_WITH_CHANNELS_ID.clone();
    let expected_buffer = SERVER_NETWORK_DATA_WITH_CHANNELS_ID_BUFFER;

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_ref(), buff.as_slice());
}

#[test]
fn buffer_length_is_correct_for_server_network_data_without_channels_id() {
    let data = SERVER_NETOWORK_DATA_WITHOUT_CHANNELS_ID.clone();
    let expected_buffer_len = SERVER_NETWORK_DATA_WITHOUT_CHANNELS_ID_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_server_network_data_with_channels_id() {
    let data = SERVER_NETOWORK_DATA_WITH_CHANNELS_ID.clone();
    let expected_buffer_len = SERVER_NETWORK_DATA_WITH_CHANNELS_ID_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}
