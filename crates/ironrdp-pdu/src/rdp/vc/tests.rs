use lazy_static::lazy_static;

use super::*;

const CHANNEL_CHUNK_LENGTH_DEFAULT: u32 = 1600;
const CHANNEL_PDU_HEADER_BUFFER: [u8; CHANNEL_PDU_HEADER_SIZE] = [0x40, 0x06, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00];

lazy_static! {
    static ref CHANNEL_PDU_HEADER: ChannelPduHeader = ChannelPduHeader {
        length: CHANNEL_CHUNK_LENGTH_DEFAULT,
        flags: ChannelControlFlags::FLAG_FIRST,
    };
}

#[test]
fn from_buffer_correct_parses_channel_header() {
    assert_eq!(
        CHANNEL_PDU_HEADER.clone(),
        ChannelPduHeader::from_buffer(CHANNEL_PDU_HEADER_BUFFER.as_ref()).unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_channel_header() {
    let channel_header = CHANNEL_PDU_HEADER.clone();

    let mut buffer = Vec::new();
    channel_header.to_buffer(&mut buffer).unwrap();

    assert_eq!(CHANNEL_PDU_HEADER_BUFFER.as_ref(), buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_channel_header() {
    let channel_header = CHANNEL_PDU_HEADER.clone();
    let expected_buf_len = CHANNEL_PDU_HEADER_BUFFER.len();

    let len = channel_header.buffer_length();

    assert_eq!(expected_buf_len, len);
}
