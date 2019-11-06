use lazy_static::lazy_static;

use super::*;

const CHANNEL_PDU_HEADER_BUFFER: [u8; CHANNEL_PDU_HEADER_SIZE] =
    [0x40, 0x06, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00];

const CHANNEL_DATA_SIZE: usize = 10;
const CHANNEL_DATA_BUFFER: [u8; CHANNEL_DATA_SIZE] =
    [0x77, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77];

const CHANNEL_CHUNK_LENGTH_DEFAULT: u32 = 1600;

lazy_static! {
    static ref CHANNEL_PDU_HEADER: ChannelPduHeader = ChannelPduHeader {
        total_length: CHANNEL_CHUNK_LENGTH_DEFAULT as u32,
        flags: ChannelControlFlags::FLAG_FIRST,
    };
    static ref CHANNEL_DATA: VirtualChannelData =
        VirtualChannelData::ChannelData(CHANNEL_DATA_BUFFER.to_vec());
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
fn to_buffer_correct_serializes_channel_header_with_invalid_flag_fails() {
    let invalid_header_buffer = vec![0x40, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01];
    match ChannelPduHeader::from_buffer(invalid_header_buffer.as_slice()) {
        Err(ChannelError::InvalidChannelPduHeader) => (),
        res => panic!("Expected the invalid channel header error, got: {:?}", res),
    };
}

#[test]
fn buffer_length_is_correct_for_channel_header() {
    let channel_header = CHANNEL_PDU_HEADER.clone();
    let expected_buf_len = CHANNEL_PDU_HEADER_BUFFER.len();

    let len = channel_header.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn from_buffer_correct_parses_channel_data() {
    assert_eq!(
        CHANNEL_DATA.clone(),
        VirtualChannelData::from_buffer(CHANNEL_DATA_BUFFER.as_ref()).unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_channel_data() {
    let channel_data = CHANNEL_DATA.clone();

    let mut buffer = Vec::new();
    channel_data.to_buffer(&mut buffer).unwrap();

    assert_eq!(CHANNEL_DATA_BUFFER.as_ref(), buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_channel_data() {
    let channel_data = CHANNEL_DATA.clone();
    let expected_buf_len = CHANNEL_DATA_BUFFER.len();

    let len = channel_data.buffer_length();

    assert_eq!(expected_buf_len, len);
}
