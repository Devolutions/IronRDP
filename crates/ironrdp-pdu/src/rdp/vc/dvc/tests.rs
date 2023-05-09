use lazy_static::lazy_static;

use super::*;

const DVC_HEADER_BUFFER: [u8; HEADER_SIZE] = [0x11];
const DVC_HEADER_WITH_INVALID_ID_LENGTH_TYPE_BUFFER: [u8; HEADER_SIZE] = [0x13];

const TEST_BUFFER_SIZE: usize = 4;
const TEST_BUFFER: [u8; TEST_BUFFER_SIZE] = [0x01, 0x02, 0x03, 0x04];

lazy_static! {
    static ref DYNAMIC_CHANNEL_HEADER: Header = Header {
        channel_id_type: FieldType::U16.to_u8().unwrap(),
        pdu_dependent: 0,
        pdu_type: PduType::Create,
    };
}

#[test]
fn from_buffer_parsing_for_dvc_header_with_invalid_pdu_type_fails() {
    let invalid_header: u8 = 0xA0;
    match Header::from_buffer([invalid_header].as_ref()) {
        Err(ChannelError::InvalidDvcPduType) => (),
        res => panic!("Expected InvalidDvcPduType error, got: {res:?}"),
    };
}

#[test]
fn from_buffer_correct_parses_dvc_header() {
    assert_eq!(
        DYNAMIC_CHANNEL_HEADER.clone(),
        Header::from_buffer(DVC_HEADER_BUFFER.as_ref()).unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_dvc_header() {
    let channel_header = DYNAMIC_CHANNEL_HEADER.clone();

    let mut buffer = Vec::new();
    channel_header.to_buffer(&mut buffer).unwrap();

    assert_eq!(DVC_HEADER_BUFFER.as_ref(), buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_dvc_header() {
    let channel_header = DYNAMIC_CHANNEL_HEADER.clone();
    let expected_buf_len = DVC_HEADER_BUFFER.len();

    let len = channel_header.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn from_buffer_parsing_for_client_dvc_pdu_with_invalid_id_length_type_fails() {
    match ClientPdu::from_buffer(DVC_HEADER_WITH_INVALID_ID_LENGTH_TYPE_BUFFER.as_ref(), HEADER_SIZE) {
        Err(ChannelError::InvalidDVChannelIdLength) => (),
        res => panic!("Expected InvalidDVChannelIdLength error, got: {res:?}"),
    };
}

#[test]
fn from_buffer_parsing_for_server_dvc_pdu_with_invalid_id_length_type_fails() {
    match ServerPdu::from_buffer(DVC_HEADER_WITH_INVALID_ID_LENGTH_TYPE_BUFFER.as_ref(), HEADER_SIZE) {
        Err(ChannelError::InvalidDVChannelIdLength) => (),
        res => panic!("Expected InvalidDVChannelIdLength error, got: {res:?}"),
    };
}

#[test]
fn from_buffer_according_to_type_u8_test() {
    let channel_id = FieldType::U8
        .read_buffer_according_to_type(TEST_BUFFER.as_ref())
        .unwrap();
    let expected_channel_id = 0x01;

    assert_eq!(expected_channel_id, channel_id);
}

#[test]
fn from_buffer_according_to_type_u16_test() {
    let channel_id = FieldType::U16
        .read_buffer_according_to_type(TEST_BUFFER.as_ref())
        .unwrap();
    let expected_channel_id = 0x0201;

    assert_eq!(expected_channel_id, channel_id);
}

#[test]
fn from_buffer_according_to_type_u32_test() {
    let channel_id = FieldType::U32
        .read_buffer_according_to_type(TEST_BUFFER.as_ref())
        .unwrap();
    let expected_channel_id = 0x0403_0201;

    assert_eq!(expected_channel_id, channel_id);
}

#[test]
fn to_buffer_according_to_type_u8_test() {
    let channel_id = 0x01;
    let mut buffer = Vec::new();
    FieldType::U8
        .to_buffer_according_to_type(&mut buffer, channel_id)
        .unwrap();

    let expected_buffer = vec![0x01];
    assert_eq!(expected_buffer, buffer);
}

#[test]
fn to_buffer_according_to_type_u16_test() {
    let channel_id = 0x0201;
    let mut buffer = Vec::new();
    FieldType::U16
        .to_buffer_according_to_type(&mut buffer, channel_id)
        .unwrap();

    let expected_buffer = vec![0x01, 0x02];
    assert_eq!(expected_buffer, buffer);
}

#[test]
fn to_buffer_according_to_type_u32_test() {
    let channel_id = 0x0403_0201;
    let mut buffer = Vec::new();
    FieldType::U32
        .to_buffer_according_to_type(&mut buffer, channel_id)
        .unwrap();

    let expected_buffer = vec![0x01, 0x02, 0x03, 0x04];
    assert_eq!(expected_buffer, buffer);
}

#[test]
fn get_length_according_to_type_u8_test() {
    let length = FieldType::U8.get_type_size();
    assert_eq!(mem::size_of::<u8>(), length);
}

#[test]
fn get_length_according_to_type_u16_test() {
    let length = FieldType::U16.get_type_size();
    assert_eq!(mem::size_of::<u16>(), length);
}

#[test]
fn get_length_according_to_type_u32_test() {
    let length = FieldType::U32.get_type_size();
    assert_eq!(mem::size_of::<u32>(), length);
}
