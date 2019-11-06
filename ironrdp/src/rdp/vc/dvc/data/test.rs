use lazy_static::lazy_static;

use super::*;

const DVC_TEST_CHANNEL_ID_U8: u32 = 0x03;

const DVC_DATA_BUFFER_SIZE: usize = 14;
const DVC_DATA_BUFFER: [u8; DVC_DATA_BUFFER_SIZE] = [
    0x30, 0x03, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71,
];

const DVC_TEST_DATA_BUFFER_SIZE: usize = 12;
const DVC_TEST_DATA_BUFFER: [u8; DVC_TEST_DATA_BUFFER_SIZE] = [
    0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71, 0x71,
];

const DVC_INVALID_DATA_MESSAGE_BUFFER: [u8; PDU_WITH_DATA_MAX_SIZE] =
    [0x77; PDU_WITH_DATA_MAX_SIZE];

lazy_static! {
    static ref DVC_DATA: DataPdu = DataPdu {
        channel_id_type: FieldType::U8,
        channel_id: DVC_TEST_CHANNEL_ID_U8,
        dvc_data: DVC_TEST_DATA_BUFFER.to_vec()
    };
}

#[test]
fn from_buffer_parsing_for_dvc_data_pdu_with_invalid_message_size_fails() {
    match DataPdu::from_buffer(DVC_INVALID_DATA_MESSAGE_BUFFER.as_ref(), FieldType::U8) {
        Err(ChannelError::InvalidDvcMessageSize) => (),
        res => panic!("Expected InvalidDvcMessageSize error, got: {:?}", res),
    };
}

#[test]
fn from_buffer_correct_parses_dvc_data_pdu() {
    assert_eq!(
        DVC_DATA.clone(),
        DataPdu::from_buffer(&DVC_DATA_BUFFER[1..], FieldType::U8).unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_dvc_data_pdu() {
    let data = DVC_DATA.clone();

    let mut buffer = Vec::new();
    data.to_buffer(&mut buffer).unwrap();

    assert_eq!(DVC_DATA_BUFFER.as_ref(), buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_dvc_data_pdu() {
    let data = DVC_DATA.clone();
    let expected_buf_len = DVC_DATA_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buf_len, len);
}
