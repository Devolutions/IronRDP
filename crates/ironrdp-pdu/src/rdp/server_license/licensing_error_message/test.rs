use lazy_static::lazy_static;

use super::*;
use crate::{
    decode, encode_vec,
    rdp::{
        headers::{BasicSecurityHeader, BasicSecurityHeaderFlags},
        server_license::{LicensePdu, PreambleFlags, PreambleVersion},
    },
};

const HEADER_MESSAGE_BUFFER: [u8; 8] = [0x80, 0x00, 0x00, 0x00, 0xFF, 0x03, 0x14, 0x00];

const LICENSE_MESSAGE_BUFFER: [u8; 12] = [
    0x07, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, // message
];

lazy_static! {
    pub static ref LICENSING_ERROR_MESSAGE: LicensePdu = {
        let mut pdu = LicensingErrorMessage {
            license_header: LicenseHeader {
                security_header: BasicSecurityHeader {
                    flags: BasicSecurityHeaderFlags::LICENSE_PKT,
                },
                preamble_message_type: PreambleType::ErrorAlert,
                preamble_flags: PreambleFlags::empty(),
                preamble_version: PreambleVersion::V3,
                preamble_message_size: 0,
            },
            error_code: LicenseErrorCode::StatusValidClient,
            state_transition: LicensingStateTransition::NoTransition,
            error_info: Vec::new(),
        };
        pdu.license_header.preamble_message_size = pdu.size() as u16;
        pdu.into()
    };
}

#[test]
fn from_buffer_correctly_parses_licensing_error_message() {
    assert_eq!(
        *LICENSING_ERROR_MESSAGE,
        decode(&[&HEADER_MESSAGE_BUFFER[..], &LICENSE_MESSAGE_BUFFER[..]].concat()).unwrap(),
    );
}

#[test]
fn to_buffer_correctly_serializes_licensing_error_message() {
    let buffer = encode_vec(&*LICENSING_ERROR_MESSAGE).unwrap();

    assert_eq!(
        [&HEADER_MESSAGE_BUFFER[..], &LICENSE_MESSAGE_BUFFER[..]].concat(),
        buffer
    );
}

#[test]
fn buffer_length_is_correct_for_licensing_error_message() {
    assert_eq!(
        HEADER_MESSAGE_BUFFER.len() + LICENSE_MESSAGE_BUFFER.len(),
        LICENSING_ERROR_MESSAGE.size()
    );
}
