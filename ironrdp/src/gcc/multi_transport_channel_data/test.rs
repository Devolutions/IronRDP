use lazy_static::lazy_static;

use super::*;

pub const SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER: [u8; 4] = [0x01, 0x03, 0x00, 0x00];

lazy_static! {
    pub static ref SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK: MultiTransportChannelData =
        MultiTransportChannelData {
            flags: MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR
                | MultiTransportFlags::TRANSPORT_TYPE_UDP_PREFERRED
                | MultiTransportFlags::SOFT_SYNC_TCP_TO_UDP,
        };
}

#[test]
fn from_buffer_correctly_parses_server_multi_transport_channel_data() {
    let buffer = SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER.as_ref();

    assert_eq!(
        *SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK,
        MultiTransportChannelData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_server_multi_transport_channel_data() {
    let data = SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK.clone();
    let expected_buffer = SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER;

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_ref(), buff.as_slice());
}

#[test]
fn buffer_length_is_correct_for_server_multi_transport_channel_data() {
    let data = SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK.clone();
    let expected_buffer_len = SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}
