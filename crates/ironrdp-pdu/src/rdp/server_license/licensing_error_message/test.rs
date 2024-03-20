use lazy_static::lazy_static;

use super::*;
use crate::{decode, encode_vec};

pub const LICENSE_MESSAGE_BUFFER: [u8; 12] = [
    0x07, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, // message
];

lazy_static! {
    pub static ref LICENSING_ERROR_MESSAGE: LicensingErrorMessage = LicensingErrorMessage {
        error_code: LicenseErrorCode::StatusValidClient,
        state_transition: LicensingStateTransition::NoTransition,
        error_info: Vec::new(),
    };
}

#[test]
fn from_buffer_correctly_parses_licensing_error_message() {
    assert_eq!(*LICENSING_ERROR_MESSAGE, decode(&LICENSE_MESSAGE_BUFFER).unwrap(),);
}

#[test]
fn to_buffer_correctly_serializes_licensing_error_message() {
    let buffer = encode_vec(&*LICENSING_ERROR_MESSAGE).unwrap();

    assert_eq!(LICENSE_MESSAGE_BUFFER.as_ref(), buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_licensing_error_message() {
    assert_eq!(LICENSE_MESSAGE_BUFFER.len(), LICENSING_ERROR_MESSAGE.size());
}
