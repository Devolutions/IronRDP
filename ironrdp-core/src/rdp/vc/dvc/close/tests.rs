use lazy_static::lazy_static;

use super::*;

const DVC_TEST_CHANNEL_ID_U16: u32 = 0x0303;

const DVC_CLOSE_BUFFER_SIZE: usize = 3;
const DVC_CLOSE_BUFFER: [u8; DVC_CLOSE_BUFFER_SIZE] = [0x41, 0x03, 0x03];

lazy_static! {
    static ref DVC_CLOSE: ClosePdu = ClosePdu {
        channel_id_type: FieldType::U16,
        channel_id: DVC_TEST_CHANNEL_ID_U16
    };
}

#[test]
fn from_buffer_correct_parses_dvc_close_pdu() {
    assert_eq!(
        DVC_CLOSE.clone(),
        ClosePdu::from_buffer(&DVC_CLOSE_BUFFER[1..], FieldType::U16).unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_dvc_close_pdu() {
    let close = DVC_CLOSE.clone();

    let mut buffer = Vec::new();
    close.to_buffer(&mut buffer).unwrap();

    assert_eq!(DVC_CLOSE_BUFFER.as_ref(), buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_dvc_close_pdu() {
    let close = DVC_CLOSE.clone();
    let expected_buf_len = DVC_CLOSE_BUFFER.len();

    let len = close.buffer_length();

    assert_eq!(expected_buf_len, len);
}
