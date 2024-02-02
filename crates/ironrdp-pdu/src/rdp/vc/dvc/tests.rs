use lazy_static::lazy_static;

use super::*;
use crate::{decode, encode_vec, PduErrorKind};

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
    match decode::<Header>([invalid_header].as_ref()) {
        Err(_) => (),
        res => panic!("Expected InvalidDvcPduType error, got: {res:?}"),
    };
}

#[test]
fn from_buffer_correct_parses_dvc_header() {
    assert_eq!(
        *DYNAMIC_CHANNEL_HEADER,
        decode::<Header>(DVC_HEADER_BUFFER.as_ref()).unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_dvc_header() {
    let channel_header = &*DYNAMIC_CHANNEL_HEADER;

    let buffer = encode_vec(channel_header).unwrap();

    assert_eq!(DVC_HEADER_BUFFER.as_ref(), buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_dvc_header() {
    let channel_header = &*DYNAMIC_CHANNEL_HEADER;
    let expected_buf_len = DVC_HEADER_BUFFER.len();

    let len = channel_header.size();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn from_buffer_parsing_for_client_dvc_pdu_with_invalid_id_length_type_fails() {
    let mut cur = ReadCursor::new(DVC_HEADER_WITH_INVALID_ID_LENGTH_TYPE_BUFFER.as_ref());
    match ClientPdu::decode(&mut cur, HEADER_SIZE) {
        Err(e) if matches!(e.kind(), PduErrorKind::InvalidMessage { .. }) => (),
        res => panic!("Expected InvalidDVChannelIdLength error, got: {res:?}"),
    };
}

#[test]
fn from_buffer_parsing_for_server_dvc_pdu_with_invalid_id_length_type_fails() {
    let mut cur = ReadCursor::new(DVC_HEADER_WITH_INVALID_ID_LENGTH_TYPE_BUFFER.as_ref());
    match ServerPdu::decode(&mut cur, HEADER_SIZE) {
        Err(e) if matches!(e.kind(), PduErrorKind::InvalidMessage { .. }) => (),
        res => panic!("Expected InvalidDVChannelIdLength error, got: {res:?}"),
    };
}

#[test]
fn from_buffer_according_to_type_u8_test() {
    let mut cur = ReadCursor::new(TEST_BUFFER.as_ref());
    let channel_id = FieldType::U8.read_according_to_type(&mut cur).unwrap();
    let expected_channel_id = 0x01;

    assert_eq!(expected_channel_id, channel_id);
}

#[test]
fn from_buffer_according_to_type_u16_test() {
    let mut cur = ReadCursor::new(TEST_BUFFER.as_ref());
    let channel_id = FieldType::U16.read_according_to_type(&mut cur).unwrap();
    let expected_channel_id = 0x0201;

    assert_eq!(expected_channel_id, channel_id);
}

#[test]
fn from_buffer_according_to_type_u32_test() {
    let mut cur = ReadCursor::new(TEST_BUFFER.as_ref());
    let channel_id = FieldType::U32.read_according_to_type(&mut cur).unwrap();
    let expected_channel_id = 0x0403_0201;

    assert_eq!(expected_channel_id, channel_id);
}

#[test]
fn to_buffer_according_to_type_u8_test() {
    let channel_id = 0x01;
    let mut buffer = vec![0; FieldType::U8.size()];
    let mut cur = WriteCursor::new(&mut buffer);
    FieldType::U8.write_according_to_type(&mut cur, channel_id).unwrap();

    let expected_buffer = vec![0x01];
    assert_eq!(expected_buffer, buffer);
}

#[test]
fn to_buffer_according_to_type_u16_test() {
    let channel_id = 0x0201;
    let mut buffer = vec![0; FieldType::U16.size()];
    let mut cur = WriteCursor::new(&mut buffer);
    FieldType::U16.write_according_to_type(&mut cur, channel_id).unwrap();

    let expected_buffer = vec![0x01, 0x02];
    assert_eq!(expected_buffer, buffer);
}

#[test]
fn to_buffer_according_to_type_u32_test() {
    let channel_id = 0x0403_0201;
    let mut buffer = vec![0; FieldType::U32.size()];
    let mut cur = WriteCursor::new(&mut buffer);
    FieldType::U32.write_according_to_type(&mut cur, channel_id).unwrap();

    let expected_buffer = vec![0x01, 0x02, 0x03, 0x04];
    assert_eq!(expected_buffer, buffer);
}

#[test]
fn get_length_according_to_type_u8_test() {
    let length = FieldType::U8.size();
    assert_eq!(1, length);
}

#[test]
fn get_length_according_to_type_u16_test() {
    let length = FieldType::U16.size();
    assert_eq!(2, length);
}

#[test]
fn get_length_according_to_type_u32_test() {
    let length = FieldType::U32.size();
    assert_eq!(4, length);
}
