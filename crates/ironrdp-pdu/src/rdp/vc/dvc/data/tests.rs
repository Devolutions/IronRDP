use lazy_static::lazy_static;

use super::*;
use crate::{encode_vec, PduErrorKind};

const DVC_TEST_CHANNEL_ID_U8: u32 = 0x03;

const DVC_FULL_DATA_BUFFER_SIZE: usize = 14;
const DVC_DATA_PREFIX: [u8; 2] = [0x30, 0x03];
const DVC_DATA_BUFFER: [u8; 12] = [0x71; 12];

const DVC_INVALID_DATA_MESSAGE_BUFFER: [u8; PDU_WITH_DATA_MAX_SIZE] = [0x77; PDU_WITH_DATA_MAX_SIZE];

const DVC_TEST_HEADER_SIZE: usize = 0x01;

lazy_static! {
    static ref DVC_FULL_DATA_BUFFER: Vec<u8> = {
        let mut result = DVC_DATA_PREFIX.to_vec();
        result.extend(DVC_DATA_BUFFER);

        result
    };
    static ref DVC_DATA: DataPdu = DataPdu {
        channel_id_type: FieldType::U8,
        channel_id: DVC_TEST_CHANNEL_ID_U8,
        data_size: DVC_DATA_BUFFER.len()
    };
}

#[test]
fn from_buffer_parsing_for_dvc_data_pdu_with_invalid_message_size_fails() {
    let mut cur = ReadCursor::new(DVC_INVALID_DATA_MESSAGE_BUFFER.as_ref());
    match DataPdu::decode(&mut cur, FieldType::U8, PDU_WITH_DATA_MAX_SIZE) {
        Err(e) if matches!(e.kind(), PduErrorKind::InvalidMessage { .. }) => (),
        res => panic!("Expected InvalidDvcMessageSize error, got: {res:?}"),
    };
}

#[test]
fn from_buffer_correct_parses_dvc_data_pdu() {
    let mut cur = ReadCursor::new(&DVC_FULL_DATA_BUFFER[1..]);
    assert_eq!(
        DVC_DATA.clone(),
        DataPdu::decode(
            &mut cur,
            FieldType::U8,
            DVC_FULL_DATA_BUFFER_SIZE - DVC_TEST_HEADER_SIZE
        )
        .unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_dvc_data_pdu() {
    let data = DVC_DATA.clone();

    let buffer = encode_vec(&data).unwrap();

    assert_eq!(DVC_DATA_PREFIX.to_vec(), buffer);
}

#[test]
fn buffer_length_is_correct_for_dvc_data_pdu() {
    let data = DVC_DATA.clone();
    let expected_buf_len = DVC_DATA_PREFIX.len();

    let len = data.size();

    assert_eq!(expected_buf_len, len);
}
