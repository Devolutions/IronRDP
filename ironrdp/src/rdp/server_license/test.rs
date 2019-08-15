use lazy_static::lazy_static;

use super::*;

pub const LICENSE_PACKET_BUFFER_LEN: usize = 16;
pub const LICENSE_PACKET_BUFFER: [u8; LICENSE_PACKET_BUFFER_LEN] = [
    0xff, 0x03, 0x10, 0x00, 0x07, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00,
];

lazy_static! {
    pub static ref LICENSE_PACKET: ServerLicense = ServerLicense {
        preamble: LicensePreamble {
            message_type: PreambleType::ErrorAlert,
            flags: PreambleFlags::empty(),
            version: PreambleVersion::V3,
        },
        error_message: LicensingErrorMessage {
            error_code: LicensingErrorCode::StatusValidClient,
            state_transition: LicensingStateTransition::NoTransition,
            error_info: LicensingBinaryBlob {
                blob_type: BlobType::Error,
                data: Vec::new(),
            },
        },
    };
}

#[test]
fn from_buffer_correct_parses_server_license() {
    assert_eq!(
        LICENSE_PACKET.clone(),
        ServerLicense::from_buffer(LICENSE_PACKET_BUFFER.as_ref()).unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_server_license() {
    let data = LICENSE_PACKET.clone();

    let mut buffer = Vec::new();
    data.to_buffer(&mut buffer).unwrap();

    assert_eq!(LICENSE_PACKET_BUFFER.as_ref(), buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_server_license() {
    let pdu = LICENSE_PACKET.clone();
    let expected_buf_len = LICENSE_PACKET_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}
