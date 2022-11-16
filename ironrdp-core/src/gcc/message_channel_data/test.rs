use super::*;

pub const SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER: [u8; 2] = [0xf0, 0x03];
pub const SERVER_GCC_MESSAGE_CHANNEL_BLOCK: ServerMessageChannelData = ServerMessageChannelData {
    mcs_message_channel_id: 0x03f0,
};

#[test]
fn from_buffer_correctly_parses_server_message_channel_data() {
    let buffer = SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER.as_ref();

    assert_eq!(
        SERVER_GCC_MESSAGE_CHANNEL_BLOCK,
        ServerMessageChannelData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_server_message_channel_data() {
    let data = SERVER_GCC_MESSAGE_CHANNEL_BLOCK.clone();
    let expected_buffer = SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER;

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_ref(), buff.as_slice());
}

#[test]
fn buffer_length_is_correct_for_server_message_channel_data() {
    let data = SERVER_GCC_MESSAGE_CHANNEL_BLOCK.clone();
    let expected_buffer_len = SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}
