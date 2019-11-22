use lazy_static::lazy_static;

use super::*;

const DVC_TEST_CHANNEL_ID_U8: u32 = 0x03;
const DVC_TEST_DATA_LENGTH: u32 = 0x0000_0C7B;

const DVC_DATA_FIRST_BUFFER_SIZE: usize = 16;
const DVC_DATA_FIRST_BUFFER: [u8; DVC_DATA_FIRST_BUFFER_SIZE] = [
    0x24, 0x03, 0x7b, 0x0c, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71,
];
const DVC_DATA_FIRST_WITH_INVALID_TOTAL_MESSAGE_SIZE_BUFFER: [u8; 6] =
    [0x03, 0x03, 0x71, 0x71, 0x71, 0x71];

const DVC_TEST_DATA_BUFFER_SIZE: usize = 12;
const DVC_TEST_DATA_BUFFER: [u8; DVC_TEST_DATA_BUFFER_SIZE] = [
    0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71,
];

const DVC_INVALID_DATA_MESSAGE_BUFFER: [u8; PDU_WITH_DATA_MAX_SIZE] =
    [0x77; PDU_WITH_DATA_MAX_SIZE];

lazy_static! {
    static ref DVC_DATA_FIRST: DataFirstPdu = DataFirstPdu {
        channel_id_type: FieldType::U8,
        channel_id: DVC_TEST_CHANNEL_ID_U8,
        data_length_type: FieldType::U16,
        data_length: DVC_TEST_DATA_LENGTH,
        dvc_data: DVC_TEST_DATA_BUFFER.to_vec()
    };
}

#[test]
fn from_buffer_parsing_for_dvc_data_first_pdu_with_invalid_message_size_fails() {
    match DataFirstPdu::from_buffer(
        DVC_INVALID_DATA_MESSAGE_BUFFER.as_ref(),
        FieldType::U8,
        FieldType::U16,
    ) {
        Err(ChannelError::InvalidDvcMessageSize) => (),
        res => panic!("Expected InvalidDvcMessageSize error, got: {:?}", res),
    };
}

#[test]
fn from_buffer_parsing_for_dvc_data_first_pdu_with_invalid_total_message_size_fails() {
    match DataFirstPdu::from_buffer(
        DVC_DATA_FIRST_WITH_INVALID_TOTAL_MESSAGE_SIZE_BUFFER.as_ref(),
        FieldType::U8,
        FieldType::U8,
    ) {
        Err(ChannelError::InvalidDvcTotalMessageSize) => (),
        res => panic!("Expected InvalidDvcTotalMessageSize error, got: {:?}", res),
    };
}

#[test]
fn from_buffer_correct_parses_dvc_data_first_pdu() {
    assert_eq!(
        DVC_DATA_FIRST.clone(),
        DataFirstPdu::from_buffer(&DVC_DATA_FIRST_BUFFER[1..], FieldType::U8, FieldType::U16)
            .unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_dvc_data_first_pdu() {
    let data_first = DVC_DATA_FIRST.clone();

    let mut buffer = Vec::new();
    data_first.to_buffer(&mut buffer).unwrap();

    assert_eq!(DVC_DATA_FIRST_BUFFER.as_ref(), buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_dvc_data_first_pdu() {
    let data_first = DVC_DATA_FIRST.clone();
    let expected_buf_len = DVC_DATA_FIRST_BUFFER.len();

    let len = data_first.buffer_length();

    assert_eq!(expected_buf_len, len);
}
